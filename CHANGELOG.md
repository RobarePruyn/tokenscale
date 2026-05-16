# Changelog

Notable changes per release. Format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

Newest releases on top. Unreleased changes accumulate under `## Unreleased`.

---

## v0.1.10 — 2026-05-16

The dashboard-credibility release. Folds in the full meta-review pass against the dashboard UX and the cost-side methodology gap. No methodology numbers change; every value visible to the user becomes more honestly *displayed* (precision matched to uncertainty, brackets making the band the primary cue, framing that doesn't oversell) and the cost side gets its first proper paper trail.

### Changed (dashboard precision + framing)

- **False precision killed across the board.** New `roundToSigFigs(value, sigFigs)` + `sigFigsForUncertainty(pct)` helpers route the environmental formatters through precision-matched-to-band rounding. `499.92 kWh ± 40%` becomes `~500 kWh`, `134.98 kg CO₂e ± 43%` becomes `~130 kg CO₂e`, `74.99 L ± 64%` becomes `~75 L`. Mapping: <5% → 4 sig figs, 5–15% → 3, ≥15% → 2.
- **± badges now show the value bracket.** All three environmental KPIs render as `~rounded (low – high) ± pct%`. Bracket recomputes from the post-rounding value each render, so the indirect-water toggle correctly shows the new bracket when flipped.
- **Counterfactual cost drops cents** via `formatRoundedDollars` — `(approx)` and ".94 cents" were saying the same imprecision twice. Subscription / other-charges cards keep cents (exact from CSV imports).
- **"Net value" → "Estimated savings vs raw API rates"**, with a caveat directly under the headline (`Assumes API list rates, no volume or enterprise discounts.`) — green-tinted positive number no longer reads as guaranteed savings.

### Added (information that should have been visible)

- **Cache-accounting strip** under the financial row. Shows `cache_read / (input + cache_read + cache_write_5m + cache_write_1h)` percentage AND the estimated dollar savings (`cache_read × 0.9 × input_rate`, summed across models). Cache writes are deliberately in the denominator — they're paid-for cache that isn't (yet) amortizing, and including them surfaces a low-amortization pattern as a low percentage. The single most informative top-line metric for Claude Code users.
- **Equation strip** below the financial cards: `$X (counterfactual) − $Y (subs) − $Z (other) = $W (savings)`. Annotates the cards with the underlying arithmetic.
- **Consumer-apps disclaimer lifted ABOVE the environmental cards** (was buried in fine print below the chart). Honest framing comes BEFORE the kWh number, not after.
- **Last-scanned timestamp on the dashboard** (next to the chart title), so users don't have to click into Methodology to check data freshness.
- **Per-option tooltips on the Counting toggle** (Raw / Cost-weighted / Cost (USD)). Hover each for the precise definition.
- **Family-aware chart colors.** Models in the same family (Opus 4.6 + Opus 4.7) now get adjacent shades of one hue; different families get different hues. Opus = blue shades, Sonnet = green shades, Haiku = amber shades.
- **Per-series cost-share % in the chart legend**, regardless of view mode. Legend shows `Sonnet 4.6 — 72% of cost` even when the chart is in Raw view, so users see relative cost contribution at a glance.

### Removed

- **Provider dropdown hidden.** "All providers" implied multi-provider data when only Claude Code is ingested today; the consumer-apps disclaimer already states scope. Re-introduce when a second provider lands; the `providerFilter` state is preserved (hard-set to `'all'`) so the diff to revive it is trivial.

### Added (cost-side credibility)

- **`docs/cost-methodology.md`** — short companion to the environmental methodology page. Documents the four load-bearing cost-side assumptions: (1) API list rates only, (2) current pricing applied retroactively to all historical events (not time-anchored — load-bearing call-out), (3) flat-daily subscription pro-rating, (4) cache reads billed at 10% of input.
- **New "Cost" tab** in the dashboard's Methodology page rendering the above doc. Sits alongside Environmental / Bibliography / Research log / Open questions.
- **"How is this computed? (4 assumptions)" disclosure** under the financial row. Inline summary of the four assumptions + link to the full doc on GitHub and to the in-app Cost tab. Brand-protection placement: visible on the dashboard, not buried two clicks away.

### Research

- **"Cost-side time-anchoring + Cost Methodology audit trail"** added as an open item in [`request-for-research.md`](docs/request-for-research.md), with a **hard trigger**: must land before the next time Anthropic changes any model's per-token price. Today, `PricingFile::lookup()` ignores `valid_from` and applies the current rate to every historical event. The moment a price changes, every historical counterfactual silently shifts. The lightweight doc + disclosure in v0.1.10 is a stopgap; the time-anchored lookup is the real fix.

### Roadmap

- **`docs/roadmap-granular-attribution.md`** — captured granular-attribution roadmap prompt (per-project / per-thread reporting → commit-attribution Tiers 1/2/3 → forward instrumentation via post-commit hook). Queued explicitly after this meta-review release + the existing Open queue. First action when picked up: Phase 0 investigation (dedup correctness, ingest coverage, project-key resolution, cwd → git toplevel).

### Notably NOT in this release

- **Cost-side time-anchoring itself** — see the open research entry above. v0.1.10 documents the gap; v0.1.x closes it.
- **Granular attribution** — captured in the roadmap, sequenced after the existing Open queue (PUE / Winget / 6b cost-side / All-view clip).

---

## v0.1.9 — 2026-05-16

The macOS notarization release. Every macOS binary the release pipeline produces is now **signed with the Developer ID Application cert and notarized by Apple** — first launch on a user's Mac no longer triggers the "cannot verify developer" Gatekeeper block dialog, and the verified-developer fingerprint matches across both apple-silicon and intel builds.

> **v0.1.8 was never released** — the initial tag landed without `allow-dirty = ["ci"]` in dist-workspace.toml, so dist's plan-job CI-drift check rejected the manual notarization edit before any artifacts could be built. Fixed in v0.1.9. Skipped tag retained for audit clarity.

### Added

- **"Sign + notarize macOS binaries" step** in `.github/workflows/release.yml` `build-local-artifacts` job. Runs only on `apple-darwin` matrix entries. Decodes six Apple/notarization secrets, imports the Developer ID Application cert into a temporary keychain, signs the `tokenscale` binary with `--options runtime --timestamp`, submits to Apple's notarization service via `xcrun notarytool submit --wait`, repacks the tarball with the signed binary, recomputes the `.sha256` sidecar, and patches `dist-manifest.json` so the Homebrew formula publish job picks up the post-notarization checksum.
- **`RELEASING.md` walkthrough** for retrieving and configuring the six secrets: `APPLE_TEAM_ID`, `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`, `APP_STORE_CONNECT_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_PRIVATE_KEY`.
- **CUSTOMIZATIONS section in `dist-workspace.toml`** inventorying every manual edit to the dist-generated release.yml — currently just the notarization step. Re-running `dist generate --mode=ci` will wipe the step; the inventory tells future-me what to re-apply.

### Why this is a manual workflow edit

Cargo-dist 0.31 doesn't have first-class macOS notarization support — [open feature request](https://github.com/axodotdev/cargo-dist/issues/1121). Rather than fork dist or wait, we splice the sign+notarize cycle into the generated workflow as a self-contained step between `dist build` and the artifact upload. When dist gains native support, the manual step gets dropped and the customization inventory becomes empty again.

### What stapling does and does not do

The notarization ticket lives on Apple's servers — `xcrun stapler` can only attach it to `.app` / `.dmg` / `.pkg` bundles, not bare Mach-O binaries. So tokenscale's CLI doesn't carry an embedded ticket. **First-launch on a user's Mac with network access**: Gatekeeper fetches the ticket from Apple, sees "Verified Developer: Robare Pruyn", and launches. **First-launch offline**: Gatekeeper falls back to checking the code signature only, which still works because we sign with `--timestamp`. Subsequent launches don't re-check.

If we ever need stapled binaries (truly offline first-launch for air-gapped environments), the path is to wrap the binary in a signed-and-notarized `.pkg` installer — a separate distribution channel alongside the existing tarballs.

---

## v0.1.7 — 2026-05-16

The full-water-footprint release. Ships **indirect (off-site, power-plant cooling) water** alongside the existing direct-DC-cooling water — closing the open research item from Sweep #2 and giving users the complete Ren et al. 2024 scope-1 + scope-2 water picture. Opt-in via a new toggle to preserve continuity; existing dashboards default to direct-only.

### Added

- **`indirect_water_l_per_kwh`** field per `[grid_factors.*]` block in `environmental-factors.toml` v0.3, with matching `indirect_water_uncertainty_range_pct`. Per-subregion values:
  - SRVC (us-east-1): **2.39 L/kWh** ±35% — direct quote from Ren et al. 2024 Table 1 Virginia row
  - RFCW (us-east-2): **1.85 L/kWh** ±35% — computed from eGRID 2023 fuel mix × Macknick 2012 recirculating-tower coefficients
  - NWPP (us-west-2): **9.50 L/kWh** ±60% — direct quote from Ren et al. 2024 Washington row (hydro-dominated; wider band reflects reservoir-evaporation methodology dispute)
  - CAMX (reference): **3.20 L/kWh** ±50% — computed via fuel mix × Macknick
- **"Include indirect water" toggle** on the dashboard's Water KPI. Defaults OFF; when checked, the Water stat card switches to "Water (direct + indirect)" and shows the combined total with sum-quadrature uncertainty. Tooltip surfaces the breakdown ("direct X L + indirect Y L per Ren et al. 2024").
- **Indirect water row in the FactorProvenancePanel** alongside the existing direct-water row, with its own ± band and source link.
- **`combineSumUncertaintyPct` helper** for fractional-uncertainty-of-a-sum math. Direct and indirect water are independent (different physical systems — DC cooling vs power-plant cooling), so quadrature of absolute σ is the right combination rule.
- **Migration `20260516000001_grid_factor_indirect_water.sql`** — additive `indirect_water_l_per_kwh` and `indirect_water_uncertainty_range_pct` columns on `grid_factors`. Promotes Sweep #2's per-region values from in-memory snapshot into the schema so per-event compute can carry them through the aggregate path natively.

### Changed

- **`GridFactors` struct, `FactorsProvenance`, and `EnvironmentalImpact`** in `tokenscale-core` gain indirect-water fields. `ModelImpact` payload (`/api/v1/usage/daily`) carries `indirectWaterL` and `indirectWaterUncertaintyPct` alongside the existing `waterL` / `waterUncertaintyPct`.
- **`aggregate_impact_by_bucket` SQL** projects `indirect_water_l_raw` and `grid_indirect_water_uncertainty_pct` per (bucket, model) cell. Same time-anchoring as every other grid factor.
- **`/api/v1/factors/active`** `GridFactorEntry` gains `indirect_water_l_per_kwh`, `indirect_water_uncertainty_range_pct`, and `source_url_indirect_water` so the provenance panel can render the new row.

### Research

- **Sweep #2: indirect water** logged in [`docs/research-log.md`](docs/research-log.md) with full methodology, source corpus (Ren et al. 2024, Macknick 2012 NREL, eGRID 2023 fuel-mix tables), and the per-region computation showing why hydro-heavy NWPP carries the widest band. "Indirect water (power-plant cooling) methodology" moved Open → Resolved in [`docs/request-for-research.md`](docs/request-for-research.md).
- Methodology page section "Water — direct vs indirect" rewritten to describe both scopes, the toggle UX, and the unresolved hydro-attribution dispute.

### Why default to OFF

For SRVC, turning on indirect water jumps reported water from 0.057 L to ~1 L per typical session (a ~17× change). For NWPP it's ~64×. Flipping the default would re-baseline every existing dashboard overnight; opt-in lets users discover the magnitude on their terms and read the methodology before committing. The toggle copy and tooltip educate. A future release may flip the default once users have had a chance to internalize the numbers.

### Notably NOT in this release

- **Hydro attribution refinement.** Macknick's 100%-to-power attribution is contested; literature gives 5×–10× range. We use as-published with widened bands for hydro-heavy regions. A future sweep could refine.
- **Recycled-water credits.** Some AWS datacenters (e.g., Loudoun County) use recycled wastewater; not yet modeled separately from fresh-water draw.

---

## v0.1.6 — 2026-05-15

The per-day-rate chart release. The dashboard's main chart now plots a **per-day rate** instead of per-bucket sums, eliminating the bucket-size-driven peak jumps that made the 1y and All views look like usage suddenly inflated 7× / 30× compared to 30d / 90d. The visual discontinuity at every preset transition is gone, and the chart finally communicates intensity-over-time honestly regardless of zoom.

### Changed

- **Chart y-axis is now a per-day rate.** Each bucket's value is divided by the number of window-days it covers (1 for daily, 7 for full weekly buckets, 28–31 for monthly, and partial counts for the first/last bucket when the user's window clips the bucket calendar — e.g. mid-May yields 15 days for the in-progress May bucket, not 31). Cumulative totals in the stat cards (energy, CO₂e, water, counterfactual cost) are unchanged — only the chart shape is normalized.
- **Chart title reflects the rate semantics**: "Token usage per day · daily" / "Token usage per day · weekly average" / "Token usage per day · monthly average". The cadence suffix tells the user how much each data point is smoothed, since bucket size no longer drives peak height but still drives visible variance.
- **Tooltip values now carry a `/day` suffix** ("1.2B tokens/day", "$45.67/day"). Removes ambiguity at hover.

