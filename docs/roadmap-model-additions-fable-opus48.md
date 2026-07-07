# Roadmap: model additions, Fable 5 + Opus 4.8 (plus subagent ingest fix)

**Status**: scoping. Empirical grounding complete (2026-06-16). D-decisions below await maintainer sign-off before edits land. Target release: v0.1.19.

**Origin**: the maintainer used Claude Fable 5 heavily for ~4 days (2026-06-09 to 06-13) on the laptop before Anthropic pulled it. "Get it ingested." Investigation surfaced that the ingest was the load-bearing gap, plus two adjacent gaps (Opus 4.8 also unpriced/unfactored; subagent transcripts never walked).

---

## 1. Empirical baseline

### 1.1 Ingest state (after this session's incremental scan)

| model | events (deduped, top-level) | date range | priced? | factored? |
|---|---:|---|:---:|:---:|
| claude-fable-5 | 3,965 | 2026-06-09 to 06-13 | no | no |
| claude-opus-4-8 | 6,693 | 2026-05-29 to 06-15 | no | no |
| claude-opus-4-7 | 23,731 | 2026-04-18 to 06-05 | yes | yes |
| claude-opus-4-6 | 1,367 | 2026-04-07 to 04-21 | yes | yes |
| claude-sonnet-4-6 | 1,020 | 2026-04-06 to 06-09 | yes | yes |
| `<synthetic>` | 134 | admin aggregate | n/a (excluded) | n/a |

Fable usage magnitude (top-level only): ~1.52B total tokens, 98% cache-read (1.486B cache-read, 27.3M cache-write-1h, 3.1M output, 1.10M input). All Mediacast laptop work across ~70 sub-projects/worktrees.

### 1.2 Two ingest gaps

