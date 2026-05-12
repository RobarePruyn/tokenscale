# How tokenscale's numbers are computed

This page explains exactly how every environmental-impact figure in the dashboard is derived — what formulas, what sources, what assumptions, and where the honest uncertainty lives.

The short version: **the dashboard adopts Google's August 2025 "comprehensive methodology"** (Elsworth et al. 2025, arXiv:2508.15734) as the canonical approach for per-token energy and facility-level multipliers, applies regional grid factors from EPA's eGRID2023, and surfaces uncertainty as a per-value ± band rather than hiding it.

The longer version follows.

---

## What we're computing

For every (event, model, region) triple, the dashboard computes four numbers:

| Symbol | What it means | Unit |
|---|---|---|
| `energy_wh` | Per-event energy at the compute level — active accelerators + host CPU/RAM + idle capacity, but **before** facility overhead | Watt-hours |
| `facility_wh` | Per-event energy at the building level — `energy_wh × PUE` (Power Usage Effectiveness) | Watt-hours |
| `co2e_g` | Per-event carbon dioxide equivalent emissions — `facility_wh × grid_CO2e_kg/kWh` | grams CO₂e |
| `water_l` | Per-event water — `(facility_wh / 1000) × grid_water_L/kWh` (direct DC cooling only in v0.1; indirect is a v0.2 enhancement) | liters |

These are computed per-event using time-anchored factor lookups, then aggregated to (bucket, model) for chart display. See [`decisions.md`](decisions.md) for why per-event and not per-bucket.

---

## Why "comprehensive methodology"

There are two common ways to attribute energy to an LLM inference:

1. **Active-accelerator only.** Count only the GPU/TPU compute time × power draw of those chips. Cleanest, simplest, popular in early literature.
2. **Comprehensive.** Active accelerators + host CPU/RAM + idle capacity (chips that were spun up but not serving the request) + facility overhead (cooling, networking, lights). Per Google's own data, the comprehensive number is **~2.4× larger** than the active-accelerator number.

`tokenscale` uses the comprehensive methodology. The active-only number is an *under*-estimate by a factor that varies with deployment but is consistently substantial. Google publishes 2.4× as their measured ratio at scale; assuming it generalizes to other vendors is a defensible default.

When a source quotes only the active-only figure (some early third-party papers did this), the factor file flags it explicitly — either we inflate by 2.4× with a note, or we set the row to `null` and add it as a research-request entry rather than fabricate a comprehensive estimate from incomplete data.

---

## Per-token energy

The factor file's `wh_per_mtok_*` fields are per-million-token energy figures, broken out by token type:

- `wh_per_mtok_input` — input tokens (prompt + system + history)
- `wh_per_mtok_output` — output tokens (model response)
- `wh_per_mtok_cache_read` — prompt-cache reads (typically ~1/10 of input)
- `wh_per_mtok_cache_write_5m` — 5-minute prompt cache writes
- `wh_per_mtok_cache_write_1h` — 1-hour prompt cache writes

Per-event energy at the compute level:

```
energy_wh = Σ (tokens_of_type × wh_per_mtok_of_type) / 1_000_000
```

A null `wh_per_mtok_*` for any token type contributes zero — the math still produces a partial answer rather than refusing to compute. The dashboard separately surfaces "models without factors" so users know when a column is missing.

### Where the per-model rates come from

Each model row in `environmental-factors.toml` cites a `source_doc` field pointing into [`sources.md`](sources.md). The provenance for v0.1:

- **Anthropic Claude family** — derived from Couch's January 2026 per-token analysis anchored to Google's Gemini disclosure, plus model-version delta estimates based on Anthropic's published "adaptive thinking efficiency" / pricing-as-compute-proxy signals. Confidence: `secondary`.
- **Google Gemini Apps median text prompt** — Elsworth et al. 2025 disclosure, 0.24 Wh per median prompt. Converted to per-token via a documented 1,500-token assumption. Confidence: `primary`.
- **OpenAI** — derived from Sam Altman's June 2025 blog (`~0.34 Wh per ChatGPT query`), Epoch AI's independent estimate, and Jegham et al. 2025 per-long-prompt benchmarks. OpenAI has not published a technical disclosure. Confidence: `secondary`.
- **DeepSeek, Meta LLaMA, Mistral** — derived from Jegham et al.'s peer-reviewed benchmarks (LLaMA, DeepSeek) and Mistral's CO₂e/water disclosure (Mistral). Confidence: `secondary`.

When a vendor publishes a first-party technical disclosure, that row's confidence upgrades to `primary` on the next research sweep. See [`research-cadence.md`](research-cadence.md).

---

## Facility overhead — Power Usage Effectiveness (PUE)

PUE is the ratio of total facility energy to IT (compute) energy. A PUE of 1.15 means for every 1 kWh of compute, 1.15 kWh enters the facility — the extra 0.15 kWh covers cooling, networking, lighting, etc.

```
facility_wh = energy_wh × pue
```

