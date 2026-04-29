//! `tokenscale-core` — domain types and pure-function math.
//!
//! This crate is the schema-version-aware home for everything that does not
//! depend on storage, HTTP, or any specific ingest source:
//!
//! - `Event` — the canonical per-usage record produced by every ingest crate.
//!   Carries `(provider, model)` so factor lookup is parameterized by both,
//!   never just model — to satisfy the v2-ready architecture.
//! - `Factors` (Phase 2) — the loaded environmental-factor model, keyed by
//!   `(provider, model)` and `region`. Honors the `schema_version`
//!   compatibility range from the factor TOML and refuses to load
//!   incompatible files.
//! - `Pricing` (Phase 2) — versioned per-provider model pricing.
//! - `cost::*` (Phase 2) — pure-function cost math (real and counterfactual).
//! - `impact::*` (Phase 2) — pure-function environmental-impact math,
//!   following the Google August 2025 "comprehensive" methodology (active
//!   compute + idle + host CPU/RAM + PUE-weighted facility overhead). The
//!   "active GPU only" approach underestimates by ~2.4× per Google's own
//!   data and is rejected.
//!
//! Phase 1 ships the `Event` type and the error type. The factor-model
//! loader and computations land in Phase 2.

mod error;
mod event;

pub use error::{CoreError, Result};
pub use event::{Event, SourceKind};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_event() -> Event {
        Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 29, 54).unwrap(),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: 6,
            output_tokens: 136,
            cache_read_tokens: 16_410,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 8_837,
            request_id: Some("req_011CaFyK1b4pQUFLfXGAuJbw".to_owned()),
            content_hash: None,
            session_id: Some("455218e7-8747-410f-a4f3-11bf11c53cc6".to_owned()),
            project_id: Some(
                "/Users/Robare/Library/Mobile Documents/com~apple~CloudDocs/Dev/QTrial".to_owned(),
            ),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        }
    }

    #[test]
    fn total_tokens_sums_all_token_types() {
        let event = sample_event();
        // 6 input + 136 output + 16_410 cache_read + 0 cache_5m + 8_837 cache_1h
        assert_eq!(event.total_tokens(), 25_389);
    }

    #[test]
    fn has_dedupe_key_is_true_when_request_id_present() {
        let event = sample_event();
        assert!(event.has_dedupe_key());
    }

    #[test]
    fn has_dedupe_key_is_true_when_only_content_hash_present() {
        let mut event = sample_event();
        event.request_id = None;
        event.content_hash = Some("dead-beef".to_owned());
        assert!(event.has_dedupe_key());
    }

    #[test]
    fn has_dedupe_key_is_false_when_neither_present() {
        let mut event = sample_event();
        event.request_id = None;
        event.content_hash = None;
        assert!(!event.has_dedupe_key());
    }

    #[test]
    fn source_kind_str_matches_database_seed() {
        assert_eq!(SourceKind::ClaudeCode.as_str(), "claude_code");
        assert_eq!(SourceKind::AdminApi.as_str(), "admin_api");
    }

    use chrono::Utc;
}
