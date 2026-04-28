# tokenscale — Architecture

> **Status:** scaffold. This document is fleshed out as each crate moves from skeleton to implementation. See [`decisions.md`](decisions.md) for the rationale behind individual choices.

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
                            │  domain types, math, factors│
                            └─────────────────────────────┘
```

## Crate responsibilities

| Crate                     | Responsibility                                                                                                                                                       |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tokenscale-core`         | Domain types (`Event`, `Factors`, `Pricing`), factor-model loader, pure-function cost and impact math. No I/O, no SQL, no HTTP.                                      |
| `tokenscale-store`        | SQLite schema, migrations, all query functions. Other crates do not write SQL — they call typed functions here.                                                      |
| `tokenscale-ingest-cc`    | Reads `~/.claude/projects/<project-id>/<session-id>.jsonl`. Idempotent re-scans via mtime tracking and `(source, request_id) ∪ (source, content_hash)` dedupe.       |
| `tokenscale-ingest-api`   | Anthropic Admin API client. Phase 2.                                                                                                                                 |
| `tokenscale-server`       | axum HTTP server. Exposes the REST API consumed by the dashboard and serves the embedded SPA via `rust-embed`.                                                       |
| `tokenscale-cli`          | Binary crate. Parses arguments, loads config, hands off to the appropriate crate. The only crate that uses `anyhow`; library crates use `thiserror`.                 |

## Data flow

1. **Ingest.** A scheduled or manual `tokenscale scan` invokes one or more ingest crates. Each ingester emits `Event`s and writes them to the database via `tokenscale-store`. Idempotency is enforced at the database layer through unique constraints.
2. **Query.** The HTTP server reads events from `tokenscale-store` and returns aggregations. Cost and impact computations apply the factor model loaded at startup.
3. **Render.** The frontend (embedded React SPA) renders charts and tables from the JSON the server returns.

## Factor-model loading

`tokenscale-core` reads `environmental-factors.toml` at startup:

1. Parse and validate `schema_version` against the supported range. **Refuse to start** if incompatible — this is the load-bearing guard for Cowork-side breaking shape changes.
2. If `file_status != "production"`, log a prominent warning and surface the status in the `/api/v1/health` response so the dashboard can show a banner.
3. Sync the file's contents into `env_factors` and `grid_factors`, versioned by `valid_from` so historical events resolve against the factors that were authoritative at their time.

## v2 readiness

The architecture is designed so adding a non-Anthropic provider in v2 is purely additive:

- `events.source REFERENCES sources(kind)` — adding a new ingest source is a row in `sources`, not a schema migration.
- `Factors` is keyed by `(provider, model)` from day one — never just `model`.
- The dashboard's filter UI already includes a provider control even though v1 has only `anthropic`.
- New ingest paths are new crates, not edits to a shared file.

## Region attribution

Anthropic does not disclose which AWS region served any given request. `tokenscale` resolves region by **configuration**, not observation: the user sets `default_inference_region` in the config, and that drives `grid_factors` lookup. The dashboard surfaces the configured region prominently as a user-controlled assumption — not a black-box estimate.

A future `region_blend` config (Phase 3) will allow weighted blends across multiple regions for users who want to model cross-region routing.

## Storage profile

SQLite, single file. Default path is platform-specific (`~/.local/share/tokenscale/tokenscale.db` on Linux, `~/Library/Application Support/tokenscale/tokenscale.db` on macOS).

The database is treated as user data. Backups, encryption-at-rest, and retention policy are the user's responsibility — `tokenscale` does not automate any of these. _(See [Privacy](../README.md#privacy) in the README.)_

## HTTP surface (Phase 1)

| Endpoint                                            | Returns                                                                       |
| --------------------------------------------------- | ----------------------------------------------------------------------------- |
| `GET /api/v1/usage/daily?from=&to=&provider=`       | Per-day token totals split by token type, optionally filtered by provider.    |
| `GET /api/v1/usage/by-model?from=&to=&provider=`    | Per-model token totals over the same window.                                  |
| `GET /api/v1/sessions/recent?limit=`                | Most-recent ingested sessions for the recent-activity panel.                  |
| `GET /api/v1/health`                                | Server status, DB status, factor-file `schema_version` and `file_status`.     |

The `provider=` parameter accepts `all` (default) or a specific provider slug (`anthropic`).
