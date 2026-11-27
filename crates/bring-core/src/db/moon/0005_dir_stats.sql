-- Folder browsing without loading every entry.
--
-- `dir` is the folder of an entry: `ring_path` up to and including its last '/'
-- ('' for top-level files). rtrim() against the set of all non-'/' characters
-- strips the file name. Indexed, so one folder's files come straight from
-- the index, newest first.
ALTER TABLE entries ADD COLUMN dir TEXT
  GENERATED ALWAYS AS (rtrim(ring_path, replace(ring_path, '/', ''))) VIRTUAL;
CREATE INDEX entries_dir ON entries(ring_id, dir, mtime_ns);

-- Per-folder totals of the files directly in it (suppressed entries excluded),
-- kept current by the triggers below. A folder's subtree is a range scan over
-- this small table instead of over every file.
CREATE TABLE dir_stats (
  ring_id  INTEGER NOT NULL REFERENCES rings(id) ON DELETE CASCADE,
  dir      TEXT NOT NULL,
  n_local  INTEGER NOT NULL DEFAULT 0,
  n_synced INTEGER NOT NULL DEFAULT 0,
  n_remote INTEGER NOT NULL DEFAULT 0,
  b_local  INTEGER NOT NULL DEFAULT 0,
  b_synced INTEGER NOT NULL DEFAULT 0,
  b_remote INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (ring_id, dir)
) WITHOUT ROWID;

INSERT INTO dir_stats (ring_id, dir, n_local, n_synced, n_remote, b_local, b_synced, b_remote)
SELECT ring_id, dir,
  SUM(state = 'local-only'),  SUM(state = 'synced'),  SUM(state = 'remote-only'),
  SUM(CASE WHEN state = 'local-only'  THEN size ELSE 0 END),
  SUM(CASE WHEN state = 'synced'      THEN size ELSE 0 END),
  SUM(CASE WHEN state = 'remote-only' THEN size ELSE 0 END)
FROM entries WHERE suppressed = 0
GROUP BY ring_id, dir;

CREATE TRIGGER dir_stats_insert AFTER INSERT ON entries WHEN NEW.suppressed = 0
BEGIN
  INSERT INTO dir_stats (ring_id, dir, n_local, n_synced, n_remote, b_local, b_synced, b_remote)
  VALUES (NEW.ring_id, NEW.dir,
    NEW.state = 'local-only', NEW.state = 'synced', NEW.state = 'remote-only',
    CASE WHEN NEW.state = 'local-only'  THEN NEW.size ELSE 0 END,
    CASE WHEN NEW.state = 'synced'      THEN NEW.size ELSE 0 END,
    CASE WHEN NEW.state = 'remote-only' THEN NEW.size ELSE 0 END)
  ON CONFLICT (ring_id, dir) DO UPDATE SET
    n_local  = n_local  + excluded.n_local,  n_synced = n_synced + excluded.n_synced,
    n_remote = n_remote + excluded.n_remote, b_local  = b_local  + excluded.b_local,
    b_synced = b_synced + excluded.b_synced, b_remote = b_remote + excluded.b_remote;
END;

CREATE TRIGGER dir_stats_delete AFTER DELETE ON entries WHEN OLD.suppressed = 0
BEGIN
  UPDATE dir_stats SET
    n_local  = n_local  - (OLD.state = 'local-only'),
    n_synced = n_synced - (OLD.state = 'synced'),
    n_remote = n_remote - (OLD.state = 'remote-only'),
    b_local  = b_local  - CASE WHEN OLD.state = 'local-only'  THEN OLD.size ELSE 0 END,
    b_synced = b_synced - CASE WHEN OLD.state = 'synced'      THEN OLD.size ELSE 0 END,
    b_remote = b_remote - CASE WHEN OLD.state = 'remote-only' THEN OLD.size ELSE 0 END
  WHERE ring_id = OLD.ring_id AND dir = OLD.dir;
END;

-- An update is "take the old contribution out, put the new one in"; the two
-- triggers commute, so their order does not matter.
CREATE TRIGGER dir_stats_update_out AFTER UPDATE ON entries WHEN OLD.suppressed = 0
BEGIN
  UPDATE dir_stats SET
    n_local  = n_local  - (OLD.state = 'local-only'),
    n_synced = n_synced - (OLD.state = 'synced'),
    n_remote = n_remote - (OLD.state = 'remote-only'),
    b_local  = b_local  - CASE WHEN OLD.state = 'local-only'  THEN OLD.size ELSE 0 END,
    b_synced = b_synced - CASE WHEN OLD.state = 'synced'      THEN OLD.size ELSE 0 END,
    b_remote = b_remote - CASE WHEN OLD.state = 'remote-only' THEN OLD.size ELSE 0 END
  WHERE ring_id = OLD.ring_id AND dir = OLD.dir;
END;

CREATE TRIGGER dir_stats_update_in AFTER UPDATE ON entries WHEN NEW.suppressed = 0
BEGIN
  INSERT INTO dir_stats (ring_id, dir, n_local, n_synced, n_remote, b_local, b_synced, b_remote)
  VALUES (NEW.ring_id, NEW.dir,
    NEW.state = 'local-only', NEW.state = 'synced', NEW.state = 'remote-only',
    CASE WHEN NEW.state = 'local-only'  THEN NEW.size ELSE 0 END,
    CASE WHEN NEW.state = 'synced'      THEN NEW.size ELSE 0 END,
    CASE WHEN NEW.state = 'remote-only' THEN NEW.size ELSE 0 END)
  ON CONFLICT (ring_id, dir) DO UPDATE SET
    n_local  = n_local  + excluded.n_local,  n_synced = n_synced + excluded.n_synced,
    n_remote = n_remote + excluded.n_remote, b_local  = b_local  + excluded.b_local,
    b_synced = b_synced + excluded.b_synced, b_remote = b_remote + excluded.b_remote;
END;
