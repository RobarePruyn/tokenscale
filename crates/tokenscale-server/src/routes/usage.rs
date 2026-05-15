//! `GET /api/v1/usage/daily`  and `GET /api/v1/usage/by-model` handlers.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokenscale_core::{BillableMultipliers, ModelPricing};
use tokenscale_store::{
    aggregate_impact_by_bucket, list_models_in_window, usage_by_model, Granularity,
    ImpactByBucketRow, ImpactQueryFactors, ALL_PROVIDERS,
};

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_WINDOW_DAYS: i64 = 30;

/// Sentinel `project_id` value used by the `?project=__none__` query string
/// to mean "filter to nothing." Real Claude Code cwd paths cannot contain
/// these byte sequences, so the SQL IN-clause matches no rows.
const NO_MATCH_SENTINEL: &str = "\u{0}__tokenscale_filter_none__\u{0}";

/// The five token types tokenscale tracks. Stable order so the frontend's
/// stack-by-token-type chart has a deterministic visual layout.
const TOKEN_TYPES: [&str; 5] = [
    "input",
    "output",
    "cache_read",
    "cache_write_5m",
    "cache_write_1h",
];

#[derive(Debug, Deserialize)]
pub struct UsageWindowParams {
    /// `YYYY-MM-DD`. Defaults to 30 days ago (UTC).
    pub from: Option<String>,
    /// `YYYY-MM-DD`. Defaults to today (UTC).
    pub to: Option<String>,
    /// `all` (default) or a specific provider slug like `anthropic`.
    pub provider: Option<String>,
    /// Comma-separated `project_id` allow-list, or `all` (default) for no
    /// filter. axum's `Query` extractor doesn't deserialize repeated params
    /// into `Vec<String>`, so we use the comma-delimited convention here —
    /// project paths effectively never contain commas in practice.
    pub project: Option<String>,
    /// Bucket granularity — `day` (default), `week`, or `month`.
    pub granularity: Option<String>,
}

impl UsageWindowParams {
    fn resolve(self) -> Result<ResolvedParams, ApiError> {
        let to_date = self
            .to
            .unwrap_or_else(|| Utc::now().date_naive().to_string());
        let from_date = self.from.unwrap_or_else(|| {
            (Utc::now().date_naive() - Duration::days(DEFAULT_WINDOW_DAYS)).to_string()
        });
        validate_iso_date(&from_date)?;
        validate_iso_date(&to_date)?;
        let provider = self.provider.unwrap_or_else(|| ALL_PROVIDERS.to_owned());

        let projects: Vec<String> = match self.project.as_deref() {
            None | Some("" | "all") => Vec::new(),
            // Sentinel for "the user explicitly selected zero projects" — a
            // valid state from the chip UI's "Select none" button. We pass
            // a value that cannot collide with any real Claude Code cwd so
            // the IN-clause matches no rows.
            Some("__none__") => vec![NO_MATCH_SENTINEL.to_owned()],
            Some(comma_separated) => comma_separated
                .split(',')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect(),
        };

        let granularity = Granularity::parse_or_default(self.granularity.as_deref());

        Ok(ResolvedParams {
            from_date,
            to_date,
            provider,
            projects,
            granularity,
        })
    }
}

struct ResolvedParams {
    from_date: String,
    to_date: String,
    provider: String,
    projects: Vec<String>,
    granularity: Granularity,
}

fn validate_iso_date(value: &str) -> Result<(), ApiError> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| ApiError::BadRequest(format!("expected YYYY-MM-DD, got {value:?}")))
}

// ----------------------------------------------------------------------------
// daily — per-token-type breakdown per (date, model), with optional
// billable-equivalent total folded in when pricing is available.
// ----------------------------------------------------------------------------

/// One model's per-day token totals, broken out by token type. When
/// `pricing.toml` has an entry for `(provider, model)`, a parallel
/// `billable` breakdown lets the dashboard render "stack by token type +
/// view billable" without needing to know multipliers itself.
#[derive(Serialize)]
pub struct ModelTokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    /// Per-token-type billable equivalents, in input-token-equivalent units.
    /// Absent when no pricing entry exists for this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billable: Option<BillableBreakdown>,
    /// Sum of `billable.*` — convenient for the "stack by model + view
    /// billable" view to skip a client-side reduce on every render.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billable_total: Option<i64>,
    /// Environmental impact for this (bucket, model) cell — the per-event
    /// time-anchored aggregate of energy / facility energy / CO₂e / water,
    /// plus the provenance counters the dashboard shows in tooltips.
    /// Always present when the row carries any events; energy is `0.0`
    /// when no env_factor row was found for any event in the bucket
    /// (counted in `events_missing_env_factor`).
    pub impact: ModelImpact,
}

