//! Chunked, resumable uploads with verified atomic commit.
//!
//! Safety properties (docs/ARCHITECTURE.md §3.5):
//! - chunks land in `<ring>/.bring/staging/<id>.bring.tmp` (same filesystem
//!   as the Ring, so the final link is atomic);
//! - commit re-hashes the whole staging file and refuses on mismatch;
//! - the file is `fsync`ed, then *hard-linked* into place — linking fails if
//!   the target exists, so an existing Ring file is never overwritten, even if
//!   the desktop creates one concurrently — and the directory is `fsync`ed;
//! - the index row and the upload state commit in one transaction, and only
//!   then is the ACK returned; afterwards the staging name is unlinked, so the
//!   committed file has exactly one link (orphans are swept by `cleanup`).
//!
//! The Ring therefore needs a filesystem with hard links (`check_hard_links`
//! verifies that at startup), and every Ring folder must be on the same
//! filesystem as `.bring/staging`.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use axum::http::StatusCode;
use bring_core::api::{
    CreateUploadRequest, CreateUploadResponse, ErrorCode, UploadAck, UploadState, UploadStatus,
};
use bring_core::db::{self, DbError};
use bring_core::hash::{hash_bytes, hash_file};
use bring_core::transfer::{self, DEFAULT_CHUNK_SIZE, UploadPlan};
use bring_core::{Blake3Hash, RelPath};
use rusqlite::{Connection, OptionalExtension, params};
use tracing::{info, warn};

use crate::devices::now_secs;
use crate::scanner::is_reserved;
use crate::server::ApiError;
use crate::server::auth::AuthDevice;
use crate::store::{self, Db, DiskStat, FileRow};

/// Uploads untouched for this long are discarded.
pub const ABANDON_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
/// How many conflict names to try before giving up.
const MAX_CONFLICT_NAMES: u32 = 1000;

/// Everything upload handling needs.
#[derive(Clone)]
pub struct Uploads {
    pub root: PathBuf,
    pub staging: PathBuf,
    pub db: Db,
    locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Uploads {
    pub fn new(root: PathBuf, staging: PathBuf, db: Db) -> Self {
        Self {
            root,
            staging,
            db,
            locks: Arc::default(),
        }
    }

    /// Fails unless the staging directory's filesystem supports hard links
    /// into the Ring (FAT/exFAT and some network filesystems do not).
    pub fn check_hard_links(&self) -> io::Result<()> {
        let probe = self.staging.join("link-probe.bring.tmp");
        let target = self
            .staging
            .parent()
            .unwrap_or(&self.root)
            .join("link-probe");
        let _ = std::fs::remove_file(&target);
        File::create(&probe)?;
        let linked = std::fs::hard_link(&probe, &target);
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&probe);
        linked
    }

    fn staging_path(&self, id: &str) -> PathBuf {
        self.staging.join(format!("{id}.bring.tmp"))
    }

    /// Serialises chunk writes, commit and abort of one upload.
    pub async fn lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let m = self
            .locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_owned())
            .or_default()
            .clone();
        m.lock_owned().await
    }

    fn forget_lock(&self, id: &str) {
        self.locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}

// ---------------------------------------------------------------------------
// Rows & helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct UploadRow {
    id: String,
    target_path: RelPath,
    size: u64,
    mtime_ns: Option<i64>,
    chunk_size: u64,
    chunk_count: u32,
    expected: Blake3Hash,
    state: UploadState,
    result_file_id: Option<i64>,
}

impl UploadRow {
    fn plan(&self) -> UploadPlan {
        UploadPlan {
            size: self.size,
            chunk_size: self.chunk_size,
        }
    }
}

const UPLOAD_COLS: &str = "id, target_path, size, mtime_ns, chunk_size, chunk_count, \
                           expected_blake3, state, result_file_id";

