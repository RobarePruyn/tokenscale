# request-for-research

This file is the engineering side's request channel to the Cowork research project that maintains the environmental-factor model.

When the engineering side discovers that the application needs a new field, value, or capability from the research side, an entry is appended below. The maintainer copies entries from this file into the Cowork project for the research agent to act on.

Pairs with `request-for-code-change.md` in the Cowork project, which carries requests in the opposite direction (research → engineering).

---

## Format

Each entry uses this template:

```markdown
### YYYY-MM-DD — short title

**Status:** open | in-progress | resolved | dropped
**Asks:** what we want from the research side, in concrete terms.
**Why:** what application behavior depends on this — the user-visible effect of having (or not having) the new field, value, or capability.
**Where:** which crate, file, or feature in the engineering tree the request relates to.
**Notes:** anything that would help the research agent scope the work — a deadline, a related source already known, a sketch of the data shape we'd want.
```

New entries go on top. Resolved entries stay in place as historical record; do not delete.

---

## Entries

### 2026-04-29 — Deliverable 3 (v0.1 environmental factor values)

**Status:** open
**Asks:** Populate `environmental-factors.toml` with the v0.1 numeric values for `wh_per_mtok_input`, `wh_per_mtok_output`, `wh_per_mtok_cache_read`, `wh_per_mtok_cache_write_5m`, `wh_per_mtok_cache_write_1h` for each of `claude-opus-4-7`, `claude-opus-4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5`, plus `co2e_kg_per_kwh`, `water_l_per_kwh`, and `pue` for `us-east-1`, `us-east-2`, `us-west-2`. Flip `file_status` to `"production"` once merged.
**Why:** Phase 1 of `tokenscale` shipped the loader, schema-version guard, DB sync, and `/api/v1/health` exposure for environmental factors — but the dashboard's "Energy / Water / CO₂e" view is gated on real values (Phase 2 work). Until those land, the dashboard can't surface any environmental signal at all; the factor-model file's `is_placeholder = true` keeps the Phase 2 UI dark.
**Where:** [`environmental-factors.toml`](environmental-factors.toml). The application loads it via `tokenscale-core::factors::EnvironmentalFactorsFile`; the in-memory snapshot lives in `AppState`, the DB rows in `env_factors` and `grid_factors`. No code change is needed when the values land — the dashboard will pick them up on the next `tokenscale serve` startup.
**Notes:** Source bibliography is [`docs/sources.md`](docs/sources.md). The Couch 2026-01-20 blog (G.1) is the strongest empirical anchor for Haiku 4.5; Opus and Sonnet of the 4-family generation extrapolate from it. Methodology marker in `defaults.methodology` is `"google-comprehensive-aug-2025"` per the kickoff prompt — the Phase 2 compute path expects PUE-weighted facility overhead, not just active GPU.
