-- tokenscale — Phase 2 ingest hardening (migration 0003).
--
-- Adds a `len` (file size, bytes) column to `_ingest_file_state` so the
-- delta-scan path can detect content changes that don't update mtime.
-- iCloud / Dropbox / OneDrive sometimes preserve mtime across syncs but
-- mutate bytes, defeating the mtime-only check. Comparing the
-- `(mtime_ns, len)` tuple is a cheap second factor.
--
-- Defaults to 0 for existing rows; the next scan will rewrite them. The
-- mismatch between stored 0 and the actual file size will force one
-- re-parse per file on first run after the migration. Subsequent runs
-- are fully incremental again.

PRAGMA foreign_keys = ON;

ALTER TABLE _ingest_file_state ADD COLUMN len INTEGER NOT NULL DEFAULT 0;
