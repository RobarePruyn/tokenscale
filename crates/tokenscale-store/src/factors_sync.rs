//! Sync `environmental-factors.toml` content into the `env_factors` and
//! `grid_factors` tables on startup, per the kickoff prompt:
//!
//! > On every startup, after loading and validating the file, sync its
//! > contents into the env_factors and grid_factors tables (versioned by
//! > `valid_from`).
//!
//! Phase 1 strategy: **full replacement**. The TOML file is canonical;
//! every startup deletes the existing rows and re-inserts from the file.
//! This is simple, idempotent, and consistent with the in-memory
//! snapshot the handlers use. The cost is loss of history when a model
//! disappears from the file — fine for Phase 1, where impact compute
//! isn't yet wired in.
//!
//! Phase 2 will refine to incremental sync that preserves history when
//! `valid_from` changes — needed for per-event impact computation that
//! resolves to the factor row authoritative at the event's timestamp.
//! Once that lands, this module's `sync_environmental_factors` is the
//! one place to swap in upsert semantics.

use tokenscale_core::EnvironmentalFactorsFile;
use tracing::{debug, info};

use crate::error::Result;
use crate::Database;

/// Result of a sync — counts so the CLI can log a useful summary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FactorsSyncSummary {
    /// Number of `(provider, model, valid_from)` rows inserted or replaced.
    pub model_factor_rows: usize,
    /// Number of `(region, valid_from)` rows inserted or replaced.
    pub grid_factor_rows: usize,
    /// Number of model entries skipped because every numeric field was
    /// `None` (placeholder rows). Phase 2 will count these as "factor
    /// data unavailable for this model" in the dashboard.
    pub model_factor_rows_all_null: usize,
}

/// Replace the contents of `env_factors` and `grid_factors` with what
/// the in-memory factor file says.
pub async fn sync_environmental_factors(
    database: &Database,
    factors_file: &EnvironmentalFactorsFile,
) -> Result<FactorsSyncSummary> {
    let mut summary = FactorsSyncSummary::default();
    let mut transaction = database.pool().begin().await?;

    // Full replacement — the file is canonical in Phase 1.
    sqlx::query("DELETE FROM env_factors")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM grid_factors")
        .execute(&mut *transaction)
        .await?;

    for (provider_id, provider) in &factors_file.providers {
        for (model_id, model) in &provider.models {
            let valid_from = model.valid_from.as_deref().unwrap_or("1970-01-01");
            sqlx::query(
                "INSERT INTO env_factors (
                    provider, model, valid_from, valid_to,
                    wh_per_mtok_input, wh_per_mtok_output, wh_per_mtok_cache_read,
                    wh_per_mtok_cache_write_5m, wh_per_mtok_cache_write_1h,
                    source_doc, notes
                 ) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(provider_id)
            .bind(model_id)
            .bind(valid_from)
            .bind(model.wh_per_mtok_input)
            .bind(model.wh_per_mtok_output)
            .bind(model.wh_per_mtok_cache_read)
            .bind(model.wh_per_mtok_cache_write_5m)
            .bind(model.wh_per_mtok_cache_write_1h)
            .bind(model.source_doc.as_deref().unwrap_or(""))
            .bind(model.notes.as_deref())
            .execute(&mut *transaction)
            .await?;

            summary.model_factor_rows += 1;
            if model.wh_per_mtok_input.is_none()
                && model.wh_per_mtok_output.is_none()
                && model.wh_per_mtok_cache_read.is_none()
                && model.wh_per_mtok_cache_write_5m.is_none()
                && model.wh_per_mtok_cache_write_1h.is_none()
            {
                summary.model_factor_rows_all_null += 1;
            }
        }
    }

    for (region_id, grid) in &factors_file.grid_factors {
        let valid_from = grid.valid_from.as_deref().unwrap_or("1970-01-01");
        // `grid_factors` requires NOT NULL on `co2e_kg_per_kwh` and `pue`.
        // For placeholder rows where these are absent in the TOML, fall
        // back to the file's `defaults.fallback_pue` (or 1.0) and a 0.0
        // sentinel for co2e. The Phase 2 compute path consults the
        // in-memory snapshot's `Option<f64>` to decide "data available?";
        // it doesn't see these sentinels.
        let pue = grid
            .pue
            .or(factors_file.defaults.fallback_pue)
            .unwrap_or(1.0);
        let co2e = grid.co2e_kg_per_kwh.unwrap_or(0.0);

        sqlx::query(
            "INSERT INTO grid_factors (
                region, valid_from, valid_to, co2e_kg_per_kwh, water_l_per_kwh,
                pue, source_url, source_accessed_at
             ) VALUES (?, ?, NULL, ?, ?, ?, ?, ?)",
        )
        .bind(region_id)
        .bind(valid_from)
        .bind(co2e)
        .bind(grid.water_l_per_kwh)
        .bind(pue)
        .bind(grid.source_url_co2e.as_deref().unwrap_or(""))
        .bind(grid.source_accessed_at.as_deref().unwrap_or(valid_from))
        .execute(&mut *transaction)
        .await?;
        summary.grid_factor_rows += 1;
    }

    transaction.commit().await?;
    info!(
        ?summary,
        "synced environmental factors from environmental-factors.toml"
    );
    debug!("env_factors / grid_factors tables now reflect the file's contents");
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACEHOLDER_TOML: &str = r#"
schema_version = 1
file_status = "placeholder"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-28"
source_doc = "docs/sources.md#G.1"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2026-04-28"
source_doc = "docs/sources.md#G.1"
wh_per_mtok_input = 0.5

[grid_factors."us-east-1"]
display_name = "AWS US East"
valid_from = "2026-04-28"
source_accessed_at = "2026-04-28"

[defaults]
fallback_pue = 1.15
"#;

    #[tokio::test]
    async fn sync_writes_one_row_per_model_and_region() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(PLACEHOLDER_TOML).unwrap();

        let summary = sync_environmental_factors(&database, &factors)
            .await
            .unwrap();
        assert_eq!(summary.model_factor_rows, 2);
        assert_eq!(summary.grid_factor_rows, 1);
        // Opus has all-null Wh values → counted; Sonnet has one populated.
        assert_eq!(summary.model_factor_rows_all_null, 1);

        // Verify rows landed.
        let env_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM env_factors")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(env_count.0, 2);
        let grid_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM grid_factors")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(grid_count.0, 1);
    }

    #[tokio::test]
    async fn sync_is_idempotent_on_repeat() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors = EnvironmentalFactorsFile::parse(PLACEHOLDER_TOML).unwrap();

        sync_environmental_factors(&database, &factors)
            .await
            .unwrap();
        sync_environmental_factors(&database, &factors)
            .await
            .unwrap();
        sync_environmental_factors(&database, &factors)
            .await
            .unwrap();

        let env_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM env_factors")
            .fetch_one(database.pool())
            .await
            .unwrap();
        // Three syncs of the same file → still two model rows.
        assert_eq!(env_count.0, 2);
    }
}
