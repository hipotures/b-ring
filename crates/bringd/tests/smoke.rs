//! Step 9 manual smoke test: the real `bringd` binary, end to end.
//!
//! Ignored by default because it shows a real desktop notification:
//!
//! ```sh
//! cargo test -p bringd --test smoke -- --ignored --nocapture
//! ```
//!
//! Starts the binary with a temporary config, pairs a simulated Moon through
//! the control socket and HTTPS, uploads a file, checks it landed in the Ring
//! and the index, waits for the (coalesced) notification, then stops the
//! daemon with SIGTERM and checks it shut down cleanly.

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bring_core::api::{
    CHUNK_HASH_HEADER, CreateUploadRequest, CreateUploadResponse, HealthResponse, IndexResponse,
    PairExchangeResponse, PairResultResponse, UploadAck,
};
use bring_core::codec::hex_decode;
use bring_core::ctl::{CtlEvent, CtlRequest};
use bring_core::hash::hash_bytes;
use bring_core::pairing::{BootstrapPayload, MoonHandshake};
use bring_core::{RelPath, transfer};
use common::{Ctl, capturing_client};
use sha2::{Digest, Sha256};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..100 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

fn log_of(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[tokio::test]
#[ignore = "manual: runs the real binary and shows a desktop notification"]
async fn binary_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let port = free_port();
    let socket = d.join("ctl.sock");
    let config = d.join("bringd.toml");
    std::fs::write(
        &config,
        format!(
            "ring_root = {:?}\ndata_dir = {:?}\nlan_listen = \"127.0.0.1:{port}\"\n\
             advertise_ip = \"127.0.0.1\"\nctl_socket = {:?}\ndebounce_ms = 200\n",
            d.join("ring"),
            d.join("data"),
            socket
        ),
    )
    .unwrap();
    let log_path = d.join("bringd.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bringd"))
        .arg("--config")
        .arg(&config)
        .env("RUST_LOG", "info,bringd=debug")
        .env("NO_COLOR", "1")
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    wait_until("bringd ready", || log_of(&log_path).contains("ready")).await;

    // --- Pair a simulated Moon ------------------------------------------
    let mut cli = Ctl::connect(&socket).await;
    cli.send(&CtlRequest::Pair).await;
    let CtlEvent::PairPayload { text, .. } = cli.recv().await else {
        panic!("no payload")
    };
    let payload = BootstrapPayload::from_text(&text).unwrap();
    let base = payload.lan_url();
    assert_eq!(base, format!("https://127.0.0.1:{port}"));

    let seen = Arc::new(Mutex::new(None));
    let http = capturing_client(seen.clone());
    let health: HealthResponse = http
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ring_id: [u8; 16] = hex_decode(&health.ring_id).unwrap();
    let (moon, req) = MoonHandshake::start(&payload, ring_id, [0x33; 16], "Smoke Phone").unwrap();
    let step = |s: &str| format!("{base}/api/v1/pair/{}/{s}", payload.pairing_id.to_hex());
    let resp: PairExchangeResponse = http
        .post(step("exchange"))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let moon = moon.finish(&resp).unwrap();
    http.post(step("confirm"))
        .json(&moon.confirm_request())
        .send()
        .await
        .unwrap();
    let CtlEvent::PairSas { sas, .. } = cli.recv().await else {
        panic!("no SAS")
    };
    assert_eq!(sas, moon.sas());
    cli.send(&CtlRequest::PairDecision { approve: true }).await;
    assert!(matches!(cli.recv().await, CtlEvent::PairDone { .. }));
    let sealed: PairResultResponse = http
        .get(step("result"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let observed: [u8; 32] = Sha256::digest(seen.lock().unwrap().as_ref().unwrap()).into();
    let paired = moon.open_result(&sealed, &observed).unwrap();
    let token = paired.device_token;

    // --- Upload a file ----------------------------------------------------
    let content: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    let create = CreateUploadRequest {
        path: RelPath::new("Inbox/smoke-test.bin").unwrap(),
        size: content.len() as u64,
        mtime_ns: None,
        blake3: hash_bytes(&content),
        chunk_size: Some(transfer::MIN_CHUNK_SIZE),
    };
    let created: CreateUploadResponse = http
        .post(format!("{base}/api/v1/uploads"))
        .bearer_auth(&token)
        .json(&create)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let CreateUploadResponse::Open {
        upload_id,
        chunk_size,
        chunk_count,
        ..
    } = created
    else {
        panic!("expected open upload")
    };
    for i in 0..chunk_count as usize {
        let chunk =
            &content[i * chunk_size as usize..content.len().min((i + 1) * chunk_size as usize)];
        let s = http
            .post(format!("{base}/api/v1/uploads/{upload_id}/chunks/{i}"))
            .bearer_auth(&token)
            .header(CHUNK_HASH_HEADER, hash_bytes(chunk).to_hex())
            .body(chunk.to_vec())
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(s, 204);
    }
    let ack: UploadAck = http
        .post(format!("{base}/api/v1/uploads/{upload_id}/commit"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ack.blake3, hash_bytes(&content));
    assert_eq!(
        std::fs::read(d.join("ring/Inbox/smoke-test.bin")).unwrap(),
        content
    );
    let index: IndexResponse = http
        .get(format!("{base}/api/v1/index"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entry = index.entries.iter().find(|e| e.id == ack.file_id).unwrap();
    assert_eq!(entry.origin.as_deref(), Some("smoke-phone"));

    let CtlEvent::Status(st) = Ctl::request(&socket, CtlRequest::Status).await else {
        panic!()
    };
    println!("status: {st:#?}");

    // --- Notification, then clean shutdown ------------------------------
    wait_until("notification attempt", || {
        let l = log_of(&log_path);
        l.contains("notification shown") || l.contains("desktop notification failed")
    })
    .await;
    let shown = log_of(&log_path).contains("notification shown");
    println!("desktop notification shown: {shown}");

    let pid = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let status = child.wait().unwrap();
    assert!(status.success(), "bringd exit status: {status}");
    assert!(!socket.exists(), "control socket removed on shutdown");
    println!("--- bringd log ---\n{}", log_of(&log_path));
}
