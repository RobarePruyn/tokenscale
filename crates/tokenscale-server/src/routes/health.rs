//! `GET /api/v1/health` — server status, DB connectivity, summary counts,
//! and pricing-file status (so the dashboard can surface a "values are seed"
//! banner when `pricing.toml` has not yet been verified).

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokenscale_store::{health_summary, most_recent_scan_at};

use crate::error::ApiError;
use crate::state::AppState;

const CLAUDE_CODE_SOURCE: &str = "claude_code";

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub total_events: i64,
    pub providers: Vec<String>,
    pub pricing: PricingStatus,
    pub environmental: EnvironmentalStatus,
    pub ingest: IngestStatus,
}

#[derive(Serialize)]
pub struct IngestStatus {
    /// MAX(_ingest_file_state.last_scanned_at) for the Claude Code
    /// source, ISO-8601 UTC. None until the first scan completes.
    /// Drives the dashboard banner's "Last ingested: N seconds ago".
    pub last_scanned_at: Option<DateTime<Utc>>,
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
    /// Human-readable file version — e.g. `"0.1"`. Surfaced as
    /// "Environmental factors v0.1" in the dashboard banner.
    pub file_version: Option<String>,
    /// ISO date the file was published. Pairs with `file_version` so
    /// the banner can show "v0.1, 2026-04-28".
    pub file_published: Option<String>,
    /// Methodology identifier — e.g. `"google-comprehensive-aug-2025"`.
    /// Drives the methodology page's title and copy.
    pub methodology: Option<String>,
    /// URL the methodology is sourced from. The methodology page
    /// links here so users can verify the math against the original.
    pub methodology_source: Option<String>,
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
    /// Configured AWS region used to attribute grid factors to events.
    /// Comes from `[inference].default_inference_region` in the user's
    /// config (default `"us-east-1"`).
    pub configured_region: String,
    /// EPA eGRID subregion code for the configured region — e.g.
    /// `"SRVC"` (us-east-1). Sourced from the in-memory factor file's
    /// grid entry. `None` when the region isn't in the file.
    pub configured_region_egrid_subregion: Option<String>,
    /// Human-readable expansion — e.g. `"SERC Virginia/Carolina"`.
    pub configured_region_egrid_subregion_full_name: Option<String>,
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    let summary = health_summary(&state.database).await?;
    let last_scanned_at = most_recent_scan_at(&state.database, CLAUDE_CODE_SOURCE).await?;
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
            file_version: factors.file_version.clone(),
            file_published: factors.file_published.clone(),
            methodology: factors.methodology.clone(),
            methodology_source: factors.methodology_source.clone(),
            model_count: factors.model_count(),
            region_count: factors.region_count(),
            is_placeholder: factors.is_placeholder(),
            needs_review: factors.is_review_pending(),
            accessed_at: factors.most_recent_grid_accessed_at().map(str::to_owned),
            configured_region: state.inference_region.clone(),
            configured_region_egrid_subregion: factors
                .grid_factors
                .get(&state.inference_region)
                .and_then(|grid| grid.egrid_subregion.clone()),
            configured_region_egrid_subregion_full_name: factors
                .grid_factors
                .get(&state.inference_region)
                .and_then(|grid| grid.egrid_subregion_full_name.clone()),
        },
        ingest: IngestStatus { last_scanned_at },
    }))
}
