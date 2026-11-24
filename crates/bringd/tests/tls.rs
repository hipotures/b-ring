//! Step 7 gate: real listeners. HTTPS with the persistent self-signed
//! identity, bearer auth (`401` for missing/revoked tokens), and the optional
//! plain-HTTP listener for a reverse proxy.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bring_core::api::HealthResponse;
use bringd::devices;
use bringd::serve::{self, Servers};
use common::{TestRing, start};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Accepts any certificate but records it (and still checks the handshake
/// signature, so the server must hold the certificate's private key).
#[derive(Debug)]
struct Capture {
    seen: Arc<Mutex<Option<Vec<u8>>>>,
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

fn install_provider() {
    // reqwest is built with `rustls-no-provider`; ignore "already installed".
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn client(seen: Arc<Mutex<Option<Vec<u8>>>>) -> reqwest::Client {
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

async fn listen(t: &TestRing) -> (Servers, SocketAddr, SocketAddr, tokio::task::JoinHandle<()>) {
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

#[tokio::test]
async fn https_auth_and_proxy_listener() {
    let t = start().await;
    let (servers, lan, proxy, task) = listen(&t).await;
    let seen = Arc::new(Mutex::new(None));
    let http = client(seen.clone());

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
