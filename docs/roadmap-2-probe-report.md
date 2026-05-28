# Roadmap, Phase 2: probe report (Probes 1 through 4)

**Status**: probe pass complete (2026-05-26), conditional on the §1 sign-off. Reported before §2 D* drafting per the maintainer's sequencing instruction. Awaiting follow-up before proceeding to §2.

**Substrate**: maintainer's production DB after `tokenscale scan --rebuild --yes` against the locally-built v0.1.17 binary (brew was on v0.1.10). Scan summary: 31 files seen, 23,071 events inserted, 14,616 tool_uses inserted, 545,415 file_snapshots inserted, 6 tool_use orphans, 0 lines malformed.

**Headline**: the §1 baseline's conformity claims weaken once the substrate widens. Dangerous commit flags do appear (rare but non-zero). The LIKE filter overcounted by 4.7x. Push-refspec fallback recovers most "missed" SHAs. Real commit failures are below 1%. Two §7-class findings surfaced incidentally.

---

## Sample-size comparison

| metric | §1 baseline (pre-rebuild) | Probe report (post-rebuild) | change |
|---|---:|---:|---:|
| total tool_uses | 8,095 | 22,711 | 2.8x |
| distinct sessions | 3 | 27 | 9.0x |
| distinct project_ids | 14 | 68 | 4.9x |
| Bash calls | 3,404 | 9,342 | 2.7x |
| Bash calls mentioning `git ` | 527 | 1,765 | 3.3x |
| Bash calls mentioning `git commit` (raw LIKE filter) | 239 | 671 | 2.8x |
| Bash calls that are real `git commit` invocations (token-walk filter) | not measured | 143 | new |

The raw LIKE filter overcounted by 4.7x in the rebuilt set. The overcounts were probe scripts (sqlite queries, awk greps) and meta-mentions captured as Bash tool_uses during prior sessions. Token-walk filtering (shlex of each `&&`-separated subcommand, checking for `git commit` as the first two tokens) is the empirically-grounded denominator going forward.

---

## Probe 1: Widened-substrate conformity

### Commit-flag presence on the clean set of 143 real invocations

| flag | §1 (n=239) | probe (n=143) | change |
|---|---:|---:|---|
| `-m` present | 100.0% | 96.5% | weaker |
| HEREDOC `cat <<'EOF'` present | 99.2% | 95.8% | weaker |
| `--amend` | 0 | 3 (2.1%) | now present |
| `--no-edit` (used with `--amend`) | not measured | 2 (1.4%) | now visible |
| `--fixup` | 0 | 1 (0.7%) | now present |
| `--allow-empty` | not measured | 1 (0.7%) | now visible |
| `-am` | 0 | 0 | unchanged |
| `-S` (sign) | 0 | 0 | unchanged |
| `--no-verify` | 0 | 0 | unchanged |
| `--reset-author` | 0 | 0 | unchanged |

The "zero dangerous flags" claim from §1 does not hold on the wider sample. The non-zero variants are still rare (under 3% combined), but the parser should not assume their absence.

The 96.5% `-m` rate and 95.8% HEREDOC rate hold the shape's narrowness. The 4-to-5% that doesn't use `-m` is dominated by `--amend` and `--amend --no-edit` (which reuse the prior commit message). Schema and parser should be designed for the `-m` HEREDOC case as primary and the amend/no-edit case as a known secondary.

### SHA capture rate on the clean set of 143

| outcome | count | pct of clean set |
|---|---:|---:|
| orphan (no tool_result) | 0 | 0.0% |
| tool_result present | 143 | 100.0% |
| SHA captured by canonical regex `[<branch>( \(root-commit\))? <sha>]` | 134 | 93.7% |
| SHA recoverable via push-refspec fallback (`<old>..<new>  <local> -> <remote>`) | +5 | cumulative 97.2% |
| Genuinely no recoverable SHA | 4 | 2.8% |

The §1 capture rate (98.3% with the root-commit extension) doesn't hold on the wider sample without the push-refspec fallback. Adding the fallback regex recovers most of the difference. See Probe 4 for what the 4 unrecoverable cases actually contain.