/// Per-(bucket, model) environmental impact rollup. Mirrors the field
/// set produced by `tokenscale_store::aggregate_impact_by_bucket`,
/// minus the dimensions used for grouping.
#[derive(Serialize)]
pub struct ModelImpact {
    /// Pre-PUE per-token energy summed across events.
    pub energy_wh: f64,
    /// Energy after PUE multiplier — the "facility-side" energy a
    /// data center actually drew, including overhead.
    pub facility_wh: f64,
    /// Grams of CO₂-equivalent. `null` when no event in the bucket had
    /// a usable grid `co2e_kg_per_kwh` — the dashboard renders that as
    /// "—" rather than 0 g.
    #[serde(rename = "co2eG")]
    pub co2e_g: Option<f64>,
    /// Liters of water. `null` when no event had a usable grid water
    /// factor and no fallback was configured.
    #[serde(rename = "waterL")]
    pub water_l: Option<f64>,
    /// Energy-side `± %` band (model-factor uncertainty only — PUE has
    /// no separately-tracked band yet, so it folds in here).
    #[serde(rename = "maxUncertaintyPct")]
    pub max_uncertainty_pct: i32,
    /// Combined `± %` for `co2e_g` — quadrature of the model and grid
    /// CO₂e uncertainty bands. See `tokenscale_core::combine_uncertainty_pct`.
    #[serde(rename = "co2eUncertaintyPct")]
    pub co2e_uncertainty_pct: i32,
    /// Combined `± %` for `water_l` — quadrature of model + grid water.
    #[serde(rename = "waterUncertaintyPct")]
    pub water_uncertainty_pct: i32,
    /// Number of events whose env_factor row was missing entirely. The
    /// dashboard surfaces this as "X events without factor data".
    #[serde(rename = "eventsMissingEnvFactor")]
    pub events_missing_env_factor: i64,
    /// Number of events that fell back to `defaults.fallback_pue`.
    #[serde(rename = "eventsUsingFallbackPue")]
    pub events_using_fallback_pue: i64,
    /// Number of events that fell back to
    /// `defaults.fallback_wue_l_per_kwh`.
    #[serde(rename = "eventsUsingFallbackWue")]
    pub events_using_fallback_wue: i64,
    #[serde(rename = "eventsCount")]
    pub events_count: i64,
}

/// Same five token-type fields as `ModelTokens`, but each pre-multiplied
/// by the model's billable weight. Sum to get `billable_total`.
#[derive(Serialize)]
pub struct BillableBreakdown {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
}

/// One row per UTC date with a `byModel` dict so the frontend's stacked
/// area chart can plug it in directly without re-shaping client-side.
#[derive(Serialize)]
pub struct DailyUsageRow {
    pub date: String,
    #[serde(rename = "byModel")]
    pub by_model: BTreeMap<String, ModelTokens>,
}

#[derive(Serialize)]
pub struct DailyUsageResponse {
    pub rows: Vec<DailyUsageRow>,
    /// Distinct model identifiers across the window, sorted by total tokens
    /// descending so the chart's stack order is stable and biggest-first.
    pub models: Vec<String>,
    /// Stable order of the per-token-type stack. The frontend uses this for
    /// its "stack by token type" view and for filter UI.
    #[serde(rename = "tokenTypes")]
    pub token_types: Vec<&'static str>,
    /// Models that appeared in the data but had no pricing entry — billable
    /// total is missing for those. Surfaced so the dashboard can mark them.
    #[serde(rename = "modelsWithoutPricing")]
    pub models_without_pricing: Vec<String>,
    /// Models that appeared in the data but had no env_factor row at any
    /// of the events' `occurred_at` dates — environmental views render
    /// these as "factor data unavailable" rather than zero. v0.1 ships
    /// real factor values for the major Anthropic models, so this is
    /// usually empty in practice.
    #[serde(rename = "modelsWithoutFactors")]
    pub models_without_factors: Vec<String>,
    /// Configured AWS region the impact figures are attributed to.
    /// Echoed back so the dashboard's environmental banner can render
    /// "us-east-1 (SRVC)" without a separate /health round-trip.
    #[serde(rename = "configuredRegion")]
    pub configured_region: String,
    /// Bucket size used for the rows. Echoed back so the frontend's x-axis
    /// formatter can match what was actually rendered (auto granularity is
    /// resolved client-side, but verifying server-side is cheap).
    pub granularity: Granularity,
    /// Per-model pricing snippet (`input_usd_per_mtok` only — sufficient
    /// for the dashboard to convert `billable` into USD via
    /// `billable_value × input_price ÷ 1_000_000`). Models without a
    /// pricing entry are absent from this map; same set as
    /// `modelsWithoutPricing`. Sized by model count, not row count, so
    /// the wire cost is trivial.
    #[serde(rename = "pricingByModel")]
    pub pricing_by_model: BTreeMap<String, ModelPricingForResponse>,
}

