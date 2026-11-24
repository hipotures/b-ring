//! Moon side of pairing (docs/ARCHITECTURE.md §3.6).
//!
//! Two calls, because a human sits in between:
//! 1. [`Moon::pair_start`]: decode the code, `GET /health` for the `ring_id`,
//!    SPAKE2 exchange, verify the Ring's key confirmation, send ours. Returns
//!    the SAS to show next to the one `bring pair` prints.
//! 2. [`Moon::pair_finish`]: poll `/result` until the user approved on the
//!    Ring, open `sealed`, check that every TLS certificate seen during
//!    pairing is the one the Ring vouches for, store the Ring.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bring_core::api::{HealthResponse, PairExchangeResponse, PairResultResponse};
use bring_core::codec::hex_decode;
use bring_core::pairing::{BootstrapPayload, MoonHandshake, MoonVerified};
use reqwest::{Client, StatusCode};

use crate::client::{self, RecordingVerifier};
use crate::error::MoonError;
use crate::{Moon, RingInfo, store};

const POLL_EVERY: Duration = Duration::from_millis(700);

/// A handshake waiting for the user's approval on the Ring.
pub(crate) struct PendingPairing {
    base: String,
    pairing_id: String,
    http: Client,
    verifier: Arc<RecordingVerifier>,
    verified: MoonVerified,
    device_uuid: [u8; 16],
}

impl PendingPairing {
    fn url(&self, step: &str) -> String {
        format!("{}/api/v1/pair/{}/{step}", self.base, self.pairing_id)
    }
}

/// What to show the user after [`Moon::pair_start`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairingSas {
    /// Six digits; must equal the code shown by `bring pair`.
    pub sas: String,
    pub ring_url: String,
}

/// Result of a completed pairing.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairedRing {
    /// Store this in the Android Keystore; it is never written to SQLite.
    pub device_token: String,
    pub ring: RingInfo,
}

impl std::fmt::Debug for PairedRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedRing")
            .field("device_token", &"<redacted>")
            .field("ring", &self.ring)
            .finish()
    }
}

fn random_uuid() -> Result<[u8; 16], MoonError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| MoonError::storage(format!("no randomness: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(b)
}

impl Moon {
    pub(crate) fn pair_start_inner(
        &self,
        code: &str,
        device_name: &str,
    ) -> Result<PairingSas, MoonError> {
        let payload = BootstrapPayload::from_text(code)?;
        let existing = store::ring(&self.db())?;
        // Re-pairing keeps the device identity, so the Ring keeps its slug.
        let device_uuid = match &existing {
            Some(r) => r.device_uuid,
            None => random_uuid()?,
        };
        let verifier = Arc::new(RecordingVerifier::default());
        let http = client::recording(verifier.clone())?;
        let base = payload.lan_url();
        let pairing_id = payload.pairing_id.to_hex();

        let verified = self.rt.block_on(async {
            let health: HealthResponse = client::json(
                http.get(format!("{base}/api/v1/health"))
                    .timeout(Duration::from_secs(5)),
            )
            .await?;
            let ring_id: [u8; 16] = hex_decode(&health.ring_id)
                .ok_or_else(|| MoonError::invalid("bad ring_id from /health"))?;
            if let Some(r) = &existing
                && r.ring_id != ring_id
            {
                return Err(MoonError::invalid(
                    "already paired with another Ring; forget it first",
                ));
            }
            let (hs, req) = MoonHandshake::start(&payload, ring_id, device_uuid, device_name)?;
            let url = |step: &str| format!("{base}/api/v1/pair/{pairing_id}/{step}");
            let resp: PairExchangeResponse =
                client::json(http.post(url("exchange")).json(&req)).await?;
            let verified = hs.finish(&resp)?;
            client::send(http.post(url("confirm")).json(&verified.confirm_request())).await?;
            Ok::<_, MoonError>(verified)
        })?;

        let sas = PairingSas {
            sas: verified.sas().to_owned(),
            ring_url: base.clone(),
        };
        *self.pending.lock().expect("pending lock") = Some(PendingPairing {
            base,
            pairing_id,
            http,
            verifier,
            verified,
            device_uuid,
        });
        Ok(sas)
    }

    pub(crate) fn pair_finish_inner(&self, timeout: Duration) -> Result<PairedRing, MoonError> {
        let pending = self
            .pending
            .lock()
            .expect("pending lock")
            .take()
            .ok_or_else(|| MoonError::invalid("no pairing in progress"))?;
        let deadline = Instant::now() + timeout;
        let polled = self.rt.block_on(async {
            loop {
                let resp = pending.http.get(pending.url("result")).send().await?;
                match resp.status() {
                    StatusCode::OK => return Ok(Some(resp.json::<PairResultResponse>().await?)),
                    StatusCode::ACCEPTED => {}
                    StatusCode::FORBIDDEN => return Err(MoonError::PairingRejected),
                    StatusCode::GONE | StatusCode::NOT_FOUND => {
                        return Err(MoonError::PairingExpired);
                    }
                    s => {
                        let body = resp.bytes().await.unwrap_or_default();
                        return Err(MoonError::from_response(s.as_u16(), &body));
                    }
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                tokio::time::sleep(POLL_EVERY).await;
            }
        });
        let sealed = match polled {
            Ok(Some(s)) => s,
            // Still waiting for the user: keep the session for another call.
            Ok(None) => {
                *self.pending.lock().expect("pending lock") = Some(pending);
                return Err(MoonError::PairingPending);
            }
            Err(e @ MoonError::Network { .. }) => {
                *self.pending.lock().expect("pending lock") = Some(pending);
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        let observed = pending
            .verifier
            .observed()
            .ok_or_else(|| MoonError::Pairing {
                detail: "the Ring's TLS certificate changed during pairing".into(),
            })?;
        let result = pending.verified.open_result(&sealed, &observed)?;
        let row = store::save_ring(&self.db(), &result, pending.device_uuid)?;
        *self.token.lock().expect("token lock") = Some(result.device_token.clone());
        Ok(PairedRing {
            device_token: result.device_token,
            ring: RingInfo::from(&row),
        })
    }
}
