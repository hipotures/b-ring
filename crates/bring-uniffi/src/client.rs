//! HTTP clients (docs/ARCHITECTURE.md §3.1).
//!
//! - **LAN:** a `rustls` verifier that accepts the Ring only if the SHA-256 of
//!   its end-entity certificate equals the pin from pairing. SAN/IP and the CA
//!   chain are ignored (DHCP changes must not break anything); the handshake
//!   signature is still verified, which proves possession of the key.
//! - **Pairing:** accepts any certificate but records every fingerprint it
//!   saw; pairing aborts unless they all equal the pin inside `sealed`.
//! - **WAN:** ordinary WebPKI verification (Mozilla roots, no platform
//!   verifier — that would need JNI on Android).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::{Client, RequestBuilder, Response};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::error::MoonError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Generous: a single 8 MiB chunk over slow Wi-Fi.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn cert_sha256(cert: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(cert.as_ref()).into()
}

fn verify12(
    p: &CryptoProvider,
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    rustls::crypto::verify_tls12_signature(message, cert, dss, &p.signature_verification_algorithms)
}

fn verify13(
    p: &CryptoProvider,
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    rustls::crypto::verify_tls13_signature(message, cert, dss, &p.signature_verification_algorithms)
}

/// Accepts exactly one certificate, identified by its SHA-256.
#[derive(Debug)]
pub struct PinnedVerifier {
    pin: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl PinnedVerifier {
    pub fn new(pin: [u8; 32]) -> Self {
        Self {
            pin,
            provider: provider(),
        }
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if cert_sha256(end_entity) == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify12(&self.provider, message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify13(&self.provider, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Pairing only: accepts any certificate and remembers each fingerprint.
#[derive(Debug)]
pub struct RecordingVerifier {
    seen: Mutex<Vec<[u8; 32]>>,
    provider: Arc<CryptoProvider>,
}

impl Default for RecordingVerifier {
    fn default() -> Self {
        Self {
            seen: Mutex::default(),
            provider: provider(),
        }
    }
}

impl RecordingVerifier {
    /// The single fingerprint seen on every connection, or `None` if none or
    /// several different ones were presented.
    pub fn observed(&self) -> Option<[u8; 32]> {
        let seen = self.seen.lock().expect("verifier lock");
        let first = *seen.first()?;
        seen.iter().all(|s| *s == first).then_some(first)
    }
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.seen
            .lock()
            .expect("verifier lock")
            .push(cert_sha256(end_entity));
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify12(&self.provider, message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify13(&self.provider, message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn with_verifier(v: Arc<dyn ServerCertVerifier>) -> Result<ClientConfig, MoonError> {
    let mut c = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| MoonError::invalid(format!("TLS setup: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(v)
        .with_no_client_auth();
    c.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(c)
}

fn build(tls: ClientConfig) -> Result<Client, MoonError> {
    Ok(Client::builder()
        .use_preconfigured_tls(tls)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// Client for the LAN listener, pinned to the Ring's certificate.
pub fn pinned(pin: [u8; 32]) -> Result<Client, MoonError> {
    build(with_verifier(Arc::new(PinnedVerifier::new(pin)))?)
}

/// Client for `/pair/*` (see module docs).
pub fn recording(v: Arc<RecordingVerifier>) -> Result<Client, MoonError> {
    build(with_verifier(v)?)
}

/// Client for `wan_url`: standard certificate validation.
pub fn webpki() -> Result<Client, MoonError> {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut c = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| MoonError::invalid(format!("TLS setup: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    c.alpn_protocols = vec![b"http/1.1".to_vec()];
    build(c)
}

/// Sends the request; non-2xx becomes [`MoonError::Server`].
pub async fn send(req: RequestBuilder) -> Result<Response, MoonError> {
    let resp = req.send().await?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.bytes().await.unwrap_or_default();
    Err(MoonError::from_response(status.as_u16(), &body))
}

/// Sends the request and decodes a JSON body.
pub async fn json<T: DeserializeOwned>(req: RequestBuilder) -> Result<T, MoonError> {
    Ok(send(req).await?.json().await?)
}
