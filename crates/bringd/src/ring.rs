//! Wiring: opens the index, starts the hasher and watcher, runs the initial
//! scan and exposes the API router.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use axum::Router;
use tracing::info;

use crate::config::Config;
use crate::ctl_server::CtlContext;
use crate::hasher::{self, HashQueue};
use crate::notify::Notifier;
use crate::pairing::{self, Pairing};
use crate::scanner::{self, RESERVED_DIR};
use crate::server::{self, AppState};
use crate::store::{self, Db};
use crate::thumbs::ThumbStore;
use crate::tls::{TlsError, TlsIdentity};
use crate::uploads::{self, Uploads};
use crate::watcher::{self, WatcherHandle};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("cannot prepare {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Db(#[from] bring_core::db::DbError),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error(
        "the Ring filesystem at {path} does not support hard links ({source}); \
         uploads need a POSIX filesystem such as ext4, btrfs or xfs"
    )]
    NoHardLinks {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot watch the Ring directory: {0}")]
    Watch(#[from] notify_debouncer_full::notify::Error),
}

/// A running Ring service.
pub struct Ring {
    pub root: PathBuf,
    pub db: Db,
    pub hashq: HashQueue,
    pub ring_id: String,
    pub thumbs: ThumbStore,
    pub uploads: Uploads,
    pub tls: TlsIdentity,
    pub pairing: Pairing,
    notifier: Notifier,
    wan_url: Option<String>,
    stream_key: [u8; 32],
    _watcher: WatcherHandle,
}

fn mkdir(path: PathBuf) -> Result<PathBuf, StartError> {
    std::fs::create_dir_all(&path).map_err(|source| StartError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

impl Ring {
    /// Starts all background services. Must run inside a Tokio runtime.
    pub async fn start(config: &Config) -> Result<Self, StartError> {
        mkdir(config.data_dir.clone())?;
        let root = mkdir(config.ring_root.clone())?;
        let root = root.canonicalize().map_err(|source| StartError::Io {
            path: root.clone(),
            source,
        })?;
        let staging = mkdir(root.join(RESERVED_DIR).join("staging"))?;
        for folder in &config.folders {
            mkdir(root.join(folder))?;
        }

        let tls = TlsIdentity::load_or_create(&config.tls_dir())?;
        let db = Db::open(&config.db_path())?;
        let ring_id = store::ring_id_hex(&db.lock())?;
        let stream_key: [u8; 32] = bring_core::db::ring_meta_blob(&db.lock(), "stream_key")?;
        let ring_id_bytes: [u8; 16] = bring_core::db::ring_meta_blob(&db.lock(), "ring_id")?;
        let pairing = Pairing::new(
            db.clone(),
            ring_id_bytes,
            tls.cert_sha256,
            advertise_addr(config, config.lan_listen.port()),
            config.wan_url.clone(),
            std::time::Duration::from_secs(config.pairing_ttl_secs),
        );
        let uploads = Uploads::new(root.clone(), staging, db.clone());
        uploads
            .check_hard_links()
            .map_err(|source| StartError::NoHardLinks {
                path: root.clone(),
                source,
            })?;
        let recovered = uploads::recover(&uploads)?;
        if recovered > 0 {
            info!(recovered, "reopened uploads interrupted during commit");
        }
        spawn_cleanup(uploads.clone());
        let thumbs_dir = config.thumbs_dir();
        let thumbs =
            ThumbStore::new(thumbs_dir.clone(), config.ffmpeg.clone()).map_err(|source| {
                StartError::Io {
                    path: thumbs_dir,
                    source,
                }
            })?;
        let hashq = hasher::spawn(root.clone(), db.clone(), thumbs.clone());
        // Watch first, then scan: nothing that happens in between is missed.
        let watcher = watcher::spawn(root.clone(), config.debounce(), db.clone(), hashq.clone())?;

        let (scan_root, scan_db) = (root.clone(), db.clone());
        let to_hash =
            tokio::task::spawn_blocking(move || scanner::reconcile(&scan_root, &scan_db, None))
                .await
                .expect("initial scan panicked")?;
        info!(root = %root.display(), pending_hashes = to_hash.len(), "initial scan done");
        hashq.enqueue_all(to_hash);

        Ok(Self {
            root,
            db,
            hashq,
            ring_id,
            thumbs,
            uploads,
            tls,
            pairing,
            notifier: if config.notifications {
                Notifier::spawn()
            } else {
                Notifier::disabled()
            },
            wan_url: config.wan_url.clone(),
            stream_key,
            _watcher: watcher,
        })
    }

    pub fn router(&self) -> Router {
        server::router(AppState {
            root: self.root.clone(),
            db: self.db.clone(),
            ring_id: self.ring_id.clone(),
            thumbs: self.thumbs.clone(),
            uploads: self.uploads.clone(),
            hashq: self.hashq.clone(),
            stream_key: self.stream_key,
            pairing: self.pairing.clone(),
            notifier: self.notifier.clone(),
        })
    }

    /// Context for the `bring` control socket.
    pub fn ctl_context(&self) -> CtlContext {
        CtlContext {
            db: self.db.clone(),
            pairing: self.pairing.clone(),
            ring_id: self.ring_id.clone(),
            ring_root: self.root.clone(),
            tls_cert_sha256: self.tls.fingerprint_hex(),
            wan_url: self.wan_url.clone(),
        }
    }
}

/// Hourly removal of abandoned uploads (first run at start).
fn spawn_cleanup(uploads: Uploads) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let up = uploads.clone();
            let res = tokio::task::spawn_blocking(move || {
                uploads::cleanup(&up, crate::devices::now_secs(), uploads::ABANDON_AFTER)
            })
            .await;
            if let Ok(Err(e)) = res {
                tracing::warn!(error = %e, "upload cleanup failed");
            }
        }
    });
}

/// The LAN address advertised in pairing payloads.
pub fn advertise_addr(config: &Config, port: u16) -> SocketAddrV4 {
    let ip = config
        .advertise_ip
        .or(match config.lan_listen {
            SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()),
            _ => None,
        })
        .or_else(pairing::detect_lan_ip)
        .unwrap_or(Ipv4Addr::LOCALHOST);
    SocketAddrV4::new(ip, port)
}
