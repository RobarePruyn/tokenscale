//! Top-level scan orchestrator. Walks the Claude Code root, parses each
//! changed JSONL file, and writes new events to the database.
//!
//! The whole-file mtime check is the cheap path: re-running `tokenscale scan`
//! against an unchanged tree is just a stat() per file. When a file's mtime
//! has advanced, we re-parse the entire file and rely on the database's
//! unique partial indexes (`(source, request_id)` and
//! `(source, content_hash)`) to dedupe lines that haven't changed since the
//! last scan. This is correct but does mean appending a single line forces
//! a full re-parse of that file; a future optimization (Phase 2+) tracks
//! a byte offset so we read only the appended tail.

use std::path::{Path, PathBuf};
use tokenscale_store::{
    count_tool_use_orphans, get_file_state, insert_events, insert_tool_data, upsert_file_state,
    Database,
};
use tracing::{debug, info, warn};

use crate::error::Result;
use crate::parser::{parse_line, ParsedRecords, ParseOutcome};
use crate::walker::{walk_claude_code_roots, JsonlFile};

const SOURCE_KIND: &str = "claude_code";

/// Aggregated outcome of a `run_scan` call. Suitable for logging or showing
/// the user.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub files_seen: usize,
    pub files_parsed: usize,
    pub files_unchanged: usize,
    pub events_inserted: usize,
    /// Hits on the existing (source, request_id) / (source, content_hash)
    /// partial unique indexes. Steady-state value is large — every
    /// unchanged-file rescan increments this, expected, silent.
    pub events_duplicates: usize,
    /// v0.1.16: hits on the new (source, uuid) partial unique index
    /// (or, more precisely, the pre-check in `insert_events` that
    /// catches the collision before INSERT OR IGNORE). Phase 0 found
    /// zero of these in 27,388 lines — steady-state value SHOULD be
    /// zero. **Non-zero is the loud signal** that Claude Code's
    /// behavior may have changed; see preceding log lines for the
    /// per-event uuid + JSONL path + line number context. Tracked
    /// separately from `events_duplicates` because bundling the
    /// counts hides the loud signal in the noisy one.
    pub uuid_duplicates_skipped: usize,
    /// v0.1.17 / Phase 1.5: rows that landed in the new auxiliary
    /// tables. Steady-state values are ~0.6× / ~0.6× / ~0.1× the
    /// assistant-event count per Phase 1.5 §1's empirical sample.
    /// Re-scan of an unchanged file lands all three at zero (the
    /// (source, *_id) UNIQUE indexes dedup via INSERT OR IGNORE,
    /// same posture as events_duplicates).
    pub tool_uses_inserted: usize,
    pub tool_results_inserted: usize,
    pub file_snapshots_inserted: usize,
    /// v0.1.17 / Phase 1.5 Addition 1: count of `tool_use` rows in
    /// the DB (post-ingest, across the source) whose `tool_use_id`
    /// has no matching `tool_result`. Phase 0 baseline on maintainer
    /// data: 5 (interrupted sessions, expected). Not a release gate;
    /// not a steady-state warning. **First sign of upstream schema
    /// drift surfaces here cheaply** — if a future scan reports 50
    /// orphans, something changed about how CC writes JSONL or how
    /// Tokenscale ingests it.
    pub tool_use_orphans: usize,
    pub lines_skipped: usize,
    pub lines_malformed: usize,
}

/// Run a full scan across one or more Claude Code roots. Each root
/// is walked independently and the union of discovered JSONL files is
/// scanned; `_ingest_file_state` keys by full path so multiple roots
/// can carry identically-named session files without collision.
///
/// `capture_raw_payloads` mirrors the `ingest.store_raw` config flag
/// — when `false`, the parser drops the raw JSONL line after
/// extracting the fields we need, trading diagnostic flexibility for
/// reduced disk usage and reduced exposure of session content.
pub async fn run_scan_multi(
    database: &Database,
    claude_code_roots: &[PathBuf],
    capture_raw_payloads: bool,
) -> Result<ScanSummary> {
    info!(
        roots = ?claude_code_roots,
        capture_raw_payloads,
        "starting Claude Code JSONL scan"
    );

    let candidate_files = walk_claude_code_roots(claude_code_roots).await?;
    run_scan_over_files(database, candidate_files, capture_raw_payloads).await
}

