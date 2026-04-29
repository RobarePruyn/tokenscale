//! Shared application state passed through axum's `State` extractor.
//!
//! Keeping this in its own module prevents callers from reaching into
//! lib.rs to add ad-hoc fields; route handlers should accept `AppState`
//! rather than individual dependencies, so adding a new dependency is a
//! one-place change.

use tokenscale_store::Database;

#[derive(Clone)]
pub struct AppState {
    pub database: Database,
}

impl AppState {
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }
}
