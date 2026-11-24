//! Ring index access: a shared SQLite connection plus the queries `bringd`
//! runs against it.
//!
//! All writers go through the single connection behind [`Db`], so
//! transactions are serialised and `seq` order equals commit order — which is
//! what makes `GET /index?since=` a correct delta stream.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use bring_core::api::IndexEntry;
use bring_core::db::{self, DbError, Role};
use bring_core::{Blake3Hash, RelPath};
use rusqlite::{Connection, OptionalExtension, params};

/// Shared handle to the Ring index.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Ok(Self::from_conn(db::open(path, Role::Ring)?))
    }

    pub fn from_conn(conn: Connection) -> Self {
        Self(Arc::new(Mutex::new(conn)))
    }

    /// Locks the connection. Only call from blocking contexts.
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` on the blocking thread pool with exclusive access.
    pub async fn call<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || f(&mut db.lock()))
            .await
            .expect("database task panicked")
    }
}

/// Size and modification time of a file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStat {
    pub size: u64,
    pub mtime_ns: i64,
}

impl DiskStat {
    pub fn of(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            size: meta.len(),
            mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
        }
    }
}

/// A row of the `files` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub path: RelPath,
    pub size: u64,
    pub mtime_ns: i64,
    pub mime: Option<String>,
    pub blake3: Option<Blake3Hash>,
    pub thumb: Option<Blake3Hash>,
    pub origin_device_id: Option<i64>,
    pub deleted: bool,
    pub seq: i64,
}

impl FileRow {
    pub fn stat(&self) -> DiskStat {
        DiskStat {
            size: self.size,
            mtime_ns: self.mtime_ns,
        }
    }
}

const FILE_COLS: &str =
    "id, path, size, mtime_ns, mime, blake3, thumb_blake3, origin_device_id, deleted, seq";

fn file_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        path: r.get(1)?,
        size: r.get(2)?,
        mtime_ns: r.get(3)?,
        mime: r.get(4)?,
        blake3: r.get(5)?,
        thumb: r.get(6)?,
        origin_device_id: r.get(7)?,
        deleted: r.get(8)?,
        seq: r.get(9)?,
    })
}

pub fn file_by_path(conn: &Connection, path: &RelPath) -> Result<Option<FileRow>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {FILE_COLS} FROM files WHERE path = ?1"),
            [path],
            file_row,
        )
        .optional()?)
}

pub fn file_by_id(conn: &Connection, id: i64) -> Result<Option<FileRow>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {FILE_COLS} FROM files WHERE id = ?1"),
            [id],
            file_row,
        )
        .optional()?)
}

pub fn guess_mime(path: &RelPath) -> Option<String> {
    mime_guess::from_path(path.as_str())
        .first()
        .map(|m| m.essence_str().to_owned())
}

/// Records a file seen on disk. Returns `true` if it needs (re)hashing,
/// or a thumbnail it should have but lacks (one that failed right after an
/// upload is retried by the next scan, e.g. at startup).
///
/// A live row with the same size and mtime is left untouched (no new `seq`).
pub fn upsert_scanned(conn: &Connection, path: &RelPath, stat: DiskStat) -> Result<bool, DbError> {
    let mime = guess_mime(path);
    match file_by_path(conn, path)? {
        Some(row) if !row.deleted && row.stat() == stat => Ok(row.blake3.is_none()
            || (row.thumb.is_none() && crate::thumbs::is_thumbable(row.mime.as_deref()))),
        Some(row) => {
            let seq = db::next_seq(conn)?;
            // A resurrected tombstone is a new desktop file, not the old upload.
            conn.execute(
                "UPDATE files SET size = ?1, mtime_ns = ?2, mime = ?3, blake3 = NULL,
                   thumb_blake3 = NULL, deleted = 0, seq = ?4,
                   origin_device_id = CASE WHEN deleted = 1 THEN NULL ELSE origin_device_id END
                 WHERE id = ?5",
                params![stat.size, stat.mtime_ns, mime, seq, row.id],
            )?;
            Ok(true)
        }
        None => {
            let seq = db::next_seq(conn)?;
            conn.execute(
                "INSERT INTO files (path, folder, size, mtime_ns, mime, seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![path, path.folder(), stat.size, stat.mtime_ns, mime, seq],
            )?;
            Ok(true)
        }
    }
}

/// Live paths at `dir` or below it (`None` = the whole Ring).
pub fn live_paths_within(
    conn: &Connection,
    dir: Option<&RelPath>,
) -> Result<Vec<RelPath>, DbError> {
    let mut out = Vec::new();
    match dir {
        None => {
            let mut stmt = conn.prepare("SELECT path FROM files WHERE deleted = 0")?;
            for p in stmt.query_map([], |r| r.get(0))? {
                out.push(p?);
            }
        }
        Some(dir) => {
            // Everything starting with "dir/" sorts in ["dir/", "dir0"):
            // '0' is the byte right after '/'. Avoids LIKE-escaping issues.
            let lo = format!("{dir}/");
            let hi = format!("{dir}0");
            let mut stmt = conn.prepare(
                "SELECT path FROM files WHERE deleted = 0
                   AND (path = ?1 OR (path >= ?2 AND path < ?3))",
            )?;
            for p in stmt.query_map(params![dir, lo, hi], |r| r.get(0))? {
                out.push(p?);
            }
        }
    }
    Ok(out)
}

/// Marks a live file as deleted. Returns `true` if a row changed.
pub fn tombstone(conn: &Connection, path: &RelPath) -> Result<bool, DbError> {
    let live: Option<i64> = conn
        .query_row(
            "SELECT id FROM files WHERE path = ?1 AND deleted = 0",
            [path],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = live else {
        return Ok(false);
    };
    let seq = db::next_seq(conn)?;
    conn.execute(
        "UPDATE files SET deleted = 1, seq = ?1 WHERE id = ?2",
        params![seq, id],
    )?;
    Ok(true)
}

/// Stores a computed hash (and thumbnail) if the row still describes the
/// exact version that was hashed. Returns `true` if the row changed.
pub fn set_hash(
    conn: &Connection,
    path: &RelPath,
    hashed: DiskStat,
    blake3: Blake3Hash,
    thumb: Option<Blake3Hash>,
) -> Result<bool, DbError> {
    let Some(row) = file_by_path(conn, path)? else {
        return Ok(false);
    };
    if row.deleted || row.stat() != hashed {
        return Ok(false);
    }
    if row.blake3 == Some(blake3) && row.thumb == thumb {
        return Ok(false);
    }
    let seq = db::next_seq(conn)?;
    conn.execute(
        "UPDATE files SET blake3 = ?1, thumb_blake3 = ?2, seq = ?3 WHERE id = ?4",
        params![blake3, thumb, seq, row.id],
    )?;
    Ok(true)
}

/// One page of the delta stream: rows with `seq > since`, oldest first.
pub fn index_since(
    conn: &Connection,
    since: i64,
    limit: u32,
) -> Result<(Vec<IndexEntry>, bool), DbError> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.path, f.folder, f.size, f.mtime_ns, f.mime, f.blake3, d.slug,
                f.thumb_blake3, f.deleted, f.seq
         FROM files f LEFT JOIN devices d ON d.id = f.origin_device_id
         WHERE f.seq > ?1 ORDER BY f.seq LIMIT ?2",
    )?;
    let mut entries = stmt
        .query_map(params![since, i64::from(limit) + 1], |r| {
            Ok(IndexEntry {
                id: r.get(0)?,
                path: r.get(1)?,
                folder: r.get(2)?,
                size: r.get(3)?,
                mtime_ns: r.get(4)?,
                mime: r.get(5)?,
                blake3: r.get(6)?,
                origin: r.get(7)?,
                thumb: r.get(8)?,
                deleted: r.get(9)?,
                seq: r.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = entries.len() > limit as usize;
    entries.truncate(limit as usize);
    Ok((entries, has_more))
}

/// Hex-encoded Ring identity.
pub fn ring_id_hex(conn: &Connection) -> Result<String, DbError> {
    let id: [u8; 16] = db::ring_meta_blob(conn, "ring_id")?;
    Ok(id.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        db::open_in_memory(Role::Ring).unwrap()
    }

    fn p(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    const S1: DiskStat = DiskStat {
        size: 10,
        mtime_ns: 100,
    };

    #[test]
    fn upsert_is_idempotent_and_bumps_seq_on_change() {
        let c = conn();
        assert!(upsert_scanned(&c, &p("DCIM/a.jpg"), S1).unwrap());
        let r1 = file_by_path(&c, &p("DCIM/a.jpg")).unwrap().unwrap();
        assert_eq!(r1.mime.as_deref(), Some("image/jpeg"));
        // Same stat, still unhashed -> needs hash, but no new seq.
        assert!(upsert_scanned(&c, &p("DCIM/a.jpg"), S1).unwrap());
        assert_eq!(
            file_by_path(&c, &p("DCIM/a.jpg")).unwrap().unwrap().seq,
            r1.seq
        );

        let h = Blake3Hash([7; 32]);
        // Hashed, but the thumbnail failed: the next scan retries it.
        assert!(set_hash(&c, &p("DCIM/a.jpg"), S1, h, None).unwrap());
        assert!(upsert_scanned(&c, &p("DCIM/a.jpg"), S1).unwrap());
        assert!(set_hash(&c, &p("DCIM/a.jpg"), S1, h, Some(Blake3Hash([8; 32]))).unwrap());
        assert!(!upsert_scanned(&c, &p("DCIM/a.jpg"), S1).unwrap());
        // Nothing to thumbnail: hashed is done.
        upsert_scanned(&c, &p("Docs/a.txt"), S1).unwrap();
        assert!(set_hash(&c, &p("Docs/a.txt"), S1, h, None).unwrap());
        assert!(!upsert_scanned(&c, &p("Docs/a.txt"), S1).unwrap());

        let s2 = DiskStat {
            size: 11,
            mtime_ns: 200,
        };
        assert!(upsert_scanned(&c, &p("DCIM/a.jpg"), s2).unwrap());
        let r2 = file_by_path(&c, &p("DCIM/a.jpg")).unwrap().unwrap();
        assert!(r2.seq > r1.seq);
        assert_eq!(r2.blake3, None, "hash must be cleared on change");
    }

    #[test]
    fn set_hash_ignores_stale_versions() {
        let c = conn();
        upsert_scanned(&c, &p("a"), S1).unwrap();
        let stale = DiskStat {
            size: 10,
            mtime_ns: 99,
        };
        assert!(!set_hash(&c, &p("a"), stale, Blake3Hash([1; 32]), None).unwrap());
        assert_eq!(file_by_path(&c, &p("a")).unwrap().unwrap().blake3, None);
    }

    #[test]
    fn tombstone_and_resurrect() {
        let c = conn();
        upsert_scanned(&c, &p("a"), S1).unwrap();
        assert!(tombstone(&c, &p("a")).unwrap());
        assert!(!tombstone(&c, &p("a")).unwrap(), "already deleted");
        let dead = file_by_path(&c, &p("a")).unwrap().unwrap();
        assert!(dead.deleted);
        assert!(upsert_scanned(&c, &p("a"), S1).unwrap());
        let alive = file_by_path(&c, &p("a")).unwrap().unwrap();
        assert!(!alive.deleted && alive.seq > dead.seq && alive.id == dead.id);
    }

    #[test]
    fn live_paths_within_uses_segment_boundaries() {
        let c = conn();
        for s in ["d/a", "d/sub/b", "d", "d0", "d-x/c", "e/f"] {
            upsert_scanned(&c, &p(s), S1).unwrap();
        }
        let mut got = live_paths_within(&c, Some(&p("d"))).unwrap();
        got.sort();
        assert_eq!(got, [p("d"), p("d/a"), p("d/sub/b")]);
        assert_eq!(live_paths_within(&c, None).unwrap().len(), 6);
    }

    #[test]
    fn index_pages() {
        let c = conn();
        for i in 0..5 {
            upsert_scanned(&c, &p(&format!("f{i}")), S1).unwrap();
        }
        let (page, more) = index_since(&c, 0, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert!(more);
        let (page2, more2) = index_since(&c, page[1].seq, 10).unwrap();
        assert_eq!(page2.len(), 3);
        assert!(!more2);
        assert!(page2.iter().all(|e| e.seq > page[1].seq));
    }
}
