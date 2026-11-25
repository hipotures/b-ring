-- When set on an entry (e.g. Share Sheet uploads staged in app-private storage),
-- delete the staged local copy and transition to `remote-only` right after a
-- verified ACK regardless of file size or the Ring's offload threshold.
ALTER TABLE entries ADD COLUMN offload_after_upload INTEGER NOT NULL DEFAULT 0;
