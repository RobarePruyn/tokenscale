# Handoff — post-v0.1.17

**Written**: 2026-05-26, immediately before the maintainer's dev-path
migration off iCloud. Intended for the first thread under the new
path; complements the canonical roadmap docs by capturing in-flight
state that isn't yet in them.

## Where we are

**v0.1.17 shipped** (tag `v0.1.17`, commit `e5780ef`, 2026-05-24).
Phase 1.5 — tool-use ingest expansion. Three new tables
(`tool_uses`, `tool_results`, `file_snapshots`) capture data the
v0.1.16 parser dropped. No new dashboard surfaces; gates Phase 2 +
Phase 3.

Phase 1 (v0.1.15 user-visible + v0.1.16 ingest layer) is closed.
Phase 1.5 (v0.1.17) is closed. Phase 2 + Phase 3 are next, with
their scoping passes due.

## In-flight items, none blocking

### Small follow-ups, owed but not urgent

1. **`FileHistorySnapshotPayload` parser-tighten + fixture-update.**
   v0.1.17 made `messageId` and `snapshot` `Option`-defaulted to
   route a legacy test-fixture shape to `Skip`. Empirical check
   confirmed real CC always emits both fields (0/3069 missing).
   The Optional posture is now weaker than the data warrants.
   Follow-up: replace `realistic_session.jsonl`'s legacy snapshot
   shape with the real shape, then tighten
   `FileHistorySnapshotPayload` back to required fields. Ships
   whenever convenient; no user-facing impact.

2. **Doc inaccuracy: trackedFileBackups frequency.**
   `docs/roadmap-1.5-tool-use-ingest.md` § 1 and the v0.1.17
   CHANGELOG entry both say "the dict can be empty (most common —
   many snapshots track nothing)". Empirically the breakdown is
   24/3069 empty (0.8%), 3,045/3069 non-empty (99.2%). The right
   wording is something like "the dict is non-empty in ~99% of
   observed snapshots; on the rare empty case the parser routes to
   Skip." Two-place patch.

