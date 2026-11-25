-- Cumulative lifetime transfer totals per Ring.
-- `entries` tracks only the live per-file state; deleted/pruned files leave
-- `entries`, while these counters preserve the total files and bytes ever
-- transferred to each Ring.

ALTER TABLE rings ADD COLUMN transferred_files INTEGER NOT NULL DEFAULT 0;
ALTER TABLE rings ADD COLUMN transferred_bytes INTEGER NOT NULL DEFAULT 0;

UPDATE rings SET
  transferred_files = COALESCE(
    (SELECT COUNT(*) FROM entries e WHERE e.ring_id = rings.id AND e.remote_file_id IS NOT NULL),
    0
  ),
  transferred_bytes = COALESCE(
    (SELECT SUM(e.size) FROM entries e WHERE e.ring_id = rings.id AND e.remote_file_id IS NOT NULL),
    0
  );
