# Changelog

Notable changes per release. Format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

Newest releases on top. Unreleased changes accumulate under `## Unreleased`.

---

## v0.1.19, 2026-06-16

**Model additions (Fable 5 + Opus 4.8) plus subagent ingest.** Three changes that travel together: (1) pricing + environmental-factor coverage for two models that were used but unpriced/unfactored, (2) a walker fix so subagent transcripts are ingested at all, and (3) the bug that combination surfaced. See `docs/roadmap-model-additions-fable-opus48.md` for the full design pass (D1 through D5, sign-off, and the §8 smoke findings).

### Two models added

`claude-fable-5` and `claude-opus-4-8` are now in `pricing.toml` (file_version 1.0 to 1.1) and `environmental-factors.toml` (file_version 0.3 to 0.4). Rates web-sourced and verified against Anthropic's pricing page on 2026-06-16 (the same page the existing rows cite); while sourcing, the full rate card was re-verified and every pre-existing row is unchanged.

- **Opus 4.8** ($5/$25 in/out, identical to Opus 4.6/4.7; launched 2026-05-28, sourced). Environmental factors held flat to Opus 4.7 on the pricing-unchanged proxy.
- **Fable 5** ($10/$50 in/out, 2x Opus, the priciest GA model Anthropic shipped; launched 2026-06-09, disabled 2026-06-12 by a US export-control directive ~3 days later). Carries `status = "retired"`. Environmental factors are a pricing-as-proxy estimate (2x Opus 4.8) with a wide +/-55% band and an explicit zero-anchor note: Fable existed 3 days, so there is no Couch/Jegham analysis or first-party disclosure and none is expected. This is an honest estimate, not a measurement.

### Subagent ingest (walker recursion)

The walker (`crates/tokenscale-ingest-cc/src/walker.rs`) previously read only `<root>/<project>/*.jsonl` and never descended into `<project>/<session-id>/subagents/agent-*.jsonl`. Subagent token spend is real account usage that prior versions silently dropped. The walker now recurses within each project directory (stray root-level files are still skipped). **This shifts every model's historical totals upward** on the next scan as subagent usage is counted for the first time; it is a correctness improvement, not a regression. Backfills on the next scan (subagent files are simply new files the walker had never recorded); `--rebuild` remains the clean re-derivation path.

In the maintainer's corpus this took the scan from 60 files seen to 264, and total events from 36,910 to 42,899.

### Drift detector

`claude-opus-4-8` and `claude-fable-5` added to the detector's tracked set, snapshot, and fixture. Fable carries a retired-model guard: a model marked `status = "retired"` in pricing.toml that is absent from the live pricing page is logged and skipped rather than raising a ParseFailure, so a future delisting cannot cry-wolf (the v0.1.14 "cry-wolf kills trust" lesson). The live detector run is clean: pricing.toml matches Anthropic's page for all 6 tracked models.

### Smoke-test-surfaced fix during build (§8 release gate)

The gate held its five-for-five-plus pattern. Subagent ingest surfaced Haiku 4.5 usage under the dated Bedrock-style ID `claude-haiku-4-5-20251001` (1,202 events), which did not match the assumed-shortened `claude-haiku-4-5` key on the pricing and factor rows, so the usage rendered both unpriced and unfactored. Root cause: the rows were keyed on a convenience-shortened ID that real usage never emits.

