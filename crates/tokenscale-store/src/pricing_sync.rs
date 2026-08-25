//! Sync `pricing.toml` content into the `pricing` table on startup.
//!
//! Mirrors `factors_sync` exactly — same replace-on-startup strategy,
//! same transactional shape, same idempotency guarantee. Per the v0.1.13
//! roadmap (`docs/roadmap-cost-time-anchoring.md` D7), this is the
//! recovery path for any wrong launch date: edit `pricing.toml`,
//! restart, sync rewrites the table.
//!
//! Phase B (v0.1.13): the table is populated for the first time. The
//! `pricing` schema itself was provisioned in the v0.1.0 initial
//! migration (lines 87-104 of `20260428000001_initial.sql`) but never
//! filled — runtime cost lookups went through the in-memory
//! `PricingFile` snapshot. Phase C (next) moves per-event cost
//! computation into the SQL aggregate path, which is the consumer of
//! this synced table.

use tokenscale_core::PricingFile;
use tracing::{debug, info};

use crate::error::Result;
use crate::Database;

/// Result of a pricing sync — counts so the CLI can log a useful summary.
/// Distinct from `FactorsSyncSummary` so callers can tell them apart in
/// startup logs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PricingSyncSummary {
    /// Number of `(provider, model, valid_from)` rows inserted. A model
    /// with two historical valid_from rows counts twice — the table is
    /// the multi-row representation, even if the in-memory snapshot
    /// originated from single-table TOML form.
    pub pricing_rows: usize,
    /// Number of distinct `(provider, model)` keys covered. Differs from
    /// `pricing_rows` once any model has more than one row.
    pub distinct_models: usize,
    /// D1 (v0.1.20): number of `(provider, raw -> canonical)` alias rows
    /// synced into `model_aliases`.
    pub alias_rows: usize,
}