/// The slice of `ModelPricing` the dashboard needs to render the "Cost
/// (USD)" view. Kept narrow on purpose: the full pricing record stays
/// server-side, both to avoid leaking values the user hasn't asked for
/// and to keep the API surface stable when more fields appear in the
/// pricing schema later.
#[derive(Serialize)]
pub struct ModelPricingForResponse {
    pub input_usd_per_mtok: f64,
}

// The body coordinates several pieces — visible-models lookup, impact
// aggregation, billable derivation, response assembly. Splitting it
// further would just add tiny single-call helpers that are harder to
// follow than the linear flow here.
#[allow(clippy::too_many_lines)]
pub async fn daily_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageWindowParams>,
) -> Result<Json<DailyUsageResponse>, ApiError> {
    let resolved = params.resolve()?;

    // The Models chip list is computed at the WINDOW level — provider +
    // date range only — so it doesn't collapse when the user narrows the
    // project filter (or selects no projects at all). Without this, "Select
    // none" on Projects would clear the Model chip list, leaving the user
    // with no chips to click back to.
    let models_in_window = list_models_in_window(
        &state.database,
        &resolved.from_date,
        &resolved.to_date,
        &resolved.provider,
    )
    .await?;
    let visible_models: std::collections::HashSet<String> = models_in_window
        .iter()
        .map(|row| row.model.clone())
        .collect();
    let window_totals: BTreeMap<String, i64> = models_in_window
        .iter()
        .map(|row| (row.model.clone(), row.total_tokens))
        .collect();

    // Per-(bucket, model) impact aggregation is the unified data source —
    // it carries token sums AND factor-anchored impact in one query. The
    // dashboard always gets the full impact block; "show / hide
    // environmental view" is a frontend concern.
    let impact_factors = ImpactQueryFactors {
        region: state.inference_region.as_str(),
        fallback_pue: state.factors.effective_fallback_pue(),
        fallback_wue_l_per_kwh: state.factors.effective_fallback_wue_l_per_kwh(),
    };
    let impact_rows = aggregate_impact_by_bucket(
        &state.database,
        &resolved.from_date,
        &resolved.to_date,
        &resolved.provider,
        &resolved.projects,
        resolved.granularity,
        &impact_factors,
    )
    .await?;

    // For Phase 1, every visible model is provider=anthropic by virtue of
    // sources.provider being filtered to anthropic when applicable. We hand
    // that to the pricing lookup. v2 will need to carry the actual provider
    // through breakdown rows — which the impact_rows now do.
    let provider_for_pricing = if resolved.provider == ALL_PROVIDERS {
        "anthropic"
    } else {
        resolved.provider.as_str()
    };

    let mut models_without_pricing: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    // Group impact rows into the nested by-date shape, with billable +
    // impact attached per (date, model) cell.
    let mut grouped_by_date: BTreeMap<String, BTreeMap<String, ModelTokens>> = BTreeMap::new();
    for row in impact_rows {
        if !visible_models.contains(&row.model) {
            continue;
        }

        let billable_pair = state
            .pricing
            .lookup(provider_for_pricing, &row.model)
            .map(|model_pricing| compute_billable_breakdown(model_pricing, &row));
        let (billable, billable_total) = if let Some((breakdown, total)) = billable_pair {
            (Some(breakdown), Some(total))
        } else {
            models_without_pricing.insert(row.model.clone());
            (None, None)
        };

        let bucket = row.bucket.clone();
        let model_key = row.model.clone();
        let model_tokens = ModelTokens {
            input: row.input_tokens,
            output: row.output_tokens,
            cache_read: row.cache_read_tokens,
            cache_write_5m: row.cache_write_5m_tokens,
            cache_write_1h: row.cache_write_1h_tokens,
            billable,
            billable_total,
            impact: ModelImpact {
                energy_wh: row.energy_wh,
                facility_wh: row.facility_wh,
                co2e_g: row.co2e_g,
                water_l: row.water_l,
                max_uncertainty_pct: row.max_uncertainty_pct,
                co2e_uncertainty_pct: row.co2e_uncertainty_pct,
                water_uncertainty_pct: row.water_uncertainty_pct,
                events_missing_env_factor: row.events_missing_env_factor,
                events_using_fallback_pue: row.events_using_fallback_pue,
                events_using_fallback_wue: row.events_using_fallback_wue,
                events_count: row.events_count,
            },
        };
        grouped_by_date
            .entry(bucket)
            .or_default()
            .insert(model_key, model_tokens);
    }

    let mut models: Vec<String> = visible_models.iter().cloned().collect();
    // Sort by raw total desc, ties broken alphabetically for determinism.
    models.sort_by(|left, right| {
        window_totals[right]
            .cmp(&window_totals[left])
            .then_with(|| left.cmp(right))
    });

    // Models present in the data but missing from the in-memory factor
    // snapshot. The aggregate query also surfaces per-event missing-factor
    // counts via `events_missing_env_factor`; this list is the model-level
    // answer for dashboard banners ("Claude Opus 4.7: factor data unavailable").
    let models_without_factors: Vec<String> = visible_models
        .iter()
        .filter(|model_id| {
            !state
                .factors
                .providers
                .get(provider_for_pricing)
                .is_some_and(|provider| provider.models.contains_key(*model_id))
        })
        .cloned()
        .collect();
    let mut models_without_factors = models_without_factors;
    models_without_factors.sort();

    let rows = grouped_by_date
        .into_iter()
        .map(|(date, by_model)| DailyUsageRow { date, by_model })
        .collect();

    // Build the per-model pricing snippet for visible models that have an
    // entry. Sized by model count, not row count.
    let pricing_by_model: BTreeMap<String, ModelPricingForResponse> = models
        .iter()
        .filter_map(|model_id| {
            state
                .pricing
                .lookup(provider_for_pricing, model_id)
                .map(|model_pricing| {
                    (
                        model_id.clone(),
                        ModelPricingForResponse {
                            input_usd_per_mtok: model_pricing.input_usd_per_mtok,
                        },
                    )
                })
        })
        .collect();

    Ok(Json(DailyUsageResponse {
        rows,
        models,
        token_types: TOKEN_TYPES.to_vec(),
        models_without_pricing: models_without_pricing.into_iter().collect(),
        models_without_factors,
        configured_region: state.inference_region.clone(),
        granularity: resolved.granularity,
        pricing_by_model,
    }))
}

