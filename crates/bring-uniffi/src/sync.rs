//! Delta sync, local scan, hash-then-upload outbox and offload
//! (docs/ARCHITECTURE.md §3.4, §3.5, §5).
//!
//! Invariants:
//! - A local file is deleted only after `offload::decide` says `Offload`,
//!   i.e. a verified ACK matches the hash snapshot and the file is unchanged.
//! - The database is updated *before* the local file is deleted. A crash in
//!   between leaves a `remote-only` row and a file on disk; the next scan
//!   turns it back into an upload that dedupes to the same ACK.
//! - Remote actions never delete local data (§5.1).

use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bring_core::api::{
    CHUNK_HASH_HEADER, CreateUploadRequest, CreateUploadResponse, INDEX_MAX_LIMIT, IndexEntry,
    IndexResponse, UploadAck, UploadState, UploadStatus,
};
use bring_core::hash::{hash_bytes, hash_file};
use bring_core::offload::{self, Decision, HashSnapshot, LocalStat, VerifiedAck};
use bring_core::transfer::UploadPlan;
use bring_core::{FileState, RelPath};
use rusqlite::{Connection, params};

use crate::error::MoonError;
use crate::store::{self, EntryRow, OutboxRow, WatchedFolder};
use crate::{Moon, Session, TransferListener, now_secs};

/// Result of [`Moon::sync_index`].
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct IndexReport {
    /// Files new to this device (or replaced on the Ring) in the delta.
    /// Metadata-only echoes, e.g. our own upload coming back with its
    /// thumbnail, do not count.
    pub received: u32,
    pub cursor: i64,
}

/// Result of [`Moon::scan_local`].
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct ScanReport {
    pub new_files: u32,
    pub changed: u32,
    pub gone: u32,
    /// Files that cannot be mapped (bad names, two files for one Ring path).
    pub skipped: u32,
    /// Watched folders that are configured but not present (unmounted).
    pub unavailable_folders: u32,
}

/// Result of [`Moon::process_outbox`].
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct OutboxReport {
    /// Files whose bytes were sent.
    pub uploaded: u32,
    /// Files the Ring already had (no bytes sent).
    pub deduplicated: u32,
    /// Local copies deleted after a verified ACK (now `remote-only`).
    pub offloaded: u32,
    /// Verified and kept locally (now `synced`).
    pub kept: u32,
    pub failed: u32,
    /// Entries still waiting in the outbox.
    pub remaining: u32,
}

/// Result of [`Moon::free_local`].
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct FreeReport {
    /// Local copies deleted (the entries are now `remote-only`).
    pub freed: u32,
    pub freed_bytes: u64,
    /// Entries left alone: not synced, waiting for an upload, or the local
    /// file is gone or no longer what was verified against the Ring.
    pub skipped: u32,
}

/// Result of [`Moon::run_sync`].
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct SyncReport {
    pub index: IndexReport,
    pub scan: ScanReport,
    pub outbox: OutboxReport,
}

// ---------------------------------------------------------------------------
// Local files
// ---------------------------------------------------------------------------

pub(crate) fn local_stat(path: &Path) -> io::Result<Option<LocalStat>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(Some(LocalStat {
            size: m.len(),
            mtime_ns: m.mtime() * 1_000_000_000 + m.mtime_nsec(),
        })),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Deletes the local copies of `synced` entries, which become `remote-only`