/// Single-root scan retained for callers that don't yet need the
/// multi-root surface (existing tests, and `run_scan` users that
/// still pass a single path). Thin wrapper around `run_scan_multi`.
pub async fn run_scan(
    database: &Database,
    claude_code_root: &Path,
    capture_raw_payloads: bool,
) -> Result<ScanSummary> {
    run_scan_multi(
        database,
        std::slice::from_ref(&claude_code_root.to_path_buf()),
        capture_raw_payloads,
    )
    .await
}

async fn run_scan_over_files(
    database: &Database,
    candidate_files: Vec<JsonlFile>,
    capture_raw_payloads: bool,
) -> Result<ScanSummary> {
    let mut summary = ScanSummary {
        files_seen: candidate_files.len(),
        ..ScanSummary::default()
    };

    for jsonl_file in candidate_files {
        match scan_one_file(database, &jsonl_file, capture_raw_payloads).await? {
            FileOutcome::Skipped => summary.files_unchanged += 1,
            FileOutcome::Processed(file_summary) => {
                summary.files_parsed += 1;
                summary.events_inserted += file_summary.events_inserted;
                summary.events_duplicates += file_summary.events_duplicates;
                summary.uuid_duplicates_skipped += file_summary.uuid_duplicates_skipped;
                summary.tool_uses_inserted += file_summary.tool_uses_inserted;
                summary.tool_results_inserted += file_summary.tool_results_inserted;
                summary.file_snapshots_inserted += file_summary.file_snapshots_inserted;
                summary.lines_skipped += file_summary.lines_skipped;
                summary.lines_malformed += file_summary.lines_malformed;
            }
        }
    }

    // v0.1.17 / Phase 1.5 Addition 1: orphan-count baseline. Run
    // once at the end of the scan rather than per-file (cheap;
    // single COUNT(*) joined against the (source, tool_use_id)
    // indexes). Surfaces upstream schema drift cheaply.
    summary.tool_use_orphans = count_tool_use_orphans(database, SOURCE_KIND).await?;

    info!(?summary, "scan complete");
    Ok(summary)
}

#[derive(Debug)]
enum FileOutcome {
    Skipped,
    Processed(FileSummary),
}

#[derive(Debug, Default)]
struct FileSummary {
    events_inserted: usize,
    events_duplicates: usize,
    uuid_duplicates_skipped: usize,
    tool_uses_inserted: usize,
    tool_results_inserted: usize,
    file_snapshots_inserted: usize,
    lines_skipped: usize,
    lines_malformed: usize,
}

