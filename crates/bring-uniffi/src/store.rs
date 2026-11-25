//! The Moon's SQLite index (`bring-core` Moon schema, docs/ARCHITECTURE.md §2.2).

use bring_core::api::PairingResult;
use bring_core::codec::{hex_decode, hex_encode};
use bring_core::{Blake3Hash, FileState};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::MoonError;

/// One paired Ring.
#[derive(Debug, Clone)]
pub struct RingRow {
    pub id: i64,
    pub ring_id: [u8; 16],
    pub name: Option<String>,
    pub lan_url: String,
    pub wan_url: Option<String>,
    pub pin: [u8; 32],
    pub device_uuid: [u8; 16],
    pub device_slug: String,
    /// `None` = engine default (100 MiB); `Some(u64::MAX)` = never offload.
    pub offload_threshold_bytes: Option<u64>,
    /// Cumulative files ever transferred to this Ring (survives deletion/pruning).
    pub transferred_files: u64,
    /// Cumulative bytes ever transferred to this Ring (survives deletion/pruning).
    pub transferred_bytes: u64,
}

impl RingRow {
    pub fn ring_id_hex(&self) -> String {
        hex_encode(&self.ring_id)
    }

    pub fn display_name(&self) -> String {
        if let Some(n) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return n.to_owned();
        }
        // Fall back to the LAN host (e.g. "192.168.1.15") so multiple unnamed Rings stay distinguishable.
        let stripped = self
            .lan_url
            .strip_prefix("https://")
            .or_else(|| self.lan_url.strip_prefix("http://"))
            .unwrap_or(&self.lan_url);
        let host = stripped.split('/').next().unwrap_or(stripped);
        let host = host.split(':').next().unwrap_or(host);
        if host.is_empty() {
            "Ring".to_owned()
        } else {
            host.to_owned()
        }
    }
}

const RING_COLS: &str = "id, ring_id, name, lan_url, wan_url, tls_cert_sha256, device_uuid, \
                         device_slug, offload_threshold_bytes, transferred_files, transferred_bytes";

fn decode_offload(v: Option<i64>) -> Option<u64> {
    v.map(|n| if n < 0 { u64::MAX } else { n as u64 })
}

fn encode_offload(bytes: u64) -> i64 {
    if bytes == u64::MAX {
        -1
    } else {
        i64::try_from(bytes).unwrap_or(i64::MAX)
    }
}

fn ring_from_row(r: &Row<'_>) -> rusqlite::Result<RingRow> {
    Ok(RingRow {
        id: r.get(0)?,
        ring_id: r.get(1)?,
        name: r.get(2)?,
        lan_url: r.get(3)?,
        wan_url: r.get(4)?,
        pin: r.get(5)?,
        device_uuid: r.get(6)?,
        device_slug: r.get(7)?,
        offload_threshold_bytes: decode_offload(r.get(8)?),
        transferred_files: r.get::<_, i64>(9)?.max(0) as u64,
        transferred_bytes: r.get::<_, i64>(10)?.max(0) as u64,
    })
}

