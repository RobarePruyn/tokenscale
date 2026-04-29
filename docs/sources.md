# tokenscale — Sources

**Status:** v0.2 (broadened multi-provider sweep, water deep dive)
**Last updated:** 2026-04-28

## Confidence tags

- **`primary`** — first-party vendor disclosure, peer-reviewed publication, or government dataset.
- **`secondary`** — third-party analyses, preprints not yet peer-reviewed, blog posts, news reporting that summarizes primary work.
- **`superseded`** — formerly authoritative, now displaced by newer or better-sourced material. Retained for traceability; do not cite for factor values.

All access dates are the date the URL was last loaded by the research agent. URLs are recorded so any reader can re-verify the claim themselves.

---

## A. Anthropic environmental disclosures

### A.1 Anthropic Transparency Hub (voluntary commitments)

- **URL:** https://www.anthropic.com/transparency/voluntary-commitments
- **Org:** Anthropic
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Anthropic's public-facing transparency page, listing voluntary commitments around safety, security, and disclosures.
- **Contribution to factor model:** Confirms what Anthropic *does* publish at company level. As of access date, the page does **not** contain per-query, per-token, or per-model energy / water / carbon figures. Contains no Scope 1/2/3 emissions disclosure. This is the load-bearing citation for the project's stance that Anthropic-specific numerics are "not disclosed."

### A.2 Anthropic FMTI 2025 report (Stanford CRFM)

