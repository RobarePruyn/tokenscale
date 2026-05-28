# Roadmap, Phase 2: probe 5 report (truncation mechanism verification)

**Status**: probe 5 complete (2026-05-26). Reported separately per the maintainer's gate. Awaiting follow-up before any §2 drafting.

**Outcome**: outcome 2 from the maintainer's three-way framing. The truncation model in `roadmap-2-probe-report.md` § 7b is wrong. The actual mechanism is the user's own commands, not Claude Code. The dual-regex design still works but its rationale needs to shift and one of the four "structurally unrecoverable" claims needs re-examination.

**Action 1 status**: Issue [#6](https://github.com/RobarePruyn/tokenscale/issues/6) filed for the `--rebuild` semantic gap.

---

## Headline finding

`roadmap-2-probe-report.md` § 7b claimed: "Tool_result content gets truncated upstream by CC for large outputs, consistently dropping git commit's `[branch sha]` first line." That was wrong. The maintainer's pushback was right.

**The actual mechanism**: the bash commands themselves pipe `git commit`'s output through `tail -N`, deliberately discarding the head of the output to compress it for context-window reasons. The head of git commit's output is the `[branch sha] <subject>` line, so `tail -3`, `tail -5`, `tail -8`, `tail -15` all cut it.

**Why the previous framing was wrong**:
1. The Claude Code Bash tool's documented behavior is middle-truncation at 30,000 chars (configurable via `BASH_MAX_OUTPUT_LENGTH`) with the head and tail preserved.
2. All 9 no-SHA cases have `tool_result.content` between 154 and 999 chars. None are anywhere near the 30k middle-truncation threshold.
3. `BASH_MAX_OUTPUT_LENGTH` is not set anywhere in the maintainer's env (`~/.zshrc`, `~/.zshenv`, `~/.zprofile`, `~/.bashrc`, `~/.bash_profile`, `~/.profile`, `~/.config/zsh/.zshrc` all checked, plus `~/.claude/settings.json` and the project's `.claude/settings*.json`).
4. The raw JSONL file confirms the truncated content is what arrived from the Bash tool, not what tokenscale stored: for case #3, raw JSONL `tool_result.content` is exactly the same 999 chars as the DB row, head missing.
5. No truncation markers were found in any of the 9 cases. CC's documented middle-truncation inserts a visible marker (per anthropics/claude-code issue context). No markers means no middle-truncation happened.

---

## Per-case classification

For each of the 9 clean no-SHA cases, the actual mechanism and the design-relevant signal:

| case | result chars | command has `\| tail -N`? | command has `\| head`? | push-refspec recoverable? | classification |
|---:|---:|:---:|:---:|:---:|---|
| #1 | 339 | NO | NO | NO | testing-in-/tmp; multi-subcommand chain where many subsequent commands swamp `git commit`'s output |
| #2 | 212 | YES, `\| tail -3` (twice; once on commit, once on push) | NO | YES (d9bd400) | maintainer head-pipe + chained push fallback works |
| #3 | 999 | **YES, `\| tail -15`** (was not visible in prior 15-line preview) | NO | NO | maintainer head-pipe, no chained push to fall back on |
| #4 | 243 | YES, `\| tail -5` (twice) | NO | YES (c410282) | maintainer head-pipe + chained push fallback works |
| #5 | 262 | **YES, `\| tail -5`** (was not visible in prior 15-line preview) | NO | NO | maintainer head-pipe, no chained push to fall back on |
| #6 | 171 | YES, `\| tail -3` (twice) | NO | YES (d8b8d9f) | maintainer head-pipe + chained push fallback works |
| #7 | 154 | YES, `\| tail -3` (twice) | NO | YES (30ad18e) | maintainer head-pipe + chained push fallback works |
| #8 | 232 | YES, `\| tail -8` | NO | NO | real failure (`.gitignore` rejection), not a head-pipe artifact |
| #9 | 480 | NO | NO | YES (023cd8b) | `git status --short` output prepended before `git commit` runs; no `| tail`; chained `--push---` separator and `git push` keep refspec |

