# Roadmap — Phase 1B-ii: parser captures + uuid UNIQUE (v0.1.16)

**Status**: scoping. No implementation in this pass.
**Date**: 2026-05-22
**Sequenced after**: v0.1.15 (Granular Attribution Phase 1, user-visible layer — landed `d90b1ed`).
**Sequenced before**: Phase 1.5 (tool-use / tool-result / file-history-snapshot ingest expansion).

This is the ingest-layer companion to v0.1.15. **No new design decisions**: D2 was signed off in the Phase 1B sign-off (storage + UNIQUE partial index on `uuid`, skip-with-warning on duplicates, forward-only). Doc surfaces implementation specifics + answers to the three open implementation-shape questions (I1–I3).

---

## 1. Schema migration

### File

`migrations/20260523000001_parser_captures.sql` (date = day after v0.1.15 commit; lexically ordered after the v0.1.13 `20260520000001_pricing_notes_column.sql`).

### Exact DDL

```sql
-- Phase 1B-ii: parser captures gitBranch, uuid, parentUuid from JSONL
-- into events. Forward-only: historical events have NULL for all
-- three columns. No re-parse, no backfill — see
-- docs/roadmap-1b-ii-parser-captures.md § 5 for rationale.

ALTER TABLE events ADD COLUMN git_branch  TEXT;
ALTER TABLE events ADD COLUMN uuid        TEXT;
ALTER TABLE events ADD COLUMN parent_uuid TEXT;

-- Partial UNIQUE index on (source, uuid) — the actual defense against
-- the same-message-different-requestId duplicate case Phase 0
-- flagged as the load-bearing correctness concern. Partial WHERE
-- clause excludes NULL uuids (every pre-v0.1.16 event) from the
-- uniqueness constraint, so the migration applies cleanly on a DB
-- with millions of NULL-uuid historical rows. Future events with
-- a non-NULL uuid collide with each other but never with NULLs.
--
-- Same idiom as the existing events_source_request_id_unique +
-- events_source_content_hash_unique partial indexes from
-- 20260428000001_initial.sql.
CREATE UNIQUE INDEX events_source_uuid_unique
    ON events (source, uuid)
    WHERE uuid IS NOT NULL;
```

### Additive only

Three nullable columns + one partial index. No data rewrite. No existing column modified. No existing index modified.

### Why `(source, uuid)` and not `(uuid)` alone

The UNIQUE constraint keys on `(source, uuid)`, not on `uuid` by itself. Two reasons:

1. **Matches the existing convention.** `events_source_request_id_unique` and `events_source_content_hash_unique` from `20260428000001_initial.sql` are both `(source, …)`. The pattern is "uniqueness is per-ingest-source." A future second source (admin API, OpenAI logs, anything) could legitimately collide with a `claude_code` uuid by coincidence — the source prefix prevents that cross-source false-positive.
2. **Safer once Phase 1.5 introduces additional sources.** Currently `claude_code` is the only source emitting `uuid`; the source prefix is precautionary. When Phase 1.5+ tool-use ingest expansion lands (and possibly when an admin-API path starts emitting uuids), the existing schema doesn't need re-keying.

Approved at Phase 1B-ii sign-off as a design choice on the record, not buried in the DDL.

### SQLite version compatibility

Partial indexes are supported in SQLite ≥ 3.8.0 (2013). The workspace's `sqlx` 0.8 ships with bundled SQLite 3.49+ (the 0.8 release line tracks current upstream). The existing migration already relies on partial-WHERE indexes (`20260428000001_initial.sql` lines 70-78), so any environment that runs the current schema supports this one. **No version gate required.**

### Migration applies cleanly against

- **Fresh DB**: ALTER TABLE on the four-column-narrower `events` shape from `20260428000001_initial.sql` → adds three columns + one index. Single-statement migrations are atomic in sqlx.
- **v0.1.15-shape DB** (production maintainer + early brew users): identical operation. No row count, no existing column type, no existing index referenced. Smoke test the maintainer's DB before tagging.

### Downgrade behavior — what fails when a v0.1.15 binary reads v0.1.16 data?

Nothing fails. The three new columns and the new index are ignored by:

- Every existing `SELECT` query in the codebase — every query lists columns explicitly; none does `SELECT *`. v0.1.15 binaries never reference `git_branch`, `uuid`, or `parent_uuid`, so they don't appear in any result row.
- Every existing `INSERT` — v0.1.15's `INSERT INTO events (source, occurred_at, model, …)` lists the v0.1.15-known columns and lets the three new columns default to NULL. The partial UNIQUE index excludes those NULLs from uniqueness, so v0.1.15 INSERTs continue to work.
- SQLite's schema-tooling (`sqlite3 .schema`) sees the new columns + index without complaining.

Downgrade is silent and clean. Re-upgrade later: new events get uuid + git_branch + parent_uuid captured again, the partial index resumes enforcement on the new rows. Forward-only convention maintained.

---

## 2. Parser changes

### Field types — three nullable TEXT columns

Not JSON blob. Each lands as a discrete typed column:

| Column | JSONL source | SQL type | Rust type | Nullability |
|---|---|---|---|---|
| `uuid` | top-level `uuid` field | `TEXT NULL` | `Option<String>` | Always present in Phase 0's empirical sample (27,388/27,388), but `Option` for schema-drift tolerance |
| `parent_uuid` | top-level `parentUuid` | `TEXT NULL` | `Option<String>` | Usually present; NULL on the first turn of a session |
| `git_branch` | top-level `gitBranch` | `TEXT NULL` | `Option<String>` | NULL when the cwd isn't in a git repo |

These join the existing `Event` struct in `tokenscale-core/src/event.rs` (post-migration). The struct already has `Option<String>` fields (`request_id`, `content_hash`, `session_id`, `project_id`, `workspace_id`, `api_key_id`) and the parser already follows the "missing field → None" pattern (`#[serde(default)]` on each field of `AssistantPayload`).

### Parser behavior on missing fields

**Emit the row with the missing field as NULL. Do not skip the line. Do not fail the parse.**

Mechanically: extend `crates/tokenscale-ingest-cc/src/parser.rs`'s `AssistantPayload` struct with three new `#[serde(default)]` `Option<String>` fields named `uuid`, `parent_uuid` (serde-renamed from `parentUuid`), `git_branch` (renamed from `gitBranch`). Same serde-drift-tolerance pattern as the existing `request_id` / `session_id` / `cwd` fields.

### Debug log on missing-field detection

Per-field debug log line so a future maintainer can spot a pattern of CC dropping a field:

```rust
if assistant.uuid.is_none() {
    debug!(
        request_id = ?assistant.request_id,
        timestamp = %assistant.timestamp,
        "JSONL assistant line missing uuid — CC schema drift?"
    );
}
// same for parent_uuid (skipped on first turn — quieter; consider trace level for that one)
// same for git_branch (NULL is legitimate for non-git cwds — trace level)
```

Log at `debug` for uuid (load-bearing for the UNIQUE defense), `trace` for `parent_uuid` and `git_branch` (NULL is the expected case for genuine non-git / first-turn). Operators can `RUST_LOG=tokenscale_ingest_cc=debug` to see the uuid case if they suspect schema drift.

### JSONL schema documentation

**Anthropic does not publish a stable Claude Code JSONL schema.** The fields are reverse-engineered from observed lines. Existing parser comments in `parser.rs` describe the shape (e.g. `ASSISTANT_LINE` test fixture lines 213-214). Phase 0's findings doc (`docs/phase-0-findings-granular-attribution.md`) explicitly records this in §Q2.

For v0.1.16, the assumed shape is:

```jsonl
{"type":"assistant", "uuid":"…", "parentUuid":"…", "gitBranch":"…", …}
```

This shape is consistent across all 27,388 assistant lines in the maintainer's 48-file sample. **No upstream documentation exists**; we capture the assumed shape, log debug when fields disappear, and treat the disappearance as schema drift rather than as a parser bug.

---

## 3. Duplicate detection: skip-with-warning shape

### Where the dedup check happens

**Pre-check in the ingest path (`tokenscale-store::insert_events`), with the SQL UNIQUE constraint as backstop.** Not in the parser.

