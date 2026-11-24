//! SQLite index: connection setup and schema migrations for both roles.
//!
//! The schema version is tracked in `PRAGMA user_version`. Migrations are
//! append-only SQL files; each runs in its own transaction.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

/// Which index a database holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Ring,
    Moon,
}

const RING_MIGRATIONS: &[&str] = &[include_str!("ring/0001_init.sql")];
const MOON_MIGRATIONS: &[&str] = &[include_str!("moon/0001_init.sql")];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("cannot obtain random bytes: {0}")]
    Random(#[from] getrandom::Error),
    #[error("corrupt meta value for key {0:?}")]
    CorruptMeta(&'static str),
}

fn migrations(role: Role) -> &'static [&'static str] {
    match role {
        Role::Ring => RING_MIGRATIONS,
        Role::Moon => MOON_MIGRATIONS,
    }
}

/// Opens (creating if needed) a database file, configures it and migrates it.
pub fn open(path: &Path, role: Role) -> Result<Connection, DbError> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    init(&mut conn, role)?;
    Ok(conn)
}

/// Opens a fresh in-memory database (tests).
pub fn open_in_memory(role: Role) -> Result<Connection, DbError> {
    let mut conn = Connection::open_in_memory()?;
    init(&mut conn, role)?;
    Ok(conn)
}

fn init(conn: &mut Connection, role: Role) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // FULL: a committed transaction survives power loss. Index writes are
    // small; durability matters more than throughput here.
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(conn, role)?;
    if role == Role::Ring {
        ensure_random_meta(conn, "ring_id", 16)?;
        ensure_random_meta(conn, "stream_key", 32)?;
    }
    Ok(())
}

/// The schema version this build migrates to.
pub fn latest_version(role: Role) -> u32 {
    migrations(role).len() as u32
}

/// Applies all pending migrations.
pub fn migrate(conn: &mut Connection, role: Role) -> Result<(), DbError> {
    let list = migrations(role);
    let supported = list.len() as u32;
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > supported {
        return Err(DbError::SchemaTooNew {
            found: current,
            supported,
        });
    }
    for (i, sql) in list.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn ensure_random_meta(conn: &Connection, key: &str, len: usize) -> Result<(), DbError> {
    let mut value = vec![0u8; len];
    getrandom::fill(&mut value)?;
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

/// Reads a fixed-size blob from the Ring `meta` table.
pub fn ring_meta_blob<const N: usize>(
    conn: &Connection,
    key: &'static str,
) -> Result<[u8; N], DbError> {
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?;
    blob.and_then(|b| b.try_into().ok())
        .ok_or(DbError::CorruptMeta(key))
}

/// Allocates the next change sequence number on a Ring index.
///
/// Must be called inside the transaction that performs the change, so the
/// counter and the change commit (or roll back) together.
pub fn next_seq(conn: &Connection) -> Result<i64, DbError> {
    let seq: i64 = conn.query_row(
        "UPDATE meta SET value = value + 1 WHERE key = 'next_seq' RETURNING value - 1",
        [],
        |r| r.get(0),
    )?;
    Ok(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn ring_schema() {
        let conn = open_in_memory(Role::Ring).unwrap();
        assert_eq!(
            tables(&conn),
            ["devices", "files", "meta", "upload_chunks", "uploads"]
        );
        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, latest_version(Role::Ring));
        let ring_id: [u8; 16] = ring_meta_blob(&conn, "ring_id").unwrap();
        assert_ne!(ring_id, [0; 16]);
        let _key: [u8; 32] = ring_meta_blob(&conn, "stream_key").unwrap();
    }

    #[test]
    fn moon_schema() {
        let conn = open_in_memory(Role::Moon).unwrap();
        assert_eq!(
            tables(&conn),
            [
                "entries",
                "outbox",
                "rings",
                "sync_cursor",
                "watched_folders"
            ]
        );
    }

    #[test]
    fn reopen_is_idempotent_and_keeps_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ring.db");
        let id1: [u8; 16] = ring_meta_blob(&open(&path, Role::Ring).unwrap(), "ring_id").unwrap();
        let id2: [u8; 16] = ring_meta_blob(&open(&path, Role::Ring).unwrap(), "ring_id").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn rejects_newer_schema() {
        let mut conn = open_in_memory(Role::Moon).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            migrate(&mut conn, Role::Moon),
            Err(DbError::SchemaTooNew { found: 99, .. })
        ));
    }

    #[test]
    fn seq_is_monotonic() {
        let conn = open_in_memory(Role::Ring).unwrap();
        let a = next_seq(&conn).unwrap();
        let b = next_seq(&conn).unwrap();
        assert_eq!((a, b), (1, 2));
    }

    #[test]
    fn active_upload_key_is_unique_but_failed_is_not() {
        let conn = open_in_memory(Role::Ring).unwrap();
        conn.execute(
            "INSERT INTO devices (device_uuid, name, slug, token_hash, created_at)
             VALUES (x'00', 'p', 'p', x'01', 0)",
            [],
        )
        .unwrap();
        let insert = |id: &str, state: &str| {
            conn.execute(
                "INSERT INTO uploads (id, device_id, client_key, target_path, size, chunk_size,
                   chunk_count, expected_blake3, state, created_at, updated_at)
                 VALUES (?1, 1, x'aa', 'a', 1, 1, 1, x'bb', ?2, 0, 0)",
                params![id, state],
            )
        };
        insert("u1", "failed").unwrap();
        insert("u2", "failed").unwrap();
        insert("u3", "open").unwrap();
        assert!(
            insert("u4", "open").is_err(),
            "second active upload must collide"
        );
        assert!(insert("u5", "committing").is_err());
        insert("u6", "committed").unwrap();
    }

    #[test]
    fn state_check_constraint() {
        let conn = open_in_memory(Role::Moon).unwrap();
        let err = conn.execute(
            "INSERT INTO entries (ring_path, size, mtime_ns, state) VALUES ('a', 0, 0, 'bogus')",
            [],
        );
        assert!(err.is_err());
    }
}
