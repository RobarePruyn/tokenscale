# Roadmap: full model coverage, every Anthropic model ever released

**Status**: SIGNED OFF 2026-08-02. All nine D-decisions confirmed as recommended. Maintainer calls on the open judgment items: one release (v0.1.20 carries code, migration, full backfill, and frontend together); historical energy-factor derivation approach accepted for the build with a separate research pass tracked as a filed issue; build-time row-by-row source verification accepted; Issue #7 closes when D1 ships. Build complete; release-gate smoke against the real DB passed clean (see §5). Awaiting maintainer clearance for the v0.1.20 tag.

**Companions**: `docs/assessment-full-codebase-2026-06.md` (codebase audit; its §8 is the code-side work list), `docs/roadmap-model-additions-fable-opus48.md` (v0.1.19 predecessor), [Issue #7](https://github.com/RobarePruyn/tokenscale/issues/7) (model-ID normalization, subsumed by D1).

**Maintainer principle (load-bearing)**: unpriced is not unfactored. Environmental impact must be captured for every model actually used, for every Anthropic model ever released, regardless of billability. Coverage keys on the model-ID strings real data emits.

---

## 1. The verified model universe (research complete)

Primary sources: Anthropic model-deprecations page, models-overview page, live pricing page (re-fetched 2026-08-02), Anthropic's official November 2023 Model Pricing PDF (www-cdn.anthropic.com, saved locally during research), Anthropic launch announcements, and the hidekazu-konishi release-timeline compilation cross-checked against dated news coverage (TechCrunch, Axios, Simon Willison). Every rate below traces to one of these; rows marked ESTIMATE are the labeled exceptions.

### 1.1 Complete lifecycle and rate table

Launch = date the model became available (ID-encoded snapshot date used when earlier, per the bias-earlier D3 dating rule). Rates are base input/output USD per MTok; cache columns follow the standard 0.1x / 1.25x / 2.0x multipliers in every era where prompt caching existed (caching launched in beta 2024-08-14).

| API model ID | Launch (valid_from) | In/Out $ | Rate provenance | Deprecated | Retired |
|---|---|---|---|---|---|
| claude-opus-5 | 2026-07-24 | 5 / 25 | live pricing page 2026-08-02; GA all platforms 2026-07-24 (Anthropic/Axios/TechCrunch) | N/A | active; NST 2027-07-24 |
| claude-sonnet-5 (intro row) | 2026-06-30 | 2 / 10 | live page: "through August 31, 2026" row | N/A | active; NST 2027-06-30 |
| claude-sonnet-5 (standard row) | 2026-09-01 | 3 / 15 | live page: "starting September 1, 2026" row | N/A | same |
| claude-fable-5 | 2026-06-09 | 10 / 50 | live page (v0.1.19, re-verified 2026-08-02) | N/A | ACTIVE, restored; NST 2027-06-09 (see 1.4) |
| claude-mythos-5 | 2026-06-09 | 10 / 50 | live page (limited availability) | N/A | active (Glasswing); restored with Fable |
| claude-mythos-preview | 2026-04-07 | ESTIMATE 10 / 50 | Glasswing launch 2026-04-07 sourced; rates never published, Fable-family ESTIMATE | deprecated (page, 2026-08-02) | retirement TBD; the scheduled 2026-06-30 retirement did NOT occur |
| claude-opus-4-8 | 2026-05-28 | 5 / 25 | v0.1.19 sourced | N/A | active |
| claude-opus-4-7 | 2026-04-16 | 5 / 25 | v0.1.13 sourced | N/A | active |
| claude-sonnet-4-6 | 2026-02-17 | 3 / 15 | launch date now sourced (was conservative 2025-09-01; correction, see D8) | N/A | active |
| claude-opus-4-6 | 2026-02-05 | 5 / 25 | launch date now sourced (TechCrunch 2026-02-05; was conservative 2025-09-01; correction, see D8) | N/A | active |
| claude-opus-4-5-20251101 | 2025-11-01 | 5 / 25 | live page; ID-encoded date (announced 2025-11-24, ID earlier, bias earlier) | N/A | active |
| claude-haiku-4-5-20251001 | 2025-10-01 | 1 / 5 | v0.1.13 sourced; live page | N/A | active |
| claude-sonnet-4-5-20250929 | 2025-09-29 | 3 / 15 | live page; ID-encoded date | N/A | active |
| claude-opus-4-1-20250805 | 2025-08-05 | 15 / 75 | live page (deprecated) | 2026-06-05 | 2026-08-05 |
| claude-opus-4-20250514 | 2025-05-14 | 15 / 75 | live page (retired-except-cloud) | 2026-04-14 | 2026-06-15 |
| claude-sonnet-4-20250514 | 2025-05-14 | 3 / 15 | live page (retired-except-cloud) | 2026-04-14 | 2026-06-15 |
| claude-3-7-sonnet-20250219 | 2025-02-19 | 3 / 15 | announced 2025-02-24, ID earlier; verify rate against launch post at build | 2025-10-28 | 2026-02-19 |
| claude-3-5-haiku-20241022 (launch row) | 2024-10-22 | 1 / 5 | Anthropic 3-5-models announcement; the launch rate | 2025-12-19 | 2026-02-19 |
| claude-3-5-haiku-20241022 (price-cut row) | 2024-12-03 | 0.80 / 4 | Anthropic price revision 2024-12-03; live page still shows 0.80/4 | same | same |
| claude-3-5-sonnet-20241022 | 2024-10-22 | 3 / 15 | verify against launch post at build | 2025-08-13 | 2025-10-28 |
| claude-3-5-sonnet-20240620 | 2024-06-20 | 3 / 15 | verify against launch post at build | 2025-08-13 | 2025-10-28 |
| claude-3-opus-20240229 | 2024-02-29 | 15 / 75 | Claude 3 family launch coverage; verify at build | 2025-06-30 | 2026-01-05 |
| claude-3-sonnet-20240229 | 2024-02-29 | 3 / 15 | verify against family launch post at build | 2025-01-21 | 2025-07-21 |
| claude-3-haiku-20240307 | 2024-03-07 | 0.25 / 1.25 | family launch coverage; verify at build | 2026-02-19 | 2026-04-20 |
| claude-2.1 | 2023-11-21 | 8 / 24 | PRIMARY: Anthropic Model Pricing PDF (Nov 2023) | 2025-01-21 | 2025-07-21 |
| claude-2.0 | 2023-07-11 | 8 / 24 | PRIMARY: Anthropic Model Pricing PDF (Nov 2023) | 2025-01-21 | 2025-07-21 |
| claude-instant-1.2 (and 1.0/1.1) | 2023-03-14 | 1.63 / 5.51 | PRIMARY: Anthropic Model Pricing PDF (Nov 2023) | 2024-09-04 | 2024-11-06 |
| claude-1.0 / 1.1 / 1.2 / 1.3 | 2023-03-14 (1.3: 2023-05-11) | ESTIMATE 8 / 24 | no primary source survives; see D8a | 2024-09-04 | 2024-11-06 |
| `<synthetic>` | sentinel, not a model | n/a | n/a | n/a | n/a |

Two genuine mid-life price changes exist in the whole history: Claude 3.5 Haiku (1/5 down to 0.80/4 on 2024-12-03) and Claude Sonnet 5 (intro 2/10 up to 3/15 on 2026-09-01, pre-announced). Both are expressed as second valid_from rows; both exercise the v0.1.13 multi-row machinery.

### 1.2 Structural facts driving D1 (normalization)

Per Anthropic's model docs: every model ID is a pinned snapshot; 4.6-generation and later IDs are dateless but pinned; before 4.6 the undated names are ALIASES that resolve to dated IDs, so real emitted usage for pre-4.6 models carries the dated form (empirically confirmed: the corpus emits dated `claude-haiku-4-5-20251001`, dateless `claude-opus-4-8`). Canonical row keys are therefore: dated IDs for pre-4.6 models, dateless IDs for 4.6+, and the alias layer handles undated/`-latest` variants.

### 1.3 Remaining build-time verifications (not blockers to sign-off)

1. Fetch the Claude 3 family, Claude 3.5 Sonnet (June + Oct 2024), and Claude 3.7 Sonnet launch posts to primary-confirm the 3/15 and 0.25/1.25 and 15/75 rates currently sourced from dated secondary coverage. Row lands only after its source URL is in hand (house rule).
2. Extract per-model Claude energy anchors from Jegham et al. (arXiv:2505.09598) for the 3-era models it covers directly; docs/sources.md already frames this (older generations are direct evidence). Era-scaled estimates with wide bands for 2.x/1.x/Instant.
3. Enumerate pre-4.6 `-latest` aliases from archived docs for the D1 alias table (claude-3-5-sonnet-latest and kin).

### 1.4 Fable 5 restoration (supplemental research, 2026-08-02)

Fable 5 is **permanently back**. The US Commerce Department lifted the export controls on Fable 5 and Mythos 5 on 2026-06-30 (CNBC; Anthropic's "Redeploying Claude Fable 5" post); Anthropic restored global access on 2026-07-01 across the Claude Platform, claude.ai, Claude Code, and Cowork, an 18-day suspension in total (2026-06-12 to 2026-07-01). The trigger was a jailbreak surfaced during Amazon security testing; Anthropic shipped a new safety classifier as part of redeployment. The deprecations page now carries a normal lifecycle row: claude-fable-5, Active, tentative retirement not sooner than 2027-06-09.

Build implications:
1. **Remove `status = "retired"` from the Fable pricing row** (with a comment preserving the suspension window as provenance history). The rates and `valid_from = 2026-06-09` are unchanged; no corrections-log entry needed since no number moves.
2. **The detector retired-skip guard keeps zero active users.** The mechanism, its tests, and the `load_retired_model_ids` plumbing stay (cheap, proven, and the Fable episode shows pulls really happen), but no row carries the marker after this release.
3. **Soften the Fable factor-row note**: "no anchor is expected (model pulled)" is no longer true; with Fable live again, Couch-style or Jegham-style measurements may emerge. The pricing-proxy estimate and the wide band stay until one does; the note should say "revisit when a benchmark emerges" without the finality.
4. New Fable usage (including the sessions building this feature) ingests and prices via the existing 2026-06-09 row; the suspension window needs no schema representation since rates never changed.

---

## 2. D-decisions (options, recommendation, reasoning); sign-off gate

### D1. Model-ID normalization (subsumes Issue #7)

Options: (A) keep hand-duplicated alias rows per variant; (B) normalize inside every SQL join predicate; (C) a `model_aliases(raw, canonical)` mapping table synced from a new TOML section, joined once at each resolution site; (D) normalize at ingest (rejected: `content_hash` hashes the model string, so rewriting changes dedupe keys); (E) registered SQL scalar function.

**Recommendation: C.** Canonical rows keyed on the real emitted IDs per §1.2; a small alias table maps undated pre-4.6 forms and `-latest` variants to canonical. Data-driven (adding an alias is a TOML edit), testable, applied uniformly to the seven resolution sites the assessment enumerated (two aggregate joins x two files, two single-row helpers, audit subquery, plus the two in-memory lookups), and display keys stay raw (`events.model` and every GROUP BY untouched). Bonus: the v0.1.19 duplicated dated-Haiku row collapses to one canonical dated row plus one alias entry (`claude-haiku-4-5` maps to `claude-haiku-4-5-20251001`), inverting the current workaround in the direction the empirical data points. Includes the dated-ID-to-canonical regression test the assessment flagged as missing.

### D2. Schema hardening

UNIQUE indexes and the fail-loud duplicate test already landed in the quick pass (migration 20260616000001). Remaining sub-item: the in-memory factors single-row-per-model asymmetry (a factor value cannot change over time for one key in the TOML). **Recommendation: document, do not change.** No historical model needs era-split factors yet; the DB schema already supports multi-row if that changes.

### D3. Pre-caching pricing rows

Models predating prompt caching (beta 2024-08-14): Claude 1.x, Instant, 2.0, 2.1, and the Claude 3 family rows at launch. The pricing schema requires all five rate fields. Options: (A) nominal standard multipliers (cache_read = 0.1x input, 1.25x / 2.0x writes) with an explanatory note; (B) zero placeholders (rejected: div-by-zero class of surprises, and zero is a lie about a price that never existed); (C) make cache fields Optional in the parser (code change, weakens the schema for live models).

**Recommendation: A.** No cache tokens can exist for those eras, so the nominal rates multiply zero and never surface a dollar; the note states plainly that caching did not exist for the model and the values are the standard multipliers applied nominally. Never zero-input rows (guard exists since the quick pass, and the authoring rule stands).

### D4. Drift-detector scope

**Recommendation: page-presence becomes the tracking axis.** TRACKED_MODELS holds only live-pricing-page models (add claude-opus-5 now); historical models are never tracked, get no snapshot or fixture entries, and cannot flip the nightly run to exit-2. The retired-skip mechanism stays (tests and plumbing intact) but has zero active users after Fable's restoration per §1.4; it is the standing defense for the next pull. Two sub-calls:
- **D4a Sonnet 5**: the live page currently renders Sonnet 5 as two qualifier-suffixed rows ("through August 31, 2026" / "starting September 1, 2026") that the row regex will not match. Track claude-sonnet-5 starting 2026-09-01 when the page collapses to a single standard row; until then it is priced-but-untracked with a dated note in TRACKED_MODELS. Zero parser risk during the intro window.
- **D4b**: detector docstring and cost-methodology detector-scope prose updated to the page-listed model.

### D5. `<synthetic>` banner policy

Options: (A) leave it permanently listed in modelsWithoutPricing / modelsWithoutFactors; (B) exclude the sentinel from both banners (it is not a model; no row can ever exist), keep it visible in the audit table's unpriced line.

**Recommendation: B.** The banners exist to signal actionable coverage gaps; a sentinel that can never be covered is permanent noise that trains banner-blindness.

### D6. Mythos rows

**Recommendation: include all three Glasswing-adjacent models.** claude-mythos-5: sourced rates (10/50) and date (2026-06-09), factor row mirrors Fable's pricing-proxy estimate. claude-mythos-preview: sourced launch (2026-04-07, Glasswing announcement), rates never published so the pricing row is a labeled Fable-family ESTIMATE; factor row same proxy with the widest band. Cheap, defensible, and a Glasswing org's CC data would otherwise render unpriced AND unfactored, violating the coverage principle.

### D7. Energy zero-vs-missing asymmetry

`energy_wh` / `facility_wh` COALESCE to 0 while cost / co2e / water go None and render as the missing-value placeholder; an unfactored model silently reads 0 Wh. Options: (A) Option-ize energy like the other impact fields (None when zero events in the cell had a factor row); (B) keep 0 and add a separate missing counter to the API.

**Recommendation: A**, for convention symmetry and honesty; `events_missing_env_factor` already carries the per-cell truth, so the display rule is mechanical. Touches ImpactByBucketRow, the two query files, the API serialization, and the frontend null-render path (which already handles None for co2e/water).

### D8. Dating discipline and corrections

All rows use sourced dates; ID-encoded snapshot dates win when earlier than announcement dates (bias earlier, consistent with the v0.1.13 D3 rule). Two corrections land with this release, each with a corrections-log entry in docs/cost-methodology.md:
- Opus 4.6 valid_from 2025-09-01 corrected to 2026-02-05, Sonnet 4.6 to 2026-02-17 (both now sourced; the old conservative estimates were deliberately early). Zero numeric impact: rates are identical on both sides of the boundary and every observed event postdates the corrected dates, so the audit gate stays green.
- **D8a Claude 1.x rates**: no primary source survives (the Nov 2023 PDF era postdates Claude 1's public card). Options: (A) ESTIMATE rows at Claude 2.0 rates (8/24) with an explicit no-primary-source note; (B) leave 1.x unpriced (factored only). **Recommendation: A**; the estimate is clearly labeled, bounded by adjacent-era primary data, and CC JSONL cannot even contain 1.x events (predates Claude Code), so the practical exposure is hypothetical API imports.

### D9. Frontend at scale

**Recommendation: do the full set now** (build-it-right): extend the display map to version-first IDs (`claude-3-5-sonnet-20241022` renders "Claude Sonnet 3.5", `claude-2.1` renders "Claude 2.1", instant and mythos-preview handled); deterministic client-side model ordering (family then version, newest first) so colors stop depending on window contents; version-aware palette assignment (shade keyed to parsed version, wrap-safe beyond 4 versions per family); cap the missing-data footnotes with a count plus expandable list. Legend collapse ("+N more") included unless it fights Recharts; if it does, it degrades to the capped-footnote treatment and a note.

---

## 3. Build plan after sign-off

1. D1 alias table (migration + sync + seven-site join change + regression tests) and D7 energy Option-ization: the two code-heavy pieces, first.
2. Pricing + factor rows for the full §1.1 universe, with the §1.3 build-time verification fetches done row-by-row as they land; seed-marker-safe phrasing throughout; corrections-log entries per D8.
3. D4 detector scope change + docstring/docs updates; D5 banner exclusion; D9 frontend.
4. Full test suite, then the release-gate smoke against the real DB (expect the bug-find; the §1.1 backfill plus D1 is the largest data change in the project's history). CHANGELOG v0.1.20 with the three-place forward-only statement if any migration warrants it (the alias table is additive and replace-on-sync, same posture as pricing).

## 4. Gates carried forward

Release-gate framing verbatim (smoke is the gate; bug-find is the expected outcome). Empirical before speculative (every rate traces to a source or carries a labeled estimate). No em-dashes in repo prose. Explicit sign-off on D1 through D9 before any code or data lands.

## 5. Smoke findings (v0.1.20 §4 release gate)

Smoke ran against an isolated copy of the maintainer's production DB (48,293 events, 752 MB) with the built release binary pointed at the repo's `pricing.toml` v1.2 and `environmental-factors.toml` v0.5 via config overrides. Migrations applied cleanly (the D1 `model_aliases` table created); `serve` startup synced 28 pricing rows / 26 models / 26 aliases and 37 model-factor rows (26 anthropic) with zero conflicts.

Results, all green:

1. **Coverage (D5).** Every real emitted model in the DB (opus-4-7, opus-4-8, fable-5, opus-4-6, haiku-4-5-20251001, sonnet-4-6, opus-5, sonnet-5) resolves to a canonical row and is both priced and factored. The only unpriced and unfactored model is `<synthetic>`, which is the coverage-exempt admin-API aggregate. The unpriced/unfactored sets are exactly `{<synthetic>}`.
2. **No double-counting (D1 + UNIQUE guard).** Non-synthetic raw event count (48,135) equals the time-anchored pricing-join row count (48,135). The alias LEFT JOIN followed by the pricing JOIN produces exactly one row per event; the UNIQUE indexes from the quick pass hold. This was the primary risk of the largest data change in the project's history and it is clean.
3. **Audit gate.** `audit pricing-launch-dates` exits 0: zero pre-launch events across all eight priced pairs. Every model's earliest observed event is at or after its `valid_from` (sonnet-5 `2026-07-02 >= 2026-06-30`; opus-5 `2026-07-25 >= 2026-07-24`; the tightest margin is opus-4-7 at `2026-04-18 >= 2026-04-16`).

**One procedural finding (not a code defect, not a blocker).** `tokenscale scan` does not run the pricing/factor/alias sync; only `serve` startup and the `audit` command do. An initial smoke pass using `scan` therefore showed the copy's pre-existing 7 pricing rows unchanged, which looked like a sync failure until traced to the command path. The behavior is pre-existing and consistent with the "replace-on-startup" contract in the config docs (the dashboard always runs under `serve`, which syncs). It is worth a follow-up decision: should `scan` also re-sync pricing/factors/aliases so a local-research-mode user who edits a TOML and runs a one-shot `scan` sees fresh rates without restarting `serve`? Filed as a candidate, not actioned this release (no sign-off to change command semantics here). The smoke checklist for any future pricing/factor/alias release must exercise the sync via `serve` or `audit`, not `scan`.

Unlike v0.1.13 through v0.1.19, this release's smoke surfaced no code bug. The coverage, double-count, and audit checks that would have caught a D1 resolution error, a fan-out regression, or a launch-date slip all passed against real data. The exhaustive-hunt bar is met: the three checks most likely to expose the D1/D7/backfill changes were run against the full production event set and are clean.