/// (the same rule as offload, §3.5 step 5, chosen by the user). Only an
/// entry the Ring holds verified bytes for qualifies, and only while the
/// local file still has the size and mtime that were verified; anything
/// else is skipped. Database first, then the delete; the Ring is never
/// touched (Moon cannot delete Ring files at all).
pub(crate) fn free_local(c: &mut Connection, ids: &[i64]) -> Result<FreeReport, MoonError> {
    let mut report = FreeReport::default();
    for &id in ids {
        let tx = c.transaction()?;
        let row = store::entry_by_id(&tx, id)?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM outbox WHERE entry_id = ?1)",
            [id],
            |r| r.get(0),
        )?;
        let target = row.and_then(
            |r| match (r.state, r.local_path, r.remote_file_id, r.blake3) {
                (FileState::Synced, Some(local), Some(_), Some(_)) if !pending => Some((
                    local,
                    LocalStat {
                        size: r.size,
                        mtime_ns: r.mtime_ns,
                    },
                )),
                _ => None,
            },
        );
        let Some((local, verified)) = target else {
            report.skipped += 1;
            continue;
        };
        let path = Path::new(&local);
        if local_stat(path)? != Some(verified) {
            // Edited or gone: the next scan sorts it out.
            report.skipped += 1;
            continue;
        }
        tx.execute(
            "UPDATE entries SET state = 'remote-only', local_path = NULL WHERE id = ?1",
            [id],
        )?;
        tx.commit()?;
        // Last check right before deleting; if it was touched in between,
        // keep it: the row is remote-only, so the next scan uploads it.
        if local_stat(path)? == Some(verified) {
            fs::remove_file(path)?;
            report.freed += 1;
            report.freed_bytes += verified.size;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}

/// A regular file found under a watched folder.
struct Found {
    local: String,
    rel: String,
    stat: LocalStat,
}

/// Walks `root`, skipping dot-files (Android's `.pending-*`, `.trashed-*`,
/// `.thumbnails`) and symlinks. Unreadable subdirectories are skipped too —
/// and their files are reported in `unreadable` so they are not mistaken for
/// deletions.
fn walk(root: &Path, unreadable: &mut Vec<PathBuf>) -> Vec<Found> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            unreadable.push(dir);
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let (Some(local), Ok(rel)) = (path.to_str(), path.strip_prefix(root)) else {
                    continue;
                };
                let Some(rel) = rel.to_str() else { continue };
                if let Ok(Some(stat)) = local_stat(&path) {
                    out.push(Found {
                        local: local.to_owned(),
                        rel: rel.to_owned(),
                        stat,
                    });
                }
            }
        }
    }
    out
}

fn is_under(path: &str, dir: &Path) -> bool {
    Path::new(path).starts_with(dir)
}

