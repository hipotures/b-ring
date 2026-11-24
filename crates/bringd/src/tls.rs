//! The Ring's self-signed TLS identity for the LAN listener.
//!
//! Moons do not trust it through a CA: they pin the SHA-256 of the
//! certificate, which they receive (encrypted) during pairing. The identity is
//! generated once and kept in `<data_dir>/tls/`, so the pin stays valid.

use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct TlsIdentity {
    pub cert_der: Vec<u8>,
    key_der: Vec<u8>,
    /// SHA-256 of `cert_der`: the pin Moons verify.
    pub cert_sha256: [u8; 32],
}

impl std::fmt::Debug for TlsIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsIdentity")
            .field("cert_sha256", &self.fingerprint_hex())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("TLS identity I/O ({path}): {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("cannot generate TLS certificate: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("invalid TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> TlsError + '_ {
    move |source| TlsError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Writes `data` to `path` atomically with the given mode.
fn write_private(path: &Path, data: &[u8], mode: u32) -> Result<(), TlsError> {
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)
        .map_err(io_at(&tmp))?;
    f.write_all(data).map_err(io_at(&tmp))?;
    f.sync_all().map_err(io_at(&tmp))?;
    std::fs::rename(&tmp, path).map_err(io_at(path))
}

impl TlsIdentity {
    /// Loads the identity from `dir`, generating it on first use.
    pub fn load_or_create(dir: &Path) -> Result<Self, TlsError> {
        let cert_path = dir.join("cert.der");
        let key_path = dir.join("key.der");
        if cert_path.exists() && key_path.exists() {
            let cert_der = std::fs::read(&cert_path).map_err(io_at(&cert_path))?;
            let key_der = std::fs::read(&key_path).map_err(io_at(&key_path))?;
            return Ok(Self::new(cert_der, key_der));
        }
        std::fs::create_dir_all(dir).map_err(io_at(dir))?;
        let ck = rcgen::generate_simple_self_signed(vec!["b-ring.local".to_owned()])?;
        let cert_der = ck.cert.der().to_vec();
        let key_der = ck.signing_key.serialize_der();
        write_private(&key_path, &key_der, 0o600)?;
        write_private(&cert_path, &cert_der, 0o644)?;
        Ok(Self::new(cert_der, key_der))
    }

    fn new(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let cert_sha256 = Sha256::digest(&cert_der).into();
        Self {
            cert_der,
            key_der,
            cert_sha256,
        }
    }

    pub fn fingerprint_hex(&self) -> String {
        self.cert_sha256
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// A rustls server config using the `ring` provider (HTTP/2 + HTTP/1.1).
    pub fn server_config(&self) -> Result<Arc<ServerConfig>, TlsError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let key = PrivateKeyDer::try_from(self.key_der.clone())
            .map_err(|e| rustls::Error::General(e.to_owned()))?;
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(self.cert_der.clone())], key)?;
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn generated_once_and_persisted() {
        let d = tempfile::tempdir().unwrap();
        let a = TlsIdentity::load_or_create(&d.path().join("tls")).unwrap();
        let b = TlsIdentity::load_or_create(&d.path().join("tls")).unwrap();
        assert_eq!(a.cert_sha256, b.cert_sha256);
        assert_eq!(a.fingerprint_hex().len(), 64);
        let mode = std::fs::metadata(d.path().join("tls/key.der"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        a.server_config().unwrap();
    }
}
