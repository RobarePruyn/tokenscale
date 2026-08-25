//! Time-anchored pricing lookup — DB side.
//!
//! Mirrors `factors_lookup::lookup_environmental_factors` exactly: for an
//! event at `as_of_date`, return the `pricing` row whose `valid_from` is
//! the latest date <= `as_of_date` for the `(provider, model)` pair.
//!
//! Phase B (v0.1.13) ships this as the single-row helper, paralleling
//! `lookup_environmental_factors`. Phase C will run the same time-anchor
//! pattern as a correlated subquery inside `aggregate_impact_by_bucket`
//! for the bulk per-event aggregation; the helper here exists for the
//! single-row use case (e.g. the v0.1.12 `Cost (USD)` view's
//! `pricingByModel` dict, until Phase C drops it).
//!
//! Returns `Ok(None)` when nothing matches — same semantics as the env
//! side, callers decide how to surface the gap.

use tokenscale_core::ModelPricing;

use crate::error::Result;
use crate::Database;

/// Look up the pricing row authoritative for an event of the given
/// `(provider, model)` at `as_of_date` (`YYYY-MM-DD`).
///
/// Returns `Ok(None)` when no matching row exists — either the pair is
/// not in the pricing file, or every row's `valid_from` is in the future
/// relative to `as_of_date` (pre-launch event per D4).
pub async fn lookup_pricing(
    database: &Database,
    provider: &str,
    model: &str,
    as_of_date: &str,
) -> Result<Option<ModelPricing>> {
    let row: Option<PricingRow> = sqlx::query_as(
        "SELECT
            display_name_placeholder,
            valid_from, valid_to,
            input_usd_per_mtok, output_usd_per_mtok, cache_read_usd_per_mtok,
            cache_write_5m_multiplier, cache_write_1h_multiplier,
            source_url, source_accessed_at,
            notes
         FROM (
            SELECT
                '' AS display_name_placeholder,
                valid_from, valid_to,
                input_usd_per_mtok, output_usd_per_mtok, cache_read_usd_per_mtok,
                cache_write_5m_multiplier, cache_write_1h_multiplier,
                source_url, source_accessed_at,
                notes
             FROM pricing
             WHERE provider = ?
               AND model = COALESCE((SELECT canonical FROM model_aliases WHERE provider = ? AND raw = ?), ?)
               AND valid_from <= ?
               AND (valid_to IS NULL OR ? < valid_to)
             ORDER BY valid_from DESC
             LIMIT 1
         )",
    )
    .bind(provider)
    .bind(provider)
    .bind(model)
    .bind(model)
    .bind(as_of_date)
    .bind(as_of_date)
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(PricingRow::into_model_pricing))
}

#[derive(sqlx::FromRow)]
struct PricingRow {
    /// The DB schema doesn't store `display_name` — that lives only in
    /// the in-memory TOML snapshot for UI rendering. Reconstructed rows
    /// carry an empty `display_name`, matching the env-side convention
    /// in [`factors_lookup`].
    #[allow(dead_code)]
    display_name_placeholder: String,
    valid_from: String,
    #[sqlx(rename = "valid_to")]
    _valid_to: Option<String>,
    input_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
    cache_read_usd_per_mtok: f64,
    cache_write_5m_multiplier: f64,
    cache_write_1h_multiplier: f64,
    source_url: String,
    source_accessed_at: String,
    notes: Option<String>,
}

