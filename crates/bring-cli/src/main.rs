//! `bring` — companion CLI for `bringd` (talks over the control socket).

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bring_core::ctl::{self, CtlEvent, CtlRequest, DeviceInfo};
use clap::{Parser, Subcommand};
use qrcode::render::unicode::Dense1x2;
use qrcode::{Color, QrCode};

#[derive(Parser)]
#[command(name = "bring", version = bring_core::VERSION, about = "Control the B-Ring daemon")]
struct Cli {
    /// Control socket of bringd.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the Ring's state.
    Status,
    /// Pair a new Moon (shows a QR code and a code for manual entry).
    Pair,
    /// List paired devices.
    Devices,
    /// Revoke a device by slug or id.
    Revoke { device: String },
    /// Rename a device by slug or id.
    Rename { device: String, name: String },
}

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Conn {
    fn open(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|e| {
            format!(
                "cannot reach bringd at {}: {e}\n(is it running? try `systemctl --user status bringd`)",
                path.display()
            )
        })?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    fn send(&mut self, req: &CtlRequest) -> Result<(), String> {
        let mut line = serde_json::to_vec(req).map_err(|e| e.to_string())?;
        line.push(b'\n');
        self.writer.write_all(&line).map_err(|e| e.to_string())
    }

    fn recv(&mut self) -> Result<CtlEvent, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("bringd closed the connection".into());
        }
        serde_json::from_str(&line).map_err(|e| format!("bad reply from bringd: {e}"))
    }
}

fn fmt_time(secs: Option<i64>) -> String {
    match secs {
        None => "never".into(),
        Some(s) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let ago = now - s;
            match ago {
                ..60 => "just now".into(),
                60..3600 => format!("{} min ago", ago / 60),
                3600..86400 => format!("{} h ago", ago / 3600),
                _ => format!("{} d ago", ago / 86400),
            }
        }
    }
}

