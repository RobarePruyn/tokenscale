-- tokenscale — Phase 2/v0.1.4: grid-factor uncertainty columns.
--
-- v0.1.3 added `co2e_uncertainty_range_pct` / `water_uncertainty_range_pct`
-- to the in-memory `GridFactors` struct (display-only). v0.1.4 promotes them
-- into the database so the per-event compute path can carry grid uncertainty
-- through to the bucket aggregate alongside model uncertainty.
--
-- Additive columns — additions to a table that's already mid-schema, no
-- recreate-and-copy needed since both columns are nullable. The next factor-
-- file sync will populate them from `environmental-factors.toml`.

ALTER TABLE grid_factors ADD COLUMN co2e_uncertainty_range_pct  INTEGER;
ALTER TABLE grid_factors ADD COLUMN water_uncertainty_range_pct INTEGER;
