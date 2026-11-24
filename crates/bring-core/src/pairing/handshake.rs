//! The pairing handshake (docs/ARCHITECTURE.md §3.6).
//!
//! ```text
//! Moon (A)                                         Ring (B)
//!   GET /health -> ring_id
//!   SPAKE2 start_a(pw, A, B) ── exchange{uuid, name, msgA} ──▶ start_b, finish
//!                            ◀── {msgB, ring_confirm} ──────── derive keys
//!   finish, derive keys, verify ring_confirm
//!   show SAS                 ── confirm{moon_confirm} ──────▶ verify, show SAS
//!                                                            user approves
//!                            ◀── result{sealed} ──────────── AEAD(PairingResult)
//!   open, cross-check TLS pin
//! ```
//!
//! - password = `pairing_id || one_time_token`
//! - identities = `"bring-moon:" || device_uuid`, `"bring-ring:" || ring_id`
//! - transcript = SHA-256 over length-prefixed protocol fields
//! - keys = HKDF-SHA256(ikm = SPAKE2 key, salt = transcript, info = label)
//! - SAS = first 4 bytes of the `sas` key, big-endian, mod 1 000 000

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use super::payload::{BootstrapPayload, OneTimeToken, PairingId};
use crate::api::{
    PairConfirmRequest, PairExchangeRequest, PairExchangeResponse, PairResultResponse,
    PairingResult,
};
use crate::codec::{b64_decode, b64_encode, hex_decode, hex_encode};

/// Longest accepted device name, in bytes.
pub const MAX_DEVICE_NAME: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairingError {
    #[error("malformed pairing message: {0}")]
    BadMessage(&'static str),
    #[error("key confirmation failed (wrong code or tampering)")]
    KeyConfirmation,
    #[error("cannot open the pairing result")]
    Open,
    #[error("the Ring's TLS certificate does not match the one seen during pairing")]
    CertificateMismatch,
    #[error("randomness unavailable")]
    Random,
}

type HmacSha256 = Hmac<Sha256>;

/// Keys derived from a completed SPAKE2 exchange.
struct Keys {
    transcript: [u8; 32],
    confirm_ring: [u8; 32],
    confirm_moon: [u8; 32],
    seal: [u8; 32],
    sas: String,
}

struct Context<'a> {
    pairing_id: &'a PairingId,
    ring_id: &'a [u8; 16],
    device_uuid: &'a [u8; 16],
    device_name: &'a str,
}

impl Context<'_> {
    fn password(&self, token: &OneTimeToken) -> Password {
        let mut pw = self.pairing_id.0.to_vec();
        pw.extend_from_slice(&token.0);
        Password::new(pw)
    }

    fn id_moon(&self) -> Identity {
        Identity::new(&[b"bring-moon:".as_slice(), self.device_uuid].concat())
    }

    fn id_ring(&self) -> Identity {
        Identity::new(&[b"bring-ring:".as_slice(), self.ring_id].concat())
    }

    fn transcript(&self, moon_msg: &[u8], ring_msg: &[u8]) -> [u8; 32] {
        let mut h = Sha256::new();
        for field in [
            b"bring/pair/v1".as_slice(),
            &self.pairing_id.0,
            self.ring_id,
            self.device_uuid,
            self.device_name.as_bytes(),
            moon_msg,
            ring_msg,
        ] {
            h.update((field.len() as u32).to_be_bytes());
            h.update(field);
        }
        h.finalize().into()
    }

    fn derive(&self, spake_key: &[u8], moon_msg: &[u8], ring_msg: &[u8]) -> Keys {
        let transcript = self.transcript(moon_msg, ring_msg);
        let hk = Hkdf::<Sha256>::new(Some(&transcript), spake_key);
        let expand = |info: &[u8]| {
            let mut out = [0u8; 32];
            hk.expand(info, &mut out)
                .expect("32 bytes is a valid HKDF length");
            out
        };
        let sas_key = expand(b"bring/sas/v1");
        let n = u32::from_be_bytes([sas_key[0], sas_key[1], sas_key[2], sas_key[3]]) % 1_000_000;
        Keys {
            transcript,
            confirm_ring: expand(b"bring/confirm/ring/v1"),
            confirm_moon: expand(b"bring/confirm/moon/v1"),
            seal: expand(b"bring/seal/v1"),
            sas: format!("{n:06}"),
        }
    }
}