/// Applies a scan of the watched folders to the index (§5.2, §5.3).
pub(crate) fn apply_scan(
    c: &mut Connection,
    folders: &[WatchedFolder],
) -> Result<ScanReport, MoonError> {
    let mut report = ScanReport::default();
    // Never treat files under an unreadable or missing root as deleted.
    let mut unavailable: Vec<PathBuf> = Vec::new();
    let mut found: Vec<(&WatchedFolder, Vec<Found>)> = Vec::new();
    for f in folders.iter().filter(|f| f.enabled) {
        let root = Path::new(&f.local_dir);
        if !root.is_dir() {
            report.unavailable_folders += 1;
            unavailable.push(root.to_path_buf());
            continue;
        }
        found.push((f, walk(root, &mut unavailable)));
    }

    let tx = c.transaction()?;
    let mut seen: HashSet<String> = HashSet::new();
    for (f, files) in &found {
        for file in files {
            let Ok(ring_path) = RelPath::new(&format!("{}/{}", f.ring_folder, file.rel)) else {
                report.skipped += 1;
                continue;
            };
            seen.insert(file.local.clone());
            if let Some(r) = store::entry_by_local_path(&tx, &file.local)? {
                if (r.size, r.mtime_ns) == (file.stat.size, file.stat.mtime_ns) {
                    continue;
                }
                report.changed += 1;
                // Edited locally: the new content is a new upload; a synced
                // file goes back to local-only and is uploaded under a
                // conflict name, never over the Ring copy (§5.3).
                tx.execute(
                    "UPDATE entries SET state = 'local-only', size = ?1, mtime_ns = ?2,
                       blake3 = NULL, remote_file_id = NULL, thumb_blake3 = NULL
                     WHERE id = ?3",
                    params![file.stat.size, file.stat.mtime_ns, r.id],
                )?;
                continue;
            }
            match store::entry_by_ring_path(&tx, ring_path.as_str())? {
                None => {
                    store::insert_local(
                        &tx,
                        ring_path.as_str(),
                        &file.local,
                        file.stat.size,
                        file.stat.mtime_ns,
                        None,
                    )?;
                    report.new_files += 1;
                }
                // The Ring has a file at this path and we have none locally
                // (e.g. offloaded, then the file reappeared). Upload it: an
                // identical file dedupes, a different one gets a conflict name.
                Some(r) if r.local_path.is_none() => {
                    tx.execute(
                        "UPDATE entries SET state = 'local-only', local_path = ?1, size = ?2,
                           mtime_ns = ?3, blake3 = NULL, remote_file_id = NULL,
                           thumb_blake3 = NULL, suppressed = 0
                         WHERE id = ?4",
                        params![file.local, file.stat.size, file.stat.mtime_ns, r.id],
                    )?;
                    report.new_files += 1;
                }
                Some(_) => report.skipped += 1,
            }
        }
    }

    // Tracked local files that disappeared (§5.2).
    let tracked: Vec<(i64, String, FileState)> = {
        let mut stmt =
            tx.prepare("SELECT id, local_path, state FROM entries WHERE local_path IS NOT NULL")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, local, state) in tracked {
        if seen.contains(&local) || unavailable.iter().any(|d| is_under(&local, d)) {
            continue;
        }
        if local_stat(Path::new(&local))?.is_some() {
            continue; // outside watched folders, still there
        }
        report.gone += 1;
        match state {
            FileState::Synced | FileState::RemoteOnly => {
                tx.execute(
                    "UPDATE entries SET state = 'remote-only', local_path = NULL WHERE id = ?1",
                    [id],
                )?;
            }
            // Never reached the Ring: forget it (the outbox row cascades).
            FileState::LocalOnly => {
                tx.execute("DELETE FROM entries WHERE id = ?1", [id])?;
            }
        }
    }
    tx.commit()?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/// Applies one index entry from the Ring. True when it brings a file this
/// device did not know (or new content for a remote-only one).
pub(crate) fn apply_index_entry(c: &Connection, e: &IndexEntry) -> Result<bool, MoonError> {
    let row = store::entry_by_ring_path(c, e.path.as_str())?;
    if e.deleted {
        let Some(r) = row else { return Ok(false) };
        if r.remote_file_id != Some(e.id) {
            return Ok(false);
        }
        match r.state {
            FileState::RemoteOnly => {
                c.execute("DELETE FROM entries WHERE id = ?1", [r.id])?;
            }
            // The Ring dropped a file we still have: keep ours, never upload
            // it again (§5.1).
            FileState::Synced => {
                c.execute(
                    "UPDATE entries SET state = 'local-only', suppressed = 1,
                       remote_file_id = NULL, thumb_blake3 = NULL
                     WHERE id = ?1",
                    [r.id],
                )?;
            }
            FileState::LocalOnly => {}
        }
        return Ok(false);
    }
    let news = match &row {
        None => true,
        Some(r) => r.state == FileState::RemoteOnly && e.blake3.is_some() && e.blake3 != r.blake3,
    };
    match row {
        None => {
            c.execute(
                "INSERT INTO entries (ring_path, remote_file_id, size, mtime_ns, mime, blake3,
                   thumb_blake3, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'remote-only')",
                params![e.path, e.id, e.size, e.mtime_ns, e.mime, e.blake3, e.thumb],
            )?;
        }
        Some(r) => match r.state {
            FileState::RemoteOnly => {
                c.execute(
                    "UPDATE entries SET remote_file_id = ?1, size = ?2, mtime_ns = ?3, mime = ?4,
                       blake3 = ?5, thumb_blake3 = ?6
                     WHERE id = ?7",
                    params![e.id, e.size, e.mtime_ns, e.mime, e.blake3, e.thumb, r.id],
                )?;
            }
            FileState::Synced if e.blake3.is_none() || e.blake3 == r.blake3 => {
                c.execute(
                    "UPDATE entries SET remote_file_id = ?1, mime = COALESCE(?2, mime),
                       thumb_blake3 = ?3
                     WHERE id = ?4",
                    params![e.id, e.mime, e.thumb, r.id],
                )?;
            }
            // The desktop replaced the version we have: same as a Ring-side
            // delete of our version (§5.1). Ours stays local, not re-uploaded.
            FileState::Synced => {
                c.execute(
                    "UPDATE entries SET state = 'local-only', suppressed = 1,
                       remote_file_id = NULL, thumb_blake3 = NULL
                     WHERE id = ?1",
                    [r.id],
                )?;
            }
            // A pending upload owns this path; its ACK resolves the clash
            // (conflict name + cursor rewind).
            FileState::LocalOnly => {}
        },
    }
    Ok(news)
}

