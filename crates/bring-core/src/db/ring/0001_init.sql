-- Ring index, schema version 1. See docs/ARCHITECTURE.md §2.1.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value BLOB NOT NULL
);
-- ring_id (16 B) and stream_key (32 B) are inserted by bring-core at open.
INSERT INTO meta (key, value) VALUES ('next_seq', 1);

CREATE TABLE devices (
  id           INTEGER PRIMARY KEY,
  device_uuid  BLOB NOT NULL UNIQUE,
  name         TEXT NOT NULL,
  slug         TEXT NOT NULL UNIQUE,
  token_hash   BLOB NOT NULL UNIQUE,
  created_at   INTEGER NOT NULL,
  last_seen_at INTEGER,
  revoked_at   INTEGER
);

CREATE TABLE files (
  id               INTEGER PRIMARY KEY,
  path             TEXT NOT NULL UNIQUE,
  folder           TEXT NOT NULL,
  size             INTEGER NOT NULL,
  mtime_ns         INTEGER NOT NULL,
  mime             TEXT,
  blake3           BLOB,
  origin_device_id INTEGER REFERENCES devices(id),
  thumb_blake3     BLOB,
  deleted          INTEGER NOT NULL DEFAULT 0,
  seq              INTEGER NOT NULL
);
CREATE INDEX files_seq    ON files(seq);
CREATE INDEX files_blake3 ON files(blake3);

CREATE TABLE uploads (
  id              TEXT PRIMARY KEY,
  device_id       INTEGER NOT NULL REFERENCES devices(id),
  client_key      BLOB NOT NULL,
  target_path     TEXT NOT NULL,
  size            INTEGER NOT NULL,
  mtime_ns        INTEGER,
  chunk_size      INTEGER NOT NULL,
  chunk_count     INTEGER NOT NULL,
  expected_blake3 BLOB NOT NULL,
  state           TEXT NOT NULL CHECK (state IN ('open', 'committing', 'committed', 'failed')),
  result_file_id  INTEGER REFERENCES files(id),
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);
CREATE UNIQUE INDEX uploads_active_key
  ON uploads(device_id, client_key) WHERE state IN ('open', 'committing');

CREATE TABLE upload_chunks (
  upload_id TEXT NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
  idx       INTEGER NOT NULL,
  len       INTEGER NOT NULL,
  blake3    BLOB NOT NULL,
  PRIMARY KEY (upload_id, idx)
) WITHOUT ROWID;