/// Replace the contents of the `pricing` table with what the in-memory
/// pricing file says.
///
/// Idempotent across repeated startups. Atomic via a transaction —
/// either every row from `pricing.toml` lands, or none do, so a sync
/// failure mid-write cannot leave the table partially populated.
pub async fn sync_pricing(
    database: &Database,
    pricing_file: &PricingFile,
) -> Result<PricingSyncSummary> {
    let mut summary = PricingSyncSummary::default();
    let mut transaction = database.pool().begin().await?;

    // Full replacement — pricing.toml is canonical, as in factors_sync.
    sqlx::query("DELETE FROM pricing")
        .execute(&mut *transaction)
        .await?;

    for (provider_id, provider) in &pricing_file.providers {
        for (model_id, rows) in &provider.models {
            if rows.is_empty() {
                continue;
            }
            summary.distinct_models += 1;
            for model in rows {
                sqlx::query(
                    "INSERT INTO pricing (
                        provider, model, valid_from, valid_to,
                        input_usd_per_mtok, output_usd_per_mtok, cache_read_usd_per_mtok,
                        cache_write_5m_multiplier, cache_write_1h_multiplier,
                        source_url, source_accessed_at, notes
                     ) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(provider_id)
                .bind(model_id)
                .bind(&model.valid_from)
                .bind(model.input_usd_per_mtok)
                .bind(model.output_usd_per_mtok)
                .bind(model.cache_read_usd_per_mtok)
                .bind(model.cache_write_5m_multiplier)
                .bind(model.cache_write_1h_multiplier)
                .bind(&model.source_url)
                .bind(&model.source_accessed_at)
                .bind(model.notes.as_deref())
                .execute(&mut *transaction)
                .await?;
                summary.pricing_rows += 1;
            }
        }
    }

    // D1: alias table, same replace-on-startup posture. Synced inside the
    // same transaction so pricing rows and their alias map can never be
    // observed out of step.
    sqlx::query("DELETE FROM model_aliases")
        .execute(&mut *transaction)
        .await?;
    for (provider_id, provider) in &pricing_file.providers {
        for (raw, canonical) in &provider.aliases {
            sqlx::query(
                "INSERT INTO model_aliases (provider, raw, canonical) VALUES (?, ?, ?)",
            )
            .bind(provider_id)
            .bind(raw)
            .bind(canonical)
            .execute(&mut *transaction)
            .await?;
            summary.alias_rows += 1;
        }
    }

    transaction.commit().await?;
    info!(?summary, "synced pricing from pricing.toml");
    debug!("pricing table now reflects the file's contents");
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multi-row TOML fixture for the sync tests. Two rows for Opus 4.7
    /// (older valid_from = 2026-01-15, newer valid_from = 2027-06-01).
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

[providers.anthropic.models."claude-sonnet-4-6"]
display_name              = "Claude Sonnet 4.6"
valid_from                = "2025-09-01"
input_usd_per_mtok        = 3.00
output_usd_per_mtok       = 15.00
cache_read_usd_per_mtok   = 0.30
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2026-05-18"
"#;

    #[tokio::test]
    async fn sync_inserts_all_rows() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();

        let summary = sync_pricing(&database, &pricing).await.unwrap();
        // Two Opus rows + one Sonnet row = 3 total.
        assert_eq!(summary.pricing_rows, 3);
        // Two distinct (provider, model) keys.
        assert_eq!(summary.distinct_models, 2);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pricing")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 3);
    }

    /// Quick-pass hardening (2026-06-16): two rows sharing the same
    /// (provider, model, valid_from) would each satisfy the aggregate
    /// join's `valid_from = (SELECT MAX(valid_from) ...)` equality and
    /// silently double-count every cost/energy SUM. The UNIQUE index
    /// added in migration 20260616000001 makes such a TOML fail the
    /// sync loudly (transaction rollback, serve refuses to start)
    /// instead. This test pins the fail-loud behavior.
    #[tokio::test]
    async fn sync_rejects_duplicate_model_valid_from_rows() {
        const DUPLICATE_VALID_FROM_TOML: &str = r#"
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
valid_from                = "2026-01-15"
input_usd_per_mtok        = 6.00
output_usd_per_mtok       = 30.00
cache_read_usd_per_mtok   = 0.60
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://platform.claude.com/docs/en/about-claude/pricing"
source_accessed_at        = "2026-05-18"
"#;
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(DUPLICATE_VALID_FROM_TOML).unwrap();

        let result = sync_pricing(&database, &pricing).await;
        assert!(
            result.is_err(),
            "duplicate (provider, model, valid_from) must fail the sync loudly, not double-count"
        );

        // The transaction rolled back: nothing landed.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pricing")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 0, "failed sync must leave the table untouched");
    }

    #[tokio::test]
    async fn sync_is_idempotent() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();

        for _ in 0..3 {
            sync_pricing(&database, &pricing).await.unwrap();
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pricing")
            .fetch_one(database.pool())
            .await
            .unwrap();
        // Three syncs of the same file still produce three rows, not nine.
        assert_eq!(count.0, 3);
    }

    /// Verifies the v0.1.12-shape-DB-upgrade path (D7 bullet a): sync
    /// runs cleanly against a DB whose `pricing` table is empty but
    /// schema-present (the pre-v0.1.13 state — table provisioned by the
    /// initial migration but never populated). `Database::open_in_memory_for_tests`
    /// applies every migration in order, including the v0.1.13 notes
    /// column addition — so the test's "fresh DB" path is structurally
    /// identical to "v0.1.12 DB after the v0.1.13 notes-column migration
    /// runs."
    #[tokio::test]
    async fn sync_works_on_pre_existing_empty_pricing_table() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        // Confirm the table exists and is empty (this is exactly the
        // v0.1.0–v0.1.12 state — table provisioned, never written to).
        let pre: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pricing")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(pre.0, 0);

        let pricing = PricingFile::parse(MULTI_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();

        let post: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pricing")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(post.0, 3);
    }

    /// Single-row TOML form (v0.1.12-compatible) syncs to one row per
    /// model. Confirms back-compat at the sync layer too.
    #[tokio::test]
    async fn single_row_toml_syncs_one_row_per_model() {
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
        let summary = sync_pricing(&database, &pricing).await.unwrap();
        assert_eq!(summary.pricing_rows, 1);
        assert_eq!(summary.distinct_models, 1);
    }
}

