# Model additions: Claude 5.5 generation and the 5.1 flagships (v0.1.23)

**Status**: SIGNED OFF 2026-10-07 ("Yes to all", D1 through D5 as recommended). Build in progress.

**Trigger**: after the v0.1.22 upgrade, `tokenscale audit pricing-launch-dates` on the maintainer's database showed two models with events and no pricing or factor rows: `claude-opus-5-5` (1,228 events from 2026-09-23) and `claude-fable-5-1` (751 events from 2026-09-09). Per the standing rule (unpriced is not unfactored; every model used gets rows), this pass scoped the whole gap, which turned out to be five models, one live pricing error, and a dead drift detector.

**Companions**: `docs/roadmap-model-additions-fable-opus48.md` (v0.1.19, the previous pass of this kind), `docs/roadmap-full-model-coverage.md` (v0.1.20, D1 normalization and D4 detector scope), `docs/cost-methodology.md` corrections log (Sonnet 5 entry dated 2026-10-07).

## 1. Empirical baseline (primary sources fetched 2026-10-07)

### 1.1 New models on Anthropic's pricing, models-overview, and model-deprecations pages

| Model | API ID | Launch | Input / output USD per MTok | Cache read | Evidence for the date |
|---|---|---|---|---|---|
| Claude Fable 5.1 | `claude-fable-5-1` | 2026-09-01 | 10 / 50 | 0.25 (0.025x) | deprecations-page commitment "not sooner than September 1, 2027"; press of 2026-09-01 |
| Claude Mythos 5.1 | `claude-mythos-5-1` | 2026-09-01 | 10 / 50 | 0.25 (0.025x) | same page; limited availability |
| Claude Opus 5.5 | `claude-opus-5-5` | 2026-09-22 | 4 / 20 | 0.20 (0.05x) | first-party post https://www.anthropic.com/news/claude-opus-5-5; commitment "September 22, 2027" |
| Claude Sonnet 5.5 | `claude-sonnet-5-5` | 2026-09-28 | 2 / 10 | 0.20 | commitment "September 28, 2027"; press of 2026-09-28 |
| Claude Haiku 5.5 | `claude-haiku-5-5` | 2026-10-07 | 0.10 / 0.50 up to 100k-token prompts; 0.50 / 2.50 above | 0.01 / 0.05 | listed on all three first-party pages as of 2026-10-07 with commitment "October 7, 2027"; announcement post not yet indexed |

Every 5-generation model so far carries a retirement commitment of exactly launch plus one year (Fable 5 2026-06-09, Sonnet 5 2026-06-30, Opus 5 2026-07-24), which is why the commitment date is accepted as the launch date where it agrees with press; it never disagreed.

Cache-read pricing is no longer a uniform 0.1x of input (footnotes 1 and 2 on the pricing page). The file stores cache read as an absolute rate, so the schema is unaffected; the drift detector's internal-consistency check is not.

### 1.2 Sonnet 5 pricing correction

The pricing page footnote (3) now reads: the 2/10 introductory price "is now the standard price. The previously scheduled increase to $3/$15 ... on September 1, 2026 will not occur." `pricing.toml` v1.2 had pre-encoded that increase as a second time-anchored row from 2026-09-01. Every Sonnet 5 event since then was priced 1.5x too high. The maintainer's database has no Sonnet 5 events after 2026-07-03, so no displayed number moved for them; other users could have been affected for five weeks. Row removed in v1.3; corrections-log entry in `docs/cost-methodology.md`.

### 1.3 Drift detector state

GitHub set the nightly `pricing-drift-check` schedule to `disabled_inactivity`; the last run was 2026-07-28. Independently, Sonnet 5 was never in `TRACKED_MODELS` (the v0.1.20 D4a deferral because of its two-row introductory pricing), so a live detector would not have caught 1.2 either. The detector also assumed a single 0.1x cache-read multiplier for every model, which Fable 5.1, Mythos 5.1, and Opus 5.5 now break, and the live page's footnote markers (`<sup>n</sup>`) sit inside table cells the parser reads.

### 1.4 Lifecycle changes since v0.1.21

Sonnet 4.5 deprecated 2026-09-30, retirement 2026-11-30, replacement Sonnet 5.5. Opus 4.1 retired 2026-08-05 (already recorded). Fable 5, Opus 5, Sonnet 5, Opus 4.8, Opus 4.7, Opus 4.6, Opus 4.5, Sonnet 4.6, Haiku 4.5 all Active with one-year commitments.

## 2. Decisions (all signed off as recommended, 2026-10-07)

### D1. Pricing rows

Add the five models with rates and dates from 1.1, each `source_url` the pricing page, `source_accessed_at` 2026-10-07. Remove the Sonnet 5 2026-09-01 row and log the correction. Haiku 5.5 ships as its under-100k-token tier, labeled PARTIAL RATE CARD in the row notes, with the over-100k tier quoted; tier-aware pricing is Issue #11 (we record per-event input tokens, so it is implementable, but it is a schema change and does not belong in a data pass). Sonnet 4.5 gets a lifecycle note.

