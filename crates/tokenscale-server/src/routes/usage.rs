//! `GET /api/v1/usage/daily`  and `GET /api/v1/usage/by-model` handlers.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokenscale_core::{BillableMultipliers, ModelPricing};
use tokenscale_store::{
    daily_usage_breakdown, usage_by_model, DailyUsageBreakdownRow, ALL_PROVIDERS,
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

        Ok(ResolvedParams {
            from_date,
            to_date,
            provider,
            projects,
        })
    }
}

struct ResolvedParams {
    from_date: String,
    to_date: String,
    provider: String,
    projects: Vec<String>,
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
}

pub async fn daily_handler(
    State(state): State<AppState>,
    Query(params): Query<UsageWindowParams>,
) -> Result<Json<DailyUsageResponse>, ApiError> {
    let resolved = params.resolve()?;
    let breakdown_rows = daily_usage_breakdown(
        &state.database,
        &resolved.from_date,
        &resolved.to_date,
        &resolved.provider,
        &resolved.projects,
    )
    .await?;

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
    let provider_for_pricing = if resolved.provider == ALL_PROVIDERS {
        "anthropic"
    } else {
        resolved.provider.as_str()
    };

    let mut models_without_pricing: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    // Second pass: group filtered rows into the nested by-date shape, with
    // billable breakdown + total computed inline.
    let mut grouped_by_date: BTreeMap<String, BTreeMap<String, ModelTokens>> = BTreeMap::new();
    for row in breakdown_rows {
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

        grouped_by_date.entry(row.date).or_default().insert(
            row.model,
            ModelTokens {
                input: row.input_tokens,
                output: row.output_tokens,
                cache_read: row.cache_read_tokens,
                cache_write_5m: row.cache_write_5m_tokens,
                cache_write_1h: row.cache_write_1h_tokens,
                billable,
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
    row: &DailyUsageBreakdownRow,
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
