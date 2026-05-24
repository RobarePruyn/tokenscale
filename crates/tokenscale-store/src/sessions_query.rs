//! Per-session impact + cost aggregation.
//!
//! Granular-attribution Phase 1A. Mirrors
//! [`crate::impact_query::aggregate_impact_by_bucket`] structurally —
//! same per-event time-anchored pricing + factor joins (the v0.1.13
//! correlated-subquery pattern + the v0.1.10 quadrature-uncertainty
//! pattern) — but `GROUP BY events.session_id` instead of
//! `(bucket, provider, model)`. The discipline transfers verbatim:
//! per-session numbers inherit time-anchoring and missingness handling
//! from the same SQL.
//!
//! Sessions span models in practice — CC routes Sonnet for cheap turns
//! and Opus for hard ones — so each row aggregates ACROSS models and
//! emits a `models` list (via `GROUP_CONCAT(DISTINCT events.model)`)
//! alongside the combined totals.
//!
//! Project filter, provider filter, and the date window match
//! `/api/v1/usage/daily`'s shape; the only new query params are
//! `limit` + `offset` for forward-compat server-side pagination
//! (effective no-ops with the default limit of 10_000, which exceeds
//! any realistic 90-day window's session count).

use serde::Serialize;
use sqlx::QueryBuilder;

use crate::error::Result;
use crate::impact_query::ImpactQueryFactors;
use crate::queries::{Granularity, ALL_PROVIDERS};
use crate::Database;

// Granularity is unused in this file but the import keeps the module
// boundary symmetric with impact_query.rs (which uses it). Re-exported
// for callers that want both.
#[allow(unused_imports)]
use Granularity as _;

/// One row per session present in the window. Fields mirror
/// [`crate::impact_query::ImpactByBucketRow`] one-to-one for token /
/// cost / impact / provenance, plus session-specific fields
/// (`session_id`, `project_id`, `models`, `first_event_at`,
/// `last_event_at`).
///
/// `session_id` is the full UUID as written by Claude Code into the
/// JSONL `sessionId` field. The dashboard truncates to the first 8
/// characters for display; the full UUID stays in the response so
/// operators can grep the same prefix against
/// `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` filenames.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummaryRow {
    pub session_id: String,

    /// Smallest non-NULL `project_id` (raw `cwd` as the parser stores
    /// it) observed on any event in this session. A session is tied to
    /// one `cwd` in practice; the rare multi-cwd case uses MIN for
    /// determinism. Phase 1B will replace raw `cwd` with the resolved
    /// git toplevel; this row's `project_id` shape doesn't change.
    pub project_id: Option<String>,

    // Future refinement candidate: per-session project_id is currently
    // MAX(project_id) — picks the lexicographically largest cwd, which
    // approximates "most-specific subdirectory" but can still be wrong
    // for sessions that genuinely span multiple repos. A
    // most-frequent-cwd aggregation would be more accurate; tracked
    // for post-v0.1.15 work.

    /// Comma-separated list of distinct model IDs seen in this
    /// session, in SQLite's `GROUP_CONCAT(DISTINCT ...)`
    /// implementation-defined order. The frontend splits on `,` and
    /// sorts for stable display. Sessions are not single-model in
    /// practice — multi-model is the normal case for non-trivial
    /// sessions.
    pub models: String,

    pub first_event_at: String,
    pub last_event_at: String,

    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,

    pub energy_wh: f64,
    pub facility_wh: f64,
    pub co2e_g: Option<f64>,
    pub water_l: Option<f64>,
    pub indirect_water_l: Option<f64>,

    pub max_uncertainty_pct: i32,
    pub co2e_uncertainty_pct: i32,
    pub water_uncertainty_pct: i32,
    pub indirect_water_uncertainty_pct: i32,

    pub cost_usd_input: f64,
    pub cost_usd_output: f64,
    pub cost_usd_cache_read: f64,
    pub cost_usd_cache_write_5m: f64,
    pub cost_usd_cache_write_1h: f64,
    pub cost_usd_total: Option<f64>,

    pub events_missing_pricing: i64,
    pub events_missing_env_factor: i64,
    pub events_using_fallback_pue: i64,
    pub events_using_fallback_wue: i64,
    pub events_count: i64,
}

