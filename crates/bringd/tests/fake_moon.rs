//! Step 10a gate: a "fake Moon" — the exact `bring-uniffi` code Android runs —
//! against the real `bringd` binary over HTTPS:
//!
//! pair → upload 150 MiB (interrupted, then resumed) → ACK → `remote-only`
//! (local copy deleted) → stream it back.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bring_core::ctl::{CtlEvent, CtlRequest};
use bring_core::hash::{hash_bytes, hash_file};
use bring_uniffi::{EntryState, Moon, MoonError, Route, TransferListener, is_valid_pairing_code};

const BIG: usize = 150 * 1024 * 1024;
const CHUNK: u64 = 8 * 1024 * 1024;

/// `seq` of a path in the Ring's index (read straight from its database).
fn ring_seq(d: &Path, path: &str, deleted: bool) -> Option<i64> {
    let c = rusqlite::Connection::open_with_flags(
        d.join("data/ring.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    c.query_row(
        "SELECT seq FROM files WHERE path = ?1 AND deleted = ?2",
        rusqlite::params![path, deleted],
        |r| r.get(0),
    )
    .ok()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `bringd` child process; SIGTERMed (and checked) on `stop`, killed on drop.
struct Ringd {
    child: Child,
    log: PathBuf,
}

impl Ringd {
    fn start(d: &Path, port: u16) -> Self {
        let config = d.join("bringd.toml");
        std::fs::write(
            &config,
            format!(
                "ring_root = {:?}\ndata_dir = {:?}\nlan_listen = \"127.0.0.1:{port}\"\n\
                 advertise_ip = \"127.0.0.1\"\nctl_socket = {:?}\ndebounce_ms = 200\n\
                 notifications = false\n",
                d.join("ring"),
                d.join("data"),
                d.join("ctl.sock"),
            ),
        )
        .unwrap();
        let log = d.join("bringd.log");
        let child = Command::new(env!("CARGO_BIN_EXE_bringd"))
            .arg("--config")
            .arg(&config)
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let r = Self { child, log };
        wait_until("bringd ready", || r.log().contains("ready"));
        r
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn stop(mut self) {
        let pid = self.child.id().to_string();
        assert!(
            Command::new("kill")
                .args(["-TERM", &pid])
                .status()
                .unwrap()
                .success()
        );
        let status = self.child.wait().unwrap();
        assert!(status.success(), "bringd exit: {status}\n{}", self.log());
    }
}

impl Drop for Ringd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Blocking control-socket client (what the `bring` CLI does).
struct Ctl {
    r: BufReader<UnixStream>,
    w: UnixStream,
}

impl Ctl {
    fn connect(path: &Path) -> Self {
        let w = UnixStream::connect(path).unwrap();
        w.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        Self {
            r: BufReader::new(w.try_clone().unwrap()),
            w,
        }
    }

    fn send(&mut self, req: &CtlRequest) {
        let mut line = serde_json::to_vec(req).unwrap();
        line.push(b'\n');
        self.w.write_all(&line).unwrap();
    }

    fn recv(&mut self) -> CtlEvent {
        let mut line = String::new();
        self.r.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    }
}

/// Records progress; cancels the sync once `cancel_after` bytes of the big
/// file were sent (simulating Wi-Fi dropping mid-upload).
struct Listener {
    moon: Arc<Moon>,
    cancel_after: Option<u64>,
    seen: Mutex<Vec<(String, u64, u64)>>,
}

impl TransferListener for Listener {
    fn on_upload_progress(&self, ring_path: String, sent: u64, total: u64) {
        if let Some(limit) = self.cancel_after
            && total == BIG as u64
            && sent >= limit
        {
            self.moon.cancel();
        }
        self.seen.lock().unwrap().push((ring_path, sent, total));
    }
}

fn listener(moon: &Arc<Moon>, cancel_after: Option<u64>) -> Arc<Listener> {
    Arc::new(Listener {
        moon: moon.clone(),
        cancel_after,
        seen: Mutex::default(),
    })
}

#[test]
fn fake_moon_full_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    // A file that exists on the desktop before the Moon ever syncs.
    std::fs::create_dir_all(d.join("ring/Download")).unwrap();
    std::fs::write(d.join("ring/Download/desktop.pdf"), b"%PDF-1.7 desktop").unwrap();

    let port = free_port();
    let ringd = Ringd::start(d, port);
    let socket = d.join("ctl.sock");

    // --- Pairing ------------------------------------------------------------
    let moon = Moon::open(d.join("moon.db").to_str().unwrap().into()).unwrap();
    assert!(matches!(moon.run_sync(None), Err(MoonError::NotPaired)));

    let mut cli = Ctl::connect(&socket);
    cli.send(&CtlRequest::Pair);
    let CtlEvent::PairPayload { text, .. } = cli.recv() else {
        panic!("no pairing payload")
    };
    assert!(is_valid_pairing_code(text.clone()));
    assert!(!is_valid_pairing_code(text.replacen(
        |c: char| c.is_ascii_digit(),
        "X",
        1
    )));

    let sas = moon.pair_start(text, "Fake Moon".into()).unwrap();
    let CtlEvent::PairSas { sas: ring_sas, .. } = cli.recv() else {
        panic!("no SAS on the Ring")
    };
    assert_eq!(sas.sas, ring_sas, "both sides show the same code");
    assert!(matches!(
        moon.pair_finish(0),
        Err(MoonError::PairingPending)
    ));
    cli.send(&CtlRequest::PairDecision { approve: true });
    assert!(matches!(cli.recv(), CtlEvent::PairDone { .. }));
    let paired = moon.pair_finish(10).unwrap();
    assert_eq!(paired.ring.device_slug, "fake-moon");

    let mut status = Ctl::connect(&socket);
    status.send(&CtlRequest::Status);
    let CtlEvent::Status(st) = status.recv() else {
        panic!("no status")
    };
    assert_eq!(
        paired.ring.tls_cert_sha256, st.tls_cert_sha256,
        "pin = bringd's cert"
    );
    assert_eq!(paired.ring.ring_id, st.ring_id);

    // A fresh process: the token comes from the Keystore, not from SQLite.
    drop(moon);
    let moon = Moon::open(d.join("moon.db").to_str().unwrap().into()).unwrap();
    assert!(matches!(moon.sync_index(), Err(MoonError::NotPaired)));
    moon.set_device_token(paired.device_token.clone());
    assert!(moon.probe_lan(500).unwrap());
    assert_eq!(moon.select_route(true).unwrap(), Some(Route::Lan));

    // --- Local files ----------------------------------------------------------
    let dcim = d.join("phone/DCIM");
    std::fs::create_dir_all(dcim.join("Camera")).unwrap();
    let mut big = vec![0u8; BIG];
    let mut xof = blake3::Hasher::new()
        .update(b"fake moon video")
        .finalize_xof();
    xof.fill(&mut big);
    let video = dcim.join("Camera/VID_0001.mp4");
    std::fs::write(&video, &big).unwrap();
    let big_hash = hash_bytes(&big);
    drop(big);
    let photo = dcim.join("Camera/IMG_0001.jpg");
    std::fs::write(&photo, b"small photo, stays on the phone").unwrap();

    let folder = moon
        .add_watched_folder(dcim.to_str().unwrap().into(), None)
        .unwrap();
    assert_eq!(
        folder.ring_folder, "fake-moon/DCIM",
        "device-first folder layout"
    );

    // --- Upload, interrupted after 3 chunks ------------------------------------
    let first = listener(&moon, Some(3 * CHUNK));
    let err = moon.run_sync(Some(first.clone())).unwrap_err();
    assert!(matches!(err, MoonError::Cancelled), "{err}");
    assert!(video.exists(), "nothing deleted without an ACK");
    let pending = moon.entries(Some("fake-moon".into())).unwrap();
    let v = pending
        .iter()
        .find(|e| e.ring_path == "fake-moon/DCIM/Camera/VID_0001.mp4")
        .unwrap();
    assert_eq!(v.state, EntryState::LocalOnly);
    assert!(v.upload_pending);
    assert_eq!(
        v.blake3.as_deref(),
        Some(big_hash.to_hex().as_str()),
        "hashed first"
    );

    // --- Resume -------------------------------------------------------------
    let second = listener(&moon, None);
    let t = Instant::now();
    let report = moon.run_sync(Some(second.clone())).unwrap();
    println!("resumed upload + commit: {:?}", t.elapsed());
    assert_eq!(report.outbox.uploaded, 1, "{report:?}");
    assert_eq!(report.outbox.offloaded, 1, "{report:?}");
    assert_eq!((report.outbox.failed, report.outbox.remaining), (0, 0));
    let resumed_from = second
        .seen
        .lock()
        .unwrap()
        .iter()
        .find(|(_, _, total)| *total == BIG as u64)
        .map(|(_, sent, _)| *sent)
        .unwrap();
    assert!(
        resumed_from >= 3 * CHUNK,
        "resumed instead of restarting (first progress at {resumed_from})"
    );

    // --- ACK → remote-only ---------------------------------------------------
    assert!(!video.exists(), "offloaded after a verified ACK");
    assert!(photo.exists(), "small files stay (threshold 100 MiB)");
    let on_ring = d.join("ring/fake-moon/DCIM/Camera/VID_0001.mp4");
    assert_eq!(hash_file(&on_ring).unwrap(), (big_hash, BIG as u64));

    let all = moon.entries(None).unwrap();
    let by_path = |p: &str| all.iter().find(|e| e.ring_path == p).unwrap().clone();
    let v = by_path("fake-moon/DCIM/Camera/VID_0001.mp4");
    assert_eq!(v.state, EntryState::RemoteOnly);
    assert_eq!(v.local_path, None);
    assert!(!v.upload_pending);
    let p = by_path("fake-moon/DCIM/Camera/IMG_0001.jpg");
    assert_eq!(p.state, EntryState::Synced);
    let desk = by_path("Download/desktop.pdf");
    assert_eq!(desk.state, EntryState::RemoteOnly, "desktop files show up");

    // --- Stream it back ------------------------------------------------------
    let file_id = v.remote_file_id.unwrap();
    let mut streamed = blake3::Hasher::new();
    let step = 16 * 1024 * 1024;
    let mut off = 0u64;
    while off < BIG as u64 {
        let len = step.min(BIG as u64 - off);
        let part = moon.read_range(file_id, off, len).unwrap();
        assert_eq!(part.len() as u64, len);
        streamed.update(&part);
        off += len;
    }
    assert_eq!(
        bring_core::Blake3Hash::from(streamed.finalize()),
        big_hash,
        "streamed bytes are the uploaded bytes"
    );
    let tail = moon.read_range(file_id, BIG as u64 - 10, 100).unwrap();
    assert_eq!(tail.len(), 10, "range clipped at EOF");

    let media = moon.media_source(file_id).unwrap();
    assert_eq!(
        media.url,
        format!("https://127.0.0.1:{port}/api/v1/files/{file_id}/content")
    );
    assert_eq!(
        media.authorization,
        format!("Bearer {}", paired.device_token)
    );
    assert_eq!(
        media.tls_cert_sha256.as_deref(),
        Some(st.tls_cert_sha256.as_str())
    );
    let signed = moon.stream_url(file_id).unwrap();
    assert!(signed.starts_with(&format!(
        "https://127.0.0.1:{port}/api/v1/files/{file_id}/content?st="
    )));

    // --- Name clash only the Ring knows about ----------------------------------
    // The Moon queues a share for Inbox/late.txt; meanwhile the desktop
    // creates a different Inbox/late.txt. The Ring keeps both, the Moon's
    // copy gets a conflict name, and the desktop file still shows up.
    let share = d.join("share/late.txt");
    std::fs::create_dir_all(share.parent().unwrap()).unwrap();
    std::fs::write(&share, b"from the phone").unwrap();
    let queued = moon
        .enqueue_upload(share.to_str().unwrap().into(), "Inbox/late.txt".into())
        .unwrap();
    assert_eq!(queued.ring_path, "Inbox/late.txt");
    std::fs::create_dir_all(d.join("ring/Inbox")).unwrap();
    std::fs::write(d.join("ring/Inbox/late.txt"), b"from the desktop").unwrap();
    wait_until("Ring indexes the desktop file", || {
        ring_seq(d, "Inbox/late.txt", false).is_some()
    });
    let r = moon.run_sync(None).unwrap();
    assert_eq!(r.outbox.uploaded, 1, "{r:?}");
    let shared = moon.entry(queued.id).unwrap();
    assert_eq!(shared.ring_path, "Inbox/late (fake-moon).txt");
    assert_eq!(shared.state, EntryState::Synced);
    assert_eq!(
        std::fs::read(d.join("ring/Inbox/late (fake-moon).txt")).unwrap(),
        b"from the phone"
    );
    let desktop = moon.entries(Some("Inbox".into())).unwrap();
    assert!(
        desktop
            .iter()
            .any(|e| e.ring_path == "Inbox/late.txt" && e.state == EntryState::RemoteOnly),
        "desktop file listed after the cursor rewind: {desktop:#?}"
    );
    // A second share with a taken name is renamed on the Moon already.
    let again = moon
        .enqueue_upload(photo.to_str().unwrap().into(), "Inbox/late.txt".into())
        .unwrap();
    assert_eq!(
        again.ring_path, "fake-moon/DCIM/Camera/IMG_0001.jpg",
        "already tracked"
    );
    let share2 = d.join("share/late2.txt");
    std::fs::write(&share2, b"second").unwrap();
    let renamed = moon
        .enqueue_upload(share2.to_str().unwrap().into(), "Inbox/late.txt".into())
        .unwrap();
    assert_eq!(renamed.ring_path, "Inbox/late (fake-moon 2).txt");
    moon.run_sync(None).unwrap();

    // --- Desktop deletes a synced file: the phone keeps its copy (§5.1) -------
    std::fs::remove_file(d.join("ring/fake-moon/DCIM/Camera/IMG_0001.jpg")).unwrap();
    wait_until("Ring tombstones the photo", || {
        ring_seq(d, "fake-moon/DCIM/Camera/IMG_0001.jpg", true).is_some()
    });
    let r = moon.run_sync(None).unwrap();
    assert_eq!(r.outbox, Default::default(), "not uploaded again: {r:?}");
    let p = moon.entry(p.id).unwrap();
    assert_eq!((p.state, p.suppressed), (EntryState::LocalOnly, true));
    assert!(photo.exists());

    // --- Steady state & verify_sync --------------------------------------------
    let again = moon.run_sync(None).unwrap();
    assert_eq!(again.outbox, Default::default(), "nothing left to do");
    assert!(video.metadata().is_err());

    let verified = moon.verify_sync().unwrap();
    assert!(verified.total_entries >= 4, "{verified:?}");
    assert_eq!(verified.queued, 0, "{verified:?}");
    assert_eq!(verified.retried, 0, "{verified:?}");

    // --- Share upload into an arbitrary folder stays remote-only ---------------
    let staged_dir = d.join("share/staged-1");
    std::fs::create_dir_all(&staged_dir).unwrap();
    let staged = staged_dir.join("note.txt");
    std::fs::write(&staged, b"tiny note").unwrap();
    let remote_share = moon
        .enqueue_upload_remote_for(
            st.ring_id.clone(),
            staged.to_str().unwrap().into(),
            "Custom/Sub/note.txt".into(),
        )
        .unwrap();
    let sr = moon.run_sync(None).unwrap();
    assert_eq!((sr.outbox.uploaded, sr.outbox.offloaded), (1, 1), "{sr:?}");
    let after_share = moon.entry(remote_share.id).unwrap();
    assert_eq!(after_share.state, EntryState::RemoteOnly);
    assert_eq!(after_share.local_path, None);
    assert!(!staged.exists(), "staged share copy must be deleted after ACK");
    assert_eq!(
        std::fs::read(d.join("ring/Custom/Sub/note.txt")).unwrap(),
        b"tiny note"
    );

    // --- Device rename from Moon API and from CLI control socket ---------------
    let renamed_ring = moon
        .rename_device_for(st.ring_id.clone(), "Phone One".into())
        .unwrap();
    assert_eq!(renamed_ring.device_slug, "phone-one");
    assert_eq!(renamed_ring.device_id.len(), 32);
    assert!(
        renamed_ring.transferred_files >= 3 && renamed_ring.transferred_bytes >= BIG as u64,
        "cumulative transfer totals must survive offload and deletion: {renamed_ring:?}"
    );
    assert_eq!(moon.ring().unwrap().unwrap().device_slug, "phone-one");
    assert!(
        !d.join("ring/fake-moon").exists()
            && d.join("ring/phone-one/DCIM/Camera/VID_0001.mp4").exists(),
        "device rename must move the Ring's top-level device directory"
    );
    assert_eq!(
        moon.watched_folders().unwrap()[0].ring_folder,
        "phone-one/DCIM"
    );

    let mut ctl = Ctl::connect(&d.join("ctl.sock"));
    ctl.send(&CtlRequest::Rename {
        device: "phone-one".into(),
        name: "Work Phone".into(),
    });
    let CtlEvent::Renamed { device } = ctl.recv() else {
        panic!("expected Renamed event");
    };
    assert_eq!(
        (device.name.as_str(), device.slug.as_str(), device.device_uuid.as_str()),
        ("Work Phone", "work-phone", renamed_ring.device_id.as_str())
    );
    assert!(
        !d.join("ring/phone-one").exists()
            && d.join("ring/work-phone/DCIM/Camera/VID_0001.mp4").exists(),
        "CLI rename must also move the Ring's device directory"
    );
    // Next sync on the Moon picks up the CLI-assigned slug and rewrites its
    // watched folders and entries automatically with zero duplicates.
    moon.run_sync(None).unwrap();
    assert_eq!(moon.ring().unwrap().unwrap().device_slug, "work-phone");
    assert_eq!(
        moon.watched_folders().unwrap()[0].ring_folder,
        "work-phone/DCIM"
    );

    // --- The pin is enforced ------------------------------------------------------
    let pinned = |pin: &[u8]| {
        rusqlite::Connection::open(d.join("moon.db"))
            .unwrap()
            .execute("UPDATE rings SET tls_cert_sha256 = ?1", [pin])
            .unwrap();
    };
    pinned(&[0u8; 32]);
    let err = moon.sync_index().unwrap_err();
    assert!(matches!(err, MoonError::Network { .. }), "{err}");
    assert!(!moon.probe_lan(500).unwrap());
    pinned(&bring_core::codec::hex_decode::<32>(&st.tls_cert_sha256).unwrap());
    moon.sync_index().unwrap();

    ringd.stop();
}
