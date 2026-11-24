//! Helpers shared by `bringd` integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bringd::{Config, Ring};
use http_body_util::BodyExt;
use rusqlite::Connection;
use tower::ServiceExt;

pub struct TestRing {
    pub ring: Ring,
    pub root: PathBuf,
    pub config: Config,
    _dir: tempfile::TempDir,
}

pub fn test_config(dir: &std::path::Path) -> Config {
    Config {
        ring_root: dir.join("ring"),
        data_dir: dir.join("data"),
        debounce_ms: 100,
        ..Config::default()
    }
}

pub async fn start() -> TestRing {
    let dir = tempfile::tempdir().unwrap();
    start_with(dir, |_| {}).await
}

pub async fn start_with(dir: tempfile::TempDir, tweak: impl FnOnce(&mut Config)) -> TestRing {
    let mut config = test_config(dir.path());
    tweak(&mut config);
    let ring = Ring::start(&config).await.unwrap();
    let root = ring.root.clone();
    TestRing {
        ring,
        root,
        config,
        _dir: dir,
    }
}

/// Polls `cond` against the index until it holds (or panics after 10 s).
pub async fn wait_for(ring: &Ring, what: &str, cond: impl Fn(&Connection) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if cond(&ring.db.lock()) {
            return;
        }
        if Instant::now() > deadline {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Sends a request through the router; returns status and body bytes.
pub async fn send(
    router: &Router,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    router: &Router,
    uri: &str,
) -> (StatusCode, T) {
    let (status, _, body) = send(router, Request::get(uri).body(Body::empty()).unwrap()).await;
    let parsed = serde_json::from_slice(&body).unwrap_or_else(|e| {
        panic!(
            "bad JSON from {uri} ({status}): {e}: {}",
            String::from_utf8_lossy(&body)
        )
    });
    (status, parsed)
}
