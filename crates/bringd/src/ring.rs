//! Wiring: opens the index, starts the hasher and watcher, runs the initial
//! scan and exposes the API router.

use std::path::PathBuf;

use axum::Router;
use tracing::info;

use crate::config::Config;
use crate::hasher::{self, HashQueue};
use crate::scanner::{self, RESERVED_DIR};
use crate::server::{self, AppState};
use crate::store::{self, Db};
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
    #[error("cannot watch the Ring directory: {0}")]
    Watch(#[from] notify_debouncer_full::notify::Error),
}

/// A running Ring service.
pub struct Ring {
    pub root: PathBuf,
    pub db: Db,
    pub hashq: HashQueue,
    pub ring_id: String,
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
        mkdir(root.join(RESERVED_DIR).join("staging"))?;
        for folder in &config.folders {
            mkdir(root.join(folder))?;
        }

        let db = Db::open(&config.db_path())?;
        let ring_id = store::ring_id_hex(&db.lock())?;
        let hashq = hasher::spawn(root.clone(), db.clone());
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
            _watcher: watcher,
        })
    }

    pub fn router(&self) -> Router {
        server::router(AppState {
            db: self.db.clone(),
            ring_id: self.ring_id.clone(),
        })
    }
}