fn upload_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<UploadRow> {
    let state: String = r.get(7)?;
    Ok(UploadRow {
        id: r.get(0)?,
        target_path: r.get(1)?,
        size: r.get(2)?,
        mtime_ns: r.get(3)?,
        chunk_size: r.get(4)?,
        chunk_count: r.get(5)?,
        expected: r.get(6)?,
        state: UploadState::parse(&state).unwrap_or(UploadState::Failed),
        result_file_id: r.get(8)?,
    })
}

/// Loads an upload owned by `dev`; anything else is "not found".
fn load(conn: &Connection, dev: &AuthDevice, id: &str) -> Result<UploadRow, ApiError> {
    if !valid_upload_id(id) {
        return Err(ApiError::not_found("no such upload"));
    }
    conn.query_row(
        &format!("SELECT {UPLOAD_COLS} FROM uploads WHERE id = ?1 AND device_id = ?2"),
        params![id, dev.id],
        upload_row,
    )
    .optional()
    .map_err(DbError::from)?
    .ok_or_else(|| ApiError::not_found("no such upload"))
}

fn received(conn: &Connection, id: &str) -> Result<Vec<u32>, DbError> {
    let mut stmt =
        conn.prepare("SELECT idx FROM upload_chunks WHERE upload_id = ?1 ORDER BY idx")?;
    let out = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(out)
}

fn set_state(conn: &Connection, id: &str, state: UploadState) -> Result<(), DbError> {
    conn.execute(
        "UPDATE uploads SET state = ?1, updated_at = ?2 WHERE id = ?3",
        params![state.as_str(), now_secs(), id],
    )?;
    Ok(())
}

/// A random UUIDv4 string.
fn new_upload_id() -> Result<String, DbError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Upload ids are canonical lowercase UUIDs; anything else is rejected
/// before it gets near a filesystem path.
pub fn valid_upload_id(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

fn io_err(e: io::Error) -> ApiError {
    ApiError::internal(e)
}

/// An ACK for an indexed file, but only if the disk still matches the row.
/// This is what lets a Moon delete its copy, so it must not be stale.
fn verified_ack(root: &Path, row: &FileRow, upload_id: Option<String>) -> Option<UploadAck> {
    let blake3 = row.blake3?;
    if row.deleted {
        return None;
    }
    let meta = std::fs::symlink_metadata(row.path.to_fs_path(root)).ok()?;
    if !meta.is_file() || DiskStat::of(&meta) != row.stat() {
        return None;
    }
    Some(UploadAck {
        upload_id,
        file_id: row.id,
        path: row.path.clone(),
        size: row.size,
        blake3,
        seq: row.seq,
    })
}

fn conflict(message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, ErrorCode::Conflict, message)
}

fn hash_mismatch(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, ErrorCode::HashMismatch, message)
}

// ---------------------------------------------------------------------------
// Create / status
// ---------------------------------------------------------------------------

