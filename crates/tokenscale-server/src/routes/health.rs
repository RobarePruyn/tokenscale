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
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    let summary = health_summary(&state.database).await?;
    let pricing = &state.pricing;
    let model_count = pricing
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
            model_count,
            needs_review: pricing.is_review_pending(),
        },
    }))
}
