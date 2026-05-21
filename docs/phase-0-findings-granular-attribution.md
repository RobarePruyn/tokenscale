# Phase 0 findings — granular attribution roadmap

**Date**: 2026-05-21
**Scope**: investigation only, no code changes per [`roadmap-granular-attribution.md`](roadmap-granular-attribution.md) → Phase 0 ("Output: a findings doc, no code").
**Method**: read of `crates/tokenscale-ingest-cc/` + `crates/tokenscale-store/src/queries.rs` + one empirical pass over the maintainer's `~/.claude/projects/` JSONLs (48 files, 27,388 assistant lines).

---

## TL;DR

| Phase 0 question | Answer | Implication |
|---|---|---|
| 1. Does the parser dedupe by message UUID before summing usage? | **No, but the current data exposes no UUID-duplication and the existing `requestId` + `content_hash` dedupe is sufficient empirically.** | No correctness bug today. Latent risk if a future CC behavior change introduces same-UUID-different-`requestId` duplicates. |
| 2. Are `cwd`, `gitBranch`, `sessionId` ingested? Are tool-use events ingested? | `cwd` ✓ (stored as `project_id`). `sessionId` ✓. **`gitBranch` is dropped** (present in JSONL, not deserialized). **Tool-use events are NOT ingested** — only `assistant` lines are parsed, the `Other` enum variant catches everything else. | Phase 1 (per-project / per-thread reporting) works on what's already ingested. **Phase 2 and Phase 3 need an ingest expansion as a prerequisite** to capture Bash, Edit, Write tool calls + file-history-snapshot records. |
| 3. What does the dashboard's "67 projects" filter key on? | Raw `cwd` string from the JSONL. `list_projects_with_totals` does `GROUP BY events.project_id` and `project_id` is set directly from `assistant.cwd` at parse time. | A single repo accessed from multiple working directories (worktrees, sub-directory `cd`s) **fragments across rows**. The user-visible "67 projects" number overcounts. |
| 4. Is `cwd` resolved to a git toplevel anywhere? | **No.** Stored raw, used raw. The walker comment explicitly says "the actual cwd lands in each event via the `cwd` field on the JSONL line itself." | Q3 above. Resolving at ingest is the right Phase 1 fix. |

**Does anything jump the queue as a correctness issue?** No. The roadmap's load-bearing concern (UUID dedup) does not match this data. Q3's project fragmentation is a polish issue, not a correctness one — the per-(time, model) numbers the dashboard reports are correct; only the per-project rollup is fragmented.

---

## Q1 — Dedup behavior

### What the code does

[`crates/tokenscale-ingest-cc/src/parser.rs`](../crates/tokenscale-ingest-cc/src/parser.rs):

- `AssistantPayload` deserializes `requestId`, `sessionId`, `cwd`, `message`, `timestamp`.
- **The JSONL `uuid` field is not deserialized.** It is dropped on the floor at parse time.
- Dedup key is `requestId` when present; `content_hash` (SHA-256 over `timestamp|model|tokens|sessionId|cwd`) fallback when absent (~error lines).

### What the roadmap expected

> "CC sometimes writes the same message (same UUID) into multiple JSONL files during branching or resume. Summing usage without deduplicating by UUID inflates totals."

### Empirical answer against current data

Counted across the maintainer's 48 JSONL files (27,388 assistant lines):

- Total assistant lines: 27,388
- Lines carrying a `uuid`: 27,388 (100%)
- Distinct `uuid` values: 27,388
- **UUIDs appearing in >1 file: 0**
- **UUIDs with >1 distinct `requestId`: 0**

Either the duplicate-UUID-across-files pattern doesn't happen in this user's CC usage (no heavy resume / branching), or recent CC versions don't write that way. Either way, there is no current bug to fix.

### Latent risk

The parser would fail to dedupe if any of these conditions arose:

1. Same `uuid`, same `requestId`, different file: **already deduped** by `requestId` (Layer 2 dedup is `UNIQUE (source, request_id)` per `crates/tokenscale-store/src/events.rs`). ✓
2. Same `uuid`, **different `requestId`** (e.g., a session resume that re-issued the API call with a new request_id): the existing dedupe would let both through, inflating totals. ✗ (Not observed in current data, but possible in principle.)
3. Two `requestId`-less error lines at the same millisecond on the same model + session + cwd: would falsely collapse to one. ✗ edge case.

**Recommendation**: add `uuid` to `AssistantPayload` and the `Event` struct as a non-load-bearing capture so future audits can detect case 2 if it appears. Treat as a small bookkeeping addition, not a correctness fix.

---

## Q2 — Ingest coverage

### Stored today on every `Event`

`Event` struct in `crates/tokenscale-core` (per the test fixtures in parser.rs + queries.rs):

