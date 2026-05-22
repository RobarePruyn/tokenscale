# Roadmap — Phase 1B: cwd → git toplevel resolution + parser captures

**Status**: scoping. No implementation in this pass.
**Date**: 2026-05-22
**Sequenced after**: Phase 1A (per-session reporting, landed on `main` in `818f33a`). Together 1A + 1B will tag as **v0.1.15** with the theme "Granular attribution Phase 1."
**Sequenced before**: Phase 1.5 (tool-use / tool-result / file-history-snapshot ingest expansion, gating Phases 2 and 3).

This doc surfaces design decisions and recommendations. No code lands until each `D*` decision is signed off.

---

## 1. cwd → git toplevel resolution strategy

### What needs solving

Phase 0 confirmed that the dashboard's "67 projects" filter keys on raw `cwd` strings. A single repo accessed from multiple working directories produces multiple "projects." Resolving each `cwd` to its git toplevel collapses the fragmentation to real repos.

### Option A — query-time in-memory map (built at server startup)

At startup:
1. Read every distinct `events.project_id` (raw cwd) from the DB.
2. For each, shell-out `git -C <cwd> rev-parse --show-toplevel` to find the toplevel.
3. Build an in-memory `HashMap<String, ResolvedProject>` keyed by raw cwd.
4. Per-project queries `GROUP BY` the resolved name instead of the raw column — but the SQL still GROUP BYs `events.project_id`; the Rust layer maps + re-aggregates after the SQL returns.

**Cost**: one shell-out per distinct cwd on startup. Maintainer's DB has 16 distinct project_ids in the 30-day window — sub-second.

**Disk-state behavior**: if a cwd's directory no longer exists, `git rev-parse` fails (exit non-zero / stderr "fatal: not a git repository"). The map falls back to using the raw cwd as its own resolved key — graceful degradation. Same for cwds that were never git repos (see Section 2).

**Reversibility**: zero. No schema migration, no backfill. Downgrade to v0.1.14 just works. A user can `rm` the map by restarting the server with a different `--no-resolve-cwds` flag (future) or by editing one in-memory data structure. The dashboard is wrong-on-fragmented-projects-but-correct-on-numbers, the v0.1.14 behavior, until restart.

**Historical consistency**: resolution reflects current disk state. If a user moves `~/Dev/MyRepo` → `~/Code/MyRepo` between two scans, the old events' resolution changes the next time the server starts. This is **the right behavior** for a per-project dashboard — the user wants to see "all the work I did in MyRepo," not "all the work I did at this filesystem path that no longer exists."

### Option B — ingest-time persistent column

A new column `events.git_toplevel TEXT` (nullable). During parse:
1. The parser shells-out `git rev-parse --show-toplevel` against the JSONL's `cwd`.
2. Result is stored in `events.git_toplevel` alongside the raw `events.project_id`.
3. Per-project queries `GROUP BY git_toplevel` directly.

**Cost**: one shell-out per assistant line at ingest. ~27,000 events on maintainer's machine → ~27,000 process spawns. Mitigatable by caching the resolution per-cwd-per-scan (one shell-out per distinct cwd) inside the ingest crate.

**Disk-state behavior**: resolution is frozen at ingest time. Project moves leave stale toplevels in the DB with no automatic correction. A `tokenscale audit project-paths` subcommand could detect and offer to re-resolve, but that's additional infrastructure.

**Migration**: a `sqlx::migrate!` forward-only migration adds the column. Existing rows have `git_toplevel = NULL`; a backfill step (run on first v0.1.15 start, or via a one-shot CLI subcommand) resolves them. The same shell-out cost applies but only once.

**Downgrade**: v0.1.14 binaries see the new column as an unknown field. `sqlx` queries that `SELECT *` would either succeed (if the binary's struct doesn't list the column) or fail loudly. v0.1.14 doesn't `SELECT *` — every query lists fields explicitly — so the column would be ignored cleanly. **But**: the migration cannot be rolled back without a manual `ALTER TABLE DROP COLUMN`, so a downgrade leaves the column orphaned, which mirrors the v0.1.13 forward-only convention.

