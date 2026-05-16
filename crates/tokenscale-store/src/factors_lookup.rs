//! Time-anchored factor lookup.
//!
//! Each event resolves its environmental factors against the row whose
//! `valid_from <= occurred_at` (and either `valid_to` is null or
//! `occurred_at < valid_to`). Per the Phase 2 kickoff:
//!
//! > Each event locks to the factor row whose `valid_from` is the latest
//! > date <= the event's `occurred_at`.
//!
//! Two single-row helpers live here:
//!
//! - [`lookup_environmental_factors`] — `(provider, model)` → `ModelFactors`
//! - [`lookup_grid_factors`] — `region` → `GridFactors`
//!
//! Both return `Ok(None)` when nothing matches the time anchor; callers
//! decide how to surface the gap (the dashboard, for instance, lists
//! models with missing factors in `modelsWithoutFactors`).
//!
//! For per-event compute across many rows, see the multi-row JOIN query
//! in [`crate::queries`] which uses correlated subqueries to attach the
//! authoritative factor row to each event in a single pass.
//!
//! Returned types are the canonical `tokenscale-core` shapes so the
//! result feeds directly into [`tokenscale_core::compute_impact`]. The
//! DB schema does not store `display_name` (it lives in the in-memory
//! TOML snapshot for UI rendering), so reconstructed instances carry an
//! empty `display_name` — `compute_impact` does not consult that field.

use tokenscale_core::{GridFactors, ModelFactors};

use crate::error::Result;
use crate::Database;

/// Look up the environmental factor row authoritative for an event of
/// the given `(provider, model)` at `as_of_date` (`YYYY-MM-DD`).
///
/// Returns `Ok(None)` when no matching row exists — either the model
/// is not in the factor file, or every row's `valid_from` is in the
/// future relative to `as_of_date`.
pub async fn lookup_environmental_factors(
    database: &Database,
    provider: &str,
    model: &str,
    as_of_date: &str,
) -> Result<Option<ModelFactors>> {
    let row: Option<EnvFactorRow> = sqlx::query_as(
        "SELECT
            valid_from, valid_to,
            wh_per_mtok_input, wh_per_mtok_output, wh_per_mtok_cache_read,
            wh_per_mtok_cache_write_5m, wh_per_mtok_cache_write_1h,
            source_doc, notes,
            uncertainty_range_pct, confidence
         FROM env_factors
         WHERE provider = ?
           AND model = ?
           AND valid_from <= ?
           AND (valid_to IS NULL OR ? < valid_to)
         ORDER BY valid_from DESC
         LIMIT 1",
    )
    .bind(provider)
    .bind(model)
    .bind(as_of_date)
    .bind(as_of_date)
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(EnvFactorRow::into_model_factors))
}

/// Look up the grid-factor row authoritative for the given region at
/// `as_of_date` (`YYYY-MM-DD`).
///
/// Returns `Ok(None)` when no matching row exists — either the region
/// is not in the factor file, or every row's `valid_from` is in the
/// future relative to `as_of_date`.
pub async fn lookup_grid_factors(
    database: &Database,
    region: &str,
    as_of_date: &str,
) -> Result<Option<GridFactors>> {
    let row: Option<GridFactorRow> = sqlx::query_as(
        "SELECT
            valid_from, valid_to,
            co2e_kg_per_kwh, water_l_per_kwh, pue,
            egrid_subregion, egrid_subregion_full_name,
            source_url, source_accessed_at,
            co2e_uncertainty_range_pct, water_uncertainty_range_pct,
            indirect_water_l_per_kwh, indirect_water_uncertainty_range_pct
         FROM grid_factors
         WHERE region = ?
           AND valid_from <= ?
           AND (valid_to IS NULL OR ? < valid_to)
         ORDER BY valid_from DESC
         LIMIT 1",
    )
    .bind(region)
    .bind(as_of_date)
    .bind(as_of_date)
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(GridFactorRow::into_grid_factors))
}