Rationale (informs I1 below): the existing `INSERT OR IGNORE` pattern returns `rows_affected = 0` on any UNIQUE conflict, but doesn't tell the caller *which* constraint fired (request_id vs content_hash vs the new uuid). The current code lumps everything under `summary.skipped_duplicate`. To distinguish uuid-collision (exceptional, warn-worthy) from request_id re-scan (normal, silent), an explicit pre-check before each INSERT is the clean answer.

Pre-check shape, sketched:

```rust
// Inside insert_events, per event in the batch:
if let Some(uuid) = &event.uuid {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT request_id FROM events WHERE source = ? AND uuid = ? LIMIT 1"
    )
    .bind(&event.source)
    .bind(uuid)
    .fetch_optional(&mut *transaction)
    .await?;
    if existing.is_some() {
        warn!(
            uuid = uuid,
            source = %event.source,
            "duplicate uuid detected during ingest — skipping; \
             CC behavior change?"
        );
        summary.uuid_duplicates_skipped += 1;
        continue;
    }
}
// existing INSERT OR IGNORE proceeds as before
```

The SQL UNIQUE constraint remains as the final defense in case of a race (two parallel scans of the same JSONL) — even if the pre-check passes, the UNIQUE index would catch the duplicate. That case still surfaces via `INSERT OR IGNORE`'s `rows_affected = 0` → `summary.skipped_duplicate += 1`. Belt and suspenders.

### Warning log line shape

```
WARN duplicate uuid detected during ingest — skipping; CC behavior change?
     uuid=db6baab1-…
     source=claude_code
     source_jsonl=/Users/.../sess-A.jsonl
     line=42
```

Fields:

- `uuid` — the colliding uuid (full UUID, not truncated; this is for grep, not display)
- `source` — `claude_code` (currently the only source that emits uuid)
- `source_jsonl` — path of the JSONL file the duplicate came from
- `line` — line number within that file

`source_jsonl` and `line` are available in the parser's call frame (where the scan loop already knows the file path and is using a line iterator) but not in `insert_events` itself. So the warning needs to be emitted from the scan caller, AFTER `insert_events` returns its summary with a `uuid_duplicates_skipped` count — OR the scan caller pre-attaches `source_jsonl`/`line` metadata to each Event so the store's warning has them.

Cleanest middle path: have `insert_events` return the list of skipped uuids (with their pre-attached `source_jsonl` + `line` metadata if present) alongside the summary count, and let the scan caller emit the final warning line with the file context. Avoids the store crate needing to know about JSONL paths.

The Event struct already carries a `raw: Option<String>` field for capture-raw mode; we could similarly add an optional `ingest_provenance: Option<{path, line}>` carried only during ingest and not stored in the DB. Decision deferred to build time — both approaches work; build chooses the smaller diff.

### Counter / metric

**New field on `ScanSummary`: `uuid_duplicates_skipped: usize`** (I3 below — separate from the existing `events_duplicates` because the two are semantically distinct).

### Ingest run's summary line

Current shape:

```
ScanSummary { files_seen: 29, files_parsed: 1, files_unchanged: 28, events_inserted: 1, events_duplicates: 4454, lines_skipped: 5475, lines_malformed: 0 }
```

Post-v0.1.16:

```
ScanSummary { files_seen: 29, files_parsed: 1, files_unchanged: 28, events_inserted: 1, events_duplicates: 4454, uuid_duplicates_skipped: 0, lines_skipped: 5475, lines_malformed: 0 }
```

When `uuid_duplicates_skipped > 0`, an operator running `tokenscale scan` sees the non-zero count in the closing summary line printed to stdout (the existing `println!` in `command_scan`). Phase 0 found zero duplicates in 27,388 lines so the steady-state value is `0`; a non-zero value is the loud signal.

The user-facing scan-output line currently reads:

```
Scan complete: 29 files seen, 1 parsed, 28 unchanged. 1 new events, 4454 duplicates skipped. 5475 non-assistant lines, 0 malformed.
```

Post-v0.1.16, when uuid duplicates are non-zero:

```
Scan complete: 29 files seen, 1 parsed, 28 unchanged. 1 new events, 4454 duplicates skipped (including 3 uuid duplicates — see logs). 5475 non-assistant lines, 0 malformed.
```