**Historical consistency**: resolution is frozen at ingest. A re-scan with `--rebuild` would re-resolve everything to current disk state. Without that, project moves silently corrupt the per-project rollup.

### Recommendation: Option A (query-time in-memory map)

The Phase 0 finding makes this the safer call. Per-project fragmentation is a **labels** issue, not a **data correctness** issue — the underlying token / cost / impact numbers are right; only the rollup grouping is fragmented. Labels can be corrected freely; data corrections require migrations. Option A keeps the labels fix in the same risk class as label changes (i.e. zero migration risk).

Three concrete arguments for A:

1. **No migration ceremony for a labels fix.** Phase 1B is one of three workstreams that touch DB state in v0.1.15 (cwd resolution, parser capture, possibly `uuid` uniqueness). Keeping cwd resolution out of the migration set leaves the migration footprint minimal — only what's strictly necessary lands as schema.
2. **Filesystem state evolves; the dashboard should follow.** When a user moves a repo, the resolved name follows. Option B requires either re-ingest discipline or an audit subcommand to keep up.
3. **Cost is negligible.** Maintainer's 16 distinct cwds resolve in sub-second. Even an extreme power user with thousands of distinct cwds would startup in a few seconds — and the resolution can be cached to disk across restarts if needed (later optimisation).

The one case Option A serves worse than B: a user reading historical reports from before a repo move would see the post-move name. If that matters, the raw `project_id` column is still in the DB and can be surfaced as a secondary field for transparency. For v0.1.15 we don't surface it; the dashboard shows the resolved name only.

When a cwd's git repo no longer exists on disk at query time: the resolution maps to the raw cwd (graceful fallback). Section 2 covers the rendering.

---

## 2. Non-git working directory handling

### What the data looks like

The maintainer's DB has cwds like `~/Library/Mobile Documents/com~apple~CloudDocs/Dev/tokenscale` (git repo) but also paths under `/tmp/`, `~/`, and other non-repo locations from one-off CC invocations. All of these need a clean rendering in the per-project list.

### Options

**Option α**: fall back to the raw cwd. Each non-git directory shows as its full path. The "67 projects" number doesn't fully collapse for users with many one-off paths.

**Option β**: group all non-git directories under a single synthetic project, e.g. `"<not in a repo>"`. Aggressive collapse; a user loses the ability to see which non-git directory the work happened in.

**Option γ**: display individually but **flagged** — e.g. show the raw cwd basename with an icon or label indicating "not a git repo." Distinguishable from git repos in the UI without total loss of detail.

### Recommendation: Option α (raw cwd fallback) for v0.1.15

Simplest, least information-loss. Each non-git directory shows as itself; the UI may want to truncate-with-tooltip for display, but the underlying value is the full raw cwd.

