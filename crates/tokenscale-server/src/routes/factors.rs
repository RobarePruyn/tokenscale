//! `GET /api/v1/factors/active` — surface the currently-active
//! environmental factor rows so the dashboard can render
//! per-value provenance ("which source backs this number").
//!
//! The methodology page is the *static* credibility surface
//! (general narrative, bibliography, research log). This endpoint
//! is the *dynamic* counterpart — for the specific (provider,
//! model, region) combinations in play right now, what factor row
//! is the dashboard resolving against, and what did that row
//! cite as its source?
//!
//! The endpoint reads from the in-memory `EnvironmentalFactorsFile`
//! snapshot rather than the DB, because that snapshot carries the
//! display names + free-form notes that the DB schema doesn't
//! retain. The DB has values; the in-memory snapshot has values
//! plus narrative.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ActiveFactorsResponse {
    /// Per-model environmental factor rows from the in-memory
    /// factor file. Includes ALL configured (provider, model)
    /// rows — the frontend filters to whichever models are
    /// visible in the chart.
    pub models: Vec<ModelFactorEntry>,
    /// Per-region grid factor rows. Includes all regions in the
    /// file; the frontend highlights `configured_region`.
    pub regions: Vec<GridFactorEntry>,
    /// Configured AWS region the dashboard attributes events to.
    /// Mirrors `/api/v1/health`'s `environmental.configured_region`
    /// so the frontend doesn't need a second round-trip.
    pub configured_region: String,
    /// File-level metadata for the methodology page link-out.
    pub file_version: Option<String>,
    pub methodology: Option<String>,
    pub methodology_source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelFactorEntry {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub released_at: Option<String>,
    pub valid_from: Option<String>,
    pub source_doc: Option<String>,
    pub confidence: Option<String>,
    pub uncertainty_range_pct: Option<i32>,
    pub wh_per_mtok_input: Option<f64>,
    pub wh_per_mtok_output: Option<f64>,
    pub wh_per_mtok_cache_read: Option<f64>,
    pub wh_per_mtok_cache_write_5m: Option<f64>,
    pub wh_per_mtok_cache_write_1h: Option<f64>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GridFactorEntry {
    pub region_id: String,
    pub display_name: String,
    pub valid_from: Option<String>,
    pub co2e_kg_per_kwh: Option<f64>,
    pub water_l_per_kwh: Option<f64>,
    pub pue: Option<f64>,
    pub egrid_subregion: Option<String>,
    pub egrid_subregion_full_name: Option<String>,
    pub source_url_co2e: Option<String>,
    pub source_accessed_at: Option<String>,
    pub notes: Option<String>,
}

pub async fn active_handler(
    State(state): State<AppState>,
) -> Result<Json<ActiveFactorsResponse>, ApiError> {
    let factors = &state.factors;

    let mut models = Vec::new();
    for (provider_id, provider) in &factors.providers {
        for (model_id, model) in &provider.models {
            models.push(ModelFactorEntry {
                provider: provider_id.clone(),
                model_id: model_id.clone(),
                display_name: model.display_name.clone(),
                released_at: model.released_at.clone(),
                valid_from: model.valid_from.clone(),
                source_doc: model.source_doc.clone(),
                confidence: model.confidence.clone(),
                uncertainty_range_pct: model.uncertainty_range_pct,
                wh_per_mtok_input: model.wh_per_mtok_input,
                wh_per_mtok_output: model.wh_per_mtok_output,
                wh_per_mtok_cache_read: model.wh_per_mtok_cache_read,
                wh_per_mtok_cache_write_5m: model.wh_per_mtok_cache_write_5m,
                wh_per_mtok_cache_write_1h: model.wh_per_mtok_cache_write_1h,
                notes: model.notes.clone(),
            });
        }
    }

    let mut regions = Vec::new();
    for (region_id, grid) in &factors.grid_factors {
        regions.push(GridFactorEntry {
            region_id: region_id.clone(),
            display_name: grid.display_name.clone(),
            valid_from: grid.valid_from.clone(),
            co2e_kg_per_kwh: grid.co2e_kg_per_kwh,
            water_l_per_kwh: grid.water_l_per_kwh,
            pue: grid.pue,
            egrid_subregion: grid.egrid_subregion.clone(),
            egrid_subregion_full_name: grid.egrid_subregion_full_name.clone(),
            source_url_co2e: grid.source_url_co2e.clone(),
            source_accessed_at: grid.source_accessed_at.clone(),
            notes: grid.notes.clone(),
        });
    }

    Ok(Json(ActiveFactorsResponse {
        models,
        regions,
        configured_region: state.inference_region.clone(),
        file_version: factors.file_version.clone(),
        methodology: factors.methodology.clone(),
        methodology_source: factors.methodology_source.clone(),
    }))
}
