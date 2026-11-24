//! Pairing sessions on the Ring: short-lived, single-use, in memory only.
//!
//! A session is created by `bring pair`, driven by the Moon through the
//! public `/pair/*` endpoints, and decided by the user at the terminal.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use bring_core::api::{
    ErrorCode, PairConfirmRequest, PairExchangeRequest, PairExchangeResponse, PairResultResponse,
    PairingResult,
};
use bring_core::codec::hex_encode;
use bring_core::pairing::{BootstrapPayload, OneTimeToken, PairingId, RingHandshake};
use tokio::sync::watch;

use crate::devices::{self, Device};
use crate::server::ApiError;
use crate::store::Db;

/// Exchange attempts per session (typos in a manually entered code).
pub const MAX_ATTEMPTS: u32 = 3;
/// Grace period for the Moon to fetch an approved result.
const RESULT_GRACE: Duration = Duration::from_secs(60);

/// Observable progress of a session (for the CLI conversation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairStatus {
    Waiting,
    AwaitingUser { sas: String, device_name: String },
    Approved { device_id: i64 },
    Rejected,
    Expired,
    TooManyAttempts,
}

enum Phase {
    Waiting,
    Exchanged(RingHandshake),
    AwaitingUser(RingHandshake),
    Approved(PairResultResponse),
    Rejected,
    TooManyAttempts,
}

struct Session {
    token: OneTimeToken,
    expires: Instant,
    attempts: u32,
    phase: Phase,
    status: watch::Sender<PairStatus>,
}

struct Settings {
    ring_id: [u8; 16],
    tls_cert_sha256: [u8; 32],
    wan_url: Option<String>,
    ttl: Duration,
    advertise: Mutex<SocketAddrV4>,
}

/// The pairing session registry.
#[derive(Clone)]
pub struct Pairing {
    sessions: Arc<Mutex<HashMap<PairingId, Session>>>,
    settings: Arc<Settings>,
    db: Db,
}

fn err(status: StatusCode, code: ErrorCode, msg: &str) -> ApiError {
    ApiError::new(status, code, msg)
}

fn expired() -> ApiError {
    err(
        StatusCode::GONE,
        ErrorCode::Expired,
        "pairing session expired",
    )
}

fn rejected() -> ApiError {
    err(
        StatusCode::FORBIDDEN,
        ErrorCode::Rejected,
        "pairing rejected",
    )
}

fn too_many() -> ApiError {
    err(
        StatusCode::FORBIDDEN,
        ErrorCode::TooManyAttempts,
        "too many pairing attempts",
    )
}

fn conflict(msg: &str) -> ApiError {
    err(StatusCode::CONFLICT, ErrorCode::Conflict, msg)
}

/// Best guess at this machine's LAN IPv4 address (no packets are sent).
pub fn detect_lan_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()),
        _ => None,
    }
}

impl Pairing {
    pub fn new(
        db: Db,
        ring_id: [u8; 16],
        tls_cert_sha256: [u8; 32],
        advertise: SocketAddrV4,
        wan_url: Option<String>,
        ttl: Duration,
    ) -> Self {
        Self {
            sessions: Arc::default(),
            settings: Arc::new(Settings {
                ring_id,
                tls_cert_sha256,
                wan_url,
                ttl,
                advertise: Mutex::new(advertise),
            }),
            db,
        }
    }

    /// The LAN address put into payloads (set once the listener is bound).
    pub fn set_advertise(&self, addr: SocketAddrV4) {
        *self
            .settings
            .advertise
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = addr;
    }