/// `POST /uploads`. Returns `(created, response)`. Blocking.
pub fn create(
    up: &Uploads,
    dev: &AuthDevice,
    req: &CreateUploadRequest,
) -> Result<(bool, CreateUploadResponse), ApiError> {
    if is_reserved(&req.path) {
        return Err(ApiError::bad_request("path is reserved"));
    }
    let plan = UploadPlan::new(req.size, req.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE))
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let key = transfer::client_key(&req.path, req.size, &req.blake3);
    let conn = up.db.lock();

    // 1. Same content already at the requested path.
    if let Some(row) = store::file_by_path(&conn, &req.path)?
        && row.blake3 == Some(req.blake3)
        && row.size == req.size
        && let Some(ack) = verified_ack(&up.root, &row, None)
    {
        return Ok((false, CreateUploadResponse::Exists { ack }));
    }

    // 2. This device already committed this content (it lost the ACK); the
    //    file may live under a conflict name.
    let committed: Option<(String, i64)> = conn
        .query_row(
            "SELECT id, result_file_id FROM uploads
             WHERE device_id = ?1 AND client_key = ?2 AND state = 'committed'
             ORDER BY updated_at DESC LIMIT 1",
            params![dev.id, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(DbError::from)?;
    if let Some((uid, file_id)) = committed
        && let Some(row) = store::file_by_id(&conn, file_id)?
        && row.blake3 == Some(req.blake3)
        && let Some(ack) = verified_ack(&up.root, &row, Some(uid))
    {
        return Ok((false, CreateUploadResponse::Exists { ack }));
    }

    // 3. Resume an active upload.
    let active: Option<UploadRow> = conn
        .query_row(
            &format!(
                "SELECT {UPLOAD_COLS} FROM uploads
                 WHERE device_id = ?1 AND client_key = ?2 AND state IN ('open', 'committing')"
            ),
            params![dev.id, key],
            upload_row,
        )
        .optional()
        .map_err(DbError::from)?;
    if let Some(u) = active {
        let received = received(&conn, &u.id)?;
        return Ok((
            false,
            CreateUploadResponse::Open {
                upload_id: u.id,
                chunk_size: u.chunk_size,
                chunk_count: u.chunk_count,
                received,
            },
        ));
    }

    // 4. New upload. Row and staging file are created under the DB lock so
    //    the cleanup job never sees one without the other.
    let id = new_upload_id()?;
    let now = now_secs();
    conn.execute(
        "INSERT INTO uploads (id, device_id, client_key, target_path, size, mtime_ns, chunk_size,
           chunk_count, expected_blake3, state, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'open', ?10, ?10)",
        params![
            id,
            dev.id,
            key,
            req.path,
            req.size,
            req.mtime_ns,
            plan.chunk_size,
            plan.chunk_count(),
            req.blake3,
            now
        ],
    )
    .map_err(DbError::from)?;
    let staging = File::create(up.staging_path(&id)).map_err(io_err)?;
    if let Err(e) = staging.set_len(req.size) {
        conn.execute("DELETE FROM uploads WHERE id = ?1", [&id])
            .map_err(DbError::from)?;
        let _ = std::fs::remove_file(up.staging_path(&id));
        return Err(ApiError::internal(format!(
            "cannot allocate staging file: {e}"
        )));
    }
    Ok((
        true,
        CreateUploadResponse::Open {
            upload_id: id,
            chunk_size: plan.chunk_size,
            chunk_count: plan.chunk_count(),
            received: Vec::new(),
        },
    ))
}

/// `GET /uploads/{id}`. Blocking.
pub fn status(up: &Uploads, dev: &AuthDevice, id: &str) -> Result<UploadStatus, ApiError> {
    let conn = up.db.lock();
    let u = load(&conn, dev, id)?;
    Ok(UploadStatus {
        received: received(&conn, &u.id)?,
        upload_id: u.id,
        state: u.state,
        chunk_size: u.chunk_size,
        chunk_count: u.chunk_count,
    })
}

// ---------------------------------------------------------------------------
// Chunks
// ---------------------------------------------------------------------------

