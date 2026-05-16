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

## Where to follow up

- Cost methodology asymmetry: tracked in [`request-for-research.md`](request-for-research.md) as "Cost-side time-anchoring + audit trail."
- Per-model pricing source: [`pricing.toml`](https://github.com/RobarePruyn/tokenscale/blob/main/pricing.toml) (run `tokenscale info pricing` to see what's loaded).
- Subscription tracking format: README's "Configuration" section.