### Why this matters

A chart titled "Weekly token usage" with bars that are 7× taller than the daily equivalent was visually lying: peak height read as "intensity" but a 7-day sum is mechanically bigger for the same usage rate. The v0.1.1 fix patched 30d ↔ 90d by keeping them both daily; this release fixes the same problem at every preset transition by changing what the chart actually measures. The full audit:

| view | granularity | bucket | prior peak | per-day rate |
|---|---|---|---|---|
| 90d  | daily   | 1 day  | ~1B    | ~1B/day   |
| 1y   | weekly  | 7 days | ~2.8B  | ~400M/day |
| All  | monthly | ~30 d  | ~6B    | ~400M/day |

Same data, comparable peaks across all three views.

### Notably NOT in this release

- **Auto-clipping the All-view to first-event date.** The 2022-12-01 lower bound is intentional (ChatGPT launch — "earliest possible LLM usage"). With rate normalization, the empty leading space no longer distorts the y-axis, so the visual cost is much smaller; we keep the honest absolute timeline.

---

## v0.1.5 — 2026-05-15

The run-it-as-a-service release. No code changes — purely installer- and docs-side polish, but a meaningful UX win: post-install messages now tell every brew / Scoop user exactly how to start the dashboard, and `brew services start tokenscale-cli` works out of the box for set-and-forget background operation.

