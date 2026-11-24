//! Reconciles the index with what is actually on disk.
//!
//! Used for the full scan on start and, per path, by the watcher. The disk is
//! authoritative: whatever events say, we `stat` and walk the real state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bring_core::RelPath;
use bring_core::db::DbError;
use tracing::warn;

use crate::store::{self, Db, DiskStat};

/// First path segment reserved for `bringd` itself (staging etc.).
pub const RESERVED_DIR: &str = ".bring";

/// True for paths inside the reserved `.bring/` directory.
pub fn is_reserved(path: &RelPath) -> bool {
    path.folder() == RESERVED_DIR
}

/// Converts an absolute path under `root` into a `RelPath`.
///
/// Returns `None` for the root itself, reserved paths, and names that cannot
/// be represented (non-UTF-8 / non-NFC; those are logged and skipped).
pub fn rel_of(root: &Path, path: &Path) -> Option<RelPath> {
    if path == root {
        return None;
    }
    match RelPath::from_fs_path(root, path) {
        Ok(rel) if !is_reserved(&rel) => Some(rel),
        Ok(_) => None,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "skipping unrepresentable path");
            None
        }
    }
}

/// Result of walking a directory tree.
#[derive(Debug, Default)]
struct Walk {
    files: Vec<(RelPath, DiskStat)>,
    /// Entries that could not be read. Their index rows must not be
    /// tombstoned: "unreadable" is not "deleted".
    unreadable: Vec<RelPath>,
}

fn walk(root: &Path, start: &Path, out: &mut Walk) {
    let mut stack: Vec<PathBuf> = vec![start.to_owned()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(dir = %dir.display(), error = %e, "cannot read directory");
                if let Some(rel) = rel_of(root, &dir) {
                    out.unreadable.push(rel);
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(rel) = rel_of(root, &path) else {
                continue;
            };
            // `file_type` does not follow symlinks: links are not Ring files.
            let Ok(ft) = entry.file_type() else {
                out.unreadable.push(rel);
                continue;
            };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                match entry.metadata() {
                    Ok(meta) => out.files.push((rel, DiskStat::of(&meta))),
                    Err(_) => out.unreadable.push(rel),
                }
            }
        }
    }
}

const BATCH: usize = 256;

/// Brings the index in line with the disk for `scope` (`None` = whole Ring).
///
/// Blocking. Returns the paths that need hashing.
pub fn reconcile(root: &Path, db: &Db, scope: Option<&RelPath>) -> Result<Vec<RelPath>, DbError> {
    if scope.is_some_and(is_reserved) {
        return Ok(Vec::new());
    }
    let target = scope.map_or_else(|| root.to_owned(), |s| s.to_fs_path(root));
    let mut found = Walk::default();
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_dir() => walk(root, &target, &mut found),
        Ok(meta) if meta.is_file() => {
            if let Some(s) = scope {
                found.files.push((s.clone(), DiskStat::of(&meta)));
            }
        }
        // Gone, a symlink, or a special file: nothing indexable here.
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            warn!(path = %target.display(), error = %e, "cannot stat");
            return Ok(Vec::new());
        }
    }

    let mut to_hash = Vec::new();
    for batch in found.files.chunks(BATCH) {
        let mut conn = db.lock();
        let tx = conn.transaction()?;
        for (rel, stat) in batch {
            if store::upsert_scanned(&tx, rel, *stat)? {
                to_hash.push(rel.clone());
            }
        }
        tx.commit()?;
    }

    // Tombstone live rows in scope that are no longer on disk.
    let seen: HashSet<&RelPath> = found.files.iter().map(|(r, _)| r).collect();
    let mut conn = db.lock();
    let tx = conn.transaction()?;
    for path in store::live_paths_within(&tx, scope)? {
        if seen.contains(&path) || found.unreadable.iter().any(|u| path.is_within(u)) {
            continue;
        }
        store::tombstone(&tx, &path)?;
    }
    tx.commit()?;
    Ok(to_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bring_core::db::{Role, open_in_memory};
    use std::fs;

    fn setup() -> (tempfile::TempDir, PathBuf, Db) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let db = Db::from_conn(open_in_memory(Role::Ring).unwrap());
        (dir, root, db)
    }

    fn p(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    fn live(db: &Db) -> Vec<String> {
        let mut v: Vec<String> = store::live_paths_within(&db.lock(), None)
            .unwrap()
            .into_iter()
            .map(|p| p.to_string())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn full_scan_skips_reserved_and_symlinks() {
        let (_d, root, db) = setup();
        fs::create_dir_all(root.join("DCIM/Cam")).unwrap();
        fs::create_dir_all(root.join(".bring/staging")).unwrap();
        fs::write(root.join("DCIM/Cam/a.jpg"), b"a").unwrap();
        fs::write(root.join("top.txt"), b"t").unwrap();
        fs::write(root.join(".bring/staging/x.bring.tmp"), b"x").unwrap();
        std::os::unix::fs::symlink(root.join("top.txt"), root.join("link.txt")).unwrap();

        let to_hash = reconcile(&root, &db, None).unwrap();
        assert_eq!(to_hash.len(), 2);
        assert_eq!(live(&db), ["DCIM/Cam/a.jpg", "top.txt"]);
    }

    #[test]
    fn scoped_reconcile_handles_delete_and_dir_moves() {
        let (_d, root, db) = setup();
        fs::create_dir_all(root.join("A/sub")).unwrap();
        fs::write(root.join("A/sub/1"), b"1").unwrap();
        fs::write(root.join("A/2"), b"2").unwrap();
        reconcile(&root, &db, None).unwrap();

        // Directory rename A -> B, reconciled per event path.
        fs::rename(root.join("A"), root.join("B")).unwrap();
        reconcile(&root, &db, Some(&p("A"))).unwrap();
        reconcile(&root, &db, Some(&p("B"))).unwrap();
        assert_eq!(live(&db), ["B/2", "B/sub/1"]);

        // File replaced by directory at the same path.
        fs::remove_file(root.join("B/2")).unwrap();
        fs::create_dir(root.join("B/2")).unwrap();
        fs::write(root.join("B/2/inner"), b"i").unwrap();
        reconcile(&root, &db, Some(&p("B/2"))).unwrap();
        assert_eq!(live(&db), ["B/2/inner", "B/sub/1"]);
    }

    #[test]
    fn unreadable_dir_is_not_tombstoned() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root, db) = setup();
        fs::create_dir(root.join("locked")).unwrap();
        fs::write(root.join("locked/f"), b"f").unwrap();
        reconcile(&root, &db, None).unwrap();
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = fs::read_dir(root.join("locked")).is_ok(); // running as root
        reconcile(&root, &db, None).unwrap();
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        if !readable_anyway {
            assert_eq!(live(&db), ["locked/f"]);
        }
    }
}