### SHA resolution rate against the current tokenscale tree

| metric | §1 baseline | probe |
|---|---:|---:|
| Distinct captured SHAs from tokenscale-project commits | 63 | 67 |
| Resolved in current tree | 61 (96.8%) | 63 (94.0%) |
| Missing (rebased / abandoned / not-yet-pulled) | 2 (3.2%) | 4 (6.0%) |

The two new missing SHAs are `3005bf8` and `b00fc87`. Likely either commits made on a feature branch that was never merged, or commits made in a different clone of the repo. The "exact-but-partial" honesty floor for SHA-resolution in this corpus is around 94%, not 100%. Schema and report design must accommodate.

---

## Probe 2: cd-target vs project_id divergence

After fixing the cd-extraction to use shlex (the initial `\S+` regex broke on backslash-escaped iCloud paths like `Library/Mobile\ Documents/...`):

| metric | value |
|---|---:|
| Real commit calls | 143 |
| With cd prefix | 123 (86.0%) |
| Without cd prefix | 20 (14.0%) |

Of the 123 cd-prefix commits, with both cd target and `events.project_id` resolved through the v0.1.15 cwd_resolver pattern (`git -C <cwd> rev-parse --show-toplevel`):

| metric | value | pct of cd-prefix |
|---|---:|---:|
| Resolve to same git toplevel | 109 | 88.6% |
| Diverge after resolution | 14 | 11.4% |

Overall divergence rate against all 143 commits: 14 / 143 = 9.8%.

### Divergence taxonomy (sampled)

1. **Sub-repo monorepo case**: cd target is the parent monorepo, project_id is a nested git subrepo of it (e.g., cd=`Dev/platform`, project_id=`Dev/platform/frontend/tenant-portal`). The session was operating in the frontend subrepo but cd'd up to the platform root for the commit. The cd target is the more accurate "what repo did this commit land in" signal.
2. **Session-jumping case**: cd target is in one repo, project_id is in a different repo entirely (e.g., cd=`Dev/OSS/mediacastnet-dotgithub`, project_id=`Dev/NetCaster`). The session was started in one project, then cd'd to a sibling repo for the commit. The cd target wins.
3. **Testing-in-tmp case**: cd target is `/tmp`, project_id is a real repo (e.g., cd=`/tmp`, project_id=`Dev/tokenscale`). The commit is in a throwaway repo (`/tmp/main-repo` etc.). The cd target wins for attribution; downstream reporting can decide whether to surface these or filter them.

The structural recommendation stands: prefer cd target when present, fall back to project_id when absent. The empirical rate quantifies how much attribution the cd capture is actually doing: in roughly 1 of every 10 commits, cd-vs-project_id picks meaningfully different repos.

---

## Probe 3: Missed-shape absence

### Worktree paths

Two commands in the rebuilt corpus contain the substring "worktree":

1. A `/tmp/main-repo` and `/tmp/wt-test-2` testing exercise that runs `git init && git commit --allow-empty && git worktree add`. Not a production commit.
2. A `cd "..../Dev/tokenscale" && git commit -m "$(cat <<'EOF'` whose commit message text mentions "worktree" in the body. The actual `cd` target is the tokenscale root, not a worktree path.

**Zero production commits in the corpus are issued from inside a git worktree.** The schema and parser can ignore worktree-specific handling for this corpus.

### Flag tokens immediately after `git commit` (all occurrences across 671 raw rows)

| token | count | known? |
|---|---:|---|
| `-m` | 663 | yes |
| `--amend` | 3 | yes |
| `--no-edit` | 2 | yes, used with `--amend` |
| `-line` | 1 | regex artifact (false positive from awk-based token extraction; not a real flag) |
| `--allow-empty` | 1 | yes |
| `-q` | 1 | yes |
| `--fixup` | 1 | yes |

After filtering the regex artifact, no unknown flags appear. The pattern space is empirically closed at the seven known flags listed above.

### `git revert` and `git cherry-pick`

