//! `GET /api/v1/health` — server status, DB connectivity, summary counts,
//! and pricing-file status (so the dashboard can surface a "values are seed"
//! banner when `pricing.toml` has not yet been verified).

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use tokenscale_store::health_summary;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub total_events: i64,
    pub providers: Vec<String>,
    pub pricing: PricingStatus,
    pub environmental: EnvironmentalStatus,
}

#[derive(Serialize)]
pub struct PricingStatus {
    pub schema_version: i64,
    pub file_status: String,
    /// Number of (provider, model) entries currently loaded.
    pub model_count: usize,
    /// `true` if `file_status != "production"` — drives the dashboard's
    /// review-pending banner.
    pub needs_review: bool,
    /// Most recent `source_accessed_at` across the loaded models. The
    /// dashboard surfaces this so users know the values are accurate as
    /// of a specific date, not just "needs review."
    pub accessed_at: Option<String>,
}

#[derive(Serialize)]
pub struct EnvironmentalStatus {
    pub schema_version: i64,
    pub file_status: String,
    /// Number of (provider, model) factor entries currently loaded.
    pub model_count: usize,
    /// Number of grid (region) factor entries currently loaded.
    pub region_count: usize,
    /// `true` when the file is the Phase 1 placeholder (every numeric
    /// value `null`). The Phase 2 environmental view will key off this
    /// to render "factor data unavailable" until Cowork's deliverable 3
    /// merges real values.
    pub is_placeholder: bool,
    /// `true` if `file_status != "production"`.
    pub needs_review: bool,
    /// Most recent `source_accessed_at` across loaded grid factors.
    pub accessed_at: Option<String>,
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    let summary = health_summary(&state.database).await?;
    let pricing = &state.pricing;
    let factors = &state.factors;
    let pricing_model_count = pricing
        .providers
        .values()
        .map(|provider| provider.models.len())
        .sum();
    Ok(Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        total_events: summary.total_events,
        providers: summary.providers,
        pricing: PricingStatus {
            schema_version: pricing.schema_version,
            file_status: pricing.file_status.clone(),
            model_count: pricing_model_count,
            needs_review: pricing.is_review_pending(),
            accessed_at: pricing.most_recent_accessed_at().map(str::to_owned),
        },
        environmental: EnvironmentalStatus {
            schema_version: factors.schema_version,
            file_status: factors.file_status.clone(),
            model_count: factors.model_count(),
            region_count: factors.region_count(),
            is_placeholder: factors.is_placeholder(),
            needs_review: factors.is_review_pending(),
            accessed_at: factors.most_recent_grid_accessed_at().map(str::to_owned),
        },
    }))
}
