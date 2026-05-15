# tokenscale — Research Log

This log is the audit trail for every research run that produces or updates the factor model. Each entry captures the question being investigated, the sources consulted, what was found, and what was committed to `environmental-factors.toml`.

Newest entries appear at the top.

---

## 2026-05-12 — Sweep #1: grid-factor uncertainty bands

### Question

`request-for-research.md` flagged that v0.1 reported only model-side uncertainty in the dashboard's `± X%` cell badge. Grid factors (`co2e_kg_per_kwh`, `water_l_per_kwh`, `pue`) carry their own uncertainty — annual EPA methodology variability, secular grid decarbonization between the eGRID year and the event's actual year, and the gap between subregion-average and a specific datacenter's real mix. Establish honest per-subregion bands.

### Methodology

Pulled total-output CO₂e emission rates (lb/MWh) from four eGRID releases — 2019, 2020, 2022, and 2023 — for the four subregions tokenscale tracks (SRVC, RFCW, NWPP, CAMX). eGRID2019 data sourced via [SIMAP](https://unhsimap.org/cmap/resources/electricity2019) in kg/kWh and converted; eGRID2020 and eGRID2022 from EPA's published summary tables PDFs; eGRID2023 from our existing v0.1 anchors. The 2019 source publishes CO₂-only rates rather than CO₂e — added a +0.5% adjustment per typical CH₄+N₂O contribution observed in the 2020-2023 deltas.

Computed for each subregion: arithmetic mean, range (max − min), range as % of mean, and sample standard deviation as % of mean. Range captures the secular drift plus any methodology change; std dev captures the year-to-year noise.

### Source corpus (this cycle)

Primary sources newly added or used:

- EPA eGRID2020 Summary Tables (PDF, January 2022 release): <https://www.epa.gov/system/files/documents/2022-01/egrid2020_summary_tables.pdf>
- EPA eGRID2022 Summary Tables (PDF, January 2024 release): <https://www.epa.gov/system/files/documents/2024-01/egrid2022_summary_tables.pdf>
- SIMAP electricity factors 2019 (Univ. of New Hampshire Sustainability Indicator Management & Analysis Platform): <https://unhsimap.org/cmap/resources/electricity2019>

### Findings — raw data

| Subregion | 2019 CO₂e | 2020 CO₂e | 2022 CO₂e | 2023 CO₂e | Range | Range % | StdDev % |
|---|---|---|---|---|---|---|---|
| SRVC | 678.7 | 626.3 | 625.9 | 596.3 | 82.4 | **13.0%** | 4.7% |
| RFCW | 1072.9 | 990.8 | 1005.9 | 916.1 | 156.8 | **15.7%** | 5.6% |
| NWPP | 718.8 | 603.8 | 605.9 | 635.3 | 115.0 | **17.9%** | 7.3% |
| CAMX | 455.5 | 515.5 | 499.3 | 430.0 | 85.5 | **18.0%** | 7.2% |

(All values in lb CO₂e per MWh. 2019 values are CO₂-only with +0.5% adjustment to approximate CO₂e.)

### What changed in `environmental-factors.toml`

- Bumped `file_version` from `"0.1"` to `"0.2"`. Schema is back-compat (new fields are optional, schema_version stays at 1).
- New `co2e_uncertainty_range_pct` field on each `[grid_factors.*]` block:
  - SRVC: **±15%** (range observed; coastal SE has lowest variance)
  - RFCW: **±20%** (range observed widened for coal-heavy sub-area variance)
  - NWPP: **±20%** (hydro-precipitation-driven variance)
  - CAMX: **±20%** (high renewables share + day-to-day mix variability)
- New `water_uncertainty_range_pct = 50` on every AWS region. AWS publishes only a global WUE figure; specific-datacenter water draw can differ substantially with climate. ±50% honestly reflects the "global-to-regional" application gap.
- Inline comments on each row trace the band back to this sweep's analysis.

### What this changes for users

The `/api/v1/factors/active` endpoint now returns `co2e_uncertainty_range_pct` and `water_uncertainty_range_pct` per grid row. The dashboard's `FactorProvenancePanel` displays these inline next to the CO₂e and water values when the user clicks "Sources for these numbers". The aggregate stat-row `± X%` badge still reflects only model uncertainty — combining model + grid uncertainty into the headline badge is a deliberate v0.3 follow-on so users can see the decomposition before we collapse it.

### Carry-forward

- Adding a fifth+ year of data (eGRID2021, possibly historical pre-2019) would tighten the std-dev estimate. Not blocking; current bands are conservative-enough for v0.2.
- AWS publishing per-region WUE would let us drop the ±50% water band to something defensible. Tracked as ongoing in `request-for-research.md`.
- Indirect water (power-plant cooling per Ren et al.) is still missing. Next sweep target.

### Resolved from `request-for-research.md`

- ✅ "Grid-factor uncertainty bands" — partially resolved. CO₂e bands established; water + PUE bands are honest "global-to-regional gap" placeholders pending per-region vendor disclosure.

---

## 2026-04-28 — v0.1 broadened factor model derivation

### Question

Produce the first production version of `environmental-factors.toml` covering as broad a set of frontier and open-weight LLMs as defensibly possible, plus AWS-region grid factors for the regions hosting Anthropic inference. Replace the placeholder file in the repo with real values where they're defensible, with explicit `null` and uncertainty flagging where they're not.

### Methodology

Adopted Google's "comprehensive methodology" from Elsworth et al. 2025 (arXiv:2508.15734) as the canonical approach: energy = active accelerator + host CPU/RAM + idle capacity + PUE-weighted facility overhead. Carbon and water are facility-level multipliers applied downstream. The choice has two consequences:

1. We do **not** count "active GPU only" estimates as authoritative. Where a third-party number was derived that way, we either inflate by the Google-reported 2.4× ratio with explicit note, or set the value `null` and flag.
2. Per-token energy is the canonical unit. Where a vendor publishes per-prompt energy (Google), we convert using a documented prompt-token assumption (1,500 tokens median) and flag the conversion with a wide uncertainty band.

### Source corpus (this cycle)

Primary sources newly added or used:

- Elsworth et al. 2025, "Measuring the environmental impact of delivering AI at Google Scale," arXiv:2508.15734 (Aug 2025). The technical paper behind Google's Aug 2025 disclosure. Methodology backbone.
- Jegham et al. 2025 v6, "How Hungry is AI?" arXiv:2505.09598. Per-long-prompt energy for ~30 frontier models including LLaMA family, DeepSeek-R1, o3, GPT-4.1 nano, GPT-5.
- Li, Yang, Islam, Ren 2024, "Making AI Less 'Thirsty'," published in Communications of the ACM. Foundational water-footprint methodology distinguishing direct (DC cooling) and indirect (power-plant cooling) water.
- EPA eGRID2023 summary tables (released 2025-09-29). Subregion-level CO₂e emission rates extracted directly: SRVC 596.3 lb/MWh, RFCW 916.1 lb/MWh, NWPP 635.3 lb/MWh, CAMX 430.0 lb/MWh, RFCE 599.2 lb/MWh.
- AWS Sustainability data centers page. Global 2024 PUE 1.15, global 2024 WUE 0.15 L/kWh.
- Microsoft Azure 2024 disclosure: WUE 0.30 L/kWh, new zero-water cooling design.
- Mistral Le Chat per-query environmental disclosure (1.14 gCO₂e + 45 mL water per 400-token response; energy not disclosed).
- Sam Altman blog (June 2025): ChatGPT median ~0.34 Wh.
- Epoch AI: GPT-4o estimated at ~0.3 Wh per query.

Secondary / supporting:

- Couch, "Electricity use of AI coding agents" (Jan 2026) — already in v0.1 as the Anthropic 4.5-class anchor.
- Vellum, Box, Caylent benchmarking analyses for Opus 4.6/4.7 relative-efficiency claims.
- AI Commission, MIT Technology Review for non-disclosure status reporting on OpenAI GPT-5.

Full bibliography in `docs/sources.md`.

### Coverage delivered

`environmental-factors.toml` v0.1 now includes per-token Wh values for 17 distinct (provider, model) tuples across five providers:

- **Anthropic:** Opus 4.5, 4.6, 4.7; Sonnet 4.5, 4.6; Haiku 4.5
- **Google:** Gemini Apps median text prompt (composite — Google does not break out by Gemini Pro/Flash/Flash-Lite individually)
- **OpenAI:** GPT-4o, GPT-5, o3, GPT-4.1 nano
- **DeepSeek:** R1 (DeepSeek-own-infra deployment)
- **Meta (LLaMA):** 3.1 8B, 3.2 1B, 3.2 3B, 3.1 405B
- **Mistral:** Large 2 (Le Chat default)

Plus three AWS region grid_factors blocks (us-east-1, us-east-2, us-west-2) anchored on eGRID2023, plus one reference block (CAMX) for cross-comparison.

### What's anchored vs estimated

**Direct primary anchors (confidence: anchored):**

- Google Gemini 0.24 Wh / 0.03 gCO₂e / 0.26 mL median text prompt — Elsworth et al. 2025
- LLaMA 3.1 8B (0.443 Wh), 3.2 1B (0.552 Wh), 3.2 3B (0.707 Wh), 3.1 405B (25.202 Wh) per long prompt — Jegham v6
- DeepSeek-R1 29.075 Wh per long prompt + 200 mL water + 17 gCO₂e — Jegham v6
- GPT-5 ~18 Wh per medium response, o3 33+ Wh per long prompt — Jegham v6
- GPT-4o ~0.3 Wh per query — Epoch AI + Altman corroboration
- Mistral 1.14 gCO₂e + 45 mL water per 400-token response — Mistral disclosure
- AWS PUE 1.15, WUE 0.15 L/kWh — AWS 2024 disclosure
- eGRID2023 CO₂e per kWh for SRVC, RFCW, NWPP — EPA
- Anthropic 4.5-class per-token rates — Couch (secondary anchor; rests on Google Gemini anchor)

**Estimates (confidence: extrapolated, with explicit derivation notes):**

- Anthropic Opus 4.6, Opus 4.7, Sonnet 4.6 — predecessor-equivalent on per-token energy with relative-efficiency adjustments cited from vendor and third-party blog posts. Pricing is unchanged for all three vs their predecessors, supporting the predecessor-equivalent assumption.
- Anthropic Haiku 4.5 — derived from Sonnet 4.5 / 3 (pricing-as-proxy) cross-checked against Couch's analysis.
- Mistral Large 2 energy — back-derived from Mistral's published CO₂e using assumed European grid intensity. Heavy uncertainty.
- Per-token apportionment of vendor-published per-prompt values — uses documented input/output ratio assumptions (60/40 chat, 40/60 long-prompt, 30/70 reasoning) with explicit notes.
- Output-vs-input per-token energy ratio (4:1) — derived from Couch's analysis and general LLM-serving cost knowledge.

**Genuinely unknown (set to `null`, flagged for next research cycle):**

- Cache write rates for non-Anthropic providers (different cache pricing models, not researched in this cycle)
- AWS per-region WUE breakdowns (AWS does not disclose; flagged)
- AWS per-region PUE breakdowns (AWS publishes global + best-region only; flagged)
- Indirect water (power-plant cooling) per region — Ren et al. methodology supports computing this, but v0.1 only models direct WUE
- Per-model breakdown for Google Gemini variants (Pro vs Flash vs Flash-Lite); Google publishes a composite median only

### Methodology decisions worth recording

1. **Comprehensive over active-only.** Adopted Google's comprehensive methodology as canonical. This means our energy numbers are roughly 2.4× higher than naive "active GPU only" estimates derived from FLOPs or theoretical utilization. Documented in `defaults.methodology` and at `tokenscale-core::compute_impact`.
2. **Per-token rather than per-prompt.** All factor values normalized to Wh per million tokens (Wh/MTok), with token-type breakdown (input, output, cache_read, cache_write_5m, cache_write_1h). Where a primary source publishes per-prompt only, we convert with a documented assumption.
3. **Subregion grid intensity over state intensity.** eGRID subregion is the EPA-recommended grain for grid-interconnection accounting; we use it as canonical. State-level emission rates are recorded as sanity-check cross-references — notable gap for Oregon, where NWPP subregion (0.288 kg/kWh) is significantly higher than OR state (0.166 kg/kWh) due to inclusion of carbon-intensive ID/MT/WY portions.
4. **Estimates use predecessor anchors with relative-efficiency adjustments where published.** Per the user's 2026-04-28 directive: when extrapolating to a model with no direct measurement, search for vendor or third-party relative-efficiency claims and cite them rather than copy-from-predecessor naively. Applied to Opus 4.6/4.7 and Sonnet 4.6.
5. **Uncertainty range is per-value, not file-level.** Each model entry carries `uncertainty_range_pct`. Direct anchors are ±30%; estimates are ±35–60% depending on derivation chain length.

### What we should monitor for the next research cycle

- **Anthropic environmental disclosures.** Continued non-disclosure as of 2026-04-28; check whether anything changes.
- **Jegham et al. v7 or successor.** The paper has already revised through v6; the model cohort and absolute numbers shift between versions. Each cycle should re-pull the latest version and update.
- **Google Gemini per-model breakdown.** The current 0.24 Wh figure is for the median text prompt across all Gemini Apps text serving. If Google publishes per-model (Pro vs Flash vs Flash-Lite) at any point, that's a major upgrade to coverage.
- **AWS per-region PUE / WUE.** Not currently disclosed; the next research cycle should check.
- **eGRID2024 release.** EPA typically releases the next year's data ~18-24 months after the data year. eGRID2024 (data year 2024) is likely to land mid-2026.
- **Indirect water modeling per Ren et al.** v0.2 enhancement — incorporate power-plant cooling water, varies by grid mix and plant type.
- **OpenAI / GPT-5 disclosure.** OpenAI declined as of August 2025; check whether anything shifts.
- **Methodology critiques of Elsworth/Jegham.** Any peer-reviewed critique or revision of the methodologies we're now leaning on.
- **Newer Claude generations (Opus 4.8+, Sonnet 4.7+).** When they release, look for any vendor or third-party relative-efficiency claims that let us anchor the new values without naive copy-from-predecessor.

### Outcome

Committed to `environmental-factors.toml` (repo root). Cowork master copy retained at `<Cowork>/repo-staging/environmental-factors.toml` as the audit-trail reference for this proposal.
