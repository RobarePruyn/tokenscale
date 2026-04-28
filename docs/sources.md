# tokenscale — Sources

**Status:** v0.1 (initial sweep)
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
- **Summary:** Google's first quantitative disclosure of per-prompt environmental impact for the median Gemini Apps text prompt — 0.24 Wh energy, 0.03 gCO₂e carbon, 0.26 mL water — with methodology covering machine energy, idle compute, datacenter overhead via Google's fleetwide PUE, and water via fleetwide WUE.
- **Contribution to factor model:** The single richest first-party anchor we have for an LLM-class system at production scale. Even though it's Google not Anthropic, it sets the order of magnitude for a frontier-model "median text prompt" and provides a transparent methodology template. The 33× / 44× year-over-year efficiency improvement claim Google makes also sets an expectation for how fast these numbers move.

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
