//! Acoustic pairing channel: the bootstrap payload as a ~1.9 s burst of
//! audible tones (ggwave-style multi-tone FSK with Reed–Solomon FEC).
//!
//! The Ring only plays sound and the Moon only listens, so Linux needs no
//! microphone and the Moon needs no speaker. Both ends share this module: the
//! CLI renders samples with [`encode`], Android feeds microphone PCM into a
//! [`Decoder`] through `bring-uniffi`.
//!
//! # Signal
//!
//! - 48 kHz, tone spacing 46.875 Hz (48000 / 1024).
//! - 4 parallel bands × 16 tones in 1875–4828 Hz: each symbol carries one
//!   nibble per band, i.e. 2 bytes. The audible band survives phone speakers,
//!   laptop speakers and voice-tuned microphones far better than ultrasound.
//! - Symbol = 3072 samples (64 ms) with 5 ms raised-cosine edges. The decoder
//!   looks at a 2048-sample Hann window that skips the first 16 ms of each
//!   symbol, so echoes of the previous symbol mostly fall outside it.
//! - Frame = 4 preamble symbols (a fixed tone pattern used for timing) +
//!   25 data symbols = 1.86 s, plus 100 ms of silence on both ends.
//! - Data = Reed–Solomon(50, 34) over `[0xB1][payload 31 B][CRC-16]`: up to
//!   8 bad bytes (≈4 bad symbols) are repaired; the CRC rejects the rare
//!   miscorrection.
//!
//! # Decoding
//!
//! The decoder is amplitude-independent (each band's winner is judged
//! relative to that band's total energy), so no automatic gain is needed;
//! Android should in fact disable AGC, noise suppression and echo
//! cancellation, which smear tones. Every 256 samples it computes the 64 tone
//! energies of one window (Goertzel), scores the preamble pattern at the
//! oldest candidate frame start, and tries a full decode where it matches.

use std::collections::VecDeque;
use std::f32::consts::PI;

use super::payload::{BootstrapPayload, PAYLOAD_LEN};
use super::rs;

/// Sample rate of [`encode`]'s output and of the decoder's internal signal.
pub const SAMPLE_RATE: u32 = 48_000;
/// Input sample rates [`Decoder::new`] accepts.
pub const INPUT_RATES: std::ops::RangeInclusive<u32> = 16_000..=192_000;

const FFT_LEN: usize = 1024;
const FIRST_BIN: usize = 40;
const BANDS: usize = 4;
const TONES: usize = 16;
const BINS: usize = BANDS * TONES;
const SYMBOL: usize = 3 * FFT_LEN;
const HOP: usize = 256;
const HOPS_PER_SYMBOL: usize = SYMBOL / HOP;
const WINDOW: usize = 2 * FFT_LEN;
const WINDOW_OFFSET_HOPS: usize = 3;
const RAMP: usize = 240;
const LEAD: usize = SAMPLE_RATE as usize / 10;
const AMPLITUDE: f32 = 0.2;

const PREAMBLE: [[u8; BANDS]; 4] = [[1, 14, 4, 11], [14, 1, 11, 4], [6, 9, 2, 13], [9, 6, 13, 2]];
/// Mean in-band energy share of the expected preamble tones that marks a
/// candidate frame start (noise scores about 1/16).
const PREAMBLE_THRESHOLD: f32 = 0.45;

const FRAME_MAGIC: u8 = 0xB1;
const MSG_LEN: usize = 1 + PAYLOAD_LEN + 2;
const PARITY: usize = 16;
const BLOCK_LEN: usize = MSG_LEN + PARITY;
const DATA_SYMBOLS: usize = BLOCK_LEN / 2;
const FRAME_SYMBOLS: usize = PREAMBLE.len() + DATA_SYMBOLS;
/// Spectra needed to judge one frame start.
const SPAN: usize = (FRAME_SYMBOLS - 1) * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS + 1;

const _: () = assert!(BLOCK_LEN.is_multiple_of(2) && SYMBOL.is_multiple_of(HOP) && WINDOW < SYMBOL);