#[cfg(test)]
mod alias_sync_tests {
    use super::*;
    use chrono::TimeZone;
    use crate::impact_query::ImpactQueryFactors;
    use crate::queries::{Granularity, ALL_PROVIDERS};
    use crate::{aggregate_impact_by_bucket, insert_events, sync_environmental_factors};
    use tokenscale_core::{EnvironmentalFactorsFile, Event};

    /// Pricing keyed on the CANONICAL dated Haiku ID plus an alias for the
    /// undated form. The event below emits the undated alias.
    const ALIASED_PRICING: &str = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[providers.anthropic.aliases]
"claude-haiku-4-5" = "claude-haiku-4-5-20251001"
[[providers.anthropic.models."claude-haiku-4-5-20251001"]]
display_name = "Claude Haiku 4.5"
valid_from = "2025-10-01"
input_usd_per_mtok = 1.00
output_usd_per_mtok = 5.00
cache_read_usd_per_mtok = 0.10
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "x"
source_accessed_at = "2026-08-02"
"#;

    /// Factors also keyed on the canonical dated ID only.
    const ALIASED_FACTORS: &str = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[providers.anthropic.models."claude-haiku-4-5-20251001"]
display_name = "Claude Haiku 4.5"
valid_from = "2025-10-01"
source_doc = "test"
wh_per_mtok_input = 70
wh_per_mtok_output = 330
wh_per_mtok_cache_read = 7
wh_per_mtok_cache_write_5m = 88
wh_per_mtok_cache_write_1h = 140
uncertainty_range_pct = 30
confidence = "secondary"
[[grid_factors."us-east-1"]]
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
    async fn sync_lands_alias_rows() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(ALIASED_PRICING).unwrap();
        let summary = sync_pricing(&database, &pricing).await.unwrap();
        assert_eq!(summary.alias_rows, 1);
        let row: (String, String) = sqlx::query_as(
            "SELECT raw, canonical FROM model_aliases WHERE provider = 'anthropic'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(row, ("claude-haiku-4-5".to_owned(), "claude-haiku-4-5-20251001".to_owned()));
        // Re-sync replaces, never duplicates.
        sync_pricing(&database, &pricing).await.unwrap();
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM model_aliases")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(n.0, 1);
    }

    /// D1 end-to-end: an event emitting an ALIAS model ID resolves to the
    /// canonical pricing AND factor rows through the aggregate join. This
    /// is the dated-ID-to-canonical regression the assessment flagged as
    /// missing; without the alias join it renders unpriced and unfactored.
    #[tokio::test]
    async fn event_emitting_alias_resolves_to_canonical_pricing_and_factors() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        sync_pricing(&database, &PricingFile::parse(ALIASED_PRICING).unwrap())
            .await
            .unwrap();
        sync_environmental_factors(
            &database,
            &EnvironmentalFactorsFile::parse(ALIASED_FACTORS).unwrap(),
        )
        .await
        .unwrap();

        let event = Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: "claude-haiku-4-5".to_owned(), // the alias, not the canonical
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some("req-alias".to_owned()),
            content_hash: None,
            session_id: Some("sess-alias".to_owned()),
            project_id: Some("/repo".to_owned()),
            workspace_id: None,
            api_key_id: None,
            uuid: None,
            parent_uuid: None,
            git_branch: None,
            raw: None,
        };
        insert_events(&database, std::slice::from_ref(&event)).await.unwrap();

        let factors = ImpactQueryFactors {
            region: "us-east-1",
            fallback_pue: 1.15,
            fallback_wue_l_per_kwh: Some(0.15),
        };
        let rows = aggregate_impact_by_bucket(
            &database, "2026-04-01", "2026-04-30", ALL_PROVIDERS, &[], Granularity::Day, &factors,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // Display key stays RAW (the alias the source emitted).
        assert_eq!(row.model, "claude-haiku-4-5");
        // Resolution went through the canonical row: priced and factored.
        assert_eq!(row.events_missing_pricing, 0, "alias must resolve to the canonical pricing row");
        assert_eq!(row.events_missing_env_factor, 0, "alias must resolve to the canonical factor row");
        // 1M input at $1 + 100K output at $5 = $1.00 + $0.50
        let cost = row.cost_usd_total.expect("priced");
        assert!((cost - 1.5).abs() < 1e-9, "cost_usd_total={cost}");
    }
}
