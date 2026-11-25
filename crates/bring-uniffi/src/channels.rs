//! Pairing channels other than QR / typed text (docs/ARCHITECTURE.md §3.6).
//!
//! Each channel only delivers the bootstrap payload; Kotlin turns what it
//! received into the usual text code and hands that to [`crate::Moon::pair_start`].

use std::sync::Mutex;

use bring_core::pairing::{BootstrapPayload, acoustic, ble};

/// Text code for raw payload bytes (e.g. the BLE characteristic value), or
/// `None` if they are not a valid payload.
#[uniffi::export]
pub fn pairing_code_from_bytes(bytes: Vec<u8>) -> Option<String> {
    BootstrapPayload::from_bytes(&bytes)
        .ok()
        .map(|p| p.to_text())
}

/// GATT service a waiting Ring advertises (`bring pair --ble`).
#[uniffi::export]
pub fn ble_pairing_service_uuid() -> String {
    ble::SERVICE_UUID.to_owned()
}

/// Read-only characteristic holding the 31 payload bytes.
#[uniffi::export]
pub fn ble_pairing_payload_uuid() -> String {
    ble::PAYLOAD_CHAR_UUID.to_owned()
}

/// Microphone receiver for `bring pair --sound`. Feed it `AudioRecord`
/// PCM; it returns the text code once a frame decodes.
#[derive(uniffi::Object)]
pub struct SoundReceiver {
    decoder: Mutex<acoustic::Decoder>,
    level: Mutex<f32>,
}

#[uniffi::export]
impl SoundReceiver {
    /// `sample_rate` of the PCM passed to [`SoundReceiver::push_pcm16`]
    /// (16–192 kHz; 48 kHz avoids resampling). `None` if unsupported.
    #[uniffi::constructor]
    pub fn new(sample_rate: u32) -> Option<std::sync::Arc<Self>> {
        acoustic::Decoder::new(sample_rate).map(|d| {
            std::sync::Arc::new(Self {
                decoder: Mutex::new(d),
                level: Mutex::new(0.0),
            })
        })
    }

    /// 16-bit little-endian mono PCM. Returns the pairing code once heard.
    pub fn push_pcm16(&self, pcm: Vec<u8>) -> Option<String> {
        let (sum, n) = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .fold((0.0f64, 0usize), |(s, n), &b| {
                let v = i16::from_le_bytes(b) as f64 / 32768.0;
                (s + v * v, n + 1)
            });
        if n > 0 {
            *self.level.lock().unwrap_or_else(|e| e.into_inner()) = (sum / n as f64).sqrt() as f32;
        }
        self.decoder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_pcm16(&pcm)
            .map(|p| p.to_text())
    }

    /// RMS of the last chunk in `[0, 1]`, for an input meter.
    pub fn level(&self) -> f32 {
        *self.level.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reset(&self) {
        self.decoder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    fn payload() -> BootstrapPayload {
        BootstrapPayload::generate(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 2), 7443)).unwrap()
    }

    #[test]
    fn bytes_to_code() {
        let p = payload();
        assert_eq!(
            pairing_code_from_bytes(p.to_bytes().to_vec()),
            Some(p.to_text())
        );
        assert_eq!(pairing_code_from_bytes(vec![1, 2, 3]), None);
    }

    #[test]
    fn sound_receiver_returns_the_text_code() {
        let p = payload();
        let wav = acoustic::encode_wav(&p);
        let rx = SoundReceiver::new(48_000).unwrap();
        let got = wav[44..]
            .chunks(3840)
            .find_map(|c| rx.push_pcm16(c.to_vec()));
        assert_eq!(got, Some(p.to_text()));
        assert!(SoundReceiver::new(1_000).is_none());
    }
}