### Added

- **`def caveats` + `service do` blocks** on the Homebrew formula. After `brew install tokenscale-cli`, users see how to run the dashboard (`tokenscale serve`) or register it as a managed background service (`brew services start tokenscale-cli`) right in the install output. The service block declares `keep_alive true`, so the dashboard auto-restarts on crash and auto-starts on login. Logs land in `$(brew --prefix)/var/log/tokenscale.log`.
- **`notes` block on the Scoop manifest** — same UX on Windows. Post-install message covers `tokenscale serve` and the NSSM recipe for running as a Windows service.
- **README "Running as a service" section** — covers macOS (`brew services`), Linux (`systemd` user unit, with a ready-to-paste unit file), and Windows (NSSM). All three platforms get a recipe that takes less than a minute to set up.

### How it's wired

dist 0.31's Homebrew installer doesn't expose hooks for `caveats` or `service` — they're not in `HomebrewInstallerLayer`. To work around that without losing dist's zero-touch publishing, the Homebrew tap repo (`RobarePruyn/homebrew-tokenscale`) gets its own GitHub Actions workflow (`.github/workflows/amend-formula.yml`) that fires whenever dist commits a new formula version, splices the two blocks in before the closing `end` of the class, and commits the amended formula. Sentinel markers bracket the injected region for idempotent strip-and-replace on re-runs.

