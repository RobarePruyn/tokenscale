//! `GET /api/v1/usage/daily`  and `GET /api/v1/usage/by-model` handlers.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokenscale_core::BillableMultipliers;
use tokenscale_store::{daily_usage_breakdown, usage_by_model, ALL_PROVIDERS};

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_WINDOW_DAYS: i64 = 30;

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
}

impl UsageWindowParams {
    fn resolve(self) -> Result<(String, String, String), ApiError> {
        let to_date = self
            .to
            .unwrap_or_else(|| Utc::now().date_naive().to_string());
        let from_date = self.from.unwrap_or_else(|| {
            (Utc::now().date_naive() - Duration::days(DEFAULT_WINDOW_DAYS)).to_string()
        });
        validate_iso_date(&from_date)?;
        validate_iso_date(&to_date)?;
        let provider = self.provider.unwrap_or_else(|| ALL_PROVIDERS.to_owned());
        Ok((from_date, to_date, provider))
    }
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

/// One model's per-day token totals, broken out by token type. The optional
/// `billable_total` is the sum weighted by API price multipliers — present
/// when `pricing.toml` carries an entry for `(provider, model)`, absent when
/// the model is unknown to the pricing file.
#[derive(Serialize)]
pub struct ModelTokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    /// Input-token-equivalent total, ready to plot on the same axis as
    /// raw counts. `null` when pricing is unavailable for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billable_total: Option<i64>,
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
}

pub async fn daily_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageWindowParams>,
) -> Result<Json<DailyUsageResponse>, ApiError> {
    let (from_date, to_date, provider_filter) = params.resolve()?;
    let breakdown_rows =
        daily_usage_breakdown(&state.database, &from_date, &to_date, &provider_filter).await?;

    // First pass: window-level totals per model, used to (a) sort models for
    // the response and (b) drop any model whose entire window is zero —
    // Claude Code emits a `<synthetic>` model identifier for internal events
    // that never carry tokens, and surfacing it in the chart legend confuses
    // users without adding signal.
    let mut window_totals: BTreeMap<String, i64> = BTreeMap::new();
    for row in &breakdown_rows {
        let row_total = row.input_tokens
            + row.output_tokens
            + row.cache_read_tokens
            + row.cache_write_5m_tokens
            + row.cache_write_1h_tokens;
        *window_totals.entry(row.model.clone()).or_default() += row_total;
    }
    let visible_models: std::collections::HashSet<String> = window_totals
        .iter()
        .filter(|(_, &total)| total > 0)
        .map(|(name, _)| name.clone())
        .collect();

    // For Phase 1, every visible model is provider=anthropic by virtue of
    // sources.provider being filtered to anthropic when applicable. We hand
    // that to the pricing lookup. v2 will need to carry the actual provider
    // through breakdown rows.
    let provider_for_pricing = if provider_filter == ALL_PROVIDERS {
        "anthropic"
    } else {
        provider_filter.as_str()
    };

    let mut models_without_pricing: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    // Second pass: group filtered rows into the nested by-date shape, with
    // billable_total computed inline.
    let mut grouped_by_date: BTreeMap<String, BTreeMap<String, ModelTokens>> = BTreeMap::new();
    for row in breakdown_rows {
        if !visible_models.contains(&row.model) {
            continue;
        }

        let billable_total =
            state
                .pricing
                .lookup(provider_for_pricing, &row.model)
                .map(|model_pricing| {
                    let multipliers = BillableMultipliers::from_pricing(model_pricing);
                    multipliers.weight_total(
                        row.input_tokens.max(0) as u64,
                        row.output_tokens.max(0) as u64,
                        row.cache_read_tokens.max(0) as u64,
                        row.cache_write_5m_tokens.max(0) as u64,
                        row.cache_write_1h_tokens.max(0) as u64,
                    ) as i64
                });
        if billable_total.is_none() {
            models_without_pricing.insert(row.model.clone());
        }

        grouped_by_date.entry(row.date).or_default().insert(
            row.model,
            ModelTokens {
                input: row.input_tokens,
                output: row.output_tokens,
                cache_read: row.cache_read_tokens,
                cache_write_5m: row.cache_write_5m_tokens,
                cache_write_1h: row.cache_write_1h_tokens,
                billable_total,
            },
        );
    }

    let mut models: Vec<String> = visible_models.into_iter().collect();
    // Sort by raw total desc, ties broken alphabetically for determinism.
    models.sort_by(|left, right| {
        window_totals[right]
            .cmp(&window_totals[left])
            .then_with(|| left.cmp(right))
    });

    let rows = grouped_by_date
        .into_iter()
        .map(|(date, by_model)| DailyUsageRow { date, by_model })
        .collect();

    Ok(Json(DailyUsageResponse {
        rows,
        models,
        token_types: TOKEN_TYPES.to_vec(),
        models_without_pricing: models_without_pricing.into_iter().collect(),
    }))
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
    let (from_date, to_date, provider) = params.resolve()?;
    let rows = usage_by_model(&state.database, &from_date, &to_date, &provider).await?;
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
