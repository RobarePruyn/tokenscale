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

## Drift detector

The companion to the cost-side time-anchoring item on the [open research queue](request-for-research.md). The time-anchoring fix (6b) closes the gap that lets a pricing change retroactively rewrite historical numbers; the **drift detector** is the alarm that tells the maintainer the moment any tracked rate has shifted away from what Anthropic publishes. Both ship together: the detector raises the alarm, the time-anchoring fix makes historical numbers stable. Until 6b lands, the detector at least guarantees no silent drift.

### Source URL

`https://platform.claude.com/docs/en/about-claude/pricing` — Anthropic's dedicated developer reference page for API pricing. This URL was deliberately picked over alternatives:

- **`claude.com/pricing#api`**: marketing landing page. Subscription tiers + a link to the reference page, but no per-model rate table.
- **`platform.claude.com/docs/en/about-claude/models/overview`**: the model comparison page. Has a pricing column but as informational metadata, not as the canonical rate-card source.
- **`platform.claude.com/docs/en/about-claude/pricing`**: the canonical reference. Includes the full `Model pricing` table plus separate sections for Prompt caching, Fast Mode, Batch, Data Residency.

### What the detector validates

1. **Per-model base rates**. `input_usd_per_mtok` and `output_usd_per_mtok` in `pricing.toml` must match the `Base Input Tokens` and `Output Tokens` columns of Anthropic's `Model pricing` table.
2. **Per-model cache rates**. `cache_read_usd_per_mtok` (stored as absolute USD/MTok in `pricing.toml`) must match the `Cache Hits & Refreshes` column. Cache writes are stored as multipliers (`cache_write_5m_multiplier = 1.25`, `cache_write_1h_multiplier = 2.0`); the detector verifies that `input × multiplier` matches Anthropic's `5m Cache Writes` and `1h Cache Writes` columns.
3. **Cache multipliers themselves**. Anthropic's prose says cache_read = 0.1x base input, cache_write_5m = 1.25x, cache_write_1h = 2x. If Anthropic ever changes those constants, the detector catches it via the `EXPECTED_CACHE_MULTIPLIERS` comparison.
4. **Internal consistency** (cache_read only). For every row, `cache_read_usd_per_mtok` must equal `input_usd_per_mtok × 0.1`. This catches the case where a maintainer updates the input rate but forgets to recompute the absolute cache_read value.

#### Why there's no equivalent "internal consistency" check for cache writes

`cache_write_5m_multiplier` and `cache_write_1h_multiplier` are stored as **multipliers** (1.25, 2.0) — primary values, not derived from `input_usd_per_mtok`. There's nothing for them to fall out of sync with internally; an assertion like `cache_write_5m_multiplier == 1.25` would just be comparing a constant to itself. The genuine drift risks for write rates are caught two other ways:

- **Wrong multiplier in `pricing.toml`** (e.g. someone types `1.5` by mistake): caught by check #2 — `input × multiplier` will no longer equal Anthropic's `5m Cache Writes` column.
- **Anthropic changes the multiplier convention** (e.g. 1.25x → 1.5x globally): caught by check #3 — `EXPECTED_CACHE_MULTIPLIERS` no longer matches Anthropic's prose, and check #2 also fires for every row.

A future reader looking for a gap here should not find one. The asymmetry between `cache_read` (one internal check) and the cache writes (no internal check) is a consequence of their different storage formats, not an oversight.

### Parser pinning — what the detector deliberately ignores

Anthropic's pricing page contains multiple price tables. The parser pins to the **base / global-routing** rates and explicitly ignores:

- **Fast Mode** (under `### Fast mode pricing`) — `$30 / $150` for Opus, 6× standard. A naive parser that locked onto the first `$X / MTok` near a model name would silently flag every Opus row as drifted.
- **Batch processing** (under `### Batch processing`) — 50% off, `$2.50 / $12.50` for Opus 4.7.
- **Data Residency** (under `### Data residency pricing`) — 1.1× multiplier for US-only inference.
- **Cloud platform pricing** (Bedrock / Vertex AI) — partner-operated; different pricing structure entirely.

Pinning is enforced via slicing: the parser only reads the content between `Model pricing` and `Cloud platform pricing`. The fixture-based test suite (`.github/scripts/tests/test_pricing_drift_check.py`) includes Fast Mode, Batch, and Data Residency sections in the test fixture and asserts that none of their numbers contaminate the result.

