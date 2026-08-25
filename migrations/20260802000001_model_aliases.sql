-- Full-model-coverage D1 (v0.1.20): model-ID alias table.
--
-- Why: every resolution site matched events.model to pricing/factor rows
-- by exact string. Real emitted IDs vary in form (dated pinned snapshots
-- before the 4.6 generation, dateless IDs from 4.6 on, convenience
-- aliases like claude-sonnet-4-5 that resolve to a dated snapshot), so
-- a form mismatch silently dropped a model from BOTH cost and impact
-- (Issue #7; surfaced by the v0.1.19 dated-Haiku case). This table maps
-- every known alias to its canonical row key. Resolution sites join it
-- and resolve with COALESCE(ma.canonical, events.model); events.model
-- itself and every GROUP BY events.model stay raw so display keys remain
-- faithful to what the source emitted.
--
-- Canonical keys, per Anthropic's model-ID docs: dated IDs for models
-- before the 4.6 generation, dateless IDs for 4.6 and later.
--
-- Sync posture: replace-on-startup from pricing.toml's
-- [providers.<provider>.aliases] table, same as the pricing and factor
-- tables. Not a forward-only data migration: no rows are touched, no
-- backfill semantics; the alias set is fully derived from the TOML on
-- every start.
--
-- A canonical key that also appears as a raw alias would make the
-- COALESCE self-referential; the UNIQUE on (provider, raw) plus the
-- sync-time check that no canonical is itself an alias key prevent it.

CREATE TABLE model_aliases (
    id         INTEGER PRIMARY KEY,
    provider   TEXT NOT NULL,
    raw        TEXT NOT NULL,
    canonical  TEXT NOT NULL
);

CREATE UNIQUE INDEX model_aliases_provider_raw_unique
    ON model_aliases (provider, raw);
