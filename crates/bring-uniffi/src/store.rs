//! The Moon's SQLite index (`bring-core` Moon schema, docs/ARCHITECTURE.md §2.2).

use bring_core::api::PairingResult;
use bring_core::codec::hex_decode;
use bring_core::{Blake3Hash, FileState};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::MoonError;

/// The paired Ring. Phase 1 supports exactly one.
#[derive(Debug, Clone)]
pub struct RingRow {
    pub id: i64,
    pub ring_id: [u8; 16],
    pub lan_url: String,
    pub wan_url: Option<String>,
    pub pin: [u8; 32],
    pub device_uuid: [u8; 16],
    pub device_slug: String,
}

pub fn ring(c: &Connection) -> Result<Option<RingRow>, MoonError> {
    Ok(c.query_row(
        "SELECT id, ring_id, lan_url, wan_url, tls_cert_sha256, device_uuid, device_slug
         FROM rings ORDER BY id LIMIT 1",
        [],
        |r| {
            Ok(RingRow {
                id: r.get(0)?,
                ring_id: r.get(1)?,
                lan_url: r.get(2)?,
                wan_url: r.get(3)?,
                pin: r.get(4)?,
                device_uuid: r.get(5)?,
                device_slug: r.get(6)?,
            })
        },
    )
    .optional()?)
}

/// Stores a pairing result. Re-pairing with the same Ring keeps its entries;
/// a different Ring must be forgotten first.
pub fn save_ring(
    c: &Connection,
    result: &PairingResult,
    device_uuid: [u8; 16],
) -> Result<RingRow, MoonError> {
    let ring_id: [u8; 16] = hex_decode(&result.ring_id)
        .ok_or_else(|| MoonError::invalid("pairing result: bad ring_id"))?;
    let pin: [u8; 32] = hex_decode(&result.tls_cert_sha256)
        .ok_or_else(|| MoonError::invalid("pairing result: bad tls_cert_sha256"))?;
    if let Some(existing) = ring(c)?
        && existing.ring_id != ring_id
    {
        return Err(MoonError::invalid(
            "already paired with another Ring; forget it first",
        ));
    }
    c.execute(
        "INSERT INTO rings (ring_id, lan_url, wan_url, tls_cert_sha256, device_uuid, device_slug)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (ring_id) DO UPDATE SET
           lan_url = excluded.lan_url, wan_url = excluded.wan_url,
           tls_cert_sha256 = excluded.tls_cert_sha256,
           device_uuid = excluded.device_uuid, device_slug = excluded.device_slug",
        params![
            ring_id,
            result.lan_url,
            result.wan_url,
            pin,
            device_uuid,
            result.device_slug
        ],
    )?;
    let row = ring(c)?.expect("ring row just written");
    c.execute(
        "INSERT OR IGNORE INTO sync_cursor (ring_id, last_seq) VALUES (?1, 0)",
        [row.id],
    )?;
    Ok(row)
}

/// Drops the Ring and everything learned from it. Local files are untouched;
/// watched folders are kept and will be rediscovered by the next scan.
pub fn forget_ring(c: &mut Connection) -> Result<(), MoonError> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM outbox", [])?;
    tx.execute("DELETE FROM entries", [])?;
    tx.execute("DELETE FROM sync_cursor", [])?;
    tx.execute("DELETE FROM rings", [])?;
    tx.commit()?;
    Ok(())
}

