//! Network listeners: HTTPS on the LAN (self-signed, pinned by Moons) and an
//! optional plain-HTTP listener for a TLS-terminating reverse proxy (WAN).

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use axum_server::{Control, Handle};

use crate::tls::{TlsError, TlsIdentity};

/// Controls both listeners; also reports their bound addresses.
#[derive(Clone)]
pub struct Servers {
    control: Control,
    pub lan: Handle<SocketAddr>,
    pub proxy: Handle<SocketAddr>,
}

impl Default for Servers {
    fn default() -> Self {
        let control = Control::new();
        Self {
            lan: Handle::with_control(control.clone()),
            proxy: Handle::with_control(control.clone()),
            control,
        }
    }
}

impl Servers {
    /// Stops accepting, lets in-flight requests finish for up to 10 s.
    pub fn shutdown(&self) {
        self.control
            .graceful_shutdown(Some(Duration::from_secs(10)));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error("listener failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Runs the listeners until [`Servers::shutdown`].
pub async fn serve(
    app: Router,
    tls: &TlsIdentity,
    lan: SocketAddr,
    proxy: Option<SocketAddr>,
    servers: Servers,
) -> Result<(), ServeError> {
    let tls = RustlsConfig::from_config(tls.server_config()?);
    let lan_srv = axum_server::bind_rustls(lan, tls)
        .handle(servers.lan.clone())
        .serve(app.clone().into_make_service());
    match proxy {
        Some(addr) => {
            let proxy_srv = axum_server::bind(addr)
                .handle(servers.proxy.clone())
                .serve(app.into_make_service());
            tokio::try_join!(lan_srv, proxy_srv)?;
        }
        None => lan_srv.await?,
    }
    Ok(())
}
