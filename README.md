# tokenscale

A self-hostable local dashboard that tells you what your Anthropic usage actually costs you — across three dimensions:

1. **Real spend** — what you actually paid Anthropic (Claude Max / Pro / Team / Enterprise subscription fees plus metered API charges).
2. **Counterfactual API spend** — what the same volume of tokens would have cost at standard Anthropic API list pricing. The "is the subscription paying for itself?" view.
3. **Environmental impact** — energy (Wh), water (mL), and carbon (gCO₂e), derived from a versioned, source-attributed factor model.

`tokenscale` ingests usage from two sources:

- **Local Claude Code logs.** The session JSONL files Claude Code writes to disk at `~/.claude/projects/<project-id>/<session-id>.jsonl`.
- **Anthropic Admin API.** Read-only access to the same billing and usage data Anthropic exposes to the account owner. _(Phase 2.)_

It runs as a single static binary — Rust backend, embedded React + Tailwind + Recharts dashboard. No external services, no cloud component, no data leaves your machine unless you explicitly export.

> **Status — Phase 1 complete.** Working today: Claude Code JSONL ingest, the four `/api/v1/usage/*` and `/api/v1/health` endpoints, the embedded SPA with model / token-type / project filters, granularity (day/week/month) and chart-type (area/bar/line) controls, log-scale y-axis, the Cost (USD) view backed by `pricing.toml`, and a Subscriptions panel that surfaces "counterfactual API cost − subscriptions paid in window = net value." Environmental-impact computation lands in **Phase 2** once the [Cowork research project](https://cowork.com) merges its v0.1 factor values into `environmental-factors.toml` (the loader, schema-version guard, and DB sync are already wired). Admin API ingest, passkey auth, and the upstream factor-pull all sit in **Phase 3**.

---

## Scope and limitations

`tokenscale` is a reporting tool. Things it does **not** do:

- Bill, charge, or invoice. No payments move through it.
- Throttle or rate-limit. It does not sit in the request path of any Claude product.
- Modify Anthropic account state. All Admin API calls (Phase 2) are read-only.
- Track non-Anthropic models in v1. v2 will extend to additional providers (OpenAI, Bedrock-hosted non-Claude models, etc.); the v1 architecture leaves explicit hooks for this so adding a provider is a row insertion plus a new ingest crate, not a schema migration.
- Constitute an audited environmental disclosure. The energy / water / carbon figures are best-effort estimates derived from public sources. They are useful for personal accounting and intuition; they are not a substitute for a vendor-issued, third-party-audited sustainability report.
- Tell you what specific dollar bill Anthropic is going to send you next. The Cost (USD) view is **counterfactual** — what your token volume would have cost on the API at the list rates in `pricing.toml`. Your actual subscription cost is whatever you put into the Subscriptions panel.

For the full charter, see [`CHARTER.md`](CHARTER.md).

## Privacy

Read this before running `tokenscale scan` against your `~/.claude/` directory.

By default, `tokenscale` stores the **full JSON payload** of each Claude Code turn in the `events.raw` SQLite column. Those JSON payloads contain whatever you typed into Claude Code — code, configuration, comments, and anything you pasted in. They can include secrets (API keys, credentials, tokens) if you ever pasted any into a Claude Code session, and they can include personal information.

Three things to know:

1. **The data does not leave your machine.** `tokenscale` runs locally and stores its database on local disk. Nothing is sent to any remote service unless you export the data yourself.
2. **You can disable raw storage.** Set `ingest.store_raw = false` in `~/.config/tokenscale/config.toml`. With that flag off, the ingester persists token counts and metadata only and discards the raw JSON. Counts, costs, and (Phase 2) environmental impact still work; debugging and reprocessing get harder.
3. **The default database file should be treated as sensitive.** Default location is platform-specific (`~/.local/share/tokenscale/tokenscale.db` on Linux, `~/Library/Application Support/tokenscale/tokenscale.db` on macOS). Back it up the way you back up other dotfiles that contain credentials — or do not back it up at all.

If you are running `tokenscale` on a shared or institutionally-managed machine, audit the data directory's permissions before scanning.

---

## Quick start

```bash
git clone https://github.com/RobarePruyn/tokenscale
cd tokenscale

# One-time frontend deps + build (rust-embed needs frontend/dist/ at compile time)
cd frontend && npm install && npm run build && cd ..

# Build, init, ingest, serve
cargo build --release
./target/release/tokenscale init
./target/release/tokenscale scan
./target/release/tokenscale serve
```

Then open `http://127.0.0.1:8787`.

The frontend is bundled into the binary at compile time via `rust-embed`, so end users do **not** need Node.js installed at runtime. Node is a build-time dependency only.

### Pre-built binaries

Not yet published — Phase 3.

## Dashboard tour

What you'll see when the dashboard loads:

- **Provider** dropdown — `All providers` or `Anthropic`. The control is here from day one even though v1 only ingests from Anthropic; v2 lights it up by adding rows to `sources` and ingest crates per provider.
- **Range** — `7d / 30d / 90d / 1y / All / Custom`. `All` clamps the lower bound at 2022-12-01 (ChatGPT launch — no LLM usage data predates it).
- **Filters** (collapsible) — multi-select chips for models, token types, and projects (each with Select-all / Select-none). Project paths come from the `cwd` field in each Claude Code JSONL turn; expect ~one chip per directory you've worked in.
- **Stack by**: `Model` (one series per model, stacked) or `Token type` (one series per token type — input/output/cache_read/cache_write_5m/cache_write_1h, stacked).
- **Counting**:
  - `Raw` — every token weighted equally.
  - `Cost-weighted` — each token type weighted by its API price relative to input (output ×5, cache_read ×0.1, cache writes ×1.25/×2).
  - `Cost (USD)` — cost-weighted converted to dollars per the model's input price. The $-axis cost figure.
- **Granularity** — `Auto / Day / Week / Month`. Auto picks the bucket size so the chart never has more than ~60 buckets.
- **Chart** — `Area / Bar / Line`. Line is unstacked (compare-trends view).
- **Scale** — `Linear / Log`. Log is the one to flip when cache_read mass is dwarfing the rest.

Above the chart, three KPIs:

- **Counterfactual API cost** for the current filtered window.
- **Subscriptions paid in window** — pro-rated from your declared subscriptions.
- **Net value** — counterfactual − subscriptions. Tinted emerald when positive (the subscription is paying for itself).

Below the chart, the **Subscriptions** panel. Add Claude Pro / Max 5× / Max 20× / Team / Enterprise / Custom from a dropdown that pre-fills the form. Edit existing subscriptions in place. Delete with a confirm.

## Configuration

`tokenscale` reads its configuration from `~/.config/tokenscale/config.toml` (or the path in `$TOKENSCALE_CONFIG`, or the path passed to `--config`). The first run of `tokenscale init` writes a starter file with annotated defaults.

Notable fields:

| Field | Purpose | Default |
|---|---|---|
| `default_inference_region` | AWS region whose grid factors apply. Anthropic doesn't disclose which region served any given request; this is your declared assumption. | (unset — Phase 2 will fall back to `us-east-1`) |
| `ingest.store_raw` | Persist the full JSONL payload. See Privacy above. | `true` |
| `ingest.claude_code_root` | Override `~/.claude/projects` | (auto-detected) |
| `storage.database_path` | Override the SQLite file location | (auto-detected) |
| `server.bind` | Address the server binds to | `127.0.0.1:8787` |
| `auth.mode` | `localhost` (no auth) or `network` (passkey required, Phase 3) | `localhost` |
| `pricing.file` | Override path to `pricing.toml`. Unset = embedded default. | (unset) |
| `factors.file` | Override path to `environmental-factors.toml`. The "local research mode" the [CHARTER](CHARTER.md) describes. | (unset) |

## Building the frontend

The frontend lives in `frontend/` and is a Vite + React + TypeScript SPA styled with Tailwind v4, charting with Recharts. The Rust binary embeds `frontend/dist/` at compile time via `rust-embed`. Workflow:

```bash
# One-time:
cd frontend && npm install && cd ..

# Each time you change frontend code:
cd frontend && npm run build && cd ..
cargo build --release
```

For frontend-only iteration, `cd frontend && npm run dev` runs Vite's dev server (typically on `http://localhost:5173`), which proxies `/api/*` calls to the Rust server you can run separately with `cargo run -p tokenscale-cli -- serve`.

## Workspace layout

```
tokenscale/
├── Cargo.toml                    # workspace manifest
├── crates/
│   ├── tokenscale-cli/           # binary entrypoint (clap)
│   ├── tokenscale-core/          # domain types: events, pricing, factors, billable math
│   ├── tokenscale-ingest-cc/     # Claude Code JSONL ingester
│   ├── tokenscale-ingest-api/    # Anthropic Admin API ingester (Phase 2)
│   ├── tokenscale-store/         # sqlx schema, migrations, queries, factors sync
│   └── tokenscale-server/        # axum HTTP server, embedded SPA
├── frontend/                     # Vite + React + Tailwind + Recharts SPA
├── migrations/                   # sqlx migrations (SQL files)
├── docs/
│   ├── architecture.md
│   ├── data-sources.md
│   ├── decisions.md              # decision log with rationales
│   └── sources.md                # factor-model bibliography
├── environmental-factors.toml    # versioned factor model (placeholder; Cowork-maintained)
├── pricing.toml                  # versioned per-model API pricing (seed values; verify before relying)
└── request-for-research.md       # engineering → research request channel
```

## The pricing model

`pricing.toml` carries Anthropic's per-model API list pricing — input/output/cache_read $/MTok and cache-write multipliers — versioned by `valid_from` and source-attributed via `source_url` + `source_accessed_at`. The dashboard's Cost (USD) view and "Counterfactual API cost" KPI both read from this file.

The `file_status` field flags whether the values have been verified against Anthropic's current pricing. `tokenscale` ships with `file_status = "needs_review"`; the dashboard surfaces a banner on the cost views explaining that the values are the published list rates as of `source_accessed_at` and pointing at <https://platform.claude.com/docs/en/about-claude/pricing> for re-verification. After verifying, set `file_status = "production"` to silence the banner.

To use freshly-verified or custom pricing without rebuilding the binary, set `[pricing] file = "/path/to/your/pricing.toml"` in `~/.config/tokenscale/config.toml`.

## The environmental-factor model

The factor model lives in `environmental-factors.toml` and is **maintained out-of-band by the [Cowork research project](https://cowork.com)** — a separate AI-research workflow run by the maintainer. Every numeric value traces to a specific source URL with an access date, recorded in [`docs/sources.md`](docs/sources.md).

Phase 1 ships the loader + DB sync; the file's values are placeholders (`null`) until Cowork's deliverable 3 lands. Phase 2 wires the per-event impact computation (energy_wh / facility_wh / co2e_g / water_l) and the dashboard view that surfaces it.

When new factor values are merged, the dashboard picks them up on the next `tokenscale serve` startup — no code changes required.

Phase 3 will add:

- `tokenscale factors update` — pull the latest factor file from the upstream public Git repo.
- Optional on-startup auto-pull, off by default.
- A maintainer-only `tokenscale factors publish` for pushing locally-edited factors upstream (deploy-key-gated; not exposed in the public binary's default behavior).

## Contributing

Conventions:

- **Conventional Commits** format (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`).
- **Atomic commits** — each one should leave the tree in a working state.
- **`rustfmt` + `clippy` clean.** `clippy::pedantic` is on workspace-wide; the noisiest lints are muted in `Cargo.toml`.
- **Variable names read like English.** `claude_code_sessions_directory` over `cc_dir`, `incoming_admin_api_response` over `resp`. CLI flag names and well-established short forms (`io`, `db`, `ctx`) are exceptions.
- **Doc comments explain *why*, not *what*.** Public APIs always carry one.
- **For decisions of consequence** (license, framework choice, schema-shape changes), append an entry to [`docs/decisions.md`](docs/decisions.md) so future contributors can recover the rationale.

For environment-side changes that need new application capabilities — e.g., a new field in `environmental-factors.toml` — append an entry to [`request-for-research.md`](request-for-research.md). The maintainer copies entries from there into the Cowork project for the research agent to act on.

The Phase 1 store layer uses `sqlx::query` / `sqlx::query_as` (runtime-checked) rather than the `sqlx::query!` macro. No `.sqlx/` cache is needed and contributors don't need a live database to build. Migrating to compile-time-verified queries is a deferred follow-up — see [`docs/decisions.md`](docs/decisions.md).

## License

Apache License, Version 2.0 — see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).

The license choice and its rationale are documented in [`docs/decisions.md`](docs/decisions.md).