### Where it runs

**GitHub Actions cron, nightly at 02:00 UTC.** Never at `tokenscale serve` startup. The dashboard is local-first; reaching out to Anthropic's web property on every user start would be both a privacy surface (the user's IP hits Anthropic) and a slow-start cost. Detection lives entirely in CI; users find out about drift through release notes once the maintainer publishes a fix.

### Failure modes

Exit codes — `0` success plus three deliberately-split failure modes. The split exists so the wrapping workflow can react differently to drift vs. detector-degradation; collapsing it into a single non-zero would lose that signal.

| Exit code | Meaning | Workflow response |
|---|---|---|
| **0** | Clean. Rates match across all tracked models. | Workflow green. |
| **1** | Real drift detected. `pricing.toml` and Anthropic's published rates disagree. | Workflow opens a `pricing-divergence`-labelled GH Issue with the diff inline, then **fails the workflow** (red X on main, email to maintainer). De-duplicates against an existing open issue. |
| **2** | Parse failure. The detector ran but couldn't extract rates — Anthropic likely restructured the page. | Workflow stays **green** with a console warning. Failing on parse-uncertainty would generate false-positive drift alerts every time Anthropic adjusted page copy. The right response is to update the detector's parser. |
| **3** | Network failure. Couldn't fetch the page at all. | Workflow stays **green** with a warning. Retry tomorrow. |

### Snapshot fallback

`pricing-rate-card.snapshot.json` at the repo root is a checked-in record of Anthropic's rates as of the last manual capture. Two roles:

1. **Staleness check**: if the snapshot is older than 90 days, the detector emits a warning on every run reminding the maintainer to re-verify. 90 days aligns with the quarterly research-sweep cadence.
2. **Offline-fallback (future)**: not used in V1, but the file shape is stable enough that a future variant could compare `pricing.toml` against the snapshot when the live fetch fails, for at-least-internal-consistency detection without network.

To re-capture: open the source URL, copy the rates into the snapshot file, update `captured_at` to today's date, commit. The detector will pick up the new date on the next nightly run.

### Runbook — what to do when the workflow opens a `pricing-divergence` issue

1. **Re-verify against the source.** Open `https://platform.claude.com/docs/en/about-claude/pricing` and confirm the rates the detector reports as upstream are what Anthropic actually publishes. If the page looks unchanged but the detector still flags drift, this is a parser issue — exit 2 territory — and the fix is to update `.github/scripts/pricing_drift_check.py`, not `pricing.toml`.
2. **Update `pricing.toml`.** Correct each flagged row. Cache-read rates are absolute; cache-write multipliers stay at 1.25 / 2.0 unless Anthropic announced a change to those constants.
3. **Re-capture `pricing-rate-card.snapshot.json`** from the live page, updating `captured_at` to today.
4. **Add a dated correction entry to the [Corrections log](#corrections-log)** above. Append-only — this is the audit trail.
5. **Tag a new release** so the corrected file ships to users. Pricing changes are user-visible (counterfactual cost shifts), so the release notes should call out the magnitude of the change.
6. **Close the issue** once the new release lands and the next nightly detector run reports clean.

### What the detector deliberately does NOT do (V1 scope)

- **Auto-bump `pricing.toml` file_status to `needs_review`.** Would brick every running tokenscale instance the moment Anthropic touched a price; brittle. Manual maintainer action stays in the loop.
- **Surface drift on the user-facing dashboard.** That's part of the 6b time-anchoring workstream (the dashboard already has the cost-methodology panel; drift surface can be added there once it can also describe which historical events are affected). V1 keeps drift detection between maintainer and CI.
- **Detect new models.** The detector iterates `TRACKED_MODELS` only — a constant in the script. New-model alerting is a v2 enhancement: useful, but a separate problem from "did existing rates drift."

---

## Where to follow up

- Cost methodology asymmetry: tracked in [`request-for-research.md`](request-for-research.md) as "Cost-side time-anchoring + audit trail."
- Per-model pricing source: [`pricing.toml`](https://github.com/RobarePruyn/tokenscale/blob/main/pricing.toml) (run `tokenscale info pricing` to see what's loaded).
- Subscription tracking format: README's "Configuration" section.