    pub fn advertise(&self) -> SocketAddrV4 {
        *self
            .settings
            .advertise
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn lan_url(&self) -> String {
        format!("https://{}", self.advertise())
    }

    pub fn ttl(&self) -> Duration {
        self.settings.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PairingId, Session>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Opens a new session.
    pub fn start(&self) -> Result<(BootstrapPayload, watch::Receiver<PairStatus>), ApiError> {
        let payload = BootstrapPayload::generate(self.advertise()).map_err(ApiError::internal)?;
        let (tx, rx) = watch::channel(PairStatus::Waiting);
        let mut map = self.lock();
        let now = Instant::now();
        map.retain(|_, s| s.expires > now);
        map.insert(
            payload.pairing_id,
            Session {
                token: payload.token.clone(),
                expires: now + self.settings.ttl,
                attempts: 0,
                phase: Phase::Waiting,
                status: tx,
            },
        );
        Ok((payload, rx))
    }

    /// Ends a session (CLI went away or timed out).
    pub fn close(&self, id: &PairingId, why: PairStatus) {
        if let Some(s) = self.lock().remove(id) {
            let _ = s.status.send(why);
        }
    }

    /// Runs `f` on a live session; expired sessions are removed (`410`).
    fn with_session<T>(
        &self,
        id: &PairingId,
        f: impl FnOnce(&mut Session) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let mut map = self.lock();
        let Some(s) = map.get_mut(id) else {
            return Err(ApiError::not_found("no such pairing session"));
        };
        if Instant::now() > s.expires {
            let s = map.remove(id).expect("present");
            let _ = s.status.send(PairStatus::Expired);
            return Err(expired());
        }
        f(s)
    }

    /// `POST /pair/{id}/exchange`
    pub fn exchange(
        &self,
        id: &PairingId,
        req: &PairExchangeRequest,
    ) -> Result<PairExchangeResponse, ApiError> {
        let ring_id = self.settings.ring_id;
        self.with_session(id, |s| {
            match s.phase {
                Phase::Waiting | Phase::Exchanged(_) => {}
                Phase::AwaitingUser(_) | Phase::Approved(_) => {
                    return Err(conflict("pairing already confirmed"));
                }
                Phase::Rejected => return Err(rejected()),
                Phase::TooManyAttempts => return Err(too_many()),
            }
            s.attempts += 1;
            if s.attempts > MAX_ATTEMPTS {
                s.phase = Phase::TooManyAttempts;
                let _ = s.status.send(PairStatus::TooManyAttempts);
                return Err(too_many());
            }
            let (ring, resp) = RingHandshake::respond(id, &s.token, &ring_id, req)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            s.phase = Phase::Exchanged(ring);
            Ok(resp)
        })
    }

    /// `POST /pair/{id}/confirm`
    pub fn confirm(&self, id: &PairingId, req: &PairConfirmRequest) -> Result<(), ApiError> {
        self.with_session(id, |s| {
            match std::mem::replace(&mut s.phase, Phase::Waiting) {
                Phase::Exchanged(ring) => {
                    if ring.verify_confirm(req).is_err() {
                        // The attempt is spent; the session waits for a retry.
                        if s.attempts >= MAX_ATTEMPTS {
                            s.phase = Phase::TooManyAttempts;
                            let _ = s.status.send(PairStatus::TooManyAttempts);
                        }
                        return Err(rejected());
                    }
                    let _ = s.status.send(PairStatus::AwaitingUser {
                        sas: ring.sas().to_owned(),
                        device_name: ring.device_name.clone(),
                    });
                    s.phase = Phase::AwaitingUser(ring);
                    Ok(())
                }
                Phase::AwaitingUser(ring) => {
                    // Idempotent retry of the same confirmation.
                    let ok = ring.verify_confirm(req).is_ok();
                    s.phase = Phase::AwaitingUser(ring);
                    if ok { Ok(()) } else { Err(rejected()) }
                }
                other => {
                    let e = match &other {
                        Phase::Rejected => rejected(),
                        Phase::TooManyAttempts => too_many(),
                        Phase::Waiting => conflict("exchange first"),
                        _ => conflict("pairing already decided"),
                    };
                    s.phase = other;
                    Err(e)
                }
            }
        })
    }

    /// `GET /pair/{id}/result`: `None` while the user has not decided.
    pub fn result(&self, id: &PairingId) -> Result<Option<PairResultResponse>, ApiError> {
        self.with_session(id, |s| match &s.phase {
            Phase::Approved(sealed) => Ok(Some(sealed.clone())),
            Phase::AwaitingUser(_) => Ok(None),
            Phase::Rejected => Err(rejected()),
            Phase::TooManyAttempts => Err(too_many()),
            Phase::Waiting | Phase::Exchanged(_) => Err(conflict("not confirmed yet")),
        })
    }

    /// The user's decision at the terminal. Blocking (writes the device).
    pub fn decide(&self, id: &PairingId, approve: bool) -> Result<Option<Device>, ApiError> {
        let st = self.settings.clone();
        let lan_url = self.lan_url();
        let db = self.db.clone();
        self.with_session(id, |s| {
            let ring = match std::mem::replace(&mut s.phase, Phase::Rejected) {
                Phase::AwaitingUser(ring) => ring,
                other => {
                    s.phase = other;
                    return Err(conflict("nothing to decide"));
                }
            };
            // From here on, any failure leaves the session rejected (fail closed).
            if !approve {
                let _ = s.status.send(PairStatus::Rejected);
                return Ok(None);
            }
            let registered = devices::register(
                &db.lock(),
                ring.device_uuid,
                &ring.device_name,
                devices::now_secs(),
            );
            let sealed = registered
                .map_err(ApiError::from)
                .and_then(|(device, token)| {
                    ring.seal(&PairingResult {
                        device_token: token,
                        device_slug: device.slug.clone(),
                        ring_id: hex_encode(&st.ring_id),
                        tls_cert_sha256: hex_encode(&st.tls_cert_sha256),
                        lan_url,
                        wan_url: st.wan_url.clone(),
                    })
                    .map(|sealed| (device, sealed))
                    .map_err(ApiError::internal)
                });
            let (device, sealed) = match sealed {
                Ok(v) => v,
                Err(e) => {
                    let _ = s.status.send(PairStatus::Rejected);
                    return Err(e);
                }
            };
            s.phase = Phase::Approved(sealed);
            s.expires = s.expires.max(Instant::now() + RESULT_GRACE);
            let _ = s.status.send(PairStatus::Approved {
                device_id: device.id,
            });
            Ok(Some(device))
        })
    }
}