### Migration note for existing v0.1.4 users

The formula amendment was applied to the live v0.1.4 formula before this tag, so the simplest path is:

```bash
brew update && brew reinstall tokenscale-cli
brew services start tokenscale-cli
```

(`brew upgrade` would no-op since the binary is unchanged; `reinstall` is what picks up the new caveats + service block.) Then open `http://127.0.0.1:8787` and the dashboard's running without a terminal open.

---

## v0.1.4 — 2026-05-15

The honest-headline release. Combines model and grid uncertainty into a single per-metric `± X%` badge on every environmental KPI — the v0.4 follow-on v0.1.3 explicitly deferred — and propagates grid uncertainty all the way through the DB compute path so per-event impact carries it natively rather than as a render-time afterthought.

### Added

- **Combined `± %` badges in the environmental stat row.** Each KPI now shows its own quadrature-combined uncertainty:
  - **Energy (facility)**: model-factor band only (PUE folds in — no separate band tracked).
  - **CO₂e**: √(model² + grid_co2e²) — e.g. Sonnet 4.6 (±35%) on SRVC (±15% CO₂e) → **±38%**.
  - **Water**: √(model² + grid_water²) — e.g. Sonnet 4.6 (±35%) on any AWS region (±50% water) → **±61%**.
- **`tokenscale_core::combine_uncertainty_pct(model, grid)`** — public quadrature helper used by both the per-event compute path (`compute_impact`) and the bucket-aggregate SQL path (`aggregate_impact_by_bucket::cook`). Single source of truth for the math, so the two redundant compute paths stay in lockstep.
- **`co2eUncertaintyPct`** and **`waterUncertaintyPct`** fields on the daily-endpoint `ModelImpact` payload, alongside the existing `maxUncertaintyPct` (now energy-only). All three are per-(bucket, model) cells; the frontend reduces to max across visible cells for the headline badges.
- **Migration `20260515000001_grid_factor_uncertainty.sql`** — additive `co2e_uncertainty_range_pct` and `water_uncertainty_range_pct` columns on the `grid_factors` table. Both nullable, populated on next factor-file sync. v0.1.3 ran the in-memory snapshot only; v0.1.4 promotes uncertainty into the schema so it can flow through per-event compute.

