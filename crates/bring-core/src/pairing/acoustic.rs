//! Acoustic pairing channel: the bootstrap payload as a ~1.9 s burst of
//! tones (ggwave-style multi-tone FSK with Reed–Solomon FEC).
//!
//! The Ring only plays sound and the Moon only listens, so Linux needs no
//! microphone and the Moon needs no speaker. Both ends share this module: the
//! CLI renders samples with [`encode`], Android feeds microphone PCM into a
//! [`Decoder`] through `bring-uniffi`.
//!
//! # Signal
//!
//! - 48 kHz, tone spacing 46.875 Hz (48000 / 1024).
//! - 4 parallel sub-bands × 16 tones, 64 tones (~3 kHz) in all: each symbol
//!   carries one nibble per sub-band, i.e. 2 bytes. Where those 3 kHz sit is
//!   a [`Band`]:
//!   - [`Band::LOW`], 1875–4828 Hz (default): audible, but survives any
//!     speaker and voice-tuned microphone.
//!   - [`Band::HIGH`], 18000–20953 Hz: inaudible to most adults. Field tests
//!     (desktop speaker → Galaxy S20 FE, `crates/sound-probe`) decoded it
//!     from 3 m even 40 dB below full level; the phone's microphone port
//!     resonance peaks right there, and its ADC filter cuts off at ~22 kHz.
//!     Played loud it is felt as pressure, so [`TxParams::high`] plays it
//!     25 dB down and with smooth transitions.
//!   - any other start within [`BAND_HZ_RANGE`] (advanced settings).
//!
//!   The decoder listens on several bands at once (low and high by default),
//!   so only the transmitter has to choose.
//! - Symbol = 3072 samples (64 ms). Classic rendering gates every symbol with
//!   5 ms raised-cosine edges; smooth rendering keeps the phase continuous and
//!   glides between tones over 10 ms around each boundary, fading only at the
//!   frame ends, which keeps clicks out of the audible range. The decoder
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
//! energies of each band for one window (Goertzel), scores the preamble
//! pattern at the oldest candidate frame start, and tries a full decode where
//! it matches.

use std::collections::VecDeque;
use std::f32::consts::PI;

use super::payload::{BootstrapPayload, PAYLOAD_LEN};
use super::rs;

/// Sample rate of [`encode`]'s output and of the decoder's internal signal.
pub const SAMPLE_RATE: u32 = 48_000;
/// Input sample rates [`Decoder::new`] accepts.
pub const INPUT_RATES: std::ops::RangeInclusive<u32> = 16_000..=192_000;
/// Where a [`Band`]'s lowest tone may sit, Hz.
pub const BAND_HZ_RANGE: std::ops::RangeInclusive<u32> = 1_000..=21_000;
/// Width of a band from its lowest to its highest tone, Hz.
pub const BAND_SPAN_HZ: u32 = ((BINS - 1) * SAMPLE_RATE as usize / FFT_LEN) as u32;
/// Allowed [`TxParams::volume_db`].
pub const VOLUME_DB_RANGE: std::ops::RangeInclusive<f32> = -60.0..=0.0;
/// Allowed [`RxParams::threshold`].
pub const THRESHOLD_RANGE: std::ops::RangeInclusive<f32> = 0.2..=0.9;
/// Level of [`TxParams::high`] relative to full level, dB.
pub const HIGH_BAND_VOLUME_DB: f32 = -25.0;

const FFT_LEN: usize = 1024;
const BIN_HZ: f32 = SAMPLE_RATE as f32 / FFT_LEN as f32;
const BANDS: usize = 4;
const TONES: usize = 16;
const BINS: usize = BANDS * TONES;
const SYMBOL: usize = 3 * FFT_LEN;
const HOP: usize = 256;
const HOPS_PER_SYMBOL: usize = SYMBOL / HOP;
const WINDOW: usize = 2 * FFT_LEN;
const WINDOW_OFFSET_HOPS: usize = 3;
const RAMP: usize = 240;
/// Smooth rendering: frequency glide centred on each symbol boundary. Must
/// stay clear of the decoder's window (samples 768..2816 of a symbol).
const GLIDE: usize = 480;
/// Smooth rendering: fade at the frame ends (15 ms).
const FADE: usize = 720;
const LEAD: usize = SAMPLE_RATE as usize / 10;
const AMPLITUDE: f32 = 0.2;

