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
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokenscale_core::{Event, FileSnapshot, ToolResult, ToolUse};
use tracing::debug;

const SOURCE_KIND: &str = "claude_code";

/// Records produced from a single JSONL line. v0.1.17 / Phase 1.5:
/// the parser fans out — one line can produce an Event (assistant
/// lines), plus auxiliary tool-use / tool-result / file-snapshot
/// records depending on the line type. Per the D2 sign-off
/// (one-to-many at parse).
#[derive(Debug, Default)]
pub struct ParsedRecords {
    /// `Some` only for assistant lines. User lines and
    /// file-history-snapshot lines never produce an Event.
    pub event: Option<Box<Event>>,
    /// Populated when an assistant message's content array contains
    /// one or more `tool_use` blocks. p99 = 1 per Phase 1.5 §1, max
    /// observed = 3.
    pub tool_uses: Vec<ToolUse>,
    /// Populated when a user message's content array contains one or
    /// more `tool_result` blocks. Linked to `tool_uses` by
    /// `tool_use_id` at query time (D3 sign-off).
    pub tool_results: Vec<ToolResult>,
    /// Populated for `file-history-snapshot` lines. Each snapshot's
    /// `trackedFileBackups` dict produces zero or many rows.
    pub file_snapshots: Vec<FileSnapshot>,
}

impl ParsedRecords {
    fn is_empty(&self) -> bool {
        self.event.is_none()
            && self.tool_uses.is_empty()
            && self.tool_results.is_empty()
            && self.file_snapshots.is_empty()
    }
}

/// What happened to one JSONL line.
#[derive(Debug)]
pub enum ParseOutcome {
    /// Nothing of interest in this line (e.g., a `queue-operation`
    /// record, or a user message with no tool_result blocks).
    Skip,
    /// Successfully parsed at least one record. The records may
    /// include an Event (assistant lines), tool_uses, tool_results,
    /// and/or file_snapshots — whichever applied to this line type.
    Records(Box<ParsedRecords>),
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
    /// User lines carry tool_result blocks (v0.1.17 / Phase 1.5).
    /// Previously dropped via the `Other` catch-all.
    #[serde(rename = "user")]
    User(Box<UserPayload>),
    /// File-history-snapshot lines carry file-edit tracking
    /// (v0.1.17 / Phase 1.5). Previously dropped.
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot(Box<FileHistorySnapshotPayload>),
    /// Any other line type — `queue-operation`, `attachment`,
    /// `last-prompt`, `ai-title`, `pr-link`, `system`. Not gated by
    /// Phase 1.5; keep dropping.
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

    /// v0.1.16: per-message UUID. Present in every observed assistant
    /// line (Phase 0 sample: 27,388/27,388), but `Option` for
    /// schema-drift tolerance. Drives the new UNIQUE partial index
    /// `events_source_uuid_unique` as the actual defense against
    /// same-message-different-requestId duplicates.
    #[serde(default)]
    uuid: Option<String>,

    /// v0.1.16: the preceding turn's UUID in this session. `None`
    /// on the first turn. Future per-thread reconstruction.
    #[serde(rename = "parentUuid", default)]
    parent_uuid: Option<String>,

    /// v0.1.16: git branch active in the session's cwd at call time.
    /// `None` when the cwd isn't in a git repo.
    #[serde(rename = "gitBranch", default)]
    git_branch: Option<String>,

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
    /// v0.1.17 / Phase 1.5: the content array can contain `text`,
    /// `tool_use`, and other block types. We only extract `tool_use`;
    /// `text` blocks are dropped (they're the assistant's prose
    /// response, not gating any phase).
    #[serde(default)]
    content: Vec<MessageBlock>,
}

