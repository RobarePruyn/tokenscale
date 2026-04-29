//! `GET /api/v1/usage/daily`  and `GET /api/v1/usage/by-model` handlers.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokenscale_store::{daily_usage, usage_by_model, ALL_PROVIDERS};

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_WINDOW_DAYS: i64 = 30;

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
// daily — stacked-by-model token totals per day
// ----------------------------------------------------------------------------

/// One row per UTC date, with a `byModel` dict so the frontend's stacked
/// area chart can plug it in directly without re-shaping client-side.
#[derive(Serialize)]
pub struct DailyUsageRow {
    pub date: String,
    #[serde(rename = "byModel")]
    pub by_model: BTreeMap<String, i64>,
}

#[derive(Serialize)]
pub struct DailyUsageResponse {
    pub rows: Vec<DailyUsageRow>,
    /// Distinct model identifiers across the window, sorted by total tokens
    /// descending so the chart's stack order is stable and biggest-first.
    pub models: Vec<String>,
}

pub async fn daily_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageWindowParams>,
) -> Result<Json<DailyUsageResponse>, ApiError> {
    let (from_date, to_date, provider) = params.resolve()?;
    let flat_rows = daily_usage(&state.database, &from_date, &to_date, &provider).await?;

    // Group flat (date, model, tokens) into the nested by-date shape.
    let mut grouped_by_date: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    let mut model_totals: BTreeMap<String, i64> = BTreeMap::new();
    for row in flat_rows {
        *model_totals.entry(row.model.clone()).or_default() += row.total_tokens;
        grouped_by_date
            .entry(row.date)
            .or_default()
            .insert(row.model, row.total_tokens);
    }

    let mut models: Vec<String> = model_totals.keys().cloned().collect();
    // Sort by total desc, ties broken alphabetically for determinism.
    models.sort_by(|left, right| {
        model_totals[right]
            .cmp(&model_totals[left])
            .then_with(|| left.cmp(right))
    });

    let rows = grouped_by_date
        .into_iter()
        .map(|(date, by_model)| DailyUsageRow { date, by_model })
        .collect();

    Ok(Json(DailyUsageResponse { rows, models }))
}

// ----------------------------------------------------------------------------
// by-model — totals across the window
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