- **URL:** https://crfm.stanford.edu/fmti/December-2025/company-reports/Anthropic_FinalReport_FMTI2025.html
- **Org:** Stanford Center for Research on Foundation Models (CRFM); reports on Anthropic
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (third-party assessment of Anthropic's disclosures)
- **Summary:** The Foundation Model Transparency Index assessment of Anthropic, December 2025 release.
- **Contribution to factor model:** Independent confirmation of what Anthropic does and does not disclose. Useful as a corroborating reference when stating that per-query environmental data is not published by Anthropic.

### A.3 Anthropic-Amazon expanded compute partnership announcement

- **URL:** https://www.anthropic.com/news/anthropic-amazon-compute
- **Org:** Anthropic
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Anthropic announcement of expanded compute commitments with AWS, including up to 5 GW of new capacity and use of Trainium2–Trainium4 hardware.
- **Contribution to factor model:** Establishes that Claude inference runs primarily on AWS infrastructure — the basis for using AWS PUE/WUE values as the data center efficiency factor.

---

## B. Comparable vendor disclosures (anchor points)

### B.1 Google: "Measuring the environmental impact of AI inference" (Gemini per-prompt disclosure)

- **URL:** https://cloud.google.com/blog/products/infrastructure/measuring-the-environmental-impact-of-ai-inference/
- **Org:** Google Cloud (with co-publication by Google Research)
- **Author/Date:** Google, published 2025-08-21
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Google's first quantitative disclosure of per-prompt environmental impact for the median Gemini Apps text prompt — 0.24 Wh energy, 0.03 gCO₂e carbon, 0.26 mL water — with methodology covering machine energy, idle compute, datacenter overhead via Google's fleetwide PUE (1.09), and water via fleetwide WUE.
- **Contribution to factor model:** The single richest first-party anchor we have for an LLM-class system at production scale. Even though it's Google not Anthropic, it sets the order of magnitude for a frontier-model "median text prompt" and provides a transparent methodology template. The 33× / 44× year-over-year efficiency improvement claim Google makes also sets an expectation for how fast these numbers move.

### B.1.1 Elsworth et al. (2025), "Measuring the environmental impact of delivering AI at Google Scale" — companion technical paper

- **URL:** https://arxiv.org/abs/2508.15734
- **Authors:** Cooper Elsworth, Keguo Huang, David Patterson, Ian Schneider, Robert Sedivy, Savannah Goodman, Ben Townsend, Parthasarathy Ranganathan, Jeff Dean, Amin Vahdat, Ben Gomes, James Manyika
- **Submitted:** 2025-08-21 (v1)
- **DOI:** https://doi.org/10.48550/arXiv.2508.15734
- **License:** CC BY 4.0
- **Access date:** 2026-04-28
- **Confidence:** `primary` (preprint, but co-authored by Google Research and Google Cloud Infrastructure leadership; methodology is fully disclosed)
- **Summary:** The technical companion to Google's Aug 2025 blog post. Defines Google's "comprehensive methodology" — accounting for active accelerator power, host system energy, idle machine capacity, and data center overhead — and applies it to median Gemini Apps text prompts. Shows that the comprehensive method is roughly 2.4× higher than active-accelerator-only estimates (0.24 Wh vs 0.10 Wh).
- **Contribution to factor model:** **This is the methodological backbone of tokenscale's `compute_impact` math.** It defines what "energy per prompt" should include and provides the canonical framing we adopt in `tokenscale-core`. The 2.4× ratio is also a useful sanity check whenever we encounter a third-party number derived from active GPU only.

### B.2 MIT Technology Review coverage of Google's Gemini disclosure

- **URL:** https://www.technologyreview.com/2025/08/21/1122288/google-gemini-ai-energy/
- **Org:** MIT Technology Review
- **Access date:** 2026-04-28
- **Confidence:** `secondary`
- **Summary:** Independent reporting on Google's August 2025 Gemini environmental disclosure, with framing on what the numbers do and do not cover.
- **Contribution to factor model:** Useful for sanity-checking how the disclosure was received by independent technical press. Not a numerical anchor.

### B.3 "Is Google's reveal of Gemini's impact progress or greenwashing?" (Towards Data Science)

- **URL:** https://towardsdatascience.com/is-googles-reveal-of-geminis-impact-progress-or-greenwashing/
- **Org:** Towards Data Science (independent)
- **Access date:** 2026-04-28
- **Confidence:** `secondary`
- **Summary:** Critical analysis of Google's Gemini disclosure, examining what the methodology does and does not cover (notably its exclusion of image/video prompts, training amortization, and supply-chain emissions).
- **Contribution to factor model:** Methodology critique we should reference whenever we adopt a Google-style accounting approach. Important reminder that "median text prompt" is a narrow surface.

---

### B.4 OpenAI / Sam Altman — ChatGPT median energy claim

- **URL:** https://blog.samaltman.com/the-gentle-singularity (host: Sam Altman blog, June 2025)
- **Org:** OpenAI (CEO statement)
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (CEO blog post; methodology not disclosed; not a vendor technical disclosure)
- **Summary:** Sam Altman, June 2025: "the average ChatGPT query uses about 0.34 watt-hours" of electricity. No methodology, no breakdown by model.
- **Contribution to factor model:** Anchor for "average ChatGPT" energy used by Couch, Epoch AI, and others. Use cautiously — it's a CEO statement with no methodology. Treat as order-of-magnitude only and corroborate with Epoch AI's independent estimate (B.5) and Jegham et al.'s benchmarks (C.1).

### B.5 Epoch AI — "How much energy does ChatGPT use?"

- **URL:** https://epoch.ai/gradient-updates/how-much-energy-does-chatgpt-use
- **Org:** Epoch AI (independent research institute)
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (independent estimate, methodology shown)
- **Summary:** Estimates GPT-4o consumes approximately 0.0003 kWh (0.3 Wh) per query, closely aligning with Altman's 0.34 Wh claim.
- **Contribution to factor model:** Independent corroboration of OpenAI's GPT-4o-class energy. Use as anchor for the "median text query" energy band for OpenAI models.

### B.6 OpenAI GPT-5 disclosure non-status

- **URL:** https://aicommission.org/2025/08/openai-will-not-disclose-gpt-5s-energy-use/
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (third-party reporting on OpenAI's non-disclosure stance)
- **Summary:** OpenAI publicly declined to disclose GPT-5 energy use. Independent estimate via Jegham et al.: ~18 Wh per medium response.
- **Contribution to factor model:** Confirms that OpenAI, like Anthropic, does not publish per-model energy. All OpenAI factor values rest on Jegham et al. and Epoch AI estimates.

### B.7 Mistral — "Le Chat" environmental impact disclosure

- **URL:** https://www.devsustainability.com/p/mistrals-environmental-impact (third-party summary; primary Mistral disclosure linked from there)
- **Org:** Mistral AI (with David Mytton third-party analysis)
- **Access date:** 2026-04-28
- **Confidence:** `primary` for the Mistral-disclosed values; `secondary` for the surrounding methodology critique
- **Summary:** Mistral published per-query CO₂e and water for a 400-token Le Chat response: 1.14 gCO₂e and 45 mL water. **Mistral did NOT publish per-query energy (Wh).** Model size for Mistral Large 2 not disclosed.
- **Contribution to factor model:** Anchor for Mistral models' carbon and water footprint. Energy for Mistral is a derived estimate using comparable-class models.

### B.8 Microsoft Azure data center water disclosure (2024)

- **URL:** https://datacenters.microsoft.com/sustainability/efficiency/
- **Org:** Microsoft
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Microsoft 2024 disclosure: average global data center WUE 0.30 L/kWh (down from 0.49 in 2021, a 39% improvement). New zero-water cooling design (introduced August 2024) avoids ~125M L/year per datacenter. Microsoft tracks at global and operating-geography level (Americas, APAC, EMEA), with localized fact sheets in 28 regions.
- **Contribution to factor model:** Reference value for Azure-hosted models (relevant for OpenAI, since OpenAI primarily runs on Azure). Useful comparator for AWS WUE.

---

## C. Peer-reviewed and preprint inference-energy studies

### C.1 Jegham et al. (2025), "How Hungry is AI? Benchmarking Energy, Water, and Carbon Footprint of LLM Inference"

- **URL:** https://arxiv.org/abs/2505.09598
- **Authors:** Nidhal Jegham, Marwan Abdelatti, Chan Young Koh, Lassad Elmoubarki, Abdeltawab Hendawi
- **Submitted:** v1 2025-05-14; **latest:** v6 2025-11-24
- **DOI:** https://doi.org/10.48550/arXiv.2505.09598
- **License:** CC BY 4.0
- **Access date:** 2026-04-28
- **Confidence:** `primary` (preprint, but methodology is transparent and benchmarks are reproducible from public API data)
- **Summary:** An infrastructure-aware benchmarking framework that estimates per-prompt energy, water, and carbon for 30 frontier LLMs by combining public API performance data with company-specific environmental multipliers and inferred hardware configurations. Per the v6 abstract, the most energy-intensive models exceed 29 Wh per long prompt, and the framework includes Claude-family models in the comparison cohort.
- **Contribution to factor model:** The richest cross-vendor inference benchmark we have. Use it to sanity-check ranges for Claude models (especially older Sonnet/Opus generations the paper directly benchmarks) and to import its methodology — particularly its treatment of inferred hardware and company-specific PUE/WUE multipliers — into our own derivation. Older Claude models in this paper are direct evidence; for current-generation models (Opus 4.6/4.7, Sonnet 4.6) we extrapolate, with ranges explicitly noted.
- **Note:** Multiple revisions through v6 — the model cohort and absolute numbers shift between versions, so any value cited from this paper must reference a specific arXiv version (e.g., v6) rather than the unversioned identifier.

### C.2 Fernandez et al. (2025), "Energy Considerations of Large Language Model Inference and Efficiency Optimizations" (ACL 2025)

- **URL:** https://arxiv.org/abs/2504.17674 (preprint); https://aclanthology.org/2025.acl-long.1563/ (ACL anthology)
- **Submitted:** 2025-04-24; ACL 2025 long paper
- **Access date:** 2026-04-28
- **Confidence:** `primary` (peer-reviewed, ACL 2025)
- **Summary:** Argues that FLOP-based and theoretical-GPU-utilization estimates of inference energy underestimate real-world consumption by 2–6× due to memory, I/O, and kernel-launch overheads. Reports up to 73% energy reduction from inference optimizations applied appropriately, with strong context-dependence (batch size, draft-model speculation tradeoffs).
- **Contribution to factor model:** Methodological warning. Any factor value we derive from FLOPs or theoretical GPU utilization should be flagged as a likely under-estimate. This paper is the basis for our preference for empirical, vendor-disclosed, or API-token-based methods over derived-from-FLOPs methods.

### C.1.1 Specific Wh-per-long-prompt values from Jegham et al. v6 used in this factor model

The following per-long-prompt energy values are extracted from Jegham et al. v6 (long prompt = ~7,000-word input + ~1,000-word output, roughly 10,300 tokens total) and used as anchors in `environmental-factors.toml`:

| Model | Wh per long prompt | Notes |
| --- | --- | --- |
| LLaMA 3.1 8B | 0.443 | Most efficient model in the cohort; floor reference |
| LLaMA 3.2 1B | 0.552 | Note: differs from 8B due to architecture/serving differences, not just parameter scaling |
| LLaMA 3.2 3B | 0.707 |  |
| LLaMA 405B | 25.202 | Steep param-scale cost |
| GPT-4.1 nano | (most efficient OpenAI-class baseline cited) | Reference for "70× more" comparison statements |
| o3 (OpenAI) | >33 Wh per long prompt | Reasoning model; high overhead |
| GPT-5 (OpenAI) | ~18 Wh per medium response | Higher than all benchmarked except o3 and DeepSeek-R1 |
| DeepSeek-R1 | 29.075 Wh | ~65× more than the most efficient model |
| Claude Sonnet 4.5 (Couch's blended derivation) | ~3 Wh per long prompt (derived) | Cross-validated against Couch's per-token rates |

**Per-token derivation:** to convert per-long-prompt values to Wh/MTok for `environmental-factors.toml`, we divide by the assumed token count of the long prompt (~10,300 tokens) and scale to per-million-token. Example: DeepSeek-R1 at 29.075 Wh / 10,300 tokens × 1,000,000 = 2,823 Wh/MTok blended. We then apportion to input vs output using a typical 7:1 input-to-output ratio for long prompts. **All of this derivation is documented inline in `environmental-factors.toml`.**

### C.3 Patterson et al. (2021), "Carbon Emissions and Large Neural Network Training"

- **URL:** https://arxiv.org/abs/2104.10350
- **Authors:** David Patterson, Joseph Gonzalez, Quoc Le, et al. (Google + UC Berkeley)
- **Access date:** 2026-04-28
- **Confidence:** `primary` (foundational methodology paper, widely cited)
- **Summary:** Foundational paper on how to account for ML-system carbon emissions, distinguishing training versus inference, geographic-grid effects, processor choice, and datacenter efficiency factors. Identifies that carbon footprint can vary by 100–1000× depending on model architecture, datacenter, and processor choices.
- **Contribution to factor model:** Methodological backbone for thinking about which factors actually matter. Older (2021) and training-focused, but the general factor-decomposition framework — energy × grid carbon intensity × datacenter PUE × WUE — is what we apply.

---

## D. Datacenter infrastructure references

### D.1 AWS Sustainability — Data Centers (PUE, WUE, water-positive program)

- **URL:** https://aws.amazon.com/sustainability/data-centers/
- **Org:** Amazon Web Services
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** AWS's published data center efficiency metrics. Reports a 2024 global PUE of 1.15, and progress against the 2030 water-positive commitment (53% in 2024, up from 41% in 2023).
- **Contribution to factor model:** The PUE multiplier we apply to per-token compute energy to get total facility energy. AWS-global is the conservative default; we should also track region-specific PUE where AWS publishes it.

### D.2 2024 Amazon Sustainability Report — AWS summary

- **URL:** https://sustainability.aboutamazon.com/2024-amazon-sustainability-report-aws-summary.pdf
- **Org:** Amazon
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** The AWS-specific section of Amazon's 2024 sustainability report. Source document for the global PUE 1.15 figure and details of regional performance (Americas best site at 1.05, Asia Pacific at 1.07).
- **Contribution to factor model:** Authoritative, citable source for AWS PUE values.

### D.3 Amazon Sustainability — Water Positive Methodology (March 2026)

- **URL:** https://sustainability.aboutamazon.com/water-positive-methodology.pdf
- **Org:** Amazon
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Updated methodology document for AWS's water-positive accounting, March 2026.
- **Contribution to factor model:** Anchor for our water-usage factor and an audit reference if anyone asks how we derive mL-per-token water values.

### D.3.1 AWS 2024 disclosed WUE (global)

- **URL:** https://aws.amazon.com/sustainability/data-centers/ (also referenced in 2024 Amazon Sustainability Report)
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** AWS 2024 global data center WUE: **0.15 L/kWh** of IT load. 17% improvement from 2023, 40% improvement since 2021. AWS does **not** publish per-region WUE values. Notable regional context: Umatilla, Oregon AWS data centers donate up to 96% of cooling water to local farmers; 18 Loudoun County, Virginia data centers use recycled wastewater.
- **Contribution to factor model:** **Canonical water-per-kWh value for all AWS regions in v0.1.** We apply 0.15 L/kWh uniformly across us-east-1, us-east-2, us-west-2 because per-region WUE is not disclosed. Flagged as a gap in `# UNKNOWN` notes; next research cycle should check whether AWS adds per-region disclosure.

### D.4 AWS Bedrock cross-region inference for Claude (region documentation)

- **URL:** https://docs.aws.amazon.com/bedrock/latest/userguide/inference-profiles-support.html
- **Org:** Amazon Web Services (Bedrock documentation)
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Documents which AWS regions support Claude inference. As of access date, the US Anthropic Claude geographic profile routes between us-east-1, us-east-2, and us-west-2.
- **Contribution to factor model:** Establishes that Claude inference for US users runs in us-east-1 (N. Virginia), us-east-2 (Ohio), and us-west-2 (Oregon) — the regions whose grid carbon intensity values we should be tracking.

---

## E. Grid carbon intensity references

### E.1 EPA eGRID (Emissions & Generation Resource Integrated Database)

- **URL:** https://www.epa.gov/egrid
- **Org:** US Environmental Protection Agency
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Authoritative US dataset for power-sector emission rates by grid subregion. As of access date, the latest release is eGRID2023rev2 (released 2025-06-12), containing 2023 data.
- **Contribution to factor model:** Source for grid CO₂e intensity (in lb/MWh or kg/MWh) for the eGRID subregions covering the AWS regions where Claude inference runs (RFC East / SERC Virginia for us-east-1; RFC East for us-east-2; WECC California-Pacific for us-west-2).
- **Note:** eGRID lags real time by roughly 18–24 months. The 2023 vintage data is the most current at the time of this writing.

### E.2 EPA Greenhouse Gas Equivalencies Calculator — Calculations and References

- **URL:** https://www.epa.gov/energy/greenhouse-gas-equivalencies-calculator-calculations-and-references
- **Org:** US Environmental Protection Agency
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Methodology document accompanying EPA's GHG Equivalencies Calculator, showing how regional eGRID factors are applied.
- **Contribution to factor model:** Reference methodology for converting kWh of grid electricity to gCO₂e emissions in the relevant subregions.

### E.2.1 eGRID2023 summary tables — exact subregion CO2e values used

- **URL:** https://www.epa.gov/system/files/documents/2025-06/summary_tables_rev2.pdf
- **Org:** US Environmental Protection Agency
- **Released:** eGRID2023 — 2025-09-29 (final v2)
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Subregion-level annual CO₂e output emission rates (lb/MWh) directly extracted from the eGRID2023 summary tables for the AWS regions where Claude inference runs:

| Subregion | Description | CO₂e lb/MWh | kg/kWh | AWS region |
| --- | --- | --- | --- | --- |
| **SRVC** | SERC Virginia/Carolina | 596.3 | **0.270** | us-east-1 (N. Virginia) |
| **RFCW** | RFC West | 916.1 | **0.416** | us-east-2 (Ohio) |
| **NWPP** | WECC Northwest | 635.3 | **0.288** | us-west-2 (Oregon) |
| **CAMX** | WECC California | 430.0 | 0.195 | (reference; no AWS Claude-Anthropic region here) |
| **RFCE** | RFC East (Mid-Atlantic) | 599.2 | 0.272 | (reference) |

- **Contribution to factor model:** **These are the canonical per-region CO₂e values used in `environmental-factors.toml` v0.1**, expressed as `co2e_kg_per_kwh` in `[grid_factors.<region>]` blocks. State-level emission rates (also in the eGRID summary tables) are documented in the file as a sanity-check comparison.

### E.3 EPA GHG Emission Factors Hub

- **URL:** https://www.epa.gov/climateleadership/ghg-emission-factors-hub
- **Org:** US Environmental Protection Agency
- **Access date:** 2026-04-28
- **Confidence:** `primary`
- **Summary:** Centralized list of EPA emission factors for purchased electricity, mobile combustion, transportation, and other GHG inventory categories, refreshed annually (January 2025 update is current).
- **Contribution to factor model:** Cross-check against eGRID values; canonical reference for any non-power-sector emission factors we adopt.

---

## F. Methodology critiques and meta-analyses

### F.1 Fernandez et al., ACL 2025 (see C.2)

Cross-listed here because its principal value to our project is methodological critique of FLOP-based estimates.

### F.2 Towards Data Science: Greenwashing critique of Google Gemini disclosure (see B.3)

Cross-listed for the same reason: methodological critique of Google's August 2025 disclosure.

### F.3 Nature Scientific Reports — "Reconciling the contrasting narratives on the environmental impact of large language models"

- **URL:** https://www.nature.com/articles/s41598-024-76682-6
- **Access date:** 2026-04-28
- **Confidence:** `primary` (peer-reviewed, Nature Scientific Reports)
- **Summary:** Reviews the spread of published estimates for LLM environmental impact and argues for narrowing methodological assumptions to reconcile divergent reports.
- **Contribution to factor model:** Useful framing for why our estimates need ranges, not point values.

### F.4 "Toward Sustainable Generative AI: A Scoping Review of Carbon" (preprint)

- **URL:** https://arxiv.org/pdf/2511.17179
- **Submitted:** 2025-11
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (preprint, not peer-reviewed at access date)
- **Summary:** Scoping review of generative-AI carbon accounting work, intended to consolidate methodologies.
- **Contribution to factor model:** Bibliography aid for the next research cycle. Watch for peer-review status.

---

## F.5 Water — dedicated deep-dive sources

### F.5.1 Li, Yang, Islam, Ren — "Making AI Less 'Thirsty': Uncovering and Addressing the Secret Water Footprint of AI Models"

- **URL (preprint):** https://arxiv.org/abs/2304.03271
- **URL (published):** https://dl.acm.org/doi/10.1145/3724499 (Communications of the ACM)
- **GitHub:** https://github.com/Ren-Research/Making-AI-Less-Thirsty
- **Authors:** Pengfei Li, Jianyi Yang, Mohammad A. Islam, Shaolei Ren (UC Riverside / UT Arlington)
- **Access date:** 2026-04-28
- **Confidence:** `primary` (peer-reviewed in Communications of the ACM)
- **Summary:** Foundational paper on AI water footprint. Distinguishes **direct water** (cooling tower evaporation at the data center) from **indirect water** (cooling tower evaporation at upstream power plants). Reports that training GPT-3 in Microsoft's US data centers can directly evaporate ~700,000 liters of clean freshwater. Projects global AI water demand at 4.2–6.6 billion m³ by 2027.
- **Contribution to factor model:** **The methodological backbone for tokenscale's water accounting.** Specifically:
  - Establishes that water computation must include both direct (data-center cooling) and indirect (power-plant cooling) components.
  - Quantifies AI water consumption mechanisms in a way that supports our `water_l_per_kwh` factor decomposition.
  - The github repository includes worked examples we can verify against.
- **What we don't yet do:** v0.1 only models direct WUE per region. Indirect water (varies by grid mix and power plant cooling type) is a v0.2 enhancement.

### F.5.2 UCR News — "AI programs consume large volumes of scarce water"

- **URL:** https://news.ucr.edu/articles/2023/04/28/ai-programs-consume-large-volumes-scarce-water
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (institutional press release; useful framing of the Ren paper for general audiences)
- **Summary:** UC Riverside press release summarizing the Ren water footprint paper, with quotable claims for the methodology page.
- **Contribution to factor model:** Supplementary; not a numerical input.

### F.5.3 Jegham et al. v6 — DeepSeek water disclosure (cross-cited from C.1)

A specific finding worth surfacing in the water section: Jegham et al. v6 reports that DeepSeek-R1 deployed on DeepSeek's own infrastructure consumes **>200 mL water per long query** vs only **34 mL** for the same model deployed on Azure — an 85% reduction attributable to infrastructure differences (cooling type, PUE, regional WUE). This is the strongest single data point in the literature showing that the choice of inference provider can dominate the choice of model for water impact.

---

## G. Claude Code-specific analyses

### G.1 Simon P. Couch — "Electricity use of AI coding agents"

- **URL:** https://simonpcouch.com/blog/2026-01-20-cc-impact/
- **Author:** Simon P. Couch
- **Published:** 2026-01-20
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (independent blog analysis, methodology shown but values are extrapolations from third-party benchmarks)
- **Summary:** Couch builds a per-token energy model for Claude Code sessions by scaling published median-query figures (notably the Google Gemini 0.24 Wh anchor) through real Claude Code session token counts pulled from the local JSONL session logs in `~/.claude/`. Reports approximate Wh/MTok by token type (input, cached input, output) for several Claude 4.5-class models, and arrives at a median Claude Code session of ~41 Wh and a typical coding day of ~1,300 Wh.
- **Contribution to factor model:** Closest existing public analog to what `tokenscale` is doing. Provides:
  - A worked methodology for going from per-prompt anchors to per-token rates.
  - Specific Wh/MTok estimates for 4.5-generation models (Opus 4.5, Sonnet 4.5, Haiku 4.5) that we can use as anchor values, with the explicit caveat that Couch's per-token rates are themselves estimates derived from Google's Gemini disclosure plus model-class adjustments.
  - The exact local-disk schema for Claude Code session logs, which the Claude Code engineering side should consume.
- **Caveats:** Couch's methodology is transparent but inherits all the uncertainty of the Gemini anchor. He explicitly excludes water from his analysis. His Opus 4.5 / Sonnet 4.5 values are *not* extrapolatable to Opus 4.6, Opus 4.7, or Sonnet 4.6 without additional reasoning — for those, his work is a starting point, not a destination.

### G.2 Anthropic Climate Score — DitchCarbon

- **URL:** https://ditchcarbon.com/organizations/anthropic
- **Org:** DitchCarbon (third-party climate-disclosure tracker)
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (aggregator, not a primary disclosure)
- **Summary:** Tracks Anthropic's public climate disclosures and assigns a transparency score; surfaces what is and is not reported (no specific carbon emissions, no committed 2030/2050 climate targets at access date).
- **Contribution to factor model:** Independent corroboration of Anthropic's non-disclosure status, useful as a cross-reference to A.1 and A.2.

### G.3 Hannah Ritchie — "What's the carbon footprint of using ChatGPT or Gemini?" (August 2025)

- **URL:** https://hannahritchie.substack.com/p/ai-footprint-august-2025
- **Author:** Hannah Ritchie (Our World in Data)
- **Published:** 2025-08
- **Access date:** 2026-04-28
- **Confidence:** `secondary` (independent analysis by a credentialed climate-data communicator)
- **Summary:** Translates Google's Gemini disclosure into per-user contextual scale (kWh/year for typical chat use, daily-driving equivalents, etc.).
- **Contribution to factor model:** Useful for the dashboard's user-facing framing of "what does this mean in scale terms." Not a numerical input to the factor model itself.

---

## What I didn't find / known gaps

- **No Anthropic-published per-query, per-token, per-model, or per-AWS-region environmental data** at access date. This is the load-bearing gap that drives why so many tokenscale factors have to be estimates.
- **No peer-reviewed Claude-4.x-family inference benchmark.** Jegham et al. covers older Claude generations directly; current generations are extrapolated.
- **No public AWS region-specific PUE/WUE breakdown for the specific compute hosting Claude.** AWS publishes a global PUE and best-region figures but doesn't publish per-region PUE for every region. We use global PUE 1.15 as a conservative default and flag this gap.
- **No vendor disclosure of model-specific kWh-per-token at any precision** for any model from any vendor. Even Google's disclosure is per "median text prompt," not per token.

These gaps directly inform the `# UNKNOWN` and `# ESTIMATE:` flags in `environmental-factors.toml`.