const PREAMBLE: [[u8; BANDS]; 4] = [[1, 14, 4, 11], [14, 1, 11, 4], [6, 9, 2, 13], [9, 6, 13, 2]];
/// Mean in-band energy share of the expected preamble tones that marks a
/// candidate frame start (noise scores about 1/16).
pub const DEFAULT_THRESHOLD: f32 = 0.45;

const FRAME_MAGIC: u8 = 0xB1;
const MSG_LEN: usize = 1 + PAYLOAD_LEN + 2;
const PARITY: usize = 16;
const BLOCK_LEN: usize = MSG_LEN + PARITY;
const DATA_SYMBOLS: usize = BLOCK_LEN / 2;
const FRAME_SYMBOLS: usize = PREAMBLE.len() + DATA_SYMBOLS;
/// Spectra needed to judge one frame start.
const SPAN: usize = (FRAME_SYMBOLS - 1) * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS + 1;

const _: () = assert!(BLOCK_LEN.is_multiple_of(2) && SYMBOL.is_multiple_of(HOP) && WINDOW < SYMBOL);
const _: () = assert!(
    GLIDE / 2 <= WINDOW_OFFSET_HOPS * HOP
        && GLIDE / 2 <= SYMBOL - WINDOW_OFFSET_HOPS * HOP - WINDOW
);

/// Where the 64 tones of a frame sit: 64 consecutive bins of 46.875 Hz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Band {
    first_bin: usize,
}

impl Band {
    /// 1875–4828 Hz: audible, works with any speaker (the default).
    pub const LOW: Self = Self { first_bin: 40 };
    /// 18000–20953 Hz: near-ultrasound, see the module docs.
    pub const HIGH: Self = Self { first_bin: 384 };

    /// The band whose lowest tone is nearest to `hz`, if that lies in
    /// [`BAND_HZ_RANGE`].
    pub fn from_hz(hz: u32) -> Option<Self> {
        BAND_HZ_RANGE.contains(&hz).then(|| Self {
            first_bin: (hz as f32 / BIN_HZ).round() as usize,
        })
    }

    /// Lowest tone, Hz.
    pub fn low_hz(self) -> u32 {
        (self.first_bin as f32 * BIN_HZ).round() as u32
    }

    /// Highest tone, Hz.
    pub fn high_hz(self) -> u32 {
        ((self.first_bin + BINS - 1) as f32 * BIN_HZ).round() as u32
    }

    fn tone_hz(self, band: usize, tone: u8) -> f32 {
        (self.first_bin + band * TONES + tone as usize) as f32 * BIN_HZ
    }
}

impl Default for Band {
    fn default() -> Self {
        Self::LOW
    }
}

impl std::fmt::Display for Band {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}–{} Hz", self.low_hz(), self.high_hz())
    }
}

/// How [`encode_with`] renders a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TxParams {
    pub band: Band,
    /// Continuous-phase glides instead of 5 ms on/off edges per symbol.
    pub smooth: bool,
    /// Level relative to full level, within [`VOLUME_DB_RANGE`] (clamped).
    pub volume_db: f32,
}

impl TxParams {
    /// The low band at full level with classic edges (what [`encode`] plays).
    pub const fn low() -> Self {
        Self {
            band: Band::LOW,
            smooth: false,
            volume_db: 0.0,
        }
    }

    /// The high band, smooth, at [`HIGH_BAND_VOLUME_DB`].
    pub const fn high() -> Self {
        Self {
            band: Band::HIGH,
            smooth: true,
            volume_db: HIGH_BAND_VOLUME_DB,
        }
    }

