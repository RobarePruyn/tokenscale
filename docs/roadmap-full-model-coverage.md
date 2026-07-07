# Roadmap: full model coverage, every Anthropic model ever released

**Status**: scoping in progress (2026-06-16). §1 model universe is verified against Anthropic's model-deprecations and models-overview pages (both fetched 2026-06-16). §2 research list is open. D* decisions drafted after research completes; sign-off before build. Target release: v0.1.20.

**Companions**: `docs/assessment-full-codebase-2026-06.md` (codebase current-state audit; its §8 is the code-side work list), `docs/roadmap-model-additions-fable-opus48.md` (the v0.1.19 predecessor whose smoke surfaced Issue #7), [Issue #7](https://github.com/RobarePruyn/tokenscale/issues/7) (model-ID normalization, subsumed here).

**Maintainer principle (load-bearing)**: unpriced is not unfactored. Environmental impact must be captured for every model actually used, for every Anthropic model ever released, regardless of billability. Coverage keys on the model-ID strings real data emits.

---

## 1. The verified model universe

Source: Anthropic model-deprecations page (lifecycle table + full deprecation history) and models-overview page, both accessed 2026-06-16. Key structural fact from the models docs: **every model ID is a pinned snapshot**. 4.6-generation and later IDs are dateless but still pinned; before 4.6, the undated names are ALIASES that resolve to dated IDs, so real emitted usage for pre-4.6 models carries the dated form. Launch dates for dated IDs are encoded in the ID itself.

### 1.1 Lifecycle table (first-party API IDs)

| API model ID | State (2026-06-16) | Launch (from ID or sourced) | Deprecated | Retired / tentative |
|---|---|---|---|---|
| claude-fable-5 | Active (listed; access suspended by export-control directive 2026-06-12) | 2026-06-09 | N/A | not announced |
| claude-mythos-5 | Limited availability (Project Glasswing) | 2026-06-09 | N/A | not announced |
| claude-mythos-preview | Limited availability, retiring | unknown (research) | N/A | June 30, 2026 |
| claude-opus-4-8 | Active | 2026-05-28 (sourced, v0.1.19) | N/A | not sooner than 2027-05-28 |
| claude-sonnet-5 | Active | unknown (research; intro pricing ends 2026-08-31) | N/A | not announced |
| claude-opus-4-7 | Active | 2026-04-16 (sourced, v0.1.13) | N/A | not sooner than 2027-04-16 |
| claude-sonnet-4-6 | Active | est. 2025-09-01 (conservative, v0.1.13; page implies ~2026-02-17 window start; research) | N/A | not sooner than 2027-02-17 |
| claude-opus-4-6 | Active | est. 2025-09-01 (conservative, v0.1.13; page implies ~2026-02-05 window start; research) | N/A | not sooner than 2027-02-05 |
| claude-opus-4-5-20251101 | Active | 2025-11-01 (from ID) | N/A | not sooner than 2026-11-24 |
| claude-haiku-4-5-20251001 | Active | 2025-10-01 (from ID) | N/A | not sooner than 2026-10-15 |
| claude-sonnet-4-5-20250929 | Active | 2025-09-29 (from ID) | N/A | not sooner than 2026-09-29 |
| claude-opus-4-1-20250805 | Deprecated | 2025-08-05 (from ID) | 2026-06-05 | 2026-08-05 |
| claude-opus-4-20250514 | Retired | 2025-05-14 (from ID) | 2026-04-14 | 2026-06-15 |
| claude-sonnet-4-20250514 | Retired | 2025-05-14 (from ID) | 2026-04-14 | 2026-06-15 |
| claude-3-7-sonnet-20250219 | Retired | 2025-02-19 (from ID) | 2025-10-28 | 2026-02-19 |
| claude-3-5-haiku-20241022 | Retired | 2024-10-22 (from ID) | 2025-12-19 | 2026-02-19 |
| claude-3-5-sonnet-20241022 | Retired | 2024-10-22 (from ID) | 2025-08-13 | 2025-10-28 |
| claude-3-5-sonnet-20240620 | Retired | 2024-06-20 (from ID) | 2025-08-13 | 2025-10-28 |
| claude-3-haiku-20240307 | Retired | 2024-03-07 (from ID) | 2026-02-19 | 2026-04-20 |
| claude-3-sonnet-20240229 | Retired | 2024-02-29 (from ID) | 2025-01-21 | 2025-07-21 |
| claude-3-opus-20240229 | Retired | 2024-02-29 (from ID) | 2025-06-30 | 2026-01-05 |
| claude-2.1 | Retired | unknown (research; ~late 2023) | 2025-01-21 | 2025-07-21 |
| claude-2.0 | Retired | unknown (research; ~mid 2023) | 2025-01-21 | 2025-07-21 |
| claude-1.0 / 1.1 / 1.2 / 1.3 | Retired | unknown (research; 2023) | 2024-09-04 | 2024-11-06 |
| claude-instant-1.0 / 1.1 / 1.2 | Retired | unknown (research; 2023) | 2024-09-04 | 2024-11-06 |
| `<synthetic>` | sentinel, not a model | n/a | n/a | n/a |

Roughly 25 canonical IDs plus pre-4.6 undated aliases (e.g. claude-3-5-sonnet-latest era aliases; enumeration is a research item). The v0.1.19 corpus emits 7 of these; the remaining rows exist for other deployments' historical data, per the coverage principle.

### 1.2 Current-page rate anchors already in hand (fetched 2026-06-16)

From the live pricing page (base rates, USD/MTok, in/out): Fable 5 and Mythos 5 10/50; Opus 4.8, 4.7, 4.6, 4.5 all 5/25; Opus 4.1 and Opus 4 15/75; Sonnet 4.6, 4.5, 4 all 3/15; Haiku 4.5 1/5; Haiku 3.5 0.80/4. Cache columns present for all, standard 0.1x/1.25x/2.0x multipliers. Models NOT on the current page (rates need archival research): Claude 3 Opus/Sonnet/Haiku, Claude 3.5 Sonnet (both), Claude 3.7 Sonnet, Claude 2.x, Claude 1.x, Instant. Sonnet 5 pricing: 3/15 standard with INTRO pricing 2/10 through 2026-08-31 (models-overview footnote), the first genuine mid-life rate-change event in the coverage set.

### 1.3 New coverage gaps discovered during this scoping (beyond the historical backfill)

1. **claude-sonnet-5 is live and uncovered** (no pricing row, no factor row, not drift-tracked). Its intro pricing means TWO time-anchored rows: valid_from = launch date at 2/10, valid_from = 2026-09-01 at 3/15. This exercises the multi-row machinery for real for the first time and interacts with the drift detector's latest-valid_from comparison (the page table shows 3/15 with a footnote; detector compares the latest row, which post-dates today; needs a D-decision on how intro windows are encoded and checked).
2. **claude-mythos-5 / claude-mythos-preview** are limited-availability but real (an approved-org user's CC data could emit them). Mythos 5 shares Fable rates and launch date. Preview retires 2026-06-30; its rates and launch need research or a documented estimate.
3. Active dated IDs claude-opus-4-5-20251101 and claude-sonnet-4-5-20250929: env-factors already carries UNDATED opus-4-5/sonnet-4-5 keys that real data would never emit (the assessment's key-set reconciliation item); pricing.toml carries neither.

## 2. Open research list (Phase C, before D* drafting completes)

1. Historical base + cache rates, from archival sources (announcement posts, archived pricing pages, reputable third-party rate trackers, cross-checked): Claude 3 Opus / Sonnet / Haiku; Claude 3.5 Sonnet (verify same for both snapshots); Claude 3.5 Haiku INCLUDING its launch-pricing history (launch-price change saga needs primary sourcing); Claude 3.7 Sonnet; Claude 2.0 / 2.1 (and any mid-life price cut); Claude 1.x; Instant 1.x. Rule: no cascaded assumptions; every rate traces to a source URL or is labeled a conservative estimate with rationale.
2. Launch dates not encoded in IDs: Claude 1.x, Instant 1.x, 2.0, 2.1, Sonnet 5, Mythos Preview; tighten Opus 4.6 / Sonnet 4.6 (v0.1.13 conservative estimates vs the deprecation-page windows).
3. Pre-4.6 undated alias enumeration (which -latest / undated aliases existed) and confirmation of what CC JSONL and the Admin API actually emit for pre-4.6 usage (expected: dated IDs; verify against any available historical JSONL).
4. Prompt-caching availability timeline (which models/eras have real cache rates at all) to drive the pre-caching placeholder D-decision.
5. Energy anchors per era: Jegham et al. covers older Claude generations DIRECTLY (per docs/sources.md), so historical factor rows may take direct benchmark anchors rather than extrapolations; extract per-model numbers and reconcile the sources.md framing.
6. Batch/intro/regional pricing variants: confirm out-of-scope (counterfactual uses list rates) except where they ARE the list rate (Sonnet 5 intro window).

## 3. D* decisions to draft after research (placeholders, from the assessment)

- **D1 Model-ID normalization** (Issue #7): alias mapping table vs registered SQL normalize function, applied to resolution keys only; events.model stays raw. Includes the display-name mapping the frontend needs.
- **D2 Schema hardening**: UNIQUE on (provider, model, valid_from) and (region, valid_from); duplicate-fan-out regression test; decide on the factors single-row-per-model asymmetry.
- **D3 Pre-caching row policy**: what pricing rows for models without real cache prices carry in the required cache fields, and the no-zero-input rule (div-by-zero guard or authoring rule).
- **D4 Drift-detector scope**: page-listed tracking axis vs TRACKED_MODELS-for-everything; retired-skip reserved for pulled-but-listed; fixture/snapshot policy for never-listed models; intro-pricing window handling.
- **D5 `<synthetic>` policy**: keep in both banners vs explicit sentinel exclusion.
- **D6 Mythos inclusion**: add rows for mythos-5/preview (cheap, defensible) vs document as out-of-scope.
- **D7 Energy 0-vs-missing asymmetry**: surface unfactored models honestly in energy figures.
- **D8 valid_from dating discipline**: sourced-exact vs D3-conservative labeling for every historical row (carry the v0.1.13 convention); audit-gate greenness guaranteed by bias-earlier rule.
- **D9 Frontend scale**: display names for all IDs, deterministic version-aware coloring, capped/collapsible legends and footnotes.

## 4. Gates carried forward

- Release-gate framing carries verbatim: smoke against the real DB is the gate; the bug-find is the expected outcome; no tag until found-and-fixed or documented-exhaustive.
- Empirical before speculative: no row lands without a source or a labeled conservative estimate; notes phrasing must avoid the seed-marker trap phrases ("assumed unchanged", "seed value", "unverified", "needs_review").
- Every non-trivial choice above gets options + recommendation + reasoning and explicit sign-off before code.
- House style: no em-dashes in any repo prose this workstream produces.
