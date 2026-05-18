# Roadmap — Cost-side time-anchoring (v0.1.13, the 6b core fix)

**Status**: scoping only. **No implementation until the design decisions below are signed off.**

This is the structural fix the drift detector (v0.1.12) is the alarm for. Closes the asymmetry between the environmental side (already time-anchored per-event via correlated subqueries on `valid_from`) and the cost side (single-row-per-model lookup, current rates applied retroactively to all historical events).

The hard trigger in [`request-for-research.md`](request-for-research.md): **time-anchoring must land before the next Anthropic price change**, or historical net-value numbers silently rewrite on every pricing refresh.

---

## 1. What changes in the data layer

### The lookup path

Today's call site (`tokenscale-core::pricing::PricingFile::lookup`):

```rust
pub fn lookup(&self, provider: &str, model: &str) -> Option<&ModelPricing>
```

After v0.1.13:

```rust
pub fn lookup(&self, provider: &str, model: &str, as_of_date: &str) -> Option<&ModelPricing>
```

The shape mirrors `tokenscale-store::factors_lookup::lookup_environmental_factors` ([factors_lookup.rs:40-69](crates/tokenscale-store/src/factors_lookup.rs)) which uses `ORDER BY valid_from DESC LIMIT 1` with `AND valid_from <= ?`.

### Loader

`PricingFile` currently stores `models: BTreeMap<String, ModelPricing>` (one row per model). After v0.1.13 it stores `BTreeMap<String, Vec<ModelPricing>>` ordered by `valid_from` ascending. The TOML schema needs an array-of-tables form for multi-row support:

```toml
[[providers.anthropic.models."claude-opus-4-7"]]
valid_from         = "2026-01-15"        # Opus 4.7 launch date
input_usd_per_mtok = 5.00
output_usd_per_mtok = 25.00
# ... etc

# A future row when Anthropic changes Opus 4.7 pricing:
[[providers.anthropic.models."claude-opus-4-7"]]
valid_from         = "2027-XX-XX"
input_usd_per_mtok = X.XX
# ...
```

Trade-off: the existing single-table form (`[providers.anthropic.models."claude-opus-4-7"]`) is more readable for the common case. The array-of-tables form is required only when a model has more than one historical price. **Recommended schema**: support both — keep single-table form for legacy / single-row entries, add array form for multi-row. The loader normalizes to `Vec<ModelPricing>` internally.

### Schema and migration

A new `pricing` DB table mirrors `env_factors`. See section 2 for the side-by-side.

### Files that have to move to dated resolution

| Caller | Today's call | After v0.1.13 |
|---|---|---|
| [`routes/usage.rs:335`](crates/tokenscale-server/src/routes/usage.rs) — per-row pricing in the bucket loop | `state.pricing.lookup(provider, model)` | Time-anchored via SQL join in `aggregate_impact_by_bucket` (no Rust-side lookup needed). |
| [`routes/usage.rs:414`](crates/tokenscale-server/src/routes/usage.rs) — `pricing_by_model` dict for response | `state.pricing.lookup(provider, model_id)` for one rate per model | Returns *all* `(valid_from, rate)` tuples for the window, OR per-bucket pricing. **Design D6 below.** |
| [`routes/factors.rs:active_handler`](crates/tokenscale-server/src/routes/factors.rs) — `/api/v1/factors/active` panel | Lists all pricing rows currently | Lists all `valid_from`-versioned rows. Provenance panel becomes more accurate. |
| `tokenscale-store/src/impact_query.rs` `aggregate_impact_by_bucket` SQL | No pricing in the SQL (Rust-side after) | Adds pricing LEFT JOIN with correlated subquery on `valid_from`, mirroring the existing `env_factors` and `grid_factors` joins. |

---

## 2. Parity with the environmental side

`env_factors` solved this exact problem. The cost side should reuse the pattern directly wherever it fits.

### Where pricing matches `env_factors` exactly