    fn gain(&self) -> f32 {
        let db = self
            .volume_db
            .clamp(*VOLUME_DB_RANGE.start(), *VOLUME_DB_RANGE.end());
        AMPLITUDE * 10f32.powf(db / 20.0)
    }
}

impl Default for TxParams {
    fn default() -> Self {
        Self::low()
    }
}

/// What [`Decoder::with_params`] listens for.
#[derive(Clone, Debug, PartialEq)]
pub struct RxParams {
    /// Bands searched in parallel (duplicates are ignored).
    pub bands: Vec<Band>,
    /// Preamble score that starts a decode attempt, within
    /// [`THRESHOLD_RANGE`]. Lower finds weaker frames but tries more noise;
    /// the CRC still rejects anything wrong.
    pub threshold: f32,
}

impl Default for RxParams {
    fn default() -> Self {
        Self {
            bands: vec![Band::LOW, Band::HIGH],
            threshold: DEFAULT_THRESHOLD,
        }
    }
}

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

/// Renders one frame in the low band as mono `f32` samples at
/// [`SAMPLE_RATE`] (peak < 0.85).
pub fn encode(payload: &BootstrapPayload) -> Vec<f32> {
    encode_with(payload, &TxParams::low())
}

/// Renders one frame as mono `f32` samples at [`SAMPLE_RATE`].
pub fn encode_with(payload: &BootstrapPayload, params: &TxParams) -> Vec<f32> {
    let symbols = frame_symbols(payload);
    let mut out = vec![0.0f32; FRAME_SAMPLES];
    let body = &mut out[LEAD..LEAD + FRAME_SYMBOLS * SYMBOL];
    if params.smooth {
        render_smooth(&symbols, params.band, params.gain(), body);
    } else {
        render_classic(&symbols, params.band, params.gain(), body);
    }
    out
}

/// Each symbol on its own, gated by raised-cosine edges.
fn render_classic(symbols: &[[u8; BANDS]], band: Band, gain: f32, out: &mut [f32]) {
    for (k, sym) in symbols.iter().enumerate() {
        let start = k * SYMBOL;
        for n in 0..SYMBOL {
            let edge = n.min(SYMBOL - 1 - n);
            let env = if edge < RAMP {
                0.5 - 0.5 * (PI * edge as f32 / RAMP as f32).cos()
            } else {
                1.0
            };
            let t = n as f32 / SAMPLE_RATE as f32;
            let v: f32 = (0..BANDS)
                .map(|b| (2.0 * PI * band.tone_hz(b, sym[b]) * t + b as f32 * 1.3).sin())
                .sum();
            out[start + n] = gain * env * v;
        }
    }
}

/// Continuous phase: each sub-band's tone glides to the next symbol's
/// frequency over [`GLIDE`] samples around the boundary, so the waveform
/// never jumps; only the frame ends fade.
fn render_smooth(symbols: &[[u8; BANDS]], band: Band, gain: f32, out: &mut [f32]) {
    use std::f64::consts::TAU;
    let half = GLIDE / 2;
    let freq = |k: usize, b: usize| band.tone_hz(b, symbols[k][b]) as f64;
    let glide = |from: f64, to: f64, x: f64| {
        from + (to - from) * (0.5 - 0.5 * (std::f64::consts::PI * x).cos())
    };
    let mut phase = [0.0f64, 1.3, 2.6, 3.9];
    let len = out.len();
    for (n, o) in out.iter_mut().enumerate() {
        let (k, pos) = (n / SYMBOL, n % SYMBOL);
        let edge = n.min(len - 1 - n);
        let env = if edge < FADE {
            0.5 - 0.5 * (PI * edge as f32 / FADE as f32).cos()
        } else {
            1.0
        };
        let mut v = 0.0f32;
        for (b, ph) in phase.iter_mut().enumerate() {
            let hz = if pos < half && k > 0 {
                glide(
                    freq(k - 1, b),
                    freq(k, b),
                    (pos + half) as f64 / GLIDE as f64,
                )
            } else if pos >= SYMBOL - half && k + 1 < symbols.len() {
                glide(
                    freq(k, b),
                    freq(k + 1, b),
                    (pos - (SYMBOL - half)) as f64 / GLIDE as f64,
                )
            } else {
                freq(k, b)
            };
            v += ph.sin() as f32;
            *ph = (*ph + TAU * hz / SAMPLE_RATE as f64) % TAU;
        }
        *o = gain * env * v;
    }
}