impl PricingRow {
    fn into_model_pricing(self) -> ModelPricing {
        ModelPricing {
            display_name: String::new(),
            valid_from: self.valid_from,
            input_usd_per_mtok: self.input_usd_per_mtok,
            output_usd_per_mtok: self.output_usd_per_mtok,
            cache_read_usd_per_mtok: self.cache_read_usd_per_mtok,
            cache_write_5m_multiplier: self.cache_write_5m_multiplier,
            cache_write_1h_multiplier: self.cache_write_1h_multiplier,
            source_url: self.source_url,
            source_accessed_at: self.source_accessed_at,
            // The DB pricing table doesn't store launch_date_source —
            // it's a TOML-only provenance field (same as display_name).
            // Rows reconstructed from the DB get None here.
            launch_date_source: None,
            notes: self.notes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenscale_core::PricingFile;

    use crate::sync_pricing;

    /// Same two-row fixture as the sync tests use, so the regression
    /// against v0.1.12 single-row behavior is consistent.
    const MULTI_ROW_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name              = "Claude Opus 4.7"
valid_from                = "2026-01-15"
input_usd_per_mtok        = 5.00
output_usd_per_mtok       = 25.00
cache_read_usd_per_mtok   = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2026-05-18"

[[providers.anthropic.models."claude-opus-4-7"]]
display_name              = "Claude Opus 4.7"
valid_from                = "2027-06-01"
input_usd_per_mtok        = 7.00
output_usd_per_mtok       = 35.00
cache_read_usd_per_mtok   = 0.70
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2027-06-01"
"#;

    #[tokio::test]
    async fn lookup_returns_none_when_pair_not_in_db() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        let result = lookup_pricing(&database, "anthropic", "claude-haiku-99", "2027-01-01")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lookup_picks_earlier_row_for_early_date() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        // 2026-04-21 is between the two valid_from dates — should hit
        // the first row (valid_from 2026-01-15).
        let result = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2026-04-21")
            .await
            .unwrap()
            .expect("row must match");
        assert!((result.input_usd_per_mtok - 5.00).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn lookup_picks_later_row_for_late_date() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        let result = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2027-07-15")
            .await
            .unwrap()
            .expect("row must match");
        assert!((result.input_usd_per_mtok - 7.00).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn lookup_returns_none_when_event_predates_valid_from() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        // 2025-12-31 is before the first row's valid_from (2026-01-15).
        let result = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2025-12-31")
            .await
            .unwrap();
        assert!(result.is_none(), "pre-launch event must return None");
    }

    /// Boundary check matching the core-side test in `pricing.rs`:
    /// `valid_from <= as_of_date`, so an event AT exactly the boundary
    /// gets the new row.
    #[tokio::test]
    async fn boundary_event_at_valid_from_gets_new_row() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        let on_boundary = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2027-06-01")
            .await
            .unwrap()
            .expect("boundary row must match");
        assert!((on_boundary.input_usd_per_mtok - 7.00).abs() < f64::EPSILON);

        let day_before = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2027-05-31")
            .await
            .unwrap()
            .expect("day-before row must match");
        assert!((day_before.input_usd_per_mtok - 5.00).abs() < f64::EPSILON);
    }

    /// Regression invariant: for the current single-row pricing.toml,
    /// the DB lookup produces the SAME ModelPricing as the in-memory
    /// PricingFile::lookup. v0.1.13 must not move numbers.
    #[tokio::test]
    async fn db_lookup_matches_in_memory_lookup_for_single_row_data() {
        let single_row = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-01-15"
input_usd_per_mtok = 5.00
output_usd_per_mtok = 25.00
cache_read_usd_per_mtok = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example"
source_accessed_at = "2026-05-18"
"#;
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(single_row).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        let in_memory = pricing
            .lookup("anthropic", "claude-opus-4-7", "2026-04-21")
            .expect("in-memory match");
        let db = lookup_pricing(&database, "anthropic", "claude-opus-4-7", "2026-04-21")
            .await
            .unwrap()
            .expect("DB match");

        // All rate fields must be identical.
        assert_eq!(in_memory.input_usd_per_mtok, db.input_usd_per_mtok);
        assert_eq!(in_memory.output_usd_per_mtok, db.output_usd_per_mtok);
        assert_eq!(in_memory.cache_read_usd_per_mtok, db.cache_read_usd_per_mtok);
        assert_eq!(
            in_memory.cache_write_5m_multiplier,
            db.cache_write_5m_multiplier
        );
        assert_eq!(
            in_memory.cache_write_1h_multiplier,
            db.cache_write_1h_multiplier
        );
        assert_eq!(in_memory.valid_from, db.valid_from);
    }
}
