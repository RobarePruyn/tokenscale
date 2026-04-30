//! Per-event time-anchored impact aggregation.
//!
//! The dashboard's daily endpoint needs energy / facility energy / CO₂e /
//! water for every (bucket, model) cell. The Phase 2 kickoff committed
//! to **per-event** factor resolution from day one (rather than the
//! simpler "resolve once per bucket"), so each event locks to whichever
//! `env_factors` and `grid_factors` rows are authoritative at its own
//! `occurred_at`. Within a bucket, that means events on either side of a
//! `valid_from` boundary use different factor rows — and the bucket
//! total is the honest sum, not an averaged-out approximation.
//!
//! Implementation: one SQL query joins each event to its authoritative
//! factor rows via a correlated subquery on `valid_from`, computes the
//! per-event impact inline (Wh / facility-Wh / CO₂e / water), and sums
//! at `(bucket, provider, model)` granularity. Rust is left to do
//! formatting and provenance plumbing — no second pass.
//!
//! This mirrors the math in [`tokenscale_core::compute_impact`] but
//! lifted into SQL for aggregate efficiency. The two paths are
//! intentionally redundant: `compute_impact` is the canonical pure-Rust
//! implementation (used for unit tests and one-off computations);
//! `aggregate_impact_by_bucket` is the bulk path. They must agree on
//! identical inputs — covered by an integration test in the server
//! crate.

use serde::Serialize;
use sqlx::QueryBuilder;

use crate::error::Result;
use crate::queries::{Granularity, ALL_PROVIDERS};
use crate::Database;

/// One row per `(bucket, provider, model)` with token sums, factor-anchored
/// impact totals, and provenance/uncertainty aggregates. All fields are
/// the dashboard's payload, post-aggregation. Null impact fields mean
/// "factor unavailable for one or more events in this bucket" — the
/// caller decides how to render.
#[derive(Debug, Clone, Serialize)]
pub struct ImpactByBucketRow {
    /// `YYYY-MM-DD`, the start date of the bucket.
    pub bucket: String,
    pub provider: String,
    pub model: String,

    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,

    /// Pre-PUE per-token energy summed over events using their own
    /// per-event-resolved `wh_per_mtok_*`. Events with a null factor
    /// for a token type contribute zero — same convention as
    /// `compute_impact`.
    pub energy_wh: f64,

    /// Energy after PUE multiplier. Per event, `COALESCE(gf.pue,
    /// ?fallback_pue)` — so events whose region has no published PUE
    /// fall back to the configured default.
    pub facility_wh: f64,

    /// `None` when *no* event in the bucket had a non-null
    /// `co2e_kg_per_kwh`. Otherwise the sum, in grams.
    pub co2e_g: Option<f64>,

    /// `None` when no event had a non-null water factor and the
    /// fallback WUE was also unset. Otherwise the sum, in liters.
    pub water_l: Option<f64>,

    /// Maximum per-event uncertainty within the bucket. The dashboard
    /// renders this as the bucket's `± X%` band — a conservative
    /// choice (the widest model's band wins).
    pub max_uncertainty_pct: i32,

    /// Number of events whose env_factor row was missing entirely
    /// (model isn't in `env_factors` at the event's `occurred_at`).
    /// The dashboard's "models without factors" footer counts these.
    pub events_missing_env_factor: i64,

    /// Number of events that fell back to `defaults.fallback_pue`
    /// because their grid row had `pue IS NULL`. Used to surface the
    /// "X% of events used fallback PUE" provenance flag.
    pub events_using_fallback_pue: i64,

    /// Number of events that fell back to
    /// `defaults.fallback_wue_l_per_kwh` because their grid row had
    /// `water_l_per_kwh IS NULL`.
    pub events_using_fallback_wue: i64,

    pub events_count: i64,
}

/// Inputs the SQL needs from the in-memory factor snapshot — the
/// configured region and the two fallback values from `[defaults]`.
/// Pulled out as a struct so the function signature stays readable.
#[derive(Debug, Clone)]
pub struct ImpactQueryFactors<'a> {
    pub region: &'a str,
    pub fallback_pue: f64,
    /// `None` means "no WUE fallback configured" — events with a null
    /// grid water factor will not contribute to `water_l`.
    pub fallback_wue_l_per_kwh: Option<f64>,
}