Out of scope by definition (not `git commit` invocations). Not probed.

---

## Probe 4: Failure-marker shapes on the clean no-SHA set

Nine clean no-SHA cases. Categorized by inspecting full result content:

| # | result-text shape | recovery via push-refspec? | classification |
|---|---|:---:|---|
| 1 | `/tmp/main-repo` worktree-test output (`git worktree list`) | no | testing in /tmp (not production) |
| 2 | ` 3 files changed, ...\nTo https://.../mediacast-netcatalog.git\n   8386b26..d9bd400  main -> main` | YES (d9bd400) | successful commit, head truncated |
| 3 | ` create mode ...` lines only (16 file creations, then output ends) | no | successful commit, head truncated, no push chain |
| 4 | git user.name advice block + ` 7 files changed, ...\nTo https://.../platform.git\n   4ab68da..c410282  main -> main` | YES (c410282) | successful commit, head truncated by advice block |
| 5 | ` create mode ...` lines only (terraform files) | no | successful commit, head truncated, no push chain |
| 6 | git user.name advice block + ` 1 file changed, ...\nTo https://.../platform.git\n   42a9998..d8b8d9f  main -> main` | YES (d8b8d9f) | same as #4 |
| 7 | same shape as #6 | YES (30ad18e) | same as #4 |
| 8 | `Exit code 1\nThe following paths are ignored by one of your .gitignore files:` | no | **REAL FAILURE**: gitignore rejection |
| 9 | `git status --short` output + ` create mode ...` + `---push---\nTo https://.../mediacaster.git\n   ef72953..023cd8b  main -> main` | YES (023cd8b) | successful commit; head truncated; chained explicit `---push---` separator | 

### Failure-marker counts on the clean no-SHA set (9 results)

| marker | count |
|---|---:|
| `nothing to commit, working tree clean` | 0 |
| `nothing added to commit` | 0 |
| `pre-commit hook` | 0 |
| `rejected` | 0 |
| `Exit code 1` (with gitignore-paths context) | 1 |

Only one of the nine clean no-SHA cases is a genuine commit failure. The "nothing to commit" / "pre-commit hook" markers I included in the §1 first-pass probe did not appear at all in the wider rebuilt corpus.

### Empirical takeaway

Of 143 real `git commit` invocations:

- 134 (93.7%) produce a canonical-format SHA captured by the primary regex
- +5 (cumulative 97.2%) produce a recoverable SHA via push-refspec fallback when chained with `git push`
- 2 (1.4%) are successful commits where CC truncated the result head and there's no push chain to fall back on (no recovery possible without an upstream tool_result-truncation fix)
- 1 (0.7%) is a real failure (`.gitignore`-rejected paths)
- 1 (0.7%) is a /tmp testing exercise

**Genuine commit failures are 1 in 143 (0.7%)** in this corpus. The "sha=None" case is overwhelmingly driven by tool_result content truncation, not by commit failures.

---

## Two §7-class findings surfaced incidentally

### 7a. `tokenscale scan --rebuild` does not wipe v0.1.17-added tool tables

Before rebuild: 8,095 tool_uses across 3 sessions. After rebuild that wiped 23,052 events: 22,711 tool_uses across 27 sessions. Scan summary reported `tool_uses_inserted: 14,616`.

The math says 8,095 prior rows were retained, then 14,616 new rows were inserted from the re-parse. The destructive `--rebuild` path in `crates/tokenscale-cli/src/main.rs` deletes events and clears `file_state`, but does not touch `tool_uses`, `tool_results`, or `file_snapshots`. The `UNIQUE (source, tool_use_id)` constraint on these tables means re-inserts are silently INSERT-OR-IGNORE'd, so no double-counting, but the semantic of `--rebuild` ("wipe and re-parse from scratch") is incomplete for the v0.1.17-added tables.

Impact on this probe pass: none. The before/after counts add cleanly, and re-running probes is idempotent on the union.

Impact on the project: a v0.1.18 bug. The right fix is to extend the destructive path to wipe all four CC-source tables in the same transaction. Worth filing as an issue once you sign off on this report.

