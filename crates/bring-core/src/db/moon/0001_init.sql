-- Moon index, schema version 1. See docs/ARCHITECTURE.md §2.2.

CREATE TABLE rings (
  id              INTEGER PRIMARY KEY,
  ring_id         BLOB NOT NULL UNIQUE,
  name            TEXT,
  lan_url         TEXT NOT NULL,
  wan_url         TEXT,
  tls_cert_sha256 BLOB NOT NULL,
  device_uuid     BLOB NOT NULL,
  device_slug     TEXT NOT NULL
);

CREATE TABLE sync_cursor (
  ring_id  INTEGER PRIMARY KEY REFERENCES rings(id) ON DELETE CASCADE,
  last_seq INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE watched_folders (
  id          INTEGER PRIMARY KEY,
  local_dir   TEXT NOT NULL UNIQUE,
  ring_folder TEXT NOT NULL,
  enabled     INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE entries (
  id             INTEGER PRIMARY KEY,
  ring_path      TEXT NOT NULL UNIQUE,
  local_path     TEXT,
  remote_file_id INTEGER,
  size           INTEGER NOT NULL,
  mtime_ns       INTEGER NOT NULL,
  mime           TEXT,
  blake3         BLOB,
  thumb_blake3   BLOB,
  state          TEXT NOT NULL CHECK (state IN ('local-only', 'synced', 'remote-only')),
  suppressed     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE outbox (
  entry_id        INTEGER PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
  upload_id       TEXT,
  hashed_size     INTEGER NOT NULL,
  hashed_mtime_ns INTEGER NOT NULL,
  attempts        INTEGER NOT NULL DEFAULT 0,
  last_error      TEXT,
  next_try_at     INTEGER
);