/// Aggregate impact by bucket, with per-event time-anchored factor
/// resolution. Mirrors [`crate::queries::daily_usage_breakdown`]'s
/// filtering surface (date window, provider, project list, granularity)
/// so the daily endpoint can present a unified row per (bucket, model)
/// regardless of which view mode the dashboard is rendering.
//
// The body is essentially one big QueryBuilder; splitting it out would
// just shuffle SQL fragments between helpers without making the math
// any clearer.
#[allow(clippy::too_many_lines)]
pub async fn aggregate_impact_by_bucket(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
    project_filter: &[String],
    granularity: Granularity,
    factors: &ImpactQueryFactors<'_>,
) -> Result<Vec<ImpactByBucketRow>> {
    let bucket_expr = granularity.bucket_expression();

    let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new("SELECT ");
    builder.push(bucket_expr);
    builder.push(" AS bucket, sources.provider AS provider, events.model AS model,
        COALESCE(SUM(events.input_tokens), 0)            AS input_tokens,
        COALESCE(SUM(events.output_tokens), 0)           AS output_tokens,
        COALESCE(SUM(events.cache_read_tokens), 0)       AS cache_read_tokens,
        COALESCE(SUM(events.cache_write_5m_tokens), 0)   AS cache_write_5m_tokens,
        COALESCE(SUM(events.cache_write_1h_tokens), 0)   AS cache_write_1h_tokens,
        -- Per-event Wh (pre-PUE), summed:
        COALESCE(SUM(
            (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
           + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
           + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
           + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
           + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
            ) / 1000000.0
        ), 0)                                            AS energy_wh,
        -- Facility Wh (energy × PUE; PUE falls back to ?fallback_pue per event):
        COALESCE(SUM(
            (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
           + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
           + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
           + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
           + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
            ) / 1000000.0
            * COALESCE(gf.pue, ");
    builder.push_bind(factors.fallback_pue);
    builder.push(")
        ), 0)                                            AS facility_wh,
        -- CO₂e in grams = facility_wh × kg/kWh. Null when grid co2e is null
        -- — `SUM` over `NULL * x` propagates NULLs to zero contribution; the
        -- outer `SUM` is non-null iff at least one event had a non-null
        -- grid co2e. We then promote the all-zero-with-no-non-null case
        -- back to NULL via a NULLIF on `events_with_co2e`.
        SUM(
            (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
           + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
           + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
           + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
           + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
            ) / 1000000.0
            * COALESCE(gf.pue, ");
    builder.push_bind(factors.fallback_pue);
    builder.push(")
            * gf.co2e_kg_per_kwh
        )                                                AS co2e_g_raw,
        SUM(CASE WHEN gf.co2e_kg_per_kwh IS NULL THEN 0 ELSE 1 END) AS events_with_co2e,
        -- Water in liters = (facility_wh / 1000) × L/kWh. Falls back when
        -- the grid is null, IF the application supplied a fallback WUE.
        SUM(
            (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
           + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
           + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
           + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
           + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
            ) / 1000000.0
            * COALESCE(gf.pue, ");
    builder.push_bind(factors.fallback_pue);
    builder.push(")
            / 1000.0
            * COALESCE(gf.water_l_per_kwh, ");
    // NULL bind preserves the all-or-nothing semantics when no fallback
    // is configured: COALESCE(gf.water, NULL) = gf.water, so events with
    // a null water factor contribute NULL → the SUM treats them as zero
    // contribution. We then promote zero-with-no-non-null to NULL.
    match factors.fallback_wue_l_per_kwh {
        Some(v) => {
            builder.push_bind(v);
        }
        None => {
            builder.push("NULL");
        }
    }
    builder.push(")
        )                                                AS water_l_raw,
        SUM(CASE
                WHEN gf.water_l_per_kwh IS NOT NULL THEN 1
                WHEN ");
    match factors.fallback_wue_l_per_kwh {
        Some(_) => {
            builder.push("1 = 1");
        }
        None => {
            builder.push("1 = 0");
        }
    }
    builder.push(" THEN 1
                ELSE 0
            END)                                         AS events_with_water,
        COALESCE(MAX(ef.uncertainty_range_pct), 0)       AS max_uncertainty_pct,
        SUM(CASE WHEN ef.id IS NULL THEN 1 ELSE 0 END)   AS events_missing_env_factor,
        SUM(CASE WHEN gf.pue IS NULL THEN 1 ELSE 0 END)  AS events_using_fallback_pue,
        SUM(CASE WHEN gf.water_l_per_kwh IS NULL THEN 1 ELSE 0 END) AS events_using_fallback_wue,
        COUNT(*)                                         AS events_count
       FROM events
       JOIN sources ON sources.kind = events.source
       LEFT JOIN env_factors ef
              ON ef.provider = sources.provider
             AND ef.model = events.model
             AND ef.valid_from = (
                 SELECT MAX(valid_from) FROM env_factors
                  WHERE provider = sources.provider
                    AND model = events.model
                    AND valid_from <= date(events.occurred_at)
             )
       LEFT JOIN grid_factors gf
              ON gf.region = ");
    builder.push_bind(factors.region.to_owned());
    builder.push("
             AND gf.valid_from = (
                 SELECT MAX(valid_from) FROM grid_factors
                  WHERE region = ");
    builder.push_bind(factors.region.to_owned());
    builder.push("
                    AND valid_from <= date(events.occurred_at)
             )
      WHERE date(events.occurred_at) BETWEEN ");
    builder.push_bind(from_date.to_owned());
    builder.push(" AND ");
    builder.push_bind(to_date.to_owned());
    builder.push(" AND (");
    builder.push_bind(provider_filter.to_owned());
    builder.push(" = ");
    builder.push_bind(ALL_PROVIDERS.to_owned());
    builder.push(" OR sources.provider = ");
    builder.push_bind(provider_filter.to_owned());
    builder.push(")");

    if !project_filter.is_empty() {
        builder.push(" AND events.project_id IN (");
        let mut separated = builder.separated(", ");
        for project in project_filter {
            separated.push_bind(project.clone());
        }
        separated.push_unseparated(")");
    }

    builder.push(
        " GROUP BY bucket, sources.provider, events.model
          ORDER BY bucket ASC, events.model ASC",
    );

    let raw_rows: Vec<RawImpactByBucketRow> =
        builder.build_query_as().fetch_all(database.pool()).await?;

    Ok(raw_rows.into_iter().map(RawImpactByBucketRow::cook).collect())
}

#[derive(sqlx::FromRow)]
struct RawImpactByBucketRow {
    bucket: String,
    provider: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_5m_tokens: i64,
    cache_write_1h_tokens: i64,
    energy_wh: f64,
    facility_wh: f64,
    co2e_g_raw: Option<f64>,
    events_with_co2e: i64,
    water_l_raw: Option<f64>,
    events_with_water: i64,
    max_uncertainty_pct: i32,
    events_missing_env_factor: i64,
    events_using_fallback_pue: i64,
    events_using_fallback_wue: i64,
    events_count: i64,
}

impl RawImpactByBucketRow {
    fn cook(self) -> ImpactByBucketRow {
        // Promote "all events had null co2e" back to None so the
        // dashboard renders "—" rather than 0 g.
        let co2e_g = if self.events_with_co2e > 0 {
            self.co2e_g_raw
        } else {
            None
        };
        let water_l = if self.events_with_water > 0 {
            self.water_l_raw
        } else {
            None
        };

        ImpactByBucketRow {
            bucket: self.bucket,
            provider: self.provider,
            model: self.model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_5m_tokens: self.cache_write_5m_tokens,
            cache_write_1h_tokens: self.cache_write_1h_tokens,
            energy_wh: self.energy_wh,
            facility_wh: self.facility_wh,
            co2e_g,
            water_l,
            max_uncertainty_pct: self.max_uncertainty_pct,
            events_missing_env_factor: self.events_missing_env_factor,
            events_using_fallback_pue: self.events_using_fallback_pue,
            events_using_fallback_wue: self.events_using_fallback_wue,
            events_count: self.events_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tokenscale_core::{Event, EnvironmentalFactorsFile};

    use crate::insert_events;
    use crate::sync_environmental_factors;

    const PROD_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2026-01-01"
source_doc = "docs/sources.md#G.1"
wh_per_mtok_input = 0.5
wh_per_mtok_output = 2.0
wh_per_mtok_cache_read = 0.05
wh_per_mtok_cache_write_5m = 0.5
wh_per_mtok_cache_write_1h = 0.5
uncertainty_range_pct = 35
confidence = "secondary"

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

    fn event(model: &str, day: u32, input: u64, output: u64) -> Event {
        Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, day, 12, 0, 0).unwrap(),
            model: model.to_owned(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(format!("req-{model}-{day}-{input}-{output}")),
            content_hash: None,
            session_id: None,
            project_id: None,
            workspace_id: None,
            api_key_id: None,
            raw: None,
        }
    }

    fn factors() -> ImpactQueryFactors<'static> {
        ImpactQueryFactors {
            region: "us-east-1",
            fallback_pue: 1.15,
            fallback_wue_l_per_kwh: Some(0.15),
        }
    }

    #[tokio::test]
    async fn aggregates_one_bucket_with_known_inputs() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors_file = EnvironmentalFactorsFile::parse(PROD_TOML).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        // 1M input + 100K output on Apr 21.
        // energy_wh = 1_000_000 * 0.5/1e6 + 100_000 * 2.0/1e6 = 0.5 + 0.2 = 0.7
        // facility_wh = 0.7 * 1.15 = 0.805
        // co2e_g = 0.805 * 0.30 = 0.2415
        // water_l = (0.805/1000) * 0.20 = 0.000161
        insert_events(&database, &[event("claude-sonnet-4-6", 21, 1_000_000, 100_000)])
            .await
            .unwrap();

        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors(),
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.bucket, "2026-04-21");
        assert_eq!(row.model, "claude-sonnet-4-6");
        assert_eq!(row.input_tokens, 1_000_000);
        assert_eq!(row.output_tokens, 100_000);
        assert!((row.energy_wh - 0.7).abs() < 1e-9, "energy_wh={}", row.energy_wh);
        assert!(
            (row.facility_wh - 0.805).abs() < 1e-9,
            "facility_wh={}",
            row.facility_wh
        );
        let co2e = row.co2e_g.expect("co2e populated");
        assert!((co2e - 0.2415).abs() < 1e-9, "co2e={co2e}");
        let water = row.water_l.expect("water populated");
        assert!((water - 0.000_161).abs() < 1e-12, "water={water}");
        assert_eq!(row.max_uncertainty_pct, 35);
        assert_eq!(row.events_missing_env_factor, 0);
        assert_eq!(row.events_using_fallback_pue, 0);
        assert_eq!(row.events_using_fallback_wue, 0);
        assert_eq!(row.events_count, 1);
    }

    #[tokio::test]
    async fn flags_events_missing_env_factor() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors_file = EnvironmentalFactorsFile::parse(PROD_TOML).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        // Use a model NOT in the factor file.
        insert_events(&database, &[event("claude-haiku-99", 22, 1_000_000, 100_000)])
            .await
            .unwrap();

        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors(),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.events_missing_env_factor, 1);
        // No env_factor → all wh_per_mtok_* COALESCE to 0 → energy 0.
        assert!(row.energy_wh.abs() < 1e-9);
        assert_eq!(row.max_uncertainty_pct, 0);
    }

    #[tokio::test]
    async fn returns_empty_for_window_with_no_events() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors_file = EnvironmentalFactorsFile::parse(PROD_TOML).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors(),
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn buckets_by_week_when_requested() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let factors_file = EnvironmentalFactorsFile::parse(PROD_TOML).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        // Apr 20 (Mon) and Apr 21 (Tue) → same ISO week starting Apr 20.
        insert_events(
            &database,
            &[
                event("claude-sonnet-4-6", 20, 1_000_000, 0),
                event("claude-sonnet-4-6", 21, 1_000_000, 0),
            ],
        )
        .await
        .unwrap();

        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Week,
            &factors(),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bucket, "2026-04-20");
        assert_eq!(rows[0].input_tokens, 2_000_000);
        // 2_000_000 * 0.5 / 1e6 = 1.0 Wh (pre-PUE).
        assert!((rows[0].energy_wh - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn null_grid_pue_uses_fallback_per_event() {
        let database = Database::open_in_memory_for_tests().await.unwrap();

        // Custom factor file with grid pue NULL.
        let toml = PROD_TOML.replace("pue = 1.15\n", "");
        let factors_file = EnvironmentalFactorsFile::parse(&toml).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        insert_events(&database, &[event("claude-sonnet-4-6", 21, 1_000_000, 0)])
            .await
            .unwrap();

        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors(), // fallback_pue = 1.15
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // facility_wh = 0.5 * 1.15 = 0.575
        assert!((row.facility_wh - 0.575).abs() < 1e-9);
        assert_eq!(row.events_using_fallback_pue, 1);
    }

    #[tokio::test]
    async fn null_grid_water_with_no_fallback_yields_none() {
        let database = Database::open_in_memory_for_tests().await.unwrap();

        // Custom factor file with grid water NULL.
        let toml = PROD_TOML.replace("water_l_per_kwh = 0.20\n", "");
        let factors_file = EnvironmentalFactorsFile::parse(&toml).unwrap();
        sync_environmental_factors(&database, &factors_file)
            .await
            .unwrap();

        insert_events(&database, &[event("claude-sonnet-4-6", 21, 1_000_000, 0)])
            .await
            .unwrap();

        let no_fallback = ImpactQueryFactors {
            region: "us-east-1",
            fallback_pue: 1.15,
            fallback_wue_l_per_kwh: None,
        };
        let rows = aggregate_impact_by_bucket(
            &database,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &no_fallback,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].water_l.is_none(), "water_l should be None");
        assert_eq!(rows[0].events_using_fallback_wue, 1);
    }
}