/// Default `limit` when callers don't specify one. Sized to exceed any
/// realistic 90-day window's session count by 1-2 orders of magnitude
/// — heaviest observed maintainer usage produces ~hundreds of sessions
/// per month. The frontend doesn't page in v0.1.15; this default is
/// the "effective unbounded" path. Server-side pagination becomes a
/// real concern only once `limit` and `offset` get used in earnest.
pub const DEFAULT_SESSION_LIMIT: i64 = 10_000;

/// Aggregate impact + cost per session. Same window / provider /
/// project filter surface as [`crate::impact_query::aggregate_impact_by_bucket`].
///
/// `limit` and `offset` are included in the signature for API
/// forward-compat with future server-side pagination; the v0.1.15
/// frontend doesn't use them. Default values via the helper above.
///
/// Sorted by `last_event_at DESC, session_id ASC` so recency-first
/// with a deterministic tie-break.
//
// The body is one big QueryBuilder, same shape as
// aggregate_impact_by_bucket. Splitting it out would only shuffle SQL
// fragments without clarifying the math.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
pub async fn list_sessions_with_totals(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
    project_filter: &[String],
    factors: &ImpactQueryFactors<'_>,
    limit: i64,
    offset: i64,
) -> Result<Vec<SessionSummaryRow>> {
    let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
        "SELECT
            events.session_id AS session_id,
            -- Per-session project pick: the LONGEST cwd seen in this
            -- session, ties broken alphabetically. The longest path
            -- is the most-specific work directory in practice — a
            -- session that mixes a one-off `cd ~` event with deep
            -- repo subdirectory work should attribute to the repo,
            -- not to home. Lexicographic MAX would fail this when a
            -- short cwd has a higher-ASCII root letter (e.g.
            -- /private/tmp/short beats /Users/long/path because
            -- 'p' > 'U'). Longest-wins is robust against that.
            -- Truly correct = most-frequent cwd, which would require
            -- a window-function aggregation; longest-wins is the
            -- v0.1.15 heuristic that handles the observed common case.
            (SELECT e2.project_id
               FROM events e2
              WHERE e2.session_id = events.session_id
                AND e2.project_id IS NOT NULL
              ORDER BY LENGTH(e2.project_id) DESC, e2.project_id ASC
              LIMIT 1) AS project_id,
            GROUP_CONCAT(DISTINCT events.model) AS models,
            MIN(events.occurred_at) AS first_event_at,
            MAX(events.occurred_at) AS last_event_at,
            COALESCE(SUM(events.input_tokens), 0)            AS input_tokens,
            COALESCE(SUM(events.output_tokens), 0)           AS output_tokens,
            COALESCE(SUM(events.cache_read_tokens), 0)       AS cache_read_tokens,
            COALESCE(SUM(events.cache_write_5m_tokens), 0)   AS cache_write_5m_tokens,
            COALESCE(SUM(events.cache_write_1h_tokens), 0)   AS cache_write_1h_tokens,
            -- Energy (pre-PUE), summed per event using each event's
            -- own time-anchored env_factor row.
            COALESCE(SUM(
                (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
               + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
               + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
               + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
               + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
                ) / 1000000.0
            ), 0)                                            AS energy_wh,
            -- Facility energy (= energy × PUE, with per-event fallback PUE).
            COALESCE(SUM(
                (events.input_tokens          * COALESCE(ef.wh_per_mtok_input, 0)
               + events.output_tokens         * COALESCE(ef.wh_per_mtok_output, 0)
               + events.cache_read_tokens     * COALESCE(ef.wh_per_mtok_cache_read, 0)
               + events.cache_write_5m_tokens * COALESCE(ef.wh_per_mtok_cache_write_5m, 0)
               + events.cache_write_1h_tokens * COALESCE(ef.wh_per_mtok_cache_write_1h, 0)
                ) / 1000000.0
                * COALESCE(gf.pue, ",
    );
    builder.push_bind(factors.fallback_pue);
    builder.push(")
            ), 0)                                            AS facility_wh,
            -- CO₂e in grams = facility_wh × kg/kWh. NULL when no event
            -- in the session had a non-null grid CO₂e factor (promoted
            -- to None in cook() via events_with_co2e).
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
            -- Direct water (L) = facility_kWh × L/kWh, with per-event fallback WUE.
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
            -- Indirect (off-site / power-plant cooling) water in L.
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
                * gf.indirect_water_l_per_kwh
            )                                                AS indirect_water_l_raw,
            SUM(CASE WHEN gf.indirect_water_l_per_kwh IS NOT NULL THEN 1 ELSE 0 END) AS events_with_indirect_water,
            -- Widest-wins uncertainty across all events in the session
            -- (so a session that ran both a Sonnet-secondary turn and
            -- an Opus-secondary turn surfaces the larger band).
            COALESCE(MAX(ef.uncertainty_range_pct), 0)             AS max_uncertainty_pct,
            COALESCE(MAX(gf.co2e_uncertainty_range_pct), 0)            AS grid_co2e_uncertainty_pct,
            COALESCE(MAX(gf.water_uncertainty_range_pct), 0)           AS grid_water_uncertainty_pct,
            COALESCE(MAX(gf.indirect_water_uncertainty_range_pct), 0)  AS grid_indirect_water_uncertainty_pct,
            -- Per-token-type cost (v0.1.13 pattern). Each is
            -- tokens × time-anchored rate / 1e6; unpriced events
            -- contribute 0 via COALESCE. The sum invariant from
            -- v0.1.14 (sum of 5 per-type = total) holds at this grain
            -- too — proven by `per_token_type_costs_sum_to_total_per_session`.
            SUM(events.input_tokens * COALESCE(pr.input_usd_per_mtok, 0) / 1000000.0)
                                                             AS cost_usd_input,
            SUM(events.output_tokens * COALESCE(pr.output_usd_per_mtok, 0) / 1000000.0)
                                                             AS cost_usd_output,
            SUM(events.cache_read_tokens * COALESCE(pr.cache_read_usd_per_mtok, 0) / 1000000.0)
                                                             AS cost_usd_cache_read,
            SUM(events.cache_write_5m_tokens * COALESCE(pr.cache_write_5m_multiplier, 0)
                                             * COALESCE(pr.input_usd_per_mtok, 0) / 1000000.0)
                                                             AS cost_usd_cache_write_5m,
            SUM(events.cache_write_1h_tokens * COALESCE(pr.cache_write_1h_multiplier, 0)
                                             * COALESCE(pr.input_usd_per_mtok, 0) / 1000000.0)
                                                             AS cost_usd_cache_write_1h,
            SUM(CASE WHEN pr.id IS NULL THEN 1 ELSE 0 END)   AS events_missing_pricing,
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
           LEFT JOIN pricing pr
                  ON pr.provider = sources.provider
                 AND pr.model = events.model
                 AND pr.valid_from = (
                     SELECT MAX(valid_from) FROM pricing
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
          WHERE events.session_id IS NOT NULL
            AND date(events.occurred_at) BETWEEN ");
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
        " GROUP BY events.session_id
          ORDER BY last_event_at DESC, events.session_id ASC
          LIMIT ",
    );
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let raw_rows: Vec<RawSessionRow> =
        builder.build_query_as().fetch_all(database.pool()).await?;
    Ok(raw_rows.into_iter().map(RawSessionRow::cook).collect())
}