#[derive(sqlx::FromRow)]
struct EnvFactorRow {
    valid_from: String,
    #[sqlx(rename = "valid_to")]
    _valid_to: Option<String>,
    wh_per_mtok_input: Option<f64>,
    wh_per_mtok_output: Option<f64>,
    wh_per_mtok_cache_read: Option<f64>,
    wh_per_mtok_cache_write_5m: Option<f64>,
    wh_per_mtok_cache_write_1h: Option<f64>,
    source_doc: String,
    notes: Option<String>,
    uncertainty_range_pct: Option<i32>,
    confidence: Option<String>,
}

impl EnvFactorRow {
    fn into_model_factors(self) -> ModelFactors {
        ModelFactors {
            display_name: String::new(),
            released_at: None,
            valid_from: Some(self.valid_from),
            source_doc: Some(self.source_doc),
            wh_per_mtok_input: self.wh_per_mtok_input,
            wh_per_mtok_output: self.wh_per_mtok_output,
            wh_per_mtok_cache_read: self.wh_per_mtok_cache_read,
            wh_per_mtok_cache_write_5m: self.wh_per_mtok_cache_write_5m,
            wh_per_mtok_cache_write_1h: self.wh_per_mtok_cache_write_1h,
            confidence: self.confidence,
            uncertainty_range_pct: self.uncertainty_range_pct,
            notes: self.notes,
        }
    }
}

#[derive(sqlx::FromRow)]
struct GridFactorRow {
    valid_from: String,
    #[sqlx(rename = "valid_to")]
    _valid_to: Option<String>,
    co2e_kg_per_kwh: Option<f64>,
    water_l_per_kwh: Option<f64>,
    pue: Option<f64>,
    egrid_subregion: Option<String>,
    egrid_subregion_full_name: Option<String>,
    source_url: String,
    source_accessed_at: String,
    co2e_uncertainty_range_pct: Option<i32>,
    water_uncertainty_range_pct: Option<i32>,
    indirect_water_l_per_kwh: Option<f64>,
    indirect_water_uncertainty_range_pct: Option<i32>,
}