The "(including N uuid duplicates — see logs)" clause is conditional. Zero uuid duplicates means the line reads exactly as today.

---

## 4. Tests

Minimum coverage, mapped to the user's list:

### Parser tests (in `tokenscale-ingest-cc::parser`)

1. `assistant_line_captures_uuid_parent_uuid_git_branch` — JSONL line with all three fields populated → Event with those values present.
2. `assistant_line_missing_uuid_yields_none_other_fields_intact` — JSONL line missing `uuid` only → Event with `uuid = None`, other captures populated, parse succeeds. Repeat for `parentUuid` and `gitBranch` (three variants of the same test or a parameterized table).
3. `assistant_line_missing_all_three_captures_yields_all_none` — JSONL line missing all three → Event with all three NULL, parse succeeds (regression for pre-v0.1.16 JSONL shape if any user has very old data).

### Migration tests (in `tokenscale-store` or workspace-level)

4. `migration_20260523_applies_against_fresh_db` — existing pattern from `tokenscale-store::tests::migrations_apply_and_seed_sources`. Run all migrations from scratch, confirm the columns exist via `PRAGMA table_info(events)` and the index exists via `sqlite_master`.
5. `migration_20260523_applies_against_v0_1_15_shape_db` — write a synthetic events table in the pre-v0.1.16 shape, apply the new migration, confirm clean. (sqlx migrate auto-tracks applied migrations; this would need a manual ALTER to set up the pre-v0.1.16 starting state.)

### Ingest tests (in `tokenscale-store::events`)

