# Roadmap, Phase 2: Tier 1 commit attribution

**Status**: scoping. §1 empirical baseline plus probe and probe 5 reports complete (2026-05-26). §§2 through 7 D-decisions drafted; awaiting maintainer sign-off before implementation begins.

**Goal**: for each session, produce the set of git commits the session authored via Bash. SHAs are captured from real `tool_result` content; misses are reported honestly with a structured signal indicating why. This is the "exact-but-partial" Tier 1 capability, sibling to Phase 3 (Tier 2 edit-survival, heuristic plus bands) and Phase 4 (Tier 3 forward instrumentation via post-commit hook).

**Builds on**:
- v0.1.17 Phase 1.5: the `tool_uses` and `tool_results` tables this phase queries. Phase 1.5 also shipped `list_session_bash_calls` (`crates/tokenscale-store/src/tool_data.rs:181`); Phase 2 consumes it through a new handler.
- v0.1.15 cwd_resolver: resolves any cwd to its git toplevel via `git -C <cwd> rev-parse --show-toplevel`. Reused for both cd-target and project_id resolution.
- `docs/roadmap-granular-attribution.md`: parent roadmap. Design decisions §1, §3, §4, §6 from that doc carry forward; the Phase 2 D-decisions here resolve the subset relevant to Tier 1.
- `docs/roadmap-2-probe-report.md`: the probe 1 through 4 report. Substantive empirical grounding for §1's numbers.
- `docs/roadmap-2-probe-5-report.md`: the truncation-mechanism verification. Corrects the truncation model in the prior probe report.

**Empirical floor**: the §1 analysis below is built on the probe-report era sample (143 real `git commit` invocations across 27 sessions and 68 project_ids). The v0.1.18 §9 smoke surfaced that the probe-report filter under-counted due to a heredoc-body-traversal bug shared between the probe filter and the pre-fix v0.1.18 filter; the corrected v0.1.18 baseline (after `strip_heredoc_bodies` fix) is **637 real commit invocations with 96.1% SHA capture**. See §1.11 for the corrected baseline; the §1.1 through §1.10 analysis is preserved for the design audit trail and the structural conclusions hold against both samples.

**Honest caveat**: the no-SHA rate measured in this corpus is project-specific. The `| tail -N` head-pipe pattern is widespread across the maintainer's projects (38% to 100% rate per project at the v0.1.18 baseline; see §1.11) rather than concentrated in any single one. Deployments without the head-pipe pattern will see a lower no-SHA rate; deployments with it will see higher. This is acknowledged throughout the design.

---

## 1. Empirical baseline

Sample: maintainer's production DB after `tokenscale scan --rebuild --yes` on 2026-05-26 against a locally-built v0.1.17 binary. The brew-installed `tokenscale-cli` was v0.1.10 and had no awareness of the v0.1.17 tool tables; the local build was required to widen the substrate. Scan summary: 31 files seen, 23,071 events inserted, 14,616 tool_uses inserted, 545,415 file_snapshots inserted, 6 tool_use orphans, 0 lines malformed.

