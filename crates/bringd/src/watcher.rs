//! Filesystem watcher: debounced `notify` events → per-path reconcile.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use bring_core::RelPath;
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::hasher::HashQueue;
use crate::scanner::{self, RESERVED_DIR};
use crate::store::Db;

/// Keeps the OS watch alive; dropping it stops watching.
pub struct WatcherHandle {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

/// What to reconcile after a batch of events.
enum Work {
    Full,
    Paths(BTreeSet<RelPath>),
}

/// Starts watching `root` (which must be canonical).
pub fn spawn(
    root: PathBuf,
    debounce: Duration,
    db: Db,
    hashq: HashQueue,
) -> notify_debouncer_full::notify::Result<WatcherHandle> {
    let (tx, mut rx) = mpsc::unbounded_channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(debounce, None, tx)?;
    debouncer.watch(&root, RecursiveMode::Recursive)?;

    let reserved = root.join(RESERVED_DIR);
    tokio::spawn(async move {
        while let Some(result) = rx.recv().await {
            let work = match result {
                Ok(events) => {
                    let mut paths = BTreeSet::new();
                    let mut full = false;
                    for ev in &events {
                        full |= ev.need_rescan();
                        for p in &ev.paths {
                            if p == &root {
                                full = true;
                            } else if !p.starts_with(&reserved)
                                && let Some(rel) = scanner::rel_of(&root, p)
                            {
                                paths.insert(rel);
                            }
                        }
                    }
                    if full { Work::Full } else { Work::Paths(paths) }
                }
                Err(errors) => {
                    for e in errors {
                        warn!(error = %e, "watcher error; rescanning");
                    }
                    Work::Full
                }
            };
            let (root, db) = (root.clone(), db.clone());
            let res = tokio::task::spawn_blocking(move || match work {
                Work::Full => scanner::reconcile(&root, &db, None),
                Work::Paths(paths) => {
                    let mut out = Vec::new();
                    for p in &paths {
                        debug!(path = %p, "reconcile");
                        out.extend(scanner::reconcile(&root, &db, Some(p))?);
                    }
                    Ok(out)
                }
            })
            .await;
            match res {
                Ok(Ok(to_hash)) => hashq.enqueue_all(to_hash),
                Ok(Err(e)) => warn!(error = %e, "reconcile failed"),
                Err(e) => warn!(error = %e, "reconcile task panicked"),
            }
        }
    });
    Ok(WatcherHandle {
        _debouncer: debouncer,
    })
}
