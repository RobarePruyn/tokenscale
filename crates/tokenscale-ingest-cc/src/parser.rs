//! JSONL line parser.
//!
//! Each line in a Claude Code session log is a JSON object whose top-level
//! `type` field describes its kind: `assistant`, `user`, `queue-operation`,
//! `file-history-snapshot`, `attachment`, `ai-title`, `last-prompt`,
//! and a few internal Claude Code variants. **Only `assistant` lines carry
//! token usage**, so the parser short-circuits everything else.
//!
//! Schema-drift tolerance:
//!
//! - Unknown JSON fields are ignored.
//! - Optional fields default sensibly.
//! - Missing required fields on an `assistant` line skip the line as
//!   `Malformed`; the scan continues.
//!
//! For lines without a `requestId` (~3-6 per session in observed data —
//! always API-error lines), the parser emits an `Event` with
//! `request_id = None` and a SHA-256 `content_hash` over a deterministic
//! projection of the row, so the database's partial unique index on
//! `(source, content_hash)` can dedupe re-scans.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokenscale_core::Event;

const SOURCE_KIND: &str = "claude_code";

/// What happened to one JSONL line.
#[derive(Debug)]
pub enum ParseOutcome {
    /// Not an `assistant` line, or an `assistant` line with no usage to
    /// record. Counted but otherwise unremarkable.
    Skip,
    /// Successfully parsed assistant turn.
    Event(Box<Event>),
    /// JSON parse failed or required fields absent. Logged and counted; the
    /// scan continues.
    Malformed { reason: String },
}

/// Top-level enum mirroring the JSONL line type. We use serde's internally
/// tagged representation: the `type` field is the discriminant, and the
/// rest of the line's fields are forwarded into the variant payload.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum JsonlLine {
    #[serde(rename = "assistant")]
    Assistant(Box<AssistantPayload>),
    /// Any other line type — `user`, `queue-operation`, `attachment`, etc.
    /// We don't need the data; the unit catches all non-assistant variants.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AssistantPayload {
    timestamp: DateTime<Utc>,

    /// Anthropic's request_id when the call succeeded; absent on API-error
    /// lines.
    #[serde(rename = "requestId", default)]
    request_id: Option<String>,

    #[serde(rename = "sessionId", default)]
    session_id: Option<String>,

    /// The shell working directory at call time. Used as the human-readable
    /// `project_id` (more legible than the slug-encoded directory name in
    /// `~/.claude/projects/`).
    #[serde(default)]
    cwd: Option<String>,

    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    /// Model identifier — e.g., `claude-opus-4-7`.
    model: String,
    /// Token-usage subobject. The `default` here means a missing `usage`
    /// is treated as zero across the board, which can happen for rare
    /// non-error / non-success edge cases.
    #[serde(default)]
    usage: AssistantUsage,
}

/// Mirrors the Anthropic API `usage` object as Claude Code persists it.
/// Field names match the API's JSON keys exactly.
#[derive(Debug, Default, Deserialize)]
struct AssistantUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,

    /// Total cache-creation tokens. When `cache_creation` (the structured
    /// sub-object) is also present, the breakdown there is authoritative;
    /// otherwise this total is attributed to 5-minute cache by convention.
    #[serde(default)]
    cache_creation_input_tokens: u64,

    #[serde(default)]
    cache_creation: Option<CacheCreationBreakdown>,
}