After rebuild: 22,711 tool_uses across 27 distinct sessions and 68 distinct project_ids. The pre-rebuild substrate was 8,095 tool_uses across 3 sessions; rebuild widened it 2.8x. Note that rebuild does not currently wipe the v0.1.17-added tables (issue [#6](https://github.com/RobarePruyn/tokenscale/issues/6)), so the post-rebuild counts include retained rows from prior partial scans plus the new inserts. The empirical analysis below is unaffected because per-row tool_use_id uniqueness prevents row duplication.

### 1.1 Canonical filter for identifying real `git commit` invocations

**Important convention**: a naive `LIKE '%git commit%'` filter on `tool_uses.input_json` overcounts by 4.7x in this corpus (671 raw hits versus 143 real invocations). The overcounts are meta-mentions: probe scripts, sqlite queries with `grep 'git commit'`, awk patterns, and other tool_uses whose command text contains the substring without actually invoking git.

The canonical filter is **shlex token-walk subcommand decomposition**:

1. Pull `input_json.command` for each `tool_use` with `tool_name = 'Bash'`.
2. Split the command on top-level `&&`, `;`, `||` separators.
3. For each subcommand, `shlex.split` it (POSIX-mode).
4. Mark the row as a real invocation if any subcommand's first two tokens are `["git", "commit"]`.

A future maintainer running queries against `tool_uses` for commit-related analysis must apply this filter or accept the inflated count. The same pattern applies to `git push`, `git tag`, and other subcommand-specific queries.

Implementation will live in `crates/tokenscale-store/src/commit_query.rs` (or equivalent), called from any handler that surfaces commit data.

### 1.2 Tool-name distribution (rebuilt corpus)

| tool_name | count |
|---|---:|
| Bash | 9,342 |
| Edit | 6,288 |
| Read | 3,521 |
| Write | 1,758 |
| TodoWrite | 1,432 |
| (others) | tail |

Phase 2 only consumes `Bash`. Phase 3 will consume `Edit` and `Write`.

### 1.3 `git` subcommand distribution (among 1,765 Bash calls mentioning `git `)

| subcommand | count |
|---|---:|
| commit | 671 (raw); 143 after shlex filter |
| add | 671 (raw) |
| push | 478 |
| status | 423 |
| log | 159 |
| diff | 134 |
| tag | 47 |
| rev-parse | 67 |
| (others) | tail |

`add` and `commit` track together by construction (CC's canonical pattern chains them). `push` and `tag` are out of Phase 2 scope; both are useful Phase 2.5 signals (commit-was-pushed, commit-was-tagged) and the data is already captured.

### 1.4 Commit-command shape on the clean set of 143 invocations

| flag or form | count | pct |
|---|---:|---:|
| `-m` present | 138 | 96.5% |
| HEREDOC `cat <<'EOF'` present | 137 | 95.8% |
| `--amend` | 3 | 2.1% |
| `--no-edit` (used with `--amend`) | 2 | 1.4% |
| `--fixup` | 1 | 0.7% |
| `--allow-empty` | 1 | 0.7% |
| `-am` | 0 | 0% |
| `-S` (sign) | 0 | 0% |
| `--no-verify` | 0 | 0% |
| `--reset-author` | 0 | 0% |

Combined `-m` and HEREDOC coverage holds at 97% in this corpus. Four dangerous-flag categories appear at low frequency: `--amend`, `--no-edit`, `--fixup`, `--allow-empty`. Three categories remain at zero: `-am`, `-S`, `--no-verify`. The narrow-shape conclusion holds in aggregate but is not absolute; the schema and parser cannot assume zero on any flag.

Representative canonical command:

```bash
cd "/Users/Robare/.../Dev/LifeOps" && git add lifeops/cli.py docs/ && git commit -m "$(cat <<'EOF'
Add migration runner and `db migrate` / `db status` commands

Body text...

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)" && git status
```

### 1.5 `cd "<path>"` prefix rate

123 of 143 (86.0%) of commit invocations begin with `cd "<absolute path>"`. CC's commit guidance encourages absolute-path prefixing as defensive practice. The cd target is the actual repo the commit lands in and may diverge from the session's `events.project_id` (the cwd at event time) when CC is operating in a parent directory.

The remaining 20 (14.0%) commits run from the session cwd without a cd prefix.

### 1.6 Tool_result shape and SHA-capture rate

For the 143 real commit invocations:

| outcome | count | pct |
|---|---:|---:|
| orphan (no tool_result) | 0 | 0.0% |
| tool_result present | 143 | 100.0% |
| Primary regex captures SHA: `[<branch>( \(root-commit\))? <sha>]` | 134 | 93.7% |
| Push-refspec fallback recovers SHA: `<old>..<new>  <local> -> <remote>` | +5 | cumulative 97.2% |
| No recoverable SHA after both regexes | 4 | 2.8% |

The 4 unrecoverable cases are:
- 2 maintainer-pipe-cut without chained push (cases #3 and #5 in `roadmap-2-probe-5-report.md`). Recoverable in principle by cross-tool_use linkage to a later push; see D2.
- 1 real failure: `.gitignore` rejection on the `git add` step, with `Exit code 1` and a `paths are ignored by one of your .gitignore files` marker.
- 1 testing exercise in `/tmp/main-repo` with multi-subcommand chain where `git commit`'s output is swamped by subsequent commands; not a production commit.

**The mechanism for cases #3, #5, and similar is the maintainer's own command pattern**, not Claude Code's tool truncation. Per probe 5, 7 of 7 head-cut cases in the corpus pipe `git commit` output through `| tail -N` deliberately, discarding the head. Claude Code's documented Bash middle-truncation at 30,000 chars (preserves head and tail, configurable via `BASH_MAX_OUTPUT_LENGTH`) is not invoked for any case; result contents are all under 1,000 chars. The maintainer's environment does not set `BASH_MAX_OUTPUT_LENGTH`. References: anthropics/claude-code [#19901](https://github.com/anthropics/claude-code/issues/19901), [#28783](https://github.com/anthropics/claude-code/issues/28783).

**Actor responsibility**: operators who want full SHA attribution can change their command pattern away from `| tail -N` on `git commit`. This is operator-actionable, not waiting on an upstream fix.

### 1.7 SHA resolution rate against current tokenscale tree

For the 67 distinct SHAs captured from tokenscale-project commits: **63 resolve in the current tree (94.0%); 4 are missing (6.0%)**. Missing SHAs: `3005bf8`, `5a39710`, `aa7f38a`, `b00fc87`. Likely reflect commits made on feature branches that were never merged, rebased away, or made in a different clone.

The empirical "exact-but-partial" floor for SHA resolution in this corpus is roughly 94%, not 100%. The schema captures the SHA verbatim regardless of resolution outcome; the report surfaces resolution status, not silently dropping unresolvable rows.

### 1.8 Per-project commit distribution

Among the 143 real commit invocations, by `tool_uses.project_id`:

| project_id (resolved) | commits |
|---|---:|
| `.../Dev/platform` | 92 |
| `.../Dev/LifeOps` | 17 |
| `.../Dev/tokenscale` | 13 |
| `.../Dev/mediacaster` | 8 |
| `.../Dev/OSS/mediacast-netcatalog` | 4 |
| (smaller buckets) | 9 |

**The probe-report era framing of "platform dominates the head-pipe pattern" turned out to be wrong on the v0.1.18 baseline.** §1.11 below shows the head-pipe rate measured per project across the rebuilt corpus: platform sits at 54%, mid-pack, with other projects ranging from 38% (QTrial) to 100% (displaycatalog). The platform repo contributes the largest absolute number of head-piped commits because it is the largest project by commit count, not because its operators use the pattern more. The sample-dependency caveat still holds: the no-SHA rate generalises only to deployments with similar command habits, and those habits vary by project, not by some single dominant cohort.

### 1.9 cd-vs-project_id divergence

After shlex-extracting the cd target and resolving both cd target and project_id through `git -C <cwd> rev-parse --show-toplevel`:

| metric | value | pct of cd-prefix |
|---|---:|---:|
| Real commit calls | 143 | |
| With cd prefix | 123 | 86.0% |
| Resolve to same git toplevel | 109 | 88.6% |
| Diverge after resolution | 14 | 11.4% |

Overall divergence against all 143 commits: 14/143 = 9.8%.

Divergence taxonomy:
1. **Sub-repo monorepo case**: cd target is the parent monorepo; project_id is a nested git subrepo (e.g., cd=`Dev/platform`, project_id=`Dev/platform/frontend/tenant-portal`). The cd target is the more accurate "what repo did this commit land in" signal.
2. **Session-jumping case**: cd target is in one repo, project_id is in a different repo (e.g., cd=`Dev/OSS/mediacastnet-dotgithub`, project_id=`Dev/NetCaster`). The cd target wins.
3. **Testing-in-/tmp case**: cd target is `/tmp` or a `/tmp` subdirectory; project_id is a real repo. The cd target wins for attribution accuracy, but the dashboard probably should not surface these as "real" project work; see D4.

### 1.10 Empirical observations carrying into design

1. The CC commit shape is narrow enough that a primary regex plus a single fallback covers 97.2% of cases. The remaining 2.8% breaks down further into recoverable-in-principle and irreducible misses.
2. The 0.7% real-failure rate is small relative to the 3.5% recovery-via-fallback rate, which shapes D5's diagnostic-field choice.
3. The 9.8% cd-vs-project_id divergence rate is meaningful attribution work.
4. The head-pipe pattern is project-specific; the corpus floor of 1.4% irreducible miss may not generalise.
5. SHA resolution against the current tree is ~94%; trees mutate, and the schema must accommodate this honestly.

### 1.11 v0.1.18 corrected baseline (post-§9 smoke)

The probe-report numbers in §1.1 through §1.10 were derived from a Python filter that traversed heredoc bodies during subcommand splitting; the v0.1.18 implementation initially had the same bug. The §9 smoke surfaced it and `strip_heredoc_bodies` was added before subcommand splitting. After the fix:

| Metric | Probe-report value | v0.1.18 actual |
|---|---:|---:|
| Real commit invocations | 143 | **637** |
| SHA capture (combined regex coverage) | 97.2% | **96.1%** |
| recovery_source = primary | 93.7% | **90.9%** |
| recovery_source = push_refspec | 3.5% | **5.2%** |
| recovery_source = none | 2.8% | **3.9%** |
| cd-prefix rate | 86.0% | **84.9%** |
| `is_amend` rate | 2.1% (3 commits) | 0.5% (**3 commits** matched) |
| /tmp filter suppressed | 1 | **1** |
| `--no-verify`, `-am`, `-S`, `--reset-author` | 0% each | **0% each** (probe zeros hold) |

The structural conclusions hold (narrow shape, dual-regex captures most, /tmp filter useful) and the absolute counts of qualitative categories (3 amends, 1 /tmp, 0 dangerous flags) match the probe-report's predictions. The absolute scale is what shifts.

**Head-pipe rate per project across all commits at n=637**, contradicting the prior "platform dominates" framing:

| Project (short) | commits | with `\| tail` | rate |
|---|---:|---:|---:|
| platform | 247 | 134 | 54% |
| LifeOps | 116 | 88 | 76% |
| QTrial | 80 | 30 | 38% |
| tokenscale | 54 | 22 | 41% |
| mediacaster | 48 | 46 | 96% |
| NetCaster | 44 | 20 | 45% |
| Pioneer | 9 | 8 | 89% |
| OSS/mediacast-netcatalog | 7 | 6 | 86% |
| Banashi-Website | 7 | 6 | 86% |
| OSS/mediacast-displaycatalog | 5 | 5 | 100% |

The `| tail -N` head-pipe pattern is **widespread, not concentrated**. Multiple projects show rates above platform's 54%; the platform contribution to absolute head-pipe-commit count is large only because platform is the largest project. The per-project rate varies from 38% to 100% with no single dominant cohort.

**25-case no-SHA breakdown** (the v0.1.18 release-gate hunt confirmed via the same taxonomy the probe used at n=9):

| Category | n=637 count | n=143 probe count (scaled) |
|---|---:|---:|
| head_piped_without_push | 17 | 2 (~9 expected at scale) |
| head_piped_with_push_but_fallback_failed | 2 | 0 (new-branch push shape; see D2 failure mode 4) |
| real_failure | 2 | 1 (gitignore reject; plus 1 cwd-deleted at n=637) |
| testing_in_tmp | 1 | 1 |
| orphan_no_result | 2 | 0 (interrupted sessions) |
| other (USER REJECTED tool use) | 1 | 0 (new semantic category) |

The new categories at n=637 (new-branch push, USER REJECTED, multiple orphans) are all legitimate semantic categories not parser failures. Re-bucketing the 2 new-branch-push under head-piped-no-recovery brings the total head-piped irreducible miss to 19 / 637 = 3.0%.

---

## 2. D1: schema for `session_commits`

### Decision

A new table `session_commits` keyed `(source, tool_use_id)` with sha-optional, cd-target verbatim, project resolved at insert time, and supporting columns for the diagnostic signals decided in D5.

### Options

**Option A: separate `session_commits` table.** New table, foreign-key by `(source, tool_use_id)` back to `tool_uses`. One row per real `git commit` invocation.

```sql
CREATE TABLE session_commits (
    id                                INTEGER PRIMARY KEY,
    source                            TEXT NOT NULL,
    tool_use_id                       TEXT NOT NULL,
    session_id                        TEXT NOT NULL,
    sha                               TEXT,                   -- nullable per 2.1% no-SHA rate
    cd_target_raw                     TEXT,                   -- verbatim, may be NULL when no cd prefix
    project_resolved                  TEXT NOT NULL,          -- forward-only, populated at insert time
    recovery_source                   TEXT NOT NULL,          -- enum 'primary' | 'push_refspec' | 'none'
    output_head_truncated_by_command  INTEGER NOT NULL,       -- 0 or 1
    is_amend                          INTEGER NOT NULL,       -- 0 or 1, see D6
    occurred_at                       TEXT NOT NULL,
    UNIQUE (source, tool_use_id)
);
CREATE INDEX session_commits_session_id_idx ON session_commits (session_id);
CREATE INDEX session_commits_project_resolved_idx ON session_commits (project_resolved);
CREATE INDEX session_commits_sha_idx ON session_commits (sha) WHERE sha IS NOT NULL;
```

Pros: clean separation of commit concerns from generic tool_use data; query patterns are straightforward; matches the Phase 1.5 D1 separate-tables pattern; nullable `sha` correctly expresses the no-SHA case.

Cons: a new migration; new INSERT path during ingest. Both consistent with Phase 1.5's pattern.

**Option B: extend `tool_uses` with commit-specific columns.** Add `commit_sha`, `commit_cd_target`, `commit_project_resolved`, etc. directly to `tool_uses`. Sparse on non-commit rows.

Pros: no new table; everything in one place.

Cons: 9,342 Bash rows plus thousands of non-Bash rows would carry sparse columns that are NULL for 9,199+ of them. Bad signal-to-noise. Couples commit-specific schema decisions to the generic `tool_uses` shape, which forecloses future tool-specific extension (e.g. Phase 3 would want similar treatment for Edit/Write).

**Option C: separate `commits` table keyed by SHA.** Key by `(source, sha)` rather than `(source, tool_use_id)`. Multiple sessions producing the same commit (rare but possible via rebase/cherry-pick from another session) collapse to one row.

Pros: deduplicates across sessions.

Cons: the no-SHA case has no natural key; would require synthetic IDs. Loses the natural foreign-key relationship to `tool_use`. The collapse-across-sessions feature is more confusing than useful at Tier 1; for "which sessions produced commit X", a query-time DISTINCT on `session_id WHERE sha = ?` answers the question without giving up the row-per-invocation semantic.

### Recommendation

**Option A.** Reasons:
- The Phase 1.5 separate-tables pattern is empirically the right shape for source-specific structured data extracted from `tool_uses` and `tool_results`. Applying it again for commits keeps the conceptual model consistent.
- The `(source, tool_use_id)` key is sufficient for uniqueness (tool_use_id is already unique per source via the `tool_uses` UNIQUE constraint). Adding session_id to the key would be redundant; instead session_id is a denormalised column with an index, matching how Phase 1.5 denormalised session_id and project_id onto the tool tables.
- **Session_id denormalisation safety**: the canonical source for session_id is `tool_uses.session_id`; the denormalised copy on `session_commits` is for query convenience. The forward-only ingest path (insert once at scan time, never update) guarantees no drift between the two. A future maintainer tempted to normalise should resist; the table is append-only by design.
- Nullable `sha` plus the `recovery_source` enum honestly expresses the multi-state outcome: captured via primary, captured via fallback, or genuinely missing.
- The `output_head_truncated_by_command` and `is_amend` flags are cheap booleans that carry diagnostic value per D5 and D6.

### Sub-decisions

- **Primary key shape**: `(source, tool_use_id)` per the reasoning above. Session_id is denormalised, not part of the key.
- **`sha` nullability**: `Option<String>`. Reflects the 2.1% combined irreducible-miss-plus-real-failure rate. NULL allowed at the schema level; partial index `WHERE sha IS NOT NULL` on the sha column keeps lookups cheap.
- **Verbatim cd target stored**: yes, as `cd_target_raw`. Forward-only resolution at insert time produces `project_resolved`. Storing the raw lets future code (or a later D3 update) re-resolve if `cwd_resolver` logic changes.
- **`project_resolved`**: NOT NULL. Always populated at insert time by running the cd target (if present) or project_id through `cwd_resolver`. Forward-only: historical sessions ingested before Phase 2 ships get no `session_commits` rows.
- **Relationship to `tool_uses`**: query-time JOIN on `(source, tool_use_id)`, no foreign key constraint. Matches Phase 1.5 D3 reasoning (parser stays pure; joins happen at the SQL layer).

---

## 3. D2: SHA capture strategy

### Decision

Two-regex within-tool_use capture, no cross-tool_use linkage. The within-tool_use linkage rule is explicit; failure modes are documented.

### Background

Per §1 empirical baseline, the primary regex captures 93.7% of clean commit invocations. The push-refspec fallback recovers an additional 3.5%. The remaining 2.8% breaks down into 1.4% maintainer-pipe-cut-without-push and 1.4% (1 real failure plus 1 /tmp testing exercise) genuinely unrecoverable.

### Within-tool_use linkage rule

For each row in `tool_uses` matching the shlex token-walk filter (§1.1):

1. Look up the corresponding `tool_results` row by `(source, tool_use_id)`.
2. If no `tool_result` exists, the commit is an **orphan**. Set `sha = NULL`, `recovery_source = 'none'`. Empirically 0 of 143 in this corpus, but the schema accommodates.
3. Apply the primary regex to `tool_result.content`: `\[(?P<branch>[\S]+?)(?: \(root-commit\))? +(?P<sha>[0-9a-f]{7,40})\] `. First match wins.
4. If primary regex captures, set `sha` and `recovery_source = 'primary'`.
5. If primary regex misses, apply the push-refspec fallback regex: `\b(?P<old>[0-9a-f]{7,40})\.\.(?P<new>[0-9a-f]{7,40})\b\s+\S+\s+->\s+\S+`. Capture the `new` group as the SHA.
6. If fallback regex captures, set `sha = new`, `recovery_source = 'push_refspec'`. **Take the FIRST push-refspec match in the content**; this is a load-bearing rule (see failure mode 2 below).
7. If both regexes miss, set `sha = NULL`, `recovery_source = 'none'`.

The linkage rule is "within the same `tool_result.content`". Push-refspec recovery only applies when `git push` is chained in the same Bash command as the `git commit` (e.g., `git commit ... && git push`).

### Cross-tool_use linkage: D2a sub-decision

For the 2/143 = 1.4% head-piped-without-push cases:

**Option A: accept as irreducible miss.** No cross-tool_use linkage. These rows have `sha = NULL`, `recovery_source = 'none'`, `output_head_truncated_by_command = 1` (per D5 Part 2). Operators can audit via the diagnostic field.

**Option B: implement cross-tool_use linkage.** Heuristic: after a no-SHA commit, look at the next `tool_use` in the same `session_id` with `tool_name = 'Bash'` matching `git push` (shlex token-walk), within a small time window (e.g. 60 seconds), with a matching cd target. Extract the push-refspec from THAT tool_result and attribute it back.

### D2a recommendation

**Option A.** Reasons:
- The 1.4% rate is small and project-specific. Other deployments will see different rates depending on command patterns.
- Option B's heuristic has its own failure modes: a commit-then-amend-then-push sequence would mis-attribute the amend SHA to the original commit; a multi-commit-then-push sequence would attribute only the last push refspec to the most recent commit, leaving earlier commits with `sha = NULL` anyway.
- The `output_head_truncated_by_command` field (D5 Part 2) gives operators a clear actionable signal: they can change their `| tail -N` pattern to recover full attribution without parser complexity.
- Phase 4 (post-commit hook with `Tokenscale-Session:` trailer) is the long-term answer for exact attribution. Adding cross-tool_use heuristic complexity now defers the work without solving the underlying problem.

### Failure modes of the chosen rule

1. **Commit and push to different repos in same command**: extremely rare (none in this corpus). Push-refspec would capture the push target's new SHA, not the commit's SHA. Detection: if `git push` is to a different upstream than the cd target's git toplevel, the recovered SHA may be wrong. Mitigation: not implemented; document as known limitation.
2. **Multiple commits chained in same command** (`git commit && git commit && git push`): none in this corpus. Push-refspec captures the LAST commit's SHA; the first commit's SHA is lost. The "first push-refspec match wins" rule means the first push (if any chained) is the source of truth. Tier 1 accepts this as a known limitation; Phase 2.5 could address if empirical rate climbs.
3. **Push-refspec contains an old SHA that coincidentally matches another captured SHA**: structurally impossible; the regex captures the `new` group (right side of `..`), which is the post-commit HEAD by git's definition.
4. **New-branch push (no `<old>..<new>` range in output)**: surfaced during v0.1.18 §9 smoke (2 of 25 no-SHA cases). When `git push` first-pushes a new branch, the output line is `* [new branch] <name> -> <name>`, not `<old>..<new>  <name> -> <name>`. There is no `<old>` SHA because the branch did not exist remotely. The push-refspec fallback regex requires the `<old>..<new>` form and correctly does not match; no SHA is recoverable from this result. The recovery rate is therefore sensitive to feature-branch workflows, not just `| tail` piping: a maintainer who does heavy new-branch work will see a lower recovery rate and a higher no-SHA rate. This is an irreducible shape, not a parser bug. Detection signal: `recovery_source = none` plus `output_head_truncated_by_command = true` plus a chained `&& git push` in the command suggests this case (versus head-piped-without-push, where no `git push` is chained).

---

## 4. D3: SHA resolution timing and access scope

### Decision

Query-time resolution via `git cat-file -e <sha>`, scoped to the resolved project root. Insert-time resolution is rejected because trees mutate. Broader git access is deferred to Phase 3 scoping.

### Options

**Option A: query-time resolution.** On each dashboard render that surfaces commit data, spawn `git -C <project_resolved> cat-file -e <sha>` per row. Store the result transiently in the response payload; no DB write.

**Option B: insert-time resolution stored as boolean.** Run `git cat-file -e <sha>` at the time `session_commits` is populated. Store `sha_resolved_at_insert: bool`. Display the stored boolean.

**Option C: cached resolution with periodic invalidation.** Store `sha_resolved_at: TIMESTAMP` plus the boolean; re-resolve if the stored timestamp is older than N hours.

### Recommendation

**Option A.** Reasons:
- Trees mutate: a SHA that resolved yesterday may be missing today after a rebase or force-push. Insert-time storage would go stale silently.
- Cache invalidation (Option C) adds complexity for negligible gain. The query-time cost is small (see cost model below).
- `cat-file -e` is the cheapest possible existence check (no object hydration; exits 0 or 1 immediately).
- Matches the Phase 1.5 D3 reasoning: derived signals computed at query time, not at insert time, when the upstream is mutable.

### Cost model

`git cat-file -e <sha>` is a process spawn (~5-10ms on modern hardware after warm filesystem cache). A typical dashboard view showing 10 sessions averaging 5 commits each is 50 calls totaling ~250-500ms in serial. Batching is possible (`git cat-file --batch-check` accepts SHAs on stdin) and reduces cost to ~50ms total for the same workload. The dashboard handler can adopt batched invocation from the start.

For the maintainer's current corpus (134 captured SHAs across the rebuilt set), a "show all commits" view would be ~134 calls. Batched: ~100ms.

**Forward-flag on scaling**: this cost scales linearly with the captured SHA count. At 1,000 SHAs the batched call lands at roughly 750ms; at 10,000 SHAs roughly 7.5 seconds. A future maintainer will hit a scaling cliff before the corpus reaches that size. Caching with invalidation (re-resolve on a TTL or on `git fetch` event) is the natural next step but is not in Phase 2 scope. Decision noted here so the cliff is hit at design time rather than release time.

### Display: exact-but-partial honesty

Each session_commits row in the API response carries:
- `sha: String | null`
- `sha_resolves_in_tree: bool | null` (null when sha is null, otherwise true/false)
- `recovery_source: "primary" | "push_refspec" | "none"`
- `cd_target_raw: String | null`
- `project_resolved: String`
- `output_head_truncated_by_command: bool`
- `is_amend: bool`

The dashboard renders `sha` verbatim, marks unresolved SHAs visually (e.g., struck through with a "not in tree" tooltip), and never silently drops rows. This is the same exact-but-partial honesty pattern §1 established for SHA capture: rows that are real-but-unverifiable stay on the screen with their status surfaced.

### Git access scope for Phase 2

`git cat-file -e <sha>` only. Scoped to `project_resolved` (the git toplevel). No `git log`, no `git blame`, no `git diff`. The parent roadmap's design decision §3 (git access architecture) is broader than Phase 2 needs; defer the full architectural decision to Phase 3 scoping, when `git blame` against actual repos becomes the load-bearing requirement.

This Phase 2 scoping deliberately accepts the narrow `cat-file -e` scope so it can ship without resolving the broader architecture.

---

## 5. D4: project attribution

### Decision

Prefer the cd target when present, fall back to `events.project_id` when absent. Resolve both through the v0.1.15 cwd_resolver. Filter /tmp testing commits from the dashboard by default with opt-in to surface them.

### Options for primary attribution

**Option A: prefer cd target, fall back to project_id, both through cwd_resolver.** Per §1.9, 86.0% of commits have a cd prefix; for those, the cd target is the more accurate signal (11.4% diverge from project_id after resolution). For the 14.0% without cd prefix, project_id is the only available signal.

**Option B: use project_id always.** Ignores the 9.8% overall divergence; mis-attributes 14 of 143 commits in this corpus.

**Option C: use cd target always.** For the 14.0% without cd prefix, no attribution is possible. Loses attribution for 20 of 143 commits.

### Recommendation

**Option A.** Reasons:
- Empirically catches both common cases (cd-prefix and no-cd-prefix).
- The 9.8% divergence rate quantifies that this is meaningful, not theoretical.
- Reuses the v0.1.15 `cwd_resolver` infrastructure for both paths; no new resolution logic.

### D4a sub-decision: /tmp testing commits

Per §1.9 divergence taxonomy, the "testing-in-/tmp" case is one of three divergence patterns. Per the probe report, /tmp commits are testing exercises (e.g., the `worktree add` test in case #1), not real project work. Default-surfacing them in the project attribution view mis-represents the operator's actual work.

**Options**:
- **A: filter by default, opt-in to surface.** Default API behavior filters out commits whose `project_resolved` starts with `/tmp/` or `/private/tmp/`. A query parameter `?include_testing=true` surfaces them.
- **B: no filtering.** All commits surface regardless of project_resolved.

**Recommendation**: **Option A** (filter by default). Reasons:
- The /tmp bucket is empirically testing, not real work.
- An opt-in flag preserves access for operators who want to audit testing activity.
- The detection rule (path-prefix on `/tmp/` or `/private/tmp/`) is simple and inexpensive.

Detection rule:

```rust
fn is_testing_project(project_resolved: &str) -> bool {
    project_resolved.starts_with("/tmp/")
        || project_resolved.starts_with("/private/tmp/")
        || project_resolved == "/tmp"
        || project_resolved == "/private/tmp"
}
```

---

## 6. D5: diagnostic fields

### Decision (two-part)

Add `recovery_source: Enum { Primary, PushRefspec, None }`. Add `output_head_truncated_by_command: bool`. Do not add `failure_reason`.

### Part 1: failure_reason vs recovery_source

**Options**:
- **A: no field.** Both "commit failed" and "parser missed" collapse to `sha = NULL`. Schema stays minimal. Diagnostic value lowest.
- **B: `failure_reason: Option<String>` only.** Captures why a sha is missing when one is. Useful for distinguishing commit failures from parser misses.
- **C: `recovery_source: Enum { Primary, PushRefspec, None }` only.** Captures where the sha came from in the success path. Useful for detecting future capture drift (e.g., primary regex starts missing more, push-refspec fallback rate climbs).
- **D: both fields.**

**Recommendation**: **Option C.** Reasons:
- The empirical 0.7% real-failure rate is small relative to the 3.5% recovery-via-fallback rate. The operationally more useful signal is where the SHA came from, because it surfaces future regression patterns (e.g., a CC version change starts breaking the primary regex; the dashboard sees `recovery_source: PushRefspec` climb sharply).
- The `None` bucket in `recovery_source` plus the `output_head_truncated_by_command` flag from Part 2 plus the `sha_resolves_in_tree` check from D3 together cover the diagnostic surface of "why is this row weird" without needing a separate string field.
- A `failure_reason` field as a separate column would be NULL for 99%+ of rows. The data does not justify the schema cost.
- If the real-failure rate climbs materially in a future corpus, this decision can be revisited; Option C does not foreclose adding a failure_reason later.

### Part 2: output_head_truncated_by_command

**Options**:
- **A: no signal.**
- **B: `output_head_truncated_by_command: bool`, detected by regex on command text for `| tail -N` patterns.**
- **C: more general truncation pattern enum** (`| head`, `| awk 'NR>X'`, other patterns).

**Recommendation**: **Option B.** Reasons:
- Cheap to compute: single regex on `input_json.command`.
- Orthogonal to `recovery_source`: a commit can have `recovery_source = 'push_refspec'` AND `output_head_truncated_by_command = true` (case #2, #4, #6, #7 in probe 5 report).
- Lets the dashboard distinguish operator-behavior misses from genuine failures. Operators can change their command pattern to recover SHAs that are currently `recovery_source = 'none'` because of head-pipe truncation.
- Matches the actor-responsibility framing: the head-cut is the operator's command pattern, not an upstream constraint.

Detection rule:

```rust
fn output_head_truncated_by_command(command: &str) -> bool {
    // Matches `| tail` with or without a numeric arg, accounting for whitespace.
    // Does not currently match `| head -n -N` (tail-equivalent); the empirical
    // pattern in this corpus is exclusively `| tail -N`.
    Regex::new(r"\|\s*tail\b").unwrap().is_match(command)
}
```

Future expansion to Option C is open if empirical evidence surfaces other output-compression patterns. The empirical rate in this corpus is 100% `| tail`, so Option B is sufficient now.

---

## 7. D6: `--amend` handling

### Decision

No special schema treatment in Phase 2. The `is_amend: bool` flag is captured for downstream filtering; semantic implications (history rewrite, orphaned prior SHAs) are not modeled in Tier 1.

### Background

Empirical: 3 of 143 (2.1%) commits use `--amend` in this corpus. All three are followed by `--no-edit` (reusing the prior commit message). The amend produces a SHA in the same `[branch sha]` shape captured by the primary regex.

### Options

- **A: no special treatment.** The amend appears in the data as a fresh commit row. The rewrite relationship is not modeled.
- **B: flag `is_amend: bool` for downstream filtering, no rewrite tracking.** Lets the dashboard filter or visually distinguish amends, but does not link to the prior commit.
- **C: track the amend chain.** At parse time, look up the prior commit in the same session matching the cd target, mark it as orphaned (rewritten). Requires cross-tool_use linkage.

### Recommendation

**Option B** (flag only, no rewrite tracking). Reasons:
- 2.1% rate is small but non-negligible; surfacing the flag costs one boolean column and avoids hiding the distinction.
- The primary regex captures the new SHA correctly; this is the SHA that lands in the tree post-amend. From a Tier 1 "what commits did this session author" perspective, the new SHA is the right answer.
- Rewrite tracking (Option C) requires the same cross-tool_use linkage complexity rejected in D2a, with similar failure modes. The 2.1% rate does not justify it now.
- The orphaned prior SHA naturally surfaces as `sha_resolves_in_tree: false` via D3, which is the right honest answer (the SHA was real when captured; it no longer resolves because of the rewrite). No additional modeling needed for Tier 1.

Detection rule:

```rust
fn is_amend_command(command: &str) -> bool {
    // Looks for --amend anywhere in the command. Cheap; false-positives
    // possible if --amend appears in a HEREDOC body, but the practical
    // rate is zero in this corpus.
    command.contains("--amend")
}
```

---

## 8. Forward-only posture

`session_commits` is a forward-only table. Historical sessions whose `tool_uses` rows were ingested before Phase 2 ships will not have `session_commits` rows; the table is populated only at ingest time going forward.

`tokenscale scan --rebuild --yes` is the user-elective backfill path. Pending the fix for issue [#6](https://github.com/RobarePruyn/tokenscale/issues/6), the rebuild semantic does not currently wipe the v0.1.17-added tool tables, so users running rebuild today will retain stale rows. The Phase 2 release should coordinate with the #6 fix to either include it or document the limitation.

**Stated in three places per the project pattern** (`feedback_forward_only_three_places.md`):
1. This scoping doc (§8 above)
2. The migration file header comment (when the Phase 2 migration is written; the comment must include "session_commits is forward-only; historical sessions have no rows; run scan --rebuild to backfill")
3. The CHANGELOG entry for the release that ships Phase 2 (v0.1.18 or similar; the entry's "Forward-only" section follows the v0.1.16 and v0.1.17 template)

This defends against future maintainers adding a "helpful" backfill step at upgrade. The user-elective rebuild path is the answer; upgrade-time backfill is not.

---

## 9. Release-gate framing

**Carries forward verbatim from `docs/roadmap-1.5-tool-use-ingest.md` § 7** per Phase 1.5 sign-off Addition 3.

Smoke against the maintainer's real DB is a release gate, not advisory.

The historical pattern across v0.1.13 through v0.1.17 is five-for-five: every schema-touching release surfaced a real bug during smoke that no test suite caught.

- v0.1.13: `daily_handler` conflation
- v0.1.14: `load_pricing_toml` schema-drift plus missing `pricing-divergence` label
- v0.1.15: `MIN(project_id)` attributed everything to home directory
- v0.1.16: pre-check inversion on same-uuid-different-request-id
- v0.1.17: `UserContent` enum too rigid, rejected 42 real user lines as Malformed

Phase 2 ships into the same discipline. **The bug-find is the expected outcome of the smoke gate.** The release does not tag until either the bug is found and fixed (with a pinning regression test added) or the bug-hunt is documented as exhaustive.

Smoke for Phase 2 specifically must include:
- Re-run all four probes from the probe pass and probe 5 against the rebuilt DB after Phase 2 ingest. Verify the canonical regex captures 134 SHAs (within Δ for any new commits since the rebuild), push-refspec fallback captures 5, `output_head_truncated_by_command` is true for exactly 7 rows (the head-pipe cases), recovery_source distribution matches §1 expectations.
- Confirm `session_commits` rows are 0 for sessions whose `tool_uses` predate the migration (forward-only invariant).
- Confirm `project_resolved` matches the v0.1.15 cwd_resolver output for every row.
- Confirm /tmp testing commits are filtered from the default dashboard view and surface only with `?include_testing=true`.
- Confirm `is_amend` is true for exactly 3 rows matching the §1 empirical count.
- Confirm the `sha_resolves_in_tree` field correctly reports true for 63 of 67 tokenscale-project captured SHAs.

If any of these surface a discrepancy, treat as the §7 expected bug-find. Investigate root cause; do not work around. Add a regression test that pins the corrected behavior. Tag the release only after the gate passes or the hunt is documented exhaustive.

---

## 10. Known limitations and Phase 3 scoping inputs

### Known limitation: `recovery_source = none` semantic conflation

The `none` bucket of the `recovery_source` enum collapses three semantically distinct outcomes into one value, identified during the v0.1.18 §9 release-gate hunt (25-case breakdown):

- **"Commit happened, SHA not recoverable"**: head-piped without chained push, or new-branch push (no `<old>..<new>`). Largest sub-class (19 of 25 at n=637, roughly 3.0% of the corpus).
- **"Commit did not happen"**: real failure (gitignore reject, CC cwd-deleted error) or user-rejected tool use. 3 of 25 at n=637.
- **"Unknown whether commit happened"**: orphan, no tool_result row (interrupted session). 2 of 25 at n=637.

The fourth row in the 25 is `testing_in_tmp` (1 case), filtered from the default dashboard view per D4a.

For Phase 2 this conflation is acceptable because none of these contribute a SHA to attribution; the `output_head_truncated_by_command` flag and the existence of a tool_result (orphan detection) plus failure markers in the content text already let an operator inspect each row's specific outcome via the API. The collapse does not affect attribution accuracy.

A future-Phase refinement could split `recovery_source = none` into `none_committed_unlinkable | none_commit_failed | none_unknown` so the dashboard surfaces "we could not link this commit" separately from "this commit did not happen". Not in Phase 2 scope.

### Phase 3 scoping input: distinguish no-SHA sub-classes for exact-but-partial honesty

When Phase 3 (Tier 2 edit-survival) scoping begins, take the `recovery_source = none` conflation above as a known input. Phase 3 builds on per-session file edits and `git blame`; its surface is the "% of session's code that survived" metric. That metric's honesty depends on attributing edits correctly to committed-or-not commits. If a session has many `recovery_source = none` rows of mixed types, Phase 3's denominator needs the distinction (committed-but-unlinkable still produced code in the tree; commit_failed produced no code; unknown is unknown).

Recommend Phase 3 scoping pass evaluate adding the three-way split to `recovery_source` as a sub-decision, with the empirical rate of each sub-class measured against the v0.1.18 corpus before recommending the schema change. The probe-then-design gate per the project pattern.

---

## Open questions for the maintainer

These emerged during §2 drafting and warrant explicit sign-off rather than implicit absorption into the build:

1. **D2a final call**: Option A (accept 1.4% irreducible miss) is recommended. Confirm or override.
2. **D5 Part 2 detection rule scope**: regex matches `| tail` anywhere in command. Should it also match within HEREDOC bodies (false positives, but currently zero in corpus)? Recommend leave as-is; revisit if false positives surface.
3. **Phase 2 release coordination with issue #6 fix**: ship Phase 2 with the #6 fix included, or ship Phase 2 first and document the rebuild caveat in the CHANGELOG? Recommend ship together (#6 fix is small and the cleanup story is cleaner).
4. **Sample-dependency phrasing in the CHANGELOG**: the corpus is dominated by the platform repo's `| tail -N` pattern. Should the CHANGELOG explicitly call out the project-specific nature of the no-SHA rate, or leave it to operators to discover via the diagnostic fields? Recommend explicit call-out for operator transparency.

Each is small enough to resolve in a single follow-up exchange. Awaiting sign-off before implementation begins.