/// Frame length in samples at [`SAMPLE_RATE`], silence included.
pub const FRAME_SAMPLES: usize = 2 * LEAD + FRAME_SYMBOLS * SYMBOL;

/// CRC-16/CCITT-FALSE.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn tone_hz(band: usize, tone: u8) -> f32 {
    (FIRST_BIN + band * TONES + tone as usize) as f32 * SAMPLE_RATE as f32 / FFT_LEN as f32
}

fn frame_symbols(payload: &BootstrapPayload) -> Vec<[u8; BANDS]> {
    let mut msg = Vec::with_capacity(MSG_LEN);
    msg.push(FRAME_MAGIC);
    msg.extend_from_slice(&payload.to_bytes());
    msg.extend_from_slice(&crc16(&msg).to_be_bytes());
    let block = rs::encode(&msg, PARITY);
    let mut symbols = PREAMBLE.to_vec();
    symbols.extend(
        block
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[a, b]| [a >> 4, a & 15, b >> 4, b & 15]),
    );
    symbols
}

/// Renders one frame as mono `f32` samples at [`SAMPLE_RATE`] (peak < 0.85).
pub fn encode(payload: &BootstrapPayload) -> Vec<f32> {
    let mut out = vec![0.0f32; FRAME_SAMPLES];
    for (k, sym) in frame_symbols(payload).iter().enumerate() {
        let start = LEAD + k * SYMBOL;
        for n in 0..SYMBOL {
            let edge = n.min(SYMBOL - 1 - n);
            let env = if edge < RAMP {
                0.5 - 0.5 * (PI * edge as f32 / RAMP as f32).cos()
            } else {
                1.0
            };
            let t = n as f32 / SAMPLE_RATE as f32;
            let v: f32 = (0..BANDS)
                .map(|b| (2.0 * PI * tone_hz(b, sym[b]) * t + b as f32 * 1.3).sin())
                .sum();
            out[start + n] = AMPLITUDE * env * v;
        }
    }
    out
}

