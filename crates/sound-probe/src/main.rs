//! `sound-probe`: which frequency bands survive a speaker → microphone path?
//!
//! Experiment for the acoustic pairing channel, kept apart from it. It uses
//! the same frame layout as `bring_core::pairing::acoustic` (48 kHz, 4 bands ×
//! 16 tones at 46.875 Hz spacing, 64 ms symbols, the same preamble), but the
//! band is not fixed:
//!
//! - `tx` plays a sweep: the lowest tone starts at `--from` Hz and moves up by
//!   `--step` Hz every `--dwell` seconds, until `--to`. Each slot repeats the
//!   frame every 2.5 s.
//! - `rx` listens on every band of that sweep at once and prints each frame
//!   it catches: the band, preamble score, byte/nibble errors against the
//!   known content, and the text it decoded. At the end it prints a per-band
//!   summary.
//!
//! The data carries no FEC (the 50 bytes are fixed, known text), so the error
//! counts show the raw channel. The real modem repairs up to 8 bad bytes per
//! frame with Reed–Solomon; the summary marks frames within that limit.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};

const RATE: u32 = 48_000;
const FFT_LEN: usize = 1024;
const BIN_HZ: f32 = RATE as f32 / FFT_LEN as f32;
/// Highest usable bin index (Nyquist).
const NYQUIST_BIN: usize = FFT_LEN / 2;
const BANDS: usize = 4;
const TONES: usize = 16;
const BINS: usize = BANDS * TONES;
const SYMBOL: usize = 3 * FFT_LEN;
const HOP: usize = 256;
const HOPS_PER_SYMBOL: usize = SYMBOL / HOP;
const WINDOW: usize = 2 * FFT_LEN;
const WINDOW_OFFSET_HOPS: usize = 3;
const RAMP: usize = 240;
const LEAD: usize = RATE as usize / 10;
const AMPLITUDE: f32 = 0.2;
const PREAMBLE: [[u8; BANDS]; 4] = [[1, 14, 4, 11], [14, 1, 11, 4], [6, 9, 2, 13], [9, 6, 13, 2]];
const DATA_BYTES: usize = 50;
const DATA_SYMBOLS: usize = DATA_BYTES / 2;
const FRAME_SYMBOLS: usize = PREAMBLE.len() + DATA_SYMBOLS;
const FRAME_SAMPLES: usize = 2 * LEAD + FRAME_SYMBOLS * SYMBOL;
/// One frame every 2.5 s inside a slot.
const FRAME_PERIOD: usize = RATE as usize * 5 / 2;
/// Spectra needed to judge one frame start.
const SPAN: usize = (FRAME_SYMBOLS - 1) * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS + 1;
/// Extra frame starts compared once the preamble matches, to pick the best.
const ALIGN_HOPS: usize = 6;
/// Bad bytes the production Reed–Solomon(50, 34) code still repairs.
const RS_REPAIRABLE: usize = 8;

const _: () = assert!(FRAME_SAMPLES < FRAME_PERIOD);

#[derive(Parser)]
#[command(about = "Sweep the pairing-modem band upward and see what a microphone still hears")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Play the sweep (or write it to a WAV file).
    Tx {
        #[command(flatten)]
        sweep: Sweep,
        /// Write the sweep to this WAV file instead of playing it.
        #[arg(long)]
        wav: Option<PathBuf>,
        /// Play the sweep this many times.
        #[arg(long, default_value_t = 1)]
        repeat: u32,
    },
    /// List the audio inputs PipeWire (or ALSA) can record from.
    Devices,
    /// Listen and print every frame heard on any band of the sweep.
    Rx {
        #[command(flatten)]
        sweep: Sweep,
        /// WAV file to analyse, or `-` for raw s16le mono PCM on stdin.
        /// Without it, records from the default microphone (pw-record / arecord).
        input: Option<String>,
        /// Sample rate of raw PCM on stdin.
        #[arg(long, default_value_t = RATE)]
        rate: u32,
        /// Stop live recording after this many seconds (default: sweep length + 30 s).
        #[arg(long)]
        seconds: Option<u64>,
        /// Preamble score that counts as a frame (production: 0.45; noise ≈ 0.06).
        #[arg(long, default_value_t = 0.45)]
        threshold: f32,
        /// Capture device (see `sound-probe devices`): a PipeWire node name
        /// or id, or an arecord `-D` device. Default: the system's default input.
        #[arg(long)]
        device: Option<String>,
    },
}