#[derive(sqlx::FromRow)]
struct RawSessionRow {
    session_id: String,
    project_id: Option<String>,
    /// `GROUP_CONCAT` returns NULL on an empty group, but every group
    /// here has at least one event (else it wouldn't be a group), so
    /// the value is always Some — we collapse to String at cook() time.
    models: Option<String>,
    first_event_at: String,
    last_event_at: String,
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
    indirect_water_l_raw: Option<f64>,
    events_with_indirect_water: i64,
    cost_usd_input: f64,
    cost_usd_output: f64,
    cost_usd_cache_read: f64,
    cost_usd_cache_write_5m: f64,
    cost_usd_cache_write_1h: f64,
    events_missing_pricing: i64,
    max_uncertainty_pct: i32,
    grid_co2e_uncertainty_pct: i32,
    grid_water_uncertainty_pct: i32,
    grid_indirect_water_uncertainty_pct: i32,
    events_missing_env_factor: i64,
    events_using_fallback_pue: i64,
    events_using_fallback_wue: i64,
    events_count: i64,
}

impl RawSessionRow {
    fn cook(self) -> SessionSummaryRow {
        // Same missingness-promotion rules as ImpactByBucketRow.
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
        let indirect_water_l = if self.events_with_indirect_water > 0 {
            self.indirect_water_l_raw
        } else {
            None
        };

        let cost_usd_total = if self.events_missing_pricing < self.events_count {
            Some(
                self.cost_usd_input
                    + self.cost_usd_output
                    + self.cost_usd_cache_read
                    + self.cost_usd_cache_write_5m
                    + self.cost_usd_cache_write_1h,
            )
        } else {
            None
        };

        let co2e_uncertainty_pct = tokenscale_core::combine_uncertainty_pct(
            self.max_uncertainty_pct,
            self.grid_co2e_uncertainty_pct,
        );
        let water_uncertainty_pct = tokenscale_core::combine_uncertainty_pct(
            self.max_uncertainty_pct,
            self.grid_water_uncertainty_pct,
        );
        let indirect_water_uncertainty_pct = tokenscale_core::combine_uncertainty_pct(
            self.max_uncertainty_pct,
            self.grid_indirect_water_uncertainty_pct,
        );

        SessionSummaryRow {
            session_id: self.session_id,
            project_id: self.project_id,
            models: self.models.unwrap_or_default(),
            first_event_at: self.first_event_at,
            last_event_at: self.last_event_at,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_5m_tokens: self.cache_write_5m_tokens,
            cache_write_1h_tokens: self.cache_write_1h_tokens,
            energy_wh: self.energy_wh,
            facility_wh: self.facility_wh,
            co2e_g,
            water_l,
            indirect_water_l,
            max_uncertainty_pct: self.max_uncertainty_pct,
            co2e_uncertainty_pct,
            water_uncertainty_pct,
            indirect_water_uncertainty_pct,
            cost_usd_input: self.cost_usd_input,
            cost_usd_output: self.cost_usd_output,
            cost_usd_cache_read: self.cost_usd_cache_read,
            cost_usd_cache_write_5m: self.cost_usd_cache_write_5m,
            cost_usd_cache_write_1h: self.cost_usd_cache_write_1h,
            cost_usd_total,
            events_missing_pricing: self.events_missing_pricing,
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
    use tokenscale_core::{EnvironmentalFactorsFile, Event};

    use crate::{insert_events, sync_environmental_factors, sync_pricing};

    const PROD_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2026-01-01"
source_doc = "test"
wh_per_mtok_input = 0.5
wh_per_mtok_output = 2.0
wh_per_mtok_cache_read = 0.05
wh_per_mtok_cache_write_5m = 0.5
wh_per_mtok_cache_write_1h = 0.5
uncertainty_range_pct = 35
confidence = "secondary"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-01-01"
source_doc = "test"
wh_per_mtok_input = 2.0
wh_per_mtok_output = 8.0
wh_per_mtok_cache_read = 0.20
wh_per_mtok_cache_write_5m = 2.0
wh_per_mtok_cache_write_1h = 2.0
uncertainty_range_pct = 50
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

    const PRICING_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name              = "Claude Sonnet 4.6"
valid_from                = "2025-09-01"
input_usd_per_mtok        = 3.00
output_usd_per_mtok       = 15.00
cache_read_usd_per_mtok   = 0.30
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "x"
source_accessed_at        = "2026-05-21"

[providers.anthropic.models."claude-opus-4-7"]
display_name              = "Claude Opus 4.7"
valid_from                = "2026-04-16"
input_usd_per_mtok        = 5.00
output_usd_per_mtok       = 25.00
cache_read_usd_per_mtok   = 0.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "x"
source_accessed_at        = "2026-05-21"
"#;

    fn factors() -> ImpactQueryFactors<'static> {
        ImpactQueryFactors {
            region: "us-east-1",
            fallback_pue: 1.15,
            fallback_wue_l_per_kwh: Some(0.15),
        }
    }

    /// Event with explicit session_id + project_id + cache fields. The
    /// existing impact_query::tests::event() helper sets session_id to
    /// None — that won't work here since the GROUP BY excludes NULLs.
    #[allow(clippy::too_many_arguments)]
    fn event_in_session(
        session: &str,
        project: &str,
        model: &str,
        day: u32,
        request_id: &str,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write_5m: u64,
        cache_write_1h: u64,
    ) -> Event {
        Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, day, 12, 0, 0).unwrap(),
            model: model.to_owned(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_5m_tokens: cache_write_5m,
            cache_write_1h_tokens: cache_write_1h,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some(session.to_owned()),
            project_id: Some(project.to_owned()),
            workspace_id: None,
            api_key_id: None,
            uuid: None,
            parent_uuid: None,
            git_branch: None,
            raw: None,
        }
    }