Fixed with explicit `claude-haiku-4-5-20251001` alias rows (identical rates/factors) in both data files. Post-fix, the live `/usage/daily` reports `modelsWithoutPricing: []` and `modelsWithoutFactors: []` for the full window: every model in the corpus is now both priced and factored. General model-ID normalization (strip `-YYYYMMDD`, resolve variant forms) is the proper fix, tracked in [Issue #7](https://github.com/RobarePruyn/tokenscale/issues/7).

**Principle reinforced:** unpriced is not unfactored. Environmental impact (energy / water / CO2e) is captured for every model actually used, for every Anthropic model ever released, regardless of billability. A bare `sonnet` string (7 mentions) was investigated and is an `Agent` tool-call argument, not a usage event; the subagent that runs on Sonnet records its own usage under the resolved ID and factors normally, so no impact is lost.

### Tests

- Walker: `walker_recurses_into_subagent_subdirectories` (discriminating; fails against the old one-level walker).
- Drift detector: 3 new tests for the retired-model guard and `load_retired_model_ids` (20 Python tests green; live run clean).
- Factor + pricing rows validated through the real Rust parsers (the embedded-file load tests parse the actual on-disk files with the new rows).

Workspace tests green; no clippy warnings in changed Rust files (pre-existing lint debt, Issue #1, untouched).

### Data-sync posture

`pricing.toml` and `environmental-factors.toml` are replace-on-startup synced (not forward-only migrations): edit the file, restart, the next sync rewrites the table. No schema migration in this release. Subagent history ingests on the next scan.

---

## v0.1.18, 2026-05-26

**Phase 2, Tier 1 commit attribution.** First user-visible attribution layer on top of v0.1.17's tool-use ingest: a new `session_commits` table populated at scan time captures every `git commit` Bash invocation per session, with the SHA extracted from the tool_result content. A new endpoint `GET /api/v1/sessions/{session_id}/commits` returns those commits with per-row resolution status against the current tree.

This release also bundles the fix for Issue [#6](https://github.com/RobarePruyn/tokenscale/issues/6) (the v0.1.17 `--rebuild` semantic gap), so a one-time `tokenscale scan --rebuild` after upgrading deduplicates the tool tables that were retained across v0.1.17 rebuilds. **v0.1.17 users should run `tokenscale scan --rebuild` after upgrading** to dedupe `tool_uses`, `tool_results`, and `file_snapshots` rows that doubled under the v0.1.17 `--rebuild` semantic gap.

See `docs/roadmap-2-commit-attribution.md` for the full design pass (six D-decisions: D1 schema, D2 dual-regex SHA capture, D3 query-time SHA resolution, D4 project attribution plus /tmp filter, D5 diagnostic fields, D6 `--amend` handling). The scoping doc also captures the four documented D2 failure modes (commit-and-push-to-different-repos, multi-commit-then-push, push-refspec old/new ambiguity, new-branch push with no `<old>..<new>` range), the `recovery_source = none` semantic conflation as a known limitation, and the Phase 3 scoping input to evaluate splitting `none` into three sub-classes (committed-unlinkable, commit-failed, unknown) for the exact-but-partial honesty story.

### Schema (`migrations/20260526000001_session_commits.sql`)

One CREATE TABLE plus four indexes (one UNIQUE, three secondary, one partial). Same `(source, *_id) WHERE * IS NOT NULL` idiom v0.1.16 introduced. `(source, tool_use_id)` is the natural unique key per the v0.1.17 `tool_uses` UNIQUE shape; session_id is denormalised for query convenience.

### Commit extraction (the `commit_extract` module)

Three pure-function pieces wired together in `crates/tokenscale-store/src/commit_extract.rs`:

1. **`is_real_commit_command`**: the canonical filter from D-decision §1.1 of the scoping doc. shlex-tokenises each `&&` / `;` / `||` subcommand and checks for `["git", "commit"]` as the first two tokens. The naive `LIKE '%git commit%'` overcounts by 4.7x in the maintainer corpus (671 raw hits versus 143 real invocations); meta-mentions like sqlite queries, probe scripts, and `grep "git commit"` lines fail this filter.
2. **`extract_sha`**: primary regex `[<branch>( \(root-commit\))? <sha>]` covers 93.7% of clean commit invocations in the corpus. Push-refspec fallback regex `<old>..<new>  <local> -> <remote>` covers an additional 3.5% (the `git push` line when chained), lifting cumulative coverage to 97.2%.
3. **Flag detection**: `--amend` and the `| tail` truncation indicator (`output_head_truncated_by_command`) are detected by regex on the command text.

### Scan summary

`ScanSummary` gains one new field:

| Field | Meaning |
|---|---|
| `session_commits_inserted` | New `session_commits` rows that landed this scan. Re-scan of unchanged data lands 0 (INSERT OR IGNORE on `(source, tool_use_id)` UNIQUE). |

The CLI `tokenscale scan` summary line is unchanged in steady-state; the new count surfaces in tracing logs. The full `SessionCommitInsertSummary` (with the `bash_tool_uses_scanned`, `bash_tool_uses_filtered_out`, `session_commits_duplicates` breakdown) is logged at `debug` for operators inspecting scan output.

### HTTP surface

`GET /api/v1/sessions/{session_id}/commits` returns the per-session commit list. Each row carries:

- `tool_use_id`, `session_id`, `occurred_at`
- `sha: Option<String>` (the captured SHA; NULL for ~3.9% of real commit invocations where neither regex captures, breakdown per §1.11 of the scoping doc: 19 head-piped or new-branch-push cases, 3 real failures, 2 orphans, 1 testing-in-/tmp)
- `sha_resolves_in_tree: Option<bool>` (filled in at query time via batched `git cat-file --batch-check` against `project_resolved`; one git invocation per unique project)
- `cd_target_raw: Option<String>` (verbatim `cd "<path>"` target if present)
- `project_resolved: String` (cwd_resolver output; cd target preferred, project_id fallback)
- `recovery_source: "primary" | "push_refspec" | "none"`
- `output_head_truncated_by_command: bool`
- `is_amend: bool`

`?include_testing=true` surfaces /tmp testing commits which are filtered by default per D4a.

### Smoke-test-surfaced fixes during build (§9 release gate)

The §9 release-gate prediction held: real-DB smoke surfaced a bug that no test caught, in the canonical shlex-token-walk filter. The `split_subcommands` helper that walks `&&` / `;` / `||` separators was traversing heredoc body content as if it were shell, which allowed probe scripts (Python heredocs embedded inside `python3 <<'PY' ... PY` blocks) to slip past the filter when their body contained `git commit` substrings AND a separator (a `;` or `&&` in a Python expression).

**Fix in commit, pinned by tests.** The pre-fix `split_subcommands` returned `Vec<&str>` over the raw command string. The post-fix version first calls a new `strip_heredoc_bodies` helper that detects `<<MARKER`, `<<'MARKER'`, `<<"MARKER"`, and `<<-MARKER` patterns and skips body lines until the matching closer; then it splits the heredoc-stripped command on top-level separators. Five regression tests pin the fix; one is broad-pattern coverage and one is the discriminating test that fails without `strip_heredoc_bodies` (verified by bypass-patching the helper to a no-op during the §9 tag-gate sanity check):

- `probe_script_with_heredoc_python_body_does_not_pass_filter` (broad pattern; does not discriminate on its own because its body fragment has an unbalanced quote that shlex rejects)
- `discriminating_test_heredoc_body_with_amp_amp_git_commit_fragment_fails_filter` (the DISCRIMINATING test; balanced-quote fragment that shlex parses to `[git, commit, ...]`; fails without the fix)
- `strip_heredoc_bodies_handles_single_and_double_quoted_markers`
- `strip_heredoc_bodies_handles_unquoted_marker`
- `split_subcommands_does_not_split_on_separators_inside_heredoc_body`

**Empirical consequence on the scoping-doc baseline.** The probe report's "143 real commits" count (`docs/roadmap-2-probe-report.md` § 1.1) was an undercount because the Python probe script that produced it had the same heredoc-body-splitting bug. The maintainer's real corpus has 637 real `git commit` invocations once the v0.1.18 implementation runs against the rebuilt DB. The structural conclusions from the probe report all hold (narrow shape, dual-regex captures most, sample-dependency on `| tail`, /tmp filter useful) but the absolute scale shifts.

**Corrected empirical baseline against the rebuilt DB** (use these in Phase 3 scoping rather than the probe-report numbers):

| Metric | Probe-report value | v0.1.18 actual |
|---|---:|---:|
| Real commit invocations | 143 | 637 |
| SHA capture (combined) | 97.2% | 96.1% |
| recovery_source = primary | 93.7% | 90.9% |
| recovery_source = push_refspec | 3.5% | 5.2% |
| recovery_source = none | 2.8% | 3.9% |
| cd-prefix rate | 86.0% | 84.9% |
| is_amend rate | 2.1% (3 commits) | 0.5% (3 commits) |
| /tmp filter suppressed | 1 | 1 |
| `output_head_truncated_by_command` (operator pipe through tail) | 4.9% (the no-SHA subset) | 57.6% (all command-structure matches; full population) |

The is_amend count (3 commits) is identical between the two passes because none of the three `--amend` commands happened to have a heredoc-body-semicolon issue that would fragment them. The /tmp filter count is also stable at 1.

The `output_head_truncated_by_command` jump (4.9% to 57.6%) reflects a framing difference, not a bug: the probe-report counted the NO-SHA subset attributable to head-piping; the v0.1.18 implementation counts every commit whose command contains `| tail`, including the many where the commit's output was short enough that `| tail -3` or `| tail -5` kept the head. Cross-tabbing with `recovery_source` separates the operator-caused misses (head_truncated AND recovery_source=none) from operator-pipe-with-recovery. The flag is a risk signal about command structure, not an outcome signal about truncation; the dashboard can use it either way.

### Tests

7 new tests in `commit_data` + 12 unit tests in `commit_extract`, plus the §9 aggregation regression test (`aggregate_impact_by_bucket_numbers_unchanged_by_session_commits_inserts`) that pins session_commits against leaking into the cost/impact aggregation path. The shlex token-walk filter is canonical, not a one-off probe artifact; it lives in `commit_extract` as documented in §1.1 of the scoping doc.

### Issue [#6](https://github.com/RobarePruyn/tokenscale/issues/6) fix bundled

The destructive `--rebuild` path in `crates/tokenscale-cli/src/main.rs` now wipes all five CC-source tables (`events`, `tool_uses`, `tool_results`, `file_snapshots`, `session_commits`) in addition to `file_state`. v0.1.17 only wiped `events` and `file_state`, which left the three v0.1.17-added tables with retained rows from prior partial scans (UNIQUE-keyed; INSERT OR IGNORE silently kept the doubled-up state). The new WARN line reports per-table deletion counts.

For v0.1.17 users: re-running `tokenscale scan --rebuild --yes` after upgrading to v0.1.18 produces the clean wipe-and-reinsert that v0.1.17's `--rebuild` was supposed to do, dropping the retained stale rows.

### Forward-only

The `session_commits` table is forward-only: historical sessions whose tool_uses rows landed before v0.1.18 ships will not have `session_commits` rows. The ingest path populates this table only at scan time going forward. Stated in three places (this CHANGELOG, `migrations/20260526000001_session_commits.sql` header comment, `docs/roadmap-2-commit-attribution.md` §8) per the project pattern. Run `tokenscale scan --rebuild --yes` for the user-elective backfill (which now correctly wipes all five tables thanks to the Issue #6 fix).

### Sample-dependency caveat (exact-but-partial honesty)

Phase 2 commit attribution achieves **96.1% SHA capture** in the maintainer's corpus at the v0.1.18 baseline (637 real commit invocations). The 3.0% irreducible-miss rate from head-piped-without-recoverable-push cases (19 of 637) is attributable to project-specific maintainer command patterns, notably `| tail -N` piping of `git commit` output to compress for context-window reasons, plus new-branch pushes where the `git push` output carries no `<old>..<new>` range for the push-refspec fallback to match. The pattern is widespread across the maintainer's projects (per-project head-pipe rates range from 38% to 100% at the v0.1.18 baseline; see scoping doc §1.11) rather than concentrated in any single cohort. Other deployments will have different rates depending on their command patterns and feature-branch workflows; the dashboard surfaces this via `output_head_truncated_by_command: true` on affected rows so operators can identify which commits are head-cut by their own command structure. See `docs/roadmap-2-probe-5-report.md` for the original empirical evidence and `docs/roadmap-2-commit-attribution.md` §1.11 for the corrected v0.1.18 baseline.

### What this enables

- **Phase 3** (Tier 2 edit-survival): `list_session_file_edits` (v0.1.17 stub) plus a new `git blame` pass over the current tree. Phase 2's `project_resolved` populated at insert time is reusable.
- **Phase 4** (Tier 3 forward instrumentation): post-commit hook writing `Tokenscale-Session:` trailer becomes the exact-attribution path going forward. Tier 1's row-per-invocation shape is what the Phase 4 hook attaches to.
- **Per-project commit reports**: the `project_resolved` denormalisation plus the `session_id` index supports "all commits in project X across sessions" queries cheaply.

---

## v0.1.17 — 2026-05-24

**Phase 1.5 — tool-use ingest expansion.** No new dashboard features; this is plumbing for Phase 2 (Tier 1 commit attribution: which commits did CC author?) and Phase 3 (Tier 2 edit-survival: how much CC-authored code is still in the repo?).

Three new tables — `tool_uses`, `tool_results`, `file_snapshots` — capture data that v0.1.16's parser dropped: every `tool_use` block inside an assistant message's `content` array (Bash, Edit, Read, Write, TodoWrite, etc.), every `tool_result` inside a user message, and the `trackedFileBackups` dict from each `file-history-snapshot` line. **Forward-only**: historical events have no rows in these tables; run `tokenscale scan --rebuild` for a backfill.

### Schema (`migrations/20260524000001_tool_use_ingest.sql`)

Three CREATE TABLE statements + eight indexes, all additive. Same `(source, *_id) WHERE * IS NOT NULL` partial-UNIQUE idiom v0.1.16 introduced — re-scans dedup via `INSERT OR IGNORE`. `(source, uuid)` keying explicitly (not `(uuid)` alone) keeps the door open for Phase 1.5+ ingest expansions adding new sources.

### Parser changes

`ParseOutcome::Event(Box<Event>)` becomes `ParseOutcome::Records(Box<ParsedRecords>)` (D2 sign-off: one-to-many at parse). `ParsedRecords` carries the optional `Event` plus `Vec<ToolUse>`, `Vec<ToolResult>`, `Vec<FileSnapshot>` — any combination may be empty depending on line type. User lines and `file-history-snapshot` lines are now parsed (were dropped via the `JsonlLine::Other` catch-all in v0.1.16).

Linkage between `tool_use` and `tool_result` resolves at **query time** (D3 sign-off) via SQL JOIN on `(source, tool_use_id)`. Phase 0's empirical 100% linkage rate (every observed `tool_result` references a known `tool_use`) makes the JOIN reliable; parser stays pure.

### Scan summary

`ScanSummary` gains four new fields:

| Field | Meaning |
|---|---|
| `tool_uses_inserted` | New `tool_uses` rows landed this scan. Re-scan of unchanged file = 0 (INSERT OR IGNORE). |
| `tool_results_inserted` | Same shape for `tool_results`. |
| `file_snapshots_inserted` | Same shape for `file_snapshots`. **NOT per file-history-snapshot RECORD** — per `(snapshot_message_id, file_path)` row. CC's `trackedFileBackups` is an open set that re-emits on every snapshot; a session with N snapshots tracking K files lands N×K rows. |
| `tool_use_orphans` | **Addition 1 from sign-off.** Count of `tool_use` rows in the source whose `tool_use_id` has no matching `tool_result` (interrupted sessions). Phase 0 baseline: 5 on maintainer data. Not a release gate; non-zero growth is the loud signal that upstream CC behavior changed. |

The CLI `tokenscale scan` summary line is unchanged in steady-state; the new counts surface in tracing logs (`?summary` field of the "scan complete" INFO line).

### Smoke-test-surfaced fixes during build (§7 release gate)

Two bugs caught in real-DB smoke before tag — exactly the pattern §7 warned about. Fixed in this commit; pinned by tests.

1. **`UserMessage.content` shape mismatch.** Real CC user lines carry `content` either as a string (text-only input like `<task-notification>` blocks) OR as an array of typed blocks. My initial `Vec<MessageBlock>` was too strict and rejected 42 real user lines as `ParseOutcome::Malformed` against the maintainer's real DB (was 0 malformed in v0.1.16). Fixed with an `UserContent` untagged enum (`Text(String)` | `Blocks(Vec<MessageBlock>)`) — text-only lines degrade to Skip cleanly; tool-result lines parse as before. Real-DB smoke now reports `lines_malformed: 0` again.
2. **`FileHistorySnapshotPayload` schema-drift tolerance.** The pre-existing `realistic_session.jsonl` test fixture used a different snapshot shape than real CC data (top-level `timestamp` + `files` array vs. the real nested `snapshot.trackedFileBackups`). My initial implementation produced `Malformed` on the legacy shape. Made `messageId` and `snapshot` both `Option`-defaulted; missing-essential-field → `Skip` cleanly. Same posture as v0.1.16's `AssistantUsage::default()`.

The §7 release-gate prediction held: every schema-touching release in this arc has surfaced a real bug during smoke that no test caught (v0.1.13's `daily_handler` conflation, v0.1.14's `load_pricing_toml` + missing label, v0.1.15's `MIN(project_id)`, v0.1.16's pre-check inversion, v0.1.17's `UserContent` rigidity). **The bug-find is the expected outcome; the release isn't tagged until it's found.**

### Phase 2/3 query primitive stubs

`list_session_bash_calls` and `list_session_file_edits` ship as functional stubs in `tokenscale-store`. Bodies are real (Phase 2/3 will use them as-is), but **no handler exposes them in v0.1.17**. Phase 2/3 can build against the committed signatures.

### Tests

7 new tests across `tokenscale-store` + `tokenscale-ingest-cc/tests/fixture_sample_session.rs`:

- `tool_data::tests::insert_tool_data_lands_three_record_kinds`
- `tool_data::tests::insert_tool_data_is_idempotent_on_rescan`
- `tool_data::tests::count_tool_use_orphans_returns_unmatched_count`
- `tool_data::tests::list_session_bash_calls_joins_through_tool_use_id`
- `tool_data::tests::list_session_file_edits_filters_to_edit_and_write`
- `tool_data::tests::aggregate_impact_by_bucket_numbers_unchanged_by_tool_data_inserts` — **the §7 aggregation regression test**; pins that the new tables don't leak into cost/impact aggregation.
- `tests/fixture_sample_session.rs::sample_session_fixture_lands_expected_tool_data` — end-to-end against the new representative `sample-session.jsonl` fixture (Addition 4 from sign-off: fixtures live in files, not strings); covers all top-5 tool names + max=3 tool_use case + one orphan + one non-empty file-snapshot + cross-line linkage.

**209 workspace tests green** (was 202 in v0.1.16; +7).

### Forward-only

The three new columns are NULL/empty for every event ingested before v0.1.17 — no historical backfill at upgrade. Stated in three places (this CHANGELOG, migration file header comment, `docs/roadmap-1.5-tool-use-ingest.md` § 5) so a future maintainer doesn't mistake the empty-historical-rows for a missing backfill step. `tokenscale scan --rebuild` is the user-elective re-ingest path; the query-time linkage (D3) means rebuild file ordering doesn't matter.

### Real-DB smoke artifacts

Against maintainer's production DB (29 files, 3 freshly parsed during the smoke):

- `events_inserted: 19` / `events_duplicates: 13,558` (steady-state rescan posture)
- `uuid_duplicates_skipped: 0` (v0.1.16 invariant holds)
- `tool_uses_inserted: 18` / `tool_results_inserted: 18` (1:1 in this batch — no orphans landed this scan)
- `file_snapshots_inserted: 345` (delta; cumulative DB count is 147,379 across 1,141 distinct snapshot records, ~129 files per snapshot — CC's `trackedFileBackups` is an open set, this is the natural row count)
- `tool_use_orphans: 4` (within Phase 0's baseline of 5; the difference is partial scan coverage, not regression)
- `lines_malformed: 0` (post-UserContent-fix; was 42 before)

### What this enables

- **Phase 2** — Tier 1 commit attribution. `list_session_bash_calls` returns Bash invocations + results; filter for `git commit` and extract SHAs from result text.
- **Phase 3** — Tier 2 edit-survival. `list_session_file_edits` returns Edit/Write file paths per session; `git blame` the current tree to measure survival rate.
- **Phase 1.5 Addition 1 audit value** — orphan count surfaces upstream schema drift cheaply if CC's behavior ever changes.

---

## v0.1.16 — 2026-05-24

**Granular Attribution Phase 1 — ingest layer.** Companion to v0.1.15, separated by risk class per the Phase 1B sign-off. v0.1.15 shipped query-layer + presentation changes (sessions tab, cwd resolution, daily-handler fix); v0.1.16 ships the ingest-layer plumbing for future phases.

**No new dashboard features.** Parser now captures `gitBranch`, `uuid`, and `parentUuid` from Claude Code JSONL into the `events` table. A partial UNIQUE index on `(source, uuid) WHERE uuid IS NOT NULL` is the actual defense against the same-message-different-requestId duplicate case Phase 0 flagged as the load-bearing correctness concern. Historical events stay NULL — see "Forward-only" below.

### Schema (`migrations/20260523000001_parser_captures.sql`)

```sql
ALTER TABLE events ADD COLUMN git_branch  TEXT;
ALTER TABLE events ADD COLUMN uuid        TEXT;
ALTER TABLE events ADD COLUMN parent_uuid TEXT;

CREATE UNIQUE INDEX events_source_uuid_unique
    ON events (source, uuid)
    WHERE uuid IS NOT NULL;
```

Forward-only, additive only. Matches the existing partial-WHERE-index idiom from `20260428000001_initial.sql`. `(source, uuid)` not `(uuid)` alone — see `docs/roadmap-1b-ii-parser-captures.md` § 1 for the rationale (matches the existing `(source, request_id)` convention; safer once Phase 1.5 introduces additional ingest sources).

### Forward-only

The three new columns are **NULL for every event ingested before v0.1.16**. No re-parse step runs at upgrade — this is deliberate, not a missing backfill. Run `tokenscale scan --rebuild` if you want historical events backfilled (user-elective, not upgrade-time default). The partial UNIQUE index excludes NULL from uniqueness, so historical NULLs do not conflict with each other or with future captures.

Stated in three places (this CHANGELOG, the migration file header, and `docs/roadmap-1b-ii-parser-captures.md` § 5) so a future maintainer doesn't mistake NULL-historical-uuid for a missing backfill.

### Duplicate detection — skip-with-warning

Per Phase 1B sign-off § D2: storage + UNIQUE + skip-with-warning over fail-line-outright.

- **Pre-check in `tokenscale-store::insert_events`** (I1.5 Option A, see scoping doc): for each event with a non-NULL `uuid`, query the existing row's `request_id`. Distinguishes:
  - **Same uuid + same request_id** = "I rescanned an unchanged file." Normal, silent. Falls through to `INSERT OR IGNORE`, counts via the existing `skipped_duplicate` channel.
  - **Same uuid, DIFFERENT request_id** = "CC behavior change." Exceptional, loud. Skip + add the event's index to `summary.uuid_duplicate_indices` so the caller emits a per-event `WARN` log with the uuid, source, JSONL path, and line number.
- **SQL `UNIQUE` constraint stays as backstop** for races (two parallel scans inserting the same uuid simultaneously).
- **`ScanSummary.uuid_duplicates_skipped: usize`** is a new field, distinct from `events_duplicates` (the existing noisy rescan count). Steady-state value is `0` per Phase 0's empirical finding (27,388/27,388 lines, zero duplicates). Non-zero is the loud signal.

The user-facing `tokenscale scan` summary line gains a conditional clause:

```
Scan complete: 29 files seen, 3 parsed, 26 unchanged. 216 new events, 12911 duplicates skipped. 16704 non-assistant lines, 0 malformed.
```

becomes, when uuid duplicates are non-zero:

```
Scan complete: 29 files seen, 3 parsed, 26 unchanged. 216 new events, 12911 duplicates skipped (including 3 uuid duplicates — see logs). 16704 non-assistant lines, 0 malformed.
```

Zero uuid duplicates means the line reads exactly as before. **The signal is loud only when it fires.**

### Parser

`crates/tokenscale-ingest-cc::parser`'s `AssistantPayload` gains three new `#[serde(default, rename = "…")] Option<String>` fields. Missing fields → row emits with that field as NULL, parse succeeds, debug log surfaces missing uuids for schema-drift detection.

The JSONL schema for these fields is not documented upstream; the assumed shape is reverse-engineered from observed lines and consistent across all 27,388 lines in the maintainer's 48-file sample. See scoping doc § 2.

### Tests

13 new tests across the workspace:

- **5 parser tests** (`tokenscale-ingest-cc::parser::tests`): all three fields captured; each individually missing → NULL with others intact; missing all three → all NULL (pre-v0.1.16 JSONL shape regression).
- **6 ingest tests** (`tokenscale-store::tests`):
  - `duplicate_uuid_different_request_id_is_skipped_with_warning` — the load-bearing case; pre-check catches it, counts in `uuid_duplicate_indices`, doesn't increment `skipped_duplicate`.
  - `duplicate_uuid_same_request_id_is_a_silent_rescan_not_a_uuid_collision` — the smoke-test-surfaced regression; pre-check distinguishes rescan from CC behavior change.
  - `two_events_with_null_uuid_both_land` — partial UNIQUE index excludes NULL.
  - `unique_violation_does_not_abort_ingest_run` — middle-of-batch collision doesn't propagate as Err.
  - `same_uuid_different_source_both_land` (**Addition 3** per sign-off) — pins the `(source, uuid)` choice against a future refactor that might drop `source` from the key.
  - `null_uuid_event_round_trips_through_queries` — pre-v0.1.16 events query/aggregate cleanly post-migration.
- **2 existing test fixtures** (`InsertSummary` struct literals in `tokenscale-store::tests`) updated for the new `uuid_duplicate_indices` field.

`202` workspace tests green (was `189` pre-v0.1.16; +13).

### Smoke-test results on maintainer's real DB

- Migration `20260523000001` applied cleanly on top of v0.1.15-shape DB
- 22,769 total events: 22,553 historical (NULL uuid) + 216 new (all three captures populated)
- **0 uuid duplicates** detected in the entire 216-event new-ingest batch (Phase 0's finding holds in production)
- **0 duplicate uuids in the non-NULL subset** (UNIQUE partial index working)
- Scan summary's conditional uuid-duplicate clause correctly omits when count is zero

### Build-pass artifacts

- **`SELECT *` grep over the workspace**: 0 hits across `crates/`, `migrations/`, `.github/scripts/`. The "v0.1.15 binary cleanly ignores v0.1.16 columns" downgrade story is structurally guaranteed, not just empirically.
- **I1.5 path chosen**: Option A (`insert_events` returns `uuid_duplicate_indices: Vec<usize>`; scan caller owns the per-event provenance map and emits the final warning). Diff is smaller than Option B (no leak of ingest-only fields into the core `Event` struct); concerns stay where they belong (store owns dedup, scan owns file context).
- **Detector check**: `grep -rn "events\." .github/scripts/` returns 0 hits. The drift detector reads `pricing.toml` + the snapshot, never touches `events`. No detector changes required.

### Risk-class note

Bundling these schema changes with v0.1.15's user-visible work would have concentrated migration risk with cosmetic risk. The v0.1.13 lesson (the manual workflow_dispatch firing surfaced two real bugs from a single tag concentrating multiple schema-touching changes) argued for separation. v0.1.15 stayed "no migration / trivial downgrade"; v0.1.16 isolates the schema change with its own soak time. Both tags read as one coherent Phase 1 narrative across CHANGELOG entries.

### What this enables

The captures + UNIQUE don't surface anywhere in v0.1.16. They enable:

- **Phase 1.5** — tool-use / tool-result / file-history-snapshot ingest expansion (commit attribution Tier 1/2 prerequisites; `parentUuid` lets tool-call linkage reconstruct edit chains)
- **Phase 2+** — `git_branch` per-session reporting once a UI surface exists
- **Audit value** — if Claude Code ever starts producing same-uuid-different-requestId duplicates, the warning surfaces it at ingest time rather than letting totals silently inflate

---

## v0.1.15 — 2026-05-22

**Granular Attribution Phase 1 — user-visible layer.** Per-session reporting and cwd → git toplevel resolution for the project view. Companion release v0.1.16 follows with the ingest-layer changes (parser captures + UNIQUE constraint on uuid); the split is by risk class, not by incompleteness — see "Why two releases" below.

This release bundles three concerns: Phase 1A (per-session reporting, already present on `main` since `818f33a`), Phase 1B-i (cwd → git toplevel resolution), and Phase 1B-iii (a small `/api/v1/usage/daily` `modelsWithoutPricing` correctness fix surfaced during 1A smoke-testing).

### Added

- **`GET /api/v1/usage/sessions`** — per-session impact + cost endpoint (Phase 1A). Same window / provider / project filter surface as `/usage/daily`, but `GROUP BY events.session_id` instead of date bucket. Reuses the v0.1.13 per-event correlated-subquery pricing + factor joins; per-session numbers inherit time-anchoring verbatim. Includes `limit` + `offset` query params from day one for API forward-compat with future server-side pagination (default `limit=10000`, `offset=0` — effective no-op for any realistic 90-day window).
- **Sessions tab in the dashboard** — new third top-level view between Dashboard and Methodology. Sortable table over the fetched response (client-side; `DEFAULT_SESSION_LIMIT=10000`). Sortable columns: project, firstEventAt, lastEventAt, eventCount, totalTokens, costUsdTotal, energyWh. Null `costUsdTotal` renders as `—` matching the per-bucket missingness convention from v0.1.13. Lazy-loaded — pays nothing if the tab is never opened.
- **`truncateSessionId(s)` frontend helper** — single source of truth for the display rule (first 8 characters of the UUID). Matches `git` short-SHA convention and the prefix of `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` filenames so a user grepping a displayed prefix hits the right file. Full UUID lives in `title=` tooltips and the raw API response for grep against logs.
- **cwd → git toplevel resolver** (Phase 1B-i) — `tokenscale-server::cwd_resolver` module. Built at server startup from the DB's distinct `project_id` values via `git -C <cwd> rev-parse --show-toplevel`. Bidirectional in-memory map: forward (raw cwd → resolved name) for display, reverse (resolved → list of raw cwds) for expanding `?project=<resolved>` filters into the SQL `WHERE project_id IN (...)` clause. Graceful fallback: a cwd whose directory no longer exists on disk OR isn't a git repo maps to itself, matching pre-v0.1.15 dashboard behavior. See `docs/roadmap-1b-cwd-resolution.md` § D1 for the choice of query-time over ingest-time.
- **`/api/v1/projects` collapses raw cwds into resolved projects** — a single repo accessed from multiple subdirectories / worktrees now appears as one row in the project filter list with summed event counts and token totals. The "67 projects" fragmentation the maintainer's data exhibited is now down to ~30 (varies by user; collapse rate depends on how many distinct cwds map to the same toplevel). Worktrees count as distinct projects per § D4 — native `git rev-parse --show-toplevel` behavior; the A1 → A2 promotion (collapse worktrees to main repo) is a future one-line change if a user pattern emerges.
- **Per-session project attribution heuristic** — when a session spans multiple cwds (e.g. one event with `cd ~` plus events in a deep repo subdirectory), the session-row attributes to the LONGEST cwd seen (ties broken alphabetically). v0.1.15 smoke test surfaced that the maintainer's home directory is a git repo, so the original `MIN(project_id)` aggregation attributed lots of work to `/Users/<name>` instead of the actual repo. Longest-cwd-wins is more robust than alphabetic MAX (which fails on root-letter ASCII edge cases like `/private/...` beating `/Users/...`). Truly correct attribution = most-frequent cwd via window function; longest-wins is the v0.1.15 heuristic that handles the observed common case. Tracked for a future refinement.

### Changed

- **`modelsWithoutPricing` on `/api/v1/usage/daily`** (Phase 1B-iii correctness fix) — switched from time-anchored `pricing.lookup` to a structural "is this model in `pricing.toml` at all?" check. Previously, a model with `valid_from = 2026-04-16` queried over a window starting 2026-04-01 would appear in `modelsWithoutPricing` for the first 15 days even though the model IS priced (just not yet on those dates). Per-event missingness is already correctly surfaced via `eventsMissingPricing` per cell; this banner is the model-level "no row at all" signal. `sessions_handler` already shipped with the correct structural check (1A landing); both handlers now share the same `model_has_pricing_row` / `model_has_factor_row` helpers in `routes/usage.rs`. Per-event time-anchored pricing on `/usage/daily` is unaffected — only the model-list banner could over-flag.

### Why two releases

Phase 1B's parser changes (capturing `gitBranch` / `uuid` / `parentUuid` into `Event` + UNIQUE partial index on `uuid` as the actual defense against the duplicate-message concern from Phase 0) ship as **v0.1.16** instead of bundling here.

Rationale: parser changes carry ingest risk; cwd-resolution doesn't. The v0.1.13 lesson (the manual `workflow_dispatch` firing in v0.1.14 surfaced two real bugs from a single tag concentrating multiple schema-touching changes) argues for separating risk classes cleanly. v0.1.15 stays scoped to "query-layer and presentation-layer" — no migrations, no parser changes, downgrade trivial. v0.1.16 will carry the schema migration and parser captures with its own soak time before brew users see it.

This is not v0.1.15 being incomplete — it's the deliberate D5 split documented in `docs/roadmap-1b-cwd-resolution.md`. The two tags land in quick succession with a coherent narrative across both CHANGELOGs.

### Tests

- 8 sessions_query tests (Phase 1A's 6 + 2 new for the multi-cwd attribution: `multi_cwd_session_picks_most_specific_project` and `longest_cwd_wins_over_higher_ascii_root`). Both Additions called out in 1A signoff are present: per-session sum invariant + multi-model session aggregation.
- 6 cwd_resolver tests covering subdirectory collapse, missing-directory fallback, non-git directory fallback, worktree-as-project (D4 pinned), reverse lookup, and the empty-resolver default.
- All 13 server test fixtures updated to pass an empty resolver via `test_resolver()`.

### Smoke-tested against maintainer's production DB

- 26 sessions in 30-day window, 11 multi-model (42%) — per-token-type sum invariant exact to f64 precision across all sampled rows.
- 34 projects after collapse (varies; the resolver-served `/Users/Robare/...` paths fold into git toplevels; legacy `/Users/robarepruyn/...` paths from a different historical user-context fall back to raw because the path doesn't exist on disk, matching D1's graceful-fallback design).
- Top sessions attribute to the deepest work cwd (`/Users/Robare/.../Dev/tokenscale`, `LifeOps`, `platform`) rather than the home-directory git toplevel — longest-cwd-wins working as designed.
- `modelsWithoutPricing` empty on both `/usage/daily` and `/usage/sessions`.

189 workspace tests green (+6 from cwd_resolver).

### Sequencing

```
v0.1.15 (this release)
  ├─ Phase 1A — per-session reporting [landed on main 818f33a]
  ├─ Phase 1B-i — cwd → git toplevel resolution
  └─ Phase 1B-iii — daily_handler modelsWithoutPricing fix

v0.1.16 (next release)
  └─ Phase 1B-ii — parser captures gitBranch/uuid/parentUuid + UNIQUE partial index on uuid
     [forward-only schema migration; historical events stay NULL for new fields]

v0.1.17+ (future)
  └─ Phase 1.5 — tool-use / tool-result / file-history-snapshot ingest expansion
     [gates Phase 2 commit-attribution Tier 1 + Phase 3 edit-survival Tier 2]
```

Open queue past Phase 1: granular-attribution Phases 2 / 3 / 4, PUE uncertainty band, Winget manifest.

---

## v0.1.14 — 2026-05-19

The "prove the detector fires" release. v0.1.12 shipped the nightly pricing-drift-check workflow but every run since was against a known-correct `pricing.toml`, so only the exit-0 path had ever actually executed in production. A manual `workflow_dispatch` firing against a throwaway branch with deliberately-wrong rates surfaced **two real bugs** that the structural test layer alone could not have caught:

> 1. **The `pricing-divergence` label did not exist on the repo.** v0.1.12 wrote `gh issue create --label pricing-divergence …` into the workflow but the label itself was never created. `gh issue create --label` aborts hard if the label is missing. The first real drift event in production would have rung silently.
>
> 2. **The detector's `load_pricing_toml()` was broken under v0.1.13's multi-row schema.** v0.1.13 introduced `[[providers.…models."<id>"]]` (array-of-tables) and tomllib parses that as a `list` of dicts, not a single dict. The detector did `model["input_usd_per_mtok"]` directly and crashed with TypeError. The detector had been silently broken on `main` for the ~4 hours between v0.1.13 shipping and the firing exercise.

Both fixed and pinned with regression tests. The detector is now genuinely a safety net rather than a nominal one. Also closes [#2](https://github.com/RobarePruyn/tokenscale/issues/2) with three sum-invariant tests for `costUsdInput + … = costUsdTotal`, and lands the post-tag correction addendum noting that the `v0.1.13` tagged commit carries the superseded audit-scope figures (corrected on `main` in `7e230d7`).

### Added

- **`.github/scripts/tests/test_drift_detector_exit_codes.py`** — 14 end-to-end tests driving `pricing_drift_check.main()` through each non-clean exit path with monkeypatched fetch/load helpers, plus structural assertions on the workflow YAML's issue-creation step. The exit-2 test asserts full **disjointness** from the exit-1 issue path — a parse-failure run cannot leak the marker strings (`Pricing drift detected`, `INTERNAL`, `pricing.toml=$`, `Anthropic=$`, `Action: re-verify…`) that the workflow's bash heredoc would interpolate into an issue body. A false-positive drift alert is worse than a missed detection: the first cry-wolf kills trust.
- **`.github/scripts/tests/fixtures/negative_path/parse_failure_restructured.md`** — deliberately-broken page (column header renamed) for exercising the exit-2 path. Lives under `negative_path/` with a "DO NOT USE AS PRODUCTION REFERENCE" header; unreachable from the live detector path because the tests monkeypatch `fetch_live_page` directly.
- **`LoadPricingTomlSchemaCompat`** test class — end-to-end exercise of `load_pricing_toml()` against the live `pricing.toml`. Catches the exact bug the manual firing surfaced: a TOML schema change that no monkeypatched test would have seen.
- **Three sum-invariant tests in `crates/tokenscale-store/src/impact_query.rs`** ([#2](https://github.com/RobarePruyn/tokenscale/issues/2)):
  - `per_token_type_costs_sum_to_total` — single bucket, all five token types non-zero (1M of each at Sonnet 4.6 rates → $28.05 total).
  - `multi_row_per_type_sum_invariant_holds` — seven daily buckets, asserts the invariant per row.
  - `partial_cell_sums_priced_events_keeping_total_non_null` — partial pre-launch case: M-of-N events pre-date `valid_from`. Asserts `cost_usd_total` is `Some` (priced events only), `events_missing_pricing` is M, and the sum invariant holds with unpriced events contributing 0.

### Fixed (caught by the manual firing exercise)

- **`load_pricing_toml()`** now walks both single-table (v0.1.12 schema) and multi-row array-of-tables (v0.1.13 schema) forms. For multi-row entries, picks the row with the latest `valid_from` — that's the rate currently live and what Anthropic's page reflects. Historical rows are intentionally not compared against the live page. `LoadPricingTomlSchemaCompat` pins the loader against future schema breaks.
- **`pricing-drift-check.yml` workflow** runs `gh label create --force pricing-divergence …` immediately before `gh issue create`. Idempotent: creates if absent, updates description if present. The `pricing-divergence` label was also created in the repo manually so the next firing succeeds even on older workflow checkouts.

### Documentation

- **`docs/cost-methodology.md`** — appended "Post-tag correction (2026-05-19)" paragraph inside the v0.1.13 corrections-log entry, recording that the tagged commit (`611d970`) carries the superseded "four real models / 21,133 events" figures and that the correct numbers (three priced models with events, 21,077 priced events) landed on `main` in `7e230d7`. The v0.1.13 tag was deliberately not moved; this addendum makes the trail visible to anyone reading the corrections log from `main`.
- **`pricing-drift-check.yml`** has a new named CI step, `Negative-path detector tests`, running the new test suite alongside the existing `Unit tests for the parser` step.

### Manual workflow_dispatch firing (captured)

One real end-to-end firing executed by hand against a throwaway branch (`test/drift-detector-firing-v0.1.14`, since deleted) with Opus 4.7 rolled back to the v0.1.0-era $15/$75 rates. Three runs total:

1. **Run [26126818352](https://github.com/RobarePruyn/tokenscale/actions/runs/26126818352)** — exited non-zero from `gh issue create` because the `pricing-divergence` label was missing. Caught the label bug.
2. **Run [26126969964](https://github.com/RobarePruyn/tokenscale/actions/runs/26126969964)** — issue opened, but the body was a Python `TypeError` traceback. Caught the multi-row loader bug.
3. **Run [26138183080](https://github.com/RobarePruyn/tokenscale/actions/runs/26138183080)** — after both fixes, the detector produced a correct drift report with all five fields named (input / output / cache_read / cache_write_5m / cache_write_1h) and both numbers each. Dedup logic correctly appended to the existing issue rather than opening a new one.

Captured drift report from run 3 (the comment on issue #3, now closed):

```
::error::Pricing drift detected — pricing.toml is stale or inconsistent.

  · claude-opus-4-7 input: pricing.toml=$15.0, Anthropic=$5.0
  · claude-opus-4-7 output: pricing.toml=$75.0, Anthropic=$25.0
  · claude-opus-4-7 cache_read: pricing.toml=$1.5, Anthropic=$0.5
  · claude-opus-4-7 cache_write_5m: pricing.toml implies $18.75 (input $15.0 × 1.25), Anthropic publishes $6.25
  · claude-opus-4-7 cache_write_1h: pricing.toml implies $30.00 (input $15.0 × 2.0), Anthropic publishes $10.0

Action: re-verify against the source URL, update pricing.toml, recapture pricing-rate-card.snapshot.json, and tag a new release. See docs/cost-methodology.md → Drift detector for the runbook.
```

The detector is now proven to fire correctly. The bash heredoc works at runtime as the structural YAML test asserted; the runbook is preserved; the source URL interpolates correctly; dedup against an existing open issue activates as designed.

### Out of scope

- **Pre-existing lint debt** ([#1](https://github.com/RobarePruyn/tokenscale/issues/1) — 2 clippy errors on `factors.rs:309` + `pricing.rs:644`, 6 frontend `setState-in-effect` errors) is a separate cleanup tracked there, intentionally not folded into this release. v0.1.14 stays scoped to "detector negative-path proof + closing test debt."
- **Granular-attribution roadmap Phase 0** is the next workstream, after v0.1.14.

---

## v0.1.13 — 2026-05-18

The cost-side time-anchoring release. Mirrors what `environmental-factors.toml` already did per event onto `pricing.toml`: every event's cost now resolves through a per-event `valid_from` lookup, so a future rate change adds a new row dated to the announcement rather than overwriting history. Closes the v0.1.0 placeholder convention where every row carried `valid_from = "2026-04-28"` (tokenscale's own ship date, not Anthropic's launch dates) — the cost-side analog of the seed-value bug that v0.1.11 fixed on the rate side.

> **Historical cost figures DO NOT shift retroactively.** Every model's input, output, cache-read, and cache-write rates are unchanged from v0.1.12. The release moves the time-dimension, not the dollar values. Every event in this dashboard's history that was priced in v0.1.12 — with the single window-wide rate — resolves to the same dollar amount in v0.1.13 via per-event time-anchoring, because every ingested event falls on or after its model's true launch date. The maintainer's release-gate audit confirmed this: exactly zero pre-launch events across the three priced models with events in the audit DB (Opus 4.6, Opus 4.7, Sonnet 4.6 — Haiku 4.5 had no events in this DB and is absent from the audit table), covering 21,077 priced events; 56 `<synthetic>` admin-API aggregate events correctly excluded as unpriced. See [`docs/cost-methodology.md`](docs/cost-methodology.md)'s new 2026-05-18 v0.1.13 corrections-log entry for the dated audit-trail entry, including the per-model launch-date table and provenance classifications.

### Changed

- **`pricing.toml` — multi-row schema + backfilled launch dates.** Each model now lives as `[[providers.<provider>.models."<id>"]]` (array-of-tables) rather than a single `[providers.<provider>.models."<id>"]` table. v0.1.13 ships exactly one row per model — future rate corrections add a new row with a later `valid_from` rather than overwriting the existing one. Each row gains a `launch_date_source` field carrying the URL or rationale backing the date. The four `valid_from` values:
  - `claude-opus-4-7`: `2026-04-16` (sourced: Anthropic `whats-new-claude-4-7` + GitHub `changelog/2026-04-16-claude-opus-4-7-is-generally-available`).
  - `claude-opus-4-6`: `2025-09-01` (conservative estimate from Bedrock ID `anthropic.claude-opus-4-6-v1`; start-of-month per D3 conservative-dating rule).
  - `claude-sonnet-4-6`: `2025-09-01` (conservative estimate; same D3 treatment).
  - `claude-haiku-4-5`: `2025-10-01` (sourced from Bedrock model ID `claude-haiku-4-5-20251001`).
- **`pricing.toml`** also gains top-level `file_version = "1.0"` and `file_published = "2026-05-18"`, mirroring `environmental-factors.toml`. Surfaced through `/api/v1/health → pricing.file_version` / `file_published` so the dashboard banner can show "pricing v1.0".
- **DB-side per-event time-anchored pricing.** `aggregate_impact_by_bucket` joins each event to its authoritative `pricing` row via a correlated subquery on `valid_from` (identical pattern to the env-factors join already in place). Per-token-type cost SUMs (`cost_usd_input`, `cost_usd_output`, `cost_usd_cache_read`, `cost_usd_cache_write_5m`, `cost_usd_cache_write_1h`) flow through `ImpactByBucketRow` → `ModelImpact` to the frontend, where the Cost (USD) view's stack-by-token-type chart reads them directly. The pre-launch case — an event whose `occurred_at` predates every `valid_from` for its `(provider, model)` pair — resolves to `costUsdTotal = null` and renders as "—" in the dashboard, matching the env-side missingness convention for CO₂e / water. `events_missing_pricing` is published alongside `eventsMissingEnvFactor` so the dashboard footer can show "X events without pricing data."

### Removed (PUBLIC API CHANGE)

- **`pricingByModel` removed from `GET /api/v1/usage/daily`.** The window-wide per-model rate dict (`{ "claude-opus-4-7": { "input_usd_per_mtok": 5.00 }, ... }`) is gone. It was fundamentally incompatible with multi-row time-anchored pricing — rates can change mid-window, so any single value would have been a lie. The frontend's Cost (USD) view, counterfactual-cost totals, per-model cost-share legend annotations, and cache-savings stat all rewired to consume the new per-bucket-row `tokens.impact.costUsd*` fields. **External consumers of this endpoint must update**: read `tokens.impact.costUsdTotal` (or the per-token-type fields) per bucket-row instead of multiplying token counts by the window-wide rate. Cost figures are now pre-computed in SQL with per-event resolution; clients no longer need to know rates.

### Added

- **`tokenscale audit pricing-launch-dates` CLI subcommand.** Reports per-(provider, model) pre-launch counts against the on-disk `pricing.toml` after syncing it into the DB. Exits non-zero when ANY priced model has events whose date predates its earliest `valid_from` — designed to gate v0.1.13+ release tags in CI / release-checklist runs. Distinguishes the two failure modes:
  - **Priced pair with pre-launch events** — the wrong-launch-date regression. Fails the gate.
  - **Unpriced model** — pair has no `pricing.toml` row at all (e.g. the `<synthetic>` admin-API aggregate, or a new model added to events before `pricing.toml`). Reported separately, does NOT fail the gate.
- **`launch_date_source: Option<String>`** on `ModelPricing`. TOML-only provenance (not stored in the DB pricing table — same convention as `display_name`). Required for new rows going forward.
- **`/api/v1/health → pricing.file_version` / `file_published`** — both `Option<String>`, mirroring the env-side fields. Frontend `HealthResponse` type updated accordingly.

### Downgrade note

**Downgrading from v0.1.13 → v0.1.12 requires reverting `pricing.toml` to single-row form.** v0.1.13's multi-row array-of-tables schema (`[[providers.anthropic.models."claude-opus-4-7"]]`) is forward-compatible with the parser (v0.1.11+'s `PricingFile::parse` accepts both single-table and array-of-tables forms via untagged-enum deserialization), but v0.1.12's parser only accepts the single-table form. Anyone running `brew unpin && brew switch tokenscale-cli @0.1.12` after taking v0.1.13 must also restore the v0.1.12-shape `pricing.toml` from git history (or accept that v0.1.12 will fail to start with the v0.1.13-shape file). The DB-side `pricing` table is forward-only — v0.1.12 wouldn't read or write `pricing` rows at all (the table was provisioned in v0.1.0's initial migration but never populated until v0.1.13), so downgrading does not require a DB migration.

### Design decisions worth re-stating (locked in the v0.1.13 plan + signoffs)

- **Five-phase atomic ship**: Phases A (in-memory multi-row TOML), B (DB sync), C (per-event SQL aggregation), D (backfilled launch dates), E (file_version + file_published) all in one v0.1.13 tag. Splitting them would have made the "no historical numbers move" claim harder to verify across releases.
- **D3 conservative-dating rule** applied to genuine estimates only — Opus 4.6 and Sonnet 4.6 are dated to `2025-09-01` (start-of-month per D3) and explicitly labeled as estimates in `launch_date_source`. Sourced rows (Opus 4.7, Haiku 4.5) carry their exact dates; D3 does NOT apply to sourced rows.
- **D4 missingness semantic**: pre-launch events render as "—" in the dashboard, not as `$0.00`. Same convention as CO₂e / water when no env factor is available for a region.
- **D6 API change**: `pricingByModel` removal called out above as a breaking change, intentional rather than incidental. The dict's window-wide single-rate shape was structurally incompatible with the per-event time-anchored model that this release is built around.
- **D7 rollback path**: forward-only DB migrations + replace-on-startup `pricing` sync absorb data corrections (wrong launch date, rate typo) without ceremony — edit `pricing.toml`, restart, the next sync rewrites the table. A wrong launch date is recoverable with a single-line edit + a `tokenscale audit pricing-launch-dates` re-run before tagging the patch.

### Sequencing remaining

- Source first-party Anthropic launch dates for Opus 4.6 and Sonnet 4.6 (currently conservative estimates); tighten those `valid_from` dates and update `launch_date_source` in a fast-follow.
- Wire the audit subcommand into release-tag CI as a hard gate so the next pricing-data release can't ship if it would create pre-launch events.

---

## v0.1.12 — 2026-05-18

The drift-detector release. Ships the nightly CI check that compares `pricing.toml` against Anthropic's published rate card and opens a GitHub Issue when they diverge. Closes the "documented trigger with no detection mechanism" gap from v0.1.11: the next time Anthropic changes any Opus / Sonnet / Haiku per-token price, the detector flags it within 24 hours instead of letting historical numbers silently drift until a maintainer manually re-checks.

Also corrects the Haiku-percentage text in the v0.1.11 in-app notice from "about 20% understated" to **"about 25% understated"** — Haiku went $0.80 → $1.00 input and $4.00 → $5.00 output, both 25% increases. Users who upgraded to v0.1.11 in the brief window before v0.1.12 will see the corrected text on upgrade.

### Added

- **`.github/workflows/pricing-drift-check.yml`** — nightly cron (02:00 UTC) + `workflow_dispatch` for manual runs. Runs parser unit tests first, then the live check. On drift: opens a `pricing-divergence`-labelled GH Issue with the diff inline and fails the workflow.
- **`.github/scripts/pricing_drift_check.py`** — Python (stdlib only) detector. Fetches `https://platform.claude.com/docs/en/about-claude/pricing`, normalizes HTML, slices to the `Model pricing` section, validates per-row base + cache rates against `pricing.toml`, validates cache multipliers against Anthropic's prose, and runs an internal-consistency check that `cache_read_usd_per_mtok == input_usd_per_mtok × 0.1`. Exit codes — `0` clean plus three failure modes split deliberately: `1` real drift, `2` parse failure (warn-only), `3` network failure (warn-only).
- **`.github/scripts/tests/test_pricing_drift_check.py`** — 17 unit tests covering parser pinning. Explicit test cases prove the parser does NOT contaminate from Fast Mode (`$30 / $150` for Opus), Batch (`$2.50 / $12.50` for Opus), or Data Residency (1.1×) — the three highest-risk false-positive sources on Anthropic's page.
- **`pricing-rate-card.snapshot.json`** — repo-root checked-in fixture. Captured 2026-05-18 against the source URL. The detector warns when this file is older than 90 days, prompting a maintainer re-capture. Future variant can use it as an offline-fallback when the live fetch fails.
- **`docs/cost-methodology.md` → "Drift detector" section** — full design rationale, exit-code semantics, runbook for handling `pricing-divergence` issues, explicit list of what's deliberately out of V1 scope (no auto-bump of `file_status`, no dashboard surface, no new-model detection).

### Changed

- **In-app notice text and CHANGELOG v0.1.11 entry** — `"about 20% understated"` → `"about 25% understated"` for Haiku. Three locations: `crates/tokenscale-server/src/notices.rs` (the live notice constant); `CHANGELOG.md` v0.1.11 header paragraph; `CHANGELOG.md` v0.1.11 effect-summary line. The cost-methodology corrections-log table already said 25%, no change needed there.

### Design decisions worth re-stating (all locked in v0.1.11 task 5 sign-off)

- **Source URL**: `https://platform.claude.com/docs/en/about-claude/pricing`. Pinned to the dedicated developer reference page, not the marketing landing page (`claude.com/pricing#api`) and not the model overview (`models/overview`).
- **Where it runs**: CI nightly cron only. Never `tokenscale serve` startup — local-first architecture, no privacy surface to Anthropic's web property on every user start.
- **Failure response**: drift opens a GH Issue + fails workflow (red X). Parse / network failures stay green-with-warning so a page restructure doesn't generate false-positive drift alerts.
- **No auto-bump** of `pricing.toml` `file_status` on drift. Auto-bumping would brick every running instance the moment Anthropic touched a price.
- **Cache validation**: `cache_read_usd_per_mtok` (absolute in `pricing.toml`) compared directly against Anthropic's column. Cache writes (multipliers) checked via `input × multiplier == Anthropic's absolute column`.
- **Parser pinned to base rates** — the load-bearing guard against the Fast Mode false-positive. The fixture-based tests prove the pinning works.

### Sequencing remaining

- **v0.1.13**: 6b cost-side time-anchoring. Promotes `valid_from` on `pricing.toml` rows to actually drive per-event lookup; mirrors the environmental-factor path. The structural fix the detector is the alarm for.
- Then: PUE uncertainty band, Winget manifest, granular-attribution roadmap Phase 0.

---

## v0.1.11 — 2026-05-18

The pricing-correctness release. **Three of four model rows in `pricing.toml` carried wrong API rates from v0.1.0 through v0.1.10** — Opus 4.7 and 4.6 were 3× overstated, Haiku 4.5 was ~25% understated. Root cause was a seed comment ("pricing assumed unchanged from Opus 4 family") plus a `file_status = "needs_review"` that was never flipped. v0.1.11 corrects the rates, removes the bug pattern, and adds a production-gate that refuses to start with unverified data so this class of bug cannot ship again.

> **This is a data correction, not a methodology change.** Historical Opus-attributed counterfactual cost and "Estimated savings vs raw API rates" figures **drop by roughly 3× on upgrade**. Haiku-attributed cost rises ~25%. Sonnet figures are unchanged. The full dated correction entry lives in [`docs/cost-methodology.md`](docs/cost-methodology.md)'s new "Corrections log" section.

### Changed (the actual fix)

- **`pricing.toml`** — Opus 4.7 and 4.6 corrected from $15/$75 to **$5/$25** input/output per MTok. Haiku 4.5 corrected from $0.80/$4.00 to **$1.00/$5.00**. Sonnet 4.6 unchanged ($3/$15). Cache-read rates (stored as absolute USD/MTok) recomputed alongside: Opus → $0.50, Haiku → $0.10. Cache-write multipliers (stored as multipliers of input) unchanged. Every row's seed-marker `notes` field removed. The file's header comment block rewritten from "needs review" to "production rates verified 2026-05-18." `file_status` flipped to `"production"`.

### Added (defensive infrastructure so this can't happen again)

- **`PricingFile::has_seed_markers()`** in `tokenscale-core`. Scans every row's `notes` for phrase-level markers (`"seed value"`, `"unverified"`, `"needs_review"`, `"assumed unchanged"`) that uniquely identify the v0.1.0–v0.1.10 bug pattern. Phrase-level not word-level — `"medium response assumed at 1,500-2,000 tokens"` in legitimate methodology prose does NOT trip the gate.
- **`EnvironmentalFactorsFile::has_seed_markers()`** — same gate for env factors. Symmetric enforcement.
- **CLI startup gate** (`command_serve`) — replaces the existing `warn!` log lines with `anyhow::bail!`. The server refuses to start if either file has `file_status != "production"` OR `has_seed_markers() == true`. v0.1.0–v0.1.10 was a soft warning; v0.1.11 is a hard error. No bypass flag — flip `file_status` in your local fork if you need to dev against unverified data.
- **Unit tests** pinning the production gate against the real repo `pricing.toml` (`the_real_repo_pricing_file_passes_production_gate`) and against the literal v0.1.0–v0.1.10 buggy notes phrasing (`has_seed_markers_detects_each_phrase`). `cargo test` fails before the binary builds if either condition reappears in the repo.

### Audit trail

- **`docs/cost-methodology.md`** — new "Corrections log" section dating this correction. Append-only, parallel to `docs/research-log.md` on the environmental side. Carries: what was wrong, the corrected rates, root cause, effect on historical figures, deferral of the effective-date question to 6b, and a list of the defensive changes shipped alongside.
- **`docs/request-for-research.md`** 6b entry gains an "Effective-date guidance for the v0.1.11 corrections" subsection. When time-anchoring lands, the corrected rates must be dated to each model's actual launch (Opus 4.7 early 2026, Opus 4.6 Sep 2025, Haiku 4.5 Oct 2025), NOT to v0.1.11's release date. Calling out the failure mode where a future contributor stamps every row with today's date and recreates the same bug at a smaller scale.

### Added (in-app notice)

- **`/api/v1/notices` + `/api/v1/notices/{id}/dismiss`** endpoints. Server-side notice catalog (compile-time constants in `crates/tokenscale-server/src/notices.rs`) with active-filter logic (not-dismissed AND not-past-expiry). Persistent dismissal state in `<config-dir>/dismissed-notices.toml`.
- **`ReleaseNoticeBanner`** in the frontend — amber strip rendering server-active notices at the top of the dashboard, above the existing environmental + pricing banners. Optimistic dismiss (banner disappears on click immediately; POST to record dismissal happens in the background).
- **First notice: `pricing-correction-v0.1.11`** with the wording you signed off on, 90-day expiry (active 2026-05-18 → 2026-08-16), linking to the cost-methodology corrections log. Always-show (any user on v0.1.11 sees it once), simplified render condition decoupled from the v0.1.0 pricing-review banner's dismissal state.

### NOT in this release (deferred to v0.1.12)

- **Drift detector** — nightly CI workflow that compares `pricing.toml` against Anthropic's published rate card. The detector's first finding (this bug) made it clear that the correctness fix should ship first as its own coherent release. v0.1.12 will land the detector with the three additions you specified: cache-rate coverage via input-multiple validation, parser pinned to base-rate columns (not Fast Mode / Batch / US 1.1x multiplier), and source URL targeting Anthropic's dedicated pricing page.

### Sequencing

- **v0.1.11** (this release): correction + audit trail + production gate + in-app notice.
- **v0.1.12**: drift detector.
- **v0.1.13**: 6b cost-side time-anchoring (the core fix that makes pricing changes audited and historical numbers stable).

---

## v0.1.10 — 2026-05-16

The dashboard-credibility release. Folds in the full meta-review pass against the dashboard UX and the cost-side methodology gap. No methodology numbers change; every value visible to the user becomes more honestly *displayed* (precision matched to uncertainty, brackets making the band the primary cue, framing that doesn't oversell) and the cost side gets its first proper paper trail.

### Changed (dashboard precision + framing)

- **False precision killed across the board.** New `roundToSigFigs(value, sigFigs)` + `sigFigsForUncertainty(pct)` helpers route the environmental formatters through precision-matched-to-band rounding. `499.92 kWh ± 40%` becomes `~500 kWh`, `134.98 kg CO₂e ± 43%` becomes `~130 kg CO₂e`, `74.99 L ± 64%` becomes `~75 L`. Mapping: <5% → 4 sig figs, 5–15% → 3, ≥15% → 2.
- **± badges now show the value bracket.** All three environmental KPIs render as `~rounded (low – high) ± pct%`. Bracket recomputes from the post-rounding value each render, so the indirect-water toggle correctly shows the new bracket when flipped.
- **Counterfactual cost drops cents** via `formatRoundedDollars` — `(approx)` and ".94 cents" were saying the same imprecision twice. Subscription / other-charges cards keep cents (exact from CSV imports).
- **"Net value" → "Estimated savings vs raw API rates"**, with a caveat directly under the headline (`Assumes API list rates, no volume or enterprise discounts.`) — green-tinted positive number no longer reads as guaranteed savings.

### Added (information that should have been visible)

- **Cache-accounting strip** under the financial row. Shows `cache_read / (input + cache_read + cache_write_5m + cache_write_1h)` percentage AND the estimated dollar savings (`cache_read × 0.9 × input_rate`, summed across models). Cache writes are deliberately in the denominator — they're paid-for cache that isn't (yet) amortizing, and including them surfaces a low-amortization pattern as a low percentage. The single most informative top-line metric for Claude Code users.
- **Equation strip** below the financial cards: `$X (counterfactual) − $Y (subs) − $Z (other) = $W (savings)`. Annotates the cards with the underlying arithmetic.
- **Consumer-apps disclaimer lifted ABOVE the environmental cards** (was buried in fine print below the chart). Honest framing comes BEFORE the kWh number, not after.
- **Last-scanned timestamp on the dashboard** (next to the chart title), so users don't have to click into Methodology to check data freshness.
- **Per-option tooltips on the Counting toggle** (Raw / Cost-weighted / Cost (USD)). Hover each for the precise definition.
- **Family-aware chart colors.** Models in the same family (Opus 4.6 + Opus 4.7) now get adjacent shades of one hue; different families get different hues. Opus = blue shades, Sonnet = green shades, Haiku = amber shades.
- **Per-series cost-share % in the chart legend**, regardless of view mode. Legend shows `Sonnet 4.6 — 72% of cost` even when the chart is in Raw view, so users see relative cost contribution at a glance.

### Removed

- **Provider dropdown hidden.** "All providers" implied multi-provider data when only Claude Code is ingested today; the consumer-apps disclaimer already states scope. Re-introduce when a second provider lands; the `providerFilter` state is preserved (hard-set to `'all'`) so the diff to revive it is trivial.

### Added (cost-side credibility)

- **`docs/cost-methodology.md`** — short companion to the environmental methodology page. Documents the four load-bearing cost-side assumptions: (1) API list rates only, (2) current pricing applied retroactively to all historical events (not time-anchored — load-bearing call-out), (3) flat-daily subscription pro-rating, (4) cache reads billed at 10% of input.
- **New "Cost" tab** in the dashboard's Methodology page rendering the above doc. Sits alongside Environmental / Bibliography / Research log / Open questions.
- **"How is this computed? (4 assumptions)" disclosure** under the financial row. Inline summary of the four assumptions + link to the full doc on GitHub and to the in-app Cost tab. Brand-protection placement: visible on the dashboard, not buried two clicks away.

### Research

- **"Cost-side time-anchoring + Cost Methodology audit trail"** added as an open item in [`request-for-research.md`](docs/request-for-research.md), with a **hard trigger**: must land before the next time Anthropic changes any model's per-token price. Today, `PricingFile::lookup()` ignores `valid_from` and applies the current rate to every historical event. The moment a price changes, every historical counterfactual silently shifts. The lightweight doc + disclosure in v0.1.10 is a stopgap; the time-anchored lookup is the real fix.

### Roadmap

- **`docs/roadmap-granular-attribution.md`** — captured granular-attribution roadmap prompt (per-project / per-thread reporting → commit-attribution Tiers 1/2/3 → forward instrumentation via post-commit hook). Queued explicitly after this meta-review release + the existing Open queue. First action when picked up: Phase 0 investigation (dedup correctness, ingest coverage, project-key resolution, cwd → git toplevel).

### Notably NOT in this release

- **Cost-side time-anchoring itself** — see the open research entry above. v0.1.10 documents the gap; v0.1.x closes it.
- **Granular attribution** — captured in the roadmap, sequenced after the existing Open queue (PUE / Winget / 6b cost-side / All-view clip).

---

## v0.1.9 — 2026-05-16

The macOS notarization release. Every macOS binary the release pipeline produces is now **signed with the Developer ID Application cert and notarized by Apple** — first launch on a user's Mac no longer triggers the "cannot verify developer" Gatekeeper block dialog, and the verified-developer fingerprint matches across both apple-silicon and intel builds.

> **v0.1.8 was never released** — the initial tag landed without `allow-dirty = ["ci"]` in dist-workspace.toml, so dist's plan-job CI-drift check rejected the manual notarization edit before any artifacts could be built. Fixed in v0.1.9. Skipped tag retained for audit clarity.

### Added

- **"Sign + notarize macOS binaries" step** in `.github/workflows/release.yml` `build-local-artifacts` job. Runs only on `apple-darwin` matrix entries. Decodes six Apple/notarization secrets, imports the Developer ID Application cert into a temporary keychain, signs the `tokenscale` binary with `--options runtime --timestamp`, submits to Apple's notarization service via `xcrun notarytool submit --wait`, repacks the tarball with the signed binary, recomputes the `.sha256` sidecar, and patches `dist-manifest.json` so the Homebrew formula publish job picks up the post-notarization checksum.
- **`RELEASING.md` walkthrough** for retrieving and configuring the six secrets: `APPLE_TEAM_ID`, `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`, `APP_STORE_CONNECT_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_PRIVATE_KEY`.
- **CUSTOMIZATIONS section in `dist-workspace.toml`** inventorying every manual edit to the dist-generated release.yml — currently just the notarization step. Re-running `dist generate --mode=ci` will wipe the step; the inventory tells future-me what to re-apply.

### Why this is a manual workflow edit

Cargo-dist 0.31 doesn't have first-class macOS notarization support — [open feature request](https://github.com/axodotdev/cargo-dist/issues/1121). Rather than fork dist or wait, we splice the sign+notarize cycle into the generated workflow as a self-contained step between `dist build` and the artifact upload. When dist gains native support, the manual step gets dropped and the customization inventory becomes empty again.

### What stapling does and does not do

The notarization ticket lives on Apple's servers — `xcrun stapler` can only attach it to `.app` / `.dmg` / `.pkg` bundles, not bare Mach-O binaries. So tokenscale's CLI doesn't carry an embedded ticket. **First-launch on a user's Mac with network access**: Gatekeeper fetches the ticket from Apple, sees "Verified Developer: Robare Pruyn", and launches. **First-launch offline**: Gatekeeper falls back to checking the code signature only, which still works because we sign with `--timestamp`. Subsequent launches don't re-check.

If we ever need stapled binaries (truly offline first-launch for air-gapped environments), the path is to wrap the binary in a signed-and-notarized `.pkg` installer — a separate distribution channel alongside the existing tarballs.

---

## v0.1.7 — 2026-05-16

The full-water-footprint release. Ships **indirect (off-site, power-plant cooling) water** alongside the existing direct-DC-cooling water — closing the open research item from Sweep #2 and giving users the complete Ren et al. 2024 scope-1 + scope-2 water picture. Opt-in via a new toggle to preserve continuity; existing dashboards default to direct-only.

### Added

- **`indirect_water_l_per_kwh`** field per `[grid_factors.*]` block in `environmental-factors.toml` v0.3, with matching `indirect_water_uncertainty_range_pct`. Per-subregion values:
  - SRVC (us-east-1): **2.39 L/kWh** ±35% — direct quote from Ren et al. 2024 Table 1 Virginia row
  - RFCW (us-east-2): **1.85 L/kWh** ±35% — computed from eGRID 2023 fuel mix × Macknick 2012 recirculating-tower coefficients
  - NWPP (us-west-2): **9.50 L/kWh** ±60% — direct quote from Ren et al. 2024 Washington row (hydro-dominated; wider band reflects reservoir-evaporation methodology dispute)
  - CAMX (reference): **3.20 L/kWh** ±50% — computed via fuel mix × Macknick
- **"Include indirect water" toggle** on the dashboard's Water KPI. Defaults OFF; when checked, the Water stat card switches to "Water (direct + indirect)" and shows the combined total with sum-quadrature uncertainty. Tooltip surfaces the breakdown ("direct X L + indirect Y L per Ren et al. 2024").
- **Indirect water row in the FactorProvenancePanel** alongside the existing direct-water row, with its own ± band and source link.
- **`combineSumUncertaintyPct` helper** for fractional-uncertainty-of-a-sum math. Direct and indirect water are independent (different physical systems — DC cooling vs power-plant cooling), so quadrature of absolute σ is the right combination rule.
- **Migration `20260516000001_grid_factor_indirect_water.sql`** — additive `indirect_water_l_per_kwh` and `indirect_water_uncertainty_range_pct` columns on `grid_factors`. Promotes Sweep #2's per-region values from in-memory snapshot into the schema so per-event compute can carry them through the aggregate path natively.

### Changed

- **`GridFactors` struct, `FactorsProvenance`, and `EnvironmentalImpact`** in `tokenscale-core` gain indirect-water fields. `ModelImpact` payload (`/api/v1/usage/daily`) carries `indirectWaterL` and `indirectWaterUncertaintyPct` alongside the existing `waterL` / `waterUncertaintyPct`.
- **`aggregate_impact_by_bucket` SQL** projects `indirect_water_l_raw` and `grid_indirect_water_uncertainty_pct` per (bucket, model) cell. Same time-anchoring as every other grid factor.
- **`/api/v1/factors/active`** `GridFactorEntry` gains `indirect_water_l_per_kwh`, `indirect_water_uncertainty_range_pct`, and `source_url_indirect_water` so the provenance panel can render the new row.

### Research

- **Sweep #2: indirect water** logged in [`docs/research-log.md`](docs/research-log.md) with full methodology, source corpus (Ren et al. 2024, Macknick 2012 NREL, eGRID 2023 fuel-mix tables), and the per-region computation showing why hydro-heavy NWPP carries the widest band. "Indirect water (power-plant cooling) methodology" moved Open → Resolved in [`docs/request-for-research.md`](docs/request-for-research.md).
- Methodology page section "Water — direct vs indirect" rewritten to describe both scopes, the toggle UX, and the unresolved hydro-attribution dispute.

### Why default to OFF

For SRVC, turning on indirect water jumps reported water from 0.057 L to ~1 L per typical session (a ~17× change). For NWPP it's ~64×. Flipping the default would re-baseline every existing dashboard overnight; opt-in lets users discover the magnitude on their terms and read the methodology before committing. The toggle copy and tooltip educate. A future release may flip the default once users have had a chance to internalize the numbers.

### Notably NOT in this release

- **Hydro attribution refinement.** Macknick's 100%-to-power attribution is contested; literature gives 5×–10× range. We use as-published with widened bands for hydro-heavy regions. A future sweep could refine.
- **Recycled-water credits.** Some AWS datacenters (e.g., Loudoun County) use recycled wastewater; not yet modeled separately from fresh-water draw.

---

## v0.1.6 — 2026-05-15

The per-day-rate chart release. The dashboard's main chart now plots a **per-day rate** instead of per-bucket sums, eliminating the bucket-size-driven peak jumps that made the 1y and All views look like usage suddenly inflated 7× / 30× compared to 30d / 90d. The visual discontinuity at every preset transition is gone, and the chart finally communicates intensity-over-time honestly regardless of zoom.

### Changed

- **Chart y-axis is now a per-day rate.** Each bucket's value is divided by the number of window-days it covers (1 for daily, 7 for full weekly buckets, 28–31 for monthly, and partial counts for the first/last bucket when the user's window clips the bucket calendar — e.g. mid-May yields 15 days for the in-progress May bucket, not 31). Cumulative totals in the stat cards (energy, CO₂e, water, counterfactual cost) are unchanged — only the chart shape is normalized.
- **Chart title reflects the rate semantics**: "Token usage per day · daily" / "Token usage per day · weekly average" / "Token usage per day · monthly average". The cadence suffix tells the user how much each data point is smoothed, since bucket size no longer drives peak height but still drives visible variance.
- **Tooltip values now carry a `/day` suffix** ("1.2B tokens/day", "$45.67/day"). Removes ambiguity at hover.

### Why this matters

A chart titled "Weekly token usage" with bars that are 7× taller than the daily equivalent was visually lying: peak height read as "intensity" but a 7-day sum is mechanically bigger for the same usage rate. The v0.1.1 fix patched 30d ↔ 90d by keeping them both daily; this release fixes the same problem at every preset transition by changing what the chart actually measures. The full audit:

| view | granularity | bucket | prior peak | per-day rate |
|---|---|---|---|---|
| 90d  | daily   | 1 day  | ~1B    | ~1B/day   |
| 1y   | weekly  | 7 days | ~2.8B  | ~400M/day |
| All  | monthly | ~30 d  | ~6B    | ~400M/day |

Same data, comparable peaks across all three views.

### Notably NOT in this release

- **Auto-clipping the All-view to first-event date.** The 2022-12-01 lower bound is intentional (ChatGPT launch — "earliest possible LLM usage"). With rate normalization, the empty leading space no longer distorts the y-axis, so the visual cost is much smaller; we keep the honest absolute timeline.

---

## v0.1.5 — 2026-05-15

The run-it-as-a-service release. No code changes — purely installer- and docs-side polish, but a meaningful UX win: post-install messages now tell every brew / Scoop user exactly how to start the dashboard, and `brew services start tokenscale-cli` works out of the box for set-and-forget background operation.

### Added

- **`def caveats` + `service do` blocks** on the Homebrew formula. After `brew install tokenscale-cli`, users see how to run the dashboard (`tokenscale serve`) or register it as a managed background service (`brew services start tokenscale-cli`) right in the install output. The service block declares `keep_alive true`, so the dashboard auto-restarts on crash and auto-starts on login. Logs land in `$(brew --prefix)/var/log/tokenscale.log`.
- **`notes` block on the Scoop manifest** — same UX on Windows. Post-install message covers `tokenscale serve` and the NSSM recipe for running as a Windows service.
- **README "Running as a service" section** — covers macOS (`brew services`), Linux (`systemd` user unit, with a ready-to-paste unit file), and Windows (NSSM). All three platforms get a recipe that takes less than a minute to set up.

### How it's wired

dist 0.31's Homebrew installer doesn't expose hooks for `caveats` or `service` — they're not in `HomebrewInstallerLayer`. To work around that without losing dist's zero-touch publishing, the Homebrew tap repo (`RobarePruyn/homebrew-tokenscale`) gets its own GitHub Actions workflow (`.github/workflows/amend-formula.yml`) that fires whenever dist commits a new formula version, splices the two blocks in before the closing `end` of the class, and commits the amended formula. Sentinel markers bracket the injected region for idempotent strip-and-replace on re-runs.

### Migration note for existing v0.1.4 users

The formula amendment was applied to the live v0.1.4 formula before this tag, so the simplest path is:

```bash
brew update && brew reinstall tokenscale-cli
brew services start tokenscale-cli
```

(`brew upgrade` would no-op since the binary is unchanged; `reinstall` is what picks up the new caveats + service block.) Then open `http://127.0.0.1:8787` and the dashboard's running without a terminal open.

---

## v0.1.4 — 2026-05-15

The honest-headline release. Combines model and grid uncertainty into a single per-metric `± X%` badge on every environmental KPI — the v0.4 follow-on v0.1.3 explicitly deferred — and propagates grid uncertainty all the way through the DB compute path so per-event impact carries it natively rather than as a render-time afterthought.

### Added

- **Combined `± %` badges in the environmental stat row.** Each KPI now shows its own quadrature-combined uncertainty:
  - **Energy (facility)**: model-factor band only (PUE folds in — no separate band tracked).
  - **CO₂e**: √(model² + grid_co2e²) — e.g. Sonnet 4.6 (±35%) on SRVC (±15% CO₂e) → **±38%**.
  - **Water**: √(model² + grid_water²) — e.g. Sonnet 4.6 (±35%) on any AWS region (±50% water) → **±61%**.
- **`tokenscale_core::combine_uncertainty_pct(model, grid)`** — public quadrature helper used by both the per-event compute path (`compute_impact`) and the bucket-aggregate SQL path (`aggregate_impact_by_bucket::cook`). Single source of truth for the math, so the two redundant compute paths stay in lockstep.
- **`co2eUncertaintyPct`** and **`waterUncertaintyPct`** fields on the daily-endpoint `ModelImpact` payload, alongside the existing `maxUncertaintyPct` (now energy-only). All three are per-(bucket, model) cells; the frontend reduces to max across visible cells for the headline badges.
- **Migration `20260515000001_grid_factor_uncertainty.sql`** — additive `co2e_uncertainty_range_pct` and `water_uncertainty_range_pct` columns on the `grid_factors` table. Both nullable, populated on next factor-file sync. v0.1.3 ran the in-memory snapshot only; v0.1.4 promotes uncertainty into the schema so it can flow through per-event compute.

### Changed

- **`FactorsProvenance`** gains `grid_co2e_uncertainty_pct` and `grid_water_uncertainty_pct` fields for symmetry with `model_factor_uncertainty_pct`. Per-event audit trail now carries every band that fed the combined badge.
- **`aggregate_impact_by_bucket` SQL** projects `MAX(gf.co2e_uncertainty_range_pct)` and `MAX(gf.water_uncertainty_range_pct)` alongside the existing `MAX(ef.uncertainty_range_pct)`. Quadrature runs in Rust (`RawImpactByBucketRow::cook`) so the SQL stays portable.
- **`factors_sync`** propagates the two new uncertainty fields from `environmental-factors.toml` into `grid_factors` on every startup. **`factors_lookup`** reads them back into the canonical `GridFactors` struct so single-row lookups carry them too.

### Research

- **Training-cost amortization** added as an aspirational open question in [`docs/request-for-research.md`](docs/request-for-research.md). Full lifecycle (training + inference) accounting per the Strubell 2019 / Patterson 2021 / Luccioni 2022 framing. Gated on Anthropic publishing Claude training compute or a defensible third-party estimate; the numerator-scope question (final run vs full envelope) is called out explicitly.

### Notably NOT in this release

- **Per-region WUE values from AWS.** Water bands stay at ±50% across all AWS regions until AWS publishes per-region WUE; tracked separately in `request-for-research.md`.
- **PUE uncertainty as a separate band.** Currently folded into the model uncertainty; honest improvement would carry it explicitly through the quadrature.

---

## v0.1.3 — 2026-05-12

Two feature additions + the first quarterly research sweep, all driven by closing the loop on items the v0.1.2 docs flagged as future work.

### Added

- **Per-value factor provenance panel** (methodology page v0.2). A new **"Sources for these numbers"** disclosure below the environmental stat row. When expanded, shows three sections:
  - Methodology identifier + link to the source paper + factor file version.
  - Per-model factor rows for the **models visible in the current chart** — display name, confidence tag, model uncertainty ±%, valid_from, source_doc, expandable notes.
  - The configured region's grid factor row — eGRID subregion, CO₂e/water/PUE values with inline uncertainty bands, link to EPA source.
- **`GET /api/v1/factors/active` endpoint** — reads the in-memory factor-file snapshot and serves every (provider, model) row + every grid row + the configured region + methodology metadata. ~5KB payload, fetched once on mount.
- **Scoop bucket** at [RobarePruyn/scoop-tokenscale](https://github.com/RobarePruyn/scoop-tokenscale) — Windows users can now `scoop bucket add tokenscale https://github.com/RobarePruyn/scoop-tokenscale && scoop install tokenscale`. Hand-maintained because `dist` v0.31 doesn't have native Scoop support yet, but uses Scoop's `autoupdate` block tied to GitHub Releases — `scoop update` propagates new tokenscale releases with zero maintainer action per release.

### Research

- **Sweep #1: grid-factor uncertainty bands** (first quarterly sweep, see [`docs/research-cadence.md`](docs/research-cadence.md)). Pulled 4 years of EPA eGRID CO₂e data (2019/2020/2022/2023) for the four subregions tokenscale tracks, computed YoY variance + std dev, and established honest ± bands per subregion. New `co2e_uncertainty_range_pct` field on each `[grid_factors.*]` block in `environmental-factors.toml`:
  - SRVC: **±15%** · RFCW: **±20%** · NWPP: **±20%** · CAMX: **±20%**
- New `water_uncertainty_range_pct = 50` across all AWS regions, honestly reflecting the gap between AWS's global WUE and any specific datacenter's real water draw (AWS publishes no per-region WUE).
- Factor file `file_version` bumped from `0.1` to `0.2`. Full audit trail in [`docs/research-log.md`](docs/research-log.md).
- "Grid-factor uncertainty bands" moved from Open → Resolved in [`docs/request-for-research.md`](docs/request-for-research.md).

### Changed

- `GridFactors` struct in `tokenscale-core::factors` gains `co2e_uncertainty_range_pct` and `water_uncertainty_range_pct` optional fields. Schema is back-compat — `schema_version` stays at 1.
- `GET /api/v1/factors/active` response includes the new uncertainty fields per grid row; FactorProvenancePanel displays them inline next to the CO₂e and water values.

### Notably NOT in this release

- **Aggregate stat-row `± X%` badge still reflects only model uncertainty.** Combining model + grid uncertainty into the headline badge is a deliberate v0.4 follow-on; v0.3 keeps the decomposition visible (model bands in the stat row, grid bands in the sources panel) so users can see both before we collapse them.
- **Per-event compute math does NOT use grid uncertainty.** The DB schema for `grid_factors` doesn't carry the uncertainty fields yet; per-event impact stays exact (point-estimate) compute. Display-only for now.

---

## v0.1.2 — 2026-05-11

The credibility-deepening release. Ships the **methodology / transparency page** the CHARTER named as required-not-optional from day one — every number on the dashboard now has a one-click trail to its source, methodology, and derivation.

### Added

- **`docs/methodology.md`** — narrative walkthrough of how every environmental number gets computed. Covers Google's comprehensive methodology adoption, per-token energy math, PUE/CO₂e/water formulas, the eGRID subregion vs state distinction, uncertainty surfacing, structurally-invisible consumer surfaces, and how factor refreshes propagate.
- **Methodology page** in the dashboard (`Methodology` nav button in the header). Four tabs:
  - **Methodology** — the new narrative doc.
  - **Bibliography** — renders `docs/sources.md`. Every factor source with confidence tag, access date, and summary.
  - **Research log** — renders `docs/research-log.md`. Audit trail of past sweeps.
  - **Open questions** — renders `docs/request-for-research.md`. What the next sweep should address.
- **`GET /api/v1/docs/<slug>`** endpoint that serves the four bundled markdown docs. Bundled via `include_str!` at compile time so the page works offline. Slug-restricted to the user-facing docs (other repo docs stay out of the API surface).
- **`react-markdown` + `remark-gfm`** for rendering. Hand-rolled `.prose-tokenscale` CSS for typography consistent with the rest of the dashboard.

### Changed

- Header gains a Dashboard / Methodology nav switcher. Pricing-review banner now suppresses on the methodology view (it's dashboard-contextual).

---

## v0.1.1 — 2026-05-11

Polish release on top of v0.1.0. No new features; quality-of-life fixes around the dashboard's wider-window views and the install/setup path now that pre-built binaries are the recommended way in.

### Added

- **`docs/research-cadence.md`** — documents how the environmental-factor model gets refreshed (quarterly default + ad-hoc triggers, what a sweep produces, how it distributes to users, the maintainer review checklist). Names the discipline that keeps factor data credible over time.
- **`docs/request-for-research.md`** — open questions for the next research sweep: grid-factor uncertainty bands, indirect-water methodology, tokenizer-change inflation factor verification, non-US eGRID coverage, methodology-choice re-verification.
- **`CHANGELOG.md`** (this file).

### Changed

- **README leads with `brew install`** and the other pre-built installers; build-from-source flow moved to a "Building from source" section for contributors. Reflects that the project actually ships as a binary now.
- **Auto-granularity thresholds retuned** from 60/365 to 120/730 days. The 90d view now stays on daily buckets (previously crossed into weekly), making 30d → 90d a visually continuous extension rather than a sudden 7× peak jump from bucket-size change.
- **Chart title reflects actual granularity** — "Weekly token usage" / "Monthly cost (USD)" / etc. — instead of always saying "Daily" regardless of bucket size.
- **Subscriptions panel** — divider line above the "Imported subscription charges" section is suppressed when there are no manual entries to separate from. Small visual cleanup.

### Fixed

- **`~/` paths in `claude_code_roots` now expand to the user's home directory.** Configs following the README example (`["~/.claude/projects", "~/.claude-synced/laptop/projects"]`) previously failed silently — TOML stored the paths verbatim, the walker tried to open a directory literally named `~`, and the scan skipped both roots without surfacing an error in the UI. The fix is a small `expand_tilde` helper applied to every configured root.

### Release pipeline

- First release fully validated through the pipeline. `dist` v0.31.0 with the GitHub-build-setup hook that runs `npm ci && npm run build` in the `frontend/` directory before `cargo build`. Homebrew tap auto-published to `RobarePruyn/homebrew-tokenscale`. Two CI bugs surfaced and fixed during v0.1.0 (npm working-directory key was stripped by dist's inlining, and the tap repo needed an initial commit before the publish job could checkout main); both fixed inline.

---

## v0.1.0 — 2026-05-11

Initial public release. The bones of the project are in place: ingest Claude Code session logs, compute environmental impact against versioned factor data, surface everything in a local-first dashboard, and ship as a one-command install on five platforms.

### Added

- **Phase 1: dashboard core**.
  - Claude Code JSONL ingest crate (`tokenscale-ingest-cc`) reading `~/.claude/projects/<project>/<session>.jsonl` with idempotent delta scans keyed by `(mtime_ns, len)` per file and request-id / content-hash dedup at the events table.
  - SQLite schema with versioned migrations, `sources`-table-keyed multi-provider design.
  - axum HTTP server with embedded React + Tailwind + Recharts SPA via `rust-embed`. Endpoints: `/api/v1/health`, `/usage/daily`, `/usage/by-model`, `/projects`, `/sessions/recent`, `/subscriptions`, `/billing/charges`.
  - Dashboard with filters (provider, models, token types, projects, date range), view modes (Raw / Cost-weighted / Cost (USD)), stacked-by-model and stacked-by-token-type chart variants, three KPI stat cards.
  - Subscription tracking — manually-declared subscriptions with date-range pro-rating, "Net value" calculation against counterfactual API cost.
- **Phase 2: environmental impact compute path**.
  - `environmental-factors.toml` v0.1 production file with 17 (provider, model) tuples across Anthropic / Google / OpenAI / DeepSeek / Meta / Mistral plus 3 AWS region grid factors anchored to EPA eGRID2023.
  - Google "comprehensive methodology" (Elsworth et al. 2025, arXiv:2508.15734) as the canonical compute path: per-token energy × PUE → facility energy → CO₂e and water as facility-level multipliers.
  - Per-event, time-anchored factor resolution — each event resolves to the env_factors row whose `valid_from <= occurred_at`. Honest sums across `valid_from` boundaries inside a bucket.
  - `[inference]` config block with `default_inference_region` for grid attribution. Back-compat reads the legacy top-level field with a deprecation warning.
  - Environmental impact block on every daily-endpoint response — energy / facility energy / CO₂e / water plus provenance counters (events using fallback PUE, events missing env_factor). Stat row + environmental banner in the dashboard.
- **Phase 2: billing data ingest via Stripe CSV import**.
  - `billing_charges` table with line-item-level granularity, `(source, external_id)` dedup, idempotent re-imports.
  - Two-step preview/commit endpoints. Preview shows parsed rows with auto-categorization and detects conflicts against manually-declared subscriptions. Commit accepts re-categorized rows + a list of manual subscription IDs to dismiss in the same transaction (avoids double-counting when CSV subscriptions duplicate manual entries).
  - Dashboard import panel — drop a CSV or paste, preview with inline category overrides + conflict resolution, commit.
- **Multi-machine ingest** — `[ingest].claude_code_roots` accepts a list of paths so multiple synced JSONL directories scan in one pass. README documents Syncthing as the recommended cross-platform sync tool.
- **Auto-scan in `serve`** — background tokio task re-runs scan every `scan_interval_seconds` (default 60), so the dashboard refreshes as Claude Code activity lands without manual `tokenscale scan`.
- **Distribution pipeline** — `dist` v0.31.0 generates prebuilt binaries for macOS (Apple Silicon + Intel), Linux (x86_64 + aarch64), and Windows (x86_64) on every tagged release. Three installer types: `curl | sh`, PowerShell, and a Homebrew formula auto-published to a separate tap repo. Full release process documented in `RELEASING.md`.

### Documentation

- `CHARTER.md` — project scope, governance, distribution model, framework-extractable design lens.
- `docs/architecture.md` — high-level system design.
- `docs/decisions.md` — running ADR log; 12 entries covering language choices, schema decisions, ingest model, sqlx offline mode, factor model design, time-anchoring SQL, per-event resolution, `[inference]` config block, `view=` parameter drop, multi-machine sync via Syncthing, manual CSV import as default billing path, and the renamed `cargo-dist` → `dist`.
- `docs/sources.md` — bibliography of every factor source with confidence tags and access dates.
- `docs/research-log.md` — audit trail of the v0.1 factor sweep.
- `docs/data-sources.md` — per-ingest-source documentation; honest acknowledgment of which surfaces are structurally invisible to any external tool (iOS, Android, desktop apps, `claude.ai` web).
- `README.md` with quick-start, install commands, dashboard tour, configuration reference, and multi-machine setup walkthrough.

### Known limitations

- Anthropic Admin API ingest exists in design but isn't built; requires an organization account, which most individual-tier users don't have.
- Consumer chat surfaces (Claude iOS / Android / desktop apps, `claude.ai` web) have no per-user usage feed and are structurally invisible to the dashboard's token-tracking view. Their subscription cost IS captured via billing imports.
- Linux distro packaging (APT / DNF / RPM), Scoop bucket, Winget manifest, and macOS notarization are not yet automated. Prebuilt `.tar.xz` archives serve those platforms in the interim.
