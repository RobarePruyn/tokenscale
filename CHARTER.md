# tokenscale — Project Charter

**Status:** v0.3 (broadens research scope to all major LLM providers from v1; adds methodology page; flags framework-extractable design lens)
**Drafted:** 2026-04-28
**Last revised:** 2026-04-28
**Maintainer:** Robare Sarif

## Goal

`tokenscale` is a self-hostable dashboard that tells me what my Anthropic usage actually costs me, tracked over time across three dimensions:

1. **Real spend.** What I actually paid Anthropic — subscription fees, API line items, and any other charges the Admin API exposes.
2. **Counterfactual API spend.** What the same volume of tokens would have cost at standard API list pricing. This is the "value-of-subscription" view: it answers whether a Claude Max / Team / Enterprise seat is paying for itself relative to pay-as-you-go API usage.
3. **Environmental impact.** Energy (Wh), water (mL), and carbon (gCO₂e) — derived from a versioned factor model that lives in the Git repo and is maintained out of this Cowork project.

The dashboard ingests from two data sources:

- **Local Claude Code logs.** The session JSONL files Claude Code writes to disk on the user's machine.
- **Anthropic Admin API.** Read-only access to billing and usage data tied to the user's Anthropic account.

## Non-goals — explicit scope boundaries

`tokenscale` does **not**:

- **Bill, charge, or invoice.** It is a reporting tool. No payments move through it.
- **Throttle or rate-limit.** It does not sit in the request path of any Claude product.
- **Modify Anthropic account state.** All Admin API access is read-only from this tool's perspective.
- **Track non-Anthropic models in v1.** OpenAI, Google, AWS Bedrock-hosted non-Claude models, and local models are out of scope for v1. v1 surfaces no metrics for them. **v2 will extend** to additional providers — the v1 architecture leaves explicit hooks for this (see "Forward-looking architecture").
- **Constitute an audited environmental disclosure.** The energy / water / carbon figures are best-effort estimates derived from public data. They are useful for personal accounting and intuition; they are not a substitute for a vendor-issued, third-party-audited sustainability report.
- **Predict future pricing or factor values.** Numbers reflect what is public *as of* the inline date stamps in the factor file. Nothing is forecast.
- **Provide tax, accounting, or compliance advice.**

## Architecture split

`tokenscale` is built across two operational surfaces, with the Git repository as the merge point.

| Surface | Owns |
| --- | --- |
| **Cowork project** (this folder) | Research, factor-model proposals, source bibliography, research log, requests-for-code-change. Heavy reading, drafting, citation work. |
| **Claude Code** (engineering) | Application source code — log ingestion, schema, storage, dashboard UI, Admin API client, factor-file parser. |
| **Git repository** (merge point) | Canonical `environmental-factors.toml` and the application source. Anything the application reads at runtime lives here. |

The factor model lives in the Git repo because the application reads it. Cowork drafts updates in a `proposals/` directory; the maintainer reviews and merges into the repo by hand. Proposals are retained in Cowork as the historical audit trail for *why* every numeric value is what it is.

## Operating model

- **Factor updates** flow Cowork → review → Git. No direct edits from Cowork into the repo's `environmental-factors.toml`.
- **Engineering requests** — when the application needs a new field, capability, or value from research — surface in `request-for-code-change.md` here in Cowork; the maintainer hands those to Claude Code.
- **Research cadence** defaults to a check every two weeks: new Anthropic disclosures, new peer-reviewed inference-energy work, updated grid-carbon-intensity figures for the AWS regions hosting Claude inference, and any methodology critiques of currently cited sources.
- **Out-of-cadence updates.** When a significant new authoritative source surfaces between cycles, the research agent flags it in the coordination thread without waiting for the next scheduled run.

## Forward-looking architecture (v2 readiness)

The v1 build is Anthropic-only, but the architecture is designed so that adding a provider in v2 is additive — not a redesign. Specifically:

- **Factor model.** `environmental-factors.toml` is structured around provider as a top-level dimension. v1 ships with `[providers.anthropic.*]` populated and other-provider blocks absent. The application gracefully handles missing provider blocks. A `schema_version` key gates compatibility.
- **Database schema.** The `events.source` field accepts new ingest types without a migration. Pricing and factor tables are vendor-agnostic in shape — adding a new provider's models is row insertions, not column additions.
- **Connector layer.** Ingest is split into per-provider crates. v1 has Claude Code (JSONL) and Anthropic Admin API. v2 adds e.g. OpenAI usage API, Bedrock cross-vendor, etc., each as an independent crate that shares a common ingest interface. Providers are configurable on/off in the dashboard.
- **Presentation.** The dashboard supports filtering and sorting by provider — view all-LLMs blended per token, Anthropic only, or any selection of configured providers.
- **Research scope is broader than application scope from v1.** The Cowork research agent crawls as wide a set of LLM providers and models as it can find good data for, *from v1 onward* — not gated on v2 ingest support. The application still only ingests from Anthropic in v1 (because that's what the user's data sources expose), but the factor file and methodology page cover the full landscape: Anthropic, OpenAI, Google Gemini, Meta LLaMA, DeepSeek, Mistral, and any provider with credible published data. This makes the public methodology page a more compelling community artifact and de-risks v2 ingest landings — factor data is already in place when the new ingest crate ships.
- **Water deserves a dedicated research dimension.** Direct water (data-center cooling) and indirect water (power-plant cooling) per Ren et al.'s "Making AI Less Thirsty" methodology. v0.1 covers direct only; indirect is a v0.2 enhancement.

## Distribution and governance model

`tokenscale` is intended for public release on GitHub. Its update story is structured around a maintainer-pushes / users-pull model with a power-user escape hatch.

- **Single source of truth.** The Git repository hosts the canonical `environmental-factors.toml`. Anyone running the binary can choose to consume that file as their factor source.
- **Maintainer instance is special.** The maintainer's local install (Robare's machine) is configured to auto-push approved factor-model updates to the upstream Git repo after they pass review in this Cowork project. That capability is gated to the maintainer's machine — it is *not* exposed in the public binary's default behavior. Mechanism is intentionally simple: a deploy key or signed token configured only on the maintainer's machine, with auto-push behavior off by default in any other install.
- **Downstream users have two modes:**
  1. **Pull mode (default for casual users).** The dashboard auto-updates its factor model from the upstream public Git repo on a configurable cadence. Read-only consumer of the maintainer's research.
  2. **Local research mode (power users).** The user's local Claude (Cowork or otherwise) runs research and updates their own local factor model. Optionally periodically resyncs with upstream.
- **In-browser research management.** The dashboard exposes a research-runs view backed by the `research_runs` table. Users can review proposals as diffs, see source attributions, and approve or reject — without having to come back to a Cowork chat for routine review work. The Cowork project remains the proposal engine; the dashboard is the review surface.
- **Setup documentation** must explain (a) how to switch between pull and local-research modes, (b) how to configure a local factor model, and (c) how to schedule periodic syncs from the upstream maintainer's repo.

## Methodology / transparency page

`tokenscale` ships a built-in methodology and transparency page that exposes the provenance of every numeric value the dashboard displays. This is required, not optional. Without it, the project's environmental impact figures — which are heavily estimated — carry no credibility. With it, the project becomes a defensible community resource.

The page surfaces, for every numeric value:

- Where the data came from (URL + author/org).
- When it was published and last polled by the Cowork research agent.
- The methodology used to derive it.
- Why the value is trusted (confidence tag, peer-reviewed vs not, primary vs secondary).
- How the conclusion was reached (derivation when estimated, range when uncertain).
- Why a reader should trust those conclusions (the full audit trail).

Implementation is hybrid:

- **Per-value provenance** is dynamic, backed by `env_factors.source_doc` joined to bundled `docs/sources.md` entries.
- **Methodology narrative** is a static `docs/methodology.md` (Cowork-maintained, bundled into the binary).
- **Bibliography** renders `docs/sources.md` with last-polled timestamps and confidence tags.
- **Research log** renders `research_runs` table content.

This is Phase 3 work for the engineering side, but the schema and data plumbing are in place from Phase 1.

## Framework-extractable design lens

The factor model and the math in `tokenscale-core` are deliberately written so they can be extracted into a standalone open-source AI cost / impact analysis framework if the project succeeds. This is not a v1 commitment — `tokenscale` ships as a single tool — but every architectural choice keeps the option open: factor-file schema is generic, `compute_impact` math has no dashboard-specific assumptions, and the research process (sources → research log → factor file) is portable.

If a future spinout is desired, it is a packaging exercise, not a rewrite.

## Standards

- **Every numeric factor traces to a specific source URL with an access date.** No exceptions.
- **Estimates are labeled as estimates.** Where a value is interpolated or derived, the comment shows the derivation and gives a range rather than a false-precision point value.
- **Unknown stays unknown.** The factor file accommodates `null` / `"not_disclosed"`. The application handles missing data gracefully; the factor model never fabricates a number to fill a gap.
- **Primary sources preferred.** Vendor disclosures and peer-reviewed papers come before blogs and aggregators. Secondary sources are used only when no primary exists, and are tagged as such.
- **No paywalled reproduction.** Cite, paraphrase, link.