pub fn cursor(c: &Connection, ring: i64) -> Result<i64, MoonError> {
    Ok(c.query_row(
        "SELECT last_seq FROM sync_cursor WHERE ring_id = ?1",
        [ring],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}

pub fn set_cursor(c: &Connection, ring: i64, seq: i64) -> Result<(), MoonError> {
    c.execute(
        "INSERT INTO sync_cursor (ring_id, last_seq) VALUES (?1, ?2)
         ON CONFLICT (ring_id) DO UPDATE SET last_seq = excluded.last_seq",
        params![ring, seq],
    )?;
    Ok(())
}

/// One row of `entries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    pub id: i64,
    pub ring_path: String,
    pub local_path: Option<String>,
    pub remote_file_id: Option<i64>,
    pub size: u64,
    pub mtime_ns: i64,
    pub mime: Option<String>,
    pub blake3: Option<Blake3Hash>,
    pub thumb: Option<Blake3Hash>,
    pub state: FileState,
    pub suppressed: bool,
}

pub const ENTRY_COLS: &str = "id, ring_path, local_path, remote_file_id, size, mtime_ns, mime, \
                              blake3, thumb_blake3, state, suppressed";

pub fn entry_from_row(r: &Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok(EntryRow {
        id: r.get(0)?,
        ring_path: r.get(1)?,
        local_path: r.get(2)?,
        remote_file_id: r.get(3)?,
        size: r.get(4)?,
        mtime_ns: r.get(5)?,
        mime: r.get(6)?,
        blake3: r.get(7)?,
        thumb: r.get(8)?,
        state: r.get(9)?,
        suppressed: r.get(10)?,
    })
}

fn entry_where(
    c: &Connection,
    cond: &str,
    arg: &dyn rusqlite::ToSql,
) -> rusqlite::Result<Option<EntryRow>> {
    c.query_row(
        &format!("SELECT {ENTRY_COLS} FROM entries WHERE {cond}"),
        [arg],
        entry_from_row,
    )
    .optional()
}

pub fn entry_by_id(c: &Connection, id: i64) -> Result<Option<EntryRow>, MoonError> {
    Ok(entry_where(c, "id = ?1", &id)?)
}

pub fn entry_by_ring_path(c: &Connection, path: &str) -> Result<Option<EntryRow>, MoonError> {
    Ok(entry_where(c, "ring_path = ?1", &path)?)
}

pub fn entry_by_local_path(c: &Connection, path: &str) -> Result<Option<EntryRow>, MoonError> {
    Ok(entry_where(c, "local_path = ?1", &path)?)
}

/// A new `local-only` entry for a file found on the Moon. Not hashed yet.
pub fn insert_local(
    c: &Connection,
    ring_path: &str,
    local_path: &str,
    size: u64,
    mtime_ns: i64,
    mime: Option<&str>,
) -> Result<i64, MoonError> {
    c.execute(
        "INSERT INTO entries (ring_path, local_path, size, mtime_ns, mime, state)
         VALUES (?1, ?2, ?3, ?4, ?5, 'local-only')",
        params![ring_path, local_path, size, mtime_ns, mime],
    )?;
    Ok(c.last_insert_rowid())
}

/// One row of `outbox`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub entry_id: i64,
    pub upload_id: Option<String>,
    pub hashed_size: u64,
    pub hashed_mtime_ns: i64,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub next_try_at: Option<i64>,
}

pub fn outbox(c: &Connection, entry_id: i64) -> Result<Option<OutboxRow>, MoonError> {
    Ok(c.query_row(
        "SELECT entry_id, upload_id, hashed_size, hashed_mtime_ns, attempts, last_error, next_try_at
         FROM outbox WHERE entry_id = ?1",
        [entry_id],
        |r| {
            Ok(OutboxRow {
                entry_id: r.get(0)?,
                upload_id: r.get(1)?,
                hashed_size: r.get(2)?,
                hashed_mtime_ns: r.get(3)?,
                attempts: r.get(4)?,
                last_error: r.get(5)?,
                next_try_at: r.get(6)?,
            })
        },
    )
    .optional()?)
}

pub fn set_upload_id(c: &Connection, entry_id: i64, id: Option<&str>) -> Result<(), MoonError> {
    c.execute(
        "UPDATE outbox SET upload_id = ?1 WHERE entry_id = ?2",
        params![id, entry_id],
    )?;
    Ok(())
}

/// Exponential backoff: 30 s, 1 min, 2 min, … capped at 1 h.
pub fn backoff_secs(attempts: u32) -> i64 {
    let exp = attempts.saturating_sub(1).min(7);
    (30_i64 << exp).min(3600)
}

pub fn record_failure(
    c: &Connection,
    entry_id: i64,
    error: &str,
    now: i64,
) -> Result<(), MoonError> {
    let attempts: u32 = c
        .query_row(
            "SELECT attempts FROM outbox WHERE entry_id = ?1",
            [entry_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0)
        + 1;
    c.execute(
        "UPDATE outbox SET attempts = ?1, last_error = ?2, next_try_at = ?3 WHERE entry_id = ?4",
        params![attempts, error, now + backoff_secs(attempts), entry_id],
    )?;
    Ok(())
}

/// Watched folder (`local_dir` → `ring_folder`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WatchedFolder {
    pub id: i64,
    pub local_dir: String,
    pub ring_folder: String,
    pub enabled: bool,
}

pub fn watched_folders(c: &Connection) -> Result<Vec<WatchedFolder>, MoonError> {
    let mut stmt =
        c.prepare("SELECT id, local_dir, ring_folder, enabled FROM watched_folders ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(WatchedFolder {
            id: r.get(0)?,
            local_dir: r.get(1)?,
            ring_folder: r.get(2)?,
            enabled: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let v: Vec<i64> = (1..=10).map(backoff_secs).collect();
        assert_eq!(&v[..4], &[30, 60, 120, 240]);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*v.last().unwrap(), 3600);
    }
}
