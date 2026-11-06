# TokenScale as an engineering-cost service: product spec, schema, API, attribution gaps

**Status**: DRAFT for sign-off, 2026-11-06. Decisions D1 to D8 below need explicit sign-off before any code or migration lands (project rule). Nothing in this document is built.

**Brief (verbatim intent)**: TokenScale exists to answer one question: what does each unit of Mediacast engineering output cost in model spend, and is that trending the right way. It will feed a business-ops/BI application, so everything it computes must be reachable via a stable API, not just rendered in the dashboard.

**Grounding**: written against the v0.1.24 codebase (inventory in section 7) and the maintainer's production database on 2026-11-06 (96,552 Claude Code events, 236 sessions, 1,704 captured commits). Every attribution claim below is measured, not assumed.

---

## 1. Product spec

### 1.1 The question, decomposed

"Cost per unit of engineering output" needs three things TokenScale does not have today and one it does:

| Need | Today |
|---|---|
| Spend per event, effective-dated, with cache economics | Yes. Per-event API list-rate cost via time-anchored `pricing` rows, aliases, and per-model cache-read pricing. |
| A unit of output to divide by (PR, feature, project) | Partial. Project (cwd) and session exist; commits are captured for 85% of event volume; PR, feature, and customer do not exist anywhere. |
| A stable, authenticated API a third party can call | No. Loopback-only, no auth, no OpenAPI, inconsistent field casing, response shapes built for one SPA. |
| A tier dimension matching a routing policy | No. No tier concept at all; `service_tier` is in the JSONL and not captured. |

The service is therefore: the existing cost engine, plus an attribution layer with an explicit "unattributed" state, plus a query API over SQL views, running on the platform's Postgres with Keycloak auth. Revenue, BI, and new dashboards are out of scope.

### 1.2 Users and consumers

