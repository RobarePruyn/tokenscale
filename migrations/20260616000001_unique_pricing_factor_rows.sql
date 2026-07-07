-- Quick-pass hardening (post-v0.1.19, pre-full-model-coverage): UNIQUE
-- indexes on the pricing/factor resolution keys.
--
-- Why: the aggregate joins resolve each event's rate row via
-- `valid_from = (SELECT MAX(valid_from) ...)` equality. With no UNIQUE
-- constraint, two rows sharing (provider, model, valid_from) both match
-- and every cost/energy SUM double-counts silently. Hand-authoring the
-- ~25-model historical backfill makes that duplicate typo likely, so
-- the guard lands BEFORE the backfill. See
-- docs/assessment-full-codebase-2026-06.md section 2 item 1.
--
-- Behavior change (deliberately loud): pricing/factor sync is
-- delete-all-then-insert inside one transaction, so a TOML file that
-- contains two rows with the same (model, valid_from) now fails the
-- sync (transaction rolls back, serve refuses to start) instead of
-- silently double-counting in every aggregate. Fail-loud matches the
-- file's existing verification-gate philosophy.
--
-- Not a data migration: indexes only, no rows touched, no backfill
-- semantics. The real DB was empirically checked for existing
-- duplicates before this migration was written (zero found in all
-- three tables).

CREATE UNIQUE INDEX pricing_provider_model_validfrom_unique
    ON pricing (provider, model, valid_from);

CREATE UNIQUE INDEX env_factors_provider_model_validfrom_unique
    ON env_factors (provider, model, valid_from);

CREATE UNIQUE INDEX grid_factors_region_validfrom_unique
    ON grid_factors (region, valid_from);