async fn scan_one_file(
    database: &Database,
    jsonl_file: &JsonlFile,
    capture_raw_payloads: bool,
) -> Result<FileOutcome> {
    let path_string = jsonl_file.path.display().to_string();

    if let Some(stored_state) = get_file_state(database, SOURCE_KIND, &path_string).await? {
        if stored_state.mtime_ns == jsonl_file.mtime_ns && stored_state.len == jsonl_file.len {
            debug!(path = %path_string, "skipping (mtime + len unchanged)");
            return Ok(FileOutcome::Skipped);
        }
    }

    let file_contents = match tokio::fs::read_to_string(&jsonl_file.path).await {
        Ok(contents) => contents,
        Err(io_error) => {
            warn!(path = %path_string, error = %io_error, "skipping unreadable file");
            return Ok(FileOutcome::Skipped);
        }
    };

    let mut events_to_insert = Vec::new();
    // v0.1.16: parallel to events_to_insert. 1-indexed line number
    // for each event (matches what an operator sees in
    // editor / `head -n N` output). Used to emit per-uuid-duplicate
    // warnings after insert_events returns its indices.
    // I1.5 Option A: scan caller owns provenance + emits warnings;
    // Event struct stays clean of ingest-only fields.
    let mut event_line_numbers: Vec<usize> = Vec::new();
    let mut file_summary = FileSummary::default();

    // v0.1.17 / Phase 1.5: collect tool-use auxiliary records alongside
    // the events. Each parsed line can produce an Event AND/OR
    // tool_uses / tool_results / file_snapshots (per the ParsedRecords
    // fan-out). Insert paths land separately but inside the same
    // per-file transaction surface.
    let mut tool_uses_to_insert: Vec<tokenscale_core::ToolUse> = Vec::new();
    let mut tool_results_to_insert: Vec<tokenscale_core::ToolResult> = Vec::new();
    let mut file_snapshots_to_insert: Vec<tokenscale_core::FileSnapshot> = Vec::new();

    for (line_index, raw_line) in file_contents.lines().enumerate() {
        match parse_line(raw_line, capture_raw_payloads) {
            ParseOutcome::Skip => file_summary.lines_skipped += 1,
            ParseOutcome::Records(records) => {
                let ParsedRecords {
                    event,
                    tool_uses,
                    tool_results,
                    file_snapshots,
                } = *records;
                if let Some(boxed_event) = event {
                    events_to_insert.push(*boxed_event);
                    event_line_numbers.push(line_index + 1);
                }
                tool_uses_to_insert.extend(tool_uses);
                tool_results_to_insert.extend(tool_results);
                file_snapshots_to_insert.extend(file_snapshots);
            }
            ParseOutcome::Malformed { reason } => {
                file_summary.lines_malformed += 1;
                warn!(
                    path = %path_string,
                    line = line_index + 1,
                    reason = %reason,
                    "skipping malformed JSONL line"
                );
            }
        }
    }

    let insert_summary = insert_events(database, &events_to_insert).await?;
    file_summary.events_inserted = insert_summary.inserted;
    file_summary.events_duplicates = insert_summary.skipped_duplicate;
    file_summary.uuid_duplicates_skipped = insert_summary.uuid_duplicate_indices.len();

    // v0.1.17 / Phase 1.5: insert the auxiliary tool-data rows. Runs
    // in its own transaction; INSERT OR IGNORE on the (source, *_id)
    // UNIQUE indexes dedups rescans automatically (same posture as
    // events). The three Vec<…> shape mirrors the parallel fan-out
    // from ParsedRecords.
    let tool_summary = insert_tool_data(
        database,
        &tool_uses_to_insert,
        &tool_results_to_insert,
        &file_snapshots_to_insert,
    )
    .await?;
    file_summary.tool_uses_inserted = tool_summary.tool_uses_inserted;
    file_summary.tool_results_inserted = tool_summary.tool_results_inserted;
    file_summary.file_snapshots_inserted = tool_summary.file_snapshots_inserted;

    // v0.1.16: emit per-event WARN logs with the documented context.
    // Steady-state value is zero (Phase 0 found no current duplicates);
    // a non-zero count here is the loud signal that CC's behavior may
    // have changed.
    for &event_index in &insert_summary.uuid_duplicate_indices {
        let event = &events_to_insert[event_index];
        let line_number = event_line_numbers[event_index];
        warn!(
            uuid = event.uuid.as_deref().unwrap_or("?"),
            source = %event.source,
            source_jsonl = %path_string,
            line = line_number,
            "duplicate uuid detected during ingest — skipping; CC behavior change?"
        );
    }

    upsert_file_state(
        database,
        SOURCE_KIND,
        &path_string,
        jsonl_file.mtime_ns,
        jsonl_file.len,
    )
    .await?;

    debug!(
        path = %path_string,
        inserted = file_summary.events_inserted,
        duplicates = file_summary.events_duplicates,
        skipped = file_summary.lines_skipped,
        malformed = file_summary.lines_malformed,
        "file processed"
    );

    Ok(FileOutcome::Processed(file_summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// One assistant + one user line in a single session file. Should ingest
    /// exactly one event.
    const TWO_LINE_SESSION: &str = "\
{\"type\":\"user\",\"timestamp\":\"2026-04-21T00:29:50.000Z\",\"content\":\"hi\"}
{\"parentUuid\":\"p\",\"isSidechain\":false,\"message\":{\"model\":\"claude-opus-4-7\",\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"cache_read_input_tokens\":30,\"cache_creation_input_tokens\":40,\"cache_creation\":{\"ephemeral_5m_input_tokens\":15,\"ephemeral_1h_input_tokens\":25}}},\"requestId\":\"req_AAA\",\"type\":\"assistant\",\"uuid\":\"u\",\"timestamp\":\"2026-04-21T00:29:54.000Z\",\"sessionId\":\"sess1\",\"cwd\":\"/proj\",\"version\":\"2.1.120\",\"userType\":\"external\",\"entrypoint\":\"claude-vscode\",\"gitBranch\":\"main\"}
";

    #[tokio::test]
    async fn end_to_end_scan_inserts_assistant_events_only() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let temp_root = TempDir::new()?;
        let project_directory = temp_root.path().join("project-x");
        fs::create_dir(&project_directory)?;
        fs::write(project_directory.join("session.jsonl"), TWO_LINE_SESSION)?;

        let summary = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(summary.files_seen, 1);
        assert_eq!(summary.files_parsed, 1);
        assert_eq!(summary.events_inserted, 1);
        assert_eq!(summary.events_duplicates, 0);
        assert_eq!(summary.lines_skipped, 1); // the user line
        assert_eq!(summary.lines_malformed, 0);
        Ok(())
    }

    #[tokio::test]
    async fn rerun_with_unchanged_files_inserts_nothing() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let temp_root = TempDir::new()?;
        let project_directory = temp_root.path().join("project-x");
        fs::create_dir(&project_directory)?;
        fs::write(project_directory.join("session.jsonl"), TWO_LINE_SESSION)?;

        let first = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(first.events_inserted, 1);

        let second = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(second.files_unchanged, 1);
        assert_eq!(second.files_parsed, 0);
        assert_eq!(second.events_inserted, 0);
        Ok(())
    }

    #[tokio::test]
    async fn rerun_with_len_changed_but_mtime_preserved_re_parses() -> Result<()> {
        // Cloud-FS drift scenario: bytes were updated but mtime was
        // preserved by the sync layer. mtime alone would skip; the
        // (mtime, len) tuple catches it.
        let database = Database::open_in_memory_for_tests().await?;
        let temp_root = TempDir::new()?;
        let project_directory = temp_root.path().join("project-x");
        fs::create_dir(&project_directory)?;
        let session_path = project_directory.join("session.jsonl");
        fs::write(&session_path, TWO_LINE_SESSION)?;

        let first = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(first.events_inserted, 1);
        let pinned_mtime = std::fs::File::open(&session_path)?
            .metadata()?
            .modified()?;

        // Append a noop comment line to grow the file, then restore
        // the original mtime — simulating sync engines that preserve
        // mtime across content updates.
        fs::write(&session_path, format!("{TWO_LINE_SESSION}// trailing\n"))?;
        std::fs::File::open(&session_path)?.set_modified(pinned_mtime)?;

        let second = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(second.files_unchanged, 0);
        assert_eq!(second.files_parsed, 1);
        // The trailing line is malformed JSON → counts as malformed; the
        // existing event de-dupes via request_id.
        assert_eq!(second.events_inserted, 0);
        assert_eq!(second.events_duplicates, 1);
        Ok(())
    }

    #[tokio::test]
    async fn rerun_with_touched_file_dedupes_via_request_id() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let temp_root = TempDir::new()?;
        let project_directory = temp_root.path().join("project-x");
        fs::create_dir(&project_directory)?;
        let session_path = project_directory.join("session.jsonl");
        fs::write(&session_path, TWO_LINE_SESSION)?;

        run_scan(&database, temp_root.path(), false).await?;

        // Touch the file to advance its mtime, content unchanged.
        let new_time = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        let file = std::fs::File::open(&session_path)?;
        file.set_modified(new_time)?;

        let second = run_scan(&database, temp_root.path(), false).await?;
        assert_eq!(second.files_unchanged, 0);
        assert_eq!(second.files_parsed, 1);
        assert_eq!(second.events_inserted, 0);
        assert_eq!(second.events_duplicates, 1);
        Ok(())
    }
}