pub fn rings(c: &Connection) -> Result<Vec<RingRow>, MoonError> {
    let mut stmt = c.prepare(&format!("SELECT {RING_COLS} FROM rings ORDER BY id"))?;
    let rows = stmt.query_map([], ring_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn ring(c: &Connection) -> Result<Option<RingRow>, MoonError> {
    Ok(c.query_row(
        &format!("SELECT {RING_COLS} FROM rings ORDER BY id LIMIT 1"),
        [],
        ring_from_row,
    )
    .optional()?)
}

pub fn ring_by_db_id(c: &Connection, id: i64) -> Result<Option<RingRow>, MoonError> {
    Ok(c.query_row(
        &format!("SELECT {RING_COLS} FROM rings WHERE id = ?1"),
        [id],
        ring_from_row,
    )
    .optional()?)
}

pub fn ring_by_hex(c: &Connection, ring_id_hex: &str) -> Result<Option<RingRow>, MoonError> {
    let Some(bytes) = hex_decode::<16>(ring_id_hex) else {
        return Ok(None);
    };
    Ok(c.query_row(
        &format!("SELECT {RING_COLS} FROM rings WHERE ring_id = ?1"),
        [bytes],
        ring_from_row,
    )
    .optional()?)
}

/// Stores a pairing result. Re-pairing with the same Ring keeps its entries,
/// watched folders, custom name and offload threshold; a new Ring is added
/// alongside existing ones.
pub fn save_ring(
    c: &Connection,
    result: &PairingResult,
    device_uuid: [u8; 16],
) -> Result<RingRow, MoonError> {
    let ring_id: [u8; 16] = hex_decode(&result.ring_id)
        .ok_or_else(|| MoonError::invalid("pairing result: bad ring_id"))?;
    let pin: [u8; 32] = hex_decode(&result.tls_cert_sha256)
        .ok_or_else(|| MoonError::invalid("pairing result: bad tls_cert_sha256"))?;
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
    let row = ring_by_hex(c, &result.ring_id)?.expect("ring row just written");
    c.execute(
        "INSERT OR IGNORE INTO sync_cursor (ring_id, last_seq) VALUES (?1, 0)",
        [row.id],
    )?;
    Ok(row)
}

pub fn rename_ring(c: &Connection, ring_db_id: i64, name: &str) -> Result<RingRow, MoonError> {
    let clean = name.trim();
    let stored = (!clean.is_empty()).then_some(clean);
    c.execute(
        "UPDATE rings SET name = ?1 WHERE id = ?2",
        params![stored, ring_db_id],
    )?;
    ring_by_db_id(c, ring_db_id)?.ok_or(MoonError::NotPaired)
}

pub fn set_device_slug(
    c: &mut Connection,
    ring_db_id: i64,
    old_slug: &str,
    new_slug: &str,
) -> Result<RingRow, MoonError> {
    let tx = c.transaction()?;
    tx.execute(
        "UPDATE rings SET device_slug = ?1 WHERE id = ?2",
        params![new_slug, ring_db_id],
    )?;
    if old_slug != new_slug && !old_slug.is_empty() && !new_slug.is_empty() {
        let old_prefix = format!("{old_slug}/");
        let new_prefix = format!("{new_slug}/");
        let folders: Vec<(i64, String)> = {
            let mut stmt =
                tx.prepare("SELECT id, ring_folder FROM watched_folders WHERE ring_id = ?1")?;
            stmt.query_map([ring_db_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        for (wf_id, rf) in folders {
            let updated_rf = if rf == old_slug {
                Some(new_slug.to_owned())
            } else {
                rf.strip_prefix(&old_prefix)
                    .map(|rest| format!("{new_prefix}{rest}"))
            };
            if let Some(target) = updated_rf {
                tx.execute(
                    "UPDATE watched_folders SET ring_folder = ?1 WHERE id = ?2",
                    params![target, wf_id],
                )?;
            }
        }
        let entries: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, ring_path FROM entries WHERE ring_id = ?1 AND ring_path LIKE ?2",
            )?;
            stmt.query_map(params![ring_db_id, format!("{old_prefix}%")], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?
        };
        for (entry_id, rp) in entries {
            if let Some(rest) = rp.strip_prefix(&old_prefix) {
                let target = format!("{new_prefix}{rest}");
                let exists: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM entries WHERE ring_id = ?1 AND ring_path = ?2)",
                    params![ring_db_id, &target],
                    |r| r.get(0),
                )?;
                if !exists {
                    tx.execute(
                        "UPDATE entries SET ring_path = ?1 WHERE id = ?2",
                        params![target, entry_id],
                    )?;
                }
            }
        }
    }
    let row = ring_by_db_id(&tx, ring_db_id)?.ok_or(MoonError::NotPaired)?;
    tx.commit()?;
    Ok(row)
}

pub fn set_ring_offload(c: &Connection, ring_db_id: i64, bytes: u64) -> Result<(), MoonError> {
    c.execute(
        "UPDATE rings SET offload_threshold_bytes = ?1 WHERE id = ?2",
        params![encode_offload(bytes), ring_db_id],
    )?;
    Ok(())
}

/// Drops all Rings and everything learned from them. Local files are untouched.
pub fn forget_ring(c: &mut Connection) -> Result<(), MoonError> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM outbox", [])?;
    tx.execute("DELETE FROM entries", [])?;
    tx.execute("DELETE FROM watched_folders", [])?;
    tx.execute("DELETE FROM sync_cursor", [])?;
    tx.execute("DELETE FROM rings", [])?;
    tx.commit()?;
    Ok(())
}

/// Drops one Ring, its watched folders, entries, outbox and cursor.
/// Local files are untouched.
pub fn forget_ring_by_db_id(c: &mut Connection, ring_db_id: i64) -> Result<(), MoonError> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM rings WHERE id = ?1", [ring_db_id])?;
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
    pub ring_id: i64,
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
    pub offload_after_upload: bool,
}