/// User-message payload (v0.1.17). User lines carry tool_result blocks
/// inside their content array; we extract those and emit per-result
/// ToolResult records. User lines without tool_result blocks produce
/// `ParseOutcome::Skip`.
///
/// `message` defaults to empty so simplified or pre-Phase-1.5
/// user-line shapes (`{"type":"user","timestamp":"…","content":"hi"}`)
/// degrade to Skip rather than Malformed — schema-drift tolerance,
/// same posture as the assistant `AssistantUsage::default()`.
#[derive(Debug, Deserialize)]
struct UserPayload {
    timestamp: DateTime<Utc>,
    #[serde(rename = "sessionId", default)]
    session_id: Option<String>,
    /// User-message uuid — joins back to the user's parent_event_uuid
    /// on the tool_result rows. Captured but Event is not emitted
    /// for user lines (cost / impact aggregation is per-assistant-
    /// turn).
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    message: UserMessage,
}

#[derive(Debug, Default, Deserialize)]
struct UserMessage {
    /// Real CC data has user message `content` in two shapes:
    /// (1) a plain string for text-only user input (e.g.,
    /// `<task-notification>` blocks the user pastes), and (2) an
    /// array of typed blocks when carrying tool_result responses.
    /// Untagged enum handles both; Vec is the only shape that
    /// produces tool_result rows.
    ///
    /// Smoke-test-surfaced bug fix during v0.1.17 build: my initial
    /// `Vec<MessageBlock>` was too strict and turned 42 real user
    /// lines into ParseOutcome::Malformed against the maintainer's
    /// real DB (was 0 malformed in v0.1.16). The dual-shape tolerance
    /// brings the count back to 0.
    #[serde(default)]
    content: UserContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UserContent {
    // String body matches CC's "text-only user input" shape; we read
    // it off the wire to satisfy serde's untagged dispatch but never
    // use the value (no tool_result extractable from a string).
    Text(#[allow(dead_code)] String),
    Blocks(Vec<MessageBlock>),
}

impl Default for UserContent {
    fn default() -> Self {
        UserContent::Blocks(Vec::new())
    }
}

/// File-history-snapshot payload (v0.1.17). One row per file in the
/// snapshot's `trackedFileBackups` dict — the dict can be empty (most
/// common — many snapshots track nothing), one file, or many.
///
/// All fields default to allow schema-drift tolerance: pre-Phase-1.5
/// fixture shapes (no `messageId`, no nested `snapshot`) parse
/// successfully but degrade to `ParseOutcome::Skip` because there's
/// nothing to ingest. Real CC data always carries both, per Phase
/// 1.5 §1's empirical sample.
#[derive(Debug, Deserialize)]
struct FileHistorySnapshotPayload {
    /// Outer messageId — joins to events.uuid of the assistant turn
    /// that triggered the snapshot. Missing → Skip (can't link).
    #[serde(rename = "messageId", default)]
    message_id: Option<String>,
    /// Inner snapshot object carrying the trackedFileBackups dict.
    /// Missing → Skip (nothing to ingest).
    #[serde(default)]
    snapshot: Option<SnapshotInner>,
    #[serde(rename = "isSnapshotUpdate", default)]
    is_snapshot_update: bool,
    /// Session + cwd context not present on snapshot lines in
    /// observed data; we'll fall back to None and let queries that
    /// need session context JOIN through events.uuid = message_id.
    #[serde(rename = "sessionId", default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnapshotInner {
    // The snapshot's own timestamp is informational; per-file
    // `backup_time` is what we land per file_snapshot row, so this
    // field is read off the wire but not stored.
    #[allow(dead_code)]
    timestamp: DateTime<Utc>,
    #[serde(rename = "trackedFileBackups", default)]
    tracked_file_backups: std::collections::HashMap<String, TrackedFileBackup>,
}

#[derive(Debug, Deserialize)]
struct TrackedFileBackup {
    #[serde(rename = "backupFileName", default)]
    backup_file_name: Option<String>,
    version: i64,
    #[serde(rename = "backupTime")]
    backup_time: DateTime<Utc>,
}

/// Block variants inside `message.content` arrays. Assistant messages
/// carry `tool_use` (and `text`); user messages carry `tool_result`
/// (and `text`). The `Other` catch-all covers `text` and any future
/// block type — we only care about the two we extract.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum MessageBlock {
    #[serde(rename = "tool_use")]
    ToolUse(ToolUseBlock),
    #[serde(rename = "tool_result")]
    ToolResult(ToolResultBlock),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ToolUseBlock {
    id: String,
    name: String,
    /// Tool-specific input object. Stored verbatim as JSON text on
    /// the ToolUse record — Phase 2/3 query primitives extract
    /// per-tool fields on demand.
    input: Value,
}

#[derive(Debug, Deserialize)]
struct ToolResultBlock {
    tool_use_id: String,
    /// Content can be a plain string (99.3% of observed records) OR
    /// a structured array (0.7% — typically tool-reference metadata
    /// like `{"type":"tool_reference","tool_name":"…"}`). We
    /// normalize both shapes to a String at parse time so the DB
    /// column stays simple TEXT.
    content: Value,
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

    match line {
        JsonlLine::Assistant(payload) => parse_assistant(*payload, raw_line, capture_raw),
        JsonlLine::User(payload) => parse_user(*payload),
        JsonlLine::FileHistorySnapshot(payload) => parse_file_history_snapshot(*payload),
        JsonlLine::Other => ParseOutcome::Skip,
    }
}

fn parse_assistant(
    assistant: AssistantPayload,
    raw_line: &str,
    capture_raw: bool,
) -> ParseOutcome {
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

    // v0.1.16: per-field debug-level log when the capture is missing.
    if assistant.uuid.is_none() {
        debug!(
            request_id = ?assistant.request_id,
            timestamp = %assistant.timestamp,
            "JSONL assistant line missing uuid — CC schema drift?"
        );
    }

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
        session_id: assistant.session_id.clone(),
        project_id: assistant.cwd.clone(),
        workspace_id: None,
        api_key_id: None,
        uuid: assistant.uuid.clone(),
        parent_uuid: assistant.parent_uuid,
        git_branch: assistant.git_branch,
        raw: capture_raw.then(|| raw_line.to_owned()),
    };

    // Fall back to a content hash when the source did not give us a
    // request_id. Computed *after* the rest of the event is filled in so
    // the projection covers every field that uniquely identifies the row.
    if event.request_id.is_none() {
        event.content_hash = Some(compute_content_hash(&event));
    }

    // v0.1.17 / Phase 1.5: extract tool_use blocks. The parent_event_uuid
    // is the assistant event's uuid — if it's None (schema-drift case),
    // the tool_uses still need SOME parent reference, so we fall back to
    // the empty string. The orphan-detection query in scan.rs surfaces
    // this case if it ever happens at scale.
    let parent_event_uuid = assistant.uuid.clone().unwrap_or_default();
    let mut tool_uses: Vec<ToolUse> = Vec::new();
    for block in assistant.message.content {
        if let MessageBlock::ToolUse(tu) = block {
            // Serialize input verbatim as JSON text. serde_json never
            // fails to serialize a Value it just deserialized.
            let input_json = serde_json::to_string(&tu.input)
                .unwrap_or_else(|_| String::from("{}"));
            tool_uses.push(ToolUse {
                tool_use_id: tu.id,
                parent_event_uuid: parent_event_uuid.clone(),
                source: SOURCE_KIND.to_owned(),
                tool_name: tu.name,
                input_json,
                occurred_at: assistant.timestamp,
                session_id: assistant.session_id.clone(),
                project_id: assistant.cwd.clone(),
            });
        }
        // text blocks and any future kind drop silently via Other.
    }

    ParseOutcome::Records(Box::new(ParsedRecords {
        event: Some(Box::new(event)),
        tool_uses,
        tool_results: Vec::new(),
        file_snapshots: Vec::new(),
    }))
}

fn parse_user(user: UserPayload) -> ParseOutcome {
    // User lines never produce an Event (cost / impact aggregation
    // is per-assistant-turn). They only contribute tool_result rows.
    let parent_event_uuid = user.uuid.unwrap_or_default();
    let blocks = match user.message.content {
        UserContent::Text(_) => {
            // Text-only user input (typed prompt, pasted text, etc.).
            // No tool_result here; nothing to ingest.
            return ParseOutcome::Skip;
        }
        UserContent::Blocks(blocks) => blocks,
    };
    let mut tool_results: Vec<ToolResult> = Vec::new();
    for block in blocks {
        if let MessageBlock::ToolResult(tr) = block {
            // Normalize content: string passes through; array gets
            // JSON-encoded. Either shape preserves the original
            // information in a single TEXT column.
            let content = match tr.content {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            tool_results.push(ToolResult {
                tool_use_id: tr.tool_use_id,
                parent_event_uuid: parent_event_uuid.clone(),
                source: SOURCE_KIND.to_owned(),
                content,
                occurred_at: user.timestamp,
                session_id: user.session_id.clone(),
            });
        }
    }
    if tool_results.is_empty() {
        // User message with no tool_result blocks — text-only user
        // prompt, ~6% of user lines per §1. Nothing to ingest.
        return ParseOutcome::Skip;
    }
    let records = ParsedRecords {
        event: None,
        tool_uses: Vec::new(),
        tool_results,
        file_snapshots: Vec::new(),
    };
    debug_assert!(!records.is_empty());
    ParseOutcome::Records(Box::new(records))
}

fn parse_file_history_snapshot(payload: FileHistorySnapshotPayload) -> ParseOutcome {
    // Schema-drift tolerance: missing messageId or missing inner
    // snapshot → can't ingest anything useful, return Skip cleanly.
    // (Pre-Phase-1.5 fixtures had a different snapshot shape — this
    // path lets them parse without raising Malformed.)
    let (Some(message_id), Some(snapshot_inner)) = (payload.message_id, payload.snapshot) else {
        return ParseOutcome::Skip;
    };

    let mut file_snapshots: Vec<FileSnapshot> = Vec::new();
    for (file_path, backup) in snapshot_inner.tracked_file_backups {
        file_snapshots.push(FileSnapshot {
            snapshot_message_id: message_id.clone(),
            source: SOURCE_KIND.to_owned(),
            file_path,
            backup_file_name: backup.backup_file_name,
            version: backup.version,
            backup_time: backup.backup_time,
            is_snapshot_update: payload.is_snapshot_update,
            session_id: payload.session_id.clone(),
            project_id: payload.cwd.clone(),
        });
    }
    if file_snapshots.is_empty() {
        // Empty trackedFileBackups dict — most common case per §1.
        // Nothing to ingest from this snapshot.
        return ParseOutcome::Skip;
    }
    ParseOutcome::Records(Box::new(ParsedRecords {
        event: None,
        tool_uses: Vec::new(),
        tool_results: Vec::new(),
        file_snapshots,
    }))
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

    /// v0.1.17 helper: extract the Event from a ParseOutcome::Records,
    /// panicking with a clear message otherwise. Centralizes the
    /// post-fan-out unpacking the existing tests used to inline via
    /// `let ParseOutcome::Event(event) = ...`.
    fn take_event(outcome: ParseOutcome) -> Box<Event> {
        match outcome {
            ParseOutcome::Records(records) => records
                .event
                .expect("expected ParseOutcome::Records to carry an Event"),
            other => panic!("expected ParseOutcome::Records, got {other:?}"),
        }
    }

    #[test]
    fn assistant_line_parses_with_full_token_breakdown() {
        let event = take_event(parse_line(ASSISTANT_LINE, false));
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
        let event = take_event(parse_line(ASSISTANT_LINE, true));
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
        let event = take_event(parse_line(ERROR_LINE_NO_REQUEST_ID, false));
        assert!(event.request_id.is_none());
        assert!(event.content_hash.is_some());
        // SHA-256 hex output is 64 chars
        assert_eq!(event.content_hash.as_deref().unwrap().len(), 64);
    }

    #[test]
    fn content_hash_is_deterministic() {
        let first = take_event(parse_line(ERROR_LINE_NO_REQUEST_ID, false));
        let second = take_event(parse_line(ERROR_LINE_NO_REQUEST_ID, false));
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
        assert!(matches!(outcome, ParseOutcome::Records(_)));
    }

    // v0.1.16 — three parser tests for the new captures.

    #[test]
    fn assistant_line_captures_uuid_parent_uuid_git_branch() {
        // All three fields present in the test fixture (per the
        // ASSISTANT_LINE constant) → Event carries them through.
        let event = take_event(parse_line(ASSISTANT_LINE, false));
        assert_eq!(event.uuid.as_deref(), Some("db6baab1"));
        assert_eq!(event.parent_uuid.as_deref(), Some("9a27c40f"));
        assert_eq!(event.git_branch.as_deref(), Some("main"));
    }

    #[test]
    fn assistant_line_missing_uuid_yields_none_other_fields_intact() {
        // Strip the uuid field from the fixture but keep everything
        // else. Parser must emit the row with uuid=None and the other
        // captures populated. Tolerance contract from § 2.
        let stripped = ASSISTANT_LINE.replace(r#","uuid":"db6baab1""#, "");
        let event = take_event(parse_line(&stripped, false));
        assert!(event.uuid.is_none(), "missing uuid must yield None");
        // Other captures unaffected:
        assert_eq!(event.parent_uuid.as_deref(), Some("9a27c40f"));
        assert_eq!(event.git_branch.as_deref(), Some("main"));
        // And the non-1B-ii fields aren't affected either:
        assert_eq!(event.request_id.as_deref(), Some("req_011CaFyK"));
        assert_eq!(event.input_tokens, 6);
    }

    #[test]
    fn assistant_line_missing_parent_uuid_yields_none_other_fields_intact() {
        let stripped = ASSISTANT_LINE.replace(r#""parentUuid":"9a27c40f","#, "");
        let event = take_event(parse_line(&stripped, false));
        assert!(event.parent_uuid.is_none());
        assert_eq!(event.uuid.as_deref(), Some("db6baab1"));
        assert_eq!(event.git_branch.as_deref(), Some("main"));
    }

    #[test]
    fn assistant_line_missing_git_branch_yields_none_other_fields_intact() {
        let stripped = ASSISTANT_LINE.replace(r#","gitBranch":"main""#, "");
        let event = take_event(parse_line(&stripped, false));
        assert!(event.git_branch.is_none());
        assert_eq!(event.uuid.as_deref(), Some("db6baab1"));
        assert_eq!(event.parent_uuid.as_deref(), Some("9a27c40f"));
    }

    #[test]
    fn assistant_line_missing_all_three_captures_yields_all_none() {
        // Regression for pre-v0.1.16 JSONL shape, in case CC's older
        // versions ever land in an ingest. Today's data always
        // carries all three (Phase 0: 27,388/27,388) but the parser
        // must not break on the legitimate-historical case.
        let stripped = ASSISTANT_LINE
            .replace(r#","uuid":"db6baab1""#, "")
            .replace(r#""parentUuid":"9a27c40f","#, "")
            .replace(r#","gitBranch":"main""#, "");
        let event = take_event(parse_line(&stripped, false));
        assert!(event.uuid.is_none());
        assert!(event.parent_uuid.is_none());
        assert!(event.git_branch.is_none());
        // Token totals unaffected.
        assert_eq!(event.input_tokens, 6);
        assert_eq!(event.output_tokens, 136);
    }

    #[test]
    fn old_format_without_cache_creation_breakdown_attributes_to_5m() {
        // 2.1.92-style line — `cache_creation_input_tokens` present but
        // `cache_creation` sub-object absent. We attribute to 5m by
        // convention so totals still reconcile.
        let old_format = r#"{"type":"assistant","timestamp":"2026-04-21T00:29:54.704Z","sessionId":"s","cwd":"/p","message":{"model":"claude-opus-4-7","id":"m","type":"message","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}},"requestId":"req_x"}"#;
        let event = take_event(parse_line(old_format, false));
        assert_eq!(event.cache_write_5m_tokens, 4);
        assert_eq!(event.cache_write_1h_tokens, 0);
    }
}