    async fn seed_factors_and_pricing(db: &Database) {
        let factors_file = EnvironmentalFactorsFile::parse(PROD_TOML).unwrap();
        sync_environmental_factors(db, &factors_file).await.unwrap();
        let pricing = tokenscale_core::PricingFile::parse(PRICING_TOML).unwrap();
        sync_pricing(db, &pricing).await.unwrap();
    }

    #[tokio::test]
    async fn groups_by_session_id() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        // Three events: two in session-A (Sonnet), one in session-B.
        insert_events(
            &db,
            &[
                event_in_session("sess-A", "/proj-A", "claude-sonnet-4-6", 21, "r1", 1_000_000, 100_000, 0, 0, 0),
                event_in_session("sess-A", "/proj-A", "claude-sonnet-4-6", 21, "r2", 500_000, 50_000, 0, 0, 0),
                event_in_session("sess-B", "/proj-B", "claude-sonnet-4-6", 22, "r3", 200_000, 20_000, 0, 0, 0),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 2, "two sessions → two rows");
        // ORDER BY last_event_at DESC — session-B (day 22) comes first.
        assert_eq!(rows[0].session_id, "sess-B");
        assert_eq!(rows[0].events_count, 1);
        assert_eq!(rows[1].session_id, "sess-A");
        assert_eq!(rows[1].events_count, 2);
        assert_eq!(rows[1].input_tokens, 1_500_000);
        assert_eq!(rows[1].output_tokens, 150_000);
    }

