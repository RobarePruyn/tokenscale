-- tokenscale — v0.1.7: indirect-water grid factor columns.
--
-- Sweep #2 (2026-05-15) shipped per-subregion `indirect_water_l_per_kwh`
-- values in the TOML factor file: off-site / power-plant cooling water,
-- distinct from on-site DC cooling (`water_l_per_kwh`). Methodology
-- follows Ren et al. 2024 "Making AI Less Thirsty" — Macknick 2012
-- per-fuel coefficients weighted by eGRID2023 fuel-mix per subregion.
--
-- This migration promotes the two new fields into the DB so the
-- per-event compute path can carry them through to the aggregate,
-- alongside the direct-water columns. Both nullable — legacy factor
-- files without indirect-water data sync to NULL, which the compute
-- path treats as "indirect water unavailable for this region."

ALTER TABLE grid_factors ADD COLUMN indirect_water_l_per_kwh           REAL;
ALTER TABLE grid_factors ADD COLUMN indirect_water_uncertainty_range_pct INTEGER;
