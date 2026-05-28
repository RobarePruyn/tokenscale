# Roadmap — Phase 1.5: tool-use ingest expansion

**Status**: scoping. No implementation in this pass.
**Date**: 2026-05-24
**Sequenced after**: Phase 1 (v0.1.15 user-visible + v0.1.16 ingest layer, both shipped).
**Sequenced before**: Phase 2 (Tier 1 commit attribution) and Phase 3 (Tier 2 edit-survival), both blocked on this expansion per Phase 0.

This is the largest schema-touching release since v0.1.13 (multi-row pricing + per-event SQL aggregation). The discipline forged across v0.1.13–v0.1.16 — forward-only, smoke-test-against-real-DB-as-gate, empirical-before-speculative — applies here in full.

---

## 1. JSONL structure: what's actually in there

Empirical pass over the maintainer's `~/.claude/projects/`, 2026-05-24: **44 files, 65,190 non-empty lines, 0 malformed.**

### Line-type distribution

| `type` | Count | % of total | Currently ingested? |
|---|---|---|---|
| `assistant` | 29,535 | 45.3% | yes (parsed for tokens + v0.1.16 uuid/parentUuid/gitBranch) |
| `user` | 19,370 | 29.7% | no |
| `last-prompt` | 4,883 | 7.5% | no |
| `file-history-snapshot` | 3,049 | 4.7% | no |
| `ai-title` | 2,817 | 4.3% | no |
| `queue-operation` | 2,794 | 4.3% | no |
| `attachment` | 2,569 | 3.9% | no |
| `pr-link` | 105 | 0.2% | no |
| `system` | 68 | 0.1% | no |

Phase 1.5 ingests the three load-bearing categories: `tool_use` blocks inside `assistant`, `tool_result` blocks inside `user`, and `file-history-snapshot` rows. Everything else stays dropped (no Phase 2/3 dependency on it).

### Tool-use frequency, per assistant message

29,535 assistant messages total:

- **11,370 (38.5%)** have zero `tool_use` blocks (pure-text responses).
- **18,165 (61.5%)** have at least one.
- mean: 0.62 per message, **p50: 1, p99: 1, max: 3.**
- **18,167 total `tool_use` blocks** across the dataset.

Implication: most tool-use rows will land in a steady ratio of ~0.6:1 against assistant rows. Storage is small (~62% of assistant message volume); index size is comparable.

### Tool-name distribution

18 distinct tool names. Top 5 cover 95.9%:

| Tool | Count | % of tool_use |
|---|---|---|
| `Bash` | 7,136 | 39.3% |
| `Edit` | 4,767 | 26.2% |
| `Read` | 2,710 | 14.9% |
| `Write` | 1,606 | 8.8% |
| `TodoWrite` | 1,209 | 6.7% |
| `WebSearch` | 267 | 1.5% |
| `WebFetch` | 143 | 0.8% |
| `Grep` | 117 | 0.6% |
| `ToolSearch` | 73 | 0.4% |
| 9 more tools | combined | 0.7% |

Phase 2 (Tier 1 commit attribution) needs `Bash` exclusively; Phase 3 (Tier 2 edit-survival) needs `Edit` + `Write`. These three tools alone cover **74.3%** of all tool calls. The schema should NOT special-case them; store all 18+ uniformly.

### tool_result blocks + linkage

- 19,370 user lines → **18,162 `tool_result` blocks** across 18,162 user messages (i.e., user messages with a tool_result carry exactly one).
- **Linkage: 18,162 of 18,167 `tool_use` ids have a matching `tool_result` (100.0%).** The 5 orphans are presumably interrupted sessions (the model emitted a `tool_use` but the session ended before the tool ran or the result was recorded).
- **Zero `tool_result` ids without a matching `tool_use`.** Every `tool_result` resolves to its parent.

The 100% linkage rate is load-bearing for D3 (linkage resolution strategy).

### Sample `tool_use` record

```json
{
  "type": "tool_use",
  "id": "toolu_01Nzkb1Vt8fuWWatogQ2w8EP",
  "name": "Bash",
  "input": {
    "command": "ls -la \"…\"",
    "description": "Inspect website root and platform frontend directory"
  },
  "caller": { "type": "direct" }
}
```