PUE varies by region, datacenter, season, and load. v0.1 uses AWS's published global 2024 PUE of **1.15** as a flat default across all AWS regions, because **AWS does not publish per-region PUE values** (only global averages and best-in-class anchors like "Americas best site 1.05"). When better data lands — AWS publishing per-region figures, or independent measurement — the factor file gets refined.

If the configured region's PUE is null in the factor file, the application falls back to the `[defaults].fallback_pue` value (also `1.15` in v0.1). The number of events that used the fallback is surfaced in the per-bucket provenance — users can see how much of their CO₂e attribution rests on the fallback.

---

## Grid CO₂e — eGRID2023 subregions

Grid carbon intensity (`co2e_kg_per_kwh`) is a regional value sourced from **EPA's eGRID2023** summary tables (released September 2025). The factor file carries one row per AWS region we track, with:

- `co2e_kg_per_kwh` — the canonical subregion-level rate
- `egrid_subregion` — the four-letter eGRID code (e.g. `SRVC` for SERC Virginia/Carolina, the subregion covering us-east-1)
- `egrid_subregion_full_name` — human-readable expansion
- `egrid_subregion_co2e_lb_per_mwh` — the raw EPA number, for audit
- `co2e_state_check_*_kg_per_kwh` — state-level cross-reference for sanity-checking

```
co2e_g = facility_wh × co2e_kg_per_kwh
```

(Wh × kg/kWh = kg/1000 = grams, dimensionally consistent.)

### Subregion vs state

We use the **subregion** number as canonical because that's what eGRID's methodology accounts for grid interconnections at, and it's what aligns with how electricity actually flows under datacenter contracts. The state-level number is shown as a cross-reference — for example, Oregon (state) is much cleaner than NWPP (subregion) because the subregion includes Idaho / Montana / Wyoming generation that Oregon-specific load doesn't always pull from. Where the state vs subregion gap is large, we flag it in the row's `notes` field.

### Region attribution is a user-declared assumption

Critically: **Anthropic does not publish which AWS region served any given request.** The dashboard resolves region by *user configuration* (`[inference].default_inference_region`), not by observation. This is an honest declared assumption surfaced in the environmental banner at the top of the dashboard. If your usage is split across regions, the CO₂e number reflects your declared region only.

---

## Water — direct, regional WUE

```
water_l = (facility_wh / 1000) × water_l_per_kwh
```

`water_l_per_kwh` is the **direct WUE** — water consumed at the datacenter for cooling, per kWh of facility energy. v0.1 uses AWS's published global 2024 WUE of **0.15 L/kWh** as a flat default across regions, because (same story as PUE) AWS doesn't publish per-region WUE.

If a region's water value is null, the application falls back to `[defaults].fallback_wue_l_per_kwh` (0.15 in v0.1). If even that is null, the water field for affected events comes back as `null` — the dashboard renders that as "—" rather than fabricating a zero.

### What's NOT in v0.1's water number

**Indirect water** — water consumed by power plants generating the electricity our datacenters draw — is not included in v0.1. For thermoelectric-heavy grids (coal, gas, nuclear), indirect water is often larger than direct water. Adding it is tracked as an open research question; see [`request-for-research.md`](request-for-research.md).

