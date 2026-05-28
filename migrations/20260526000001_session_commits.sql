-- Phase 2 (v0.1.18): Tier 1 commit attribution. Adds the
-- `session_commits` table. One row per real `git commit` invocation
-- captured from CC Bash tool_uses (per the shlex token-walk filter
-- in docs/roadmap-2-commit-attribution.md § 1.1).
--
-- See docs/roadmap-2-commit-attribution.md for the full design pass
-- including the six D-decisions (D1 through D6) and the §9 release
-- gate framing. This file pins D1.
--
-- Forward-only: historical sessions whose tool_uses rows landed
-- before v0.1.18 ships will NOT have session_commits rows. The
-- ingest path populates this table only at scan time going forward.
-- Stated in three places per project convention: this comment, the
-- scoping doc (§8), and the v0.1.18 CHANGELOG entry.
--
-- A user who wants historical sessions backfilled can run
-- `tokenscale scan --rebuild` post-upgrade. v0.1.18 also bundles
-- issue #6's fix to the --rebuild wipe set so the rebuild now
-- correctly wipes tool_uses, tool_results, file_snapshots, and
-- session_commits before re-parsing.

CREATE TABLE session_commits (
    id                                INTEGER PRIMARY KEY,
    source                            TEXT    NOT NULL,
    tool_use_id                       TEXT    NOT NULL,
    session_id                        TEXT    NOT NULL,
    -- Nullable per the 0.7% real-failure rate (gitignore reject,
    -- nothing-to-commit, etc.) plus the 1.4% maintainer-pipe-cut-
    -- without-chained-push cases. See § 1.6 of the scoping doc.
    sha                               TEXT,
    -- Verbatim `cd "<path>"` target extracted from the command.
    -- NULL when the command has no cd prefix (14% of commits in
    -- the maintainer corpus).
    cd_target_raw                     TEXT,
    -- Resolved at insert time via `git -C <cwd> rev-parse
    -- --show-toplevel` against (cd_target_raw or events.project_id).
    -- Always populated. Forward-only.
    project_resolved                  TEXT    NOT NULL,
    -- Enum: 'primary' | 'push_refspec' | 'none'. Records which
    -- regex captured the SHA. The 'none' bucket covers both real
    -- failures and parser misses; distinguish via the
    -- output_head_truncated_by_command flag and external markers.
    recovery_source                   TEXT    NOT NULL,
    -- 0 or 1. True when the command contains `| tail` anywhere,
    -- signalling the operator deliberately truncated git commit's
    -- output head. See D5 Part 2.
    output_head_truncated_by_command  INTEGER NOT NULL,
    -- 0 or 1. True when --amend appears in the command. The amend
    -- produces a fresh SHA captured by the primary regex; the
    -- prior commit's SHA is orphaned and naturally surfaces as
    -- sha_resolves_in_tree=false at query time. See D6.
    is_amend                          INTEGER NOT NULL,
    occurred_at                       TEXT    NOT NULL
);

-- (source, tool_use_id) is sufficient for uniqueness: tool_use_id
-- is already unique per source via the tool_uses UNIQUE index.
-- INSERT OR IGNORE on this constraint makes re-scans idempotent.
CREATE UNIQUE INDEX session_commits_source_tool_use_id_unique
    ON session_commits (source, tool_use_id);

CREATE INDEX session_commits_session_id_idx       ON session_commits (session_id);
CREATE INDEX session_commits_project_resolved_idx ON session_commits (project_resolved);

-- Partial index: SHA lookups are a hot path (resolution checks,
-- cross-session dedup queries). NULL shas don't participate.
CREATE INDEX session_commits_sha_idx ON session_commits (sha) WHERE sha IS NOT NULL;