    #[tokio::test]
    async fn anchors_costs_per_session() {
        // Phase-1A version of v0.1.13's cost-anchoring test. Two Sonnet
        // events in one session: 1M input + 100K output → $3 input cost
        // + $1.50 output cost = $4.50 total.
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        insert_events(
            &db,
            &[event_in_session("s1", "/p", "claude-sonnet-4-6", 21, "r1", 1_000_000, 100_000, 0, 0, 0)],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        let cost = row.cost_usd_total.expect("priced session has total");
        assert!((cost - 4.50).abs() < 1e-9, "cost_usd_total={cost}");
        assert_eq!(row.events_missing_pricing, 0);
    }

    #[tokio::test]
    async fn filters_by_project() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        insert_events(
            &db,
            &[
                event_in_session("s1", "/proj-A", "claude-sonnet-4-6", 21, "r1", 1_000_000, 0, 0, 0, 0),
                event_in_session("s2", "/proj-B", "claude-sonnet-4-6", 21, "r2", 1_000_000, 0, 0, 0, 0),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &["/proj-A".to_owned()], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "s1");
        assert_eq!(rows[0].project_id.as_deref(), Some("/proj-A"));
    }

    #[tokio::test]
    async fn excludes_events_outside_date_window() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        insert_events(
            &db,
            &[event_in_session("s1", "/p", "claude-sonnet-4-6", 21, "r1", 1_000_000, 0, 0, 0, 0)],
        )
        .await
        .unwrap();

        // Window before the event.
        let rows = list_sessions_with_totals(
            &db, "2026-03-01", "2026-03-31",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert!(rows.is_empty(), "session outside window must not appear");

        // Window containing the event.
        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    // ----------------------------------------------------------------
    // Addition 1: sum-invariant at session grain.
    // v0.1.14 closed #2 by pinning costUsdInput+…+costUsdCacheWrite1h
    // == costUsdTotal at the per-bucket grain. Same discipline applied
    // at the new session grain: if session-level aggregation breaks
    // the invariant, this catches it before 1A lands.
    // ----------------------------------------------------------------

    #[tokio::test]
    async fn per_token_type_costs_sum_to_total_per_session() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        // Three events in one session, all five token types non-zero
        // on each so the per-type SUMs each have multiple contributions.
        insert_events(
            &db,
            &[
                event_in_session("s1", "/p", "claude-sonnet-4-6", 21, "r1", 1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000),
                event_in_session("s1", "/p", "claude-sonnet-4-6", 22, "r2", 500_000, 500_000, 500_000, 500_000, 500_000),
                event_in_session("s1", "/p", "claude-sonnet-4-6", 23, "r3", 250_000, 250_000, 250_000, 250_000, 250_000),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];

        let total = row.cost_usd_total.expect("priced session must have total");
        let sum = row.cost_usd_input
            + row.cost_usd_output
            + row.cost_usd_cache_read
            + row.cost_usd_cache_write_5m
            + row.cost_usd_cache_write_1h;
        assert!(
            (sum - total).abs() < 1e-9,
            "per-type sum {sum} must equal session-total {total} (Δ={})",
            sum - total,
        );
    }

    // ----------------------------------------------------------------
    // Addition 2: multi-model session.
    // Sessions are not single-model in practice; CC routes Sonnet for
    // cheap turns + Opus for hard ones. This test pins the new
    // session-axis aggregation against the model-axis the v0.1.13 SQL
    // already proves correct — wrong implementation could lose the
    // model dimension entirely (one row per (session, model) instead
    // of one row per session) or could double-count.
    // ----------------------------------------------------------------

    #[tokio::test]
    async fn multi_cwd_session_picks_most_specific_project() {
        // v0.1.15 1B-i smoke test surfaced that the maintainer's home
        // directory was a git repo, so any session containing a
        // `cd ~` event would have MIN(project_id) = `/Users/home`
        // and the whole session's work attributed to home. MAX picks
        // the lexicographically largest, which for prefixed cwds
        // (home + subdirectories) gives the longer / more-specific
        // path. This pins the v0.1.15 fix against regression to MIN.
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        // One session with two events: one shallow cwd, one deep.
        // The deep cwd represents where the real work happened; the
        // shallow one (typically the user's home dir) is a one-off.
        insert_events(
            &db,
            &[
                event_in_session("s1", "/Users/home", "claude-sonnet-4-6", 21, "r1", 100, 50, 0, 0, 0),
                event_in_session("s1", "/Users/home/Dev/myrepo/src", "claude-sonnet-4-6", 21, "r2", 100, 50, 0, 0, 0),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // Longest-cwd-wins picks the longer/more-specific path. NOT
        // "/Users/home" (which MIN would have picked, attributing
        // real work to the home-directory git toplevel) and NOT
        // dependent on lexicographic ordering of root letters.
        assert_eq!(
            row.project_id.as_deref(),
            Some("/Users/home/Dev/myrepo/src"),
            "multi-cwd session must attribute to the deepest path, not the shallowest",
        );
    }

    #[tokio::test]
    async fn longest_cwd_wins_over_higher_ascii_root() {
        // Pins the v0.1.15 fix that drove the choice away from MAX:
        // a session with a short cwd that has a higher-ASCII root
        // letter (e.g. /private/...) must NOT outrank a longer cwd
        // under /Users/... even though MAX would pick /private/
        // because 'p' > 'U' in ASCII.
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        insert_events(
            &db,
            &[
                // Short cwd, higher-ASCII root.
                event_in_session("s1", "/private/tmp/scratch", "claude-sonnet-4-6", 21, "r1", 100, 50, 0, 0, 0),
                // Long cwd, lower-ASCII root.
                event_in_session("s1", "/Users/me/Library/Documents/MyRepo/src/deep", "claude-sonnet-4-6", 21, "r2", 100, 50, 0, 0, 0),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        let row = &rows[0];
        assert_eq!(
            row.project_id.as_deref(),
            Some("/Users/me/Library/Documents/MyRepo/src/deep"),
            "longest cwd must win regardless of root-letter ASCII; \
             MAX would have wrongly picked /private/tmp/scratch",
        );
    }

    #[tokio::test]
    async fn multi_model_session_aggregates_across_models() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        seed_factors_and_pricing(&db).await;

        // One session: one Sonnet turn + one Opus turn.
        //   Sonnet: 1M input + 100K output → cost = 1M×3/1e6 + 100K×15/1e6 = 3 + 1.5 = 4.50
        //   Opus:    100K input + 10K output  → cost = 100K×5/1e6 + 10K×25/1e6 = 0.50 + 0.25 = 0.75
        //   Combined session cost: 5.25
        insert_events(
            &db,
            &[
                event_in_session("multi", "/p", "claude-sonnet-4-6", 21, "rs", 1_000_000, 100_000, 0, 0, 0),
                event_in_session("multi", "/p", "claude-opus-4-7",   21, "ro", 100_000, 10_000, 0, 0, 0),
            ],
        )
        .await
        .unwrap();

        let rows = list_sessions_with_totals(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], &factors(),
            DEFAULT_SESSION_LIMIT, 0,
        )
        .await
        .unwrap();
        // One row per session, NOT per (session, model). If a wrong
        // implementation left model in GROUP BY, this would be 2.
        assert_eq!(rows.len(), 1, "multi-model session must collapse to one row");
        let row = &rows[0];
        assert_eq!(row.session_id, "multi");
        assert_eq!(row.events_count, 2);

        // The `models` field carries both, order-independent.
        let mut model_list: Vec<&str> = row.models.split(',').collect();
        model_list.sort();
        assert_eq!(
            model_list,
            vec!["claude-opus-4-7", "claude-sonnet-4-6"],
            "models list must carry both models seen in the session",
        );

        // Combined cost equals sum of per-model contributions.
        let total = row.cost_usd_total.expect("priced session");
        assert!(
            (total - 5.25).abs() < 1e-9,
            "multi-model session cost = sonnet + opus contributions: got {total}, expected 5.25",
        );
        assert!((row.cost_usd_input - 3.50).abs() < 1e-9,
            "input cost = sonnet (1M × 3 / 1e6 = 3.00) + opus (100K × 5 / 1e6 = 0.50) = 3.50");
        assert!((row.cost_usd_output - 1.75).abs() < 1e-9,
            "output cost = sonnet (100K × 15 / 1e6 = 1.50) + opus (10K × 25 / 1e6 = 0.25) = 1.75");

        // Sum invariant still holds across models.
        let sum = row.cost_usd_input + row.cost_usd_output
            + row.cost_usd_cache_read
            + row.cost_usd_cache_write_5m + row.cost_usd_cache_write_1h;
        assert!(
            (sum - total).abs() < 1e-9,
            "sum invariant must hold across multi-model session",
        );

        // Widest-wins uncertainty: Sonnet=35, Opus=50 → MAX=50.
        assert_eq!(row.max_uncertainty_pct, 50,
            "widest-wins uncertainty across the two models");
    }
}