Summary:
- 7 cases involve the maintainer's own `| tail -N` pipe on git commit
- 1 case (#1) is a /tmp testing exercise with no `| tail` but a long subcommand chain
- 1 case (#8) is a real `.gitignore` failure
- 1 case (#9) has prepended output (`git status --short`) but the canonical `[branch sha]` line is still missing; this one warrants closer look but the dominant pattern is clear

The original probe report's claim of "2 structurally unrecoverable head-truncated commits" (cases #3 and #5) is now reframed: both are maintainer-pipe-cut WITHOUT a chained push. They are still unrecoverable from the result text alone, but the framing should not invoke "structural" because the mechanism is the user's own command, not an upstream constraint.

---

## CC middle-truncation: not invoked

The middle-truncation mechanism is documented behavior of Claude Code's Bash tool:
- Default limit: 30,000 chars (`BASH_MAX_OUTPUT_LENGTH` env var to adjust)
- Middle-truncation: preserves head and tail, removes middle
- Visible truncation marker inserted at the cut point

Probe 5 confirms middle-truncation is not the mechanism for any of the 9 no-SHA cases:
- All result contents are under 1,000 chars (vs the 30k threshold)
- No truncation markers found in any case
- `BASH_MAX_OUTPUT_LENGTH` is unset in the maintainer's env

The 50k disk-persistence threshold (claude-code issue [#28783](https://github.com/anthropics/claude-code/issues/28783) context) is also not in play; the contents would not approach that threshold.

References:
- anthropics/claude-code [#19901](https://github.com/anthropics/claude-code/issues/19901) for Bash tool docs
- anthropics/claude-code [#28783](https://github.com/anthropics/claude-code/issues/28783) for persistence thresholds

The probe report's § 7b claim ("Tool_result content gets truncated upstream by CC for large outputs") needs to be retracted and replaced. There is no evidence that CC's truncation mechanism affects any commit in this corpus.

---

## Why the maintainer (or CC sessions) uses `| tail -N`

Empirical observation, not a definitive root cause: 7 of 7 head-piped cases were authored by Claude Opus 4.6 or 4.7 sessions (per the `Co-Authored-By` trailer in the commit messages) operating on the Mediacast Platform repo. The platform repo commits dominate the no-SHA cases (7 of 9). Tokenscale repo commits in this corpus don't use `| tail` (they follow CC's default unfiltered commit guidance).

The most likely reason for the `| tail` pattern: an earlier CC session learned to compress git commit output to save context, or a project-specific instruction told it to. Either way, this is **session-specific behavior, not universal**. Other projects (tokenscale, NetCaster, LifeOps, etc.) use the canonical unfiltered form.

Design implication: the no-SHA rate is highly project-specific. The 6.3% no-SHA rate on the rebuilt corpus reflects the platform repo's session habits; other users may see effectively 0% no-SHA. The schema design should not over-fit to head-pipe recovery, but the push-refspec fallback is still a robust general improvement.

---

## Implications for § 2 D-options (corrected from the previous preview)

These are corrections to what `roadmap-2-probe-report.md` § "Implications for §2 D* options" said. Not commitments; awaiting your sign-off.

### D2 (SHA extraction strategy): the rationale shifts, the design stands

- Primary regex still captures 93.7% of clean commit results.
- Push-refspec fallback still recovers 5 of 9 no-SHA cases, lifting cumulative coverage to 97.2%.
- The fallback is no longer framed as "recovering from CC truncation" because that's empirically wrong. It is now framed as "recovering from any output-compression pattern in the command (most commonly maintainer-driven `| tail -N`) when a chained `git push` runs after."
- Cases #3 and #5 are no longer "structurally unrecoverable"; they are "maintainer-pipe-cut without a chained push." Recoverable in principle by:
  - Looking at the same session's next `git push` tool_use and matching it back to this commit (the "linkage heuristic" you flagged for D2)
  - Reading the commit SHA from local git via `git log --since="<occurred_at>"` and matching (out of scope for parser-only design)
- The genuine irreducible miss is now smaller: 1 real failure + 1 /tmp testing exercise = 2 of 143 (1.4%), not the 2.8% previously claimed.

### D2 push-refspec linkage heuristic (still needed; the rule below is a corrected starting point)

For chained `&& git push` cases (5 of 9 no-SHA, plus all the cases where push-refspec is redundant with the captured `[branch sha]` SHA), the linkage is structurally trivial because the push refspec appears in the SAME tool_result as the commit output. No cross-tool_use linkage needed.

For maintainer-pipe-cut without chained push (cases #3 and #5), recovering the SHA would require cross-tool_use linkage: look at the next `git push` tool_use in the same session with a matching cd target, within a small time window. **This is a separate design choice from the primary push-refspec fallback** and the empirical rate (2 of 143 = 1.4%) is small enough that it may not be worth the complexity.

### D-decision "failure_reason vs recovery_source": empirical answer is sharper now

- Real failures: 1 of 143 = 0.7% (case #8, .gitignore rejection)
- Head-piped cases: 7 of 143 = 4.9% (head-pipe-cut, of which 5 push-refspec-recovered, 2 unrecovered)
- Testing-in-tmp: 1 of 143 = 0.7%
- The `failure_reason` field would still be NULL for 99%+ of rows
- The `recovery_source: Enum { Primary, PushRefspec, None }` field gains a fourth category worth surfacing: head-piped-no-push (currently "None"). Or alternatively `recovery_source: Enum { Primary, PushRefspec, ChainedPushFallback, None }` to distinguish the chained-push-in-same-result case (#2, #4, #6, #7) from cases where the SHA was captured directly.

The original recommendation toward Option C (recovery_source) holds; the empirical evidence has clarified the values worth tracking.

### Framing updates needed in the §2 draft

The previous probe report's § 7b is wrong and the wording must change. Replacement should read approximately:

> A small subset of `git commit` invocations (7 of 143 = 4.9% in the rebuilt corpus) pipe the output through `tail -N` in the maintainer's own command, deliberately discarding the head of `git commit`'s output. The `[branch sha] <subject>` line is the first line of git commit's output and is the casualty. When the same command chains `&& git push`, the push refspec `<old>..<new>  <local> -> <remote>` is preserved and the dual-regex's push-refspec fallback recovers the SHA. When no push is chained (cases #3 and #5 in the probe corpus), the SHA cannot be recovered from this tool_result alone; cross-tool_use linkage to a subsequent push in the same session is possible but adds parser complexity. Claude Code's documented Bash tool middle-truncation (30k chars, head and tail preserved) is not invoked for any case in the corpus; result contents are all under 1,000 chars.

---

## Three-way outcome verdict

Maintainer framed three possible outcomes:

1. **Confirmed (chained-command middle-truncation)**: NOT this. CC's truncation is not the mechanism.
2. **Different mechanism than middle-truncation**: **YES, this.** The mechanism is the user's own `| tail -N` pipe. The dual-regex design still works (push-refspec fallback applies to the same chained-push pattern), but the rationale shifts and the framing about "structural unrecoverability" needs to soften.
3. **Mixed or unclear**: not necessary; the mechanism is clear after looking at the full commands.

The §2 D-options can proceed but two things must carry into the draft:
- The corrected truncation framing (no CC-truncation claim; the mechanism is user pipes)
- The decision about whether cases #3 and #5 (head-piped without chained push) warrant a cross-tool_use linkage recovery path, or whether 2/143 = 1.4% is small enough to accept as the irreducible miss

---

## Open questions for the maintainer

1. **Cross-tool_use linkage for head-piped-without-push cases**: worth a D-option, or accept 1.4% as irreducible miss? Empirical rate is small but the linkage logic (next push in same session, matching cd target, time window) is well-defined and reusable for the explicit D2 linkage heuristic anyway.
2. **The `| tail -N` pattern itself**: should the parser detect `| tail -N` in `input_json.command` and emit a structured signal (e.g., `output_head_truncated_by_command: true`) so reports can distinguish "we couldn't capture SHA because the user piped it out" from "we couldn't capture SHA because git failed"? This is an alternative to D2's failure_reason / recovery_source approach.
3. **Sample width again**: the head-pipe pattern is concentrated in the Mediacast Platform repo's CC sessions. If we widen to `~/.claude-synced/laptop/projects` or further (you said do not widen), the no-SHA rate likely drops because other projects use unfiltered commits. Acknowledging this in the §2 draft as a known sample-dependency.
