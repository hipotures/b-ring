//! Channel-agnostic device pairing.
//!
//! The bootstrap transport only carries a [`payload::BootstrapPayload`]; the
//! cryptographic handshake is independent of how that payload arrived.

pub mod base32;
pub mod handshake;
pub mod payload;

pub use handshake::{MoonHandshake, MoonVerified, PairingError, RingHandshake};
pub use payload::{BootstrapPayload, OneTimeToken, PairingId, PayloadError};