/// One frame as a 16-bit PCM mono WAV file.
pub fn encode_wav(payload: &BootstrapPayload) -> Vec<u8> {
    let samples = encode(payload);
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    w.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

/// Streaming linear-interpolation resampler to [`SAMPLE_RATE`].
struct Resampler {
    step: f64,
    pos: f64,
    hist: Vec<f32>,
}

impl Resampler {
    fn new(input_rate: u32) -> Self {
        Self {
            step: input_rate as f64 / SAMPLE_RATE as f64,
            pos: 0.0,
            hist: Vec::new(),
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.step == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        self.hist.extend_from_slice(input);
        while self.pos + 1.0 < self.hist.len() as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            out.push(self.hist[i] * (1.0 - f) + self.hist[i + 1] * f);
            self.pos += self.step;
        }
        let used = (self.pos as usize).min(self.hist.len());
        self.hist.drain(..used);
        self.pos -= used as f64;
    }
}

type Spectrum = [f32; BINS];

/// Streaming receiver: push microphone samples, get a payload back once a
/// frame decodes. Cheap enough for real time on a phone (64 Goertzel filters
/// over 2048 samples every 256 samples).
pub struct Decoder {
    resampler: Resampler,
    samples: Vec<f32>,
    scratch: Vec<f32>,
    spectra: VecDeque<Spectrum>,
    hann: Vec<f32>,
    coeffs: [f32; BINS],
}

impl Decoder {
    /// `sample_rate` is the rate of the samples passed to [`Decoder::push`];
    /// it must lie in [`INPUT_RATES`].
    pub fn new(sample_rate: u32) -> Option<Self> {
        if !INPUT_RATES.contains(&sample_rate) {
            return None;
        }
        let hann = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / WINDOW as f32).cos())
            .collect();
        let mut coeffs = [0.0f32; BINS];
        for (i, c) in coeffs.iter_mut().enumerate() {
            let k = (FIRST_BIN + i) as f32 * (WINDOW / FFT_LEN) as f32;
            *c = 2.0 * (2.0 * PI * k / WINDOW as f32).cos();
        }
        Some(Self {
            resampler: Resampler::new(sample_rate),
            samples: Vec::with_capacity(WINDOW + 8192),
            scratch: Vec::with_capacity(WINDOW),
            spectra: VecDeque::with_capacity(SPAN + 1),
            hann,
            coeffs,
        })
    }

    /// Forgets everything heard so far.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.spectra.clear();
        self.resampler.hist.clear();
        self.resampler.pos = 0.0;
    }

    /// Feeds 16-bit little-endian PCM (Android `AudioRecord` bytes).
    pub fn push_pcm16(&mut self, bytes: &[u8]) -> Option<BootstrapPayload> {
        let samples: Vec<f32> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
            .collect();
        self.push(&samples)
    }

    /// Feeds mono samples in `[-1, 1]`; returns the payload once a frame
    /// decodes (the decoder then starts over).
    pub fn push(&mut self, input: &[f32]) -> Option<BootstrapPayload> {
        let mut resampled = Vec::with_capacity(input.len() + 8);
        self.resampler.process(input, &mut resampled);
        self.samples.extend_from_slice(&resampled);
        let mut found = None;
        let mut consumed = 0;
        while self.samples.len() - consumed >= WINDOW {
            let spectrum = self.spectrum(consumed);
            consumed += HOP;
            self.spectra.push_back(spectrum);
            if self.spectra.len() > SPAN {
                self.spectra.pop_front();
            }
            if found.is_none()
                && self.spectra.len() == SPAN
                && let Some(p) = self.try_frame()
            {
                found = Some(p);
                self.spectra.clear();
            }
        }
        self.samples.drain(..consumed);
        found
    }

    /// Tone energies of the window starting at `self.samples[start]`.
    fn spectrum(&mut self, start: usize) -> Spectrum {
        let window = &self.samples[start..start + WINDOW];
        self.scratch.clear();
        self.scratch
            .extend(window.iter().zip(&self.hann).map(|(x, w)| x * w));
        let mut out = [0.0f32; BINS];
        for (o, &c) in out.iter_mut().zip(&self.coeffs) {
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &x in &self.scratch {
                let s0 = x + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            *o = s1 * s1 + s2 * s2 - c * s1 * s2;
        }
        out
    }

    fn symbol(&self, k: usize) -> &Spectrum {
        &self.spectra[k * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS]
    }

    fn try_frame(&self) -> Option<BootstrapPayload> {
        let mut score = 0.0;
        for (k, expected) in PREAMBLE.iter().enumerate() {
            let s = self.symbol(k);
            for (b, &tone) in expected.iter().enumerate() {
                let band = &s[b * TONES..(b + 1) * TONES];
                let total: f32 = band.iter().sum();
                if total > 0.0 {
                    score += band[tone as usize] / total;
                }
            }
        }
        if score / ((PREAMBLE.len() * BANDS) as f32) < PREAMBLE_THRESHOLD {
            return None;
        }
        let mut block = Vec::with_capacity(BLOCK_LEN);
        for k in PREAMBLE.len()..FRAME_SYMBOLS {
            let s = self.symbol(k);
            let nib = |b: usize| -> u8 {
                let band = &s[b * TONES..(b + 1) * TONES];
                let mut best = 0;
                for t in 1..TONES {
                    if band[t] > band[best] {
                        best = t;
                    }
                }
                best as u8
            };
            block.push(nib(0) << 4 | nib(1));
            block.push(nib(2) << 4 | nib(3));
        }
        let msg = rs::decode(&block, PARITY)?;
        let (body, crc) = msg.split_at(MSG_LEN - 2);
        if body[0] != FRAME_MAGIC || crc16(body).to_be_bytes() != crc {
            return None;
        }
        BootstrapPayload::from_bytes(&body[1..]).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::{OneTimeToken, PairingId};
    use std::net::{Ipv4Addr, SocketAddrV4};

    fn sample() -> BootstrapPayload {
        BootstrapPayload {
            pairing_id: PairingId([9, 8, 7, 6, 5, 4, 3, 2]),
            token: OneTimeToken([0x5C; 16]),
            addr: SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 23), 7443),
        }
    }

    /// Deterministic noise in [-1, 1] (xorshift).
    fn noise(seed: u64, n: usize) -> Vec<f32> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
            })
            .collect()
    }

    fn feed(dec: &mut Decoder, signal: &[f32], chunk: usize) -> Option<BootstrapPayload> {
        signal.chunks(chunk).find_map(|c| dec.push(c))
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn frame_shape() {
        let s = encode(&sample());
        assert_eq!(s.len(), FRAME_SAMPLES);
        assert!((1.8..2.1).contains(&(s.len() as f32 / SAMPLE_RATE as f32)));
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.85, "peak {peak}");
        assert!(tone_hz(0, 0) > 1800.0 && tone_hz(BANDS - 1, 15) < 5000.0);
        let wav = encode_wav(&sample());
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 2 * FRAME_SAMPLES);
    }

    #[test]
    fn clean_signal_with_arbitrary_offset() {
        let mut signal = vec![0.0; 777];
        signal.extend(encode(&sample()));
        let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
        assert_eq!(feed(&mut dec, &signal, 4800), Some(sample()));
    }

    #[test]
    fn pcm16_input() {
        let wav = encode_wav(&sample());
        let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
        let got = wav[44..].chunks(1922).find_map(|c| dec.push_pcm16(c));
        assert_eq!(got, Some(sample()));
    }

    #[test]
    fn noisy_room_with_echo_quiet_and_44k1_with_clock_drift() {
        let clean = encode(&sample());
        // Two echoes: 7 ms at -6 dB, 23 ms at -12 dB.
        let mut room = clean.clone();
        for (delay, gain) in [(336, 0.5), (1104, 0.25)] {
            for i in delay..room.len() {
                room[i] += gain * clean[i - delay];
            }
        }
        // Played at 48 kHz, captured at 44.1 kHz by a clock 150 ppm fast.
        let ratio = 44_100.0 / 48_000.0 * (1.0 + 150e-6);
        let n = (room.len() as f64 * ratio) as usize;
        let mut captured: Vec<f32> = (0..n)
            .map(|i| {
                let p = i as f64 / ratio;
                let j = p as usize;
                let f = (p - j as f64) as f32;
                let a = room[j];
                let b = *room.get(j + 1).unwrap_or(&0.0);
                0.02 * (a * (1.0 - f) + b * f)
            })
            .collect();
        // Broadband noise at roughly the signal's RMS.
        for (c, z) in captured.iter_mut().zip(noise(7, n)) {
            *c += 0.02 * z;
        }
        let mut dec = Decoder::new(44_100).unwrap();
        assert_eq!(feed(&mut dec, &captured, 441), Some(sample()));
    }

    #[test]
    fn burst_of_noise_is_repaired() {
        let mut signal = encode(&sample());
        // 120 ms of loud clatter in the middle of the data.
        let start = LEAD + 12 * SYMBOL;
        for (s, z) in signal[start..start + 5760].iter_mut().zip(noise(3, 5760)) {
            *s += 1.5 * z;
        }
        let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
        assert_eq!(feed(&mut dec, &signal, 1024), Some(sample()));
    }

    #[test]
    fn decodes_again_after_success_and_ignores_noise() {
        let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
        assert_eq!(
            feed(&mut dec, &noise(11, 3 * SAMPLE_RATE as usize), 4096),
            None
        );
        let mut twice = encode(&sample());
        twice.extend(encode(&sample()));
        let hits = twice.chunks(4096).filter_map(|c| dec.push(c)).count();
        assert_eq!(hits, 2);
    }

    #[test]
    fn rejects_unsupported_rates() {
        assert!(Decoder::new(8_000).is_none());
        assert!(Decoder::new(44_100).is_some());
    }
}