6. `duplicate_uuid_skipped_with_warning_via_pre_check` — insert event with `uuid = "u1"`, then insert a second event with `uuid = "u1"` (different request_id). Assert `summary.inserted = 1` and `summary.uuid_duplicates_skipped = 1`. (Tracing log assertion is overkill for unit tests — covered by I2's design rather than test.)
7. `two_events_with_null_uuid_both_land` — two events, both `uuid = None`, different `request_id`. Partial index excludes NULL → both INSERTs succeed.
8. `unique_constraint_backstops_pre_check` — bypass the pre-check (test-only) and confirm `INSERT OR IGNORE` still skips the duplicate via SQL UNIQUE constraint. Belt-and-suspenders coverage.
9. `unique_violation_does_not_abort_ingest_run` — batch of 3 events, middle one is a uuid duplicate. Assert all 3 are processed (1 inserted, 1 skipped, 1 inserted) and no `Err` returned from `insert_events`.

### Regression tests (existing data + new schema)

10. `existing_null_uuid_events_round_trip_through_queries` — pre-create rows with `uuid IS NULL` (e.g. from `crates/tokenscale-store/src/tests.rs::sample_event`), run `aggregate_impact_by_bucket` / `list_sessions_with_totals` / `list_projects_with_totals`. Assert all return correct row counts and totals. Confirms historical events query cleanly without uuid.

---

## 5. Backfill behavior — forward-only, stated in three places

Per the user's clarification at Phase 1B sign-off, this MUST be documented in three places so a future maintainer does not "helpfully add a backfill."

### Place 1 — this scoping doc

**The new fields (`git_branch`, `uuid`, `parent_uuid`) are forward-only.** Historical events (every event ingested before v0.1.16) have `NULL` for all three columns. **No re-parse of historical JSONL.** **No backfill step is missing.**

The forward-only design is intentional:

- Re-parsing every historical JSONL on upgrade would multiply v0.1.16 install time by N (N = number of JSONL files), and would also re-fire `lines_malformed` warnings for any malformed lines that have since been fixed upstream.
- The partial UNIQUE index on `(source, uuid) WHERE uuid IS NOT NULL` correctly excludes NULL from uniqueness, so historical NULLs do not conflict with each other or with future captures. The dedup defense activates the moment a new event lands with a captured uuid.
- For uuid-collision-aware audit, the forward-only data is sufficient — Phase 0 found zero current duplicates in the existing JSONL surface, so retrospective audit value of populated uuids is bounded. New events going forward have the defense; old events don't need it.

A user who genuinely wants historical events backfilled with uuids can run `tokenscale scan --rebuild` post-upgrade. The existing rebuild path wipes events + file_state for the source and re-parses every JSONL, which would re-ingest with the v0.1.16 parser (and capture uuid + git_branch + parent_uuid into the now-populated columns). This is a user-elective action, not an upgrade-time default.

### Place 2 — migration file comment

The migration file's top-of-file comment block restates the same in operator-facing language. Copied verbatim into the migration SQL header:

```sql
-- Phase 1B-ii: parser captures gitBranch, uuid, parentUuid from JSONL
-- into events. Forward-only: historical events have NULL for all
-- three columns. No re-parse, no backfill — see
-- docs/roadmap-1b-ii-parser-captures.md § 5 for rationale.
--
-- A user who wants historical events backfilled can run
-- `tokenscale scan --rebuild` post-upgrade; this is user-elective,
-- not an upgrade-time default. The partial UNIQUE index excludes
-- NULL from uniqueness, so the lack of backfill is correct, not
-- a bug.
```

### Place 3 — v0.1.16 CHANGELOG

The v0.1.16 CHANGELOG entry's "Schema" subsection includes a one-liner:

> **Forward-only**: the three new columns are NULL for every event ingested before v0.1.16. No re-parse step runs at upgrade; this is deliberate. Run `tokenscale scan --rebuild` if you want historical events backfilled. See `docs/roadmap-1b-ii-parser-captures.md` § 5.

Three independent restatements give a future maintainer three chances to find the design intent before mistaking the NULL-historical-uuid state for a missing backfill.

---

## 6. Detector implications — none, confirmed by grep

`.github/scripts/pricing_drift_check.py` reads `pricing.toml` and `pricing-rate-card.snapshot.json` at the repo root. It does NOT touch:

- Any DB
- Any events table
- Any column the v0.1.16 migration adds or modifies

Verified via:

```
grep -rn "events\." .github/scripts/
# returns nothing
```

The detector workflow YAML (`pricing-drift-check.yml`) similarly has no DB interaction — it runs the Python detector script and opens GitHub Issues. No regression possible from v0.1.16's schema change.

**No detector changes required.** No detector tests need updating.

---

## 7. CHANGELOG framing

v0.1.16 ships in quick succession with v0.1.15 — the Phase 1B sign-off explicitly framed the split as "by risk class, not by incompleteness." The CHANGELOG must make the split deliberate.

### Lead paragraph

> **Granular Attribution Phase 1 — ingest layer.** Companion to v0.1.15, separated by risk class per the Phase 1B sign-off. v0.1.15 shipped query-layer + presentation changes (sessions tab, cwd resolution, daily handler fix); v0.1.16 ships the ingest-layer plumbing for future phases.
>
> **No new dashboard features.** Parser now captures `gitBranch`, `uuid`, and `parentUuid` from Claude Code JSONL into the `events` table. A partial UNIQUE index on `(source, uuid) WHERE uuid IS NOT NULL` is the actual defense against the same-message-different-requestId duplicate case Phase 0 flagged as the load-bearing correctness concern. Historical events stay NULL — see "Forward-only" below.

### Forward-only note

Restates §5 in user-facing language. See above.

### What this enables (signals the user-visible value is future, not present)

> The captures and the UNIQUE constraint don't surface anywhere in v0.1.16. They enable:
>
> - **Phase 1.5** — tool-use / tool-result / file-history-snapshot ingest expansion (commit attribution Tier 1/2 prerequisites; the `parentUuid` capture lets future tool-call linkage to assistant turns reconstruct edit chains)
> - **Phase 2+** — `git_branch` per-session reporting once a UI surface exists
> - **Audit value** — if Claude Code ever starts producing same-uuid duplicates, the warning surfaces it at ingest time rather than letting totals silently inflate

### Risk-class note

Last paragraph, explicit:

> Bundling these schema changes with v0.1.15's user-visible work would have concentrated migration risk with cosmetic risk. The v0.1.13 lesson (the manual workflow_dispatch firing surfaced two real bugs from a single tag concentrating multiple schema-touching changes) argued for separation. v0.1.15 stays "no migration / trivial downgrade"; v0.1.16 isolates the schema change with its own soak time. Both tags read as one coherent Phase 1 narrative across CHANGELOG entries.

---

## Open implementation questions

### I1 — Duplicate-skip logic: parser or ingest path?

**Options:**

- **A** — Parser catches before insert: parser does an existence query against the DB before emitting the Event, skips the line if duplicate.
- **B** — Ingest path catches UNIQUE violation: store layer's `insert_events` does the existence pre-check and emits the warning. (Recommended.)
- **C** — SQL UNIQUE constraint catches via `INSERT OR IGNORE`'s `rows_affected = 0`, with the count split out from the existing `skipped_duplicate` tally.

**Recommendation: B.** The parser is a pure function over JSONL strings (`parse_line` in `parser.rs`); pulling DB access into it breaks that and complicates testing. The store layer already owns the existing dedup machinery (`request_id` + `content_hash` partial indexes via `INSERT OR IGNORE`). The pre-check shape sketched in §3 is the same place those constraints live conceptually. SQL UNIQUE stays as backstop for race conditions (two scans inserting the same uuid simultaneously).

Option C is tempting (no per-row pre-check overhead) but `INSERT OR IGNORE` cannot distinguish *which* unique constraint fired — uuid-collision vs request_id re-scan look identical to the caller. Distinguishing matters per §3 (one is normal, one is exceptional). Pre-check is the cost of distinguishability.

### I2 — Warning log content

**Options:**

- **A** — Log only the new duplicate's metadata (uuid + source_jsonl + line). (Recommended.)
- **B** — Log both the new duplicate AND the conflicting existing event's metadata (extra SELECT to fetch the existing row's timestamp / request_id / model).

