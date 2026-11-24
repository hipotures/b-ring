-- Moon index, schema version 2: multi-Ring support.
-- Each Ring has its own watched folders, entries, sync cursor and offload threshold.
-- Multiple Rings may watch the same local_dir.

ALTER TABLE rings ADD COLUMN offload_threshold_bytes INTEGER;

CREATE TABLE watched_folders_new (
  id          INTEGER PRIMARY KEY,
  ring_id     INTEGER NOT NULL REFERENCES rings(id) ON DELETE CASCADE,
  local_dir   TEXT NOT NULL,
  ring_folder TEXT NOT NULL,
  enabled     INTEGER NOT NULL DEFAULT 1,
  UNIQUE (ring_id, local_dir)
);

INSERT INTO watched_folders_new (id, ring_id, local_dir, ring_folder, enabled)
SELECT w.id, (SELECT id FROM rings ORDER BY id LIMIT 1), w.local_dir, w.ring_folder, w.enabled
FROM watched_folders w
WHERE EXISTS (SELECT 1 FROM rings);

DROP TABLE watched_folders;
ALTER TABLE watched_folders_new RENAME TO watched_folders;

CREATE TABLE entries_new (
  id             INTEGER PRIMARY KEY,
  ring_id        INTEGER NOT NULL REFERENCES rings(id) ON DELETE CASCADE,
  ring_path      TEXT NOT NULL,
  local_path     TEXT,
  remote_file_id INTEGER,
  size           INTEGER NOT NULL,
  mtime_ns       INTEGER NOT NULL,
  mime           TEXT,
  blake3         BLOB,
  thumb_blake3   BLOB,
  state          TEXT NOT NULL CHECK (state IN ('local-only', 'synced', 'remote-only')),
  suppressed     INTEGER NOT NULL DEFAULT 0,
  UNIQUE (ring_id, ring_path)
);

INSERT INTO entries_new (
  id, ring_id, ring_path, local_path, remote_file_id, size, mtime_ns,
  mime, blake3, thumb_blake3, state, suppressed
)
SELECT
  e.id, (SELECT id FROM rings ORDER BY id LIMIT 1), e.ring_path, e.local_path, e.remote_file_id,
  e.size, e.mtime_ns, e.mime, e.blake3, e.thumb_blake3, e.state, e.suppressed
FROM entries e
WHERE EXISTS (SELECT 1 FROM rings);

CREATE TEMP TABLE outbox_migration_bak AS
SELECT entry_id, upload_id, hashed_size, hashed_mtime_ns, attempts, last_error, next_try_at
FROM outbox;

DROP TABLE outbox;
DROP TABLE entries;
ALTER TABLE entries_new RENAME TO entries;

CREATE TABLE outbox (
  entry_id        INTEGER PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
  upload_id       TEXT,
  hashed_size     INTEGER NOT NULL,
  hashed_mtime_ns INTEGER NOT NULL,
  attempts        INTEGER NOT NULL DEFAULT 0,
  last_error      TEXT,
  next_try_at     INTEGER
);

INSERT INTO outbox (entry_id, upload_id, hashed_size, hashed_mtime_ns, attempts, last_error, next_try_at)
SELECT entry_id, upload_id, hashed_size, hashed_mtime_ns, attempts, last_error, next_try_at
FROM outbox_migration_bak
WHERE entry_id IN (SELECT id FROM entries);

DROP TABLE outbox_migration_bak;

CREATE INDEX entries_ring_local ON entries(ring_id, local_path);
CREATE INDEX entries_local_path ON entries(local_path);