| Field | Source | Status |
|---|---|---|
| `source` | hard-coded `"claude_code"` | ✓ |
| `occurred_at` | JSONL `timestamp` | ✓ |
| `model` | JSONL `message.model` | ✓ |
| `input_tokens` / `output_tokens` / cache fields | JSONL `message.usage.*` | ✓ |
| `request_id` | JSONL `requestId` | ✓ |
| `content_hash` | derived | ✓ |
| `session_id` | JSONL `sessionId` | ✓ |
| `project_id` | JSONL `cwd` (raw) | ✓ |
| `workspace_id` / `api_key_id` | not Claude-Code-sourced | both `None` |
| `raw` | full JSONL line (if `capture_raw=true`) | ✓ |

### Not ingested

- **`gitBranch`** — present on every assistant JSONL line (visible in the test fixture: `"gitBranch":"main"`), but `AssistantPayload` doesn't have a field for it. Discarded. **Easy to add: one struct field + one Event field.** Not currently surfaced anywhere — would just be available for future reports.
- **`uuid`** — same as above (see Q1).
- **`parentUuid`** — also present per the test fixture. Would let the dashboard reconstruct turn ordering inside a session if needed.
- **Tool-use events** — `assistant`-typed lines have a `message.content` array that can contain `tool_use` blocks (the test fixture has `"stop_reason":"tool_use"` confirming this happens). The parser's `AssistantMessage` struct has only `model` and `usage` fields — **the content array, and any tool calls inside it, are entirely discarded**. Same goes for `tool_result` lines (user-typed lines with a tool result block in their content): the parser routes them through `JsonlLine::Other` and skips them.
- **`file-history-snapshot` records** — not ingested. Same pattern.

### Implication for Phase 2 + 3

- **Phase 2** (commit attribution Tier 1, "find `git commit` Bash calls + capture SHAs") cannot ship without ingesting `tool_use` blocks in the assistant content array AND `tool_result` blocks from user lines that follow. This is a real ingest expansion: new event variants, new schema columns or a `tool_calls` JSON column.
- **Phase 3** (edit-survival heuristic) also needs `Edit`/`Write` tool-use events + `file-history-snapshot` records, all of which are currently dropped.

The Phase 2/3 prerequisite is concrete: a new event-kind in the store for tool calls, with at minimum `(parent_event_id, tool_name, tool_input_json, tool_result_json, occurred_at)`. Plus a parser pass over the assistant message's content array and the matching user message's content array.

---

## Q3 — "67 projects" filter dimension

### What it actually counts

`list_projects_with_totals` in [`crates/tokenscale-store/src/queries.rs`](../crates/tokenscale-store/src/queries.rs:312) does:

```sql
SELECT events.project_id, COUNT(...), SUM(...)
FROM events
WHERE ... AND events.project_id IS NOT NULL
GROUP BY events.project_id
```

`events.project_id` is set directly from `assistant.cwd` at parse time, **with no normalization**. So every distinct `cwd` string is a separate "project" row. A single repo accessed from:

- `/Users/x/Dev/MyRepo`
- `/Users/x/Dev/MyRepo/src`
- `/Users/x/Dev/MyRepo/subdir`
- `/Users/x/git/checkouts/MyRepo` (worktree)

…produces **four separate "projects"** in the filter, each with a slice of the real total.

### Practical effect

Whether the "67 projects" number is meaningful depends on the user's `cd` habits. The maintainer's data has many `~/Library/Mobile Documents/com~apple~CloudDocs/Dev/<repo>/<subdir>` paths because CC was launched in a subdirectory of the repo at least once. The dashboard groups these as separate projects.

### Phase 1 fix candidate

Resolve `cwd` → git toplevel at ingest time. If the path is inside a git working tree, store the toplevel as `project_id`; otherwise store the raw `cwd`. Surface the **un-resolved raw `cwd`** as a secondary field (`workspace_id` is currently unused and could hold it).

**Open**: design decision #1 in the roadmap ("'Project' unit: raw `cwd`, resolved git toplevel, or git remote URL?"). See blocking-decisions list below.

---

## Q4 — cwd resolution

No code path resolves `cwd` to a git toplevel today.

- `crates/tokenscale-ingest-cc/src/parser.rs:168`: `project_id: assistant.cwd` — stored raw.
- `crates/tokenscale-ingest-cc/src/walker.rs` comment: "tries to reconstruct the original cwd from [the slug-encoded `~/.claude/projects/` directory name]. The actual cwd lands in each event via the `cwd` field on the JSONL line itself."

The walker does compute a reverse-encoded `cwd` from the slug, but that value is never assigned to `project_id` — it's only used to scope the file walk.

---

## Per-phase implications summary

