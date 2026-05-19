-- tokenscale — v0.1.13 phase B: add `notes` column to `pricing`.
--
-- The `pricing` table itself was provisioned in the v0.1.0 initial
-- migration (20260428000001_initial.sql lines 87-104) but never
-- populated — runtime pricing lookups went through the in-memory
-- `PricingFile` snapshot. v0.1.13 promotes pricing into per-event
-- DB lookup mirroring `env_factors`, which means `pricing_sync`
-- writes the TOML rows into the table on every startup.
--
-- `notes` is added here because `ModelPricing` in the core crate
-- carries an `Option<String>` notes field that the `has_seed_markers`
-- production gate scans on startup. Without this column, the sync
-- would either have to drop notes silently (losing the gate
-- coverage on DB-resident rows) or write them somewhere else.
--
-- ALTER TABLE ADD COLUMN with no default is the SQLite-friendly form;
-- existing rows (currently zero — table was empty) get NULL.

ALTER TABLE pricing ADD COLUMN notes TEXT;