/// `POST /uploads/{id}/chunks/{index}`. Caller holds the upload lock. Blocking.
pub fn put_chunk(
    up: &Uploads,
    dev: &AuthDevice,
    id: &str,
    index: u32,
    claimed: Blake3Hash,
    body: &[u8],
) -> Result<(), ApiError> {
    let u = load(&up.db.lock(), dev, id)?;
    if u.state != UploadState::Open {
        return Err(conflict("upload is not open"));
    }
    let (offset, len) = u
        .plan()
        .chunk_range(index)
        .ok_or_else(|| ApiError::bad_request("chunk index out of range"))?;
    if body.len() as u64 != len {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::BadChunkLength,
            format!("chunk {index} must be {len} bytes, got {}", body.len()),
        ));
    }
    let actual = hash_bytes(body);
    if actual != claimed {
        return Err(hash_mismatch(format!("chunk {index} hash mismatch")));
    }
    // Retries simply overwrite the same range: idempotent.
    let f = OpenOptions::new()
        .write(true)
        .open(up.staging_path(&u.id))
        .map_err(io_err)?;
    f.write_all_at(body, offset).map_err(io_err)?;

    let conn = up.db.lock();
    conn.execute(
        "INSERT OR REPLACE INTO upload_chunks (upload_id, idx, len, blake3) VALUES (?1, ?2, ?3, ?4)",
        params![u.id, index, len, actual],
    )
    .map_err(DbError::from)?;
    conn.execute(
        "UPDATE uploads SET updated_at = ?1 WHERE id = ?2",
        params![now_secs(), u.id],
    )
    .map_err(DbError::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Commit
// ---------------------------------------------------------------------------

/// Candidate names: `a.jpg`, `a (slug).jpg`, `a (slug 2).jpg`, ...
fn candidates(path: &RelPath, slug: &str) -> impl Iterator<Item = RelPath> {
    let name = path.file_name().to_owned();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_owned(), name[i..].to_owned()),
        _ => (name.clone(), String::new()),
    };
    let path = path.clone();
    let slug = slug.to_owned();
    (1..=MAX_CONFLICT_NAMES).filter_map(move |n| match n {
        1 => Some(path.clone()),
        2 => path.with_file_name(&format!("{stem} ({slug}){ext}")).ok(),
        n => path
            .with_file_name(&format!("{stem} ({slug} {}){ext}", n - 1))
            .ok(),
    })
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// What the commit did with the verified staging file.
enum Placed {
    Linked(RelPath),
    /// Identical content already existed at this path.
    Existing(FileRow),
}

/// Links the staging file to the first free candidate path. Caller holds the
/// DB transaction, so the watcher cannot index the new file half-way.
fn place(
    up: &Uploads,
    conn: &Connection,
    u: &UploadRow,
    dev: &AuthDevice,
) -> Result<Placed, ApiError> {
    let staging = up.staging_path(&u.id);
    for cand in candidates(&u.target_path, &dev.slug) {
        let fs = cand.to_fs_path(&up.root);
        match std::fs::symlink_metadata(&fs) {
            Ok(meta) => {
                if meta.is_file()
                    && let Some(row) = store::file_by_path(conn, &cand)?
                    && row.blake3 == Some(u.expected)
                    && verified_ack(&up.root, &row, None).is_some()
                {
                    return Ok(Placed::Existing(row));
                }
                continue; // occupied by something else: never overwrite
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(e)),
        }
        let parent = fs.parent().expect("ring paths have a parent");
        if std::fs::create_dir_all(parent).is_err() {
            return Err(conflict(
                "target directory cannot be created (a file is in the way)",
            ));
        }
        match std::fs::hard_link(&staging, &fs) {
            Ok(()) => {
                fsync_dir(parent).map_err(io_err)?;
                return Ok(Placed::Linked(cand));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                return Err(ApiError::internal(format!(
                    "{} is on a different filesystem than .bring/staging; \
                     the Ring must live on one filesystem",
                    cand.folder()
                )));
            }
            Err(e) => return Err(io_err(e)),
        }
    }
    Err(conflict("no free name for this file"))
}

/// Writes the committed file's index row. Returns the row.
fn record_file(
    conn: &Connection,
    path: &RelPath,
    stat: DiskStat,
    blake3: Blake3Hash,
    device_id: i64,
) -> Result<FileRow, DbError> {
    let seq = db::next_seq(conn)?;
    conn.execute(
        "INSERT INTO files (path, folder, size, mtime_ns, mime, blake3, origin_device_id, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (path) DO UPDATE SET
           size = excluded.size, mtime_ns = excluded.mtime_ns, mime = excluded.mime,
           blake3 = excluded.blake3, thumb_blake3 = NULL,
           origin_device_id = excluded.origin_device_id, deleted = 0, seq = excluded.seq",
        params![
            path,
            path.folder(),
            stat.size,
            stat.mtime_ns,
            store::guess_mime(path),
            blake3,
            device_id,
            seq
        ],
    )?;
    Ok(store::file_by_path(conn, path)?.expect("row just written"))
}

