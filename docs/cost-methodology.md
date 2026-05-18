# tokenscale — Cost methodology (short)

Companion to [`methodology.md`](methodology.md). That page covers the **environmental** side in depth; this one is the **cost** side. Shorter because the cost math is simpler — but the assumptions baked into it are load-bearing for the headline "Estimated savings vs raw API rates" number, so they need to be visible, not buried.

If you want full audit-trail parity between cost and environmental data — versioned `pricing.toml`, per-event time-anchored pricing, sweep cycle — see [request-for-research.md](request-for-research.md)'s "Cost-side time-anchoring + audit trail" entry.

---

## What the dashboard reports

Four cost-side numbers, three of them dollar amounts and one of them a derived percentage:

- **Counterfactual API cost** — what these tokens would have cost if paid at Anthropic's published API list rates.
- **Subscriptions paid in window** — manually-declared subscriptions pro-rated over their overlap with the window, plus any imported billing rows tagged `subscription`.
- **Other charges in window** — imported billing rows not tagged subscription (overage, one-time, refunds).
- **Estimated savings vs raw API rates** — counterfactual − (subscriptions + other charges). The headline number, tinted emerald when positive.

Cache savings (the "Cache hits" strip below the cards) is derived from the same data but tracked separately because it's a *discount* the user is already getting on their actual bill, not a comparison to a counterfactual.

---

## Four assumptions you should know about

### 1. API list rates, no volume / enterprise / batch discount

