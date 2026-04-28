//! `tokenscale-ingest-cc` — Claude Code JSONL ingester.
//!
//! Walks `~/.claude/projects/<project-id>/<session-id>.jsonl`, parses each line
//! into a `tokenscale-core::Event`, and writes new rows to the database via
//! `tokenscale-store`.
//!
//! Idempotency model:
//!
//!   * Track each file's last-seen mtime in the database. On re-scan, files
//!     unchanged since the last run are stat()'d and skipped without parsing.
//!   * For each line, if the JSON carries a `request_id`, dedupe on
//!     `(source = 'claude_code', request_id)`.
//!   * For lines missing a `request_id` (older Claude Code revisions), fall
//!     back to a SHA-256 over a deterministic projection of the row
//!     (`occurred_at || model || token counts || session_id || project_id`),
//!     stored in `events.content_hash`. The unique index on
//!     `(source, content_hash)` does the deduplication.
//!
//! Schema-drift tolerance:
//!
//!   * Unknown JSON fields are ignored.
//!   * Missing optional fields default sensibly.
//!   * Missing required fields skip the line, log a structured warning, and
//!     do not crash the scan.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
