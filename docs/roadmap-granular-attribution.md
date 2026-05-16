# Roadmap — Granular attribution (queued)

**Status**: queued. Sequenced AFTER:
1. The current meta-review workstream (v0.1.10).
2. The existing Open queue (PUE uncertainty as a separate band, Winget manifest, the v0.1.6 follow-on All-view auto-clipping if still wanted, Cost Methodology page 6b).

Once those are clear, the first pass on this roadmap is **investigation only — Phase 0 below.** No code changes until the user reviews the Phase 0 findings and signs off on the phase shape.

---

## Goal

Tokenscale today reports tokens, counterfactual cost, and environmental impact aggregated by `(time bucket, model)`. Granular attribution adds a layer on top: per-repo, per-project, per-session/thread attribution of usage / cost / environmental impact, and eventually an estimate of how much CC-produced code survived into committed history.

Target capability, stated as a sentence the dashboard should be able to produce:

> *Building Tokenscale itself consumed X tokens, cost Y counterfactual dollars, had Z environmental impact across N sessions, and an estimated W% of the code CC produced is still in the repo.*

## Substrate assumptions (verify in Phase 0)

- CC partitions sessions by project on disk under `~/.claude/projects/<encoded-cwd>/`, the directory name being the working-directory path with slashes and spaces replaced by dashes.
- Each JSONL line carries session metadata: session ID, git branch, working directory; per-turn token usage; tool calls with their exact inputs and outputs.
- CC sometimes writes the same message (same UUID) into multiple JSONL files during branching or resume. **Summing usage without deduplicating by UUID inflates totals.** This is the load-bearing correctness assumption Phase 0 must confirm.
- CC keeps file-history-snapshot records that can be correlated with file-write tool calls to reconstruct what a session changed.
- Long sessions get compacted near the context limit; old sessions get trimmed off disk. Recent data is reliable, deep history is lossy.

## Phase 0 — investigation

**Output**: a findings doc, no code. Surface results to the user; flag the dedup result loudly if it confirms a live correctness bug.

1. **Dedup.** Does the parser deduplicate by message UUID before summing usage? If not, current per-thread/per-project numbers are already inflated. Treat this the way the meta-review treated pricing time-anchoring: not a "future feature," a current correctness issue. Tell the user if it should jump the queue.
2. **Ingest coverage.** Confirm whether the ingest crate stores `cwd`, `gitBranch`, `sessionId` per event. Critically: are tool-use events (Edit, Write, Bash including inputs and results) ingested at all, or only assistant usage blocks? If tool-use events are not ingested, commit-attribution Tiers 1 and 2 need an ingest expansion as a prerequisite.
3. **Existing project dimension.** The dashboard filter shows "67 projects" — confirm what it keys on. Raw `cwd`? An encoded path? Something else?
4. **`cwd` resolution.** Does any code resolve `cwd` to a git toplevel today, or is `cwd` used raw everywhere?

## Phase 1 — per-project and per-thread reporting

Reuses all existing methodology; no new uncertainty model.

- **If Phase 0 confirms the dedup gap, fix it here as item 0.** Possibly pull forward of the rest of this phase.
- Resolve `cwd` → git toplevel at ingest. Raw `cwd` fragments a single repo across subdirectories and worktrees.
- Per-project report: tokens, counterfactual cost, energy, CO₂e, water, session count, date range. GROUP BY over existing per-event data.
- Per-session/thread report: same metrics at session granularity.

## Phase 2 — commit attribution Tier 1 (direct CC commits, **exact but partial**)

- **Prerequisite**: Bash tool-use events ingested (Phase 0 confirms; may need ingest expansion).
- Parse Bash tool calls for `git commit`, capture resulting SHAs from tool output.
- Attribute those commits to their session and project.
- **Label as "commits CC authored directly."** Must not read as total CC contribution — it's a strict subset.

## Phase 3 — commit attribution Tier 2 (edit-survival, **heuristic + bands**)

Research-flavored. Closer in shape to an environmental factor sweep than to a normal feature.

- **Prerequisites**: Edit/Write tool events ingested + file-history-snapshot records ingested + git read access (see design decisions).
- For each session, collect the set of files/line ranges CC edited, then `git blame` the current tree and measure how much survives.
- Present as an estimate with an explicit uncertainty band, mirroring how the environmental numbers are presented. Squash merges, reformatting, later hand-edits all erode the estimate.
- Probably warrants its own short methodology doc.

## Phase 4 — commit attribution Tier 3 (forward instrumentation, **exact for new commits**)

The only path to exact attribution for human-authored commits made during a CC session.

- Ship an opt-in `post-commit` git hook that writes a `Tokenscale-Session: <id>` commit trailer when a commit lands during a CC session.
- Documented + install command. Does nothing for the existing backlog (Tier 1/2 handles that).
- Future commit attribution becomes a database join.

## Design decisions to surface (block phases above)

1. **"Project" unit**: raw `cwd`, resolved git toplevel, or git remote URL? Worktrees and multi-subdirectory usage argue for toplevel or remote. State recommendation + tradeoffs.
2. **Non-git `cwd`s**: some `cwd` values aren't repos. How do they appear in per-project reports?
3. **Git access architecture**: Tiers 2/3 need Tokenscale to read actual repos, not just `~/.claude`. Options: shell out to `git` at report time (needs repos still present at their original paths); snapshot relevant state at ingest; or a hybrid. The **meatiest** call here — it changes Tokenscale from "reads my CC logs" to "reads my CC logs and my repos."
4. **Privacy posture**: the tool is now public. Reading users' repos is more invasive than reading CC logs. Should commit attribution be opt-in / allowlisted / globally enabled by default? Propose a default.
5. **Report surface**: new dashboard view, filter mode, exported report, or combination? Same question for commit attribution.
6. **Tier 1 vs Tier 2 in UI**: Tier 1 is exact, Tier 2 is heuristic. How does the dashboard make this unambiguous so no viewer mistakes Tier 2 for ground truth?

## Output expected from Phase 0 + roadmap pass

Phased plan doc (this file replaced/expanded), Phase 0 findings doc, blocking-design-decisions list. **No code until user reviews and signs off.**

## How this sequences against other workstreams

- Meta-review v0.1.10 → finish first.
- Open queue (PUE uncertainty, Winget, Cost-methodology 6b, All-view auto-clip if still wanted) → user sequences alongside.
- macOS notarization → shipped in v0.1.9.
- **This roadmap** → starts after all the above. First action when picked up: Phase 0 investigation report.
