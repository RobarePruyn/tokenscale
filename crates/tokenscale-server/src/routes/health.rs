//! `GET /api/v1/health` — server status, DB connectivity, summary counts.

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
}

/// Returns 200 OK on success. The dashboard polls this on load to confirm
/// the server is reachable and to populate the provider filter.
pub async fn handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    let summary = health_summary(&state.database).await?;
    Ok(Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        total_events: summary.total_events,
        providers: summary.providers,
    }))
}
