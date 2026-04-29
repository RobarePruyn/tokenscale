-- tokenscale — Phase 2 environmental factor columns (migration 0002).
--
-- Cowork's v0.1 production factor file landed with fields the Phase 1
-- schema didn't model:
--
--   * env_factors needs `uncertainty_range_pct` and `confidence` —
--     surfaced through the API so the dashboard can render ± bands and
--     primary/secondary tags.
--   * grid_factors needs `egrid_subregion` and `egrid_subregion_full_name`
--     so the environmental banner can show the user "us-east-1 → SRVC
--     (SERC Virginia/Carolina)" without re-deriving from notes.
--   * grid_factors `co2e_kg_per_kwh` and `pue` need to be nullable.
--     Phase 1 wrote sentinel zeros when the TOML had nulls; Phase 2
--     compute distinguishes "0" from "unavailable", so the column types
--     have to preserve the difference.
--
-- The grid_factors table change requires a recreate-and-copy because
-- SQLite can't ALTER TABLE to drop NOT NULL.

PRAGMA foreign_keys = ON;

-- ----------------------------------------------------------------------------
-- env_factors: additive columns
-- ----------------------------------------------------------------------------
ALTER TABLE env_factors ADD COLUMN uncertainty_range_pct INTEGER;
ALTER TABLE env_factors ADD COLUMN confidence            TEXT;

-- ----------------------------------------------------------------------------
-- grid_factors: recreate with nullable co2e/pue + new egrid_subregion fields
-- ----------------------------------------------------------------------------
CREATE TABLE grid_factors_new (
    id                         INTEGER PRIMARY KEY AUTOINCREMENT,
    region                     TEXT NOT NULL,
    valid_from                 TEXT NOT NULL,
    valid_to                   TEXT,

    co2e_kg_per_kwh            REAL,                  -- nullable in Phase 2
    water_l_per_kwh            REAL,
    pue                        REAL,                  -- nullable in Phase 2

    -- New in v0.1 production:
    egrid_subregion            TEXT,
    egrid_subregion_full_name  TEXT,

    source_url                 TEXT NOT NULL,
    source_accessed_at         TEXT NOT NULL
);

INSERT INTO grid_factors_new (
    id, region, valid_from, valid_to,
    co2e_kg_per_kwh, water_l_per_kwh, pue,
    egrid_subregion, egrid_subregion_full_name,
    source_url, source_accessed_at
)
SELECT
    id, region, valid_from, valid_to,
    -- Phase 1 wrote sentinel 0.0 for nulls. Convert back to NULL so
    -- compute can distinguish "computed and zero" from "unavailable."
    CASE WHEN co2e_kg_per_kwh = 0.0 THEN NULL ELSE co2e_kg_per_kwh END,
    water_l_per_kwh,
    CASE WHEN pue = 1.0 THEN NULL ELSE pue END,
    NULL,                                     -- egrid_subregion populated on next sync
    NULL,                                     -- egrid_subregion_full_name populated on next sync
    source_url, source_accessed_at
FROM grid_factors;

DROP TABLE grid_factors;
ALTER TABLE grid_factors_new RENAME TO grid_factors;

-- The (region, valid_from) index from migration 0001 was attached to the
-- old table; recreate it on the new one.
CREATE INDEX grid_factors_region_idx ON grid_factors (region, valid_from);
