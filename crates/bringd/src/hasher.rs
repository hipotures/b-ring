//! Background hashing queue.
//!
//! Paths are hashed one at a time (the disk is the bottleneck). A hash is only
//! stored if the file did not change while it was being read and the index row
//! still describes that exact version; otherwise the watcher will re-queue it.

use std::collections::HashSet;
use std::path::PathBuf;

use bring_core::RelPath;
use bring_core::hash::hash_file;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::store::{self, Db, DiskStat};

/// Handle for enqueueing paths.
#[derive(Clone)]
pub struct HashQueue {
    tx: mpsc::UnboundedSender<RelPath>,
}

impl HashQueue {
    pub fn enqueue(&self, path: RelPath) {
        // The worker only stops at shutdown; dropping work then is fine.
        let _ = self.tx.send(path);
    }

    pub fn enqueue_all(&self, paths: impl IntoIterator<Item = RelPath>) {
        for p in paths {
            self.enqueue(p);
        }
    }
}

/// Starts the worker task.
pub fn spawn(root: PathBuf, db: Db) -> HashQueue {
    let (tx, mut rx) = mpsc::unbounded_channel::<RelPath>();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            // Drain and de-duplicate whatever piled up, keeping order.
            let mut batch = vec![first];
            while let Ok(p) = rx.try_recv() {
                batch.push(p);
            }
            let mut seen = HashSet::new();
            batch.retain(|p| seen.insert(p.clone()));
            for path in batch {
                let (root, db) = (root.clone(), db.clone());
                let res = tokio::task::spawn_blocking(move || process(&root, &db, &path)).await;
                match res {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => warn!(error = %e, "hashing failed"),
                    Err(e) => warn!(error = %e, "hashing task panicked"),
                }
            }
        }
    });
    HashQueue { tx }
}

#[derive(Debug, thiserror::Error)]
enum HashError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] bring_core::db::DbError),
}

fn process(root: &std::path::Path, db: &Db, path: &RelPath) -> Result<(), HashError> {
    let fs_path = path.to_fs_path(root);
    let before = match std::fs::symlink_metadata(&fs_path) {
        Ok(m) if m.is_file() => DiskStat::of(&m),
        _ => return Ok(()), // gone or not a file; the watcher handles it
    };
    match store::file_by_path(&db.lock(), path)? {
        Some(row) if !row.deleted && row.stat() == before && row.blake3.is_none() => {}
        _ => return Ok(()), // not indexed yet, outdated, or already hashed
    }

    let (hash, n) = hash_file(&fs_path)?;
    let after = DiskStat::of(&std::fs::symlink_metadata(&fs_path)?);
    if after != before || n != before.size {
        debug!(%path, "file changed while hashing; waiting for the next event");
        return Ok(());
    }
    store::set_hash(&db.lock(), path, before, hash, None)?;
    debug!(%path, %hash, "hashed");
    Ok(())
}
