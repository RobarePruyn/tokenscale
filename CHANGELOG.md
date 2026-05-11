# Changelog

Notable changes per release. Format loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

Newest releases on top. Unreleased changes accumulate under `## Unreleased`.

---

## v0.1.1 — 2026-05-11

Polish release on top of v0.1.0. No new features; quality-of-life fixes around the dashboard's wider-window views and the install/setup path now that pre-built binaries are the recommended way in.

### Added

- **`docs/research-cadence.md`** — documents how the environmental-factor model gets refreshed (quarterly default + ad-hoc triggers, what a sweep produces, how it distributes to users, the maintainer review checklist). Names the discipline that keeps factor data credible over time.
- **`docs/request-for-research.md`** — open questions for the next research sweep: grid-factor uncertainty bands, indirect-water methodology, tokenizer-change inflation factor verification, non-US eGRID coverage, methodology-choice re-verification.
- **`CHANGELOG.md`** (this file).

### Changed

- **README leads with `brew install`** and the other pre-built installers; build-from-source flow moved to a "Building from source" section for contributors. Reflects that the project actually ships as a binary now.
- **Auto-granularity thresholds retuned** from 60/365 to 120/730 days. The 90d view now stays on daily buckets (previously crossed into weekly), making 30d → 90d a visually continuous extension rather than a sudden 7× peak jump from bucket-size change.
- **Chart title reflects actual granularity** — "Weekly token usage" / "Monthly cost (USD)" / etc. — instead of always saying "Daily" regardless of bucket size.
- **Subscriptions panel** — divider line above the "Imported subscription charges" section is suppressed when there are no manual entries to separate from. Small visual cleanup.

### Fixed

- **`~/` paths in `claude_code_roots` now expand to the user's home directory.** Configs following the README example (`["~/.claude/projects", "~/.claude-synced/laptop/projects"]`) previously failed silently — TOML stored the paths verbatim, the walker tried to open a directory literally named `~`, and the scan skipped both roots without surfacing an error in the UI. The fix is a small `expand_tilde` helper applied to every configured root.

### Release pipeline

- First release fully validated through the pipeline. `dist` v0.31.0 with the GitHub-build-setup hook that runs `npm ci && npm run build` in the `frontend/` directory before `cargo build`. Homebrew tap auto-published to `RobarePruyn/homebrew-tokenscale`. Two CI bugs surfaced and fixed during v0.1.0 (npm working-directory key was stripped by dist's inlining, and the tap repo needed an initial commit before the publish job could checkout main); both fixed inline.

---

## v0.1.0 — 2026-05-11

Initial public release. The bones of the project are in place: ingest Claude Code session logs, compute environmental impact against versioned factor data, surface everything in a local-first dashboard, and ship as a one-command install on five platforms.

### Added

- **Phase 1: dashboard core**.
  - Claude Code JSONL ingest crate (`tokenscale-ingest-cc`) reading `~/.claude/projects/<project>/<session>.jsonl` with idempotent delta scans keyed by `(mtime_ns, len)` per file and request-id / content-hash dedup at the events table.
  - SQLite schema with versioned migrations, `sources`-table-keyed multi-provider design.
  - axum HTTP server with embedded React + Tailwind + Recharts SPA via `rust-embed`. Endpoints: `/api/v1/health`, `/usage/daily`, `/usage/by-model`, `/projects`, `/sessions/recent`, `/subscriptions`, `/billing/charges`.
  - Dashboard with filters (provider, models, token types, projects, date range), view modes (Raw / Cost-weighted / Cost (USD)), stacked-by-model and stacked-by-token-type chart variants, three KPI stat cards.
  - Subscription tracking — manually-declared subscriptions with date-range pro-rating, "Net value" calculation against counterfactual API cost.
- **Phase 2: environmental impact compute path**.
  - `environmental-factors.toml` v0.1 production file with 17 (provider, model) tuples across Anthropic / Google / OpenAI / DeepSeek / Meta / Mistral plus 3 AWS region grid factors anchored to EPA eGRID2023.
  - Google "comprehensive methodology" (Elsworth et al. 2025, arXiv:2508.15734) as the canonical compute path: per-token energy × PUE → facility energy → CO₂e and water as facility-level multipliers.
  - Per-event, time-anchored factor resolution — each event resolves to the env_factors row whose `valid_from <= occurred_at`. Honest sums across `valid_from` boundaries inside a bucket.
  - `[inference]` config block with `default_inference_region` for grid attribution. Back-compat reads the legacy top-level field with a deprecation warning.
  - Environmental impact block on every daily-endpoint response — energy / facility energy / CO₂e / water plus provenance counters (events using fallback PUE, events missing env_factor). Stat row + environmental banner in the dashboard.
- **Phase 2: billing data ingest via Stripe CSV import**.
  - `billing_charges` table with line-item-level granularity, `(source, external_id)` dedup, idempotent re-imports.
  - Two-step preview/commit endpoints. Preview shows parsed rows with auto-categorization and detects conflicts against manually-declared subscriptions. Commit accepts re-categorized rows + a list of manual subscription IDs to dismiss in the same transaction (avoids double-counting when CSV subscriptions duplicate manual entries).
  - Dashboard import panel — drop a CSV or paste, preview with inline category overrides + conflict resolution, commit.
- **Multi-machine ingest** — `[ingest].claude_code_roots` accepts a list of paths so multiple synced JSONL directories scan in one pass. README documents Syncthing as the recommended cross-platform sync tool.
- **Auto-scan in `serve`** — background tokio task re-runs scan every `scan_interval_seconds` (default 60), so the dashboard refreshes as Claude Code activity lands without manual `tokenscale scan`.
- **Distribution pipeline** — `dist` v0.31.0 generates prebuilt binaries for macOS (Apple Silicon + Intel), Linux (x86_64 + aarch64), and Windows (x86_64) on every tagged release. Three installer types: `curl | sh`, PowerShell, and a Homebrew formula auto-published to a separate tap repo. Full release process documented in `RELEASING.md`.

### Documentation

- `CHARTER.md` — project scope, governance, distribution model, framework-extractable design lens.
- `docs/architecture.md` — high-level system design.
- `docs/decisions.md` — running ADR log; 12 entries covering language choices, schema decisions, ingest model, sqlx offline mode, factor model design, time-anchoring SQL, per-event resolution, `[inference]` config block, `view=` parameter drop, multi-machine sync via Syncthing, manual CSV import as default billing path, and the renamed `cargo-dist` → `dist`.
- `docs/sources.md` — bibliography of every factor source with confidence tags and access dates.
- `docs/research-log.md` — audit trail of the v0.1 factor sweep.
- `docs/data-sources.md` — per-ingest-source documentation; honest acknowledgment of which surfaces are structurally invisible to any external tool (iOS, Android, desktop apps, `claude.ai` web).
- `README.md` with quick-start, install commands, dashboard tour, configuration reference, and multi-machine setup walkthrough.

### Known limitations

- Anthropic Admin API ingest exists in design but isn't built; requires an organization account, which most individual-tier users don't have.
- Consumer chat surfaces (Claude iOS / Android / desktop apps, `claude.ai` web) have no per-user usage feed and are structurally invisible to the dashboard's token-tracking view. Their subscription cost IS captured via billing imports.
- Linux distro packaging (APT / DNF / RPM), Scoop bucket, Winget manifest, and macOS notarization are not yet automated. Prebuilt `.tar.xz` archives serve those platforms in the interim.
