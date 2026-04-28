//! `tokenscale-ingest-api` — Anthropic Admin API ingester. **Phase 2 — placeholder.**
//!
//! When implemented, this crate will pull from:
//!
//!   * `GET /v1/organizations/usage_report/messages`
//!   * `GET /v1/organizations/cost_report`
//!
//! using an Admin API key (distinct from a regular API key). The Admin API
//! exposes per-request `request_id`, `workspace_id`, and `api_key_id` fields,
//! all of which round-trip into `events`.
//!
//! Phase 1 leaves this crate as a placeholder so the workspace member list and
//! dependency graph are stable from day one — adding the implementation later
//! is purely additive.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