### 7b. Tool_result content is truncated upstream when results are large

Seven of the nine clean no-SHA cases (#2, #3, #4, #5, #6, #7, #9) have result text that starts mid-output. The `[branch sha]` line that `git commit` writes first is consistently the casualty when the output is large (many `create mode` lines for a multi-file commit, or a chained `git status` that prepends many lines, or the git user.name advice block when committer identity is auto-configured).

This is not a tokenscale parser bug. The truncation happens upstream of the JSONL (either CC truncates the tool_result before writing, or it's a Bash subshell output-buffer limit). The parser receives what it receives.

Impact on Phase 2 design: the push-refspec fallback regex recovers 5 of the 7 truncation casualties because chained `&& git push` writes the new SHA at the END of the output (which survives), not the beginning (which gets truncated). The remaining 2 truncated-no-push cases are structurally unrecoverable without upstream changes. This shapes D2 (SHA extraction strategy): a dual regex (primary + fallback) is empirically required, and a small irreducible miss floor (~1.4% in this corpus) must be acknowledged in the schema and report.

---

## Implications for §2 D* options (preview only; awaiting sign-off)

These are not commitments. They preview where the empirical evidence leans for each pending D-decision.

### D1 (schema)

`session_commits` table keyed `(source, session_id, tool_use_id)` still holds. The empirical evidence reinforces:
- `sha: Option<String>` is correct; the no-SHA case is ~2.8% of clean invocations after fallback regex, ~6.3% without it
- `cd_target: Option<String>` (verbatim) is worth capturing; the divergence rate is 9.8%
- A small set of flag captures (`amended: bool`, `allow_empty: bool`) is cheap and lets the schema express the 4% non-`-m` cases

### D2 (SHA extraction strategy)

Dual regex empirically required:
- Primary: `\[<branch>( \(root-commit\))? <sha>\] ` (93.7% coverage)
- Fallback: push-refspec line `\b<old_sha>\.\.<new_sha>\b\s+\S+\s+->\s+\S+` (recovers +3.5pp to 97.2%)
- Acknowledge 2.8% irreducible miss from upstream truncation

### D3 (per-project attribution)

D3a (cd extraction via shlex), D3b (cd first, project_id fallback, both through cwd_resolver), D3c (SHA resolution at query time) all hold. The 9.8% divergence rate quantifies that this is meaningful, not theoretical.

### D4 (surface)

Carries forward from parent roadmap §6. No new evidence from this probe.

### D5 (forward-only)

Carries forward standard pattern.

### New explicit D-decision: failure_reason field

Empirical answer informs the recommendation:
- Genuine commit failures are 1 in 143 (0.7%) in this corpus
- The vast majority of "no SHA" cases are truncation artifacts, not failures
- A `failure_reason: Option<String>` field would be NULL for 99%+ of rows and would conflate "truncated" with "failed" without additional logic

**Recommendation leans toward Option A (no failure_reason field)** plus a small `result_present: bool` (always true on this corpus) and possibly a `recovery_source: Enum { Primary, PushRefspec, None }` to capture which regex caught the SHA. The diagnostic value of `failure_reason` is too low to justify the schema cost given the data, but `recovery_source` carries real diagnostic weight for any future SHA-capture drop.

To be debated explicitly in §2.

---

## Open questions for the maintainer before §2 drafts

1. **Tool_result truncation (§7b)**: any prior knowledge or filed issue about CC's behavior here? Worth tracking for Phase 2's "irreducible miss floor" framing.
2. **Rebuild semantic (§7a)**: file a v0.1.18 issue now, or leave it for a v0.1.18 scoping pass?
3. **Testing-in-tmp commits (Probe 2, divergence case 3)**: should the dashboard surface these or filter them by default? Probably a D4 sub-decision but flagging it now.
4. **Sample-depth follow-up**: this rebuild widened to 27 sessions / 143 real commits. Any need to widen further (e.g. include the `~/.claude-synced/laptop/projects` root which also appeared in the scan output)? Or is 143 sufficient empirical floor for the design pass?
