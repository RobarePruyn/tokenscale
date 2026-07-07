# Full codebase assessment, 2026-06-16

**Purpose**: current-state audit feeding the full-model-coverage workstream (every Anthropic model ever released, with specific timelines) and the cost-methodology deep review. Produced by four parallel read-only assessment passes (core math crate; store SQL layer; server/cli/ingest crates; frontend/detector/docs) plus maintainer-session spot verification of the load-bearing claims. Findings marked VERIFIED were independently re-checked against source.

**Companion**: `docs/roadmap-full-model-coverage.md` (the scoping doc this assessment feeds).

---

## 1. Verdict

The architecture is sound for the expansion: pricing and environmental factors are fully independent lookups (separate LEFT JOINs, separate missingness counters), which structurally enforces "unpriced is not unfactored." Adding model rows is mostly pure data. But the assessment surfaced six correctness landmines, one central design gap (exact-string model matching, Issue #7), a detector-scaling failure mode that would blind drift detection entirely, and a frontend display layer that degrades at historical-model scale. None are blockers; all are now on the record with file:line cites.

## 2. Correctness landmines (fix before or during the expansion)

1. **Join fan-out with no UNIQUE guard (highest priority).** The aggregate joins resolve pricing/factors via `valid_from = (SELECT MAX(valid_from) ...)` equality with no UNIQUE constraint on `(provider, model, valid_from)` (`impact_query.rs:333`, `sessions_query.rs:304`; tables at `migrations/20260428000001_initial.sql:87-128`). Two rows sharing a valid_from match BOTH and every cost/energy SUM double-counts silently. Hand-authoring 30+ rows makes a duplicate-valid_from typo likely. Hardening: UNIQUE index on `(provider, model, valid_from)` and `(region, valid_from)` plus a duplicate-fan-out regression test.
2. **Unfactored models read as zero energy, not missing.** `energy_wh`/`facility_wh` are non-optional and COALESCE to 0 (`impact_query.rs:181-200`), unlike cost/CO2e/water which go `None` and render as the missing-value placeholder. A used-but-unfactored model silently shows 0 Wh next to real dollar figures. Conflicts with the impact-coverage principle; candidate fix is Option-izing energy like the others or surfacing `events_missing_env_factor` more loudly.
3. **Division by zero in billable multipliers.** VERIFIED: `BillableMultipliers::from_pricing` divides output and cache_read rates by `input_usd_per_mtok` with no zero guard (`billable.rs:36-37`). Any historical row entered with input = 0.0 yields NaN/inf weights. Guard or document a no-zero-input rule for row authors.
4. **Silent 1970-01-01 default for factor valid_from.** `factors_sync.rs:59,97` defaults a missing `valid_from` to `1970-01-01` ("always in effect"), which can shadow correct time-anchoring. Pricing sync has no such default (missing field = parse error, safer). Historical factor rows MUST carry explicit valid_from.
5. **The audit gate resolves differently from the query path.** `audit_pricing_launch_dates` uses `MIN(valid_from)` with no date filter (`audit.rs:81-98`); the query path uses `MAX(valid_from) <= date(occurred_at)`. Both are correct for their purposes today, but any normalization layer must be applied to all resolution sites (2 aggregate joins x 2 files, 2 single-row helpers, the audit subquery, core in-memory lookups) or the audit reports phantom unpriced pairs.
6. **JOINs ignore valid_to; single-row helpers honor it.** Masked only because sync always writes `valid_to = NULL` (`pricing_sync.rs:69`, `factors_sync.rs:67,110`). If any future change populates valid_to, the aggregate path and helpers diverge. Keep history expressed as multiple valid_from rows; never populate valid_to without fixing the joins.

## 3. The central design gap: exact-string model matching (Issue #7)

Every resolution surface matches `events.model` by exact string: SQL joins (`impact_query.rs:317,332`; `sessions_query.rs:294,303`), audit (`audit.rs:83-98`), in-memory lookups (`pricing.rs:248`, `factors.rs:275`), server banners (`usage.rs:71-90`). No normalization code exists anywhere (repo-wide grep confirmed). The dated-Haiku alias rows in both TOMLs are the interim workaround.

Normalization hook options assessed (blast radius in parentheses):
- **A. Sync-time alias-row duplication** (current de-facto; data-only; does not scale)
- **B. Normalize the join key in SQL** (~7 SQL fragments across 3 files kept in lockstep; SQLite needs a registered function)
- **C. `model_aliases(raw, canonical)` mapping table** (new migration + one extra JOIN per site; data-driven)
- **D. Normalize at ingest** (touches `parser.rs:389` and `content_hash` which HASHES the model string, so re-hashing changes dedupe keys; widest blast radius; would need raw preserved separately)
- **E. Registered SQL scalar function strip/normalize** (central definition in `database.rs:42-65`; still referenced at every site)

Store-layer recommendation carried into scoping: **C or E applied to the resolution key only**, leaving `events.model` and every `GROUP BY events.model` raw so display keys stay faithful to emitted data. Ingest-time (D) is disfavored due to the content-hash coupling.

Empirical anchor from Anthropic's model docs (fetched 2026-06-16): every model ID is a pinned snapshot; 4.6+ IDs are dateless-but-pinned; pre-4.6 the undated names are ALIASES that resolve to dated IDs. So real emitted usage for pre-4.6 models carries dated IDs, which matches the DB evidence (dated `claude-haiku-4-5-20251001`, undated `claude-opus-4-8`). Row keys must therefore be dated IDs for pre-4.6 history and dateless IDs for 4.6+, with aliases handled by the normalization layer.

## 4. Gates and traps for the historical backfill

- **Pricing rows require all five rate fields** (`pricing.rs:148-170`; only `launch_date_source`/`notes` optional). Pre-prompt-caching models (Claude 1.x, Instant, 2.x, and Claude 3 rows predating caching GA) have no real cache prices; a placeholder policy is a D-decision. Factor rows are the opposite: only `display_name` is required, all Wh fields nullable, so pre-caching factor rows are trivially expressible.
- **Seed-marker phrase trap.** VERIFIED: the startup gate trips on `"assumed unchanged"`, `"seed value"`, `"unverified"`, `"needs_review"` in any notes field (`pricing.rs:300`, `factors.rs:307`), refusing server start. Natural historical-row phrasing like "cache pricing assumed unchanged from Claude 2.1" would brick startup. Safe in-tree phrasings: "ESTIMATE", "conservative estimate", "pricing-as-proxy". ALSO VERIFIED: the CLI bail message (`main.rs:255-256`) lists the WRONG phrases (claims bare "seed"/"assumed"/"verify" trip; they do not) and will mislead whoever edits rows. Fix the message.
- **Audit-gate interaction.** Adding a pricing row moves a model from the non-gated "unpriced pairs" bucket into the gated bucket: any event dated before the row's earliest valid_from hard-fails `audit pricing-launch-dates` (`main.rs:669-678`). Every historical row must satisfy valid_from <= earliest possible event date, which the D3 bias-earlier dating rule already guarantees if followed.
- **In-memory factors are single-row-per-model** (`factors.rs:90`, no Vec, no date arg on `lookup_model` at `factors.rs:274-276`) while pricing is multi-row time-anchored. A factor value that changes over time for one model key cannot be expressed in the TOML today. Latent asymmetry; matters only if a historical model needs era-split factors.
- **`file_status` defaults to "production"** when omitted (`pricing.rs:99-101`, `factors.rs:78-80`), skipping review gates. Backfill edits must keep the field explicit.

## 5. Detector scaling (the second-highest-priority finding)

The nightly drift detector iterates TRACKED_MODELS and raises ParseFailure for any tracked model missing from the live page unless marked retired (`pricing_drift_check.py:206-222`). Most historical models were NEVER on the live pricing page. Tracking them naively means the first missing model flips every nightly run to exit-2, and `check_drift` never executes: **the detector goes permanently blind on the live models while appearing green-but-degraded**. Abusing `status = "retired"` for never-listed models would silence it but conflates three lifecycle states (pulled-but-listed Fable; delisted-legacy; never-listed historical).

Scoping direction: make page-presence the tracking axis (a page_listed marker or equivalent split), keep the retired-skip only for the genuine pulled-while-listed case, scope the fixture contract (`test_pricing_drift_check.py:80-86` forces a fixture row per tracked model) to page-listed models, and exempt historical models from snapshot capture (the snapshot currently earns nothing beyond a staleness warning; `pricing_drift_check.py:428-440`). The module docstring is already stale ("the four models actually tracked", `pricing_drift_check.py:38-39`).

## 6. Frontend at historical scale

- `modelDisplayName()` only matches `^claude-(opus|sonnet|haiku)-(\d+)-(\d+)$` (`App.tsx:698-707`): every historical ID, every dated ID, and `claude-fable-5` TODAY render as raw machine strings. Highest-value single frontend fix; should share the normalization/display mapping.
- Family palettes carry 4 shades (`App.tsx:450-455`) with order-dependent assignment (`App.tsx:1311-1328`): 7+ Opus versions guarantee hue collisions, and a model's color changes depending on which other models are visible. Needs version-aware deterministic assignment plus a client-side sort (none exists; server order is inherited, `App.tsx:1258`).
- Legends, chips, and the two missing-data footnotes render unbounded lists (`App.tsx:3861-3873`, `3987`, `2075`, `2566`); at 30+ models these need collapse/cap treatment.

## 7. Staleness and corrections

- **`docs/cost-methodology.md` assumption 2 is false.** VERIFIED: both the DB aggregation path AND the in-memory `PricingFile::lookup` are time-anchored (`pricing.rs:242-254` takes `as_of_date`; callers pass the bucket date at `usage.rs:440-448`). The "current pricing is applied retroactively ... the big one" section predates v0.1.13 and must be rewritten. The corrections log in the same file already documents the fix that obsoletes it.
- **Correction to a claim made in-session by the assistant**: `<synthetic>` events originate from Claude Code's own JSONL (passed through the parser verbatim), NOT from Admin API ingest; `tokenscale-ingest-api` is an empty placeholder crate (`lib.rs` doc-comment plus `it_compiles` test only). The cost-methodology line "Admin API ingest, designed but unbuilt" is accurate; the earlier in-session statement that it was stale is withdrawn.
- Parser notes for historical JSONL: `message.model` is required with no default (missing model = Malformed line drop, counted); absent `cache_creation` breakdown buckets all cache-write as 5m (`parser.rs:363-375`); usage cache fields default to 0 so pre-caching-era lines parse cleanly. CONFIRMED structurally: tool-call `input.model` strings (the bare `sonnet` case) can never become usage events; `Event.model` sources exclusively from `AssistantMessage.model` (`parser.rs:143,389`).
- Coverage sets already diverge between the two TOMLs: env-factors carries Opus 4.5/Sonnet 4.5 rows that pricing.toml lacks; pricing carries Opus 4.8 variants factored differently. Reconciliation is part of the expansion. Non-anthropic factor rows (google/openai/meta/deepseek/mistral) are unreachable by the join path because `sources.provider` is always `anthropic`; they are reference data only.
- Doc claims that go stale with all-model coverage, to be updated in the build: `environmental-factors.toml:10-13` coverage enumeration; `docs/cost-methodology.md:202-206` detector scope; `docs/methodology.md:57-66` single-derivation-path framing; `docs/sources.md:149,361,388-389` older-models-are-direct-evidence framing (which INVERTS: Jegham et al. covers older Claude generations directly, so historical rows may carry stronger anchors than current ones).

## 8. What all-model support requires (summary)

- **Pure data**: pricing + factor rows for the full model universe (see the coverage roadmap), each with sourced rates, exact-or-conservative valid_from, gate-safe notes phrasing, and reconciled key sets across both files.
- **Code, required**: normalization layer (Issue #7; option C or E), detector page-listed split, UNIQUE fan-out hardening + regression test.
- **Code, strongly recommended**: display-name mapping for historical/dated IDs; deterministic model ordering + version-aware palette; seed-marker CLI message fix; energy 0-vs-missing asymmetry decision.
- **Docs**: the staleness list in §7, plus rewriting cost-methodology assumption 2 to match shipped reality.
