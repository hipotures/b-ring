//! The bootstrap payload: the only data a pairing channel (QR code, manual
//! entry, later sound/BLE) has to carry.
//!
//! Binary layout (31 bytes, big-endian):
//!
//! ```text
//! [ver 1B][pairing_id 8B][token 16B][ipv4 4B][port 2B]
//! ```
//!
//! Text form: Crockford base32 with a check symbol, grouped by 5 characters.
//! `wan_url` and the TLS certificate pin are *not* in the payload; they arrive
//! encrypted at the end of the handshake.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};

use super::base32::{self, Base32Error};

/// Current payload format version.
pub const PAYLOAD_VERSION: u8 = 1;
/// Encoded length in bytes.
pub const PAYLOAD_LEN: usize = 31;
/// Characters per group in the text form.
pub const TEXT_GROUP: usize = 5;

/// Identifies one pairing session on the Ring.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairingId(pub [u8; 8]);

impl PairingId {
    /// Lowercase hex, as used in `/pair/{pairing_id}/...` URLs.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 16 || !s.is_ascii() {
            return None;
        }
        let mut out = [0u8; 8];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl fmt::Debug for PairingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairingId({})", self.to_hex())
    }
}

/// The one-time secret that bootstraps SPAKE2.
#[derive(Clone, PartialEq, Eq)]
pub struct OneTimeToken(pub [u8; 16]);

impl fmt::Debug for OneTimeToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OneTimeToken(<redacted>)")
    }
}

/// Everything a Moon needs to start pairing with a Ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPayload {
    pub pairing_id: PairingId,
    pub token: OneTimeToken,
    /// LAN address of `bringd`'s HTTPS listener.
    pub addr: SocketAddrV4,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    #[error("payload text is not valid: {0}")]
    Text(#[from] Base32Error),
    #[error("payload has wrong length {0} (expected {PAYLOAD_LEN})")]
    Length(usize),
    #[error("unsupported payload version {0}")]
    Version(u8),
}

impl BootstrapPayload {
    /// Creates a payload with a fresh random pairing id and token.
    pub fn generate(addr: SocketAddrV4) -> Result<Self, getrandom::Error> {
        let mut id = [0u8; 8];
        let mut token = [0u8; 16];
        getrandom::fill(&mut id)?;
        getrandom::fill(&mut token)?;
        Ok(Self {
            pairing_id: PairingId(id),
            token: OneTimeToken(token),
            addr,
        })
    }

    pub fn to_bytes(&self) -> [u8; PAYLOAD_LEN] {
        let mut out = [0u8; PAYLOAD_LEN];
        out[0] = PAYLOAD_VERSION;
        out[1..9].copy_from_slice(&self.pairing_id.0);
        out[9..25].copy_from_slice(&self.token.0);
        out[25..29].copy_from_slice(&self.addr.ip().octets());
        out[29..31].copy_from_slice(&self.addr.port().to_be_bytes());
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, PayloadError> {
        if b.len() != PAYLOAD_LEN {
            return Err(PayloadError::Length(b.len()));
        }
        if b[0] != PAYLOAD_VERSION {
            return Err(PayloadError::Version(b[0]));
        }
        let mut id = [0u8; 8];
        id.copy_from_slice(&b[1..9]);
        let mut token = [0u8; 16];
        token.copy_from_slice(&b[9..25]);
        let ip = Ipv4Addr::new(b[25], b[26], b[27], b[28]);
        let port = u16::from_be_bytes([b[29], b[30]]);
        Ok(Self {
            pairing_id: PairingId(id),
            token: OneTimeToken(token),
            addr: SocketAddrV4::new(ip, port),
        })
    }

    /// Text form shown by `bring pair` and carried by the QR code.
    pub fn to_text(&self) -> String {
        base32::encode_grouped(&self.to_bytes(), TEXT_GROUP)
    }

    /// Parses the text form (case-insensitive; hyphens/spaces ignored).
    pub fn from_text(s: &str) -> Result<Self, PayloadError> {
        Self::from_bytes(&base32::decode(s)?)
    }

    /// The LAN base URL, e.g. `https://192.168.1.10:7443`.
    pub fn lan_url(&self) -> String {
        format!("https://{}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> BootstrapPayload {
        BootstrapPayload {
            pairing_id: PairingId([1, 2, 3, 4, 5, 6, 7, 8]),
            token: OneTimeToken([0xAB; 16]),
            addr: SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 7443),
        }
    }

    #[test]
    fn text_shape() {
        let text = sample().to_text();
        // 31 bytes -> 50 data symbols + 1 check = 51 chars, 11 groups.
        assert_eq!(text.replace('-', "").len(), 51);
        assert_eq!(text.split('-').count(), 11);
        assert_eq!(BootstrapPayload::from_text(&text).unwrap(), sample());
        assert_eq!(sample().lan_url(), "https://192.168.1.10:7443");
    }

    #[test]
    fn rejects_wrong_version_and_length() {
        let mut b = sample().to_bytes();
        b[0] = 2;
        assert_eq!(
            BootstrapPayload::from_bytes(&b),
            Err(PayloadError::Version(2))
        );
        assert_eq!(
            BootstrapPayload::from_bytes(&b[..30]),
            Err(PayloadError::Length(30))
        );
        let short = base32::encode_grouped(&[1, 2, 3], 5);
        assert_eq!(
            BootstrapPayload::from_text(&short),
            Err(PayloadError::Length(3))
        );
    }

    #[test]
    fn typo_is_caught_locally() {
        let text = sample().to_text();
        let mut chars: Vec<char> = text.chars().collect();
        chars[3] = if chars[3] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        assert!(matches!(
            BootstrapPayload::from_text(&typo),
            Err(PayloadError::Text(Base32Error::Checksum))
        ));
    }

    #[test]
    fn debug_redacts_token() {
        let dbg = format!("{:?}", sample());
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("171")); // 0xAB
    }

    #[test]
    fn pairing_id_hex() {
        let id = PairingId([0, 1, 0xAB, 0xFF, 9, 10, 11, 12]);
        assert_eq!(id.to_hex(), "0001abff090a0b0c");
        assert_eq!(PairingId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(PairingId::from_hex("xyz"), None);
        assert_eq!(PairingId::from_hex("ąąąąąąąą"), None);
    }

    #[test]
    fn generate_is_random() {
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1);
        let a = BootstrapPayload::generate(addr).unwrap();
        let b = BootstrapPayload::generate(addr).unwrap();
        assert_ne!(a.token, b.token);
        assert_ne!(a.pairing_id, b.pairing_id);
    }

    proptest! {
        #[test]
        fn roundtrip(id in any::<[u8; 8]>(), token in any::<[u8; 16]>(), ip in any::<[u8; 4]>(), port in any::<u16>()) {
            let p = BootstrapPayload {
                pairing_id: PairingId(id),
                token: OneTimeToken(token),
                addr: SocketAddrV4::new(Ipv4Addr::from(ip), port),
            };
            prop_assert_eq!(BootstrapPayload::from_bytes(&p.to_bytes()).unwrap(), p.clone());
            prop_assert_eq!(BootstrapPayload::from_text(&p.to_text()).unwrap(), p.clone());
            prop_assert_eq!(BootstrapPayload::from_text(&p.to_text().to_lowercase()).unwrap(), p);
        }
    }
}
