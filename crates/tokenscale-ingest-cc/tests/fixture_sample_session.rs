//! End-to-end scan against the v0.1.17 / Phase 1.5 representative
//! sample fixture. Built per Addition 4 of the Phase 1.5 sign-off:
//! "JSONL test fixtures live in files, not strings."
//!
//! The fixture covers:
//! - All five top tool names (Bash, Edit, Read, Write, TodoWrite)
//! - An assistant message with max=3 tool_use blocks
//! - An orphan tool_use (no matching tool_result — interrupted session)
//! - A file-history-snapshot with non-empty trackedFileBackups
//! - Cross-line tool_use → tool_result linkage (natural flow)

use std::fs;
use tempfile::TempDir;
use tokenscale_ingest_cc::run_scan;
use tokenscale_store::{count_events, count_tool_use_orphans, Database};

const FIXTURE_BYTES: &[u8] = include_bytes!("fixtures/sample-session.jsonl");

#[tokio::test]
async fn sample_session_fixture_lands_expected_tool_data() {
    let database = Database::open_in_memory_for_tests().await.unwrap();
    let temp_root = TempDir::new().unwrap();
    let project_directory = temp_root.path().join("project-sample");
    fs::create_dir(&project_directory).unwrap();
    fs::write(project_directory.join("session.jsonl"), FIXTURE_BYTES).unwrap();

    let summary = run_scan(&database, temp_root.path(), false).await.unwrap();

    // The fixture has:
    //   - 4 assistant lines → 4 events
    //   - 3 user lines (all with tool_results)
    //   - 1 file-history-snapshot line (with non-empty trackedFileBackups)
    // Total: 8 non-empty lines, 0 malformed, 0 skipped (every line
    // produces records or an event).
    assert_eq!(summary.files_seen, 1);
    assert_eq!(summary.files_parsed, 1);
    assert_eq!(summary.events_inserted, 4, "4 assistant events");
    assert_eq!(summary.events_duplicates, 0);
    assert_eq!(summary.lines_malformed, 0);
    assert_eq!(count_events(&database).await.unwrap(), 4);

    // tool_uses: assistant A1 has 3 (Bash, Edit, Read); A2 has 1
    // (Write); A3 has 1 (TodoWrite); A4 has 1 (Bash, orphan).
    // Total = 6.
    assert_eq!(
        summary.tool_uses_inserted, 6,
        "6 tool_use blocks: 3+1+1+1 across the four assistant lines"
    );

    // tool_results: user U1 has 3; U2 has 1; U3 has 1. Total = 5.
    // (A4's orphan Bash has no matching user line at all.)
    assert_eq!(
        summary.tool_results_inserted, 5,
        "5 tool_results: 3+1+1 across the three user lines"
    );

    // file_snapshots: the one file-history-snapshot record carries
    // 2 entries in its trackedFileBackups dict (new_file.rs + foo.rs).
    assert_eq!(
        summary.file_snapshots_inserted, 2,
        "1 snapshot record → 2 file_snapshot rows (trackedFileBackups has 2 entries)"
    );

    // Orphan count: exactly 1 (toolu_ORPHAN, the trailing Bash with
    // no following user line — interrupted-session pattern from
    // Phase 1.5 §1).
    assert_eq!(
        summary.tool_use_orphans, 1,
        "interrupted-session: A4's tool_use has no matching tool_result"
    );
    // Cross-check via the dedicated query path.
    let direct = count_tool_use_orphans(&database, "claude_code").await.unwrap();
    assert_eq!(direct, 1);

    // Rescan is idempotent across the tool_data tables too.
    let session_path = project_directory.join("session.jsonl");
    let new_time = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    fs::File::open(&session_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();

    let second = run_scan(&database, temp_root.path(), false).await.unwrap();
    assert_eq!(second.events_inserted, 0);
    assert_eq!(second.events_duplicates, 4);
    assert_eq!(second.tool_uses_inserted, 0, "rescan dedups via (source, tool_use_id) UNIQUE");
    assert_eq!(second.tool_results_inserted, 0);
    assert_eq!(second.file_snapshots_inserted, 0);
    // Orphan count unchanged across rescans.
    assert_eq!(second.tool_use_orphans, 1);
}
