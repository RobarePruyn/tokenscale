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
    /// at startup. Phase 1 only exposes the snapshot's status via
    /// `/api/v1/health`; the per-event impact computation that consumes the
    /// values is Phase 2.
    pub factors: Arc<EnvironmentalFactorsFile>,
}

impl AppState {
    #[must_use]
    pub fn new(
        database: Database,
        pricing: Arc<PricingFile>,
        factors: Arc<EnvironmentalFactorsFile>,
    ) -> Self {
        Self {
            database,
            pricing,
            factors,
        }
    }
}