fn mac(key: &[u8; 32], transcript: &[u8; 32]) -> Vec<u8> {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key length");
    m.update(transcript);
    m.finalize().into_bytes().to_vec()
}

fn verify_mac(key: &[u8; 32], transcript: &[u8; 32], tag: &[u8]) -> Result<(), PairingError> {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key length");
    m.update(transcript);
    m.verify_slice(tag)
        .map_err(|_| PairingError::KeyConfirmation)
}

fn validate_name(name: &str) -> Result<(), PairingError> {
    if name.trim().is_empty() || name.len() > MAX_DEVICE_NAME || name.chars().any(char::is_control)
    {
        return Err(PairingError::BadMessage("device name"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Moon side
// ---------------------------------------------------------------------------

/// Moon state after sending the exchange request.
pub struct MoonHandshake {
    spake: Spake2<Ed25519Group>,
    pairing_id: PairingId,
    ring_id: [u8; 16],
    device_uuid: [u8; 16],
    device_name: String,
    moon_msg: Vec<u8>,
}

impl MoonHandshake {
    /// Starts pairing. `ring_id` comes from `GET /health` (unauthenticated:
    /// a wrong value only makes key confirmation fail).
    pub fn start(
        payload: &BootstrapPayload,
        ring_id: [u8; 16],
        device_uuid: [u8; 16],
        device_name: &str,
    ) -> Result<(Self, PairExchangeRequest), PairingError> {
        validate_name(device_name)?;
        let ctx = Context {
            pairing_id: &payload.pairing_id,
            ring_id: &ring_id,
            device_uuid: &device_uuid,
            device_name,
        };
        let (spake, moon_msg) = Spake2::<Ed25519Group>::start_a(
            &ctx.password(&payload.token),
            &ctx.id_moon(),
            &ctx.id_ring(),
        );
        let req = PairExchangeRequest {
            device_uuid: hex_encode(&device_uuid),
            device_name: device_name.to_owned(),
            spake_msg: b64_encode(&moon_msg),
        };
        Ok((
            Self {
                spake,
                pairing_id: payload.pairing_id,
                ring_id,
                device_uuid,
                device_name: device_name.to_owned(),
                moon_msg,
            },
            req,
        ))
    }

    /// Processes the Ring's answer; fails unless the Ring knew the token.
    pub fn finish(self, resp: &PairExchangeResponse) -> Result<MoonVerified, PairingError> {
        let ring_msg = b64_decode(&resp.spake_msg).ok_or(PairingError::BadMessage("spake_msg"))?;
        let ring_confirm =
            b64_decode(&resp.ring_confirm).ok_or(PairingError::BadMessage("ring_confirm"))?;
        let key = self
            .spake
            .finish(&ring_msg)
            .map_err(|_| PairingError::BadMessage("spake_msg"))?;
        let ctx = Context {
            pairing_id: &self.pairing_id,
            ring_id: &self.ring_id,
            device_uuid: &self.device_uuid,
            device_name: &self.device_name,
        };
        let keys = ctx.derive(&key, &self.moon_msg, &ring_msg);
        verify_mac(&keys.confirm_ring, &keys.transcript, &ring_confirm)?;
        Ok(MoonVerified { keys })
    }
}

/// Moon state once the Ring has proven knowledge of the token.
pub struct MoonVerified {
    keys: Keys,
}

impl MoonVerified {
    /// The 6-digit code to show the user.
    pub fn sas(&self) -> &str {
        &self.keys.sas
    }

    pub fn confirm_request(&self) -> PairConfirmRequest {
        PairConfirmRequest {
            moon_confirm: b64_encode(&mac(&self.keys.confirm_moon, &self.keys.transcript)),
        }
    }

    /// Decrypts the result and checks that the TLS certificate this Moon
    /// actually saw during pairing (`observed_cert_sha256`) is the one the
    /// Ring vouches for. A mismatch means someone sat in the TLS path.
    pub fn open_result(
        &self,
        resp: &PairResultResponse,
        observed_cert_sha256: &[u8; 32],
    ) -> Result<PairingResult, PairingError> {
        let raw = b64_decode(&resp.sealed).ok_or(PairingError::Open)?;
        if raw.len() < 12 {
            return Err(PairingError::Open);
        }
        let (nonce, ct) = raw.split_at(12);
        let plain = ChaCha20Poly1305::new((&self.keys.seal).into())
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: &self.keys.transcript,
                },
            )
            .map_err(|_| PairingError::Open)?;
        let result: PairingResult =
            serde_json::from_slice(&plain).map_err(|_| PairingError::BadMessage("result"))?;
        let pinned = hex_decode::<32>(&result.tls_cert_sha256)
            .ok_or(PairingError::BadMessage("tls_cert_sha256"))?;
        if &pinned != observed_cert_sha256 {
            return Err(PairingError::CertificateMismatch);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Ring side
// ---------------------------------------------------------------------------

/// Ring state after answering an exchange.
pub struct RingHandshake {
    keys: Keys,
    pub device_uuid: [u8; 16],
    pub device_name: String,
}

impl RingHandshake {
    /// Answers a Moon's exchange request.
    pub fn respond(
        pairing_id: &PairingId,
        token: &OneTimeToken,
        ring_id: &[u8; 16],
        req: &PairExchangeRequest,
    ) -> Result<(Self, PairExchangeResponse), PairingError> {
        validate_name(&req.device_name)?;
        let device_uuid =
            hex_decode::<16>(&req.device_uuid).ok_or(PairingError::BadMessage("device_uuid"))?;
        let moon_msg = b64_decode(&req.spake_msg).ok_or(PairingError::BadMessage("spake_msg"))?;
        let ctx = Context {
            pairing_id,
            ring_id,
            device_uuid: &device_uuid,
            device_name: &req.device_name,
        };
        let (spake, ring_msg) =
            Spake2::<Ed25519Group>::start_b(&ctx.password(token), &ctx.id_moon(), &ctx.id_ring());
        let key = spake
            .finish(&moon_msg)
            .map_err(|_| PairingError::BadMessage("spake_msg"))?;
        let keys = ctx.derive(&key, &moon_msg, &ring_msg);
        let resp = PairExchangeResponse {
            spake_msg: b64_encode(&ring_msg),
            ring_confirm: b64_encode(&mac(&keys.confirm_ring, &keys.transcript)),
        };
        Ok((
            Self {
                keys,
                device_uuid,
                device_name: req.device_name.clone(),
            },
            resp,
        ))
    }

    /// Checks the Moon's key confirmation (constant-time).
    pub fn verify_confirm(&self, req: &PairConfirmRequest) -> Result<(), PairingError> {
        let tag = b64_decode(&req.moon_confirm).ok_or(PairingError::BadMessage("moon_confirm"))?;
        verify_mac(&self.keys.confirm_moon, &self.keys.transcript, &tag)
    }

    pub fn sas(&self) -> &str {
        &self.keys.sas
    }

    /// Encrypts the result for this Moon only.
    pub fn seal(&self, result: &PairingResult) -> Result<PairResultResponse, PairingError> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|_| PairingError::Random)?;
        let plain = serde_json::to_vec(result).map_err(|_| PairingError::BadMessage("result"))?;
        let ct = ChaCha20Poly1305::new((&self.keys.seal).into())
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &self.keys.transcript,
                },
            )
            .map_err(|_| PairingError::Open)?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(PairResultResponse {
            sealed: b64_encode(&out),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    const RING: [u8; 16] = [0xAA; 16];
    const UUID: [u8; 16] = [0x55; 16];
    const CERT: [u8; 32] = [0xCE; 32];

    fn payload() -> BootstrapPayload {
        BootstrapPayload::generate(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7443)).unwrap()
    }

    fn result() -> PairingResult {
        PairingResult {
            device_token: "tok".into(),
            device_slug: "pixel".into(),
            ring_id: hex_encode(&RING),
            tls_cert_sha256: hex_encode(&CERT),
            lan_url: "https://127.0.0.1:7443".into(),
            wan_url: None,
        }
    }

    /// Runs the full protocol; the Ring uses `ring_token`.
    fn run(ring_token: &OneTimeToken) -> Result<(MoonVerified, RingHandshake), PairingError> {
        let p = payload();
        let (moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8")?;
        let (ring, resp) = RingHandshake::respond(&p.pairing_id, ring_token, &RING, &req)?;
        let moon = moon.finish(&resp)?;
        ring.verify_confirm(&moon.confirm_request())?;
        Ok((moon, ring))
    }

    #[test]
    fn honest_run_agrees() {
        let p = payload();
        let (moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8").unwrap();
        let (ring, resp) = RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).unwrap();
        let moon = moon.finish(&resp).unwrap();
        ring.verify_confirm(&moon.confirm_request()).unwrap();
        assert_eq!(moon.sas(), ring.sas());
        assert_eq!(moon.sas().len(), 6);
        assert!(moon.sas().chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            (ring.device_uuid, ring.device_name.as_str()),
            (UUID, "Pixel 8")
        );

        let sealed = ring.seal(&result()).unwrap();
        assert_eq!(moon.open_result(&sealed, &CERT).unwrap(), result());
    }

    #[test]
    fn wrong_token_fails_key_confirmation() {
        let p = payload();
        let (moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8").unwrap();
        let wrong = OneTimeToken([0; 16]);
        let (_ring, resp) = RingHandshake::respond(&p.pairing_id, &wrong, &RING, &req).unwrap();
        assert_eq!(
            moon.finish(&resp).err(),
            Some(PairingError::KeyConfirmation)
        );
        assert!(run(&wrong).is_err());
    }

    #[test]
    fn wrong_ring_identity_fails() {
        let p = payload();
        let (moon, req) = MoonHandshake::start(&p, [0x11; 16], UUID, "Pixel 8").unwrap();
        let (_ring, resp) = RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).unwrap();
        assert_eq!(
            moon.finish(&resp).err(),
            Some(PairingError::KeyConfirmation)
        );
    }

    #[test]
    fn forged_moon_confirm_is_rejected() {
        let p = payload();
        let (_moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8").unwrap();
        let (ring, _) = RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).unwrap();
        let forged = PairConfirmRequest {
            moon_confirm: b64_encode(&[0u8; 32]),
        };
        assert_eq!(
            ring.verify_confirm(&forged),
            Err(PairingError::KeyConfirmation)
        );
    }

    #[test]
    fn certificate_cross_check() {
        let p = payload();
        let (moon, ring) = {
            let (moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8").unwrap();
            let (ring, resp) =
                RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).unwrap();
            (moon.finish(&resp).unwrap(), ring)
        };
        let sealed = ring.seal(&result()).unwrap();
        assert_eq!(
            moon.open_result(&sealed, &[0u8; 32]),
            Err(PairingError::CertificateMismatch)
        );
    }

    #[test]
    fn tampered_or_foreign_result_does_not_open() {
        let p = payload();
        let (moon, ring) = {
            let (moon, req) = MoonHandshake::start(&p, RING, UUID, "Pixel 8").unwrap();
            let (ring, resp) =
                RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).unwrap();
            (moon.finish(&resp).unwrap(), ring)
        };
        let mut raw = b64_decode(&ring.seal(&result()).unwrap().sealed).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        let tampered = PairResultResponse {
            sealed: b64_encode(&raw),
        };
        assert_eq!(moon.open_result(&tampered, &CERT), Err(PairingError::Open));

        // A result sealed in a different session (same token, fresh keys).
        let q = payload();
        let (_, req2) = MoonHandshake::start(&q, RING, UUID, "Pixel 8").unwrap();
        let (other_ring, _) =
            RingHandshake::respond(&q.pairing_id, &q.token, &RING, &req2).unwrap();
        let foreign = other_ring.seal(&result()).unwrap();
        assert_eq!(moon.open_result(&foreign, &CERT), Err(PairingError::Open));
    }

    #[test]
    fn rejects_bad_names_and_messages() {
        let p = payload();
        assert!(MoonHandshake::start(&p, RING, UUID, "").is_err());
        assert!(MoonHandshake::start(&p, RING, UUID, &"x".repeat(65)).is_err());
        assert!(MoonHandshake::start(&p, RING, UUID, "a\nb").is_err());
        let (_, mut req) = MoonHandshake::start(&p, RING, UUID, "ok").unwrap();
        req.spake_msg = b64_encode(b"short");
        assert!(RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).is_err());
        req.device_uuid = "nothex".into();
        assert!(RingHandshake::respond(&p.pairing_id, &p.token, &RING, &req).is_err());
    }
}
