//! Step 7 gate: real listeners. HTTPS with the persistent self-signed
//! identity, bearer auth (`401` for missing/revoked tokens), and the optional
//! plain-HTTP listener for a reverse proxy.

mod common;

use std::sync::{Arc, Mutex};

use bring_core::api::HealthResponse;
use bringd::devices;
use common::{capturing_client, install_provider, listen, start};

#[tokio::test]
async fn https_auth_and_proxy_listener() {
    let t = start().await;
    let (servers, lan, proxy, task) = listen(&t).await;
    let seen = Arc::new(Mutex::new(None));
    let http = capturing_client(seen.clone());

    let health: HealthResponse = http
        .get(format!("https://{lan}/api/v1/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health.ring_id, t.ring.ring_id);
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some(t.ring.tls.cert_der.as_slice()),
        "the pinned identity is what the LAN listener presents"
    );

    let index = format!("https://{lan}/api/v1/index");
    let s = http.get(&index).send().await.unwrap().status();
    assert_eq!(s, 401, "missing token");
    let s = http
        .get(&index)
        .bearer_auth("bogus")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 401, "unknown token");
    let s = http
        .get(&index)
        .bearer_auth(&t.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 200);

    let me = devices::by_token(&t.ring.db.lock(), &t.token)
        .unwrap()
        .unwrap();
    let before = me.last_seen_at;
    assert!(before.is_some(), "activity is recorded");
    devices::revoke(&t.ring.db.lock(), me.id, devices::now_secs()).unwrap();
    let s = http
        .get(&index)
        .bearer_auth(&t.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 401, "revoked token");

    // Reverse-proxy listener: plain HTTP, same API.
    install_provider();
    let plain = reqwest::Client::builder().build().unwrap();
    let s = plain
        .get(format!("http://{proxy}/api/v1/health"))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, 200);
    // ...while the LAN port only speaks TLS.
    assert!(
        plain
            .get(format!("http://{lan}/api/v1/health"))
            .send()
            .await
            .map(|r| !r.status().is_success())
            .unwrap_or(true)
    );

    servers.shutdown();
    task.await.unwrap();
}

#[tokio::test]
async fn certificate_survives_restart() {
    let t = start().await;
    let first = t.ring.tls.cert_sha256;
    let t = t.restart().await;
    assert_eq!(t.ring.tls.cert_sha256, first);
}
