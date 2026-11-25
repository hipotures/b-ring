//! `bring pair --sound`: plays the acoustic pairing frame
//! (`bring_core::pairing::acoustic`) on repeat until a Moon picks it up.
//!
//! Playback goes through whatever player the desktop has (`pw-play`,
//! `paplay`, `aplay`, `ffplay`), so the CLI links no audio library. The WAV
//! holds the one-time token, so it lives in a `0600` file inside the
//! `0700` runtime directory and is deleted when the beacon stops.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Pause between two frames (a frame lasts about 2 s).
const GAP: Duration = Duration::from_millis(1200);

/// Candidate players, best first, with the arguments that play one file.
const PLAYERS: &[(&str, &[&str])] = &[
    ("pw-play", &[]),
    ("paplay", &[]),
    ("aplay", &["-q"]),
    ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
];

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| fs::metadata(d.join(bin)).is_ok_and(|m| m.is_file()))
    })
}

/// The first available player, or an explanation.
pub fn find_player() -> Result<(&'static str, &'static [&'static str]), String> {
    PLAYERS
        .iter()
        .copied()
        .find(|(bin, _)| on_path(bin))
        .ok_or_else(|| {
            "no audio player found (install pipewire-bin, pulseaudio-utils, alsa-utils or ffmpeg), \
             or use --wav FILE and play it elsewhere"
                .into()
        })
}

/// Writes `data` to `path` readable only by the user.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

fn wav_path() -> std::io::Result<PathBuf> {
    let dir = bring_core::ctl::default_socket_path()
        .parent()
        .map_or_else(std::env::temp_dir, Path::to_path_buf);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    Ok(dir.join(format!("pair-{}.wav", std::process::id())))
}

/// Plays the frame on repeat in a background thread until dropped.
pub struct SoundBeacon {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    wav: PathBuf,
}

impl SoundBeacon {
    pub fn start(wav: &[u8]) -> Result<Self, String> {
        let (player, args) = find_player()?;
        let path = wav_path().map_err(|e| format!("cannot create the sound file: {e}"))?;
        write_private(&path, wav).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            let path = path.clone();
            std::thread::spawn(move || play_loop(player, args, &path, &stop))
        };
        Ok(Self {
            stop,
            thread: Some(thread),
            wav: path,
        })
    }
}

fn play_loop(player: &str, args: &[&str], path: &Path, stop: &AtomicBool) {
    let mut failures = 0;
    while !stop.load(Ordering::Relaxed) {
        let child = Command::new(player)
            .args(args)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let ok = match child {
            Ok(child) => wait_or_kill(child, stop),
            Err(_) => false,
        };
        if !ok {
            failures += 1;
            if failures == 3 {
                eprintln!("bring: `{player}` keeps failing; sound pairing is off");
                return;
            }
        }
        let until = Instant::now() + GAP;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Waits for the player, killing it early when `stop` is raised.
fn wait_or_kill(mut child: Child, stop: &AtomicBool) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if stop.load(Ordering::Relaxed) => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

impl Drop for SoundBeacon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.wav);
    }
}