impl GridFactorRow {
    fn into_grid_factors(self) -> GridFactors {
        GridFactors {
            display_name: String::new(),
            valid_from: Some(self.valid_from),
            co2e_kg_per_kwh: self.co2e_kg_per_kwh,
            co2e_uncertainty_range_pct: self.co2e_uncertainty_range_pct,
            water_l_per_kwh: self.water_l_per_kwh,
            water_uncertainty_range_pct: self.water_uncertainty_range_pct,
            indirect_water_l_per_kwh: self.indirect_water_l_per_kwh,
            indirect_water_uncertainty_range_pct: self.indirect_water_uncertainty_range_pct,
            pue: self.pue,
            egrid_subregion: self.egrid_subregion,
            egrid_subregion_full_name: self.egrid_subregion_full_name,
            // Schema collapses the three TOML URL fields into one DB
            // column; the methodology page reads from the in-memory
            // snapshot for the per-pollutant breakdown.
            source_url_co2e: Some(self.source_url),
            source_url_water: None,
            source_url_indirect_water: None,
            source_url_pue: None,
            source_accessed_at: Some(self.source_accessed_at),
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenscale_core::EnvironmentalFactorsFile;

    use crate::sync_environmental_factors;

    /// Two valid_from rows for the same (provider, model) so we can
    /// verify the lookup picks the latest one <= the event's date.
    const HISTORY_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2026-01-01"
source_doc = "docs/sources.md#G.1"
wh_per_mtok_input = 0.5

[grid_factors."us-east-1"]
display_name = "AWS US East"
valid_from = "2026-01-01"
source_accessed_at = "2026-01-01"
co2e_kg_per_kwh = 0.30
water_l_per_kwh = 0.20
pue = 1.15
egrid_subregion = "SRVC"
egrid_subregion_full_name = "SERC Virginia/Carolina"
"#;

    #[tokio::test]
    async fn lookup_returns_none_when_pair_not_in_db() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(HISTORY_TOML).unwrap();
        sync_environmental_factors(&database, &factors).await.unwrap();

        let result =
            lookup_environmental_factors(&database, "anthropic", "claude-opus-4-7", "2026-04-29")
                .await
                .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lookup_returns_row_when_pair_present_and_date_is_after_valid_from() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(HISTORY_TOML).unwrap();
        sync_environmental_factors(&database, &factors).await.unwrap();

        let result = lookup_environmental_factors(
            &database,
            "anthropic",
            "claude-sonnet-4-6",
            "2026-04-29",
        )
        .await
        .unwrap();
        let model = result.expect("factor row should exist");
        assert_eq!(model.wh_per_mtok_input, Some(0.5));
        assert_eq!(model.valid_from.as_deref(), Some("2026-01-01"));
    }

    #[tokio::test]
    async fn lookup_returns_none_when_event_predates_valid_from() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(HISTORY_TOML).unwrap();
        sync_environmental_factors(&database, &factors).await.unwrap();

        let result = lookup_environmental_factors(
            &database,
            "anthropic",
            "claude-sonnet-4-6",
            "2025-12-31",
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lookup_grid_returns_row_with_egrid_subregion_populated() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(HISTORY_TOML).unwrap();
        sync_environmental_factors(&database, &factors).await.unwrap();

        let grid = lookup_grid_factors(&database, "us-east-1", "2026-04-29")
            .await
            .unwrap()
            .expect("us-east-1 should be in the file");
        assert_eq!(grid.co2e_kg_per_kwh, Some(0.30));
        assert_eq!(grid.water_l_per_kwh, Some(0.20));
        assert_eq!(grid.pue, Some(1.15));
        assert_eq!(grid.egrid_subregion.as_deref(), Some("SRVC"));
        assert_eq!(
            grid.egrid_subregion_full_name.as_deref(),
            Some("SERC Virginia/Carolina")
        );
    }

    #[tokio::test]
    async fn lookup_grid_returns_none_for_unknown_region() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(HISTORY_TOML).unwrap();
        sync_environmental_factors(&database, &factors).await.unwrap();

        let grid = lookup_grid_factors(&database, "eu-north-99", "2026-04-29")
            .await
            .unwrap();
        assert!(grid.is_none());
    }

    /// When two rows share `(provider, model)` but have different
    /// `valid_from` dates, the lookup must pick the latest row whose
    /// `valid_from <= as_of_date`. This is the time-anchoring behavior
    /// that makes Phase 2's per-event compute correct.
    #[tokio::test]
    async fn lookup_picks_latest_valid_from_le_as_of_date() {
        let database = Database::open_in_memory_for_tests().await.unwrap();

        // Insert two rows directly so we can verify time-anchoring
        // even though the file-driven sync only carries one row per
        // (provider, model) in v0.1. Phase 2's history-preserving
        // upsert (future) will produce real multi-row data.
        sqlx::query(
            "INSERT INTO env_factors (
                provider, model, valid_from, valid_to,
                wh_per_mtok_input, source_doc
             ) VALUES
                ('anthropic', 'claude-sonnet-4-6', '2026-01-01', NULL, 0.5, 'doc-v1'),
                ('anthropic', 'claude-sonnet-4-6', '2026-04-15', NULL, 0.7, 'doc-v2')",
        )
        .execute(database.pool())
        .await
        .unwrap();

        // Event in March → first row (older valid_from is the one in effect).
        let march = lookup_environmental_factors(
            &database,
            "anthropic",
            "claude-sonnet-4-6",
            "2026-03-10",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(march.wh_per_mtok_input, Some(0.5));
        assert_eq!(march.source_doc.as_deref(), Some("doc-v1"));

        // Event in late April → second row.
        let april = lookup_environmental_factors(
            &database,
            "anthropic",
            "claude-sonnet-4-6",
            "2026-04-29",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(april.wh_per_mtok_input, Some(0.7));
        assert_eq!(april.source_doc.as_deref(), Some("doc-v2"));
    }
}