/// Compute the per-token-type billable equivalents for one (date, model)
/// breakdown row. Returns the per-type breakdown alongside the sum.
///
/// Intentional `as i64` casts: `BillableMultipliers::weight_total` returns
/// `f64` for lossless internal math; we truncate at the API boundary because
/// the dashboard doesn't render fractional tokens. The values are far below
/// `f64`'s 2^53 mantissa limit even on years-of-data instances, so the
/// `i64 → f64` casts on the input side are also intentional. The local
/// `cache_write_5m` / `cache_write_1h` bindings mirror the Anthropic API's
/// own naming for the two prompt-cache classes.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::similar_names
)]
fn compute_billable_breakdown(
    model_pricing: &ModelPricing,
    row: &ImpactByBucketRow,
) -> (BillableBreakdown, i64) {
    let multipliers = BillableMultipliers::from_pricing(model_pricing);
    let input = (row.input_tokens.max(0) as f64 * multipliers.input) as i64;
    let output = (row.output_tokens.max(0) as f64 * multipliers.output) as i64;
    let cache_read = (row.cache_read_tokens.max(0) as f64 * multipliers.cache_read) as i64;
    let cache_write_5m =
        (row.cache_write_5m_tokens.max(0) as f64 * multipliers.cache_write_5m) as i64;
    let cache_write_1h =
        (row.cache_write_1h_tokens.max(0) as f64 * multipliers.cache_write_1h) as i64;
    let total = input + output + cache_read + cache_write_5m + cache_write_1h;
    (
        BillableBreakdown {
            input,
            output,
            cache_read,
            cache_write_5m,
            cache_write_1h,
        },
        total,
    )
}

// ----------------------------------------------------------------------------
// by-model — totals across the window (unchanged by Iteration A)
// ----------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ByModelResponse {
    pub rows: Vec<ByModelRow>,
}

#[derive(Serialize)]
pub struct ByModelRow {
    pub model: String,
    pub event_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
}

pub async fn by_model_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageWindowParams>,
) -> Result<Json<ByModelResponse>, ApiError> {
    let resolved = params.resolve()?;
    let rows = usage_by_model(
        &state.database,
        &resolved.from_date,
        &resolved.to_date,
        &resolved.provider,
    )
    .await?;
    let rows = rows
        .into_iter()
        .map(|row| ByModelRow {
            model: row.model,
            event_count: row.event_count,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_read_tokens: row.cache_read_tokens,
            cache_write_5m_tokens: row.cache_write_5m_tokens,
            cache_write_1h_tokens: row.cache_write_1h_tokens,
        })
        .collect();
    Ok(Json(ByModelResponse { rows }))
}