fn print_devices(devices: &[DeviceInfo]) {
    if devices.is_empty() {
        println!("No paired devices. Run `bring pair`.");
        return;
    }
    println!(
        "{:<10} {:<20} {:<24} {:<14} STATE",
        "ID", "SLUG", "NAME", "LAST SEEN"
    );
    for d in devices {
        let short_id: String = d.device_uuid.chars().take(8).collect();
        let id_col = if short_id.is_empty() {
            d.id.to_string()
        } else {
            short_id
        };
        println!(
            "{:<10} {:<20} {:<24} {:<14} {}",
            id_col,
            d.slug,
            d.name,
            fmt_time(d.last_seen_at),
            if d.revoked { "revoked" } else { "active" }
        );
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn unexpected(ev: CtlEvent) -> String {
    match ev {
        CtlEvent::Error { message } => message,
        other => format!("unexpected reply: {other:?}"),
    }
}

fn ask_yes_no(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "t" | "tak"
    )
}

/// Modules of light margin around the code (the QR spec asks for 4).
const QUIET_ZONE: usize = 4;

/// Draws the code black on white with explicit ANSI colours, two modules per
/// character cell (`▀`: foreground = upper module, background = lower).
///
/// Plain block characters take the terminal's own colours: on a dark theme
/// they come out inverted, with no light quiet zone, and many scanners
/// (ZXing among them) do not find such a code.
fn qr_ansi(qr: &QrCode) -> String {
    let width = qr.width();
    let colors = qr.to_colors();
    let size = width + 2 * QUIET_ZONE;
    let dark = |x: usize, y: usize| {
        x >= QUIET_ZONE
            && y >= QUIET_ZONE
            && x < QUIET_ZONE + width
            && y < QUIET_ZONE + width
            && colors[(y - QUIET_ZONE) * width + (x - QUIET_ZONE)] == Color::Dark
    };
    let mut out = String::new();
    for y in (0..size).step_by(2) {
        for x in 0..size {
            match (dark(x, y), dark(x, y + 1)) {
                // Solid spaces for matching pairs avoid half-block glyph seams inside modules.
                (true, true) => out.push_str("\x1b[40m "),
                (false, false) => out.push_str("\x1b[107m "),
                (true, false) => out.push_str("\x1b[30;107m▀"),
                (false, true) => out.push_str("\x1b[97;40m▀"),
            }
        }
        out.push_str("\x1b[0m\n");
    }
    out.pop();
    out
}

fn pair(conn: &mut Conn) -> Result<(), String> {
    conn.send(&CtlRequest::Pair)?;
    match conn.recv()? {
        CtlEvent::PairPayload {
            text,
            lan_url,
            expires_in_secs,
        } => {
            let qr = QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
            if std::io::stdout().is_terminal() {
                println!("{}", qr_ansi(&qr));
            } else {
                println!("{}", qr.render::<Dense1x2>().quiet_zone(true).build());
            }
            println!("Scan this with the B-Ring app, or type the code:\n");
            println!("    {text}\n");
            let expires = match expires_in_secs {
                s @ 0..120 => format!("{s} s"),
                s => format!("{} min", s / 60),
            };
            println!("Ring: {lan_url} · expires in {expires}");
            println!("Waiting for a device...");
        }
        other => return Err(unexpected(other)),
    }
    loop {
        match conn.recv()? {
            CtlEvent::PairSas { sas, device_name } => {
                println!("\n\"{device_name}\" wants to pair.");
                println!("Code on this computer:  {}", ctl::format_sas(&sas));
                let ok = ask_yes_no("Does the phone show the same code?");
                conn.send(&CtlRequest::PairDecision { approve: ok })?;
            }
            CtlEvent::PairDone { device } => {
                println!("Paired \"{}\" as `{}`.", device.name, device.slug);
                return Ok(());
            }
            CtlEvent::PairRejected => return Err("pairing rejected".into()),
            CtlEvent::PairExpired => {
                return Err("pairing code expired; run `bring pair` again".into());
            }
            CtlEvent::PairFailed { message } => return Err(format!("pairing failed: {message}")),
            other => return Err(unexpected(other)),
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let socket = cli.socket.unwrap_or_else(ctl::default_socket_path);
    let mut conn = Conn::open(&socket)?;
    match cli.command {
        Command::Status => {
            conn.send(&CtlRequest::Status)?;
            match conn.recv()? {
                CtlEvent::Status(s) => {
                    println!("bringd {}  ring {}", s.version, s.ring_id);
                    println!("Ring dir      {}", s.ring_root);
                    println!("LAN           {}", s.lan_url);
                    println!(
                        "WAN           {}",
                        s.wan_url.as_deref().unwrap_or("(not configured)")
                    );
                    println!("TLS SHA-256   {}", s.tls_cert_sha256);
                    println!(
                        "Files         {} ({}), {} awaiting hash",
                        s.files,
                        human_bytes(s.bytes),
                        s.pending_hashes
                    );
                    println!("Devices       {} active", s.devices);
                    println!("Uploads       {} in progress", s.open_uploads);
                    Ok(())
                }
                other => Err(unexpected(other)),
            }
        }
        Command::Devices => {
            conn.send(&CtlRequest::Devices)?;
            match conn.recv()? {
                CtlEvent::Devices { devices } => {
                    print_devices(&devices);
                    Ok(())
                }
                other => Err(unexpected(other)),
            }
        }
        Command::Revoke { device } => {
            conn.send(&CtlRequest::Revoke { device })?;
            match conn.recv()? {
                CtlEvent::Revoked { device } => {
                    println!("Revoked \"{}\" ({}).", device.name, device.slug);
                    Ok(())
                }
                other => Err(unexpected(other)),
            }
        }
        Command::Rename { device, name } => {
            conn.send(&CtlRequest::Rename { device, name })?;
            match conn.recv()? {
                CtlEvent::Renamed { device } => {
                    println!("Renamed to \"{}\" ({}).", device.name, device.slug);
                    Ok(())
                }
                other => Err(unexpected(other)),
            }
        }
        Command::Pair => pair(&mut conn),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bring: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_is_black_on_white_with_a_light_quiet_zone() {
        let qr = QrCode::new(b"040G4-0R40M-30E25").unwrap();
        let size = qr.width() + 2 * QUIET_ZONE;
        let out = qr_ansi(&qr);
        let rows: Vec<Vec<&str>> = out
            .lines()
            .map(|l| {
                l.strip_suffix("\x1b[0m")
                    .unwrap()
                    .split('\x1b')
                    .skip(1)
                    .collect()
            })
            .collect();
        assert_eq!(rows.len(), size.div_ceil(2));
        assert!(rows.iter().all(|r| r.len() == size));
        // Quiet zone: the first two module rows and the left margin are white.
        assert!(rows[0].iter().all(|c| *c == "[107m "));
        assert!(rows.iter().all(|r| r[0] == "[107m "));
        // Top-left finder pattern starts dark at module (4, 4): row pair 2.
        assert_eq!(rows[2][QUIET_ZONE], "[40m ");
        assert!(out.lines().all(|l| l.ends_with("\x1b[0m")));
    }
}