### D2. Environmental-factor rows (no measurement exists for any of them)

- Fable 5.1 and Mythos 5.1: held flat versus Fable 5 (720 / 3,600 Wh per MTok in/out), band 55. Same rate card; the cache-read cut is a caching-infrastructure price, not per-token compute.
- Opus 5.5: 0.8x Opus 5 (288 / 1,440), band 45. Anthropic's post frames it as efficiency ("costs 40% less to run than Opus 5", output "more than 30% faster") alongside a 20% price cut; the file's Opus 4.5 precedent treats an efficiency-framed price cut as a compute signal. The 20% price move is applied, not the 40% workload figure (which includes the cache-read cut).
- Sonnet 5.5: held flat versus Sonnet 5 (195 / 965), band 40. Same price; the 30% speed claim is latency, not energy.
- Haiku 5.5: held flat versus Haiku 4.5 (70 / 330), band 55, despite a 10x lower under-100k price. Sweep #3 found the Haiku price proxy had underestimated 3.5 Haiku's output energy about 5x against Jegham et al.'s measurement, and a prompt-length-tiered price is a positioning signal. The alternative (0.5x) was considered and rejected; the row says so and the band records the risk of overstating.

### D3. Drift detector

Re-enable the schedule. Add per-model cache-read multiplier overrides (0.025 for Fable 5.1 and Mythos 5.1, 0.05 for Opus 5.5) to the internal-consistency check. Strip footnote markers from table cells before matching. Tighten the display-name boundary so "Claude Opus 5" cannot anchor on "Claude Opus 5.5". Track the five new models plus Sonnet 5 (single-rate now, so the D4a reason is gone); Haiku 5.5 matches its under-100k row by page order. Re-capture `pricing-rate-card.snapshot.json` and the test fixture against the 2026-10-07 page.

### D4. Lifecycle notes

Sonnet 4.5 deprecation recorded on its pricing row. No numeric change.

### D5. Packaging

One release, v0.1.23: `pricing.toml` 1.2 to 1.3, `environmental-factors.toml` 0.6 to 0.7, detector and fixtures, this document, corrections log, research-log entry for the factor rows, CHANGELOG. No schema migration. Release-gate smoke via `serve` and `audit` against a copy of the production database (the sync runs on those paths, not `scan`); expected outcome: opus-5-5 and fable-5-1 move into the priced and factored set, audit stays at zero pre-launch events (first events 2026-09-23 and 2026-09-09 against launches 2026-09-22 and 2026-09-01). Tag held for maintainer clearance. The tap formula still needs the manual publish step until `HOMEBREW_TAP_TOKEN` is rotated (Issue #10).

## 3. Gates carried forward

Every rate traces to a source or carries a labeled estimate (all five are sourced; all five factor rows are labeled ESTIMATE). No em-dashes in repo prose. Smoke is the gate.

## 4. Smoke findings (release gate, 2026-10-07)

Against a fresh copy of the maintainer's production database (50,431 events) with the v0.1.23 build pointed at the repo TOMLs via config overrides. Sync via `serve` startup: 32 pricing rows across 31 models, 26 aliases, 42 model-factor rows, 10 grid rows, no conflicts, seed-marker gate clean.

1. **Coverage.** `claude-opus-5-5` (1,240 events) and `claude-fable-5-1` (751) resolve to priced and factored rows; the unpriced and unfactored set is exactly `{<synthetic>}` again.
2. **Sonnet 5 correction landed.** The database carries one `claude-sonnet-5` pricing row (2026-06-30, 2 / 10); the 3 / 15 row is gone.
3. **Audit gate.** `audit pricing-launch-dates` exits 0: ten priced pairs, zero pre-launch events. Tightest margins: Opus 5.5 first seen 2026-09-23 against launch 2026-09-22; Fable 5.1 2026-09-09 against 2026-09-01.

**Bug found by the gate, detector side.** Running the detector's own test suite (which CI had not executed since the schedule was disabled in July) exposed three failures. Two were stale tests (a substring filter that now also removed the Fable 5.1 row; an assertion that Fable 5 is still `status = "retired"`, false since v0.1.20). The third was a real latent defect: `load_pricing_toml` looked tracked IDs up as row keys, but since v0.1.20 D1 the Haiku 4.5 row is keyed on its dated ID with `claude-haiku-4-5` as an alias, so every live run would have reported Haiku 4.5 as "missing from pricing.toml". The loader now resolves tracked IDs through the alias table. The bug-find-on-smoke pattern holds; this release's find was in the tooling, not the product.

**Second find, live page.** A dry run against the live page after the parser fixes still exited 2: on 2026-10-07 the rendered HTML became a current-lineup view with a two-level header, a description cell between each model name and its prices, and the columns reordered (input, output, 5m write, 1h write, cache read). The fixed-order regex would have mis-assigned rates silently had the header guard not tripped first. The detector now fetches the page's markdown source (`pricing.md`), which keeps the documented column order and every model; the dry run then exited 0 with all 13 tracked models matching `pricing.toml`.