/// Outcome of a successful commit.
pub struct Committed {
    pub ack: UploadAck,
    /// True if this call created the file (false for repeats/dedupe).
    pub new_file: bool,
}

/// `POST /uploads/{id}/commit`. Caller holds the upload lock. Blocking.
pub fn commit(up: &Uploads, dev: &AuthDevice, id: &str) -> Result<Committed, ApiError> {
    let u = load(&up.db.lock(), dev, id)?;
    match u.state {
        UploadState::Committed => {
            return repeat_ack(up, &u).map(|ack| Committed {
                ack,
                new_file: false,
            });
        }
        UploadState::Failed => {
            return Err(hash_mismatch(
                "upload failed verification; create a new upload",
            ));
        }
        UploadState::Open | UploadState::Committing => {}
    }

    {
        let conn = up.db.lock();
        let got = received(&conn, &u.id)?;
        if got.len() as u32 != u.chunk_count {
            let have: std::collections::HashSet<u32> = got.into_iter().collect();
            let missing: Vec<String> = (0..u.chunk_count)
                .filter(|i| !have.contains(i))
                .take(20)
                .map(|i| i.to_string())
                .collect();
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                ErrorCode::MissingChunks,
                format!("missing chunks: {}", missing.join(",")),
            ));
        }
        set_state(&conn, &u.id, UploadState::Committing)?;
    }

    // Verify the whole file without holding the DB lock.
    let staging = up.staging_path(&u.id);
    let verified = hash_file(&staging);
    let ok = matches!(&verified, Ok((h, n)) if *h == u.expected && *n == u.size);
    if !ok {
        set_state(&up.db.lock(), &u.id, UploadState::Failed)?;
        let _ = std::fs::remove_file(&staging);
        return Err(match verified {
            Ok((h, _)) => hash_mismatch(format!(
                "file hash mismatch: expected {}, got {h}",
                u.expected
            )),
            Err(e) => ApiError::internal(format!("staging file unreadable: {e}")),
        });
    }

    let f = OpenOptions::new()
        .write(true)
        .open(&staging)
        .map_err(io_err)?;
    if let Some(ns) = u.mtime_ns.filter(|ns| *ns >= 0) {
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_nanos(ns as u64));
    }
    f.sync_all().map_err(io_err)?;
    drop(f);

    let mut conn = up.db.lock();
    let tx = conn.transaction().map_err(DbError::from)?;
    let placed = place(up, &tx, &u, dev)?;
    let (row, new_file) = match placed {
        Placed::Linked(path) => {
            let meta = std::fs::symlink_metadata(path.to_fs_path(&up.root)).map_err(io_err)?;
            (
                record_file(&tx, &path, DiskStat::of(&meta), u.expected, dev.id)?,
                true,
            )
        }
        Placed::Existing(row) => (row, false),
    };
    tx.execute(
        "UPDATE uploads SET state = 'committed', result_file_id = ?1, updated_at = ?2 WHERE id = ?3",
        params![row.id, now_secs(), u.id],
    )
    .map_err(DbError::from)?;
    tx.execute("DELETE FROM upload_chunks WHERE upload_id = ?1", [&u.id])
        .map_err(DbError::from)?;
    tx.commit().map_err(DbError::from)?;
    drop(conn);

    if let Err(e) = std::fs::remove_file(&staging) {
        warn!(error = %e, "cannot remove staging file; cleanup will retry");
    }
    up.forget_lock(&u.id);
    info!(path = %row.path, device = %dev.slug, size = row.size, "upload committed");
    Ok(Committed {
        ack: UploadAck {
            upload_id: Some(u.id),
            file_id: row.id,
            path: row.path,
            size: row.size,
            blake3: u.expected,
            seq: row.seq,
        },
        new_file,
    })
}

