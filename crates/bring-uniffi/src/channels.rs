//! Pairing channels other than QR / typed text (docs/ARCHITECTURE.md §3.6).
//!
//! Each channel only delivers the bootstrap payload; Kotlin turns what it
//! received into the usual text code and hands that to [`crate::Moon::pair_start`].

use std::sync::Mutex;

use bring_core::pairing::{BootstrapPayload, acoustic, ble};

use crate::MoonError;

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

/// Bounds and defaults of the sound channel, for the advanced settings UI.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SoundLimits {
    /// Lowest tone of the default (audible) band, Hz.
    pub low_band_hz: u32,
    /// Lowest tone of the high (near-ultrasound) band, Hz.
    pub high_band_hz: u32,
    /// Lowest to highest tone of any band, Hz.
    pub band_span_hz: u32,
    /// Range for a custom band's lowest tone, Hz.
    pub min_band_hz: u32,
    pub max_band_hz: u32,
    pub default_threshold: f32,
    pub min_threshold: f32,
    pub max_threshold: f32,
}

#[uniffi::export]
pub fn sound_limits() -> SoundLimits {
    SoundLimits {
        low_band_hz: acoustic::Band::LOW.low_hz(),
        high_band_hz: acoustic::Band::HIGH.low_hz(),
        band_span_hz: acoustic::BAND_SPAN_HZ,
        min_band_hz: *acoustic::BAND_HZ_RANGE.start(),
        max_band_hz: *acoustic::BAND_HZ_RANGE.end(),
        default_threshold: acoustic::DEFAULT_THRESHOLD,
        min_threshold: *acoustic::THRESHOLD_RANGE.start(),
        max_threshold: *acoustic::THRESHOLD_RANGE.end(),
    }
}

/// What a [`SoundReceiver`] listens for (advanced settings).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SoundListenConfig {
    /// Lowest tone of each band to search, Hz (see [`SoundLimits`]); each is
    /// rounded to the nearest 46.875 Hz step.
    pub bands_hz: Vec<u32>,
    /// Preamble score that starts a decode attempt.
    pub threshold: f32,
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
    /// Listens on the low and the high band. `sample_rate` of the PCM passed
    /// to [`SoundReceiver::push_pcm16`] (16–192 kHz; 48 kHz avoids
    /// resampling and is needed for the high band).
    #[uniffi::constructor]
    pub fn new(sample_rate: u32) -> Result<std::sync::Arc<Self>, MoonError> {
        Self::build(sample_rate, &acoustic::RxParams::default())
    }

    /// Listens as `config` says.
    #[uniffi::constructor]
    pub fn with_config(
        sample_rate: u32,
        config: SoundListenConfig,
    ) -> Result<std::sync::Arc<Self>, MoonError> {
        let bands = config
            .bands_hz
            .iter()
            .map(|&hz| {
                acoustic::Band::from_hz(hz).ok_or_else(|| MoonError::Invalid {
                    detail: format!(
                        "band at {hz} Hz is outside {}–{} Hz",
                        acoustic::BAND_HZ_RANGE.start(),
                        acoustic::BAND_HZ_RANGE.end()
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::build(
            sample_rate,
            &acoustic::RxParams {
                bands,
                threshold: config.threshold,
            },
        )
    }

    /// Lowest tone of each band listened on, Hz, in the order of
    /// [`SoundReceiver::take_scores`].
    pub fn bands_hz(&self) -> Vec<u32> {
        self.lock().bands().iter().map(|b| b.low_hz()).collect()
    }

    /// Highest preamble score per band since the previous call (~0.06 for
    /// noise, 1.0 for a clean frame start), for a diagnostics view.
    pub fn take_scores(&self) -> Vec<f32> {
        self.lock().take_scores()
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
        self.lock().push_pcm16(&pcm).map(|p| p.to_text())
    }

    /// RMS of the last chunk in `[0, 1]`, for an input meter.
    pub fn level(&self) -> f32 {
        *self.level.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reset(&self) {
        self.lock().reset();
    }
}

impl SoundReceiver {
    fn build(
        sample_rate: u32,
        params: &acoustic::RxParams,
    ) -> Result<std::sync::Arc<Self>, MoonError> {
        if !acoustic::INPUT_RATES.contains(&sample_rate) {
            return Err(MoonError::Invalid {
                detail: format!("unsupported sample rate {sample_rate} Hz"),
            });
        }
        let d = acoustic::Decoder::with_params(sample_rate, params).ok_or_else(|| {
            MoonError::Invalid {
                detail: format!(
                    "need at least one band and a threshold within {:?}",
                    acoustic::THRESHOLD_RANGE
                ),
            }
        })?;
        Ok(std::sync::Arc::new(Self {
            decoder: Mutex::new(d),
            level: Mutex::new(0.0),
        }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, acoustic::Decoder> {
        self.decoder.lock().unwrap_or_else(|e| e.into_inner())
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
        assert!(SoundReceiver::new(1_000).is_err());
    }

    #[test]
    fn configured_receiver_hears_only_its_bands() {
        let p = payload();
        let high = acoustic::encode_wav_with(&p, &acoustic::TxParams::high());
        let feed = |rx: &SoundReceiver| {
            high[44..]
                .chunks(3840)
                .find_map(|c| rx.push_pcm16(c.to_vec()))
        };
        assert_eq!(
            feed(&SoundReceiver::new(48_000).unwrap()),
            Some(p.to_text())
        );
        let limits = sound_limits();
        let low_only = SoundReceiver::with_config(
            48_000,
            SoundListenConfig {
                bands_hz: vec![limits.low_band_hz],
                threshold: limits.default_threshold,
            },
        )
        .unwrap();
        assert_eq!(feed(&low_only), None);
        assert_eq!(low_only.bands_hz(), vec![1875]);
        assert_eq!(low_only.take_scores().len(), 1);
        let custom = SoundReceiver::with_config(
            48_000,
            SoundListenConfig {
                bands_hz: vec![17_990, 1875],
                threshold: 0.3,
            },
        )
        .unwrap();
        assert_eq!(custom.bands_hz(), vec![18_000, 1875]);
        let bad = |hz: Vec<u32>, threshold: f32| {
            SoundReceiver::with_config(
                48_000,
                SoundListenConfig {
                    bands_hz: hz,
                    threshold,
                },
            )
            .is_err()
        };
        assert!(bad(vec![], 0.45));
        assert!(bad(vec![500], 0.45));
        assert!(bad(vec![1875], 0.05));
    }
}