### Changed

- **`FactorsProvenance`** gains `grid_co2e_uncertainty_pct` and `grid_water_uncertainty_pct` fields for symmetry with `model_factor_uncertainty_pct`. Per-event audit trail now carries every band that fed the combined badge.
- **`aggregate_impact_by_bucket` SQL** projects `MAX(gf.co2e_uncertainty_range_pct)` and `MAX(gf.water_uncertainty_range_pct)` alongside the existing `MAX(ef.uncertainty_range_pct)`. Quadrature runs in Rust (`RawImpactByBucketRow::cook`) so the SQL stays portable.
- **`factors_sync`** propagates the two new uncertainty fields from `environmental-factors.toml` into `grid_factors` on every startup. **`factors_lookup`** reads them back into the canonical `GridFactors` struct so single-row lookups carry them too.

### Research

- **Training-cost amortization** added as an aspirational open question in [`docs/request-for-research.md`](docs/request-for-research.md). Full lifecycle (training + inference) accounting per the Strubell 2019 / Patterson 2021 / Luccioni 2022 framing. Gated on Anthropic publishing Claude training compute or a defensible third-party estimate; the numerator-scope question (final run vs full envelope) is called out explicitly.

### Notably NOT in this release

- **Per-region WUE values from AWS.** Water bands stay at ±50% across all AWS regions until AWS publishes per-region WUE; tracked separately in `request-for-research.md`.
- **PUE uncertainty as a separate band.** Currently folded into the model uncertainty; honest improvement would carry it explicitly through the quadrature.

---

