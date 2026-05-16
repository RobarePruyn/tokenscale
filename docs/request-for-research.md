# tokenscale — Open Research Questions

Open questions the maintainer (or Cowork research agent, when that lands) should pick up in the next quarterly sweep — or sooner if any of them get answered by external publication.

See [`research-cadence.md`](research-cadence.md) for the process around how these get worked. See [`research-log.md`](research-log.md) for past sweep outcomes.

Format: each entry has a status, the question, why it matters, what good answers look like, and pointers for where to start looking.

---

## Open

### Anthropic tokenizer-change inflation factor verification

**Status**: Open. v0.1 file estimates the factor from third-party analysis; we'd like a primary source.

**Question**: What's Anthropic's actual tokenizer-change ratio for Opus 4.7 vs 4.6 — i.e., how much more does Opus 4.7 spend in tokens for the same English text?

**Why it matters**: The v0.1 factor file uses `tokenizer_token_count_inflation_factor = 1.175` for Opus 4.7 (the midpoint of a 1.0–1.35× range observed by Caylent's third-party analysis). If we ever surface "per-task" comparisons across model versions, this factor is load-bearing — per-token energy may be lower for Opus 4.7 but per-task energy may be similar or higher because the same task takes more tokens.

**What good answers look like**:

- A primary source from Anthropic confirming (or correcting) the 1.0–1.35× range.
- Per-language tokenizer behavior: the inflation factor probably varies by input language. English is one number; CJK languages might be very different. v0.1 doesn't model this; it should be flagged or addressed.

**Starting points**:

- [Vellum benchmarks for Opus 4.7](https://www.vellum.ai/blog/claude-opus-4-7-benchmarks-explained).
- [Caylent's tokenizer analysis](https://caylent.com/...) (cited in `environmental-factors.toml`).
- Anthropic's tokenizer source if/when they publish one.

---

### eGRID coverage for non-US AWS regions

**Status**: Open. v0.1 covers us-east-1, us-east-2, us-west-2 only.

**Question**: What grid-intensity equivalents to eGRID exist for non-US AWS regions where Anthropic might run inference (eu-west-1 Ireland, ap-northeast-1 Tokyo, etc.)?

**Why it matters**: Users outside the US can't currently configure their `default_inference_region` to a non-US AWS region and get honest CO₂e numbers. The `grid_factors` table only carries US subregions; everything else falls back to defaults.

**What good answers look like**:

- New `[grid_factors.*]` rows for the major AWS regions: eu-west-1, eu-central-1, ap-northeast-1, ap-southeast-1, ap-southeast-2, eu-north-1.
- For each, the equivalent of an eGRID subregion code: in EU it's ENTSO-E zones; in Japan it's TEPCO/KEPCO/etc.; in Australia it's AEMO regions.
- Source URLs and methodology notes explaining what "subregion" means for that country (every country defines this differently).

**Starting points**:

- ENTSO-E published carbon intensity data (Europe).
- [Electricity Maps](https://app.electricitymaps.com/) — has region-by-region annual data with API access.
- AWS's own region pages occasionally list carbon-intensity context.

---

### Methodology — confidence in the "comprehensive" methodology choice over time

**Status**: Open ongoing. v0.1 chose Google's August 2025 "comprehensive methodology" (Elsworth et al. 2025) as the canonical approach. We should periodically verify that's still the right call.

**Question**: As the LLM impact research field evolves, does Google's comprehensive methodology remain the most defensible single methodology to standardize on — or has a peer-reviewed alternative shifted the consensus?

**Why it matters**: This is the methodological foundation. Switching it later would require recomputing every impact number in the dashboard, retroactively. We should make sure the foundation is still correct each year rather than discovering after years that we anchored on a now-superseded methodology.

**Triggers for action**:

- Anthropic publishes their own methodology (currently unannounced).
- A consensus emerges in academic literature for a non-Google methodology.
- Google publishes a v2 of their methodology that materially shifts the approach.

---

### Cost-side time-anchoring + Cost Methodology audit trail

**Status**: Open. **HARD TRIGGER: must land before the next time Anthropic changes any model's per-token price.**

**Question**: Promote `pricing.toml`'s already-present `valid_from` field to actually drive per-event time-anchored pricing lookup, with a versioned audit trail mirroring the environmental factor file. Today, `PricingFile::lookup(provider, model)` ignores the date and returns whatever single row matches `(provider, model)` — so the current pricing is applied retroactively to every event in history.

**Why it matters**: This is a credibility-of-the-tool issue, not a feature request. The whole environmental side is built on "every number traces to a source" and "factors resolve to the row authoritative at the event's timestamp." The cost side is the headline of the net-value calculation, and it currently has neither property. The asymmetry is documented (in [`docs/cost-methodology.md`](cost-methodology.md) and in the dashboard's "How is this computed?" disclosure), but documentation is a stopgap. The moment Anthropic changes a price, every historical counterfactual silently shifts and a public methodology-forward tool cannot have that happen undisclosed.

**What good answers look like**:

- `PricingFile::lookup(provider, model, as_of_date)` time-anchored against `valid_from` per-event, mirroring `lookup_environmental_factors`.
- `pricing.toml` carries a `file_version` and supports multiple rows per `(provider, model)` keyed by `valid_from` (the schema already has the field; the loader needs to accept multiple rows).
- A parallel **Cost Methodology** section in the dashboard's Methodology page covering: pricing source, refresh cadence, tier assumption, retroactive-pricing fix (this item), subscription pro-rating rule, cache discount math. v0.1.10 ships a short version of this doc; 6b expands it to environmental-page parity.
- An audit trail of past pricing changes (analog to `research-log.md`) so any historical price shift is visible.

**Triggers for action**:

- **Anthropic changes any model's published per-token price.** This is the hard deadline.
- The pricing review checklist (`pricing_review_pending = true` in `pricing.toml`) catches a deferred refresh that would touch historical numbers.

**Starting points**:

- [`crates/tokenscale-core/src/pricing.rs`](../crates/tokenscale-core/src/pricing.rs) — `PricingFile::lookup` is the load-bearing call site. Currently ignores date.
- [`crates/tokenscale-store/src/factors_lookup.rs`](../crates/tokenscale-store/src/factors_lookup.rs) — `lookup_environmental_factors` is the model for what the time-anchored pricing lookup should look like.
- [`crates/tokenscale-store/src/impact_query.rs`](../crates/tokenscale-store/src/impact_query.rs) — the SQL aggregate path uses correlated subqueries on `valid_from` for env factors. Pricing would need an analogous structure.
- Server-side pricing was originally loaded once at startup from the embedded `pricing.toml`. The promotion is: TOML → DB tables (analogous to `env_factors` / `grid_factors`) → per-event lookup at compute time.

#### Companion item: pricing-rate-card divergence detection

The hard trigger above ("must land before the next Anthropic price change") is documented but has no detection mechanism. A documented trigger with no detector depends on a human noticing — exactly the failure mode the trigger exists to prevent. Anthropic could change Opus or Sonnet per-token pricing and the historical net-value numbers would drift silently in the gap before the next maintainer-initiated pricing review.

**Self-enforcing fix**: a CI check (nightly cron, not user-startup) that compares the rates in `pricing.toml` against Anthropic's published rate card and warns on divergence. Converts "hope someone notices" into a GitHub Actions alarm. This item is independently useful even before the time-anchoring fix lands: if 6b takes time to design and ship, the detector at least tells the maintainer the *moment* historical numbers go stale.

**Implementation sketch**:

- Nightly GH Action that fetches Anthropic's published rate card (e.g. parse `https://docs.anthropic.com/...` pricing page, or a stable JSON endpoint if one exists). Failure modes: page restructuring, network errors, parse mismatches — the action should fail open (warn, don't block) on parse uncertainty.
- Compare each `input_usd_per_mtok` / `output_usd_per_mtok` / `cache_*_usd_per_mtok` in `pricing.toml` against the parsed rate card.
- On divergence: open a GitHub Issue tagged `pricing-divergence` with the affected models + before/after rates, ping the maintainer, and (optionally) bump `[pricing] file_status` from `production` to `review_pending` so the dashboard's banner surfaces it to users.
- Fallback if Anthropic's pricing page is too brittle to scrape reliably: a hand-maintained `pricing-rate-card.snapshot.json` committed to the repo, with a nightly check that the snapshot timestamp is < 90 days old AND a per-quarter prompt to re-verify against the live page. Less automated but more robust.
- **NOT user-startup**: every user start should NOT hit Anthropic's web property. The dashboard is local-first; networked checks at startup would violate that and add a privacy surface. Keep detection on the CI side, propagate via release tagging or a banner refresh.

**Why file together with the time-anchoring fix**: the detector is the trigger; the fix is the resolution. Wiring them in one item (or as siblings) ensures the implementer doesn't ship one without the other and rebuild the same gap a year later.

**Triggers for action**:

- Same as parent (next Anthropic price change), but with this companion landed, the maintainer doesn't have to notice — the CI does.

**Starting points**:

- `.github/workflows/` — the amend-formula workflow in the homebrew-tokenscale tap is a working precedent for "GH Action that compares one source-of-truth file against another and surfaces drift."
- [`pricing.toml`](../pricing.toml) — the file the detector compares against. Already has `valid_from` / `accessed_at` / `file_status` plumbing that the detector can update.

---

### Amortized model-training cost (energy / CO₂e / water) per served token

**Status**: Open — aspirational. The dashboard today only reports **inference-side** impact. A model's training run consumed energy, CO₂e, and water that is logically attributable to every token the model serves over its lifetime; including it would make tokenscale's lifecycle picture complete.

**Question**: Can we add a per-token *amortized training cost* (energy / CO₂e / water) that, when added to the per-token inference cost, gives a full-lifecycle impact figure? As global usage of a model grows, the per-token amortized share naturally drops.

**Why it matters**: Inference-only numbers systematically *under-state* a model's full environmental footprint. Frontier-class models have non-trivial training footprints (BLOOM-176B: ~25 tCO₂e training-run, ~50 tCO₂e full-envelope per Luccioni 2022); ignoring them means the dashboard tells only part of the story. The amortization framing is academic-consensus (Strubell 2019, Patterson 2021, Luccioni 2022).

**What good answers look like**:

- A `[providers.<p>.models.<m>.training]` block in `environmental-factors.toml` carrying `training_energy_kwh`, `training_co2e_kg`, `training_water_l`, `cumulative_tokens_served` (as of a `valid_at` date), and a `training_amortization_confidence` tag.
- A computed per-token amortization (`training_*` / `cumulative_tokens_served`) summed with the existing inference per-token factors when an "Include amortized training cost" toggle is on.
- Honest acknowledgment of three load-bearing data gaps: **(a)** Anthropic has not published Claude training compute — confidence on third-party FLOPs estimates will be `low_speculative` and uncertainty bands will be wide (±100% is realistic); **(b)** `cumulative_tokens_served` is unpublished by every provider — we will have to Fermi-estimate from subscriber counts × usage assumptions; **(c)** the numerator question — what counts in "training cost"? Conventional definition is the final successful run; honest definition is +failed runs +data prep +embodied GPU carbon, which is ~3–5× larger per Luccioni 2022's BLOOM-2 envelope analysis. We should pick a position and label it explicitly.

**Triggers for action**:

- Anthropic publishes Claude training compute or LCA numbers.
- A reliable third-party estimate of Claude training compute lands (peer-reviewed or audited).
- The aggregate amortized envelope across the frontier-model field becomes well-enough characterized that the headline number isn't dominated by guesswork.

**Starting points**:

- [Strubell et al., "Energy and Policy Considerations for Deep Learning in NLP"](https://aclanthology.org/P19-1355/) — original amortization framing.
- [Patterson et al., "Carbon Emissions and Large Neural Network Training"](https://arxiv.org/abs/2104.10350) — Google's methodology, includes amortization formulas.
- [Luccioni et al., "Estimating the Carbon Footprint of BLOOM"](https://arxiv.org/abs/2211.02001) — full lifecycle including failed runs and embodied carbon. BLOOM-2 envelope analysis is the gold standard reference.
- Hugging Face's `codecarbon` and the AI Energy Score initiative for open-model training-cost estimates.

---

## Resolved

(Move entries here when they're answered in `research-log.md`.)

### Grid-factor uncertainty bands (CO₂e portion)

**Status**: Partially resolved by Sweep #1, 2026-05-12. See [research-log.md](research-log.md).

**Resolution**: Per-subregion `co2e_uncertainty_range_pct` bands now ship in `environmental-factors.toml` v0.2, derived from year-over-year variance across eGRID 2019 / 2020 / 2022 / 2023 plus a buffer for the subregion-to-datacenter mix gap:
- SRVC: ±15% · RFCW: ±20% · NWPP: ±20% · CAMX: ±20%

`water_uncertainty_range_pct = 50` applied across all AWS regions, reflecting the global-to-regional WUE application gap (AWS publishes no per-region WUE).

**Remaining**: Combining model + grid uncertainty into the dashboard's headline `± X%` cell badge is a deliberate v0.3 follow-on — kept separate in v0.2 so users can see the decomposition before we collapse it. Per-region WUE values from AWS would let us tighten the water band; tracked separately.

### Indirect water (power-plant cooling) methodology

**Status**: Resolved by Sweep #2, 2026-05-15. See [research-log.md](research-log.md).

**Resolution**: Per-subregion `indirect_water_l_per_kwh` ships in `environmental-factors.toml` v0.3:
- SRVC: 2.39 L/kWh ±35% (Ren et al. 2024 VA direct quote)
- RFCW: 1.85 L/kWh ±35% (eGRID×Macknick computed)
- NWPP: 9.50 L/kWh ±60% (Ren et al. 2024 WA direct quote, hydro-dominated)
- CAMX: 3.20 L/kWh ±50% (eGRID×Macknick computed)

Dashboard adds a "Include indirect water" toggle on the Water KPI. When enabled, displayed water = direct + indirect with quadrature-of-sum uncertainty; tooltip shows the breakdown.

**Remaining**: Hydro attribution methodology is contested (Macknick uses 100% reservoir-evaporation attribution to power generation; literature gives 5×–10× range). Currently using Macknick as-published with widened bands for hydro-heavy regions. A future sweep could refine the hydro coefficient or split reservoir vs run-of-river.
