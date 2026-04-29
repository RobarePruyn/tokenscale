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

use std::path::Path;
use tokenscale_store::{get_file_state, insert_events, upsert_file_state, Database};
use tracing::{debug, info, warn};

use crate::error::Result;
use crate::parser::{parse_line, ParseOutcome};
use crate::walker::{walk_claude_code_root, JsonlFile};

const SOURCE_KIND: &str = "claude_code";

/// Aggregated outcome of a `run_scan` call. Suitable for logging or showing
/// the user.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub files_seen: usize,
    pub files_parsed: usize,
    pub files_unchanged: usize,
    pub events_inserted: usize,
    pub events_duplicates: usize,
    pub lines_skipped: usize,
    pub lines_malformed: usize,
}

/// Run a full scan against the given Claude Code root.
///
/// `capture_raw_payloads` mirrors the `ingest.store_raw` config flag — when
/// `false`, the parser drops the raw JSONL line after extracting the fields
/// we need, trading diagnostic flexibility for reduced disk usage and
/// reduced exposure of session content. See the README's Privacy section.
pub async fn run_scan(
    database: &Database,
    claude_code_root: &Path,
    capture_raw_payloads: bool,
) -> Result<ScanSummary> {
    info!(
        root = %claude_code_root.display(),
        capture_raw_payloads,
        "starting Claude Code JSONL scan"
    );

    let candidate_files = walk_claude_code_root(claude_code_root).await?;
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
                summary.lines_skipped += file_summary.lines_skipped;
                summary.lines_malformed += file_summary.lines_malformed;
            }
        }
    }

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
        if stored_state.mtime_ns == jsonl_file.mtime_ns {
            debug!(path = %path_string, "skipping (mtime unchanged)");
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
    let mut file_summary = FileSummary::default();

    for (line_index, raw_line) in file_contents.lines().enumerate() {
        match parse_line(raw_line, capture_raw_payloads) {
            ParseOutcome::Skip => file_summary.lines_skipped += 1,
            ParseOutcome::Event(boxed_event) => events_to_insert.push(*boxed_event),
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

    upsert_file_state(database, SOURCE_KIND, &path_string, jsonl_file.mtime_ns).await?;

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
