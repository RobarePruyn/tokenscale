-- Phase 1.5 (v0.1.17): tool-use ingest expansion. Adds three new
-- tables for CC's tool_use blocks (inside assistant message content
-- arrays), tool_result blocks (inside user message content arrays,
-- previously dropped entirely), and file-history-snapshot records
-- (a fourth JSONL line type, also previously dropped).
--
-- Gates Phase 2 (Tier 1 commit attribution: which Bash `git commit`
-- calls did CC author?) and Phase 3 (Tier 2 edit-survival: how much
-- CC-authored code survives via Edit/Write?). See
-- docs/roadmap-1.5-tool-use-ingest.md for the design.
--
-- Forward-only: historical events (every event ingested before
-- v0.1.17) have no rows in any of these three tables. Their
-- session_id queries return empty results — Phase 2/3 attribution
-- for those sessions is "no data," not "incorrect data." Same
-- posture as v0.1.16's NULL-uuid handling.
--
-- A user who wants historical events backfilled can run
-- `tokenscale scan --rebuild` post-upgrade. This is user-elective,
-- not an upgrade-time default. Query-time linkage between
-- tool_uses and tool_results means rebuild file ordering does not
-- matter — see § 5 of the scoping doc.

-- ---------------------------------------------------------------
-- tool_uses
-- One row per `tool_use` block inside an assistant message's
-- content array. parent_event_uuid joins to events.uuid; tool_use_id
-- is Anthropic's internal toolu_… identifier (distinct from CC's
-- per-message uuid that v0.1.16 captures).
-- ---------------------------------------------------------------
CREATE TABLE tool_uses (
    id                  INTEGER PRIMARY KEY,
    tool_use_id         TEXT NOT NULL,
    parent_event_uuid   TEXT NOT NULL,
    source              TEXT NOT NULL,
    tool_name           TEXT NOT NULL,
    input_json          TEXT NOT NULL,
    occurred_at         TEXT NOT NULL,
    session_id          TEXT,
    project_id          TEXT
);

-- (source, tool_use_id) — mirrors the v0.1.16 (source, uuid) idiom.
-- Cross-source uuids would coincidentally collide; the source prefix
-- prevents the false positive once Phase 1.5+ ingest expansions add
-- new sources.
CREATE UNIQUE INDEX tool_uses_source_tool_use_id_unique
    ON tool_uses (source, tool_use_id);

CREATE INDEX tool_uses_parent_event_uuid_idx ON tool_uses (parent_event_uuid);
CREATE INDEX tool_uses_session_id_idx ON tool_uses (session_id);
-- (project_id, tool_name) — Phase 2/3 query primitives filter on this
-- shape (e.g., all Bash calls in a project) and want index coverage.
CREATE INDEX tool_uses_project_id_tool_name_idx ON tool_uses (project_id, tool_name);

-- ---------------------------------------------------------------
-- tool_results
-- One row per `tool_result` block inside a user message's content
-- array. tool_use_id joins to tool_uses.tool_use_id within the same
-- session (per § 3's D3 query-time linkage). Per Phase 0's empirical
-- 100% linkage rate (5 orphan tool_uses, 0 orphan tool_results),
-- (source, tool_use_id) is unique here as well.
-- ---------------------------------------------------------------
CREATE TABLE tool_results (
    id                  INTEGER PRIMARY KEY,
    tool_use_id         TEXT NOT NULL,
    parent_event_uuid   TEXT NOT NULL,
    source              TEXT NOT NULL,
    content             TEXT NOT NULL,
    occurred_at         TEXT NOT NULL,
    session_id          TEXT
);

CREATE UNIQUE INDEX tool_results_source_tool_use_id_unique
    ON tool_results (source, tool_use_id);
CREATE INDEX tool_results_session_id_idx ON tool_results (session_id);

-- ---------------------------------------------------------------
-- file_snapshots
-- One row per file in a file-history-snapshot's trackedFileBackups
-- dict. snapshot_message_id joins to events.uuid (the assistant turn
-- that triggered the snapshot). One snapshot may carry zero or many
-- file rows; we explode the dict to one row per file at ingest time.
-- ---------------------------------------------------------------
CREATE TABLE file_snapshots (
    id                  INTEGER PRIMARY KEY,
    snapshot_message_id TEXT NOT NULL,
    source              TEXT NOT NULL,
    file_path           TEXT NOT NULL,
    backup_file_name    TEXT,
    version             INTEGER NOT NULL,
    backup_time         TEXT NOT NULL,
    is_snapshot_update  INTEGER NOT NULL,
    session_id          TEXT,
    project_id          TEXT
);

-- Dedup on (source, snapshot_message_id, file_path) — a re-scan of
-- the same JSONL must idempotently re-produce the same rows; the
-- index lets INSERT OR IGNORE handle that.
CREATE UNIQUE INDEX file_snapshots_source_msg_path_unique
    ON file_snapshots (source, snapshot_message_id, file_path);
CREATE INDEX file_snapshots_session_id_idx ON file_snapshots (session_id);
CREATE INDEX file_snapshots_project_id_idx ON file_snapshots (project_id);