- **BI application** (first consumer): time-series and group-by cost queries by attribution key; joins revenue on its side.
- **Steward** (routing policy): the source of tier labels; TokenScale consumes its policy, never defines it.
- **Engineers** (today's dashboard): unchanged until the API is stable; the existing SPA becomes one more API consumer later.
- **Ingest agents** on developer machines: push normalized events to the service.

### 1.3 Capabilities, in the brief's priority order, with what each requires

1. **Ingestion.** Claude Code JSONL (exists), Anthropic Admin API usage and cost reports (crate is a stub; wiring it gives `api_key_id` and `workspace_id`), gateway usage (OpenRouter generations export; nothing exists). All three normalize to one `usage_events` row shape (section 2). Ingest moves from "the server scans this laptop's disk" to "agents push to the service," which also retires the Syncthing-based multi-machine workaround.
2. **Attribution.** Project, branch, PR, feature or epic, customer. Partial by design; every event always resolves to exactly one row per dimension, and that row may be the sentinel `unattributed` with a reason (section 2.3).
3. **Cost model.** Effective-dated rates (exists; extend with a prompt-length tier for Haiku 5.5, Issue #11), subscription versus API pricing (exists as flat monthly rows; needs a real plan and seat model), and tier labels from Steward (new `tier_assignments` table, effective-dated).
4. **Core queries.** Cost per PR, per feature, per project, cost by tier over time, frontier share of spend, fan-out anomalies. All as SQL views the BI app may read directly (section 3).
5. **Guardrails.** Budget thresholds evaluated by a job; read-only status endpoint and an alert sink. No enforcement.
6. **API.** `/api/v2`, OpenAPI generated from code, Keycloak bearer auth, uniform query contract (section 4).

### 1.4 Explicit non-goals

Revenue attribution; the BI app; any new dashboard view before `/api/v2` is stable; billing, throttling, or touching provider account state (charter non-goals carry over); predicting prices.

### 1.5 The fork this creates (read before the decisions)

tokenscale today is a self-hostable, local-first, single-user tool with a charter built on privacy and SQLite. The brief describes an internal multi-user service on shared infrastructure. Both can live in one workspace, but not as one deployment. The decisions below assume: the **service** is the product this spec defines; the **local tool** stays maintained as the ingest agent plus an offline mode, and keeps its SQLite store. Section 5 lists what that costs.

---

## 2. Normalized schema (Postgres; names are proposals)

Conventions: `timestamptz` everywhere; money as `numeric(14,6)` USD; every fact table has `source_id`; every attribution dimension has a sentinel row with `id = 0` named `unattributed`; nothing is deleted on re-ingest (dedupe keys instead).

### 2.1 Reference and rate tables (effective-dated, never a single current number)

```
sources            (id, kind, provider, display_name, enabled)            -- claude_code | anthropic_admin | openrouter | ...
model_aliases      (provider, raw, canonical)                              -- as today, synced from pricing.toml
price_rates        (provider, model, valid_from, valid_to,
                    prompt_tokens_min, prompt_tokens_max,                  -- NULL/NULL = all prompts; Haiku 5.5 gets two rows (Issue #11)
                    input_usd_per_mtok, output_usd_per_mtok, cache_read_usd_per_mtok,
                    cache_write_5m_multiplier, cache_write_1h_multiplier,
                    source_url, source_accessed_at, notes)
                    UNIQUE (provider, model, valid_from, prompt_tokens_min)
tiers              (tier, display_name, description)                       -- vocabulary defined by Steward's routing policy
tier_assignments   (provider, model, tier, valid_from, valid_to, policy_version, source_url)
                    -- a model can change tier when Steward's policy changes; effective-dated like prices
plans              (id, provider, plan_name, monthly_usd, seats, valid_from, valid_to, notes)
                    -- replaces today's subscriptions
plan_seats         (plan_id, principal, valid_from, valid_to)              -- principal = Keycloak subject; today: one row
charges            (id, source_id, occurred_at, amount_usd, category, description, external_id, raw)
                    -- today's billing_charges, unchanged
env_factors, grid_factors                                                  -- unchanged; impact stays in the service
```

### 2.2 Fact tables

```
usage_events (
  id              bigint PK,
  source_id       int  FK sources,
  occurred_at     timestamptz,
  provider        text,
  model_raw       text,                     -- as emitted
  model           text,                     -- canonical via model_aliases (materialized at ingest)
  tier            text,                     -- resolved from tier_assignments at occurred_at; 'untiered' if none
  input_tokens, output_tokens, cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens  bigint,
  prompt_tokens   bigint,                   -- input + cache_read + cache writes; the key for prompt-length pricing
  service_tier    text,                     -- provider-reported (JSONL service_tier), kept raw
  cost_usd        numeric(14,6),            -- API list-rate cost at occurred_at, materialized at ingest; NULL = unpriced
  cost_rate_id    bigint FK price_rates,    -- which rate priced it (audit trail; NULL = unpriced)
  session_id      text,
  request_id      text, content_hash text, uuid text, parent_uuid text,
  -- attribution keys, always populated (0 = unattributed):
  project_id      int FK projects,
  branch_id       int FK branches,
  pr_id           int FK pull_requests,
  feature_id      int FK features,
  customer_id     int FK customers,
  attribution_version int,                  -- bump when the resolver changes; a backfill job re-resolves older versions
  raw             jsonb,                    -- optional, per ingest config
  UNIQUE (source_id, request_id) WHERE request_id IS NOT NULL,
  UNIQUE (source_id, content_hash) WHERE request_id IS NULL
)
sessions (session_id PK, source_id, first_event_at, last_event_at, project_id, branch_id, machine, principal)
session_commits (as today, plus branch_id, pr_id)                           -- Tier 1 attribution evidence
tool_uses, tool_results, file_snapshots                                    -- as today; evidence for Tier 2 later
```

Why materialize `cost_usd` at ingest instead of the correlated subquery at query time: the BI app reads views directly and must not depend on a query-time join, and a price correction (Sonnet 5.5 on 2026-10-08) becomes a re-price job that updates `cost_usd` and `cost_rate_id` for the affected rows with a row count, which is the audit trail the current design lacks.

### 2.3 Attribution dimensions, each with an explicit unattributed state

```
projects      (id, canonical_key, display_name, repo_remote_url, kind)       -- kind: repo | worktree_of | non_git | unattributed
project_paths (project_id, machine, path)                                    -- the 558 cwd strings map here (about 12 repos today)
branches      (id, project_id, name, kind)                                   -- kind: named | detached | unattributed
pull_requests (id, project_id, number, title, head_branch, merged_at, url)   -- from GitHub
features      (id, key, title, source)                                       -- epic or feature from the tracker; source: github_project | jira | manual
customers     (id, key, display_name)                                        -- only a key; revenue joins happen downstream
attribution_reasons (event_id, dimension, reason)                            -- no_cwd | detached_head | no_commit | sha_unresolved |
                                                                             --   no_pr_for_branch | no_feature_map | no_customer_map
```

Rules: a row is never dropped for lacking a key. `unattributed` is a real group in every group-by result, with its share reported, so the BI app can show "72% of spend attributed to a PR" rather than silently under-count.

### 2.4 Guardrails

```
budgets        (id, scope_dimension, scope_key, window, amount_usd, threshold_pct[], valid_from, valid_to)
budget_status  (budget_id, evaluated_at, spend_usd, pct, breached_threshold_pct)   -- written by a job, read by the API
alerts         (id, budget_id, fired_at, threshold_pct, delivered_to)              -- sink only; no enforcement
```

---

## 3. Core queries as SQL views

All views share one grain (`usage_events` joined to its dimensions) and are safe for the BI app to read directly. Materialize the daily rollup; everything else is a plain view over it.

| View | Grain | Definition sketch |
|---|---|---|
| `v_cost_daily` (materialized, refreshed hourly) | day x provider x model x tier x project x branch x pr x feature x customer x source | sums of tokens and `cost_usd`, event count, unpriced count |
| `v_cost_by_pr` | pr | from `v_cost_daily` where `pr_id <> 0`, plus the unattributed share per project |
| `v_cost_by_feature` | feature | same shape; join `features` |
| `v_cost_by_project` | project x day | same shape; includes the `unattributed` project row |
| `v_cost_by_tier` | tier x day | join `tier_assignments` as of day; `frontier_share = cost(tier = frontier) / cost(all)` |
| `v_frontier_share` | day | derived from `v_cost_by_tier` |
| `v_session_cost` | session | per-session sum, with a trailing-30-day median from a window function |
| `v_fanout_anomalies` | session | sessions where `cost > N * median` (N from a parameter table, default 10; today's data: median 44 events per session, p95 1,866, max 18,057) |
| `v_subscription_vs_api` | month x plan | billed (`charges` plus prorated `plans`) versus counterfactual `cost_usd`; moves the frontend's 30.4375-day proration into SQL |

Note: the current code rejects views in favor of correlated subqueries (`docs/decisions.md`, 2026-05-06) for a single-user SQLite dataset. That decision was explicitly conditioned on scale and consumer; both change here. The materialized daily rollup is the only place a refresh lag is accepted.

---

## 4. API surface (`/api/v2`)

### 4.1 Conventions

- Base path `/api/v2`; `/api/v1` keeps serving the SPA unchanged until the SPA is ported, then retires on a dated schedule.
- Auth: `Authorization: Bearer <Keycloak JWT>`; roles `tokenscale.read` (BI, dashboards), `tokenscale.ingest` (agents), `tokenscale.admin` (budgets, plans, tiers). Service accounts for the BI app and the agents.
- OpenAPI 3.1 generated from code (utoipa), served at `/api/v2/openapi.json`; the document is the contract and is versioned with the crate.
- All fields `snake_case`. All timestamps RFC 3339 UTC. Money as decimal strings in USD.
- One query contract for every aggregate endpoint:
  - `from`, `to` (RFC 3339 or date), `granularity` (`day | week | month`)
  - `group_by` (comma list from `provider, model, tier, project, branch, pr, feature, customer, source, session`)
  - filters: `provider=`, `model=`, `tier=`, `project=`, `branch=`, `pr=`, `feature=`, `customer=`, `source=`, `attributed=true|false`
  - pagination on row-shaped endpoints: `limit`, `cursor`
- One response shape:

```json
{
  "window": {"from": "...", "to": "...", "granularity": "day"},
  "totals": {
    "cost_usd": "123.456", "events": 1234,
    "tokens": {"input": 0, "output": 0, "cache_read": 0, "cache_write_5m": 0, "cache_write_1h": 0},
    "unpriced_events": 0,
    "unattributed_share": {"project": 0.00, "branch": 0.63, "pr": 0.46, "feature": 1.00, "customer": 1.00}
  },
  "groups": [
    {"key": {"project": "mediacast/platform", "tier": "frontier"},
     "totals": {"...": "same shape as above"},
     "series": [{"t": "2026-11-01", "cost_usd": "...", "events": 0, "tokens": {"...": 0}}]}
  ],
  "meta": {"pricing_file_version": "1.4", "attribution_version": 3, "generated_at": "..."}
}
```

### 4.2 Routes

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v2/cost` | the general aggregate: totals plus series for any `group_by`; backs every "cost per X" question |
| GET | `/api/v2/cost/by-pr` | convenience over `/cost?group_by=pr`; adds PR metadata (title, merged_at, url) and attributed share |
| GET | `/api/v2/cost/by-feature` | same for features |
| GET | `/api/v2/cost/by-project` | same for projects, with each project's path aliases |
| GET | `/api/v2/cost/by-tier` | tier series plus `frontier_share` per bucket |
| GET | `/api/v2/anomalies/sessions` | fan-out anomalies: `multiple` (default 10) and window; sessions with cost, median, ratio, project, branch |
| GET | `/api/v2/sessions`, `/api/v2/sessions/{id}` | row-shaped, paginated; per-session cost, attribution, commits |
| GET | `/api/v2/sessions/{id}/commits` | as v1, plus `branch` and `pr` |
| GET | `/api/v2/attribution/coverage` | share of events and spend attributed per dimension per window; the honesty endpoint |
| GET | `/api/v2/rates` | effective-dated price rates and tier assignments (`as_of=` optional) |
| GET, POST, PUT | `/api/v2/tiers`, `/api/v2/tiers/assignments` | Steward's vocabulary and model-to-tier mapping (admin) |
| GET, POST, PUT, DELETE | `/api/v2/plans`, `/api/v2/plans/{id}/seats` | subscription plans and seats (admin) |
| GET, POST | `/api/v2/charges`, `/api/v2/charges/import` | billed charges; CSV and Admin API imports |
| GET, POST, PUT, DELETE | `/api/v2/budgets`; GET `/api/v2/budgets/{id}/status` | guardrails, read-only evaluation |
| POST | `/api/v2/ingest/events` | agents push normalized events (batched, idempotent on dedupe keys); returns accepted and duplicate counts |
| POST | `/api/v2/ingest/reprice` | admin: re-price a window after a rate correction; returns rows touched |
| GET | `/api/v2/health`, `/api/v2/openapi.json` | liveness plus the contract |

The impact endpoints (`/factors/active` and the environmental fields) carry over unchanged under `/api/v2/impact/*`; they are not part of the BI contract but cost nothing to keep.

---

## 5. Decisions for sign-off

**D1. Storage.** Postgres is the canonical store for the service; SQLite survives only inside the ingest agent (local buffer and offline dashboard). Recommendation: put the store behind a trait and build the Postgres backend first; the three time-anchored tables, aliases, and `usage_events` port directly (the SQL is ANSI apart from `date()` calls). Cost: two backends to test, and every `sqlx::query!` needs `.sqlx/` entries for both. Alternative: Postgres only and retire the local tool. Rejected because the agent needs a buffer and the OSS tool has users.

**D2. Ingest topology.** Agents on developer machines parse JSONL locally and push to `/ingest/events`; the service never reads a laptop's disk. The Admin API connector runs server-side. Gateway (OpenRouter) ingest is a server-side poller of the generations export. Alternative: keep server-side disk scanning plus Syncthing. Rejected; that is the current pain.

**D3. Cost materialization.** `cost_usd` and `cost_rate_id` are set at ingest, with a `reprice` job for corrections. Alternative: keep query-time correlated subqueries. Rejected for the BI-reads-views requirement and for auditability (the Sonnet 5.5 correction of 2026-11-06 would have been a one-command reprice with a row count instead of a silent recompute).

**D4. Attribution representation.** A sentinel `unattributed` row per dimension plus `attribution_reasons`, and an `attribution_version` on every event so resolver improvements backfill deterministically. Alternative: nullable keys. Rejected; nulls disappear from group-bys and the brief forbids dropping rows.

**D5. Project identity.** The canonical key is the normalized git remote URL, not the toplevel path. Measured reason: 558 cwd strings in the maintainer's data collapse to about 12 repositories, split by worktrees, user-name case (`Robare` vs `robare`), and iCloud versus `~/Dev` paths. Non-git cwds become `kind = non_git` projects keyed by path. Alternative: toplevel path (today). Rejected; it splits one repo across machines and worktrees.

**D6. Tier source.** Tiers are Steward's vocabulary, imported as effective-dated `tier_assignments` with a `policy_version`. TokenScale never invents a tier; until Steward publishes one, every event is `untiered` and the frontier-share view says so. Needs a one-line contract from Steward: (model, tier, effective date), ideally a JSON endpoint or a file in its repo.

**D7. API contract.** `/api/v2` as in section 4, utoipa-generated OpenAPI, Keycloak roles as named. v1 stays for the SPA only. Alternative: evolve v1 in place. Rejected; v1's camelCase and snake_case mix and SPA-shaped responses are not a contract anyone else should be asked to honor.

**D8. Views for BI.** Materialized daily rollup plus plain views as in section 3; supersedes the 2026-05-06 "no views" decision for the service deployment only. Record the supersession in `docs/decisions.md`.

---

## 6. Attribution data we lack, and where it would come from

Measured against the production database on 2026-11-06.

| Key | Today | Gap | Source and mechanism | Effort |
|---|---|---|---|---|
| **Project** | 100% of events have a cwd; resolved to git toplevel at query time | 558 strings for about 12 repos; worktrees and path variants split projects | Resolve at ingest to the normalized `git remote get-url origin`; `project_paths` keeps every alias | Small; data exists |
| **Branch** | `git_branch` on 99.5% of events, but 63% read `HEAD` (detached worktrees created by Claude Code sessions) | Branch unknown for most events | At ingest, for `HEAD`: read the worktree's `.git/HEAD`, or map the worktree via `git worktree list`; for merged worktrees, recover from the commit SHA's branch membership | Small to medium; the agent already runs git locally |
| **Commit (Tier 1)** | 1,704 commits captured; 85% of event volume sits in sessions with a commit; 54% in sessions with a resolved SHA | Half the captured commits have no SHA (867 of 1,704 are `recovery_source = none`): pipes, `| tail`, failed commits | Split `none` into committed-unlinkable, failed, unknown (already scoped in roadmap-2 section 10); the Tier 3 post-commit hook trailer makes new commits exact | Medium; the Tier 3 hook is the real fix |
| **PR** | Nothing | No PR key anywhere | GitHub API: commit SHA to PR (`/repos/{r}/commits/{sha}/pulls`), and head-branch name to PR for unmerged work; cached in `pull_requests`; needs a GitHub App or token with read on Mediacast repos | Medium; depends on branch and SHA quality above |
| **Feature or epic** | Nothing | No tracker link | Whatever Mediacast uses for epics: GitHub Projects fields or labels, or Jira keys in branch names and PR titles (branch names like `feat/phase1-a-tenancy-foundation` already encode phases); a `features` import plus a branch and PR-name parser | Medium; needs a decision on the tracker of record |
| **Customer** | Nothing | No customer concept in engineering data | Only derivable from a feature-to-customer or project-to-customer mapping maintained outside TokenScale; TokenScale stores the key and the mapping table, nothing more | Small here; the mapping is someone else's data |
| **Principal (who)** | None; single user | A multi-user service needs the human or agent behind a session | Agent config carries the Keycloak principal; the Admin API gives `api_key_id` | Small |
| **Machine** | Implicit in paths | Which machine produced the session | Agent config | Trivial |
| **API key and workspace** | Always null (Claude Code emits none; the Admin API crate is a stub) | Cannot split API spend by key or workspace | Wire `tokenscale-ingest-api` to `/v1/organizations/usage_report/messages` and `/cost_report` | Medium |
| **Tier** | Nothing; `service_tier` is in the JSONL and not captured | No tier dimension; frontier share cannot be computed | Capture `service_tier`; import Steward's assignments (D6) | Small plus an external contract |
| **Gateway usage** | Nothing | Open-weight eval spend is invisible | OpenRouter generations export (per-generation model, tokens, cost, app or key); normalize to `usage_events` with `source = openrouter` | Medium; a new ingest crate |
| **Prompt-length tier** | One row for Haiku 5.5 | Over-100k prompts underpriced 5x on that model | `price_rates.prompt_tokens_min` and `max` (section 2.1); Issue #11 | Small |

---

## 7. What exists today (the inventory the spec was written against)

Single-user axum server bound to loopback, SQLite store, embedded SPA, no auth, no OpenAPI, every route under `/api/v1` with mixed field casing. Per-event API list-rate cost is computed in SQL at query time through a correlated `valid_from` subquery with alias resolution and per-model cache-read rates. Subscriptions are flat monthly rows; the subscription-versus-API comparison lives in the frontend. Commit attribution is Tier 1 (Bash `git commit` extraction) and is not joined to cost anywhere. Ingest is Claude Code JSONL only; the Admin API crate is a stub; nothing exists for gateways. `git_branch`, `parent_uuid`, and `service_tier` are captured or available and unused. Route and column inventory: `docs/architecture.md` and the store migrations; the per-route table produced for this spec matches the code at v0.1.24.

## 8. Suggested sequencing (a dependency order, not a plan)

1. D5 project identity and branch recovery in the existing tool: pure attribution quality, benefits v1 immediately, needs no infrastructure.
2. Tier 3 post-commit hook trailer: makes new commits exact and unblocks PR mapping.
3. Postgres store behind the trait plus the agent push path (D1, D2, D3).
4. `/api/v2` with OpenAPI and Keycloak (D7), the views (D8), and the Admin API connector.
5. PR, feature, customer, Steward tiers, OpenRouter, budgets, in whatever order their external dependencies unblock.