Likewise, **recycled-water credits** (some AWS datacenters use recycled wastewater, which arguably shouldn't count the same as fresh-water draw) are not modeled separately. Loudoun County's 18 AWS datacenters reportedly use recycled wastewater for cooling — material to actual water draw but not yet quantified in our factor model.

---

## Uncertainty — surfaced, not hidden

Every factor row in the file carries a `uncertainty_range_pct` field reflecting the honest ± band on the value:

- **Direct vendor anchors** (Google's Gemini disclosure, Mistral's Le Chat disclosure): typically ±30%, reflecting the underlying methodology paper's own uncertainty.
- **Anchor-derived estimates** (Anthropic Sonnet derived from Couch's analysis derived from Google's Gemini): typically ±35%, the original uncertainty plus a derivation-step bump.
- **Multi-step estimates** (OpenAI derived from Altman's blog × Jegham et al. benchmarks × token-share assumptions): typically ±50%, reflecting compounding uncertainty.
- **Deep extrapolations** (Mistral energy reverse-derived from CO₂e + assumed European grid mix): 60%+.

The dashboard's per-bucket "± X%" badge shows the **widest** model band in the visible cell. Grid uncertainty is currently NOT in the band (only model uncertainty); adding it is tracked as an open research question.

### What "primary" / "secondary" / "superseded" mean

Each row carries a `confidence` tag:

- **`primary`** — vendor disclosure (Mistral) or peer-reviewed technical paper (Elsworth et al., Jegham et al.). The number rests on the vendor's own measurement or an independent measurement that was peer-reviewed.
- **`secondary`** — independent benchmark (third-party analysis), CEO blog (Altman), derivation from another source. Reasonable evidence but not vendor-confirmed at this resolution.
- **`superseded`** — replaced by a newer measurement; kept in the file for audit. Only relevant once we have history; v0.1 has no superseded rows.

---

## What's structurally invisible

The dashboard cannot see, and never will see without external action, usage from:

- **Claude iOS / Android / desktop apps** — sandboxed; no per-app local data accessible to other apps.
- **`claude.ai` web** — server-side at Anthropic; no per-user feed.
- **Equivalents for other providers** — ChatGPT consumer (chatgpt.com, iOS, Mac app), Gemini consumer, etc. Same structural pattern.

The **cost** of those products is captured via imported billing data (Stripe CSV today; Anthropic Admin API once they expose individual-account access). The **token-level detail** is not. The dashboard footnote under the chart names this explicitly so users don't think their data is being silently dropped.

This is not a tokenscale limitation; it's the current state of every major consumer LLM product's privacy model. If/when Anthropic (or any consumer provider) ships a per-user usage feed, integrating it is purely additive — a new ingest source row in the `sources` table and a new ingest crate.

---

## What changes when factors get refreshed

When a research sweep updates `environmental-factors.toml`:

1. The new file ships with the next binary release.
2. On `tokenscale serve` startup, the new file syncs to the `env_factors` and `grid_factors` DB tables. Currently full-replacement; v0.2 will be history-preserving upsert so multiple `valid_from` rows can coexist.
3. The dashboard's per-event lookup resolves each event to whichever factor row was authoritative *at the event's timestamp*. So a v0.2 file landing in Q3 2026 changes nothing about events from Q1 2026 — those still resolve to the row whose `valid_from` covers their date.

This is why the dashboard's numbers move smoothly across factor updates: historical periods stay anchored to the factor data of their time, while ongoing usage gets the latest estimates. See [`decisions.md`](decisions.md)'s "Per-event factor resolution" entry for the long-form rationale.

---

## Source bibliography

Every numeric value in the factor file traces to a source in [`sources.md`](sources.md). That page is the audit trail — every URL, every author, every access date, every confidence tag.

Most relevant primary sources for v0.1:

- [Elsworth et al. 2025 (Google, arXiv:2508.15734)](https://arxiv.org/abs/2508.15734) — the methodological backbone of the per-token energy compute path.
- [Google Aug 2025 blog post](https://blog.google/inside-google/sustainability/...) — companion to Elsworth et al.; sets the anchor numbers.
- [EPA eGRID2023 summary tables](https://www.epa.gov/system/files/documents/2025-06/summary_tables_rev2.pdf) — every CO₂e number in our grid factors.
- [Jegham et al. 2025 (arXiv:2505.09598)](https://arxiv.org/abs/2505.09598) — peer-reviewed per-long-prompt benchmarks for OpenAI / DeepSeek / Meta / Mistral models.
- [Ren et al. 2024 (Communications of the ACM)](https://arxiv.org/abs/2304.03271) — water-footprint methodology distinguishing direct and indirect water. v0.1 uses direct only; v0.2 will add indirect.
- [Couch (Cumulator) Jan 2026 analysis](https://couch.is/cumulator) — derivation source for per-token Anthropic rates.

---

## How to read the dashboard's numbers honestly

A few framing notes to keep in mind when looking at the environmental view:

- **The numbers are estimates within ± bands.** Even the "primary" sources carry their own uncertainty; multi-step derivations stack uncertainty on uncertainty. The dashboard shows the widest model band per cell as a "± X%" badge — that's a floor, not a ceiling, on real uncertainty.
- **Region is your declared assumption.** Unless you happen to know Anthropic's request-routing logic, the CO₂e and water numbers reflect the region you set in `[inference].default_inference_region`. If you're worldwide and don't know where your inference ran, the configured region is a defensible-but-arguable best guess.
- **Comparisons across model versions are tricky.** A new model version might consume fewer Wh per output token but more output tokens per task (because of tokenizer changes — see Opus 4.7's reported 1.0–1.35× ratio). Per-task comparisons across versions need the tokenizer-inflation factor; the factor file carries it where we know it, defaults to 1.0× where we don't.
- **Cost ≠ environmental impact.** The dashboard's "Counterfactual API cost" and "Net value" math is about money, not joules. The two correlate but not 1:1 — a model with cheaper output tokens can still consume more energy per token than a more expensive alternative if it's deployed less efficiently.

---

## Open questions and where this evolves

Things v0.1 does NOT do well, with where to follow up:

- Grid-factor uncertainty bands (currently only model uncertainty in the ± badge).
- Indirect water (power-plant cooling) per Ren et al.
- Anthropic tokenizer-change factor verification (currently using a third-party estimate).
- Non-US AWS region eGRID equivalents (currently us-east-1 / us-east-2 / us-west-2 only).
- Periodic re-verification that Google's comprehensive methodology remains the right canonical choice.

All five are tracked in [`request-for-research.md`](request-for-research.md) with status, why-it-matters, and starting points. The quarterly research sweep cycle (see [`research-cadence.md`](research-cadence.md)) picks them up; pull requests welcome from anyone with credible data to contribute.
