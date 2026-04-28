//! `tokenscale-server` — axum HTTP server.
//!
//! Exposes the tokenscale REST API and serves the embedded React dashboard.
//!
//! Phase 1 endpoints (sufficient for one chart in the dashboard):
//!
//! - `GET /api/v1/usage/daily?from=&to=&provider=`
//! - `GET /api/v1/usage/by-model?from=&to=&provider=`
//! - `GET /api/v1/sessions/recent?limit=`
//!
//! The `provider` query parameter accepts `all` (default) or a specific
//! provider slug. Even though v1 has only `anthropic`, the parameter is
//! present from day one so the API surface does not change in v2.
//!
//! Static-asset serving: production builds embed `frontend/dist/` into the
//! binary at compile time via `rust-embed`. The wiring is added in the
//! rust-embed integration step; in the scaffold commit the server is
//! API-only.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