Counterfactual cost uses the per-million-token rates published in [`pricing.toml`](https://github.com/RobarePruyn/tokenscale/blob/main/pricing.toml), which mirror Anthropic's posted API list rates. No volume tiers are modeled. No enterprise discount is applied. The batch API's 50% discount isn't modeled either.

If you have an enterprise contract that gives you a different rate, the counterfactual over-states what the API would actually cost you, which in turn over-states your subscription savings. The number is still a defensible upper bound — your real API cost is at most the list-rate counterfactual, often less.

### 2. **Current pricing is applied retroactively to all historical events** ← the big one

The environmental side resolves each event against the env_factors row whose `valid_from` is the latest date `≤ event.occurred_at`. **The cost side does not do this today.** `pricing.toml` rows carry `valid_from` fields, but `PricingFile::lookup()` ignores them — it returns whatever single row matches `(provider, model)`, applied uniformly to every event in history.

Practical consequence: when Anthropic next changes a model's per-token price, every counterfactual number in your dashboard silently shifts. The April-2026 cost number you screenshotted will read differently after the next pricing sweep. This is a known asymmetry between the environmental and cost sides; closing it is on the open research queue with a **hard trigger**: it has to land before the next Anthropic pricing change.

### 3. Subscription pro-rating: flat daily

Manually-declared subscriptions are pro-rated as `monthly_usd × overlap_days / 30`, where `overlap_days` is the number of days the subscription's `[from, to]` overlaps the dashboard's window. No billing-cycle alignment, no leap-year correction, `30` is hard-coded as `AVERAGE_DAYS_PER_MONTH`. Cheap and intuitive; less correct than aligning to actual billing dates.

CSV-imported subscription charges use their published dates as-is (no further pro-rating). The import preview deduplicates against manual entries so you don't double-count.

### 4. Cache reads billed at 10% of input

The "Cache hits" strip's `~$Y saved` figure assumes Anthropic's published cache-read price of 10% of the input rate, so `savings = cache_read_tokens × 0.9 × input_usd_per_mtok / 1_000_000`, summed across visible models. Cache writes are billed at 1.25× (5m) or 2× (1h) of input — those costs are real and counted in the counterfactual; only cache *reads* save you money.

---

## What's NOT in the cost picture

- **Time-anchored historical pricing** (see assumption 2). On the open queue.
- **Volume / enterprise / batch discount modeling** — list rates only.
- **Anthropic Admin API ingest** — designed but unbuilt; would let users with org-tier accounts cross-check imported billing against actual API spend.
- **Multi-currency** — USD only. CSV imports in other currencies are not converted.
- **Bedrock / Vertex AI** — third-party hosting markups aren't separated from list rates.

---

## Time zone for daily aggregation

UTC. All `date(occurred_at)` extraction in the SQL aggregation uses UTC YYYY-MM-DD. This matches how the environmental side aggregates and how Anthropic's billing month-boundaries land in practice.

---

## Corrections log

This section dates every correction applied to the cost factor file (`pricing.toml`). It is the cost-side analog to `docs/research-log.md`. Append-only; never rewrite or delete an entry.

### 2026-05-18 — Opus 4.7 / 4.6 + Haiku 4.5 rate corrections

**What was wrong**: `pricing.toml` carried wrong API rates for three of four tracked models since v0.1.0:

| Model | Wrong rate (v0.1.0 – v0.1.10) | Correct rate | Effect on dashboard |
|---|---|---|---|
| `claude-opus-4-7` | $15 / $75 input/output per MTok | **$5 / $25** | Opus-attributed counterfactual cost dropped ~3× |
| `claude-opus-4-6` | $15 / $75 | **$5 / $25** | Opus-attributed counterfactual cost dropped ~3× |
| `claude-haiku-4-5` | $0.80 / $4.00 | **$1.00 / $5.00** | Haiku-attributed cost rose ~25% |
| `claude-sonnet-4-6` | $3 / $15 | $3 / $15 (no change) | — |

Cache-read rates (stored as absolute USD/MTok) recomputed alongside the input rates: Opus 4.7/4.6 → $0.50 (10% of $5); Haiku 4.5 → $0.10 (10% of $1).

**Root cause**: Each row in `pricing.toml` had a `notes` field marking it as a seed value (`"Seed value — pricing assumed unchanged from Opus 4 family. Verify."`) and the file's `file_status = "needs_review"`. Neither marker had teeth — the only enforcement was a soft `warn!` log line at startup that the server ignored. The "Opus 4 family pricing is stable" assumption baked into the seed comments was wrong from the 4.5 generation onward — Anthropic dropped Opus pricing from $15/$75 to $5/$25 starting with Opus 4.5. tokenscale's Phase 1 seed values were never re-verified before shipping.

**Effect on historical net-value figures**: because `PricingFile::lookup()` does not consult `valid_from` (the cost-side time-anchoring gap — see [request-for-research.md](request-for-research.md)'s 6b entry), the corrected rates apply retroactively to every event in history. Users with Opus-dominated usage will see their **counterfactual API cost drop by roughly 3×** on upgrade to v0.1.11, and "Estimated savings vs raw API rates" will drop proportionally. This is a data correction, not a methodology change.

**Effective-date question (deferred to 6b)**: $5/$25 was the real Anthropic price for the entire life of Opus 4.6 and 4.7; stamping it `valid_from = "2026-05-18"` (the v0.1.11 release date) when 6b time-anchoring lands would make a future re-derivation conclude those models had no published price before May 2026 — a smaller version of the seed-value bug. When 6b implements time-anchoring, the corrected rates must be dated to each model's actual launch. The 6b RFR entry carries this note.

**Defensive changes shipped alongside the fix**:

- `pricing.toml` `file_status` flipped to `"production"`.
- `PricingFile::has_seed_markers()` + `EnvironmentalFactorsFile::has_seed_markers()` added in `tokenscale-core`. Both scan every row's `notes` field for phrase-level markers (`seed value`, `unverified`, `needs_review`, `assumed unchanged`) that uniquely identify the bug pattern without false-tripping on legitimate methodology prose like `"medium response assumed at 1,500-2,000 tokens"`.
- CLI startup (`command_serve`) `bail!`s when either gate fails. The v0.1.0–v0.1.10 mechanism was a `warn!` log line; v0.1.11 makes it a hard error. The server refuses to start with unverified data.
- Unit tests in `crates/tokenscale-core/src/pricing.rs` (`the_real_repo_pricing_file_passes_production_gate`, `has_seed_markers_detects_each_phrase`) pin the gate against the live `pricing.toml` and against the literal buggy phrasing — `cargo test` would fail before the binary builds if either condition reappeared.

**Discoverable in-app**: the dashboard's "How is this computed? (4 assumptions)" disclosure links here; users who notice their headline dropped can trace the correction in three clicks.

---

## Where to follow up

- Cost methodology asymmetry: tracked in [`request-for-research.md`](request-for-research.md) as "Cost-side time-anchoring + audit trail."
- Per-model pricing source: [`pricing.toml`](https://github.com/RobarePruyn/tokenscale/blob/main/pricing.toml) (run `tokenscale info pricing` to see what's loaded).
- Subscription tracking format: README's "Configuration" section.
