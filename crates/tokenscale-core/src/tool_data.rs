//! Tool-use ingest types (v0.1.17 / Phase 1.5).
//!
//! Three lightweight records carry data that was previously dropped:
//!
//! - `ToolUse` — one `tool_use` block from an assistant message's
//!   `content` array. Bash command, Edit/Write file path, Read path,
//!   TodoWrite todos, etc. The `input_json` field stores the full
//!   `input` object verbatim as JSON text — Phase 2/3 query primitives
//!   filter / extract from it without needing per-tool deserialization
//!   surface here.
//!
//! - `ToolResult` — one `tool_result` block from a user message's
//!   `content` array. Linked back to its `ToolUse` by `tool_use_id`
//!   at query time (per the Phase 1.5 D3 decision; 100% empirical
//!   linkage rate in observed data makes the JOIN reliable).
//!
//! - `FileSnapshot` — one entry from a `file-history-snapshot` line's
//!   `trackedFileBackups` dict. Each snapshot may produce zero or many
//!   `FileSnapshot` rows depending on how many files it tracks.
//!
//! All three are pure domain types with no DB knowledge. The store
//! crate's `insert_tool_data` does the actual SQL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One `tool_use` block extracted from an assistant message.
///
/// Carries enough context (`source`, `parent_event_uuid`, `session_id`,
/// `project_id`, `occurred_at`) to be queryable on its own without an
/// inner JOIN to `events`. `input_json` is the full original `input`
/// object as JSON text so Phase 2/3 query primitives can extract
/// tool-specific fields (Bash `command`, Edit `file_path`, etc.)
/// without the parser needing per-tool schema knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUse {
    /// The Anthropic tool-use identifier (e.g. `toolu_01Nzk…`). Joins
    /// to `ToolResult.tool_use_id` at query time. Distinct from CC's
    /// per-message `uuid` (which lives on `Event.uuid`).
    pub tool_use_id: String,
    /// `Event.uuid` of the assistant turn that produced this tool_use.
    pub parent_event_uuid: String,
    /// `events.source` — e.g. `"claude_code"`.
    pub source: String,
    /// Tool name as CC reports it — `Bash`, `Edit`, `Read`, `Write`,
    /// `TodoWrite`, etc. Stored verbatim; new tool names land here
    /// without any code change.
    pub tool_name: String,
    /// The full `input` object as JSON text. Phase 2/3 query
    /// primitives parse this on-demand for tool-specific fields.
    pub input_json: String,
    /// Inherited from the parent assistant event's timestamp.
    pub occurred_at: DateTime<Utc>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
}

/// One `tool_result` block extracted from a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// References the `ToolUse.tool_use_id` this is a result for.
    /// Linkage resolves via SQL JOIN at query time.
    pub tool_use_id: String,
    /// `Event.uuid` of the USER turn that carried this tool_result.
    pub parent_event_uuid: String,
    pub source: String,
    /// The raw tool output. Bash stdout, file contents from Read, the
    /// success-acknowledgement string from Edit/Write, etc. Can be
    /// large for verbose Bash output; stored as-is.
    pub content: String,
    pub occurred_at: DateTime<Utc>,
    pub session_id: Option<String>,
}

/// One file row extracted from a `file-history-snapshot`'s
/// `trackedFileBackups` dict. A single snapshot record may produce
/// zero (empty dict, most common — 3,049 records, most are empty in
/// the maintainer's sample), one, or many `FileSnapshot` rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileSnapshot {
    /// The outer `messageId` field on the snapshot record, which
    /// joins back to `events.uuid` (the assistant turn that triggered
    /// the snapshot).
    pub snapshot_message_id: String,
    pub source: String,
    /// The file path (key in the `trackedFileBackups` dict). Relative
    /// to the repo root as CC observes it.
    pub file_path: String,
    /// `backupFileName` field — nullable per observed data.
    pub backup_file_name: Option<String>,
    pub version: i64,
    pub backup_time: DateTime<Utc>,
    /// `isSnapshotUpdate` on the outer record — `true` for snapshots
    /// chained from a previous one, `false` for fresh snapshots.
    pub is_snapshot_update: bool,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
}