pub const ENTRY_COLS: &str = "id, ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, \
                              mime, blake3, thumb_blake3, state, suppressed, offload_after_upload";

pub fn entry_from_row(r: &Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok(EntryRow {
        id: r.get(0)?,
        ring_id: r.get(1)?,
        ring_path: r.get(2)?,
        local_path: r.get(3)?,
        remote_file_id: r.get(4)?,
        size: r.get(5)?,
        mtime_ns: r.get(6)?,
        mime: r.get(7)?,
        blake3: r.get(8)?,
        thumb: r.get(9)?,
        state: r.get(10)?,
        suppressed: r.get(11)?,
        offload_after_upload: r.get(12)?,
    })
}

pub fn entry_by_id(c: &Connection, id: i64) -> Result<Option<EntryRow>, MoonError> {
    Ok(c.query_row(
        &format!("SELECT {ENTRY_COLS} FROM entries WHERE id = ?1"),
        [id],
        entry_from_row,
    )
    .optional()?)
}

pub fn entry_by_ring_path(
    c: &Connection,
    ring_db_id: i64,
    path: &str,
) -> Result<Option<EntryRow>, MoonError> {
    Ok(c.query_row(
        &format!("SELECT {ENTRY_COLS} FROM entries WHERE ring_id = ?1 AND ring_path = ?2"),
        params![ring_db_id, path],
        entry_from_row,
    )
    .optional()?)
}

pub fn entry_by_local_path(
    c: &Connection,
    ring_db_id: i64,
    path: &str,
) -> Result<Option<EntryRow>, MoonError> {
    Ok(c.query_row(
        &format!("SELECT {ENTRY_COLS} FROM entries WHERE ring_id = ?1 AND local_path = ?2"),
        params![ring_db_id, path],
        entry_from_row,
    )
    .optional()?)
}

/// A new `local-only` entry for a file found on the Moon. Not hashed yet.
pub fn insert_local(
    c: &Connection,
    ring_db_id: i64,
    ring_path: &str,
    local_path: &str,
    size: u64,
    mtime_ns: i64,
    mime: Option<&str>,
) -> Result<i64, MoonError> {
    c.execute(
        "INSERT INTO entries (ring_id, ring_path, local_path, size, mtime_ns, mime, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'local-only')",
        params![ring_db_id, ring_path, local_path, size, mtime_ns, mime],
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

/// Watched folder (`local_dir` → `ring_folder`) belonging to one Ring.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WatchedFolder {
    pub id: i64,
    pub ring_id: String,
    pub local_dir: String,
    pub ring_folder: String,
    pub enabled: bool,
}

pub fn watched_folders(c: &Connection, ring_db_id: i64) -> Result<Vec<WatchedFolder>, MoonError> {
    let mut stmt = c.prepare(
        "SELECT w.id, r.ring_id, w.local_dir, w.ring_folder, w.enabled
         FROM watched_folders w
         JOIN rings r ON r.id = w.ring_id
         WHERE w.ring_id = ?1
         ORDER BY w.id",
    )?;
    let rows = stmt.query_map([ring_db_id], |r| {
        let rid: [u8; 16] = r.get(1)?;
        Ok(WatchedFolder {
            id: r.get(0)?,
            ring_id: hex_encode(&rid),
            local_dir: r.get(2)?,
            ring_folder: r.get(3)?,
            enabled: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// All enabled watched folders of other Rings (`ring_db_id != exclude_ring_db_id`).
pub fn other_rings_watched_dirs(
    c: &Connection,
    exclude_ring_db_id: i64,
) -> Result<Vec<(i64, String)>, MoonError> {
    let mut stmt = c.prepare(
        "SELECT ring_id, local_dir FROM watched_folders
         WHERE enabled = 1 AND ring_id != ?1",
    )?;
    let rows = stmt.query_map([exclude_ring_db_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
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

    #[test]
    fn offload_encoding_roundtrips() {
        assert_eq!(decode_offload(None), None);
        assert_eq!(
            decode_offload(Some(encode_offload(u64::MAX))),
            Some(u64::MAX)
        );
        assert_eq!(
            decode_offload(Some(encode_offload(100 * 1024 * 1024))),
            Some(100 * 1024 * 1024)
        );
    }
}