Option β throws away information a user might actually want ("did I work in `/tmp/test-stuff` or `/tmp/another-test`?"). Option γ adds a UI distinction layer that's worth shipping later but isn't load-bearing for v0.1.15 — the raw cwd already carries the signal (the path doesn't look like a typical repo location).

The frontend's existing `projectShortName` helper already produces sensible short labels from raw paths; this works for non-git cwds without changes.

---

## 3. Git worktree handling

### Empirical check (this scoping pass)

Confirmed: `git rev-parse --show-toplevel` from inside a worktree returns the **worktree path**, not the main repo path. From `/tmp/main-repo` it returns `/tmp/main-repo`; from `/tmp/wt-test-2` (a worktree of the same repo) it returns `/tmp/wt-test-2`. The main repo path can be recovered via `git rev-parse --git-common-dir`, which from a worktree returns `/tmp/main-repo/.git` — strip the `.git` and you have the main worktree path.

So Option A's native `--show-toplevel` treats worktrees as distinct projects. Option B would too unless an additional resolution step runs.

### Options

**Option A1**: worktree-as-project. Each `git worktree add ../wt-branch` produces a new dashboard project. Pros: native, no extra shell-out, matches the user's filesystem mental model. Cons: a heavy worktree user gets N entries for one logical codebase.

**Option A2**: collapse worktrees to main repo. Additional `git rev-parse --git-common-dir` per cwd; strip `.git` to find the main worktree. All worktrees of the same repo group under the main path. Pros: one entry per logical repo. Cons: extra shell-out per cwd (small), and the displayed project path is the main worktree even if work happened in a feature-branch worktree.

### Recommendation: A1 (worktree-as-project) for v0.1.15

CC's primary use cases haven't been observed to involve heavy worktree usage by the maintainer (the live data shows no worktree-pair cwds). Pursuing A2 builds infrastructure for a case that may not exist for this user base, and A2's "main worktree path as the displayed name" can be confusing — a user working in a feature-branch worktree sees their session attributed to the main repo's path.

If a future user reports worktree fragmentation, A1 → A2 is a one-line change in the resolver (compose `--git-common-dir` lookup on top of `--show-toplevel`). Cheap to add later, cheap to defer now.

Worth noting in the v0.1.15 release notes: "worktrees currently count as distinct projects; tell us if this is a problem for you."

---

## 4. Parser changes — gitBranch / uuid / parentUuid

### Current state

`crates/tokenscale-ingest-cc/src/parser.rs`'s `AssistantPayload` deserializes:

```rust
timestamp, requestId (Option), sessionId (Option), cwd (Option), message
```

The JSONL line carries additional fields the parser drops: `uuid`, `parentUuid`, `gitBranch`. All three are present in every assistant line per the test fixture, but the parser doesn't extract them.

### Proposed additions

Three new fields on `Event`:

| Field | Type | Source | Nullability |
|---|---|---|---|
| `uuid` | `Option<String>` | JSONL `uuid` | Always present in current CC data per Phase 0 (27,388/27,388 lines), but `Option` for schema-drift tolerance. |
| `parent_uuid` | `Option<String>` | JSONL `parentUuid` | Usually present; null on the first turn of a session. |
| `git_branch` | `Option<String>` | JSONL `gitBranch` | Usually present; null when the cwd isn't in a git repo. |

All three are `Option` in Rust + nullable in SQLite, so the parser tolerates JSONL lines missing any of them and skips the field rather than failing the line. This matches the existing schema-drift convention (unknown fields ignored; missing fields default sensibly).

### Schema

Three new columns on `events`:

```sql
ALTER TABLE events ADD COLUMN uuid TEXT;
ALTER TABLE events ADD COLUMN parent_uuid TEXT;
ALTER TABLE events ADD COLUMN git_branch TEXT;
```

Whether `uuid` gets a `UNIQUE` index is the D2 decision (Section 5).

`gitBranch` and `parentUuid` are not load-bearing for any v0.1.15 dashboard surface; they're future-proofing. `gitBranch` will be useful for "filter by branch" in a later phase; `parentUuid` will be useful for reconstructing turn order within a session in a per-thread report (Phase 1B+ or beyond).

### Parser changes are isolated

The parser changes touch `crates/tokenscale-ingest-cc/src/parser.rs` and the `Event` struct in `crates/tokenscale-core`. The DB-side `INSERT` in `crates/tokenscale-store/src/events.rs` needs the new columns wired through. No query in the workspace currently reads any of these fields — they land as new ingestion-only data until a future phase surfaces them.

---

## 5. uuid: storage vs UNIQUE enforcement

### Phase 0 finding (restated)

27,388 assistant lines across 48 JSONL files, **zero UUID duplicates across files, zero same-UUID-different-`requestId`**. The roadmap's load-bearing correctness concern doesn't fire on this dataset.

### Options

**Storage only**: `events.uuid TEXT NULL`, no constraint. Cheap. Provides retrospective audit value (a future check can scan the column for duplicates and report). Zero defense — if CC behavior changes and starts producing duplicates, totals inflate silently between the change and the next audit.

**Storage + UNIQUE constraint**: `events.uuid TEXT NULL, CREATE UNIQUE INDEX ON events(source, uuid) WHERE uuid IS NOT NULL`. The partial index lets pre-v0.1.15 rows (with `uuid IS NULL`) coexist, so the migration doesn't break the existing dedup layer (`UNIQUE (source, request_id)`). Going forward, any uuid collision causes the INSERT to error.

Error-handling shape for UNIQUE:

- **Skip-with-warning**: catch the unique-violation, log `tracing::warn!`, skip the duplicate, continue. Same posture as the existing dedup logic. A future CC behavior change degrades gracefully — totals stay correct, the warning surfaces in logs.
- **Fail-the-line-outright**: bubble the error up, abort the scan. Loud signal that something changed, but a single bad line aborts an entire scan.

### Recommendation: storage + UNIQUE constraint with skip-with-warning

Phase 0 confirmed no current duplicates, but the roadmap's framing ("the load-bearing correctness concern") deserves an actual defense, not just retrospective audit data. Storage-only is half a defense. Adding the UNIQUE is one additional line in the migration; the cost is trivial.

Skip-with-warning rather than fail-outright matches the existing dedup pattern at `events.request_id`. A future CC change that introduces duplicates would degrade gracefully — `scan` continues, totals stay correct, the warning appears in logs and on the `--rescan` summary. If anyone notices duplicates in the wild and grows worried, they can rerun the scan and read the warning count.

The partial index (`WHERE uuid IS NOT NULL`) is structurally required because pre-1B rows have `uuid IS NULL` from the column default. Without the partial filter, a global UNIQUE would treat all those NULLs as colliding in some database engines (SQLite is permissive here, but partial index makes the intent explicit and portable).

---

## 6. daily_handler conflation fix

### What's actually wrong

In `crates/tokenscale-server/src/routes/usage.rs`, `daily_handler`'s loop sets `models_without_pricing` from the **time-anchored** lookup:

```rust
let billable_pair = state
    .pricing
    .lookup(provider_for_pricing, &row.model, &row.bucket)
    .map(|model_pricing| compute_billable_breakdown(model_pricing, &row));
let (billable, billable_total) = if let Some((breakdown, total)) = billable_pair {
    (Some(breakdown), Some(total))
} else {
    models_without_pricing.insert(row.model.clone());  // ← the conflation
    (None, None)
};
```

A model with launch_date 2026-04-16 in a window 2026-04-01..2026-04-30 fails the lookup for buckets 2026-04-01 through 2026-04-15 and gets added to `models_without_pricing`, even though the model is fully priced — just not for those dates. Sessions endpoint already gets this right by using the structural check.

### Important: the time-anchored lookup is NOT itself wrong

The `pricing.lookup(provider, model, &row.bucket)` call IS correct for computing `billable_pair` — billable multipliers need a per-bucket time-anchored rate, and a `None` result means "this bucket-row genuinely has no priced billable." The bug is **using that same `None` to populate `models_without_pricing`**, conflating "no row at this bucket date" with "no row at all for this model."

### Fix shape

Split the two concerns:

1. **`billable_pair` keeps the per-bucket time-anchored lookup** (correct for billable computation).
2. **`models_without_pricing` switches to the structural check** — same pattern `sessions_query` already uses:

   ```rust
   if !state.pricing.providers
       .get(provider_for_pricing)
       .is_some_and(|p| p.models.contains_key(&row.model))
   {
       models_without_pricing.insert(row.model.clone());
   }
   ```

### Shared helper

A new private fn in `tokenscale-server` (most likely `routes/usage.rs` itself, near both handlers, or a small `routes/_helpers.rs`):

```rust
fn model_has_pricing_row(
    pricing: &PricingFile,
    provider: &str,
    model: &str,
) -> bool {
    pricing.providers
        .get(provider)
        .is_some_and(|p| p.models.contains_key(model))
}
```

Both `daily_handler` and `sessions_handler` call this for `models_without_pricing` population. `daily_handler`'s billable lookup stays untouched. `sessions_handler`'s existing inline implementation gets replaced by the helper.

A similar helper for `models_without_factors` is justifiable; both handlers already do the same inline structural check on factors, so extracting it is a small bonus cleanup.

### Other call sites with the same conflation

Grepped the codebase for `pricing.lookup`:

- `crates/tokenscale-server/src/routes/usage.rs:363` — the daily_handler line above. Fix described.
- `crates/tokenscale-server/src/routes/usage.rs:752` (sessions_handler) — already uses the structural check after the 1A bonus fix; will move to the helper for symmetry.
- `crates/tokenscale-store/src/billable.rs` test — uses `lookup` for the billable invariant test; that's the correct usage (billable IS time-anchored).
- `crates/tokenscale-store/src/pricing_lookup.rs` — the lookup itself; not a caller.

No other handlers exhibit the conflation. The blast radius is just `daily_handler`.

### CHANGELOG entry shape

v0.1.15 release notes should mention this as a small correctness fix alongside the granular-attribution feature:

> **Fixed**: `modelsWithoutPricing` on `/api/v1/usage/daily` could include models that were priced but had launch dates inside the query window. The check now asks the structural question ("is this model in `pricing.toml`?") rather than the time-anchored one ("is there a row valid at the first bucket?"). Per-event pricing on the daily endpoint was unaffected — only the model-list banner could over-flag.

---

## 7. 1B atomicity with 1A

### What can split

Two natural sub-units:

- **1B-i**: cwd → git toplevel resolution (Sections 1, 2, 3). Pure server-side logic, no parser change, no migration. Frontend gets the resolved project name from the API. Reversible.
- **1B-ii**: parser captures gitBranch / uuid / parentUuid + UNIQUE constraint (Sections 4, 5). Schema migration, parser changes, ingest risk.

Plus 1B-iii: the `daily_handler` conflation fix (Section 6). Trivial — one shared helper replacing two inline checks.

### Risk asymmetry

- 1B-i: zero migration risk. Worst-case failure mode is "resolution doesn't work for some user's setup," surfaced as raw cwd fallback in the dashboard — same as v0.1.14 behavior. No data corruption possible.
- 1B-ii: real ingest risk. Migration runs on every user's DB. Parser changes affect every scan. A bug in the parser could fail-loud-on-startup or, worse, silently drop events.
- 1B-iii: zero risk; pure rendering fix.

### Options

**Option ψ — bundle 1B-i + 1B-ii + 1B-iii into one v0.1.15 tag.** The user sees one coherent "granular attribution Phase 1" release.

**Option χ — split**: 1B-i + 1B-iii land into v0.1.15 alongside 1A. 1B-ii (parser changes) lands as v0.1.16, no rush, more soak time in CI before brew users see it.

### Recommendation: Option χ (split parser work to v0.1.16)

Three reasons:

1. **1B-ii is not load-bearing for any user-visible feature in 1A or 1B-i.** `gitBranch` / `uuid` / `parentUuid` are future-proofing for Phase 1.5+ uses. Shipping them later costs nothing user-facing.
2. **1B-i answers the actual question 1B was meant to answer.** "67 projects fragments because cwd isn't resolved" — fixing that is what users will notice. The parser captures are quieter, less urgent.
3. **Risk concentration.** v0.1.13 shipped multi-row pricing schema + per-event SQL + frontend rewire + backfilled launch dates atomically, and we found two real bugs during the manual firing exercise. Concentrating another schema migration + parser change + UNIQUE constraint in the SAME tag as cwd-resolution sets up the same risk profile. Splitting cleanly separates the "labels fix" risk class from the "ingest schema" risk class.

The maintainer's call here is yours to make — Option ψ has the advantage of one coherent release narrative. But the split has the merit that one mistake in the parser doesn't take cwd-resolution with it.

If Option χ: v0.1.15 ships Phase 1A (sessions reporting) + 1B-i (cwd resolution) + 1B-iii (daily conflation fix). v0.1.16 ships 1B-ii (parser captures + UNIQUE). Phase 1.5 (tool-use ingest expansion) is its own release after that.

---

## 8. Migration and rollback

### If Option A is chosen for Section 1 (recommended)

No migration. No schema change for cwd resolution. The in-memory map lives in `AppState`. Downgrade from v0.1.15-with-1B-i to v0.1.14 is trivial — restart the older binary.

### If Option B is chosen for Section 1

Forward-only `sqlx::migrate!` migration adding `events.git_toplevel TEXT`. Mirrors v0.1.13's discipline: no down migration. A v0.1.14 binary reading a v0.1.15-shape DB would see an unknown column. v0.1.14's queries don't `SELECT *`; every column is named explicitly, so the unknown column is ignored cleanly. **However**, the DB now has a column the v0.1.14 binary doesn't know about — operationally fine (queries succeed), but `tokenscale audit ...` subcommands on the downgrade-target version don't see it. Documented downgrade note in the v0.1.15 CHANGELOG.

### Section 5 UNIQUE constraint migration (applies regardless of Section 1 choice)

Even with Option A for cwd resolution, **the UNIQUE constraint requires a migration**:

```sql
ALTER TABLE events ADD COLUMN uuid TEXT;
ALTER TABLE events ADD COLUMN parent_uuid TEXT;
ALTER TABLE events ADD COLUMN git_branch TEXT;
CREATE UNIQUE INDEX events_uuid_unique ON events(source, uuid) WHERE uuid IS NOT NULL;
```

Forward-only. No down migration. v0.1.14 binaries reading the v0.1.15 DB:

- See three new TEXT columns. Ignored by existing queries.
- The unique index doesn't affect any v0.1.14 INSERT path because v0.1.14 doesn't set `uuid` — every existing INSERT inserts `uuid=NULL`, which the partial index excludes.
- Downgrade is silent. Re-upgrade later: new events get uuids again, the partial index resumes enforcement.

### Recommendation per the split in Section 7

If Option χ (split parser to v0.1.16):
- **v0.1.15**: no migration. Phase 1A is already on `main`, 1B-i is server-side only, 1B-iii is one helper.
- **v0.1.16**: forward-only migration for the three columns + UNIQUE.

If Option ψ (bundle):
- **v0.1.15**: forward-only migration for the three columns + UNIQUE.

Either way, downgrade is documented in the relevant release notes.

---

## Design decisions to sign off

### D1 — Resolution timing (Section 1)

| Option | Pros | Cons |
|---|---|---|
| **A. Query-time in-memory map** *(recommended)* | No migration; reversible; tracks current disk state; ~sub-second startup cost | Lookup runs once per server start; depends on disk |
| B. Ingest-time column + backfill | Persistent; immune to disk-state changes | Migration + backfill; frozen at ingest; project moves cause stale toplevels |

**Recommendation: A.** Phase 1B is a labels fix, not a data correctness fix. Keep the migration footprint minimal; let the resolution follow filesystem state.

### D2 — uuid storage vs UNIQUE enforcement (Section 5)

| Option | Pros | Cons |
|---|---|---|
| Storage only | Cheap; retrospective audit value | Half a defense; future CC change could inflate totals silently |
| **Storage + UNIQUE (partial index) + skip-with-warning** *(recommended)* | Actual defense against the roadmap's load-bearing concern; graceful when triggered | One extra index in the migration; minor |
| Storage + UNIQUE + fail-outright | Loudest signal | One bad line aborts the entire scan; doesn't match the v0.1.10-era dedup posture |

**Recommendation: storage + UNIQUE with skip-with-warning.** Phase 0 found zero current duplicates, but storage alone is half a defense. UNIQUE with the existing dedup error posture is consistent and cheap.

### D3 — Non-git directory display (Section 2)

| Option | Pros | Cons |
|---|---|---|
| **α. Raw cwd fallback** *(recommended)* | Information-preserving; simplest; uses existing `projectShortName` | Doesn't collapse the "many one-off cwds" case |
| β. Single synthetic "Not in a repo" bucket | Aggressive collapse | Loses information; which `/tmp/...` was the work in? |
| γ. Individual + flagged (UI distinction) | Best of both | Extra UI work for v0.1.15; can be a v0.1.16+ refinement |

**Recommendation: α.** Ship the simplest correct thing. γ is the right v2 refinement once user feedback identifies whether the flag is useful.

### D4 — Worktree grouping unit (Section 3)

| Option | Pros | Cons |
|---|---|---|
| **A1. Worktree-as-project** *(recommended)* | Native `--show-toplevel` behavior; matches filesystem mental model | Heavy worktree user sees N entries for one logical codebase |
| A2. Collapse worktrees to main repo | One entry per logical repo | Extra shell-out per cwd; main-worktree-path display can confuse |

**Recommendation: A1.** No observed worktree pattern in current data. A1 → A2 is a future one-line addition if a user reports the fragmentation. Release notes call out the choice so users with heavy worktree usage know to ask.

### D5 — 1B atomicity (Section 7)

| Option | Pros | Cons |
|---|---|---|
| ψ. Bundle 1B-i + ii + iii into v0.1.15 | One coherent release narrative | Concentrates migration risk with cwd-resolution work |
| **χ. Split: 1B-i + iii → v0.1.15; 1B-ii → v0.1.16** *(recommended)* | Risk separation; cwd-resolution fix unblocked; parser changes get soak time | Two releases, more sequencing overhead |

**Recommendation: χ.** Parser captures aren't load-bearing for any user-visible v0.1.15 feature. Splitting separates the labels-fix risk class from the ingest-schema risk class. Mirrors v0.1.13's lesson: concentrating multiple schema-touching changes in one tag amplifies risk; the v0.1.13 manual firing surfaced two real bugs that a less-concentrated release pattern would have isolated.

---

## Output expected from sign-off

After D1–D5 are answered:

1. Phase 1B-i (cwd resolution) + 1B-iii (daily_handler fix) implement as a single pass, lands in v0.1.15 alongside 1A.
2. CHANGELOG entry mentions the `modelsWithoutPricing` fix as a small correctness improvement.
3. Phase 1B-ii (parser + UNIQUE) scoped separately when v0.1.15 lands. Or, if D5 = ψ, folded into the same v0.1.15 pass.
4. Phase 1.5 (tool-use ingest expansion, gating Phases 2 + 3) is the next scoping pass after v0.1.15 (or v0.1.16 if χ).

## Sequencing summary

```
v0.1.15  (next tag)
  ├─ Phase 1A — per-session reporting [landed on main 818f33a]
  ├─ Phase 1B-i — cwd → git toplevel (this pass, pending sign-off)
  └─ Phase 1B-iii — daily_handler modelsWithoutPricing fix (this pass)

v0.1.16  (if D5 = χ)
  └─ Phase 1B-ii — parser captures gitBranch / uuid / parentUuid + UNIQUE on uuid

v0.1.17+ (Phase 1.5)
  └─ tool-use / tool-result / file-history-snapshot ingest expansion
     gates Phase 2 (Tier 1 commit attribution) + Phase 3 (Tier 2 edit-survival)
```

No code until D1–D5 are signed off.
