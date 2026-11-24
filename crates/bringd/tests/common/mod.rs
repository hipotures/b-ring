//! Helpers shared by `bringd` integration tests.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header, request};
use bring_core::ctl::{CtlEvent, CtlRequest};
use bringd::serve::{self, Servers};
use bringd::{Config, Ring, devices};
use http_body_util::BodyExt;
use rusqlite::Connection;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
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
        // Never pop up desktop notifications from automated tests.
        notifications: false,
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

// ---------------------------------------------------------------------------
// Real listeners + a TLS client that records the server certificate
// ---------------------------------------------------------------------------

/// Starts the HTTPS LAN listener (and a proxy listener) on loopback.
pub async fn listen(
    t: &TestRing,
) -> (Servers, SocketAddr, SocketAddr, tokio::task::JoinHandle<()>) {
    let servers = Servers::default();
    let handles = servers.clone();
    let (app, tls) = (t.router(), t.ring.tls.clone());
    let any: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let task = tokio::spawn(async move {
        serve::serve(app, &tls, any, Some(any), handles)
            .await
            .unwrap();
    });
    let lan = servers.lan.listening().await.expect("lan listener");
    let proxy = servers.proxy.listening().await.expect("proxy listener");
    (servers, lan, proxy, task)
}

/// Accepts any certificate but records it, while still verifying the
/// handshake signature (the server must hold the certificate's key).
#[derive(Debug)]
pub struct Capture {
    pub seen: Arc<Mutex<Option<Vec<u8>>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = Some(end_entity.to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// reqwest is built with `rustls-no-provider`; ignore "already installed".
pub fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn capturing_client(seen: Arc<Mutex<Option<Vec<u8>>>>) -> reqwest::Client {
    install_provider();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Capture { seen, provider }))
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Control socket client
// ---------------------------------------------------------------------------

pub struct Ctl {
    lines: Lines<BufReader<OwnedReadHalf>>,
    w: OwnedWriteHalf,
}

impl Ctl {
    pub async fn connect(path: &PathBuf) -> Self {
        let (r, w) = UnixStream::connect(path).await.unwrap().into_split();
        Self {
            lines: BufReader::new(r).lines(),
            w,
        }
    }

    pub async fn send(&mut self, req: &CtlRequest) {
        let mut line = serde_json::to_vec(req).unwrap();
        line.push(b'\n');
        self.w.write_all(&line).await.unwrap();
    }

    pub async fn recv(&mut self) -> CtlEvent {
        let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("control reply timed out")
            .unwrap()
            .expect("control socket closed");
        serde_json::from_str(&line).unwrap()
    }

    pub async fn request(path: &PathBuf, req: CtlRequest) -> CtlEvent {
        let mut c = Self::connect(path).await;
        c.send(&req).await;
        c.recv().await
    }
}
