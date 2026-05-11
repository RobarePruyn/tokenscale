# tokenscale — Decision Log

A running record of non-obvious decisions made during the build, with the rationale captured at the time. New entries go on top.

The goal here is simple: anyone (the maintainer, a future contributor, a future me) should be able to read this and understand *why* something is the way it is, without having to reconstruct the conversation that led to it.

---

## 2026-05-06 — Correlated-subquery time-anchoring instead of a SQL view

**Decision.** Time-anchored factor resolution in the dashboard query
joins each event to the authoritative `env_factors` / `grid_factors`
row using an inline correlated subquery:

```sql
LEFT JOIN env_factors ef
       ON ef.provider = sources.provider
      AND ef.model = events.model
      AND ef.valid_from = (
          SELECT MAX(valid_from) FROM env_factors
           WHERE provider = sources.provider
             AND model = events.model
             AND valid_from <= date(events.occurred_at)
      )
```

(and the symmetric form for `grid_factors`). The same pattern lives
inside the per-row helpers `lookup_environmental_factors` /
`lookup_grid_factors`.

**Why.** The kickoff prompt's first instinct was a SQL view —
something like `CREATE VIEW events_with_factors AS ...`. We rejected
that because:

- **Region is a runtime input, not schema state.** The grid-factor
  lookup needs the user's configured `[inference].default_inference_region`
  — a value that only exists at query time, not at schema-creation
  time. SQL views can't take parameters.