### Phase 1 — per-project + per-thread reporting

**Can ship on current ingest** (modulo design decisions):

- Per-session report: already supported. `events.session_id` exists; just needs a new query.
- Per-project report: works as-is for users whose `cwd` is consistent. **Needs `cwd` → git-toplevel resolution at ingest** (this is the load-bearing Phase 1 work) to avoid the fragmentation in Q3.
- Optionally capture `gitBranch`, `uuid`, `parentUuid` while we're touching the parser.

### Phase 2 — commit attribution Tier 1

**Blocked on ingest expansion.** Cannot proceed until:

- `tool_use` blocks in assistant message content are parsed.
- `tool_result` blocks from user-typed lines are parsed (Bash output for the `git commit` SHA capture).
- A new event kind or a `tool_calls` column lands in the store.

This expansion is sizeable enough to be its own phase, not a "small follow-up." Suggest it be Phase 1.5 or a precondition explicitly listed inside Phase 2 with its own sign-off gate.

### Phase 3 — commit attribution Tier 2 (edit-survival)

Same prerequisite as Phase 2, plus `file-history-snapshot` ingestion. Same caveat: prerequisite work is non-trivial.

### Phase 4 — commit attribution Tier 3 (post-commit hook)

Independent of the ingest expansion. The hook writes a `Tokenscale-Session: <id>` trailer into commit messages, and Tokenscale's reader side just needs to extract the trailer from `git log` output at report time. Could ship as a standalone first if Phase 2/3 prerequisites are too costly upfront.

---

## Blocking design decisions (from roadmap §"Design decisions to surface")

Each of these needs an answer before the corresponding phase can be implemented. Repeated here so a Phase 1+ sign-off pass can resolve them in one round.

1. **"Project" unit**: raw `cwd`, resolved git toplevel, or git remote URL? (Phase 1.)
   - **Recommendation**: resolved git toplevel at ingest (filesystem-level, doesn't require network); fall back to raw `cwd` for non-git directories. `git remote URL` would be cleanest for cross-machine attribution but requires a `git config --get remote.origin.url` shell-out per project AND fails on local-only repos. Toplevel is the better default.

2. **Non-git `cwd`s**: how do they appear in per-project reports? (Phase 1.)
   - **Recommendation**: bucket them under their raw `cwd` and tag the row as "non-git" in the API + dashboard. Don't hide them.

3. **Git access architecture** for Tier 2 / 3: shell-out at report time, snapshot at ingest, or hybrid? (Phase 2+.)
   - **Recommendation deferred** to Phase 2 sign-off — meatiest call in the roadmap and worth its own focused decision pass. Snapshot-at-ingest is cheaper at report time but requires Tokenscale to write to a new table on every scan; shell-out-at-report is more accurate (always reads current HEAD) but requires repos still living at their original paths and is slow on large repos.

4. **Privacy posture**: commit attribution opt-in / allowlist / on by default? (Phase 2+.)
   - **Recommendation**: opt-in by default. The tool just promoted from "reads my CC logs" to "reads my repos." Per-project allowlist (`commit_attribution.enabled_for = [path1, path2]`) is one notch more friction than a single global flag and gives users explicit per-repo control.

5. **Report surface**: new dashboard view, filter mode, exported report, or combination? (Phase 1+.)
   - **Recommendation**: project / session reports get a new dashboard section with the existing filter chips. Commit-attribution numbers get a dedicated tab with explicit Tier 1 / Tier 2 / Tier 3 labels (cf. decision 6).

6. **Tier 1 vs Tier 2 in UI**: how does the dashboard distinguish exact-but-partial from heuristic? (Phase 2+.)
   - **Recommendation**: separate stat cards labeled "Direct CC commits (exact)" and "Edit-survival estimate (± band)". Never mix into a single "% of code from CC" number.

---

## Recommended next moves

In order of decreasing certainty:

1. **Phase 1 — pilot per-session reporting on existing data.** No new ingest required; existing `session_id` is sufficient. Validates the data path before committing to the larger ingest expansion. Could be a single small PR.
2. **Decide on the "project" unit** (decision 1 above). Block Phase 1's per-project work on this single decision.
3. **Capture `gitBranch` + `uuid` + `parentUuid` in `Event`** (cheap, future-proof; lets later audits cross-check the UUID-duplication assumption against newer CC versions).
4. **Scope the ingest expansion as Phase 1.5** (tool-use + tool-result events as a new event kind / column). Explicitly gates Phase 2 + 3.
5. **Decide design decisions 3 (git access) and 4 (privacy posture)** before Phase 2 starts.

**No code until the user reviews this doc and signs off on phase shape.** This file replaces the "phase 0 outputs expected" item in `roadmap-granular-attribution.md`.
