//! `tokenscale-core` — domain types and pure-function math.
//!
//! This crate is the schema-version-aware home for everything that does not
//! depend on storage, HTTP, or any specific ingest source:
//!
//! - `Event` — the canonical per-usage record produced by every ingest crate.
//!   Carries `(provider, model)` so factor lookup is parameterized by both,
//!   never just model — to satisfy the v2-ready architecture.
//! - `Factors` — the loaded environmental-factor model, keyed by
//!   `(provider, model)` and `region`. Honors the `schema_version`
//!   compatibility range from the factor TOML and refuses to load
//!   incompatible files.
//! - `Pricing` — versioned per-provider model pricing.
//! - `cost::*` — pure-function cost math (real and counterfactual).
//! - `impact::*` — pure-function environmental-impact math, following the
//!   Google August 2025 "comprehensive" methodology (active compute + idle +
//!   host CPU/RAM + PUE-weighted facility overhead). The "active GPU only"
//!   approach underestimates by ~2.4× per Google's own data and is rejected.
//!
//! Phase 1 ships only the types and the factor-model loader. The actual cost
//! and impact computations land in Phase 2.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