// ---------------------------------------------------------------------------
// Upload
// ---------------------------------------------------------------------------

/// How a file reached the Ring.
enum Sent {
    Uploaded(UploadAck),
    Deduplicated(UploadAck),
}

/// One outbox item, ready to go.
struct Job {
    entry: EntryRow,
    outbox: OutboxRow,
    snapshot: HashSnapshot,
    local: String,
}

impl Moon {
    fn check_cancel(&self) -> Result<(), MoonError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(MoonError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn sync_index_inner(&self, s: &Session) -> Result<IndexReport, MoonError> {
        let mut report = IndexReport::default();
        loop {
            self.check_cancel()?;
            let since = store::cursor(&self.db(), s.ring.id)?;
            let page: IndexResponse = s
                .json(s.get(&format!("/index?since={since}&limit={INDEX_MAX_LIMIT}")))
                .await?;
            {
                let mut db = self.db();
                let tx = db.transaction()?;
                for e in &page.entries {
                    if apply_index_entry(&tx, e)? {
                        report.received += 1;
                    }
                }
                store::set_cursor(&tx, s.ring.id, page.next_since)?;
                tx.commit()?;
            }
            report.cursor = page.next_since;
            if !page.has_more {
                return Ok(report);
            }
        }
    }

    pub(crate) fn scan_local_inner(&self) -> Result<ScanReport, MoonError> {
        let folders = store::watched_folders(&self.db())?;
        apply_scan(&mut self.db(), &folders)
    }

    /// Hash-first (§3.5 step 0): every pending `local-only` entry gets a full
    /// BLAKE3 and a size/mtime snapshot before anything is sent.
    fn prepare_outbox(&self) -> Result<(), MoonError> {
        let todo: Vec<EntryRow> = {
            let db = self.db();
            let mut stmt = db.prepare(&format!(
                "SELECT {} FROM entries e LEFT JOIN outbox o ON o.entry_id = e.id
                 WHERE e.state = 'local-only' AND e.suppressed = 0 AND e.local_path IS NOT NULL
                   AND (o.entry_id IS NULL OR e.blake3 IS NULL
                        OR o.hashed_size != e.size OR o.hashed_mtime_ns != e.mtime_ns)",
                store::ENTRY_COLS
                    .split(", ")
                    .map(|c| format!("e.{c}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            stmt.query_map([], store::entry_from_row)?
                .collect::<Result<_, _>>()?
        };
        for entry in todo {
            self.check_cancel()?;
            let local = entry.local_path.clone().expect("filtered on local_path");
            let path = Path::new(&local);
            let Some(before) = local_stat(path)? else {
                continue; // gone: the next scan drops it
            };
            let (hash, n) = hash_file(path)?;
            let after = local_stat(path)?;
            if after != Some(before) || n != before.size {
                continue; // still being written; try next round
            }
            self.db().execute(
                "UPDATE entries SET blake3 = ?1, size = ?2, mtime_ns = ?3 WHERE id = ?4",
                params![hash, before.size, before.mtime_ns, entry.id],
            )?;
            // A fresh snapshot invalidates any upload begun for older content
            // (the Ring discards abandoned uploads after 7 days).
            self.db().execute(
                "INSERT INTO outbox (entry_id, hashed_size, hashed_mtime_ns)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (entry_id) DO UPDATE SET
                   hashed_size = excluded.hashed_size, hashed_mtime_ns = excluded.hashed_mtime_ns,
                   upload_id = NULL, attempts = 0, last_error = NULL, next_try_at = NULL",
                params![entry.id, before.size, before.mtime_ns],
            )?;
        }
        Ok(())
    }

    fn due_jobs(&self, now: i64) -> Result<Vec<Job>, MoonError> {
        let db = self.db();
        let ids: Vec<i64> = {
            let mut stmt = db.prepare(
                "SELECT o.entry_id FROM outbox o JOIN entries e ON e.id = o.entry_id
                 WHERE e.state = 'local-only' AND e.suppressed = 0 AND e.blake3 IS NOT NULL
                   AND e.local_path IS NOT NULL
                   AND o.hashed_size = e.size AND o.hashed_mtime_ns = e.mtime_ns
                   AND (o.next_try_at IS NULL OR o.next_try_at <= ?1)
                 ORDER BY o.attempts, e.size",
            )?;
            stmt.query_map([now], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            let (Some(entry), Some(outbox)) =
                (store::entry_by_id(&db, id)?, store::outbox(&db, id)?)
            else {
                continue;
            };
            let snapshot = HashSnapshot {
                blake3: entry.blake3.expect("filtered"),
                size: outbox.hashed_size,
                mtime_ns: outbox.hashed_mtime_ns,
            };
            let local = entry.local_path.clone().expect("filtered");
            jobs.push(Job {
                entry,
                outbox,
                snapshot,
                local,
            });
        }
        Ok(jobs)
    }

    pub(crate) async fn process_outbox_inner(
        &self,
        s: &Session,
        listener: Option<&Arc<dyn TransferListener>>,
    ) -> Result<OutboxReport, MoonError> {
        self.prepare_outbox()?;
        let mut report = OutboxReport::default();
        for job in self.due_jobs(now_secs())? {
            self.check_cancel()?;
            match self.run_job(s, &job, listener).await {
                Ok(JobOutcome { sent, decision }) => {
                    match sent {
                        true => report.uploaded += 1,
                        false => report.deduplicated += 1,
                    }
                    match decision {
                        Decision::Offload => report.offloaded += 1,
                        Decision::KeepSynced => report.kept += 1,
                        _ => {}
                    }
                }
                Err(MoonError::Cancelled) => return Err(MoonError::Cancelled),
                Err(e) => {
                    report.failed += 1;
                    store::record_failure(&self.db(), job.entry.id, &e.to_string(), now_secs())?;
                    // The Ring is unreachable: the rest would fail the same way.
                    if matches!(e, MoonError::Network { .. }) {
                        break;
                    }
                }
            }
        }
        report.remaining = self
            .db()
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?;
        Ok(report)
    }

    async fn run_job(
        &self,
        s: &Session,
        job: &Job,
        listener: Option<&Arc<dyn TransferListener>>,
    ) -> Result<JobOutcome, MoonError> {
        let path = Path::new(&job.local);
        let current = local_stat(path)?;
        if current
            != Some(LocalStat {
                size: job.snapshot.size,
                mtime_ns: job.snapshot.mtime_ns,
            })
        {
            // Changed or gone since hashing: rehash (or let the scan drop it).
            self.db().execute(
                "UPDATE entries SET blake3 = NULL WHERE id = ?1",
                [job.entry.id],
            )?;
            return Err(MoonError::invalid("file changed since it was hashed"));
        }
        let sent = self.upload(s, job, listener).await?;
        let (ack, uploaded) = match sent {
            Sent::Uploaded(a) => (a, true),
            Sent::Deduplicated(a) => (a, false),
        };
        let decision = self.apply_ack(s, job, &ack)?;
        Ok(JobOutcome {
            sent: uploaded,
            decision,
        })
    }

    /// Sends the file (resuming if possible) and returns the Ring's ACK.
    async fn upload(
        &self,
        s: &Session,
        job: &Job,
        listener: Option<&Arc<dyn TransferListener>>,
    ) -> Result<Sent, MoonError> {
        let entry_id = job.entry.id;
        let mut resume: Option<(String, u64, u32, Vec<u32>)> = None;

        if let Some(id) = &job.outbox.upload_id {
            match s
                .json::<UploadStatus>(s.get(&format!("/uploads/{id}")))
                .await
            {
                Ok(st) => match st.state {
                    UploadState::Committed => match self.commit(s, id).await {
                        Ok(ack) => return Ok(Sent::Uploaded(ack)),
                        // The committed file is gone from the Ring: start over.
                        Err(e) if e.status() == Some(409) => {}
                        Err(e) => return Err(e),
                    },
                    UploadState::Failed => {}
                    UploadState::Open | UploadState::Committing => {
                        resume = Some((id.clone(), st.chunk_size, st.chunk_count, st.received));
                    }
                },
                Err(e) if e.status() == Some(404) => {}
                Err(e) => return Err(e),
            }
            if resume.is_none() {
                store::set_upload_id(&self.db(), entry_id, None)?;
            }
        }

        let (upload_id, chunk_size, chunk_count, received) = match resume {
            Some(r) => r,
            None => {
                let path = RelPath::new(&job.entry.ring_path)
                    .map_err(|e| MoonError::invalid(format!("{}: {e}", job.entry.ring_path)))?;
                let req = CreateUploadRequest {
                    path,
                    size: job.snapshot.size,
                    mtime_ns: Some(job.snapshot.mtime_ns),
                    blake3: job.snapshot.blake3,
                    chunk_size: None,
                };
                match s
                    .json::<CreateUploadResponse>(s.post("/uploads").json(&req))
                    .await?
                {
                    CreateUploadResponse::Exists { ack } => return Ok(Sent::Deduplicated(ack)),
                    CreateUploadResponse::Open {
                        upload_id,
                        chunk_size,
                        chunk_count,
                        received,
                    } => {
                        store::set_upload_id(&self.db(), entry_id, Some(&upload_id))?;
                        (upload_id, chunk_size, chunk_count, received)
                    }
                }
            }
        };

        let plan = UploadPlan::new(job.snapshot.size, chunk_size)
            .map_err(|e| MoonError::invalid(format!("upload plan: {e}")))?;
        if plan.chunk_count() != chunk_count {
            return Err(MoonError::invalid("the Ring's chunk plan does not match"));
        }
        let have: HashSet<u32> = received.into_iter().collect();
        let file = File::open(&job.local)?;
        let mut sent: u64 = have
            .iter()
            .filter_map(|i| plan.chunk_range(*i))
            .map(|(_, len)| len)
            .sum();
        for i in 0..chunk_count {
            if have.contains(&i) {
                continue;
            }
            self.check_cancel()?;
            let (offset, len) = plan.chunk_range(i).expect("index below chunk_count");
            let mut buf = vec![0u8; len as usize];
            file.read_exact_at(&mut buf, offset)?;
            let hash = hash_bytes(&buf);
            s.send(
                s.post(&format!("/uploads/{upload_id}/chunks/{i}"))
                    .header(CHUNK_HASH_HEADER, hash.to_hex())
                    .body(buf),
            )
            .await?;
            sent += len;
            if let Some(l) = listener {
                l.on_upload_progress(job.entry.ring_path.clone(), sent, job.snapshot.size);
            }
        }
        match self.commit(s, &upload_id).await {
            Ok(ack) => Ok(Sent::Uploaded(ack)),
            Err(e) => {
                if e.server_code() == Some("hash_mismatch") {
                    // The file changed under us: rehash and start a new upload.
                    store::set_upload_id(&self.db(), entry_id, None)?;
                    self.db()
                        .execute("UPDATE entries SET blake3 = NULL WHERE id = ?1", [entry_id])?;
                }
                Err(e)
            }
        }
    }

    async fn commit(&self, s: &Session, upload_id: &str) -> Result<UploadAck, MoonError> {
        s.json(s.post(&format!("/uploads/{upload_id}/commit")))
            .await
    }

    /// The retention rule (§3.5 step 5). Database first, then the delete.
    fn apply_ack(&self, s: &Session, job: &Job, ack: &UploadAck) -> Result<Decision, MoonError> {
        let path = Path::new(&job.local);
        let current = local_stat(path)?;
        let decision = offload::decide(
            &self.policy(),
            &job.snapshot,
            Some(&VerifiedAck {
                blake3: ack.blake3,
                size: ack.size,
            }),
            current.as_ref(),
        );
        let id = job.entry.id;
        let mut db = self.db();
        let tx = db.transaction()?;
        match decision {
            Decision::Retry(reason) => {
                tx.execute(
                    "UPDATE outbox SET upload_id = NULL WHERE entry_id = ?1",
                    [id],
                )?;
                tx.commit()?;
                return Err(MoonError::invalid(format!(
                    "the Ring's ACK does not match the local file ({reason:?})"
                )));
            }
            Decision::LocalChanged => {
                // The Ring has the old content; the new one is uploaded next
                // round (under a conflict name).
                tx.execute("UPDATE entries SET blake3 = NULL WHERE id = ?1", [id])?;
                tx.execute(
                    "UPDATE outbox SET upload_id = NULL WHERE entry_id = ?1",
                    [id],
                )?;
            }
            Decision::Offload | Decision::KeepSynced | Decision::LocalGone => {
                // An index sync may already have listed our new file.
                tx.execute(
                    "DELETE FROM entries WHERE ring_path = ?1 AND id != ?2 AND state = 'remote-only'",
                    params![ack.path, id],
                )?;
                let path_free: bool = tx.query_row(
                    "SELECT NOT EXISTS (SELECT 1 FROM entries WHERE ring_path = ?1 AND id != ?2)",
                    params![ack.path, id],
                    |r| r.get(0),
                )?;
                let ring_path = if path_free {
                    ack.path.as_str()
                } else {
                    job.entry.ring_path.as_str()
                };
                let synced = decision == Decision::KeepSynced;
                tx.execute(
                    "UPDATE entries SET state = ?1, local_path = ?2, ring_path = ?3,
                       remote_file_id = ?4, blake3 = ?5, size = ?6
                     WHERE id = ?7",
                    params![
                        if synced {
                            FileState::Synced
                        } else {
                            FileState::RemoteOnly
                        },
                        if synced {
                            Some(job.local.as_str())
                        } else {
                            None
                        },
                        ring_path,
                        ack.file_id,
                        ack.blake3,
                        ack.size,
                        id
                    ],
                )?;
                tx.execute("DELETE FROM outbox WHERE entry_id = ?1", [id])?;
            }
        }
        // Stored under a conflict name: the original path holds another file
        // we may have skipped. Re-reading the index is idempotent and cheap.
        if ack.path.as_str() != job.entry.ring_path {
            store::set_cursor(&tx, s.ring.id, 0)?;
        }
        tx.commit()?;
        drop(db);

        if decision.may_delete_local() {
            // Last check right before deleting: still exactly what we hashed.
            let now = local_stat(path)?;
            // If it was touched in the last instant, keep it: the row is
            // already remote-only, so the next scan uploads the new content.
            if now
                == Some(LocalStat {
                    size: job.snapshot.size,
                    mtime_ns: job.snapshot.mtime_ns,
                })
            {
                fs::remove_file(path)?;
            }
        }
        Ok(decision)
    }
}

struct JobOutcome {
    sent: bool,
    decision: Decision,
}

#[cfg(test)]
mod tests;
