# tokenscale — Decision Log

A running record of non-obvious decisions made during the build, with the rationale captured at the time. New entries go on top.

The goal here is simple: anyone (the maintainer, a future contributor, a future me) should be able to read this and understand *why* something is the way it is, without having to reconstruct the conversation that led to it.

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