/// One low-band frame as a 16-bit PCM mono WAV file.
pub fn encode_wav(payload: &BootstrapPayload) -> Vec<u8> {
    encode_wav_with(payload, &TxParams::low())
}

/// One frame as a 16-bit PCM mono WAV file.
pub fn encode_wav_with(payload: &BootstrapPayload, params: &TxParams) -> Vec<u8> {
    let samples = encode_with(payload, params);
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

/// Tone energies of one window: [`BINS`] per listened band, band after band.
type Spectrum = Vec<f32>;

/// Streaming receiver: push microphone samples, get a payload back once a
/// frame decodes. Cheap enough for real time on a phone (64 Goertzel filters
/// per band over 2048 samples every 256 samples).
pub struct Decoder {
    resampler: Resampler,
    samples: Vec<f32>,
    scratch: Vec<f32>,
    spectra: VecDeque<Spectrum>,
    hann: Vec<f32>,
    bands: Vec<Band>,
    coeffs: Vec<f32>,
    threshold: f32,
    /// Highest preamble score per band since [`Decoder::take_scores`].
    peak_scores: Vec<f32>,
}

impl Decoder {
    /// Listens on [`RxParams::default`] (low and high band).
    /// `sample_rate` is the rate of the samples passed to [`Decoder::push`];
    /// it must lie in [`INPUT_RATES`].
    pub fn new(sample_rate: u32) -> Option<Self> {
        Self::with_params(sample_rate, &RxParams::default())
    }

    /// `None` if the rate is outside [`INPUT_RATES`], there is no band, or
    /// the threshold is outside [`THRESHOLD_RANGE`].
    pub fn with_params(sample_rate: u32, params: &RxParams) -> Option<Self> {
        if !INPUT_RATES.contains(&sample_rate)
            || params.bands.is_empty()
            || !THRESHOLD_RANGE.contains(&params.threshold)
        {
            return None;
        }
        let mut bands: Vec<Band> = Vec::with_capacity(params.bands.len());
        for b in &params.bands {
            if !bands.contains(b) {
                bands.push(*b);
            }
        }
        let hann = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / WINDOW as f32).cos())
            .collect();
        let coeffs = bands
            .iter()
            .flat_map(|b| b.first_bin..b.first_bin + BINS)
            .map(|bin| {
                let k = bin as f32 * (WINDOW / FFT_LEN) as f32;
                2.0 * (2.0 * PI * k / WINDOW as f32).cos()
            })
            .collect();
        Some(Self {
            resampler: Resampler::new(sample_rate),
            samples: Vec::with_capacity(WINDOW + 8192),
            scratch: Vec::with_capacity(WINDOW),
            spectra: VecDeque::with_capacity(SPAN + 1),
            hann,
            peak_scores: vec![0.0; bands.len()],
            bands,
            coeffs,
            threshold: params.threshold,
        })
    }

    /// The bands listened on, in order (duplicates removed).
    pub fn bands(&self) -> &[Band] {
        &self.bands
    }

    /// Highest preamble score per band (same order as [`Decoder::bands`])
    /// since the previous call: ~1/16 for noise, 1.0 for a clean frame start.
    /// A diagnostic for tuning; it resets on every call.
    pub fn take_scores(&mut self) -> Vec<f32> {
        let out = self.peak_scores.clone();
        self.peak_scores.iter_mut().for_each(|s| *s = 0.0);
        out
    }

    /// Forgets everything heard so far.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.spectra.clear();
        self.resampler.hist.clear();
        self.resampler.pos = 0.0;
        self.peak_scores.iter_mut().for_each(|s| *s = 0.0);
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
                && let Some(p) = self.try_frames()
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
        let mut out = vec![0.0f32; self.coeffs.len()];
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

    /// The [`BINS`] tone energies of band number `band` in symbol `k`.
    fn symbol(&self, band: usize, k: usize) -> &[f32] {
        &self.spectra[k * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS][band * BINS..(band + 1) * BINS]
    }

    /// Scores every band at the oldest frame start and decodes those that
    /// pass, best score first.
    fn try_frames(&mut self) -> Option<BootstrapPayload> {
        let mut candidates: Vec<(usize, f32)> = Vec::new();
        for band in 0..self.bands.len() {
            let score = self.preamble_score(band);
            let peak = &mut self.peak_scores[band];
            *peak = peak.max(score);
            if score >= self.threshold {
                candidates.push((band, score));
            }
        }
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        candidates.iter().find_map(|&(band, _)| self.decode(band))
    }

    fn preamble_score(&self, band: usize) -> f32 {
        let mut score = 0.0;
        for (k, expected) in PREAMBLE.iter().enumerate() {
            let s = self.symbol(band, k);
            for (b, &tone) in expected.iter().enumerate() {
                let sub = &s[b * TONES..(b + 1) * TONES];
                let total: f32 = sub.iter().sum();
                if total > 0.0 {
                    score += sub[tone as usize] / total;
                }
            }
        }
        score / ((PREAMBLE.len() * BANDS) as f32)
    }

    fn decode(&self, band: usize) -> Option<BootstrapPayload> {
        let mut block = Vec::with_capacity(BLOCK_LEN);
        for k in PREAMBLE.len()..FRAME_SYMBOLS {
            let s = self.symbol(band, k);
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
        assert_eq!((Band::LOW.low_hz(), Band::LOW.high_hz()), (1875, 4828));
        assert_eq!((Band::HIGH.low_hz(), Band::HIGH.high_hz()), (18000, 20953));
        assert_eq!(BAND_SPAN_HZ, 2953);
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

    #[test]
    fn band_from_hz() {
        assert_eq!(Band::from_hz(18_000), Some(Band::HIGH));
        assert_eq!(Band::from_hz(1_875), Some(Band::LOW));
        assert_eq!(Band::from_hz(12_010).unwrap().low_hz(), 12_000);
        assert_eq!(Band::from_hz(21_000).unwrap().high_hz(), 23_953);
        assert!(Band::from_hz(999).is_none() && Band::from_hz(21_001).is_none());
        assert_eq!(Band::HIGH.to_string(), "18000–20953 Hz");
    }

    #[test]
    fn rejects_bad_rx_params() {
        let mut p = RxParams {
            threshold: 0.1,
            ..RxParams::default()
        };
        assert!(Decoder::with_params(48_000, &p).is_none());
        p.threshold = 0.45;
        p.bands.clear();
        assert!(Decoder::with_params(48_000, &p).is_none());
        p.bands = vec![Band::HIGH, Band::HIGH, Band::LOW];
        let d = Decoder::with_params(48_000, &p).unwrap();
        assert_eq!(d.bands(), &[Band::HIGH, Band::LOW]);
    }

    fn high_quiet() -> TxParams {
        TxParams::high()
    }

    #[test]
    fn default_decoder_hears_both_bands_and_every_style() {
        let styles = [
            TxParams::low(),
            TxParams {
                smooth: true,
                ..TxParams::low()
            },
            TxParams {
                smooth: false,
                ..TxParams::high()
            },
            high_quiet(),
        ];
        for params in styles {
            let mut signal = vec![0.0; 1111];
            signal.extend(encode_with(&sample(), &params));
            let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
            assert_eq!(feed(&mut dec, &signal, 4800), Some(sample()), "{params:?}");
        }
    }

    #[test]
    fn high_band_is_quiet_and_clickless() {
        let high = encode_with(&sample(), &high_quiet());
        let peak = high.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // 0.8 × 10^(-25/20) ≈ 0.045
        assert!(peak < 0.05, "peak {peak}");
        // Energy below 15 kHz relative to the whole frame (Goertzel at 1 kHz
        // steps over the steady part): smooth rendering leaves nothing there,
        // classic edges leak far more.
        fn leak(samples: &[f32]) -> f32 {
            let body = &samples[LEAD..samples.len() - LEAD];
            let power = |hz: f32| {
                let c = 2.0 * (2.0 * PI * hz / SAMPLE_RATE as f32).cos();
                let (mut s1, mut s2) = (0.0f32, 0.0f32);
                for &x in body {
                    let s0 = x + c * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                s1 * s1 + s2 * s2 - c * s1 * s2
            };
            let low: f32 = (1..15).map(|k| power(k as f32 * 1000.0)).sum();
            let total: f32 = body.iter().map(|x| x * x).sum::<f32>() * body.len() as f32 / 2.0;
            10.0 * (low / total).log10()
        }
        let classic = leak(&encode_with(
            &sample(),
            &TxParams {
                smooth: false,
                ..high_quiet()
            },
        ));
        let smooth = leak(&encode_with(&sample(), &high_quiet()));
        assert!(
            smooth < classic - 20.0,
            "classic {classic:.1} dB, smooth {smooth:.1} dB"
        );
    }

    #[test]
    fn high_band_through_44k1_capture_with_echo_and_noise() {
        let clean = encode_with(&sample(), &high_quiet());
        let mut room = clean.clone();
        for (delay, gain) in [(336, 0.5), (1104, 0.25)] {
            for i in delay..room.len() {
                room[i] += gain * clean[i - delay];
            }
        }
        let ratio = 44_100.0 / 48_000.0 * (1.0 - 120e-6);
        let n = (room.len() as f64 * ratio) as usize;
        let mut captured: Vec<f32> = (0..n)
            .map(|i| {
                let p = i as f64 / ratio;
                let j = p as usize;
                let f = (p - j as f64) as f32;
                room[j] * (1.0 - f) + room.get(j + 1).copied().unwrap_or(0.0) * f
            })
            .collect();
        // Broadband noise louder than the (quiet) signal.
        for (c, z) in captured.iter_mut().zip(noise(5, n)) {
            *c += 0.03 * z;
        }
        let mut dec = Decoder::new(44_100).unwrap();
        assert_eq!(feed(&mut dec, &captured, 441), Some(sample()));
    }

    #[test]
    fn custom_band_only_and_scores() {
        let band = Band::from_hz(12_000).unwrap();
        let params = TxParams {
            band,
            smooth: true,
            volume_db: -10.0,
        };
        let signal = encode_with(&sample(), &params);
        // A decoder on the default bands does not hear it...
        let mut dec = Decoder::new(SAMPLE_RATE).unwrap();
        assert_eq!(feed(&mut dec, &signal, 4800), None);
        assert!(dec.take_scores().iter().all(|&s| s < DEFAULT_THRESHOLD));
        // ...one listening there does, and its score shows it.
        let rx = RxParams {
            bands: vec![Band::LOW, band],
            threshold: 0.3,
        };
        let mut dec = Decoder::with_params(SAMPLE_RATE, &rx).unwrap();
        let mut scores = vec![0.0f32; 2];
        let mut got = None;
        for c in signal.chunks(4800) {
            got = got.or(dec.push(c));
            for (s, t) in scores.iter_mut().zip(dec.take_scores()) {
                *s = s.max(t);
            }
        }
        assert_eq!(got, Some(sample()));
        // The decode happens at the first frame start over the threshold.
        assert!(scores[1] >= 0.3 && scores[0] < 0.2, "{scores:?}");
    }
}
