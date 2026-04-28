# tokenscale

A self-hostable local dashboard that tells you what your Anthropic usage actually costs you — across three dimensions:

1. **Real spend** — what you actually paid Anthropic (Claude Max / Team / Enterprise subscription fees plus metered API charges).
2. **Counterfactual API spend** — what the same volume of tokens would have cost at standard API list pricing. The "is the subscription paying for itself?" view.
3. **Environmental impact** — energy (Wh), water (mL), and carbon (gCO₂e), derived from a versioned, source-attributed factor model.

`tokenscale` ingests usage from two sources:

- **Local Claude Code logs.** The session JSONL files Claude Code writes to disk at `~/.claude/projects/<project-id>/<session-id>.jsonl`.
- **Anthropic Admin API.** Read-only access to the same billing and usage data Anthropic exposes to the account owner. _(Phase 2.)_

It runs as a single static binary — Rust backend, embedded React dashboard. No external services, no cloud component, no data leaves your machine unless you explicitly export.

> **Status:** v0.1 scaffold. Phase 1 — JSONL ingest and the first chart. The Admin API path, environmental-impact computation, and counterfactual cost are Phase 2; passkey auth, in-browser research-runs review, and the upstream factor-sync are Phase 3.

---

## Scope and limitations

`tokenscale` is a reporting tool. Things it does **not** do:

- Bill, charge, or invoice. No payments move through it.
- Throttle or rate-limit. It does not sit in the request path of any Claude product.
- Modify Anthropic account state. All Admin API calls are read-only.
- Track non-Anthropic models in v1. v2 will extend to additional providers (OpenAI, Bedrock-hosted non-Claude models, etc.); the v1 architecture leaves explicit hooks for this so adding a provider is a row insertion plus a new ingest crate, not a schema migration.
- Constitute an audited environmental disclosure. The energy / water / carbon figures are best-effort estimates derived from public sources. They are useful for personal accounting and intuition; they are not a substitute for a vendor-issued, third-party-audited sustainability report.

For the full charter, see [`CHARTER.md`](CHARTER.md).

## Privacy

Read this before running `tokenscale scan` against your `~/.claude/` directory.

By default, `tokenscale` stores the **full JSON payload** of each Claude Code turn in the `events.raw` SQLite column. Those JSON payloads contain whatever you typed into Claude Code — code, configuration, comments, and anything you pasted in. They can include secrets (API keys, credentials, tokens) if you ever pasted any into a Claude Code session, and they can include personal information.

Three things to know:

1. **The data does not leave your machine.** `tokenscale` runs locally and stores its database on local disk. Nothing is sent to any remote service unless you export the data yourself.
2. **You can disable raw storage.** Set `ingest.store_raw = false` in `~/.config/tokenscale/config.toml`. With that flag off, the ingester persists token counts and metadata only and discards the raw JSON. Counts, costs, and environmental impact still work; debugging and reprocessing get harder.
3. **The default database file should be treated as sensitive.** Default location is platform-specific (`~/.local/share/tokenscale/tokenscale.db` on Linux, `~/Library/Application Support/tokenscale/tokenscale.db` on macOS). Back it up the way you back up other dotfiles that contain credentials — or do not back it up at all.

If you are running `tokenscale` on a shared or institutionally-managed machine, audit the data directory's permissions before scanning.

---

## Installation

### From source (current path)

```bash
git clone https://github.com/RobarePruyn/tokenscale
cd tokenscale
cargo build --release
./target/release/tokenscale init
./target/release/tokenscale scan
./target/release/tokenscale serve
```

The frontend is bundled into the binary at compile time, so end users do **not** need Node.js installed at runtime. Node is a build-time dependency.

### Pre-built binaries

Not yet published — Phase 3.

## Configuration

`tokenscale` reads its configuration from `~/.config/tokenscale/config.toml` (or the path in `$TOKENSCALE_CONFIG`, or the path passed to `--config`). The first run of `tokenscale init` writes a starter file with annotated defaults.

Notable fields:

- `default_inference_region` — AWS region whose grid factors are applied to your usage. Anthropic does not disclose which region served any given request; this is your declared, user-controlled assumption. Common values: `us-east-1`, `us-east-2`, `us-west-2`. _(See `environmental-factors.toml` for the supported regions.)_
- `ingest.store_raw` — see Privacy above. Default `true`.
- `auth.mode` — `localhost` (default; bind to 127.0.0.1, no auth) or `network` (bind to 0.0.0.0, passkey required). _(`network` mode is Phase 3.)_

## Building the frontend

The frontend lives in `frontend/` and is a Vite + React + TypeScript SPA. The Rust binary embeds the production build of the frontend at compile time via `rust-embed`. Workflow:

```bash
# One-time:
cd frontend && npm install && cd ..

# Each time you change frontend code:
cd frontend && npm run build && cd ..
cargo build --release
```

For frontend-only iteration, `cd frontend && npm run dev` runs Vite's dev server (typically on `http://localhost:5173`) which proxies API calls to the Rust server you can run separately with `cargo run -p tokenscale-cli -- serve`.

## Workspace layout

```
tokenscale/
├── Cargo.toml                    # workspace manifest
├── crates/
│   ├── tokenscale-cli/           # binary entrypoint (clap)
│   ├── tokenscale-core/          # domain types, factor math, cost math
│   ├── tokenscale-ingest-cc/     # Claude Code JSONL ingester
│   ├── tokenscale-ingest-api/    # Anthropic Admin API ingester (Phase 2)
│   ├── tokenscale-store/         # sqlx schema, migrations, queries
│   └── tokenscale-server/        # axum HTTP server
├── frontend/                     # Vite + React SPA
├── migrations/                   # sqlx migrations (SQL files)
├── docs/
│   ├── architecture.md
│   ├── data-sources.md
│   ├── decisions.md              # decision log with rationales
│   └── sources.md                # factor-model bibliography
├── environmental-factors.toml    # versioned factor model
└── request-for-research.md       # engineering → research request channel
```

## The factor model

The numerical factors used to compute environmental impact live in [`environmental-factors.toml`](environmental-factors.toml). Every numeric value traces to a specific source URL with an access date, recorded in [`docs/sources.md`](docs/sources.md).

The factor file is maintained out-of-band by the [Cowork research project](https://cowork.com) (a separate AI-research workflow run by the maintainer) and merged into this repository on a roughly bi-weekly cycle. v1 ships with placeholder values; the production v0.1 values land via the maintainer's review-and-merge flow.

Phase 3 will add:

- `tokenscale factors update` — pull the latest factor file from the upstream public Git repo.
- An optional on-startup auto-pull, off by default.
- A maintainer-only `tokenscale factors publish` for pushing locally-edited factors upstream (deploy-key-gated; not exposed in the public binary's default behavior).

## Contributing

The contributor workflow has one quirk: sqlx's compile-time query verification needs either a live database or the committed `.sqlx/` query cache. After editing any `sqlx::query!` or `sqlx::query_as!` invocation, run:

```bash
cargo sqlx prepare --workspace
```

and commit the resulting `.sqlx/` changes. CI runs with `SQLX_OFFLINE=true`, so without this step CI will fail.

Other conventions:

- Conventional Commits format (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`).
- Atomic commits — each one should leave the tree in a working state.
- `rustfmt` and `clippy` clean. `clippy::pedantic` is on workspace-wide with the noisiest lints muted in `Cargo.toml`.
- Variable names read like English. `claude_code_sessions_directory` over `cc_dir`, `incoming_admin_api_response` over `resp`. The few exceptions are CLI flag names and well-established short forms (`io`, `db`, `ctx`).
- Doc comments explain *why*, not *what*. Public APIs always carry one.

For decisions of consequence (license, framework choice, schema-shape changes), append an entry to [`docs/decisions.md`](docs/decisions.md) so future contributors can recover the rationale.

## License

Apache License, Version 2.0 — see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).

The license choice and its rationale are documented in [`docs/decisions.md`](docs/decisions.md).
