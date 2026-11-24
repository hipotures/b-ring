//! Step 8 gate: pairing.
//!
//! - end to end over real HTTPS and the real control socket: payload text →
//!   SPAKE2 → matching SAS on both sides → user approval → sealed result →
//!   TLS pin cross-check → working device token → revoke;
//! - wrong code yields no key, attempt limit, forged confirmation, user
//!   rejection (`403`), expiry (`410`), CLI hang-up cancels the session.

mod common;

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use bring_core::api::{
    ErrorBody, ErrorCode, HealthResponse, PairConfirmRequest, PairExchangeResponse, PairPending,
    PairResultResponse,
};
use bring_core::codec::{b64_encode, hex_decode};
use bring_core::ctl::{CtlEvent, CtlRequest};
use bring_core::pairing::{BootstrapPayload, MoonHandshake, OneTimeToken, PairingError};
use bringd::ctl_server;
use common::{Ctl, TestRing, capturing_client, listen, start, start_with};
use sha2::{Digest, Sha256};

const UUID: [u8; 16] = [0x42; 16];

async fn start_ctl(t: &TestRing) -> PathBuf {
    let path = t.config.data_dir.join("ctl.sock");
    let listener = ctl_server::bind(&path).await.unwrap();
    tokio::spawn(ctl_server::run(listener, t.ring.ctl_context()));
    path
}

// ---------------------------------------------------------------------------
// Moon over the in-process router
// ---------------------------------------------------------------------------

fn ring_id(t: &TestRing) -> [u8; 16] {
    hex_decode(&t.ring.ring_id).unwrap()
}

fn uri(p: &BootstrapPayload, step: &str) -> String {
    format!("/api/v1/pair/{}/{step}", p.pairing_id.to_hex())
}

/// Runs the exchange with `token` (possibly wrong) and returns the HTTP
/// status plus, on `200`, the Moon's verification outcome.
async fn exchange(
    t: &TestRing,
    p: &BootstrapPayload,
    token: &OneTimeToken,
) -> (
    StatusCode,
    Option<Result<bring_core::pairing::MoonVerified, PairingError>>,
) {
    let mut moon_view = p.clone();
    moon_view.token = token.clone();
    let (moon, req) = MoonHandshake::start(&moon_view, ring_id(t), UUID, "Pixel 8").unwrap();
    let (s, _, body) = t
        .call(
            axum::http::Request::post(uri(p, "exchange"))
                .header("content-type", "application/json"),
            serde_json::to_vec(&req).unwrap(),
        )
        .await;
    if s != StatusCode::OK {
        return (s, None);
    }
    let resp: PairExchangeResponse = serde_json::from_slice(&body).unwrap();
    (s, Some(moon.finish(&resp)))
}

