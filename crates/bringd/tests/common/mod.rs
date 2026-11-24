//! Helpers shared by `bringd` integration tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header, request};
use bringd::{Config, Ring, devices};
use http_body_util::BodyExt;
use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt;

pub struct TestRing {
    pub ring: Ring,
    pub root: PathBuf,
    pub config: Config,
    /// Bearer token of a paired test device ("Test Phone" → `test-phone`).
    pub token: String,
    dir: tempfile::TempDir,
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
    start_with(tempfile::tempdir().unwrap(), |_| {}).await
}

pub async fn start_with(dir: tempfile::TempDir, tweak: impl FnOnce(&mut Config)) -> TestRing {
    let mut config = test_config(dir.path());
    tweak(&mut config);
    let ring = Ring::start(&config).await.unwrap();
    let (_, token) = devices::register(&ring.db.lock(), [7; 16], "Test Phone", 0).unwrap();
    TestRing {
        root: ring.root.clone(),
        ring,
        config,
        token,
        dir,
    }
}

impl TestRing {
    /// Stops all services and starts them again on the same directories.
    /// The test device is re-paired (same UUID → same device, new token).
    pub async fn restart(self) -> TestRing {
        let TestRing {
            ring, config, dir, ..
        } = self;
        drop(ring);
        tokio::time::sleep(Duration::from_millis(50)).await;
        start_with(dir, |c| *c = config).await
    }

    pub fn router(&self) -> Router {
        self.ring.router()
    }

    pub fn authed(&self, builder: request::Builder) -> request::Builder {
        builder.header(header::AUTHORIZATION, format!("Bearer {}", self.token))
    }

    /// Authenticated request with an optional raw body.
    pub async fn call(
        &self,
        builder: request::Builder,
        body: impl Into<Body>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        send(
            &self.router(),
            self.authed(builder).body(body.into()).unwrap(),
        )
        .await
    }

    pub async fn get_json<T: DeserializeOwned>(&self, uri: &str) -> (StatusCode, T) {
        let (status, _, body) = self.call(Request::get(uri), Body::empty()).await;
        (status, parse(uri, status, &body))
    }

    pub async fn post_json<T: DeserializeOwned>(
        &self,
        uri: &str,
        payload: &impl Serialize,
    ) -> (StatusCode, T) {
        let (status, _, body) = self
            .call(
                Request::post(uri).header(header::CONTENT_TYPE, "application/json"),
                serde_json::to_vec(payload).unwrap(),
            )
            .await;
        (status, parse(uri, status, &body))
    }
}

pub fn parse<T: DeserializeOwned>(uri: &str, status: StatusCode, body: &[u8]) -> T {
    serde_json::from_slice(body).unwrap_or_else(|e| {
        panic!(
            "bad JSON from {uri} ({status}): {e}: {}",
            String::from_utf8_lossy(body)
        )
    })
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

/// Sends a request through the router; returns status, headers and body.
pub async fn send(router: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
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

/// Unauthenticated GET returning parsed JSON.
pub async fn get_json<T: DeserializeOwned>(router: &Router, uri: &str) -> (StatusCode, T) {
    let (status, _, body) = send(router, Request::get(uri).body(Body::empty()).unwrap()).await;
    (status, parse(uri, status, &body))
}