## v0.1.3 — 2026-05-12

Two feature additions + the first quarterly research sweep, all driven by closing the loop on items the v0.1.2 docs flagged as future work.

### Added

- **Per-value factor provenance panel** (methodology page v0.2). A new **"Sources for these numbers"** disclosure below the environmental stat row. When expanded, shows three sections:
  - Methodology identifier + link to the source paper + factor file version.
  - Per-model factor rows for the **models visible in the current chart** — display name, confidence tag, model uncertainty ±%, valid_from, source_doc, expandable notes.
  - The configured region's grid factor row — eGRID subregion, CO₂e/water/PUE values with inline uncertainty bands, link to EPA source.
- **`GET /api/v1/factors/active` endpoint** — reads the in-memory factor-file snapshot and serves every (provider, model) row + every grid row + the configured region + methodology metadata. ~5KB payload, fetched once on mount.
- **Scoop bucket** at [RobarePruyn/scoop-tokenscale](https://github.com/RobarePruyn/scoop-tokenscale) — Windows users can now `scoop bucket add tokenscale https://github.com/RobarePruyn/scoop-tokenscale && scoop install tokenscale`. Hand-maintained because `dist` v0.31 doesn't have native Scoop support yet, but uses Scoop's `autoupdate` block tied to GitHub Releases — `scoop update` propagates new tokenscale releases with zero maintainer action per release.

### Research

- **Sweep #1: grid-factor uncertainty bands** (first quarterly sweep, see [`docs/research-cadence.md`](docs/research-cadence.md)). Pulled 4 years of EPA eGRID CO₂e data (2019/2020/2022/2023) for the four subregions tokenscale tracks, computed YoY variance + std dev, and established honest ± bands per subregion. New `co2e_uncertainty_range_pct` field on each `[grid_factors.*]` block in `environmental-factors.toml`:
  - SRVC: **±15%** · RFCW: **±20%** · NWPP: **±20%** · CAMX: **±20%**
- New `water_uncertainty_range_pct = 50` across all AWS regions, honestly reflecting the gap between AWS's global WUE and any specific datacenter's real water draw (AWS publishes no per-region WUE).
- Factor file `file_version` bumped from `0.1` to `0.2`. Full audit trail in [`docs/research-log.md`](docs/research-log.md).
- "Grid-factor uncertainty bands" moved from Open → Resolved in [`docs/request-for-research.md`](docs/request-for-research.md).

### Changed

- `GridFactors` struct in `tokenscale-core::factors` gains `co2e_uncertainty_range_pct` and `water_uncertainty_range_pct` optional fields. Schema is back-compat — `schema_version` stays at 1.
- `GET /api/v1/factors/active` response includes the new uncertainty fields per grid row; FactorProvenancePanel displays them inline next to the CO₂e and water values.

### Notably NOT in this release

- **Aggregate stat-row `± X%` badge still reflects only model uncertainty.** Combining model + grid uncertainty into the headline badge is a deliberate v0.4 follow-on; v0.3 keeps the decomposition visible (model bands in the stat row, grid bands in the sources panel) so users can see both before we collapse them.
- **Per-event compute math does NOT use grid uncertainty.** The DB schema for `grid_factors` doesn't carry the uncertainty fields yet; per-event impact stays exact (point-estimate) compute. Display-only for now.

---

## v0.1.2 — 2026-05-11

The credibility-deepening release. Ships the **methodology / transparency page** the CHARTER named as required-not-optional from day one — every number on the dashboard now has a one-click trail to its source, methodology, and derivation.

### Added

- **`docs/methodology.md`** — narrative walkthrough of how every environmental number gets computed. Covers Google's comprehensive methodology adoption, per-token energy math, PUE/CO₂e/water formulas, the eGRID subregion vs state distinction, uncertainty surfacing, structurally-invisible consumer surfaces, and how factor refreshes propagate.
- **Methodology page** in the dashboard (`Methodology` nav button in the header). Four tabs:
  - **Methodology** — the new narrative doc.
  - **Bibliography** — renders `docs/sources.md`. Every factor source with confidence tag, access date, and summary.
  - **Research log** — renders `docs/research-log.md`. Audit trail of past sweeps.
  - **Open questions** — renders `docs/request-for-research.md`. What the next sweep should address.
- **`GET /api/v1/docs/<slug>`** endpoint that serves the four bundled markdown docs. Bundled via `include_str!` at compile time so the page works offline. Slug-restricted to the user-facing docs (other repo docs stay out of the API surface).
- **`react-markdown` + `remark-gfm`** for rendering. Hand-rolled `.prose-tokenscale` CSS for typography consistent with the rest of the dashboard.

### Changed

- Header gains a Dashboard / Methodology nav switcher. Pricing-review banner now suppresses on the methodology view (it's dashboard-contextual).

---

## v0.1.1 — 2026-05-11

Polish release on top of v0.1.0. No new features; quality-of-life fixes around the dashboard's wider-window views and the install/setup path now that pre-built binaries are the recommended way in.

### Added

- **`docs/research-cadence.md`** — documents how the environmental-factor model gets refreshed (quarterly default + ad-hoc triggers, what a sweep produces, how it distributes to users, the maintainer review checklist). Names the discipline that keeps factor data credible over time.
- **`docs/request-for-research.md`** — open questions for the next research sweep: grid-factor uncertainty bands, indirect-water methodology, tokenizer-change inflation factor verification, non-US eGRID coverage, methodology-choice re-verification.
- **`CHANGELOG.md`** (this file).

### Changed

- **README leads with `brew install`** and the other pre-built installers; build-from-source flow moved to a "Building from source" section for contributors. Reflects that the project actually ships as a binary now.
- **Auto-granularity thresholds retuned** from 60/365 to 120/730 days. The 90d view now stays on daily buckets (previously crossed into weekly), making 30d → 90d a visually continuous extension rather than a sudden 7× peak jump from bucket-size change.
- **Chart title reflects actual granularity** — "Weekly token usage" / "Monthly cost (USD)" / etc. — instead of always saying "Daily" regardless of bucket size.
- **Subscriptions panel** — divider line above the "Imported subscription charges" section is suppressed when there are no manual entries to separate from. Small visual cleanup.

### Fixed

- **`~/` paths in `claude_code_roots` now expand to the user's home directory.** Configs following the README example (`["~/.claude/projects", "~/.claude-synced/laptop/projects"]`) previously failed silently — TOML stored the paths verbatim, the walker tried to open a directory literally named `~`, and the scan skipped both roots without surfacing an error in the UI. The fix is a small `expand_tilde` helper applied to every configured root.

### Release pipeline

- First release fully validated through the pipeline. `dist` v0.31.0 with the GitHub-build-setup hook that runs `npm ci && npm run build` in the `frontend/` directory before `cargo build`. Homebrew tap auto-published to `RobarePruyn/homebrew-tokenscale`. Two CI bugs surfaced and fixed during v0.1.0 (npm working-directory key was stripped by dist's inlining, and the tap repo needed an initial commit before the publish job could checkout main); both fixed inline.

---

## v0.1.0 — 2026-05-11

Initial public release. The bones of the project are in place: ingest Claude Code session logs, compute environmental impact against versioned factor data, surface everything in a local-first dashboard, and ship as a one-command install on five platforms.

### Added

- **Phase 1: dashboard core**.
  - Claude Code JSONL ingest crate (`tokenscale-ingest-cc`) reading `~/.claude/projects/<project>/<session>.jsonl` with idempotent delta scans keyed by `(mtime_ns, len)` per file and request-id / content-hash dedup at the events table.
  - SQLite schema with versioned migrations, `sources`-table-keyed multi-provider design.
  - axum HTTP server with embedded React + Tailwind + Recharts SPA via `rust-embed`. Endpoints: `/api/v1/health`, `/usage/daily`, `/usage/by-model`, `/projects`, `/sessions/recent`, `/subscriptions`, `/billing/charges`.
  - Dashboard with filters (provider, models, token types, projects, date range), view modes (Raw / Cost-weighted / Cost (USD)), stacked-by-model and stacked-by-token-type chart variants, three KPI stat cards.
  - Subscription tracking — manually-declared subscriptions with date-range pro-rating, "Net value" calculation against counterfactual API cost.
- **Phase 2: environmental impact compute path**.
  - `environmental-factors.toml` v0.1 production file with 17 (provider, model) tuples across Anthropic / Google / OpenAI / DeepSeek / Meta / Mistral plus 3 AWS region grid factors anchored to EPA eGRID2023.
  - Google "comprehensive methodology" (Elsworth et al. 2025, arXiv:2508.15734) as the canonical compute path: per-token energy × PUE → facility energy → CO₂e and water as facility-level multipliers.
  - Per-event, time-anchored factor resolution — each event resolves to the env_factors row whose `valid_from <= occurred_at`. Honest sums across `valid_from` boundaries inside a bucket.
  - `[inference]` config block with `default_inference_region` for grid attribution. Back-compat reads the legacy top-level field with a deprecation warning.
  - Environmental impact block on every daily-endpoint response — energy / facility energy / CO₂e / water plus provenance counters (events using fallback PUE, events missing env_factor). Stat row + environmental banner in the dashboard.
- **Phase 2: billing data ingest via Stripe CSV import**.
  - `billing_charges` table with line-item-level granularity, `(source, external_id)` dedup, idempotent re-imports.
  - Two-step preview/commit endpoints. Preview shows parsed rows with auto-categorization and detects conflicts against manually-declared subscriptions. Commit accepts re-categorized rows + a list of manual subscription IDs to dismiss in the same transaction (avoids double-counting when CSV subscriptions duplicate manual entries).
  - Dashboard import panel — drop a CSV or paste, preview with inline category overrides + conflict resolution, commit.
- **Multi-machine ingest** — `[ingest].claude_code_roots` accepts a list of paths so multiple synced JSONL directories scan in one pass. README documents Syncthing as the recommended cross-platform sync tool.
- **Auto-scan in `serve`** — background tokio task re-runs scan every `scan_interval_seconds` (default 60), so the dashboard refreshes as Claude Code activity lands without manual `tokenscale scan`.
- **Distribution pipeline** — `dist` v0.31.0 generates prebuilt binaries for macOS (Apple Silicon + Intel), Linux (x86_64 + aarch64), and Windows (x86_64) on every tagged release. Three installer types: `curl | sh`, PowerShell, and a Homebrew formula auto-published to a separate tap repo. Full release process documented in `RELEASING.md`.

### Documentation

- `CHARTER.md` — project scope, governance, distribution model, framework-extractable design lens.
- `docs/architecture.md` — high-level system design.
- `docs/decisions.md` — running ADR log; 12 entries covering language choices, schema decisions, ingest model, sqlx offline mode, factor model design, time-anchoring SQL, per-event resolution, `[inference]` config block, `view=` parameter drop, multi-machine sync via Syncthing, manual CSV import as default billing path, and the renamed `cargo-dist` → `dist`.
- `docs/sources.md` — bibliography of every factor source with confidence tags and access dates.
- `docs/research-log.md` — audit trail of the v0.1 factor sweep.
- `docs/data-sources.md` — per-ingest-source documentation; honest acknowledgment of which surfaces are structurally invisible to any external tool (iOS, Android, desktop apps, `claude.ai` web).
- `README.md` with quick-start, install commands, dashboard tour, configuration reference, and multi-machine setup walkthrough.

### Known limitations

- Anthropic Admin API ingest exists in design but isn't built; requires an organization account, which most individual-tier users don't have.
- Consumer chat surfaces (Claude iOS / Android / desktop apps, `claude.ai` web) have no per-user usage feed and are structurally invisible to the dashboard's token-tracking view. Their subscription cost IS captured via billing imports.
- Linux distro packaging (APT / DNF / RPM), Scoop bucket, Winget manifest, and macOS notarization are not yet automated. Prebuilt `.tar.xz` archives serve those platforms in the interim.