fn code(body: &[u8]) -> ErrorCode {
    serde_json::from_slice::<ErrorBody>(body).unwrap().error
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pairing_end_to_end_over_tls_and_control_socket() {
    let t = start().await;
    let (servers, lan, _proxy, task) = listen(&t).await;
    t.ring
        .pairing
        .set_advertise(SocketAddrV4::new(Ipv4Addr::LOCALHOST, lan.port()));
    let ctl_path = start_ctl(&t).await;

    // `bring pair`
    let mut cli = Ctl::connect(&ctl_path).await;
    cli.send(&CtlRequest::Pair).await;
    let CtlEvent::PairPayload { text, lan_url, .. } = cli.recv().await else {
        panic!("expected payload")
    };
    assert_eq!(lan_url, format!("https://127.0.0.1:{}", lan.port()));

    // The Moon types/scans the code and talks to the LAN URL in it.
    let payload = BootstrapPayload::from_text(&text.to_lowercase()).unwrap();
    let base = payload.lan_url();
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
    let (moon, req) = MoonHandshake::start(&payload, ring_id, UUID, "Pixel 8").unwrap();
    let pair_url =
        |step: &str| format!("{base}/api/v1/pair/{}/{step}", payload.pairing_id.to_hex());
    let resp: PairExchangeResponse = http
        .post(pair_url("exchange"))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let moon = moon
        .finish(&resp)
        .expect("ring proved knowledge of the code");
    let r = http
        .post(pair_url("confirm"))
        .json(&moon.confirm_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);

    // Both screens show the same code.
    let CtlEvent::PairSas { sas, device_name } = cli.recv().await else {
        panic!("expected SAS")
    };
    assert_eq!(sas, moon.sas());
    assert_eq!(device_name, "Pixel 8");

    // Not decided yet.
    let r = http.get(pair_url("result")).send().await.unwrap();
    assert_eq!(r.status(), 202);
    assert_eq!(
        r.json::<PairPending>().await.unwrap().status,
        "awaiting_user"
    );

    cli.send(&CtlRequest::PairDecision { approve: true }).await;
    let CtlEvent::PairDone { device } = cli.recv().await else {
        panic!("expected done")
    };
    assert_eq!(device.slug, "pixel-8");

    let sealed: PairResultResponse = http
        .get(pair_url("result"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let observed: [u8; 32] = Sha256::digest(seen.lock().unwrap().as_ref().unwrap()).into();
    assert_eq!(
        moon.open_result(&sealed, &[0u8; 32]),
        Err(PairingError::CertificateMismatch),
        "a different certificate (MITM) is detected"
    );
    let result = moon.open_result(&sealed, &observed).unwrap();
    assert_eq!(result.device_slug, "pixel-8");
    assert_eq!(result.ring_id, t.ring.ring_id);
    assert_eq!(result.tls_cert_sha256, t.ring.tls.fingerprint_hex());
    assert_eq!(result.lan_url, base);

    // The issued token works, and can be revoked from the CLI.
    let index = format!("{base}/api/v1/index");
    let s = http
        .get(&index)
        .bearer_auth(&result.device_token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 200);

    let CtlEvent::Devices { devices } = Ctl::request(&ctl_path, CtlRequest::Devices).await else {
        panic!()
    };
    let slugs: Vec<&str> = devices.iter().map(|d| d.slug.as_str()).collect();
    assert_eq!(slugs, ["test-phone", "pixel-8"]);
    let CtlEvent::Status(st) = Ctl::request(&ctl_path, CtlRequest::Status).await else {
        panic!()
    };
    assert_eq!(
        (st.devices, st.ring_id.as_str()),
        (2, t.ring.ring_id.as_str())
    );

    let ev = Ctl::request(
        &ctl_path,
        CtlRequest::Revoke {
            device: "pixel-8".into(),
        },
    )
    .await;
    assert!(matches!(ev, CtlEvent::Revoked { ref device } if device.revoked));
    let s = http
        .get(&index)
        .bearer_auth(&result.device_token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 401);
    let ev = Ctl::request(
        &ctl_path,
        CtlRequest::Revoke {
            device: "nope".into(),
        },
    )
    .await;
    assert!(matches!(ev, CtlEvent::Error { .. }));

    servers.shutdown();
    task.await.unwrap();
}

#[tokio::test]
async fn wrong_code_yields_no_key_and_attempts_are_limited() {
    let t = start().await;
    let (payload, _rx) = t.ring.pairing.start().unwrap();
    let wrong = OneTimeToken([0; 16]);
    for _ in 0..3 {
        let (s, outcome) = exchange(&t, &payload, &wrong).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(outcome.unwrap().err(), Some(PairingError::KeyConfirmation));
    }
    // Attempts exhausted: even the right code is refused now.
    let (s, _) = exchange(&t, &payload, &payload.token).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, body) = t
        .call(
            axum::http::Request::get(uri(&payload, "result")),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(
        (s, code(&body)),
        (StatusCode::FORBIDDEN, ErrorCode::TooManyAttempts)
    );
}

#[tokio::test]
async fn forged_confirmation_then_user_rejection() {
    let t = start().await;
    let (payload, _rx) = t.ring.pairing.start().unwrap();

    let (_, ok) = exchange(&t, &payload, &payload.token).await;
    ok.unwrap().unwrap();
    let forged = PairConfirmRequest {
        moon_confirm: b64_encode(&[0u8; 32]),
    };
    let (s, body): (_, ErrorBody) = t.post_json(&uri(&payload, "confirm"), &forged).await;
    assert_eq!(
        (s, body.error),
        (StatusCode::FORBIDDEN, ErrorCode::Rejected)
    );

    // A genuine retry still works...
    let (_, ok) = exchange(&t, &payload, &payload.token).await;
    let moon = ok.unwrap().unwrap();
    let (s, _): (_, PairPending) = t
        .post_json(&uri(&payload, "confirm"), &moon.confirm_request())
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    // ...and a second exchange after confirmation is refused.
    let (s, _) = exchange(&t, &payload, &payload.token).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // The user says no.
    assert_eq!(
        t.ring.pairing.decide(&payload.pairing_id, false).unwrap(),
        None
    );
    let (s, _, body) = t
        .call(
            axum::http::Request::get(uri(&payload, "result")),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(
        (s, code(&body)),
        (StatusCode::FORBIDDEN, ErrorCode::Rejected)
    );
    let devices = bringd::devices::list(&t.ring.db.lock()).unwrap();
    assert_eq!(devices.len(), 1, "no device was registered");
}

#[tokio::test]
async fn expired_and_unknown_sessions() {
    let t = start_with(tempfile::tempdir().unwrap(), |c| c.pairing_ttl_secs = 1).await;
    let (payload, _rx) = t.ring.pairing.start().unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (s, _) = exchange(&t, &payload, &payload.token).await;
    assert_eq!(s, StatusCode::GONE);
    let (s, _) = exchange(&t, &payload, &payload.token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "expired sessions are removed");
    let (s, _, _) = t
        .call(
            axum::http::Request::get("/api/v1/pair/not-hex/result"),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cli_hangup_cancels_and_expiry_is_reported() {
    let t = start_with(tempfile::tempdir().unwrap(), |c| c.pairing_ttl_secs = 1).await;
    let ctl_path = start_ctl(&t).await;

    let mut cli = Ctl::connect(&ctl_path).await;
    cli.send(&CtlRequest::Pair).await;
    let CtlEvent::PairPayload { text, .. } = cli.recv().await else {
        panic!()
    };
    drop(cli);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let payload = BootstrapPayload::from_text(&text).unwrap();
    let (s, _) = exchange(&t, &payload, &payload.token).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let mut cli = Ctl::connect(&ctl_path).await;
    cli.send(&CtlRequest::Pair).await;
    assert!(matches!(cli.recv().await, CtlEvent::PairPayload { .. }));
    assert_eq!(cli.recv().await, CtlEvent::PairExpired);
}