#[derive(Args, Clone)]
struct Sweep {
    /// Lowest tone of the first slot, Hz.
    #[arg(long, default_value_t = 2000)]
    from: u32,
    /// Lowest tone of the last slot, Hz (the band spans ~3 kHz above it).
    #[arg(long, default_value_t = 17000)]
    to: u32,
    /// Shift between slots, Hz.
    #[arg(long, default_value_t = 1000)]
    step: u32,
    /// Seconds per slot.
    #[arg(long, default_value_t = 10)]
    dwell: u32,
}

impl Sweep {
    /// First tone bin of each slot.
    fn first_bins(&self) -> Result<Vec<usize>, String> {
        if self.step == 0 || self.from > self.to {
            return Err("need --step > 0 and --from <= --to".into());
        }
        if (self.dwell as usize) * (RATE as usize) < FRAME_PERIOD {
            return Err("--dwell must be at least 3 s".into());
        }
        let bins: Vec<usize> = (self.from..=self.to)
            .step_by(self.step as usize)
            .map(|hz| (hz as f32 / BIN_HZ).round() as usize)
            .collect();
        if let Some(&b) = bins.iter().find(|&&b| b == 0 || b + BINS > NYQUIST_BIN) {
            return Err(format!(
                "band at {:.0} Hz does not fit below {:.0} Hz; lower --to",
                b as f32 * BIN_HZ,
                NYQUIST_BIN as f32 * BIN_HZ
            ));
        }
        Ok(bins)
    }

    fn slot_samples(&self) -> usize {
        self.dwell as usize * RATE as usize
    }
}

fn band_label(first_bin: usize) -> String {
    format!(
        "{:>5.0}–{:>5.0} Hz",
        first_bin as f32 * BIN_HZ,
        (first_bin + BINS - 1) as f32 * BIN_HZ
    )
}

/// The known frame content of a band: printable text, so errors show as typos.
fn expected(first_bin: usize) -> [u8; DATA_BYTES] {
    const FILL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz";
    let head = format!("B-RING {:>5.0}Hz ", first_bin as f32 * BIN_HZ);
    let mut out = [0u8; DATA_BYTES];
    for (i, o) in out.iter_mut().enumerate() {
        *o = head
            .as_bytes()
            .get(i)
            .copied()
            .unwrap_or(FILL[(i + first_bin) % FILL.len()]);
    }
    out
}

fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '·'
            }
        })
        .collect()
}

// ---------------------------------------------------------------- transmit

fn tone_hz(first_bin: usize, band: usize, tone: u8) -> f32 {
    (first_bin + band * TONES + tone as usize) as f32 * BIN_HZ
}

/// Adds one frame (silence lead included) at `out[at..]`.
fn render_frame(first_bin: usize, out: &mut [f32]) {
    let data = expected(first_bin);
    let mut symbols = PREAMBLE.to_vec();
    symbols.extend(
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|&[a, b]| [a >> 4, a & 15, b >> 4, b & 15]),
    );
    for (k, sym) in symbols.iter().enumerate() {
        let start = LEAD + k * SYMBOL;
        for n in 0..SYMBOL {
            let edge = n.min(SYMBOL - 1 - n);
            let env = if edge < RAMP {
                0.5 - 0.5 * (PI * edge as f32 / RAMP as f32).cos()
            } else {
                1.0
            };
            let t = n as f32 / RATE as f32;
            let v: f32 = (0..BANDS)
                .map(|b| (2.0 * PI * tone_hz(first_bin, b, sym[b]) * t + b as f32 * 1.3).sin())
                .sum();
            out[start + n] = AMPLITUDE * env * v;
        }
    }
}

