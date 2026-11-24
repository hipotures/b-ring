//! Paired devices (Moons) and their bearer tokens.
//!
//! Tokens are 32 random bytes, base64url-encoded; only `blake3(token)` is
//! stored, so a leaked database does not leak usable credentials.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bring_core::db::DbError;
use bring_core::hash::hash_bytes;
use rusqlite::{Connection, OptionalExtension, params};

/// Seconds since the Unix epoch.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64,
    pub device_uuid: [u8; 16],
    pub name: String,
    pub slug: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

const COLS: &str = "id, device_uuid, name, slug, created_at, last_seen_at, revoked_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    let uuid: Vec<u8> = r.get(1)?;
    Ok(Device {
        id: r.get(0)?,
        device_uuid: uuid.try_into().unwrap_or([0; 16]),
        name: r.get(2)?,
        slug: r.get(3)?,
        created_at: r.get(4)?,
        last_seen_at: r.get(5)?,
        revoked_at: r.get(6)?,
    })
}

/// A path-safe identifier derived from a device name: `"Pixel 8 Pro"` →
/// `"pixel-8-pro"`. Non-ASCII letters are dropped; never empty.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 32 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "device".to_owned()
    } else {
        slug
    }
}

fn token_hash(token: &str) -> [u8; 32] {
    *hash_bytes(token.as_bytes()).as_bytes()
}

fn new_token() -> Result<String, DbError> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw)?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

/// Registers a device (or re-pairs an existing one with the same UUID, which
/// keeps its id and slug) and returns it with a fresh bearer token.
pub fn register(
    conn: &Connection,
    device_uuid: [u8; 16],
    name: &str,
    now: i64,
) -> Result<(Device, String), DbError> {
    let token = new_token()?;
    let hash = token_hash(&token);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM devices WHERE device_uuid = ?1",
            [&device_uuid[..]],
            |r| r.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => {
            conn.execute(
                "UPDATE devices SET name = ?1, token_hash = ?2, revoked_at = NULL WHERE id = ?3",
                params![name, &hash[..], id],
            )?;
            id
        }
        None => {
            let base = slugify(name);
            let mut slug = base.clone();
            let mut n = 2;
            while conn
                .query_row("SELECT 1 FROM devices WHERE slug = ?1", [&slug], |_| Ok(()))
                .optional()?
                .is_some()
            {
                slug = format!("{base}-{n}");
                n += 1;
            }
            conn.execute(
                "INSERT INTO devices (device_uuid, name, slug, token_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![&device_uuid[..], name, slug, &hash[..], now],
            )?;
            conn.last_insert_rowid()
        }
    };
    let device = by_id(conn, id)?.expect("device just written");
    Ok((device, token))
}

pub fn by_id(conn: &Connection, id: i64) -> Result<Option<Device>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM devices WHERE id = ?1"),
            [id],
            row,
        )
        .optional()?)
}

/// The non-revoked device owning `token`.
pub fn by_token(conn: &Connection, token: &str) -> Result<Option<Device>, DbError> {
    let hash = token_hash(token);
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM devices WHERE token_hash = ?1 AND revoked_at IS NULL"),
            [&hash[..]],
            row,
        )
        .optional()?)
}

pub fn list(conn: &Connection) -> Result<Vec<Device>, DbError> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM devices ORDER BY id"))?;
    let out = stmt.query_map([], row)?.collect::<Result<_, _>>()?;
    Ok(out)
}

/// Finds a device by numeric id or slug.
pub fn find(conn: &Connection, key: &str) -> Result<Option<Device>, DbError> {
    if let Ok(id) = key.parse::<i64>() {
        return by_id(conn, id);
    }
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM devices WHERE slug = ?1"),
            [key],
            row,
        )
        .optional()?)
}

/// Revokes a device's token. Returns `false` if it was already revoked.
pub fn revoke(conn: &Connection, id: i64, now: i64) -> Result<bool, DbError> {
    Ok(conn.execute(
        "UPDATE devices SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
        params![now, id],
    )? > 0)
}

/// Records activity, at most once a minute per device (avoids a write per request).
pub fn touch(conn: &Connection, id: i64, now: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE devices SET last_seen_at = ?1
         WHERE id = ?2 AND (last_seen_at IS NULL OR last_seen_at < ?1 - 60)",
        params![now, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bring_core::db::{Role, open_in_memory};

    #[test]
    fn slugs() {
        assert_eq!(slugify("Pixel 8 Pro"), "pixel-8-pro");
        assert_eq!(slugify("  Mój Quest 3!! "), "m-j-quest-3");
        assert_eq!(slugify("ąę"), "device");
        assert_eq!(slugify(&"x".repeat(100)).len(), 32);
    }

    #[test]
    fn register_lookup_revoke() {
        let c = open_in_memory(Role::Ring).unwrap();
        let (a, ta) = register(&c, [1; 16], "Pixel 8", 100).unwrap();
        let (b, tb) = register(&c, [2; 16], "Pixel 8", 100).unwrap();
        assert_eq!((a.slug.as_str(), b.slug.as_str()), ("pixel-8", "pixel-8-2"));
        assert_eq!(ta.len(), 43);
        assert_ne!(ta, tb);
        assert_eq!(by_token(&c, &ta).unwrap().unwrap().id, a.id);
        assert_eq!(by_token(&c, "nope").unwrap(), None);

        assert!(revoke(&c, a.id, 200).unwrap());
        assert!(!revoke(&c, a.id, 201).unwrap());
        assert_eq!(by_token(&c, &ta).unwrap(), None);

        // Re-pairing the same phone keeps id/slug, issues a new token.
        let (a2, ta2) = register(&c, [1; 16], "Pixel 8 (new)", 300).unwrap();
        assert_eq!((a2.id, a2.slug.as_str()), (a.id, "pixel-8"));
        assert_eq!(a2.revoked_at, None);
        assert_eq!(by_token(&c, &ta).unwrap(), None, "old token stays dead");
        assert_eq!(by_token(&c, &ta2).unwrap().unwrap().id, a.id);
        assert_eq!(find(&c, "pixel-8-2").unwrap().unwrap().id, b.id);
        assert_eq!(find(&c, &b.id.to_string()).unwrap().unwrap().id, b.id);
    }

    #[test]
    fn touch_is_throttled() {
        let c = open_in_memory(Role::Ring).unwrap();
        let (d, _) = register(&c, [1; 16], "p", 0).unwrap();
        touch(&c, d.id, 1000).unwrap();
        touch(&c, d.id, 1030).unwrap();
        assert_eq!(by_id(&c, d.id).unwrap().unwrap().last_seen_at, Some(1000));
        touch(&c, d.id, 1061).unwrap();
        assert_eq!(by_id(&c, d.id).unwrap().unwrap().last_seen_at, Some(1061));
    }
}