1. **Timing (resolved):** the laptop top-level session files had not synced into `~/.claude-synced/laptop/projects` at the time of this session's `--rebuild`. An incremental scan picked them up (files_seen 36 to 60); the 3,965 Fable events above are now in the DB.
2. **Subagent transcripts (open):** the walker (`crates/tokenscale-ingest-cc/src/walker.rs`) reads exactly `<root>/<project>/*.jsonl`. It never descends into `<project>/<session-id>/subagents/agent-*.jsonl`. Raw grep shows **3,606 additional `claude-fable-5` events in subagent files** (plus every other model's subagent spend across all projects). These are structurally invisible today. See D4.

### 1.3 Pricing + dates (web-sourced 2026-06-16, verify-against-source)

From Anthropic's pricing page (`https://platform.claude.com/docs/en/about-claude/pricing`, the same URL `pricing.toml` already cites) plus the public launch record:

| model | input | output | cache-read | cw-5m | cw-1h | valid_from | source |
|---|---:|---:|---:|---:|---:|---|---|
| claude-fable-5 | $10 | $50 | $1 | $12.50 | $20 | 2026-06-09 | pricing page; launch per Anthropic news + InfoQ/TheNewStack |
| claude-opus-4-8 | $5 | $25 | $0.50 | $6.25 | $10 | 2026-05-28 | pricing page; launch per Axios/Simon Willison/Memeburn |

Multipliers are the standard 0.1x cache-read, 1.25x 5m-write, 2.0x 1h-write for both. Opus 4.8 pricing is identical to Opus 4.6/4.7. Fable 5 is 2x Opus (most expensive GA model Anthropic has shipped).

**Re-verification side benefit:** while fetching the page I confirmed every existing `pricing.toml` row is unchanged (Opus 4.6/4.7 still $5/$25, Sonnet 4.6 $3/$15, Haiku 4.5 $1/$5). So this release doubles as a clean quarterly re-verification of the whole rate card.

### 1.4 Fable lifecycle (for the dating + drift-detector decisions)

Launched 2026-06-09; disabled 2026-06-12 (~3 days) by a US-government export-control directive (pulled alongside Mythos 5; first time a US export control pulled a live deployed model). Still listed on the pricing page as of 2026-06-16 despite being disabled. No announced restoration date. This shapes D3 (dating) and D5 (drift-detector tolerance).

### 1.5 Tokenizer note

Opus 4.7+ (so Opus 4.8 and Fable 5) use a new tokenizer that can use up to ~35% more tokens for the same text. The factor file already carries `tokenizer_token_count_inflation_factor = 1.175` on Opus 4.7. Both new rows should carry it.

---

## 2. D1: pricing rows

### Decision

Add two `[[providers.anthropic.models."<id>"]]` array-of-tables rows to `pricing.toml`, sourced from §1.3. Bump `file_version` 1.0 to 1.1, `file_published` to 2026-06-16. Additive only; the existing time-anchored multi-row machinery handles it with no code change.

No real alternative here; the rates are sourced and the schema is established. The only sub-choice is the cache-write encoding: store the multiplier (`cache_write_5m_multiplier = 1.25`, `cache_write_1h_multiplier = 2.00`) exactly as existing rows do, not absolute per-MTok values. Both new models use the standard 1.25 / 2.00, so this is mechanical.

---

## 3. D2: environmental-factor rows

### D2a: Opus 4.8

**Decision: factors equal to Opus 4.7 (flat), uncertainty 40, carry tokenizer inflation.**

Reasoning: pricing unchanged from 4.7 ($5/$25), which the factor file's own methodology treats as a strong proxy for unchanged per-token compute. Opus 4.8 is a "modest but tangible improvement" (Simon Willison) with no published per-token energy change. Holding flat rather than guessing another efficiency step down is the honest "no evidence of change" position; the +/-40% band covers it.

Proposed values (mirroring `claude-opus-4-7`): `wh_per_mtok_input = 360`, `output = 1800`, `cache_read = 36`, `cw_5m = 450`, `cw_1h = 720`, `uncertainty_range_pct = 40`, `tokenizer_token_count_inflation_factor = 1.175`, `valid_from = 2026-05-28`.

### D2b: Fable 5 (the load-bearing uncertain decision)

Fable has **zero anchor data**: it existed 3 days, so there is no Couch-style analysis, no Jegham benchmark, no first-party disclosure. It is a novel tier above Opus (2x price, "Mythos-class"). Three options:

**Option A: pricing-as-proxy = 2x Opus 4.8.** Apply the same pricing-proxy heuristic the file already uses for Haiku (Haiku = Sonnet / 3 because priced ~1/3). Fable priced 2x Opus to 2x Opus energy. Values: input 720, output 3600, cache_read 72, cw_5m 900, cw_1h 1440, `uncertainty_range_pct = 55` (wider than Opus 4.7's 40: zero anchor, novel tier, and pricing-proxy is weaker projecting UP-tier than Haiku's down-tier). Heavily flagged as estimate.

**Option B: Opus-class estimate = equal to Opus 4.8.** Treat the 2x price as positioning/scarcity for a flagship, not 2x compute; assume Fable serves at roughly Opus-tier energy. Narrower deviation from a known anchor, but understates if Fable is genuinely a larger model.

**Option C: null energy factors (pricing only).** Leave `wh_per_mtok_*` null, like the file already does for genuinely-unknown values (Gemini/OpenAI cache fields). Fable then shows token counts + counterfactual cost, but energy/CO2e/water render the missing-value placeholder (an em-dash) via the existing `modelsWithoutFactors` path. Most conservative; publishes no fabricated energy number for a model with zero anchor and no path to ever get one (it is pulled).

**Recommendation: Option A**, as the methodologically-consistent choice (the file populates wide-band estimates rather than nulling whenever a derivation path exists, and pricing-as-proxy is an established path here). The +/-55% band plus an explicit "pricing-proxy, zero anchor, model pulled after 3 days, not expected to be re-measurable" note keeps it honest. **If you would rather publish no energy number than a 2x-pricing-proxy guess, Option C is the clean fallback** and I will not argue hard against it given the genuinely-zero anchor. This is the decision I most want your call on.

Bump factors `file_version` 0.3 to 0.4, `file_published` to 2026-06-16.

---

## 4. D3: valid_from dating

**Decision: sourced launch dates.** Fable `valid_from = 2026-06-09`, Opus 4.8 `valid_from = 2026-05-28`, each with a `launch_date_source` citing the announcement URLs. Both are <= their earliest observed event (Fable 2026-06-09 = first event; Opus 4.8 2026-05-28 <= first event 2026-05-29), so the `audit pricing-launch-dates` gate stays green (zero pre-launch events). No conservative-estimate (D3-from-v0.1.13) treatment needed since both dates are sourced.

---

## 5. D4: walker subagent ingest

### Decision

Change `walk_claude_code_root` to recursively collect `*.jsonl` within each project directory, not just direct children, so `<project>/<session-id>/subagents/agent-*.jsonl` is ingested. Preserve the existing rule that stray `*.jsonl` directly at the root level is skipped (the `walker_skips_files_at_root_level` test contract).

### Reasoning + sub-decisions

- **Why recurse, not special-case `subagents/`:** a general "all *.jsonl under a project dir" walk is simpler and catches future nesting. Observed non-jsonl artifacts under session dirs (`tool-results/*.pdf`, `memory/*.md`) are not `.jsonl`, so a *.jsonl filter stays clean.
- **Dedup safety:** subagent events carry their own uuid / request_id; the existing `(source, uuid)` and `(source, request_id)` partial-unique indexes dedup any overlap. INSERT OR IGNORE posture is unchanged.
- **This shifts every model's historical totals.** Subagent token spend is real account usage that prior versions silently omitted; counting it is the correct behavior, but the dashboard's numbers for ALL models will jump on the next scan. This is the riskiest part of the release and the primary §9 smoke target. Must verify totals move up in the expected direction with no double-counting.
- **Forward + backfill:** new subagent events ingest going forward; historical subagent files ingest on the next scan (they are "new" files the walker had never recorded in `_ingest_file_state`), so no `--rebuild` is strictly required, though `--rebuild` remains the clean re-derivation path.

Tests: add a walker case with a `<project>/<session>/subagents/agent-x.jsonl` fixture asserting it is found; keep the root-level-skip test green.

---

## 6. D5: drift-detector tolerance for a pulled model

The `pricing-drift-check` workflow fetches the live pricing page and compares to `pricing.toml`. Fable is still on the page now, so the detector would match today. But Anthropic may delist a pulled model later, at which point the detector would see "Fable in pricing.toml, absent from page" and could false-alarm.

**Decision: add a `retired` / `status` marker to the Fable row and teach the detector to treat a `pricing.toml` model absent from the live page as expected (skip, not drift) when so marked.** Scope sub-choice:

- **Option A (recommended): minimal now.** Add a `status = "retired"` field to the Fable row (TOML-only provenance, like `launch_date_source`) and a one-line detector guard: a model marked retired that is missing from the page is logged, not flagged. Small, lands with this release.
- **Option B: defer.** File an issue; do not touch the detector now (Fable is still on the page, so no immediate false-alarm). Risk: a silent future cry-wolf if Fable is delisted before the issue is worked.

Recommend A: the detector's whole value is that its alarms are trustworthy (the v0.1.14 "cry-wolf kills trust" lesson), and a pulled-but-retained model is a foreseeable false-alarm we can cheaply prevent now.

---

## 7. Forward-only / data-sync posture

`pricing.toml` and `environmental-factors.toml` are replace-on-startup synced into their DB tables (not forward-only migrations): edit the file, restart, the next sync rewrites the table. So the pricing/factor additions need no migration and no three-places forward-only statement (there is no schema change). The walker change is behavioral, not schema; its "backfill" is simply the next scan. The only forward-only-style note worth stating: subagent history ingests on next scan, and `--rebuild` is the clean re-derivation path (and now correctly wipes all five CC tables per the v0.1.18 Issue #6 fix).

---

## 8. Release-gate framing (carries forward verbatim from Phase 1.5 §7 / Phase 2 §9)

Smoke against the maintainer's real DB is a release gate. The historical pattern (v0.1.13 to v0.1.18) is bug-find-on-smoke every time; the release does not tag until the bug is found and fixed, or the hunt is documented exhaustive.

Smoke checklist for this release:
- Re-scan with the new walker; confirm subagent `*.jsonl` now ingested (files_seen jumps; Fable subagent events appear; total event count rises across models).
- Confirm Fable 5 and Opus 4.8 are now priced and factored: they drop out of `modelsWithoutPricing` / `modelsWithoutFactors`; `audit pricing-launch-dates` shows zero pre-launch events and the unpriced set shrinks to just `<synthetic>`.
- Spot-check the Fable counterfactual cost (rough hand-calc: ~1.49B cache-read x $1 + 27.3M cw1h x $20 + 3.1M output x $50 + 1.10M input x $10 ~= $2,200 top-level, more once subagents ingest). Confirm the dashboard renders a sane Fable cost and a banded Fable impact (or the missing-value placeholder if D2b Option C).
- Confirm no model's cost/impact aggregation double-counts after the walker change.
- Confirm existing models' figures are unchanged except for the expected subagent increase.

Expect a bug. Find it before tag.

---

## 9. Open questions for the maintainer (sign-off gate)

1. **D2b (Fable energy factor):** Option A (pricing-proxy 2x Opus, +/-55%, flagged) vs Option C (null, pricing-only). I recommend A; C is the clean conservative fallback. Your call is the one that most shapes the release.
2. **D5 (drift detector):** add the `retired` marker + detector guard now (Option A) or defer to an issue (Option B). I recommend A.
3. Everything else (D1 pricing rows, D2a Opus 4.8 = 4.7, D3 sourced dates, D4 recursive walker) I will proceed with as specified unless you flag.

On sign-off I will build in this order: pricing rows + factor rows (data, low-risk) to land Fable/Opus pricing first; then the walker change + tests (the totals-shifting piece); then the drift-detector guard; then smoke; then CHANGELOG + v0.1.19 tag.

---

## 11. Smoke findings (v0.1.19 §8 gate)

The §8 gate delivered its expected bug-find. Recap:

- **Subagent ingest worked:** files_seen 60 to 264; total events 36,910 to 42,899; Fable rose 3,965 to 6,026 as subagent Fable usage landed.
- **Fable 5 and Opus 4.8 priced + factored:** both drop out of `modelsWithoutPricing` and `modelsWithoutFactors`; `audit pricing-launch-dates` shows zero pre-launch events for both.
- **Bug-find (dated Haiku ID):** subagent ingest surfaced Haiku 4.5 usage under the dated Bedrock-style ID `claude-haiku-4-5-20251001` (1,202 events), which did not match the assumed-shortened `claude-haiku-4-5` key on the pricing AND factor rows, so the usage rendered unpriced AND unfactored. Root cause: the rows were keyed on a convenience-shortened ID that real usage never emits (an assumed-shape error, the empirical-before-speculative lesson applied to model-ID keying). Fix per maintainer (sign-off): added explicit `claude-haiku-4-5-20251001` alias rows (identical rates/factors) to both files. General model-ID normalization is the proper fix, tracked in [Issue #7](https://github.com/RobarePruyn/tokenscale/issues/7).
- **Principle reinforced (maintainer):** unpriced is not unfactored. Environmental impact must be captured for every model actually used, for every Anthropic model ever released, regardless of billability. The one legitimate exception is `<synthetic>` (admin-API aggregate sentinel, not a real model).
- **Non-issue investigated:** a bare `sonnet` string (7 raw mentions) is an `Agent` tool-call argument (`input.model`), not a usage event. The subagent that runs on Sonnet records its own usage under the resolved ID (`claude-sonnet-4-6`), which ingests and factors normally. No impact lost.
- **Post-fix verification:** live server `/usage/daily` over the full window reports `modelsWithoutPricing: []` and `modelsWithoutFactors: []`; every model in the corpus is now both priced and factored.
