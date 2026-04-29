# tokenscale — Architecture

> Captures the shape of the system as of Phase 1 close-out. See [`decisions.md`](decisions.md) for the rationale behind individual choices.

## Component map

```
                       ┌────────────────────────────────────────┐
                       │             tokenscale-cli             │
                       │   clap entrypoint, subcommand router   │
                       └───────┬───────────────────────┬────────┘
                               │                       │
                ┌──────────────▼──────┐   ┌────────────▼─────────┐
                │  tokenscale-server  │   │  tokenscale-ingest-* │
                │   axum + REST API   │   │   per-source ingest  │
                │   embedded SPA      │   │   (cc, api, …)       │
                └──────────────┬──────┘   └────────────┬─────────┘
                               │                       │
                               └──────────┬────────────┘
                                          │
                            ┌─────────────▼──────────────┐
                            │      tokenscale-store       │
                            │  sqlx + SQLite, migrations  │
                            └─────────────┬──────────────┘
                                          │
                            ┌─────────────▼──────────────┐
                            │      tokenscale-core        │
                            │ events / pricing / factors  │
                            │ billable + cost math        │
                            └─────────────────────────────┘
```

## Crate responsibilities

| Crate                     | Responsibility                                                                                                                                                       |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tokenscale-core`         | Domain types (`Event`, `PricingFile`, `EnvironmentalFactorsFile`), TOML loaders + schema-version guards, pure-function billable-multiplier math. No I/O, no SQL, no HTTP. |
| `tokenscale-store`        | SQLite schema, migrations, all query functions. Other crates do not write SQL — they call typed functions here. Includes the startup factors → DB sync.              |
| `tokenscale-ingest-cc`    | Reads `~/.claude/projects/<project-id>/<session-id>.jsonl`. Idempotent re-scans via mtime tracking and `(source, request_id) ∪ (source, content_hash)` dedupe.       |
| `tokenscale-ingest-api`   | Anthropic Admin API client. Phase 2.                                                                                                                                 |
| `tokenscale-server`       | axum HTTP server. Exposes the REST API consumed by the dashboard and serves the embedded SPA via `rust-embed`. Carries the in-memory pricing + factors snapshots.   |
| `tokenscale-cli`          | Binary crate. Parses arguments, loads config, hands off. The only crate that uses `anyhow`; library crates use `thiserror`.                                          |

## Data flow

1. **Ingest.** `tokenscale scan` invokes one or more ingest crates. Each ingester emits `Event`s and writes them via `tokenscale-store`. Idempotency is enforced at the database layer through partial unique indexes on `events`.
2. **Startup.** `tokenscale serve` opens the database, loads `pricing.toml` + `environmental-factors.toml`, syncs the factor file into `env_factors` / `grid_factors`, and mounts the axum router.
3. **Query.** Dashboard requests hit `/api/v1/*` handlers. Token aggregations come from SQL; per-token-type billable equivalents are computed from the in-memory pricing snapshot. Phase 2's per-event impact computation will join the in-memory factors snapshot.
4. **Render.** The embedded React SPA renders charts and panels from the JSON the server returns.

## Pricing-model loading (Phase 1, working)

`tokenscale-core::pricing` reads `pricing.toml` at startup:

1. Parse and validate `schema_version` against the supported range. **Refuse to start** if incompatible.
2. Resolve precedence: `[pricing] file = "..."` from the config file → embedded copy from `include_str!`.
3. Hand the parsed `PricingFile` to the server, which holds it as `Arc<PricingFile>` in `AppState`.
4. The dashboard's Cost (USD) view computes per-token-type and per-(date, model) cost from `billable × input_price ÷ 1_000_000`.

The `file_status` field flags whether the values have been verified against current Anthropic pricing. The dashboard surfaces a banner on cost views with the file's `source_accessed_at` so users know the values are accurate as of a specific date.

## Factor-model loading (Phase 1 plumbing, Phase 2 compute)

`tokenscale-core::factors` mirrors the pricing pattern:

1. Parse and validate `schema_version`. Refuse incompatible files.
2. Pre-process `key = null` lines into comments before serde (TOML 1.0 doesn't have `null`). Maintainer convention preserved; missing keys deserialize to `Option::None`.
3. Resolve from `[factors] file = "..."` or embedded copy.
4. **Sync into the DB** via `tokenscale-store::sync_environmental_factors` — full replacement on every startup in Phase 1, history-preserving upsert in Phase 2.
5. Hand the snapshot to the server as `Arc<EnvironmentalFactorsFile>`.

The placeholder file ships with every numeric value `null`. The dashboard's `/api/v1/health` exposes the file status (`is_placeholder`, `needs_review`, `accessed_at`) so the Phase 2 view can render "factor data unavailable" until Cowork's deliverable 3 lands real values.

Phase 2 will add the per-event compute following Google's August 2025 "comprehensive" methodology — energy includes idle, host CPU/RAM, and PUE-weighted facility overhead, not just active GPU.

## v2 readiness

The architecture is designed so adding a non-Anthropic provider in v2 is purely additive:

- `events.source REFERENCES sources(kind)` — adding a new ingest source is a row in `sources`, not a schema migration.
- `pricing.toml` and `environmental-factors.toml` are keyed by `(provider, model)` from day one — never just `model`.
- The dashboard's Provider filter is wired from day one even though v1 has only `anthropic`.
- New ingest paths are new crates, not edits to a shared file.

## Region attribution

Anthropic does not disclose which AWS region served any given request. `tokenscale` resolves region by **configuration**, not observation: the user sets `default_inference_region` in the config, and Phase 2 will drive `grid_factors` lookup off it. The dashboard will surface the configured region prominently as a user-controlled assumption — not a black-box estimate.

A future `region_blend` config (Phase 3) will allow weighted blends across multiple regions for users who want to model cross-region routing.

## Storage profile

SQLite, single file. Default path is platform-specific (`~/.local/share/tokenscale/tokenscale.db` on Linux, `~/Library/Application Support/tokenscale/tokenscale.db` on macOS).

The database is treated as user data. Backups, encryption-at-rest, and retention policy are the user's responsibility — `tokenscale` does not automate any of these. _(See [Privacy](../README.md#privacy) in the README.)_

## HTTP surface (Phase 1)

| Endpoint                                                                   | Returns                                                                                                          |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `GET /api/v1/health`                                                       | Status, version, total events, providers, pricing block, environmental block.                                    |
| `GET /api/v1/usage/daily?from=&to=&provider=&project=&granularity=`        | Per-bucket per-(model, token-type) breakdown. Cost-weighted and per-model `input_usd_per_mtok` included.         |
| `GET /api/v1/usage/by-model?from=&to=&provider=`                           | Per-model token totals over the window.                                                                          |
| `GET /api/v1/sessions/recent?limit=`                                       | Most-recent ingested sessions for the recent-activity panel.                                                     |
| `GET /api/v1/projects?from=&to=&provider=`                                 | Distinct `project_id` values present in the window, with per-project rollups.                                    |
| `GET /api/v1/subscriptions`                                                | List declared subscriptions.                                                                                     |
| `POST /api/v1/subscriptions`                                               | Create a subscription. JSON body: `{ plan_name, monthly_usd, started_at, ended_at? }`.                           |
| `PUT /api/v1/subscriptions/{id}`                                           | Replace an existing subscription.                                                                                |
| `DELETE /api/v1/subscriptions/{id}`                                        | Delete a subscription.                                                                                           |

The `provider=` parameter accepts `all` (default) or a specific provider slug (`anthropic`). The `project=` parameter accepts `all`, `__none__` (explicit empty filter — used by the dashboard's "Select none" chip action), or a comma-separated list of project paths. The `granularity=` parameter is `day` (default), `week`, or `month`.