3. **Sessions attribution refinement** — [issue #4](https://github.com/RobarePruyn/tokenscale/issues/4).
   v0.1.15 currently uses "longest cwd wins" as a heuristic for
   per-session project attribution. Issue #4 spells out the
   refinement to "most-frequent cwd" via window-function aggregation.
   Not blocking; tracked.

4. **Pre-existing lint debt** — [issue #1](https://github.com/RobarePruyn/tokenscale/issues/1).
   2 clippy errors (factors.rs:309, pricing.rs:644) + 6 frontend
   `react-hooks/set-state-in-effect` errors. All pre-date v0.1.13.
   Filed as a cleanup item; intentionally not folded into any
   feature release.

### Open queue

- **Phase 2** — Tier 1 commit attribution (exact-but-partial). Find
  `git commit` Bash calls via `list_session_bash_calls`, capture
  SHAs from result text, attribute to session.
- **Phase 3** — Tier 2 edit-survival (heuristic + bands). Use
  `list_session_file_edits` to find files Edit/Write'd per session,
  `git blame` current tree to measure survival rate.
- **Phase 4** — Tier 3 forward instrumentation. Opt-in post-commit
  hook writing `Tokenscale-Session: <id>` trailer.
- **PUE uncertainty band** — small factor-refinement.
- **Winget manifest** — small distribution work.

## Migration-specific notes

The dev-path migration (iCloud → stable local path) is happening
between this commit and whatever the next session writes. Expected
effects:

- **Project list fragmentation.** Historical events have
  `events.project_id = /Users/Robare/Library/Mobile Documents/com~apple~CloudDocs/Dev/tokenscale`.
  New events will have the new path. The dashboard's project list
  will show two separate "projects" for the same logical repo,
  one resolved (new path → git toplevel) and one raw-fallback
  (old path → no longer exists on disk → returns raw). This is
  the D1 query-time-resolution design behavior, not a bug.
  Both render correctly; new-path entries will grow over time
  and the old becomes historical.
- **DB unchanged.** Lives at
  `~/Library/Application Support/tokenscale/tokenscale.db`, not
  in the migrated tree. No action needed.
- **CC's `~/.claude/projects/` directory unchanged.** Already-
  written JSONL files keep their original `cwd` values (CC wrote
  them at session-start time). New CC sessions in the new path
  create new `~/.claude/projects/-Users-Robare-Dev-tokenscale/`
  subdirectories. The new tokenscale binary will scan both.

A future patch could rewrite `events.project_id` values for the
historical-iCloud-path range, but it's heavy-handed and not
required — the resolver handles the split cleanly at display time.

## Project conventions the next thread must honor

These are durable across the project and explicit in the prior
roadmap docs. Naming them here so the next thread doesn't need to
discover them.

1. **Forward-only schema changes.** No re-parse of historical
   JSONL at upgrade; no historical backfill. Stated in three
   places per schema-touching release (CHANGELOG, migration file
   header, scoping doc). v0.1.16 and v0.1.17 both follow this;
   future schema work should too.

2. **Smoke-test against real DB is a release gate, not advisory.**
   The "§7 release-gate framing" in
   `docs/roadmap-1.5-tool-use-ingest.md` § 7 carries forward
   verbatim into Phase 2 + Phase 3 scoping docs per the v0.1.17
   sign-off. The historical pattern across v0.1.13 → v0.1.17:
   every schema-touching release surfaced a real bug during
   smoke that no test caught. **The bug-find is the expected
   outcome.** Don't tag until either the bug is found and
   fixed/accepted, OR the bug-hunt is documented as exhaustive.

3. **Empirical before speculative.** Phase 0's discipline
   (`docs/phase-0-findings-granular-attribution.md`): sample real
   data before recommending schema shapes. Phase 1.5 § 1 did this
   with line-count breakdowns; Phase 2 + Phase 3 scoping must do
   the same (sample `tool_uses` for `git commit` patterns in
   Phase 2; sample `file_snapshots` for survival-rate methodology
   in Phase 3).

4. **Surface design decisions; don't bury them.** Every
   non-trivial choice gets a `D*` decision in the scoping doc
   with options + recommendation + reasoning. The maintainer
   signs off explicitly before code lands. Pattern locked from
   v0.1.13 onward.

5. **Build-it-right-the-first-time.** Maintainer preference
   recorded in `/Users/Robare/.claude/projects/-Users-Robare-Library-Mobile-Documents-com-apple-CloudDocs-Dev-tokenscale/memory/feedback_build_it_right.md`
   (auto-memory; will be stranded after migration — note for the
   next thread). The maintainer prefers up-front infrastructure
   investment over incremental minimum-viable.

6. **Three-places-stated forward-only.** When a schema migration
   adds forward-only fields (NULL for historical rows), state it
   in the CHANGELOG, the migration file header, AND the scoping
   doc. v0.1.16 and v0.1.17 both follow this; the discipline
   prevents future maintainers from "helpfully" adding a backfill
   step.

## Key reading for the next thread

In order of urgency:

1. **`CHANGELOG.md`** — v0.1.13 through v0.1.17 entries. Names
   every behavioral change and every release-gate finding.
2. **`docs/roadmap-1.5-tool-use-ingest.md`** — most recent scoping
   doc; § 7 is the release-gate framing template.
3. **`docs/phase-0-findings-granular-attribution.md`** — the
   empirical-grounding pattern.
4. **`docs/cost-methodology.md`** — corrections log; the audit
   trail for every pricing change. Append-only by policy.
5. **`docs/roadmap-1b-cwd-resolution.md`** — D1's cwd-resolver
   design; relevant to the migration-fragmentation note above.
6. **`docs/roadmap-cost-time-anchoring.md`** — the v0.1.13
   scoping doc, the original D1-D7 pattern other roadmaps follow.

## Open issues

- [#1](https://github.com/RobarePruyn/tokenscale/issues/1) — lint debt
- [#4](https://github.com/RobarePruyn/tokenscale/issues/4) — sessions attribution refinement