#[derive(Debug, Default, Deserialize)]
struct CacheCreationBreakdown {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

/// Parse a single JSONL line into a `ParseOutcome`. Pure function — no I/O.
//
// `cache_write_5m` and `cache_write_1h` are domain-meaningful names (the two
// Anthropic prompt-cache classes); the local `similar_names` lint would have
// us rename them to something less clear, so it's silenced here.
#[allow(clippy::similar_names)]
pub fn parse_line(raw_line: &str, capture_raw: bool) -> ParseOutcome {
    if raw_line.trim().is_empty() {
        return ParseOutcome::Skip;
    }

    let line: JsonlLine = match serde_json::from_str(raw_line) {
        Ok(parsed) => parsed,
        Err(serde_error) => {
            return ParseOutcome::Malformed {
                reason: format!("json: {serde_error}"),
            };
        }
    };

    let assistant = match line {
        JsonlLine::Assistant(payload) => payload,
        JsonlLine::Other => return ParseOutcome::Skip,
    };

    let (cache_write_5m, cache_write_1h) = match assistant.message.usage.cache_creation {
        Some(breakdown) => (
            breakdown.ephemeral_5m_input_tokens,
            breakdown.ephemeral_1h_input_tokens,
        ),
        None => {
            // Older Claude Code versions don't break out the 5m/1h split.
            // Attribute the total to 5m (the API's default cache class) so
            // the totals reconcile, and log a per-line debug to help spot
            // these in the wild.
            (assistant.message.usage.cache_creation_input_tokens, 0)
        }
    };

    let mut event = Event {
        source: SOURCE_KIND.to_owned(),
        occurred_at: assistant.timestamp,
        model: assistant.message.model,
        input_tokens: assistant.message.usage.input_tokens,
        output_tokens: assistant.message.usage.output_tokens,
        cache_read_tokens: assistant.message.usage.cache_read_input_tokens,
        cache_write_5m_tokens: cache_write_5m,
        cache_write_1h_tokens: cache_write_1h,
        request_id: assistant.request_id,
        content_hash: None,
        session_id: assistant.session_id,
        project_id: assistant.cwd,
        workspace_id: None,
        api_key_id: None,
        raw: capture_raw.then(|| raw_line.to_owned()),
    };

    // Fall back to a content hash when the source did not give us a
    // request_id. Computed *after* the rest of the event is filled in so
    // the projection covers every field that uniquely identifies the row.
    if event.request_id.is_none() {
        event.content_hash = Some(compute_content_hash(&event));
    }

    ParseOutcome::Event(Box::new(event))
}

/// Deterministic SHA-256 over the fields that uniquely identify an event
/// when the source does not give us a request_id. The projection MUST be
/// stable across releases: it goes into a unique index, so changing the
/// hashed shape would silently break dedupe on re-scan.
fn compute_content_hash(event: &Event) -> String {
    let projection = format!(
        "{ts}|{model}|{in_tok}|{out_tok}|{cr_tok}|{cw5_tok}|{cw1_tok}|{session}|{project}",
        ts = event
            .occurred_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        model = event.model,
        in_tok = event.input_tokens,
        out_tok = event.output_tokens,
        cr_tok = event.cache_read_tokens,
        cw5_tok = event.cache_write_5m_tokens,
        cw1_tok = event.cache_write_1h_tokens,
        session = event.session_id.as_deref().unwrap_or(""),
        project = event.project_id.as_deref().unwrap_or(""),
    );
    let mut hasher = Sha256::new();
    hasher.update(projection.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One realistic assistant line from the live data — token counts
    /// match the QTrial session inspected during Phase B kickoff.
    const ASSISTANT_LINE: &str = r#"{"parentUuid":"9a27c40f","isSidechain":false,"message":{"model":"claude-opus-4-7","id":"msg_01UwNt","type":"message","role":"assistant","content":[],"stop_reason":"tool_use","usage":{"input_tokens":6,"cache_creation_input_tokens":8837,"cache_read_input_tokens":16410,"output_tokens":136,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":8837},"service_tier":"standard"}},"requestId":"req_011CaFyK","type":"assistant","uuid":"db6baab1","timestamp":"2026-04-21T00:29:54.704Z","sessionId":"455218e7","cwd":"/Users/r/Dev/QTrial","version":"2.1.114","userType":"external","entrypoint":"claude-vscode","gitBranch":"main"}"#;

    const USER_LINE: &str =
        r#"{"type":"user","timestamp":"2026-04-21T00:29:50.000Z","content":"hello"}"#;

    const QUEUE_OP_LINE: &str = r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-04-21T00:29:52.508Z","sessionId":"455218e7"}"#;

    const ERROR_LINE_NO_REQUEST_ID: &str = r#"{"parentUuid":"x","isSidechain":false,"message":{"model":"claude-opus-4-7","id":"msg_err","type":"message","role":"assistant","content":[],"stop_reason":"error","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}},"type":"assistant","uuid":"y","timestamp":"2026-04-21T00:30:00.000Z","sessionId":"455218e7","cwd":"/tmp/proj","version":"2.1.120","userType":"external","entrypoint":"claude-vscode","gitBranch":"main","isApiErrorMessage":true,"error":"overloaded"}"#;

    #[test]
    fn assistant_line_parses_with_full_token_breakdown() {
        let outcome = parse_line(ASSISTANT_LINE, false);
        let ParseOutcome::Event(event) = outcome else {
            panic!("expected Event, got {outcome:?}");
        };
        assert_eq!(event.source, "claude_code");
        assert_eq!(event.model, "claude-opus-4-7");
        assert_eq!(event.input_tokens, 6);
        assert_eq!(event.output_tokens, 136);
        assert_eq!(event.cache_read_tokens, 16_410);
        assert_eq!(event.cache_write_5m_tokens, 0);
        assert_eq!(event.cache_write_1h_tokens, 8_837);
        assert_eq!(event.request_id.as_deref(), Some("req_011CaFyK"));
        assert!(event.content_hash.is_none());
        assert_eq!(event.session_id.as_deref(), Some("455218e7"));
        assert_eq!(event.project_id.as_deref(), Some("/Users/r/Dev/QTrial"));
        assert!(event.raw.is_none()); // capture_raw=false
    }

    #[test]
    fn assistant_line_with_capture_raw_stores_payload() {
        let ParseOutcome::Event(event) = parse_line(ASSISTANT_LINE, true) else {
            panic!("expected Event");
        };
        assert_eq!(event.raw.as_deref(), Some(ASSISTANT_LINE));
    }

    #[test]
    fn user_line_is_skipped() {
        assert!(matches!(parse_line(USER_LINE, false), ParseOutcome::Skip));
    }

    #[test]
    fn queue_operation_line_is_skipped() {
        assert!(matches!(
            parse_line(QUEUE_OP_LINE, false),
            ParseOutcome::Skip
        ));
    }

    #[test]
    fn empty_line_is_skipped() {
        assert!(matches!(parse_line("", false), ParseOutcome::Skip));
        assert!(matches!(parse_line("   \n", false), ParseOutcome::Skip));
    }

    #[test]
    fn malformed_json_returns_malformed() {
        let outcome = parse_line("{not json", false);
        assert!(matches!(outcome, ParseOutcome::Malformed { .. }));
    }

    #[test]
    fn assistant_missing_required_field_is_malformed() {
        // No `timestamp`, no `message` — required for AssistantPayload.
        let outcome = parse_line(r#"{"type":"assistant"}"#, false);
        assert!(matches!(outcome, ParseOutcome::Malformed { .. }));
    }

    #[test]
    fn error_line_without_request_id_gets_content_hash() {
        let ParseOutcome::Event(event) = parse_line(ERROR_LINE_NO_REQUEST_ID, false) else {
            panic!("expected Event");
        };
        assert!(event.request_id.is_none());
        assert!(event.content_hash.is_some());
        // SHA-256 hex output is 64 chars
        assert_eq!(event.content_hash.as_deref().unwrap().len(), 64);
    }

    #[test]
    fn content_hash_is_deterministic() {
        let ParseOutcome::Event(first) = parse_line(ERROR_LINE_NO_REQUEST_ID, false) else {
            panic!()
        };
        let ParseOutcome::Event(second) = parse_line(ERROR_LINE_NO_REQUEST_ID, false) else {
            panic!()
        };
        assert_eq!(first.content_hash, second.content_hash);
    }

    #[test]
    fn unknown_top_level_fields_are_ignored() {
        // Add a `surprise: "field"` to a normal assistant line; it should
        // still parse cleanly. This is the schema-drift-tolerance contract.
        let drifted = ASSISTANT_LINE.replace(
            r#""type":"assistant""#,
            r#""type":"assistant","surprise":"new field appearing in v2.99""#,
        );
        let outcome = parse_line(&drifted, false);
        assert!(matches!(outcome, ParseOutcome::Event(_)));
    }

    #[test]
    fn old_format_without_cache_creation_breakdown_attributes_to_5m() {
        // 2.1.92-style line — `cache_creation_input_tokens` present but
        // `cache_creation` sub-object absent. We attribute to 5m by
        // convention so totals still reconcile.
        let old_format = r#"{"type":"assistant","timestamp":"2026-04-21T00:29:54.704Z","sessionId":"s","cwd":"/p","message":{"model":"claude-opus-4-7","id":"m","type":"message","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}},"requestId":"req_x"}"#;
        let ParseOutcome::Event(event) = parse_line(old_format, false) else {
            panic!()
        };
        assert_eq!(event.cache_write_5m_tokens, 4);
        assert_eq!(event.cache_write_1h_tokens, 0);
    }
}
