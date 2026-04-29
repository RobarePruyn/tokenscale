//! `Event` — the canonical per-usage record produced by every ingest crate.
//!
//! An `Event` is provider-agnostic: ingest-cc produces them from local Claude
//! Code JSONL, ingest-api produces them from the Anthropic Admin API, and any
//! v2 provider crate (OpenAI, Bedrock, …) produces them from its own source.
//! All downstream computation — cost, environmental impact, time-series
//! aggregation — operates on these.
//!
//! The `source` field carries the ingest-source `kind`, which the database
//! foreign-keys to the `sources` table to recover provider, display name, and
//! enabled-state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One usage event.
///
/// Fields map 1:1 to the `events` table columns. See
/// `migrations/20260428000001_initial.sql`.
///
/// Idempotency: an `Event` with `request_id = Some(_)` is deduped on
/// `(source, request_id)`. An `Event` with `request_id = None` MUST carry a
/// `content_hash` so the partial-unique-index dedupe can take effect; the
/// ingest layer is responsible for computing that hash before insert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Foreign key to `sources.kind`. e.g., `"claude_code"`, `"admin_api"`.
    pub source: String,

    /// When the underlying API call occurred. UTC, ISO-8601 on the wire.
    pub occurred_at: DateTime<Utc>,

    /// Model identifier as reported by the source. e.g., `"claude-opus-4-7"`.
    pub model: String,

    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,

    /// Stable request identifier when the source provides one (e.g., the
    /// Anthropic `request_id`). When present, this is the dedupe key.
    pub request_id: Option<String>,

    /// SHA-256 over a deterministic projection of the row, used as the
    /// dedupe key when `request_id` is unavailable. The ingest layer fills
    /// this in; consumers should treat it as opaque.
    pub content_hash: Option<String>,

    pub session_id: Option<String>,

    /// Human-readable project identifier — for Claude Code this is the `cwd`
    /// at the time of the call, not the directory-name slug.
    pub project_id: Option<String>,

    /// Admin API only — workspace UUID, when provided.
    pub workspace_id: Option<String>,

    /// Admin API only — API key UUID, when provided.
    pub api_key_id: Option<String>,

    /// Original payload as a JSON string. The ingester populates this only
    /// when `ingest.store_raw = true`. Useful for debugging schema drift and
    /// for re-deriving fields the parser missed; trades disk for diagnostics.
    /// See README "Privacy" for the data-sensitivity discussion.
    pub raw: Option<String>,
}

impl Event {
    /// Total tokens across all token types. Useful for top-line aggregations
    /// in the dashboard.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_5m_tokens
            + self.cache_write_1h_tokens
    }

    /// True if this event has no dedupe key set. Useful in tests; an event
    /// without either a `request_id` or a `content_hash` would silently
    /// duplicate on re-ingest, so the ingest layer must reject it.
    #[must_use]
    pub fn has_dedupe_key(&self) -> bool {
        self.request_id.is_some() || self.content_hash.is_some()
    }
}

/// Ingest-source identifiers known at v1. Adding a v2 source is a row in
/// `sources`, not a variant here — this enum is for the ingest crates'
/// convenience, not as a database constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ClaudeCode,
    AdminApi,
}

impl SourceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::AdminApi => "admin_api",
        }
    }
}