**Recommendation: A.** Phase 0 found zero current duplicates in 27,388 lines; the warning is precautionary. The forensic detail of the conflicting existing event has small value relative to its log-line bloat. The uuid in the warning line is sufficient to grep the existing event's row in any post-mortem. If duplicates ever materialize in the wild and the maintainer wants more context, a follow-up patch can enrich the log line — that's a v0.1.17+ refinement gated on actual incidence.

### I3 — Summary metric on `ScanSummary`

**Options:**

- **A** — Reuse the existing `events_duplicates: usize` field; aggregate uuid-collision count into the same number.
- **B** — Add a separate `uuid_duplicates_skipped: usize` field. (Recommended.)

**Recommendation: B.** Semantically distinct counts:

- `events_duplicates` (existing): "a re-scan or sync hit a row already in the DB." Steady-state value is large (every unchanged file rescan increments this), expected, silent.
- `uuid_duplicates_skipped` (new): "Claude Code emitted a same-uuid duplicate that wasn't a request_id rescan." Steady-state value is zero per Phase 0. Non-zero is the loud signal.

Bundling them into one field would hide the loud signal in the noisy one. The summary line's "(including N uuid duplicates — see logs)" clause from §3 makes the distinction visible at scan-summary level.

The new field follows the existing `ScanSummary` shape (`usize`, public, `Default::default()` of 0), so no breaking API change on `tokenscale-ingest-cc`. Tests asserting on the full struct shape need updating (a small chore; ~3 sites per Phase 0's count of struct-shape assertions in `scan.rs::tests`).

---

## Output expected from sign-off

After this scoping doc + I1–I3 recommendations are approved:

1. v0.1.16 implementation lands: migration, parser changes, ingest pre-check, scan-summary field, ~10 tests per §4, CHANGELOG entry per §7.
2. Commit + tag v0.1.16 + push.
3. Phase 1.5 scoping (tool-use ingest expansion) is the next scoping pass after v0.1.16.

No code in this pass.

---

## Sequencing summary

```
v0.1.15  (released d90b1ed, 2026-05-22)
  ├─ Phase 1A — per-session reporting
  ├─ Phase 1B-i — cwd → git toplevel resolution
  └─ Phase 1B-iii — daily_handler modelsWithoutPricing fix

v0.1.16  (this scoping doc, next release)
  └─ Phase 1B-ii — parser captures + UNIQUE on uuid + skip-with-warning

v0.1.17+ (future)
  └─ Phase 1.5 — tool-use / tool-result / file-history-snapshot ingest expansion
     (gates Phase 2 commit attribution Tier 1 + Phase 3 edit-survival Tier 2)
```
