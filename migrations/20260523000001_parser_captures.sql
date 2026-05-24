-- Phase 1B-ii (v0.1.16): parser captures gitBranch, uuid, parentUuid
-- from Claude Code JSONL into events. Forward-only: historical
-- events have NULL for all three columns. No re-parse, no backfill
-- — see docs/roadmap-1b-ii-parser-captures.md § 5 for rationale.
--
-- A user who wants historical events backfilled can run
-- `tokenscale scan --rebuild` post-upgrade; this is user-elective,
-- not an upgrade-time default. The partial UNIQUE index below
-- excludes NULL from uniqueness, so the lack of backfill is
-- correct, not a bug.

ALTER TABLE events ADD COLUMN git_branch  TEXT;
ALTER TABLE events ADD COLUMN uuid        TEXT;
ALTER TABLE events ADD COLUMN parent_uuid TEXT;

-- Partial UNIQUE index on (source, uuid) — the actual defense
-- against the same-message-different-requestId duplicate case
-- Phase 0 flagged as the load-bearing correctness concern.
--
-- WHERE uuid IS NOT NULL excludes every pre-v0.1.16 event from
-- the uniqueness constraint, so the migration applies cleanly
-- against any DB regardless of row count and historical NULLs
-- never conflict with each other or with future captures.
--
-- (source, uuid) not (uuid) alone — matches the existing
-- (source, request_id) / (source, content_hash) idiom from
-- 20260428000001_initial.sql, and is the safer key once
-- Phase 1.5 introduces additional ingest sources that could
-- legitimately collide with a claude_code uuid by coincidence.
CREATE UNIQUE INDEX events_source_uuid_unique
    ON events (source, uuid)
    WHERE uuid IS NOT NULL;