Fields: `id` (Anthropic tool-use UUID, distinct from CC's per-message `uuid`), `name` (tool kind), `input` (tool-specific JSON), `caller` (rarely interesting). Inline inside the assistant message's `content` array.

### Sample `tool_result` record

```json
{
  "tool_use_id": "toolu_01Nzkb1Vt8fuWWatogQ2w8EP",
  "type": "tool_result",
  "content": "total 232\ndrwx------  26 robare  staff    832 May  2 17:02 .\n…"
}
```

Linkage via `tool_use_id` (string match against the `tool_use.id`). `content` is the raw output (Bash stdout, file content for Read, etc.) — can be large, especially for Bash with verbose output.

### Sample `file-history-snapshot` record

Two forms observed. Across 3,069 file-history-snapshot lines in 44 maintainer JSONL files, 3,045 (99.2%) are non-empty and 24 (0.8%) are empty. The rare empty case is handled by the parser but is not the common path.

**Empty** (0.8% of observed):
```json
{
  "type": "file-history-snapshot",
  "messageId": "2632706f-…",
  "snapshot": {
    "messageId": "2632706f-…",
    "trackedFileBackups": {},
    "timestamp": "2026-05-05T15:33:06.338Z"
  },
  "isSnapshotUpdate": false
}
```

**Non-empty** (99.2% of observed, emitted after a file edit):
```json
{
  "type": "file-history-snapshot",
  "messageId": "9735af40-…",
  "snapshot": {
    "messageId": "af0d685b-…",
    "trackedFileBackups": {
      ".github/workflows/sync-api-docs.yml": {
        "backupFileName": null,
        "version": 1,
        "backupTime": "2026-05-05T18:10:17.658Z"
      }
    },
    "timestamp": "2026-05-05T17:41:01.494Z"
  },
  "isSnapshotUpdate": true
}
```

Fields: `messageId` on the outer record points to the assistant turn that triggered the snapshot. `snapshot.messageId` (often different) points to the previous snapshot — chain reconstruction for `isSnapshotUpdate=true` records. `trackedFileBackups` is the actual edit data: file path → `{backupFileName, version, backupTime}`. **No content hash; no diff.** Just "this file was touched at this time, version N."

### What's NOT in this expansion

`last-prompt`, `ai-title`, `queue-operation`, `attachment`, `pr-link`, `system` lines stay dropped. None gates any Phase 2/3 query. If a future workstream needs them, that's a separate scoping pass; out of scope for 1.5.

---

## 2. Schema design

### Options

**Option A — separate tables (`tool_uses`, `tool_results`, `file_snapshots`).** One table per concern, foreign-keyed back to `events.uuid` via a new `parent_event_uuid` column.

Schema sketch:

```sql
CREATE TABLE tool_uses (
    id                  INTEGER PRIMARY KEY,
    tool_use_id         TEXT NOT NULL,           -- toolu_… Anthropic id
    parent_event_uuid   TEXT NOT NULL,           -- events.uuid of the assistant turn
    source              TEXT NOT NULL,           -- 'claude_code' for now
    tool_name           TEXT NOT NULL,           -- 'Bash', 'Edit', …
    input_json          TEXT NOT NULL,           -- the full input JSON, verbatim
    occurred_at         TEXT NOT NULL,           -- inherited from parent event timestamp
    session_id          TEXT,                    -- denormalized for cheap session queries
    project_id          TEXT,                    -- denormalized for cheap project queries
    UNIQUE (source, tool_use_id)                 -- dedup, mirrors v0.1.16 (source, uuid)
);
CREATE INDEX tool_uses_parent_event_uuid_idx ON tool_uses (parent_event_uuid);
CREATE INDEX tool_uses_session_id_idx ON tool_uses (session_id);
CREATE INDEX tool_uses_project_id_tool_name_idx ON tool_uses (project_id, tool_name);

CREATE TABLE tool_results (
    id                  INTEGER PRIMARY KEY,
    tool_use_id         TEXT NOT NULL,           -- joins to tool_uses.tool_use_id
    parent_event_uuid   TEXT NOT NULL,           -- the USER event uuid that contained this result
    source              TEXT NOT NULL,
    content             TEXT NOT NULL,           -- raw tool output (Bash stdout, file content, …)
    occurred_at         TEXT NOT NULL,
    session_id          TEXT,
    UNIQUE (source, tool_use_id)                 -- one result per tool_use_id (per the 100% linkage)
);
CREATE INDEX tool_results_session_id_idx ON tool_results (session_id);

CREATE TABLE file_snapshots (
    id                  INTEGER PRIMARY KEY,
    snapshot_message_id TEXT NOT NULL,           -- outer messageId (joins to events.uuid)
    source              TEXT NOT NULL,
    file_path           TEXT NOT NULL,           -- key in trackedFileBackups
    backup_file_name    TEXT,                    -- nullable per data
    version             INTEGER NOT NULL,
    backup_time         TEXT NOT NULL,
    is_snapshot_update  INTEGER NOT NULL,        -- 0/1; boolean
    session_id          TEXT,
    project_id          TEXT,
    UNIQUE (source, snapshot_message_id, file_path)
);
CREATE INDEX file_snapshots_session_id_idx ON file_snapshots (session_id);
CREATE INDEX file_snapshots_project_id_idx ON file_snapshots (project_id);
```

Pros: normalized, queryable. Phase 2 query "find all Bash `git commit` calls in this session" becomes `SELECT … FROM tool_uses WHERE tool_name = 'Bash' AND session_id = ? AND input_json LIKE '%git commit%'`. Indexes match the query patterns. Each table can be extended independently (e.g., add a `cwd` column to `tool_uses` later without touching the other two).

Cons: three migrations (one per table); three INSERT paths in `insert_events` (or a separate `insert_tool_data`).

**Option B — single `tool_events` table with `kind` enum.** One table covering tool-use, tool-result, and file-snapshot rows distinguished by a `kind TEXT` column.

```sql
CREATE TABLE tool_events (
    id                  INTEGER PRIMARY KEY,
    kind                TEXT NOT NULL,           -- 'tool_use' | 'tool_result' | 'file_snapshot'
    tool_use_id         TEXT,                    -- non-NULL for kinds 'tool_use' and 'tool_result'
    parent_event_uuid   TEXT NOT NULL,
    source              TEXT NOT NULL,
    tool_name           TEXT,                    -- non-NULL for 'tool_use'
    file_path           TEXT,                    -- non-NULL for 'file_snapshot'
    payload_json        TEXT,                    -- catch-all (input for tool_use, content for tool_result, …)
    occurred_at         TEXT NOT NULL,
    session_id          TEXT,
    project_id          TEXT
);
```

Pros: one migration, one INSERT path. Less code.

Cons: sparse columns (a `file_snapshot` row's `tool_name` is NULL; a `tool_result` row's `file_path` is NULL). Queries become noisier (`WHERE kind = 'tool_use' AND tool_name = 'Bash'`). Adding a fourth kind later means widening this table; risks the "one table per concern" reasoning being repeatedly violated.

**Option C — JSON blob on `events`.** Single new column `tool_data TEXT` on `events`. Smallest schema change.

```sql
ALTER TABLE events ADD COLUMN tool_data TEXT;  -- JSON: [{tool_use…}, {tool_use…}, …]
```

Pros: minimal change. No new tables. Downgrade trivial.

Cons: queries that need to filter by tool name or correlate tool_use with tool_result require SQLite JSON1 functions (slow on large datasets), can't index into JSON cleanly, breaks Phase 2 + Phase 3 query primitive design entirely. Also: `tool_result` blocks live on USER lines, not assistant lines — they'd need a separate column anyway, breaking the "one column" simplicity claim immediately.

### Recommendation: Option A (separate tables)

Three concrete reasons:

1. **Phase 2 and Phase 3 query primitives map cleanly to separate tables.** Phase 2's `list_session_commits` becomes `SELECT … FROM tool_uses WHERE session_id = ? AND tool_name = 'Bash' AND input_json LIKE '%git commit%'` plus a JOIN to `tool_results` for the SHA capture. Phase 3's `list_session_edits` becomes `SELECT … FROM tool_uses WHERE session_id = ? AND tool_name IN ('Edit', 'Write')` plus a JOIN to `file_snapshots` for survival measurement. Both queries are simple, indexable, and don't pay a sparse-column tax.
2. **Phase 1.5 ingest is one expansion, but the three concepts have different lifecycles.** `tool_uses` are emitted by the model and never modified after ingest. `tool_results` arrive ~one assistant turn later (when the tool runs). `file_snapshots` arrive at file-edit time. Modeling them in one table conflates the three lifecycles; modeling separately keeps each lifecycle visible in the schema.
3. **Migration risk is the same in practice.** Three additive `CREATE TABLE` statements in one migration file are no riskier than one `CREATE TABLE` — neither modifies existing data, neither blocks. The "three tables = three risks" framing breaks down once you note the three tables don't interact at the SQL level except through joins (which are read-only).

Down-side acknowledged: Option A's parser change has more INSERTs to plumb. Mitigated by the parser fan-out decision (D2 below): a single `parse_line` extension returns a struct with three Vec fields, and `insert_events` (or a sibling `insert_tool_data`) walks all three.

---

## 3. Parser changes

### Current shape

`crates/tokenscale-ingest-cc/src/parser.rs::parse_line` returns `ParseOutcome`:

```rust
enum ParseOutcome {
    Skip,
    Event(Box<Event>),
    Malformed { reason: String },
}
```

One JSONL line → at most one `Event`. Tool-use blocks inside `message.content` are dropped (Phase 0 finding).

### Option A — one-to-many at parse: `ParseOutcome::Event` carries auxiliary records

`ParseOutcome::Event` extends from `Box<Event>` to a struct that carries `Event` + `Vec<ToolUse>` + `Vec<ToolResult>` + `Vec<FileSnapshot>` (any of which may be empty depending on line type). Same pure-function shape: input is one JSONL line, output is one structured outcome.

```rust
struct ParsedRecords {
    event: Box<Event>,
    tool_uses: Vec<ToolUse>,        // populated when assistant line has tool_use blocks
    tool_results: Vec<ToolResult>,  // populated when user line has tool_result blocks (NEW: user lines now parse)
    file_snapshots: Vec<FileSnapshot>, // populated for file-history-snapshot lines
}

enum ParseOutcome {
    Skip,
    Records(ParsedRecords),
    Malformed { reason: String },
}
```

Caller (the scan loop) unpacks and routes to the appropriate insert path.

### Option B — auxiliary pass: parse twice

First pass: existing `parse_line` for assistant Events. Second pass: separate `parse_tool_blocks` over the same JSONL string. Wasteful — every line parsed as JSON twice.

### Recommendation: Option A (one-to-many at parse)

JSON parsing is the cost; auxiliary records are nearly free once you have the parsed AST. Option B is a non-starter on perf grounds.

One subtlety: **user lines now need to parse.** Currently `JsonlLine::Other` catches every non-assistant line via serde's `#[serde(other)]` and the parser returns `Skip`. Phase 1.5 needs to extract `tool_result` blocks from user lines AND parse `file-history-snapshot` lines. The `JsonlLine` enum extends to recognize those line types explicitly.

User lines that have no `tool_result` (text-only user messages — the maintainer's data shows 1,208 of 19,370 user lines, ~6%) still emit `ParseOutcome::Skip` since there's nothing to ingest.

### Tool_use_id linkage: D3 — parse-time vs query-time

Two ways to handle `tool_result.tool_use_id` references:

**Parse-time resolution**: when parsing a `tool_result`, look up the matching `tool_use` and resolve the reference (e.g., to the tool_use's primary-key id). Requires shared state across lines in a session (since tool_use precedes its tool_result by some number of lines).

**Query-time resolution**: store the `tool_use_id` as a string on the `tool_results` row. SQL JOIN at query time: `JOIN tool_uses ON tool_uses.tool_use_id = tool_results.tool_use_id AND tool_uses.session_id = tool_results.session_id`.

**Recommendation: query-time (D3 below).** Three reasons:

1. **100% linkage** in observed data means the JOIN is reliable; pre-resolution buys nothing.
2. **Parser stays pure.** Per-line parsing has no cross-line state. Maintainability and testability both benefit.
3. **Rebuild safety.** Phase 1.5's `--rebuild` path re-ingests JSONL from scratch (§5). If we resolved at parse time, a tool_result parsed BEFORE its matching tool_use was re-ingested would fail to resolve. Query-time avoids the temporal-coupling problem entirely.

### Interrupted-session handling

5 of 18,167 `tool_use` ids have no matching `tool_result` (interrupted sessions). The parser handles this gracefully: the `tool_use` row lands; the `tool_result` row never lands. Queries that JOIN see the orphan `tool_use` rows with a NULL result; Phase 2/3 query primitives must handle that. Not a parser concern.

---

## 4. Storage layer changes

### Aggregation paths — explicitly NOT affected

Critical: **tool_uses, tool_results, file_snapshots do NOT participate in cost or environmental aggregation.** The token-cost-and-impact math is owned by `events`; tool-use rows are metadata that explains *why* an assistant turn produced its tokens, not additional billable events.

`aggregate_impact_by_bucket` keeps reading only `events`. The Phase 1.5 release-gate smoke (§7) confirms this with a direct numerical comparison: same window, same dashboard output, before and after the migration.

### New query primitives Phase 2/3 will need

```rust
// Phase 2 (Tier 1 commit attribution) — list Bash `git commit` calls in a session.
pub async fn list_session_bash_calls(
    db: &Database,
    session_id: &str,
) -> Result<Vec<SessionBashCallRow>>;

// SessionBashCallRow fields: tool_use_id, command (parsed from input_json),
// result_content (joined from tool_results), occurred_at, parent_event_uuid.
// Phase 2 will filter result_content for SHA-shaped output to capture
// commit hashes.

// Phase 3 (Tier 2 edit-survival) — list files Edit/Write'd in a session.
pub async fn list_session_file_edits(
    db: &Database,
    session_id: &str,
) -> Result<Vec<SessionFileEditRow>>;

// SessionFileEditRow fields: tool_use_id, tool_name (Edit|Write),
// file_path (parsed from input_json), occurred_at. Joins to
// file_snapshots optionally for the snapshot reference.
```

Both are simple `SELECT … JOIN … WHERE session_id = ? AND tool_name = …` queries. Indexes per §2 (`tool_uses_session_id_idx`, `file_snapshots_session_id_idx`) cover them. Phase 1.5 ships these as **stubs only** — they exist so Phase 2/3 can build against a known signature, but the in-Phase-1.5 release doesn't surface them in any handler or dashboard.

### New insert path

A sibling to `insert_events`: `insert_tool_data(db, &[ToolUse], &[ToolResult], &[FileSnapshot])` that bulk-INSERTs into the three new tables, each inside the same transaction as the parent `insert_events` call. Atomicity matters: a partial ingest where the assistant Event landed but its tool_uses didn't would leave a permanently broken linkage. Single transaction.

### Orphan-count baseline as a tracked invariant (Addition 1)

The 100% empirical linkage rate is what makes D3's query-time JOIN reliable. To keep that assumption observable rather than implicit, `ScanSummary` gains a quietly-tracked field:

```rust
pub tool_use_orphans: usize,  // tool_use rows whose tool_use_id has no matching tool_result
```

Counted at the end of each scan via a SQL EXCEPT or NOT EXISTS query against the freshly-inserted source. Maintainer's current baseline is **5** (interrupted sessions, expected). Not a release gate; not a warning at steady-state. Just an observable number alongside the existing `events_inserted` / `uuid_duplicates_skipped` fields.

The first sign of upstream schema drift surfaces here cheaply: if a future ingest reports 50 orphans, something changed about how CC writes JSONL or how Tokenscale ingests it.

---

## 5. Forward-only + rebuild path

Same discipline as v0.1.16.

**Forward-only**: Historical events (every event ingested before the Phase 1.5 release tag, presumably v0.1.17) have no tool-use data. Their `tool_uses`, `tool_results`, `file_snapshots` rows simply do not exist. Queries that JOIN over these tables for a pre-1.5 session return empty results — Phase 2 attribution for that session is "no data," not "incorrect data." Same posture as v0.1.16's NULL-uuid handling.

**Rebuild path**: `tokenscale scan --rebuild` wipes events + file_state for the source and re-parses every JSONL. Post-1.5, the rebuild path re-ingests with the new parser and populates all four tables (events + the three new ones).

### Rebuild linkage correctness

If a user runs `--rebuild` after 1.5 lands, the rebuild re-parses JSONL sequentially per file. Tool_use rows land first (assistant lines come earlier in the JSONL than their tool_result user lines), then tool_result rows. Query-time linkage (D3) means correctness depends on both rows existing at query time, not at parse time — so within a single rebuild transaction the linkage resolves correctly regardless of file ordering.

The one edge case to verify: a `tool_use` and its `tool_result` split across two JSONL files (e.g., a long session compacted across files). Empirical check: zero such cases in the maintainer's 44 files (every linkage resolved within the same file). Phase 1.5's rebuild can assume single-file linkage; if a future CC version starts splitting, the query-time JOIN still works as long as both rows land — it doesn't care which file each came from.

### Stated in three places

Per v0.1.16 discipline:

1. This scoping doc (§5).
2. Phase 1.5 migration file header comment.
3. Phase 1.5 CHANGELOG "Forward-only" subsection.

So a future maintainer doesn't mistake the empty-table-for-historical-sessions for a missing backfill.

---

## 6. Detector implications

Same grep discipline as v0.1.16:

```
grep -rn "tool_use\|tool_result\|file_snapshot\|tool_event" .github/scripts/
grep -rn "events\." .github/scripts/
```

Phase 1.5 build pass must run this grep and confirm **0 hits.** The drift detector reads `pricing.toml` and the snapshot fixture; it does not touch any events/tool/snapshot table. No detector changes expected. If a hit ever appears, surface it and either rewrite or document the dependency.

---

## 7. Release gate: smoke-test against real DB

State this as a **release-blocking gate**, not as a quality nice-to-have.

The pattern across v0.1.13 → v0.1.16: every schema-touching release in this arc surfaced a real bug during smoke that no test caught.

- **v0.1.13** — `daily_handler` `modelsWithoutPricing` conflation (caught while building 1A; would have shipped to brew users).
- **v0.1.14** — `load_pricing_toml` four-hour silent failure under multi-row schema + missing `pricing-divergence` label (both caught during the manual `workflow_dispatch` firing).
- **v0.1.15** — `MIN(project_id)` attributing all sessions to home dir (caught during 1B smoke); MAX's `/private/` ASCII edge case (caught when MIN-fix surfaced it).
- **v0.1.16** — pre-check counting every rescan as a uuid collision (caught when scan tests broke after the pre-check landed).

That's 5 distinct real bugs across 4 schema-touching releases, each surfaced by smoke against actual data and none by unit tests. **The smoke gate is not optional; the bug-find is the expected outcome.**

### Specific Phase 1.5 smoke gate

Before tagging the release:

1. **Migration applies cleanly against fresh DB** (in-memory tests cover this with every test run).
2. **Migration applies cleanly against v0.1.16-shape DB** — smoke against the maintainer's actual production DB.
3. **`tokenscale scan` against the real JSONL surface lands order-of-magnitude-correct row counts:**
   - ~18,000 tool_use rows (per §1's 18,167 measured)
   - ~18,000 tool_result rows
   - ~3,000 file_snapshot rows
   - **0 orphan tool_results** (every tool_result links to a tool_use within the same source)
4. **Regression check**: `aggregate_impact_by_bucket` returns identical numbers before and after the migration, for the same window. Run pre-1.5 binary against the same DB, capture; run post-1.5 binary, capture; diff. **Any divergence is a release blocker.** This is the "tool tables do not leak into existing aggregation" guarantee made concrete.
5. **No existing test failed silently due to a schema-touching refactor.** Run the full workspace test suite; assert that the count of passing tests is ≥ v0.1.16's count + new tests, AND no test was inadvertently `#[ignore]`-d or deleted during the build pass.
6. **Find the bug.** The historical pattern says one exists. Don't tag until either it's found and fixed/accepted, OR the maintainer signs off that the bug-hunt was thorough enough.

The release gate is satisfied when all six items are confirmed in the v0.1.17 commit description AND smoke output is captured in the release notes.

### §7 framing carries forward (Addition 3)

The release-gate paragraph above — "smoke-bug-surfacing is the expected outcome, and the release is not tagged until either the bug is found and fixed/accepted or the hunt is documented as exhaustive" — is the most important sentence in this doc. It survives **into Phase 2 and Phase 3 scoping verbatim**, not reset to "the test suite is comprehensive enough."

Phase 2 (Tier 1 commit attribution) and Phase 3 (Tier 2 edit-survival) both touch ingest and query paths. The historical pattern argues those will surface real bugs too. The §7 framing carries forward as a template paragraph in those future scoping docs.

---

## 8. CHANGELOG framing

Phase 1.5 ships ingest plumbing only. No dashboard surfaces tool-use data in v0.1.17. The CHANGELOG must read accordingly:

> **Phase 1.5 — tool-use ingest expansion.** Companion to v0.1.16's parser captures, extending the ingest layer to cover `tool_use` blocks (Bash, Edit, Write, etc.), `tool_result` blocks (linked back to their parent tool calls), and `file-history-snapshot` records. **No new dashboard features.** This release gates Phase 2 (Tier 1 commit attribution: which commits did CC author?) and Phase 3 (Tier 2 edit-survival: how much CC-authored code is still in the repo?).
>
> Three new tables: `tool_uses`, `tool_results`, `file_snapshots`. All linked to `events` via existing `uuid` (v0.1.16+). **Forward-only**: historical events have no tool-use data. Run `tokenscale scan --rebuild` if you want a backfill.
>
> Token / cost / environmental aggregations are unchanged. The new tables hold metadata about what assistant turns *did* with their tokens, not new billable events.

Then a "What this enables" block similar to v0.1.16's, listing Phase 2 + Phase 3 + future audit value.

---

## 9. Phasing within 1.5

### Options

**Single tag (v0.1.17)**: tool_uses + tool_results + file_snapshots all in one migration, one parser pass, one CHANGELOG.

**Split**:
- **1.5-a** → tool_use ingest (Bash, Edit, Write, ...). Most commonly hit; biggest data volume.
- **1.5-b** → tool_result ingest. Adds the joined-row linkage.
- **1.5-c** → file-history-snapshot ingest. Independent of the other two.

### Recommendation: single tag (v0.1.17)

The v0.1.13 splitting argument — "isolate risk classes" — doesn't apply cleanly here because **the three concepts are interlocked at the data-model level**:

- `tool_results` is meaningless without `tool_uses` (no parent to JOIN against).
- `file_snapshots` references `events.uuid` (same as tool_uses), with a similar lifecycle — file-touch events arrive at the same model-turn boundary.
- A user reading "v0.1.17 ships tool_use ingest" reasonably assumes tool_result lands too; surprising them with v0.1.18 a week later for the dependent piece would feel arbitrary.

Phase 1B-i / 1B-ii split worked because the two were genuinely separable concerns (query layer vs. ingest layer). The three sub-concepts within 1.5 are NOT separable — they're the same expansion.

**Mitigation**: the v0.1.13 splitting argument was about *risk* isolation, not concept isolation. To address the risk, lean harder on §7's smoke-test gate rather than on splitting. Add to the gate: per-table smoke verification (each of the three tables has rows; row counts are order-of-magnitude correct; linkage queries return non-empty results).

### Escape-hatch trigger (Addition 2 — sharper than "if scope balloons")

If, **during scoping or implementation of `file_snapshots`**, the work develops a complication that materially changes the release shape — specifically:

- A separate migration concern (e.g., file_snapshots needs its own schema iteration distinct from tool_uses / tool_results), OR
- A non-trivial parser branch beyond the straight-forward field extraction documented in §1, OR
- Any work that adds **more than half a day** to the critical path of the rest of v0.1.17

…then `file_snapshots` carves out as v0.1.18 with its own scoping pass. What's built for tool_uses + tool_results tags as v0.1.17 on its own.

The pre-commitment is what makes the split safe. "I'll decide if it balloons" reliably ships the bloated version. The trigger is decided pre-build, not made under release pressure at the end. File snapshots are the cleanest split-point because they don't reference `tool_use_id` (independent JOIN graph from the other two tables).

---

## Design decisions to sign off

### D1 — Schema grain (§2)

| Option | Pros | Cons |
|---|---|---|
| **A. Separate tables (`tool_uses`, `tool_results`, `file_snapshots`)** *(recommended)* | Maps to Phase 2/3 query primitives cleanly; lifecycles visible in schema; indexable; future-proof | 3 tables to migrate; 3 INSERT paths to plumb |
| B. Single `tool_events` with `kind` enum | One migration, one INSERT | Sparse columns; noisier queries; risks repeated violation later |
| C. JSON blob on `events` | Minimal schema change | Breaks Phase 2/3 query design; tool_result lives on user lines anyway |

**Recommendation: A.** Query primitives for the downstream phases are the load-bearing concern; A maps cleanly. The "extra migrations" cost is one-time at build, not ongoing.

### D2 — Parser fan-out (§3)

| Option | Pros | Cons |
|---|---|---|
| **A. One-to-many at parse (`ParseOutcome::Records {event, tool_uses, …}`)** *(recommended)* | Single pass; pure function; auxiliary records nearly free | `ParseOutcome` enum changes signature; all call sites update |
| B. Auxiliary pass over the same JSONL | No `ParseOutcome` change | Parses every line twice; wasteful |

**Recommendation: A.** Option B is a non-starter on perf grounds. The `ParseOutcome` signature change ripples to ~3 call sites (`scan.rs` + 2 tests); trivial.

### D3 — tool_use ↔ tool_result linkage (§3)

| Option | Pros | Cons |
|---|---|---|
| Parse-time resolution | Eager linkage in the parser | Requires cross-line state; breaks rebuild ordering; brittle on interrupted sessions |
| **Query-time resolution (store `tool_use_id` as string; JOIN at query)** *(recommended)* | Parser stays pure; rebuild-safe; handles orphans gracefully | Slightly more SQL in Phase 2/3 query primitives |

**Recommendation: query-time.** 100% empirical linkage means the JOIN is cheap and reliable. Parse-time buys nothing and breaks rebuild ordering.

### D4 — Phasing (§9)

| Option | Pros | Cons |
|---|---|---|
| **Single tag v0.1.17** *(recommended)* | Cohesive — the three concepts ARE one expansion at the data-model level | Larger migration than v0.1.16; concentrated risk |
| Split into 1.5-a / 1.5-b / 1.5-c | Smaller per-release risk | Artificial transitional states (tool_result with no tool_use, etc.); reader expectations broken |

**Recommendation: single tag.** Risk mitigation lives in §7's smoke gate, not in splitting. Escape hatch: if file_snapshots adds disproportionate scope during build, carve it out as v0.1.18 (it's the most independent of the three).

---

### Test fixtures live in files, not strings (Addition 4)

Tool-use blocks have meaningful inner structure: Bash with its command, Edit with file path and diff, Write with full content. String-literal test fixtures will get tedious quickly and obscure what is being tested. **Plan**: commit a small representative-sample JSONL file at `crates/tokenscale-ingest-cc/tests/fixtures/sample-session.jsonl` early in the build pass and reference it across tests.

Required fixture coverage:

- At least one of each top-five tool (Bash, Edit, Read, Write, TodoWrite).
- At least one interrupted session producing an **orphan** `tool_use` (no matching `tool_result`).
- At least one `file-history-snapshot` record with non-empty `trackedFileBackups`.
- At least one `tool_use` → `tool_result` linkage spanning **multiple lines** (real flow: tool_use on assistant line N, tool_result on user line N+1+).
- At least one assistant message with **multiple** tool_use blocks (the max=3 case from §1).

Fixture is small, hand-authored, and lives in version control. Tests reference it via `include_str!` or `std::fs::read_to_string` (latter for the live-file shape the parser already uses).

## Output expected from sign-off

After D1–D4 are approved:

1. Phase 1.5 implementation lands as v0.1.17: one migration, parser fan-out, three new tables + indexes, four new tests minimum (parser captures each line type, INSERT atomicity, linkage JOIN, aggregation regression).
2. Smoke against maintainer's real DB per §7 — the bug WILL be there; finding it is the gate, not the surprise.
3. Phase 2 scoping pass follows once v0.1.17 is on `main`.

---

## Sequencing summary

```
v0.1.15  (released 2026-05-22) — Granular Attribution Phase 1, user-visible layer
v0.1.16  (released 2026-05-24) — Granular Attribution Phase 1, ingest layer

v0.1.17  (this scoping doc, next release)
  └─ Phase 1.5 — tool-use / tool-result / file-history-snapshot ingest expansion

v0.1.18+ (future)
  └─ Phase 2 — Tier 1 commit attribution (Bash `git commit` SHA capture; exact-but-partial)
  └─ Phase 3 — Tier 2 edit-survival heuristic (Edit/Write paths via file_snapshots; heuristic + bands)
  └─ Phase 4 — Tier 3 forward instrumentation (post-commit hook with Tokenscale-Session trailer; exact for new commits)
```

No code until D1–D4 are signed off.