/// The ACK for an already committed upload — only while the file is still
/// there, unchanged. Otherwise the Moon must upload again.
fn repeat_ack(up: &Uploads, u: &UploadRow) -> Result<UploadAck, ApiError> {
    let conn = up.db.lock();
    let row = match u.result_file_id {
        Some(fid) => store::file_by_id(&conn, fid)?,
        None => None,
    };
    row.filter(|r| r.blake3 == Some(u.expected))
        .and_then(|r| verified_ack(&up.root, &r, Some(u.id.clone())))
        .ok_or_else(|| conflict("the committed file is no longer on the Ring; upload again"))
}

// ---------------------------------------------------------------------------
// Abort / maintenance
// ---------------------------------------------------------------------------

/// `DELETE /uploads/{id}`. Caller holds the upload lock. Blocking.
pub fn abort(up: &Uploads, dev: &AuthDevice, id: &str) -> Result<(), ApiError> {
    let conn = up.db.lock();
    let u = load(&conn, dev, id)?;
    if u.state == UploadState::Committed {
        return Err(conflict("upload is already committed"));
    }
    conn.execute("DELETE FROM uploads WHERE id = ?1", [&u.id])
        .map_err(DbError::from)?;
    drop(conn);
    let _ = std::fs::remove_file(up.staging_path(&u.id));
    up.forget_lock(&u.id);
    Ok(())
}

/// After a crash, `committing` uploads are retried from scratch by the Moon.
pub fn recover(up: &Uploads) -> Result<usize, DbError> {
    Ok(up.db.lock().execute(
        "UPDATE uploads SET state = 'open' WHERE state = 'committing'",
        [],
    )?)
}

/// Removes abandoned uploads and orphaned staging files. Blocking.
pub fn cleanup(up: &Uploads, now: i64, max_age: Duration) -> Result<usize, DbError> {
    let conn = up.db.lock();
    let cutoff = now - max_age.as_secs() as i64;
    let stale: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM uploads WHERE state IN ('open', 'failed') AND updated_at < ?1",
        )?;
        stmt.query_map([cutoff], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    for id in &stale {
        conn.execute("DELETE FROM uploads WHERE id = ?1", [id])?;
    }
    // Any staging file not belonging to an active upload is garbage (e.g. a
    // crash between commit and unlink). Safe: creation happens under this lock.
    let active: std::collections::HashSet<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM uploads WHERE state IN ('open', 'committing')")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let mut removed = stale.len();
    if let Ok(entries) = std::fs::read_dir(&up.staging) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let id = name.strip_suffix(".bring.tmp").unwrap_or(&name);
            if !active.contains(id) {
                let _ = std::fs::remove_file(e.path());
                removed += 1;
            }
        }
    }
    if removed > 0 {
        info!(removed, "cleaned up abandoned uploads");
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_ids() {
        let id = new_upload_id().unwrap();
        assert!(valid_upload_id(&id), "{id}");
        assert_eq!(&id[14..15], "4");
        assert!(!valid_upload_id("../../etc/passwd"));
        assert!(!valid_upload_id(&id.to_uppercase()));
        assert!(!valid_upload_id(&id[..35]));
    }

    #[test]
    fn conflict_candidates() {
        let p = RelPath::new("DCIM/IMG_0042.jpg").unwrap();
        let c: Vec<String> = candidates(&p, "pixel-8")
            .take(4)
            .map(|p| p.to_string())
            .collect();
        assert_eq!(
            c,
            [
                "DCIM/IMG_0042.jpg",
                "DCIM/IMG_0042 (pixel-8).jpg",
                "DCIM/IMG_0042 (pixel-8 2).jpg",
                "DCIM/IMG_0042 (pixel-8 3).jpg"
            ]
        );
        let dot = RelPath::new(".profile").unwrap();
        assert_eq!(
            candidates(&dot, "q").nth(1).unwrap().as_str(),
            ".profile (q)"
        );
        let noext = RelPath::new("README").unwrap();
        assert_eq!(
            candidates(&noext, "q").nth(1).unwrap().as_str(),
            "README (q)"
        );
        assert_eq!(candidates(&p, "x").count(), MAX_CONFLICT_NAMES as usize);
    }
}