fn render_sweep(sweep: &Sweep, bins: &[usize]) -> Vec<f32> {
    let slot = sweep.slot_samples();
    let mut out = vec![0.0f32; slot * bins.len()];
    for (i, &bin) in bins.iter().enumerate() {
        let mut at = i * slot;
        while at + FRAME_SAMPLES <= (i + 1) * slot {
            render_frame(bin, &mut out[at..at + FRAME_SAMPLES]);
            at += FRAME_PERIOD;
        }
    }
    out
}

fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| fs::metadata(d.join(bin)).is_ok_and(|m| m.is_file()))
    })
}

fn first_available<'a>(list: &'a [(&'a str, &'a [&'a str])]) -> Option<(&'a str, &'a [&'a str])> {
    list.iter().copied().find(|(bin, _)| on_path(bin))
}

fn tx(sweep: &Sweep, wav: Option<&Path>, repeat: u32) -> Result<(), String> {
    let bins = sweep.first_bins()?;
    let bytes = wav_bytes(&render_sweep(sweep, &bins));
    if let Some(path) = wav {
        fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "wrote {} ({} slots × {} s)",
            path.display(),
            bins.len(),
            sweep.dwell
        );
        return Ok(());
    }
    const PLAYERS: &[(&str, &[&str])] = &[
        ("pw-play", &[]),
        ("paplay", &[]),
        ("aplay", &["-q"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ];
    let (player, args) = first_available(PLAYERS)
        .ok_or("no audio player (pw-play, paplay, aplay, ffplay); use --wav FILE")?;
    let path = std::env::temp_dir().join(format!("sound-probe-{}.wav", std::process::id()));
    fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let result = (|| {
        for round in 0..repeat {
            if repeat > 1 {
                println!("--- round {}/{repeat}", round + 1);
            }
            let mut child = Command::new(player)
                .args(args)
                .arg(&path)
                .stdin(Stdio::null())
                .spawn()
                .map_err(|e| format!("{player}: {e}"))?;
            let started = Instant::now();
            for (i, &bin) in bins.iter().enumerate() {
                println!("[{:>4} s] {}", i as u32 * sweep.dwell, band_label(bin));
                let next = started + Duration::from_secs((i as u64 + 1) * sweep.dwell as u64);
                while Instant::now() < next {
                    if let Ok(Some(_)) = child.try_wait() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            let status = child.wait().map_err(|e| format!("{player}: {e}"))?;
            if !status.success() {
                return Err(format!("{player} exited with {status}"));
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(&path);
    result
}

// ----------------------------------------------------------------- receive

/// In-place radix-2 complex FFT.
struct Fft {
    rev: Vec<usize>,
    twiddle: Vec<(f32, f32)>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let shift = usize::BITS - n.trailing_zeros();
        Self {
            rev: (0..n).map(|i| i.reverse_bits() >> shift).collect(),
            twiddle: (0..n / 2)
                .map(|k| {
                    let a = -2.0 * PI * k as f32 / n as f32;
                    (a.cos(), a.sin())
                })
                .collect(),
        }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        for (i, &j) in self.rev.iter().enumerate() {
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            for s in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = self.twiddle[k * step];
                    let (a, b) = (s + k, s + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

/// Linear-interpolation resampler to [`RATE`] (same as the production one).
struct Resampler {
    step: f64,
    pos: f64,
    hist: Vec<f32>,
}

impl Resampler {
    fn new(input_rate: u32) -> Self {
        Self {
            step: input_rate as f64 / RATE as f64,
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

#[derive(Default)]
struct BandStats {
    frames: u32,
    clean: u32,
    repairable: u32,
    byte_errors: u32,
    best_errors: Option<usize>,
}

struct Band {
    first_bin: usize,
    expected: [u8; DATA_BYTES],
    stats: BandStats,
}

struct Receiver {
    resampler: Resampler,
    fft: Fft,
    hann: Vec<f32>,
    samples: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Energy at bins k·46.875 Hz, k = 0..NYQUIST_BIN, one entry per hop.
    spectra: VecDeque<Vec<f32>>,
    /// Index of the next spectrum to be produced.
    produced: u64,
    bands: Vec<Band>,
    threshold: f32,
    /// Spectrum index before which nothing is re-checked (after a hit).
    quiet_until: u64,
}

impl Receiver {
    fn new(input_rate: u32, bins: &[usize], threshold: f32) -> Self {
        Self {
            resampler: Resampler::new(input_rate),
            fft: Fft::new(WINDOW),
            hann: (0..WINDOW)
                .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / WINDOW as f32).cos())
                .collect(),
            samples: Vec::new(),
            re: vec![0.0; WINDOW],
            im: vec![0.0; WINDOW],
            spectra: VecDeque::with_capacity(SPAN + ALIGN_HOPS + 1),
            produced: 0,
            bands: bins
                .iter()
                .map(|&first_bin| Band {
                    first_bin,
                    expected: expected(first_bin),
                    stats: BandStats::default(),
                })
                .collect(),
            threshold,
            quiet_until: 0,
        }
    }

    fn push(&mut self, input: &[f32]) {
        let mut resampled = Vec::with_capacity(input.len() + 8);
        self.resampler.process(input, &mut resampled);
        self.samples.extend_from_slice(&resampled);
        let mut consumed = 0;
        while self.samples.len() - consumed >= WINDOW {
            self.spectrum(consumed);
            consumed += HOP;
            if self.spectra.len() == SPAN + ALIGN_HOPS {
                self.scan();
            }
        }
        self.samples.drain(..consumed);
    }

    fn spectrum(&mut self, start: usize) {
        let window = &self.samples[start..start + WINDOW];
        for ((r, i), (x, w)) in self
            .re
            .iter_mut()
            .zip(self.im.iter_mut())
            .zip(window.iter().zip(&self.hann))
        {
            *r = x * w;
            *i = 0.0;
        }
        self.fft.run(&mut self.re, &mut self.im);
        // Window is 2 × FFT_LEN long, so tone bin k sits at FFT index 2k.
        let energy = (0..NYQUIST_BIN)
            .map(|k| self.re[2 * k].powi(2) + self.im[2 * k].powi(2))
            .collect();
        if self.spectra.len() == SPAN + ALIGN_HOPS {
            self.spectra.pop_front();
        }
        self.spectra.push_back(energy);
        self.produced += 1;
    }

    /// Index (in `produced` numbering) of `self.spectra[0]`.
    fn oldest(&self) -> u64 {
        self.produced - self.spectra.len() as u64
    }

    fn band_energies(&self, start: usize, k: usize, first_bin: usize, band: usize) -> &[f32] {
        let s = &self.spectra[start + k * HOPS_PER_SYMBOL + WINDOW_OFFSET_HOPS];
        let lo = first_bin + band * TONES;
        &s[lo..lo + TONES]
    }

    fn preamble_score(&self, start: usize, first_bin: usize) -> f32 {
        let mut score = 0.0;
        for (k, expected) in PREAMBLE.iter().enumerate() {
            for (b, &tone) in expected.iter().enumerate() {
                let e = self.band_energies(start, k, first_bin, b);
                let total: f32 = e.iter().sum();
                if total > 0.0 {
                    score += e[tone as usize] / total;
                }
            }
        }
        score / (PREAMBLE.len() * BANDS) as f32
    }

    fn decode(&self, start: usize, first_bin: usize) -> [u8; DATA_BYTES] {
        let mut out = [0u8; DATA_BYTES];
        for (i, o) in out.iter_mut().enumerate() {
            let k = PREAMBLE.len() + i / 2;
            let nib = |b: usize| -> u8 {
                let e = self.band_energies(start, k, first_bin, b);
                (0..TONES).fold(0, |best, t| if e[t] > e[best] { t } else { best }) as u8
            };
            let (hi, lo) = if i % 2 == 0 { (0, 1) } else { (2, 3) };
            *o = nib(hi) << 4 | nib(lo);
        }
        out
    }

    /// The transmitter plays one band at a time, and a strong frame can lift
    /// a distant band's preamble score just over the threshold (window
    /// leakage, overlapping bands). So once any band matches, every band and
    /// the next few frame starts are compared, only the best is reported, and
    /// the receiver then stays quiet for one frame.
    fn scan(&mut self) {
        let oldest = self.oldest();
        if oldest < self.quiet_until
            || !self
                .bands
                .iter()
                .any(|b| self.preamble_score(0, b.first_bin) >= self.threshold)
        {
            return;
        }
        let (i, start, score) = (0..self.bands.len())
            .flat_map(|i| (0..=ALIGN_HOPS).map(move |s| (i, s)))
            .map(|(i, s)| (i, s, self.preamble_score(s, self.bands[i].first_bin)))
            .fold((0, 0, f32::MIN), |a, b| if b.2 > a.2 { b } else { a });
        self.quiet_until = oldest + start as u64 + (FRAME_SYMBOLS * HOPS_PER_SYMBOL) as u64;
        {
            let first_bin = self.bands[i].first_bin;
            let got = self.decode(start, first_bin);
            let band = &mut self.bands[i];
            let bytes_bad = got
                .iter()
                .zip(&band.expected)
                .filter(|(a, b)| a != b)
                .count();
            let nibbles_bad: u32 = got
                .iter()
                .zip(&band.expected)
                .map(|(a, b)| ((a ^ b) >> 4 != 0) as u32 + ((a ^ b) & 15 != 0) as u32)
                .sum();
            let st = &mut band.stats;
            st.frames += 1;
            st.clean += (bytes_bad == 0) as u32;
            st.repairable += (bytes_bad <= RS_REPAIRABLE) as u32;
            st.byte_errors += bytes_bad as u32;
            st.best_errors = Some(st.best_errors.map_or(bytes_bad, |b| b.min(bytes_bad)));
            let at = (oldest + start as u64) as f64 * HOP as f64 / RATE as f64;
            println!(
                "[{at:>7.1} s] {}  preamble {score:.2}  bytes {bytes_bad:>2}/{DATA_BYTES}  nibbles {nibbles_bad:>3}/{}  {}  \"{}\"",
                band_label(first_bin),
                2 * DATA_BYTES,
                if bytes_bad == 0 {
                    "OK  "
                } else if bytes_bad <= RS_REPAIRABLE {
                    "RS  "
                } else {
                    "FAIL"
                },
                printable(&got)
            );
        }
    }

    fn summary(&self) {
        println!();
        println!(
            "band               frames  clean  ≤{RS_REPAIRABLE} bad (RS ok)  avg bad bytes  best"
        );
        for band in &self.bands {
            let st = &band.stats;
            if st.frames == 0 {
                println!(
                    "{}       0      -            -              -     -",
                    band_label(band.first_bin)
                );
                continue;
            }
            println!(
                "{}  {:>6}  {:>5}  {:>14}  {:>13.1}  {:>4}",
                band_label(band.first_bin),
                st.frames,
                st.clean,
                st.repairable,
                st.byte_errors as f32 / st.frames as f32,
                st.best_errors.unwrap_or(0)
            );
        }
    }
}

fn pcm16(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
}

/// Mono samples and rate of a 16-bit PCM WAV (channels are averaged).
fn read_wav(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{}: not a WAV file", path.display()));
    }
    let (mut fmt, mut pcm) = (None, None);
    let mut at = 12;
    while at + 8 <= data.len() {
        let id = &data[at..at + 4];
        let len = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &data[at + 8..(at + 8 + len).min(data.len())];
        match id {
            b"fmt " if body.len() >= 16 => fmt = Some(body),
            b"data" => pcm = Some(body),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let fmt = fmt.ok_or("WAV has no fmt chunk")?;
    let pcm = pcm.ok_or("WAV has no data chunk")?;
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let (format, channels, bits) = (u16_at(0), u16_at(2) as usize, u16_at(14));
    let rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    if !(format == 1 || format == 0xFFFE) || bits != 16 || channels == 0 {
        return Err(format!(
            "need 16-bit PCM WAV (got format {format}, {bits} bit); convert with e.g. \
             `ffmpeg -i in -ac 1 -ar 48000 -c:a pcm_s16le out.wav`"
        ));
    }
    let all: Vec<f32> = pcm16(pcm).collect();
    let mono = all
        .chunks_exact(channels)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, rate))
}

fn rx(
    sweep: &Sweep,
    input: Option<&str>,
    rate: u32,
    seconds: Option<u64>,
    threshold: f32,
    device: Option<&str>,
) -> Result<(), String> {
    let bins = sweep.first_bins()?;
    println!(
        "listening on {} bands: {} … {}",
        bins.len(),
        band_label(bins[0]),
        band_label(*bins.last().unwrap())
    );
    match input {
        Some("-") => {
            let mut rcv = Receiver::new(rate, &bins, threshold);
            stream(&mut std::io::stdin().lock(), &mut rcv, None);
            rcv.summary();
        }
        Some(path) => {
            let (samples, rate) = read_wav(Path::new(path))?;
            if !(16_000..=192_000).contains(&rate) {
                return Err(format!("unsupported sample rate {rate}"));
            }
            if rate < 44_100 {
                println!(
                    "note: {rate} Hz input cannot carry tones above {} Hz",
                    rate / 2
                );
            }
            let mut rcv = Receiver::new(rate, &bins, threshold);
            for chunk in samples.chunks(4800) {
                rcv.push(chunk);
            }
            rcv.summary();
        }
        None => {
            let rate_s = RATE.to_string();
            let recorders: [(&str, &[&str]); 2] = [
                (
                    "pw-record",
                    &["--rate", &rate_s, "--channels", "1", "--format", "s16"],
                ),
                (
                    "arecord",
                    &["-q", "-f", "S16_LE", "-r", &rate_s, "-c", "1", "-t", "raw"],
                ),
            ];
            let (bin, args) = first_available(&recorders)
                .ok_or("no recorder (pw-record, arecord); record a WAV and pass it as INPUT")?;
            let mut args: Vec<&str> = args.to_vec();
            let picked;
            let device = if bin == "pw-record" {
                picked = pick_pipewire_source(device)?;
                picked.as_deref()
            } else {
                device
            };
            if let Some(dev) = device {
                args.extend([if bin == "pw-record" { "--target" } else { "-D" }, dev]);
            }
            if bin == "pw-record" {
                args.push("-");
            }
            let limit =
                Duration::from_secs(seconds.unwrap_or(bins.len() as u64 * sweep.dwell as u64 + 30));
            println!(
                "recording with {bin} for {} s (Ctrl+C stops without summary)",
                limit.as_secs()
            );
            let mut child: Child = Command::new(bin)
                .args(args)
                .stdout(Stdio::piped())
                .stdin(Stdio::null())
                .spawn()
                .map_err(|e| format!("{bin}: {e}"))?;
            let mut out = child.stdout.take().unwrap();
            let mut rcv = Receiver::new(RATE, &bins, threshold);
            let deadline = Instant::now() + limit;
            let got = stream(&mut out, &mut rcv, Some(deadline));
            let early = Instant::now() < deadline;
            let _ = child.kill();
            let _ = child.wait();
            if got == 0 {
                return Err(format!(
                    "{bin} delivered no audio from {}; `sound-probe devices` lists the inputs",
                    device.unwrap_or("the default input")
                ));
            }
            if early {
                eprintln!("sound-probe: {bin} stopped early; summary covers what was heard");
            }
            rcv.summary();
        }
    }
    Ok(())
}

/// An audio input as PipeWire reports it.
struct Source {
    id: u64,
    name: String,
    description: String,
    default: bool,
}

/// Audio inputs (`media.class = Audio/Source`) from `pw-dump`, or `None`
/// when PipeWire cannot be queried.
fn pipewire_sources() -> Option<Vec<Source>> {
    let out = Command::new("pw-dump")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_sources(&out.stdout)
}

fn parse_sources(pw_dump: &[u8]) -> Option<Vec<Source>> {
    let objects: Vec<serde_json::Value> = serde_json::from_slice(pw_dump).ok()?;
    let default_name = objects
        .iter()
        .filter(|o| {
            o["type"] == "PipeWire:Interface:Metadata" && o["props"]["metadata.name"] == "default"
        })
        .flat_map(|o| o["metadata"].as_array().into_iter().flatten())
        .find(|m| m["key"] == "default.audio.source")
        .and_then(|m| m["value"]["name"].as_str().map(str::to_owned));
    Some(
        objects
            .iter()
            .filter(|o| {
                o["type"] == "PipeWire:Interface:Node"
                    && o["info"]["props"]["media.class"] == "Audio/Source"
            })
            .map(|o| {
                let props = &o["info"]["props"];
                let name = props["node.name"].as_str().unwrap_or("?").to_owned();
                Source {
                    id: o["id"].as_u64().unwrap_or(0),
                    description: props["node.description"]
                        .as_str()
                        .or(props["node.nick"].as_str())
                        .unwrap_or(&name)
                        .to_owned(),
                    default: default_name.as_deref() == Some(name.as_str()),
                    name,
                }
            })
            .collect(),
    )
}

/// The PipeWire node to record from: `wanted` if it exists, else the default
/// input, else the only one. Explains what is available when that fails.
fn pick_pipewire_source(wanted: Option<&str>) -> Result<Option<String>, String> {
    let Some(sources) = pipewire_sources() else {
        // pw-dump unavailable: let pw-record decide.
        return Ok(wanted.map(str::to_owned));
    };
    if sources.is_empty() {
        return Err(
            "PipeWire reports no audio input at all (no Audio/Source node): \
                    this machine has no microphone it can see. Plug one in, or record \
                    on another device and pass the WAV file as INPUT"
                .into(),
        );
    }
    let chosen = match wanted {
        Some(w) => sources
            .iter()
            .find(|s| s.name == w || s.id.to_string() == w || s.description == w)
            .ok_or_else(|| {
                format!(
                    "no audio input called `{w}`; available:\n{}",
                    list(&sources)
                )
            })?,
        None => sources.iter().find(|s| s.default).unwrap_or(&sources[0]),
    };
    println!("recording from: {} ({})", chosen.description, chosen.name);
    Ok(Some(chosen.name.clone()))
}

fn list(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|s| {
            format!(
                "{} {:>4}  {}  ({})",
                if s.default { "*" } else { " " },
                s.id,
                s.description,
                s.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn devices() -> Result<(), String> {
    match pipewire_sources() {
        Some(s) if s.is_empty() => println!("PipeWire: no audio inputs."),
        Some(s) => println!("PipeWire audio inputs (* = default):\n{}", list(&s)),
        None => {
            let out = Command::new("arecord")
                .arg("-l")
                .output()
                .map_err(|_| "neither pw-dump nor arecord is available")?;
            print!("{}", String::from_utf8_lossy(&out.stdout));
        }
    }
    Ok(())
}

/// Feeds raw s16le PCM from `src` until EOF or `deadline`; prints the input
/// level every 5 s of audio. Returns the number of bytes read.
fn stream(src: &mut dyn Read, rcv: &mut Receiver, deadline: Option<Instant>) -> u64 {
    const LEVEL_EVERY: u64 = 5 * RATE as u64;
    let mut buf = vec![0u8; 9600];
    let mut carry: Option<u8> = None;
    let (mut total, mut seen, mut peak, mut sq) = (0u64, 0u64, 0f32, 0f64);
    loop {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
        let n = match src.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut bytes = Vec::with_capacity(n + 1);
        bytes.extend(carry.take());
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() % 2 == 1 {
            carry = bytes.pop();
        }
        let samples: Vec<f32> = pcm16(&bytes).collect();
        total += n as u64;
        for &x in &samples {
            peak = peak.max(x.abs());
            sq += (x * x) as f64;
            seen += 1;
            if seen % LEVEL_EVERY == 0 {
                let db = |v: f64| 20.0 * v.max(1e-9).log10();
                println!(
                    "[{:>7.1} s] input level: peak {:>6.1} dBFS, rms {:>6.1} dBFS",
                    seen as f64 / RATE as f64,
                    db(peak as f64),
                    db((sq / LEVEL_EVERY as f64).sqrt())
                );
                (peak, sq) = (0.0, 0.0);
            }
        }
        rcv.push(&samples);
    }
    total
}

fn main() {
    let result = match Cli::parse().cmd {
        Cmd::Tx { sweep, wav, repeat } => tx(&sweep, wav.as_deref(), repeat),
        Cmd::Devices => devices(),
        Cmd::Rx {
            sweep,
            input,
            rate,
            seconds,
            threshold,
            device,
        } => rx(
            &sweep,
            input.as_deref(),
            rate,
            seconds,
            threshold,
            device.as_deref(),
        ),
    };
    if let Err(e) = result {
        eprintln!("sound-probe: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sweep(from: u32, to: u32) -> Sweep {
        Sweep {
            from,
            to,
            step: 1000,
            dwell: 5,
        }
    }

    #[test]
    fn expected_is_printable_and_labelled() {
        let e = expected(256);
        assert!(printable(&e).starts_with("B-RING 12000Hz "));
        assert!(e.iter().all(|b| (0x20..0x7f).contains(b)));
    }

    #[test]
    fn parses_pw_dump_sources() {
        let dump = br#"[
          {"id": 31, "type": "PipeWire:Interface:Node", "info": {"props": {
            "media.class": "Audio/Sink", "node.name": "out"}}},
          {"id": 42, "type": "PipeWire:Interface:Node", "info": {"props": {
            "media.class": "Audio/Source", "node.name": "alsa_input.usb-cam",
            "node.description": "Webcam Mic"}}},
          {"id": 43, "type": "PipeWire:Interface:Node", "info": {"props": {
            "media.class": "Audio/Source", "node.name": "alsa_input.pci"}}},
          {"id": 0, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
           "metadata": [{"subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON",
                         "value": {"name": "alsa_input.pci"}}]}
        ]"#;
        let s = parse_sources(dump).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].id, s[0].description.as_str(), s[0].default),
            (42, "Webcam Mic", false)
        );
        assert_eq!(
            (s[1].description.as_str(), s[1].default),
            ("alsa_input.pci", true)
        );
        assert!(parse_sources(b"[]").unwrap().is_empty());
    }

    #[test]
    fn rejects_bands_above_nyquist() {
        assert!(sweep(2000, 17000).first_bins().is_ok());
        assert!(sweep(2000, 22000).first_bins().is_err());
    }

    #[test]
    fn loopback_hears_every_band_clean() {
        let sw = sweep(2000, 18000);
        let bins = sw.first_bins().unwrap();
        let mut signal = vec![0.0f32; 1234];
        signal.extend(render_sweep(&sw, &bins));
        let mut rcv = Receiver::new(RATE, &bins, 0.45);
        for c in signal.chunks(4096) {
            rcv.push(c);
        }
        for b in &rcv.bands {
            // 5 s slots hold 2 frames each.
            assert_eq!(b.stats.frames, 2, "{}", band_label(b.first_bin));
            assert_eq!(b.stats.clean, 2, "{}", band_label(b.first_bin));
        }
    }

    #[test]
    fn resampled_44k1_capture_still_decodes_high_band() {
        let sw = sweep(17000, 17000);
        let bins = sw.first_bins().unwrap();
        let clean = render_sweep(&sw, &bins);
        let ratio = 44_100.0 / 48_000.0;
        let n = (clean.len() as f64 * ratio) as usize;
        let captured: Vec<f32> = (0..n)
            .map(|i| {
                let p = i as f64 / ratio;
                let j = p as usize;
                let f = (p - j as f64) as f32;
                0.1 * (clean[j] * (1.0 - f) + clean.get(j + 1).copied().unwrap_or(0.0) * f)
            })
            .collect();
        let mut rcv = Receiver::new(44_100, &bins, 0.45);
        for c in captured.chunks(441) {
            rcv.push(c);
        }
        assert!(rcv.bands[0].stats.repairable >= 1);
    }
}
