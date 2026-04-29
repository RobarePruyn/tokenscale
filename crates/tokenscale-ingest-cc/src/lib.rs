//! `tokenscale-ingest-cc` — Claude Code JSONL ingester.
//!
//! Walks `~/.claude/projects/<project-id>/<session-id>.jsonl`, parses each
//! line into a `tokenscale-core::Event`, and writes new rows to the database
//! via `tokenscale-store`.
//!
//! Idempotency model:
//!
//! - Track each file's last-seen mtime in the database. On re-scan, files
//!   unchanged since the last run are stat()'d and skipped without parsing.
//! - For each line, if the JSON carries a `requestId`, dedupe on
//!   `(source = 'claude_code', request_id)`.
//! - For lines missing a `requestId` (API-error lines and a few older
//!   Claude Code revisions), fall back to a SHA-256 over a deterministic
//!   projection of the row, stored in `events.content_hash`. The unique
//!   index on `(source, content_hash)` does the deduplication.
//!
//! Schema-drift tolerance: unknown JSON fields are ignored, missing
//! optional fields default sensibly, missing required fields skip the
//! line and log a warning rather than crashing the scan.

mod error;
mod parser;
mod scan;
mod walker;

pub use error::{IngestError, Result};
pub use parser::{parse_line, ParseOutcome};
pub use scan::{run_scan, ScanSummary};
pub use walker::{walk_claude_code_root, JsonlFile};