- **`MAX(valid_from)` with a `<=` constraint is the cleanest
  expression of "latest row authoritative at the event's date".** It
  composes naturally with the LEFT JOIN: when no row matches (event
  predates the file's earliest `valid_from`), the JOIN produces a
  null row, COALESCE turns missing factors into 0/None contributions,
  and the event's impact silently zeroes out — which is the correct
  honest behavior (a fact we learned the hard way when the v0.1 file
  shipped with `valid_from = "2026-04-28"` on every row and made
  most of the user's data invisible until we fixed it).
- **Per-event resolution stays a query-time concern.** No materialized
  view, no triggers, no denormalized factor columns on the events
  table. The events table stores the raw token counts; factor
  application is recomputed every read. This means a new factor file
  (v0.2 with refined values, a new model row) reshapes the dashboard
  instantly on next request — no rebuild step.

**How to apply.**

- New time-anchored lookups across the codebase should follow the
  same correlated-subquery pattern. It's terse, it composes with
  filters, and it generalizes (cost_factors, pricing_factors,
  whatever Phase 2+ adds).
- Resist materialized views or denormalized factor columns on events
  without a measured perf justification. SQLite on a single-user
  dataset handles correlated subqueries trivially for the data scales
  we're targeting (millions of events, not billions).
- The fallback-when-no-row-matches behavior is load-bearing. If a
  future revision adds error-on-no-match semantics, it MUST also
  add a corresponding fallback path so users don't silently lose
  data when factor files lag behind their event history.

---

## 2026-05-06 — Per-event factor resolution, not per-bucket

**Decision.** Environmental impact is computed by resolving each
event to its own time-anchored factor row, then aggregating to the
bucket. Not: resolve the factor row at the bucket level (e.g. "the
factor row in effect at the bucket's midpoint"), apply it to the
summed tokens, and call it done.

**Why.** At kickoff, the simpler per-bucket approach was on the
table. Three reasons we went per-event:

- **Correctness at boundary buckets.** When a `valid_from` boundary
  falls inside a bucket (the file shipped v0.2 in the middle of a
  week the user is viewing), per-bucket resolution forces an
  arbitrary choice — use the old factor or the new one for the
  whole bucket? Both are wrong. Per-event resolution honestly
  attributes events before the boundary to the old factor and
  events after to the new one. The bucket totals are then the
  honest sum, not an averaged-out approximation.
- **The kickoff's "build it right the first time" preference.**
  Per-event resolution is the right shape for the long run.
  Per-bucket would have been a v1 shortcut that earned compute
  debt every time a factor file evolved — and the user explicitly
  rejected that trade.
- **The SQL cost is bounded.** SQLite's query planner handles the
  correlated subquery + `LEFT JOIN` shape against indexed
  `(provider, model, valid_from)` and `(region, valid_from)`
  efficiently at the scales tokenscale targets. The "save compute
  by collapsing first" path doesn't materially help.

**How to apply.**

- New impact metrics (water, scope-2 carbon, indirect-water as v0.2
  research lands) should resolve at the event level, not at the
  bucket level, even if the prototype could "just for now" sum-then-
  multiply. The boundary-bucket correctness argument applies to any
  time-anchored factor.
- If a future feature finds per-event resolution genuinely
  expensive — say, a years-of-data tier-1 customer with billions of
  events — the right escape is materialized rollups (precomputed
  daily impact totals) keyed by `(bucket_date, model, factor_valid_from)`,
  not a switch to per-bucket resolution. The materialized form
  preserves per-event correctness while giving up the
  recompute-on-every-read flexibility.

---

## 2026-05-06 — `[inference]` config block (vs top-level `default_inference_region`)

**Decision.** Phase-2 inference settings live under an `[inference]`
config table:

```toml
[inference]
default_inference_region = "us-east-1"
```

Not at the top level. The legacy top-level form
(`default_inference_region = "..."` at the root) still parses for
back-compat, but emits a one-time deprecation warning on load.

**Why.**

- **Phase 2 added more inference-related knobs than fit at the top
  level.** Region was the first; water methodology selection,
  uncertainty band rendering, and per-region overrides are
  plausibly next. A `[inference]` table groups them.
- **TOML tables make the file more readable.** Three flat
  top-level keys is fine; ten is a wall. Grouping by feature area
  is the conventional shape — `[ingest]`, `[storage]`, `[server]`,
  `[auth]`, `[pricing]`, `[factors]` were already in place, all
  feature-area tables. `[inference]` follows that pattern.
- **The back-compat path is cheap and proven.** We already had to
  do the same migration for `claude_code_root` → `claude_code_roots`.
  Same pattern: prefer plural/grouped form, fall back to
  singular/top-level, warn once on the fallback. Users keep working
  configs; the warning nudges toward the canonical form at their
  convenience.

**How to apply.**

- New config fields whose semantics overlap a feature area should
  go under that feature's table, not at the top level. Top-level is
  for genuinely cross-cutting concerns (which we don't currently
  have any of).
- Back-compat reads of deprecated fields should always go through
  an `effective_*` accessor that resolves precedence in one place,
  not direct `config.x.is_some()` checks scattered through the
  codebase. The accessor is also where the deprecation warning fires.
- When the deprecated form is eventually removed (we haven't
  removed any yet), bump the config schema version and document the
  break in `CHANGELOG.md`. Don't silently break configs.

---

## 2026-05-06 — Daily endpoint always returns the full impact block (no `view=` parameter)

**Decision.** `/api/v1/usage/daily` returns the full payload —
tokens, billable equivalents, USD cost, environmental impact — in
every response. There is no `?view=energy` or `?view=cost` switch.

**Why.**

- **The frontend already chose the view client-side**, in the
  ViewMode toggle (`Raw`, `Cost-weighted`, `Cost (USD)`, future
  `Energy`/`CO2`/`Water`). The server has no idea which the user
  picked at any given moment. Round-tripping that state would just
  shift the decision from "where in the JSON do we look" to "what
  do we put in the URL." No net simplification.
- **Payload size cost is trivial.** The impact block adds ~80 bytes
  per (bucket, model) cell. A 1y window × 5 models × daily buckets =
  365 × 5 × 80 ≈ 145 KB. Gzipped it's a fraction of that. Network
  isn't the bottleneck; it's not even on the same order as the
  600KB frontend bundle.
- **Single payload, single source of truth.** The dashboard's
  cross-view interactions (e.g. "switch from Cost view to Energy
  view") happen client-side without a refetch. No race conditions,
  no "I switched too fast and got the wrong data" UX.
- **API consumers external to the dashboard get everything by
  default.** When/if there's a CLI or third-party integration
  reading these endpoints, they don't have to know which `view=`
  to ask for — they get the union and pick what they need.

**How to apply.**

- New per-event metrics (water breakdown, scope-2 carbon, etc.)
  should land as additional fields on the existing `impact` block,
  not as new endpoint variants. The pattern: every read query
  computes everything it can; the frontend renders what the user
  asks for.
- If a future metric genuinely *is* expensive to compute (think:
  full per-event SHA-256s for content auditing), THAT one earns an
  opt-in flag. Default is "include." Opt-out, not opt-in.
- This decision composes with the per-event-resolution one above:
  both push as much as possible into a single read pass. Combined
  they keep the dashboard responsive without per-request
  configuration ceremony.

---

## 2026-05-06 — Multi-machine ingest leans on external sync (Syncthing recommended); native `tokenscale agent` mode deferred to Phase 3

**Decision.** v1 supports multi-machine Claude Code usage by reading from multiple locally-accessible JSONL roots (`[ingest].claude_code_roots = [...]`). Getting JSONL from other machines onto the host running `tokenscale serve` is the user's job, accomplished with any file-sync tool — Syncthing is the recommended default in the README. A future Phase 3 `tokenscale agent` mode (a lightweight daemon on each machine that HTTP-posts events to a central `tokenscale serve`) is deferred; tracked as a future enhancement, not part of v1.

**Why.**
- **Easy install + cross-platform are v1 priorities.** Syncthing is packaged everywhere (`brew`, `apt`, `dnf`, `pacman`, `winget`), runs as a service, requires no account, and works on every OS `tokenscale` targets. Pairing two machines is a UI-driven one-time setup. Anyone willing to install `tokenscale` is willing to install one supporting tool alongside it.
- **A native agent does more work for the same outcome in v1.** Building `tokenscale agent` properly requires auth (shared secret or webauthn), networking config UX (LAN vs WAN, NAT, optional reverse-proxy / Tailscale), conflict resolution against the central DB, and an upgrade story for the agent independent of the server. None of that is necessary for users who can already drop a folder into Syncthing.
- **`claude_code_roots` is sync-tool-agnostic.** Whether the user picks Syncthing, Dropbox, iCloud, OneDrive, `rsync` over cron, or something else, the tokenscale-side config is the same. We don't tie ourselves to any specific sync technology.
- **The agent mode pays off later, not now.** Once `tokenscale` is being packaged for `brew install` etc. (the longer-term distribution goal), having a single binary that can self-orchestrate multi-machine ingest becomes more compelling. Until then, the engineering cost outweighs the marginal install-burden reduction.

**How to apply.**
- Anyone building multi-machine ingest features should target `claude_code_roots` first (it composes with every sync tool) before reaching for new architecture.
- When/if Phase 3 lands a native agent, it should land *additively* — `claude_code_roots` stays the default path, the agent is an opt-in for users who prefer it.
- Don't refactor away the file-walking ingester in favor of a network-only ingester. Local JSONL is the canonical path for local users and stays the fastest install surface.

---

## 2026-05-06 — Manual CSV import is the default billing data path until Admin API is available

**Decision.** Anthropic billing data lands in `tokenscale` via a user-driven CSV import flow (parse, preview, commit). Auto-ingest from the Anthropic Admin API is parked as a follow-on for users with organization accounts; the manual path remains the always-supported fallback.

**Why.**
- The Anthropic Admin API is gated to organization accounts; individual-tier accounts cannot access it.
- Service-account credentials (`svac_...`) use a Workload Identity Federation OIDC exchange that is impractical for a local-dev tool — it assumes the caller is a cloud workload with an IdP-issued OIDC token.
- Neither claude.ai nor the Anthropic Console currently exposes a bulk billing CSV export. Per-invoice PDFs only.
- A manually-composed CSV (assembled from on-screen invoice rows) is workable for typical Claude Pro / Max / Team users: ~12-30 charges per year, parsed in under a minute, repeatable as new invoices arrive.

**How to apply.**
- Keep the `billing_charges` schema source-tagged (`source = 'stripe_csv'` today, `'anthropic_admin'` later). New ingest sources land *additively*; they don't replace the manual path.
- The CSV importer's preview / commit / conflict-resolution flow is the canonical UX; auto-ingest sources should produce the same `BillingCharge` shape and hit the same persistence path.
- Don't gate the manual path behind a feature flag once auto-ingest exists — some users will have data that auto-ingest can't see (older charges, refunds, manual adjustments).

---

## 2026-04-29 — TOML `= null` rewritten to comments at load time

**Decision.** The factor-file loader pre-processes `key = null` lines into comments before handing the TOML to `serde`. Maintainers continue to use the kickoff-prompt convention of `wh_per_mtok_input = null` to mark "explicitly unknown"; missing keys then surface as `Option::None` through `#[serde(default)]`.

**Why.** TOML 1.0 doesn't have a `null` literal. The pre-staged `environmental-factors.toml` uses `= null` extensively to flag values pending Cowork's deliverable 3 — that file is authoritative on shape (per the kickoff prompt), and the convention is more readable than omitting keys entirely. The pre-processor converts on load so the maintainer-facing format stays clean without forcing a change to the TOML spec.

**How to apply.** Anyone editing `environmental-factors.toml` (or future factor-flavored TOMLs that adopt the same convention) can use `key = null` interchangeably with omitting the key. The loader treats both as "value not provided." The pre-processor only fires on lines whose right-hand side is exactly `null` (with optional trailing comment); it never alters content inside string literals.

**Trade-off.** The convention isn't documented anywhere outside this entry and `factors.rs`'s doc comment on `rewrite_null_assignments`. A future maintainer who tries the same in a hand-rolled TOML parser will get a syntax error. Acceptable for Phase 1; if other tools start consuming the file, switch the convention to comments-only and drop the rewriter.

---

## 2026-04-28 — License: Apache-2.0

**Decision.** `tokenscale` ships under the Apache License, Version 2.0.

**Rationale.**

- `tokenscale` is intended for broad public use, including individuals and any organization that finds it useful (no enterprise-only carve-out, but no enterprise-hostile carve-out either).
- Apache-2.0 includes an explicit, irrevocable patent grant from contributors to users. MIT does not — it grants copyright permission only and is silent on patents. For a project that touches usage telemetry and might attract contributions from people working at AI labs or cloud providers, the patent grant is meaningful protection for downstream users.
- Apache-2.0 is the dominant license in modern Rust infrastructure (`tokio`, `axum`, `sqlx`, `reqwest`, the wider Rust toolchain crates) and in the JavaScript build-tooling ecosystem we depend on. Matching that convention reduces friction for contributors and downstream packagers.
- It is GPL-compatible (one-way: Apache-2.0 code can be used in GPLv3 projects, not vice versa), so we do not block anyone whose downstream needs that.

**Alternatives considered.**

- **MIT.** Simpler, equally permissive on the copyright dimension, but lacks the patent grant. For a single-author hobby project it would be fine; for a project we expect external contributions to and that may end up running inside companies, the patent grant matters.
- **MPL-2.0.** File-level copyleft. Reasonable for libraries, but `tokenscale` is an application, not a library someone is going to embed into a closed-source product. The friction of weak-copyleft does not buy us anything here.
- **AGPLv3.** Would force any hosted instance to publish modifications. Wrong for a tool whose primary deployment story is "user runs it on their own laptop against their own data." We do not need network-use copyleft.

**What this commits us to.**

- All source files may carry an Apache-2.0 header but are not required to (the LICENSE file at the repo root is sufficient under Section 4).
- The `NOTICE` file lists the copyright holder. Substantial future contributors who want attribution should add themselves there.
- Any vendored or pasted-in code from elsewhere must be license-compatible (Apache-2.0, MIT, BSD, ISC, etc.) and credited in `NOTICE`.

---

## 2026-04-28 — Frontend styling: Tailwind from day one

**Decision.** The Vite + React frontend uses Tailwind CSS from the first commit, not plain CSS or CSS modules.

**Rationale.**

- Phase 1 ships one chart, but the dashboard's roadmap includes sortable tables (recent sessions), a filter bar (provider, date range, model), a research-runs review surface with diff rendering, and time-window controls. That is more UI than plain CSS comfortably scales to.
- Tailwind composes well with Recharts (the chosen chart library) and with whatever sortable-table primitive we adopt (TanStack Table is the leading candidate). Both are styling-agnostic and pick up Tailwind utility classes with no friction.
- Retrofitting a utility-class CSS framework onto an existing component tree is more work than starting with one. The marginal cost now (~10 minutes of setup, one config file, one build-time PostCSS pass) is much less than the marginal cost later.
- The maintainer's stated preference is "build it right the first time."

**What this commits us to.**

- A small PostCSS pipeline in the frontend (`tailwindcss`, `autoprefixer`, `postcss`).
- A `tailwind.config.js` with sensible defaults; theming via CSS variables so a later dark mode is additive.
- Discipline on not abusing `@apply` — utility classes in JSX, custom components for repeated patterns, no big bespoke stylesheet.

---

## 2026-04-28 — Workspace member: separate ingest crate per source

**Decision.** Each ingest source gets its own crate (`tokenscale-ingest-cc`, `tokenscale-ingest-api`, future `tokenscale-ingest-openai`, etc.) rather than a single `tokenscale-ingest` with submodules.

**Rationale.**

- Per the CHARTER's v2-readiness section, adding a new provider should be additive — a new crate, not edits to a shared file. Separate crates enforce that boundary at the build-system level.
- Compile times: a change to the JSONL parser does not invalidate the Admin API client, and vice versa.
- Optional features: Phase 3 may want to ship binaries that exclude one or another ingest path (e.g., a pure-Admin-API build for users who do not run Claude Code locally). Per-crate boundaries make that mechanical.

**Trade-off.** Six crates is more `Cargo.toml` files than five. Worth it.

---

## 2026-04-28 — sqlx offline mode (`.sqlx/` cache committed)

**Decision.** sqlx 0.8 is used in compile-time-verified mode. The `.sqlx/` query cache is committed to the repo, and CI builds with `SQLX_OFFLINE=true`.

**Rationale.**

- Compile-time verification catches schema drift between SQL and Rust code at `cargo check` time, not at runtime.
- Without offline mode, every contributor and every CI run would need a live SQLite database with the schema applied. With offline mode they do not.
- The `.sqlx/` directory is a small JSON cache — text, diff-friendly, easy to review.

**What this commits us to.**

- After editing any `sqlx::query!` or `sqlx::query_as!` invocation, run `cargo sqlx prepare --workspace` and commit the resulting `.sqlx/` changes.
- Document this step in the contributor section of the README.

---

## 2026-04-28 — MSRV pinned in Cargo.toml; no `rust-toolchain.toml`

**Decision.** Workspace `Cargo.toml` declares `rust-version = "1.82"` (revised from the original 1.78 baseline on 2026-04-29 when the JSONL ingester landed using `Option::is_none_or`). We do not ship a `rust-toolchain.toml` file.

**Rationale.**

- `rust-version` produces a friendly compiler error if a too-old toolchain is used.
- A `rust-toolchain.toml` file would force every contributor to install a specific Rust version even if their installed version is newer and compatible. That is appropriate for a project pinned to a nightly feature; it is hostile for a stable-channel project.
- 1.82 (released October 2024) is still over 18 months old at scaffold time. The features it stabilized — `Option::is_none_or`, the precise capturing syntax — are useful enough that the cost of an MSRV bump beats the cost of working around them.
- Future bumps follow the same rule: if a feature is genuinely useful and the version it was stabilized in is over a year old, bump and document. Else work around.