| Concern | `env_factors` (existing) | `pricing` (after v0.1.13) |
|---|---|---|
| **Schema** | `(provider, model, valid_from, valid_to, ... rates ...)` indexed on `(provider, model, valid_from)` | Same: `(provider, model, valid_from, valid_to, ... rates ...)` indexed identically |
| **Per-event lookup** | Correlated subquery: `valid_from = (SELECT MAX(valid_from) WHERE provider=? AND model=? AND valid_from <= date(events.occurred_at))` | Same pattern, swap table name |
| **Sync from TOML** | `factors_sync::sync_environmental_factors` deletes and re-inserts all rows on each startup | Mirror: `pricing_sync::sync_pricing` |
| **Single-row lookup helper** | `lookup_environmental_factors(db, provider, model, as_of_date)` returns `Option<ModelFactors>` | Mirror: `lookup_pricing(db, provider, model, as_of_date)` returning `Option<ModelPricing>` |
| **File-level metadata in memory** | `EnvironmentalFactorsFile` keeps file_version / methodology / etc. for the dashboard banner | `PricingFile` keeps schema_version / file_version (new) / sources |
| **Health endpoint surfaces** | `/api/v1/health` reports `environmental.file_version`, `environmental.model_count`, etc. | Mirror: pricing block already has `pricing.model_count` etc; add `pricing.file_version` |

### Where pricing genuinely differs

| Concern | env | pricing | Why different |
|---|---|---|---|
| **Cache-rate storage** | n/a | `cache_read` absolute, `cache_write_*` as multipliers | Pricing's per-token cache convention is unique; env has no analog |
| **`/api/v1/usage/daily` pricing dict** | n/a (env rates are per-bucket via SQL) | Today's response includes a top-level `pricingByModel` dict for the frontend's Cost (USD) view | Frontend code consumes a single rate per model; time-anchored model needs schema design (D6) |
| **Default-region attribution** | env has `configured_region` (`us-east-1` etc.) for grid factor selection | n/a — pricing isn't region-dependent | Cost is a function of the model + date, not the user's regional config |

### Recommendation

**Cost side adopts the env-side pattern verbatim** for storage / sync / per-event lookup. Schema columns differ (pricing has no PUE / water / CO₂e) but the structural pattern is identical. The dashboard's `pricingByModel` dict (D6) is the only place where the cost side genuinely needs a different API surface — pricing is consumed for a Cost (USD) chart axis the env side doesn't have an analog of.

---

## 3. Backfill of the corrected rates

This is the part the user flagged in v0.1.11's RFR effective-date subsection: the corrected rates that v0.1.11 shipped must NOT all be dated to a single recent date when multi-row support lands.

### Current single-row state

After v0.1.11:

| Model | Current `valid_from` | Real launch (verify before v0.1.13 ships) |
|---|---|---|
| `claude-opus-4-7` | `2026-04-28` | **Requires real source at Phase D.** (Knowledge cutoff dates are not launch dates and must not be used to infer one.) |
| `claude-opus-4-6` | `2026-04-28` | ~Sep 2025 (Bedrock ID `anthropic.claude-opus-4-6-v1`) |
| `claude-sonnet-4-6` | `2026-04-28` | ~Sep 2025 |
| `claude-haiku-4-5` | `2026-04-28` | 2025-10-01 (Bedrock ID `claude-haiku-4-5-20251001`) |

### Backfill rule

When v0.1.13 lands, **each model's existing row gets its `valid_from` rewritten to the model's actual launch date.** This means:

- A `claude-haiku-4-5` event from 2025-11-15 resolves to `valid_from = 2025-10-01` (correct: $1/$5).
- A `claude-haiku-4-5` event from 2025-09-15 resolves to no pricing match (the model didn't exist yet — see section 4 for the behavior).

### Critical anti-pattern to avoid

**Do NOT use `valid_from = "2026-05-XX"` (the v0.1.13 release date).** Doing so would record that Anthropic published these rates on the v0.1.13 release date, which is false. The rates were always $5/$25 for Opus 4.6+; v0.1.11 just corrected our copy of them. A future re-derivation reading the rewritten file would conclude these models had no published price before v0.1.13's date — a smaller version of the original seed-value bug.

### Verification expected at implementation time

Each launch date in the backfill PR must be either:
- An Anthropic-stated launch date (release notes, blog post), with the URL committed alongside the row, OR
- A defensible inference from Bedrock / Vertex AI model-ID conventions (e.g. `claude-haiku-4-5-20251001` strongly implies 2025-10-01), with the source documented.

If neither is available for a model, **the maintainer flags it in the backfill PR rather than guessing a date.** The corrections log in `docs/cost-methodology.md` records the date and the source per row.

---

## 4. Historical recompute behavior

### Goal

Once dated resolution is live, historical numbers must NOT move again. They shifted ~3× for Opus users at v0.1.11; that was the only legitimate retroactive shift the cost data should experience. v0.1.13 must produce identical numbers for every past event.

### What changes for events with a model that has only one historical row

Today (single-row): every Opus 4.7 event applies $5/$25.

After v0.1.13 (multi-row, but Opus 4.7 still has only one row dated to its launch): every Opus 4.7 event from on-or-after the launch date applies $5/$25.

**Identical** for any event ingested today (all such events are from after the model launched). ✅

### What changes for events that PREDATE a model's launch

Today: such events still get a pricing match (incorrectly — $5/$25 applied to a 2024 event when the model didn't exist).

After v0.1.13: such events get no pricing match (correctly).

**Different** — but this is a correctness improvement, not a regression. An event can only predate its model's existence if data is corrupted or the date is mis-attributed; in practice this should be observable in zero or near-zero events. Flag for QA at implementation time.

### What happens if Anthropic does change a price in the future

After v0.1.13:
1. Drift detector (v0.1.12) flags the divergence within 24 hours.
2. Maintainer adds a NEW row to `pricing.toml` with the new `valid_from` = announcement-date.
3. Historical events before that date keep using the old row.
4. New events use the new row.
5. Historical numbers do not move.

This is the property the workstream exists to establish.

### Edge case: events on the `valid_from` boundary

The env side uses `valid_from <= occurred_at`, so an event AT the `valid_from` timestamp gets the new rate. Pricing should follow the same convention. Tests should cover (a) event one second before `valid_from`, (b) event at exactly `valid_from`, (c) event one second after — to confirm the boundary behaves as documented.

---

## 5. Versioned pricing file

### Current state

`pricing.toml` has:
- `schema_version = 1`
- `file_status = "production"`
- No `file_version`
- No `file_published` / `methodology` fields

Compare `environmental-factors.toml`:
- `schema_version = 1`
- `file_status = "production"`
- `file_version = "0.3"` (bumped per Sweep)
- `file_published = "2026-05-15"`
- `methodology = "google-comprehensive-aug-2025"` (not applicable to pricing)

### Recommendation

Bump `pricing.toml` to:
- `schema_version = 1` unchanged (schema is the table set; if we add an `(array-of-tables)` form for multi-row pricing, schema_version stays 1 because old single-row files still load — back-compat)
- `file_version = "1.0"` (NEW). The "time-anchored era" starts at 1.0. Future pricing changes bump minor (1.1, 1.2 ...) and major if the audit trail rules change.
- `file_published = "2026-05-XX"` (NEW; date of the v0.1.13 release; this is the "when did this version of the file ship," not the validity-window of any single row inside it).

Audit trail mirror: `docs/cost-methodology.md`'s "Corrections log" already serves this purpose. Existing 2026-05-18 entry stays; a new entry dated to v0.1.13 release documents the time-anchored migration.

---

## 6. Phasing

Five phases. Phases A–C are sequential and must land in one release tag (they form the structural change). D and E can be split if needed, but for credibility — historical numbers must not move during the workstream — **D should ship in the same tag as A–C.** E is documentation and can be a fast-follow.

### Phase A — In-memory multi-row support (one-shot)

**Goal**: `PricingFile::lookup` takes an `as_of_date` and returns the row whose `valid_from <= as_of_date` is latest. `pricing.toml` supports both single-table and array-of-tables forms.

**Files**:
- `crates/tokenscale-core/src/pricing.rs` — change `models: BTreeMap<String, ModelPricing>` to `BTreeMap<String, Vec<ModelPricing>>`, update `lookup`, update `parse`/`load_from_path` to accept either TOML form.
- `pricing.toml` — no schema change yet, single-row entries still valid.
- Tests: cover both TOML forms, cover edge-case `valid_from` boundary lookup.
- Call sites at [`routes/usage.rs:335,414`](crates/tokenscale-server/src/routes/usage.rs) — pass the bucket date for now (later moved into SQL).

**Prereqs**: none.

**Decisions blocking**: D1 (storage choice — informs whether this phase is in-memory permanently or stepping stone to DB).

### Phase B — DB table + sync (one-shot, depends on A)

**Goal**: pricing lives in a `pricing` DB table, mirroring `env_factors`. Synced from TOML on startup.

**Files**:
- NEW migration `migrations/20260520000001_pricing_table.sql`. Schema mirrors `env_factors`. Columns: `id, provider, model, valid_from, valid_to, input_usd_per_mtok, output_usd_per_mtok, cache_read_usd_per_mtok, cache_write_5m_multiplier, cache_write_1h_multiplier, source_doc, notes`. Index on `(provider, model, valid_from)`.
- NEW `crates/tokenscale-store/src/pricing_sync.rs` — mirror of `factors_sync.rs`. Replace-on-startup pattern.
- NEW `crates/tokenscale-store/src/pricing_lookup.rs` — mirror of `factors_lookup.rs`. `lookup_pricing(db, provider, model, as_of_date) -> Option<ModelPricing>`.
- CLI startup (`command_serve`) calls `sync_pricing` after `sync_environmental_factors`.

**Prereqs**: Phase A landed.

**Decisions blocking**: D1.

### Phase C — Per-event SQL pricing in `aggregate_impact_by_bucket` (one-shot, depends on B)

**Goal**: cost-per-event computed in the SQL aggregate path, time-anchored via correlated subquery. Server's `aggregate_impact_by_bucket` returns per-(bucket, model) cells with cost already attached. Frontend no longer needs `pricingByModel` for the Cost (USD) view if cost arrives pre-computed; provenance panel can still surface the underlying rates.

**Files**:
- `crates/tokenscale-store/src/impact_query.rs` — add pricing LEFT JOIN with correlated subquery on `valid_from`, compute per-event input cost + output cost + cache costs in SQL, sum at the (bucket, provider, model) granularity. New columns on `ImpactByBucketRow`: `cost_usd_input`, `cost_usd_output`, etc. — OR a single `cost_usd_total` plus per-token-type breakdown that mirrors the existing `billable` breakdown.
- `crates/tokenscale-server/src/routes/usage.rs` — drop the Rust-side `pricing.lookup` calls. `pricingByModel` becomes optional (D6).
- Frontend: `Cost (USD)` view reads `cost_usd_total` directly from the server response instead of computing client-side from `billable × input_rate`.

**Prereqs**: Phase B landed.

**Decisions blocking**: D2, D6.

### Phase D — Backfill correct launch dates (one-shot, MUST ship with A+B+C)

**Goal**: `pricing.toml` rewritten with verified launch dates per model. NO release-date stamps on rates Anthropic published months/years earlier.

**Files**:
- `pricing.toml` — rewrite each row's `valid_from` to the verified launch date. Add a `launch_date_source = "..."` field per row with the URL of the Anthropic release notes / blog post / Bedrock ID convention that justifies the date. If a date is genuinely unknown, the row is flagged and the maintainer decides per-row.
- `pricing-rate-card.snapshot.json` — re-capture (snapshot age check expects this).
- `docs/cost-methodology.md` Corrections log — new dated entry documenting the backfill.

**Prereqs**: Phases A and B landed (so the loader can read multi-row TOML; otherwise this commit breaks `cargo test`).

**Decisions blocking**: D3.

### Phase E — File version + audit trail discipline (fast-follow, can ship in same tag)

**Goal**: `pricing.toml` carries `file_version = "1.0"` and a `file_published` date. Corrections log discipline documented.

**Files**:
- `pricing.toml` — add the two fields.
- `crates/tokenscale-core/src/pricing.rs` — add `file_version` and `file_published` to the `PricingFile` struct.
- `crates/tokenscale-server/src/routes/health.rs` — surface them on `/api/v1/health` like environmental side does.
- `docs/cost-methodology.md` — explicit "file_version bump cadence" note.

**Prereqs**: Phase D landed.

**Decisions blocking**: D5.

---

## Design decisions blocking implementation

Sign-off (or pushback) needed on each before v0.1.13 implementation starts.

### D1 — Storage architecture

**Question**: Promote pricing to a DB table mirroring `env_factors`, or keep it in-memory with multi-row support?

**Options**:
- **(a) DB table** (recommended): exact parity with env side, per-event SQL lookup, single-source-of-truth pattern. Cost: schema migration + sync code.
- **(b) In-memory only**: simpler. Pricing data is small (~4 models × few rows). Per-event lookup happens in Rust after the SQL aggregate. Cost: code asymmetry with env side; per-event Rust lookup is more code than per-event SQL join.

**Recommendation**: (a). The asymmetry argument is the workstream's whole point.

### D2 — Lookup granularity

**Question**: Time-anchor per-event (mirroring env) or per-(bucket, model) cell (simpler)?

**Options**:
- **(a) Per-event** (recommended): correlated subquery on each event's `occurred_at`. Same shape as env.
- **(b) Per-(bucket, model)**: one lookup per cell, using the bucket date. Misses within-bucket transitions.

**Recommendation**: (a). Within-bucket transitions are rare (pricing changes happen at most a few times a year) but the whole point of time-anchoring is correctness at the edges. Per-bucket would technically defeat the workstream's purpose for events on either side of a `valid_from` within the same bucket.

### D3 — Who verifies each model's launch date during Phase D

**Question**: Anthropic doesn't always publish exact launch dates. How are they sourced?

**Options**:
- **(a) Anthropic release notes / blog posts** (when available) — primary source.
- **(b) Bedrock / Vertex AI model-ID date stamps** (e.g. `claude-haiku-4-5-20251001`) — strong inference when the date is encoded in the ID.
- **(c) Maintainer-stated estimate, flagged as such in the corrections log** — fallback when neither (a) nor (b) are available.

**Recommendation**: prefer (a), use (b) as secondary, use (c) only when both fail and document the uncertainty in the corrections log. Each row's `launch_date_source` field carries the URL or rationale.

#### Conservative-dating rule for option (c) — date EARLIER, not later

A guessed launch date silently generates errors: every event between the real launch and a wrong guessed date resolves incorrectly. The two error directions have very different failure modes:

- **An over-early `valid_from`** (guess is before the real launch): a small number of genuinely-pre-launch events get priced as if the model existed. In practice these are zero or near-zero (events can't predate their model). If any do exist, they're visible — the dashboard shows pricing for a model that wasn't yet available, which a reviewer would notice.
- **An over-late `valid_from`** (guess is after the real launch): real events from the gap between true launch and our guessed-late date get NO pricing match. They silently drop from the cost view. **This is the exact failure mode that started this work** — events processed without their correct pricing context.

**Rule**: when a launch date is uncertain, **date the row earlier than the best guess, not later.** Concretely, if the maintainer's confidence interval is "launched somewhere between 2025-09-01 and 2025-09-30," set `valid_from = 2025-09-01` (or earlier — picking the start of the model's announcement quarter is a safe heuristic).

This bias is recorded in the corrections-log entry alongside `launch_date_source`. A future re-verification that produces an exact date should tighten the row's `valid_from`; the audit trail keeps the original conservative guess visible.

### D4 — Frontend behavior when no pricing match (pre-launch event, or model removed)

**Question**: What does the dashboard show for an event whose `(provider, model, occurred_at)` finds no pricing row?

**Options**:
- **(a) `null` / "—" in the cost cells** (recommended): mirrors env-side behavior when no factor row matches. Honest about missingness.
- **(b) Fall back to the latest available row** (extrapolate forward only): convenient but misleading.
- **(c) Fall back to any row**: silently wrong.

**Recommendation**: (a). Honest. The `modelsWithoutFactors` array on `/api/v1/usage/daily` already has an analog the dashboard surfaces.

### D5 — `file_version` bump strategy

**Question**: What does `file_version` mean for pricing, and when does it bump?

**Options**:
- **(a) Semver-ish (`1.0` → `1.1` → ...)** (recommended): minor bumps when rates change, major bumps when schema changes. Mirrors environmental-factors.toml's `0.3`-style cadence.
- **(b) Tied to release version**: confusing — `pricing.toml` version diverges from `tokenscale` version regularly.
- **(c) Pure date (`2026-05-18`)**: redundant with `file_published`.

**Recommendation**: (a). Starts at `1.0` with v0.1.13's time-anchored era.

### D6 — `pricingByModel` API surface

**Question**: The `/api/v1/usage/daily` response currently includes `pricingByModel: { <model>: { input_usd_per_mtok } }` — a window-wide dict the frontend uses for the Cost (USD) view. With time-anchored pricing, what does this become?

**Options**:
- **(a) Drop the dict; server pre-computes cost in SQL** (recommended): aligns with the Phase C change where cost arrives per-event in the response.
- **(b) Per-bucket dict**: `{ <bucket>: { <model>: { input_usd_per_mtok } } }`. Cumbersome.
- **(c) Per-`valid_from` slice**: include all historical rates with their `valid_from` ranges; frontend picks per bucket. Complex client-side logic.

**Recommendation**: (a). Cost moves into the per-bucket-row payload, computed in SQL with time-anchoring. The provenance panel (`/api/v1/factors/active`) keeps a full list of all `(valid_from, rate)` rows for audit-trail surfacing.

### D7 — Rollback and migration-safety for the A–D bundle

**Question**: A–D land atomically as one v0.1.13 tag. Phase B introduces a schema migration, Phase D rewrites pricing data. What's the down-path if (i) the schema migration misbehaves in production, (ii) a backfilled launch date is wrong, or (iii) a user binary-downgrades from v0.1.13 back to v0.1.12?

#### Environmental-side precedent (mirror this)

The env side established the precedent when its factor tables first landed: **forward-only migrations + `sqlx::migrate!()` runtime macro + replace-on-startup sync** ([`crates/tokenscale-store/src/database.rs:61`](crates/tokenscale-store/src/database.rs#L61) calls `sqlx::migrate!("../../migrations").run(&pool)` on every startup). The `migrations/` directory has no `down.sql` companions and no revert files; every migration is forward-only. Schema evolution happens via additive columns or recreate-and-copy patterns (see the v0.1.0 → v0.1.1 grid_factors recreate at [migrations/20260429000001_phase2_factor_columns.sql:50-72](migrations/20260429000001_phase2_factor_columns.sql)). Data evolution happens via `sync_environmental_factors` rewriting the table from `environmental-factors.toml` on every server start.

**v0.1.13 mirrors this verbatim.** No new migration discipline; no down migrations; no schema-versioning innovation. Rollback safety comes from the replace-on-startup pattern + the fact that a desktop SQLite DB is a per-user disposable artifact that can be rebuilt from JSONL re-scan.

#### Per-bullet answer

**(a) Is the Phase B migration `20260520000001_pricing_table.sql` reversible?**

In principle yes — the migration creates a NEW table (`pricing`), leaves env_factors and grid_factors untouched, and doesn't modify or drop any existing schema. A down migration would be `DROP TABLE pricing;` plus `DROP INDEX IF EXISTS pricing_provider_model_valid_from_idx;`. **In practice, no down migration is shipped** — that's not the project's pattern, and the env-side precedent rejected it.

The recovery move if the migration itself misbehaves in production (rare — `CREATE TABLE` on a fresh table is hard to break): the maintainer ships a forward fix-up migration (e.g. `20260521_pricing_table_repair.sql`) that corrects the broken state. Same pattern as any other schema bug.

**(b) Does a binary downgrade from v0.1.13 → v0.1.12 leave a working state?**

Two paths matter — database state and `pricing.toml` state.

**Database state**: When v0.1.12's `sqlx::migrate!()` runs against a DB that already has the v0.1.13 `pricing_table` migration applied, sqlx's default behavior (no `MigrateError` raised) is to leave the unknown-applied migration as-is and run any later un-applied migrations from the binary's embedded set (none, since we're downgrading). The `pricing` table sits in the SQLite file as an orphan; v0.1.12's code doesn't query it. No corruption, no data loss. The orphan resolves itself when the user upgrades back.

**`pricing.toml` state**: This is the harder path. v0.1.13 introduces an array-of-tables form (`[[providers.anthropic.models."claude-opus-4-7"]]`) for multi-row pricing. v0.1.12's serde deserializer expects single-table form (`[providers.anthropic.models."claude-opus-4-7"]`) and **fails on the multi-row form**. The CLI startup gate would error out with a TOML parse error before serving any traffic.

This is the right behavior — **fail loudly, do not serve corrupt data**. A user who genuinely needs to downgrade must also revert `pricing.toml` to its v0.1.12 single-row shape (which still carries the correct v0.1.11 rates and would serve correctly). The fail-loudly path is preferred over a silent fallback that might produce wrong cost numbers.

**Action item for the v0.1.13 release notes**: explicitly document that downgrading from v0.1.13 → v0.1.12 requires reverting `pricing.toml` to single-row form. Anyone running `brew unpin && brew switch tokenscale-cli @0.1.12` should see this in the v0.1.13 CHANGELOG.

**(c) Recovery move if Phase D ships a wrong launch date**

The replace-on-startup pattern in `sync_pricing` (Phase B) makes this a TOML-level fix, not a migration-level one. Recovery:

1. Edit `pricing.toml` — update the offending row's `valid_from` to the correct date.
2. Add a dated correction entry to [`docs/cost-methodology.md`](docs/cost-methodology.md) Corrections log explaining what was wrong and the source for the new date.
3. Re-capture `pricing-rate-card.snapshot.json` if needed.
4. Tag a patch release (`v0.1.13.1` or `v0.1.14`).
5. Users `brew upgrade` and on next `tokenscale serve` startup, `sync_pricing` deletes the `pricing` table contents and re-inserts from the corrected TOML. All historical events are re-resolved against the corrected `valid_from` boundaries on the next dashboard load.

No schema migration is involved — the table structure is correct; only the data was wrong. The replace-on-startup design absorbs data corrections without ceremony, same as the env side has done for every research sweep since v0.1.3.

**(d) What the env side does NOT cover that this case might**

The env-side precedent is solid for schema + data evolution, but it has never had to handle a "we shipped a wrong VALUE for an extended period" recovery — every env-side correction so far has been a refinement (new uncertainty band, new indirect-water field), not a "this number was wrong." The closest parallel is the v0.1.11 pricing correction itself, which used the replace-on-startup pattern successfully. **The pattern is proven; the only addition v0.1.13 needs is the CHANGELOG-level downgrade documentation in (b).**

#### Recovery time objectives

Worst-case timings, end-to-end:

| Failure mode | Detection | Fix authoring | User upgrade | Total |
|---|---|---|---|---|
| Wrong launch date in shipped Phase D | Manual review of dashboard / drift detector | Edit `pricing.toml` + audit log + patch tag | brew upgrade cycle (~minutes per user) | < 24h once detected |
| Schema migration crashes on a real DB | First user reports startup failure | Forward-fix migration + patch tag | Same | < 48h |
| Per-event SQL join produces wrong cost | Unit tests should catch in CI; if it lands, manual diff vs. v0.1.12 numbers | Forward-fix in `aggregate_impact_by_bucket` | Same | < 48h |

#### Pre-release safety checklist

The release-tag CI for v0.1.13 must additionally pass:

1. Phase B migration applies cleanly against a fresh SQLite + against a DB that already has v0.1.12's schema. (Existing test infrastructure covers fresh; add a one-off `tests/migrations.rs`-style scenario for the upgrade-from-previous case.)
2. v0.1.12 `pricing.toml` (single-row form) still parses under v0.1.13's loader. The multi-row support is additive, not breaking.
3. The section 4 pre-launch-event QA check (hard gate — see "Hard gate" below).
4. A regression test: `aggregate_impact_by_bucket` with the v0.1.12 single-row `pricing.toml` produces identical results to the time-anchored implementation reading the same data as a single-row file. **Numbers must not move.**

---

## Sequencing against the rest of the queue

- This workstream (v0.1.13) is the **next** item after v0.1.12.
- It's **larger** than recent releases — likely 3–5 days of implementation, all-Rust + frontend.
- After v0.1.13:
  - **PUE uncertainty band** (small, factor-refinement)
  - **Winget manifest** (small, distribution)
  - **Granular-attribution roadmap Phase 0** (research only)

## Output expected from sign-off

When this scoping doc is approved (with any pushback on D1–D7 folded in), the implementation pass produces:

1. Phases A → B → C → D landed in a single v0.1.13 tag (the structural change is atomic).
2. Phase E as a fast-follow or bundled, maintainer's call.
3. CHANGELOG entry that explicitly says: "Historical Opus / Sonnet / Haiku numbers are unchanged from v0.1.12. Time-anchoring does not retroactively shift any cost figure."
4. **CHANGELOG entry must call out the `pricingByModel` removal as a public API change**, not bury it inside cost-computation work. The `/api/v1/usage/daily` response shape changes — anyone scripting against the field needs the heads-up. v0.1.12's introduction of `/api/v1/factors/active` confirmed the API has external surface area; breaking changes to that surface get billed explicitly.
5. Tests covering: multi-row lookup, `valid_from` boundary behavior, pre-launch events returning None, regression against v0.1.12 single-row results for current data.
6. The D7 downgrade-safety note in the CHANGELOG: "Downgrading to v0.1.12 requires reverting `pricing.toml` to single-row form. The new multi-row TOML format fails loudly on older binaries by design — no silent fallback that could serve wrong numbers."

### Hard gate before tagging v0.1.13

Section 4's pre-launch-event QA check is a **release-blocking gate**, not advisory. The CHANGELOG promise that "time-anchoring does not retroactively shift any cost figure" is only honest if the count of events that predate their model's launch in the user's real DB is zero or near-zero. If that count comes back non-trivial at implementation time:

- The tag DOES NOT ship.
- The maintainer investigates whether (a) a backfilled launch date is wrong (over-late, the failure mode D3's conservative-dating rule is designed to prevent) or (b) ingest is mis-attributing event dates.
- Resolve before any v0.1.13 release artifact reaches users.

The implementation must produce and report this count as a release artifact (a one-line CI step or a manual `tokenscale audit pricing-launch-dates`-style command — maintainer's call on shape). The count + a per-model breakdown lands in the v0.1.13 release notes either way (zero → "verified zero pre-launch events"; non-zero → blocker).
