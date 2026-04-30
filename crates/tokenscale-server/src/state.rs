//! Shared application state passed through axum's `State` extractor.
//!
//! Keeping this in its own module prevents callers from reaching into
//! lib.rs to add ad-hoc fields; route handlers should accept `AppState`
//! rather than individual dependencies, so adding a new dependency is a
//! one-place change.

use std::sync::Arc;
use tokenscale_core::{EnvironmentalFactorsFile, PricingFile};
use tokenscale_store::Database;

#[derive(Clone)]
pub struct AppState {
    pub database: Database,
    /// Pricing snapshot loaded from `pricing.toml` at startup. `Arc` so the
    /// state is cheap to clone for every request, and so the snapshot can be
    /// hot-swapped later (Phase 3 reload-on-SIGHUP) without breaking
    /// in-flight handlers.
    pub pricing: Arc<PricingFile>,
    /// Environmental-factor snapshot loaded from `environmental-factors.toml`
    /// at startup. The factor *values* live in the DB (synced on boot); this
    /// snapshot carries the file-level metadata (methodology, file version,
    /// `[defaults]` fallbacks, display names) that the dashboard reads.
    pub factors: Arc<EnvironmentalFactorsFile>,
    /// Configured AWS region used as the grid-factor lookup key for every
    /// event. Derived from `[inference].default_inference_region` (with
    /// back-compat for the legacy top-level field). Anthropic does not
    /// disclose which region served any given request, so this is a declared
    /// user assumption — surfaced in the dashboard's environmental banner.
    pub inference_region: String,
}

impl AppState {
    #[must_use]
    pub fn new(
        database: Database,
        pricing: Arc<PricingFile>,
        factors: Arc<EnvironmentalFactorsFile>,
        inference_region: String,
    ) -> Self {
        Self {
            database,
            pricing,
            factors,
            inference_region,
        }
    }
}
