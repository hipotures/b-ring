//! Bluetooth LE pairing channel: identifiers shared by the Ring (BlueZ GATT
//! peripheral in `bring pair --ble`) and the Moon (Android scanner).
//!
//! The Ring advertises [`SERVICE_UUID`] (connectable, no bonding, no OS-level
//! Bluetooth pairing). The Moon connects, reads [`PAYLOAD_CHAR_UUID`] — the
//! 31 raw bytes of the bootstrap payload — and disconnects. Everything after
//! that goes over HTTPS exactly like the QR path. The read is unauthenticated
//! on purpose: like a QR code on screen or a chirp in the room, the payload
//! is only a bootstrap; SPAKE2 plus the SAS comparison on both screens is what
//! keeps a nearby listener out.

/// Primary GATT service of a Ring that is waiting for a Moon.
pub const SERVICE_UUID: &str = "f2f84715-ec17-4848-bcfa-8681209126fa";
/// Read-only characteristic holding [`super::BootstrapPayload::to_bytes`].
pub const PAYLOAD_CHAR_UUID: &str = "f2f84716-ec17-4848-bcfa-8681209126fa";
/// Local name in the advertisement (fits a legacy 31-byte advert next to the
/// 128-bit service UUID and the flags).
pub const LOCAL_NAME: &str = "B-Ring";
