//! Integration test: end-to-end scan against a checked-in fixture session.
//!
//! The fixture covers the edge cases the parser needs to survive in the
//! wild:
//!
//! - non-assistant lines (queue-operation, user, file-history-snapshot,
//!   ai-title)
//! - duplicate-`requestId` assistant lines (the sidechain-replay case
//!   identified during Phase B kickoff)
//! - an API-error line with no `requestId` (must dedupe via content_hash)
//! - an older-format line with no `cache_creation` sub-object
//! - a line of plain text that should be flagged Malformed but not crash
//!   the scan

use std::fs;
use tempfile::TempDir;
use tokenscale_ingest_cc::run_scan;
use tokenscale_store::{count_events, Database};

const FIXTURE_BYTES: &[u8] = include_bytes!("fixtures/realistic_session.jsonl");

#[tokio::test]
async fn realistic_fixture_scan_yields_expected_events() {
    let database = Database::open_in_memory_for_tests().await.unwrap();
    let temp_root = TempDir::new().unwrap();
    let project_directory = temp_root.path().join("project-realistic");
    fs::create_dir(&project_directory).unwrap();
    fs::write(project_directory.join("session.jsonl"), FIXTURE_BYTES).unwrap();

    let summary = run_scan(&database, temp_root.path(), false).await.unwrap();

    // The fixture has 10 lines:
    //   3 non-assistant lines (queue, user, file-history, ai-title — actually 4)
    //   5 assistant lines (2 share a requestId, 1 is an error w/o requestId,
    //                      1 is old-format, 1 is opus-different)
    //   1 malformed line (plain text)
    assert_eq!(summary.files_seen, 1);
    assert_eq!(summary.files_parsed, 1);
    assert_eq!(summary.files_unchanged, 0);
    // 4 non-assistant lines (queue-operation, user, file-history-snapshot, ai-title)
    assert_eq!(summary.lines_skipped, 4);
    assert_eq!(summary.lines_malformed, 1);

    // Assistant lines = 5; one is a duplicate requestId, so 4 unique events
    // land in the database.
    assert_eq!(summary.events_inserted, 4);
    assert_eq!(summary.events_duplicates, 1);
    assert_eq!(count_events(&database).await.unwrap(), 4);

    // Re-scan — touch the file so mtime advances; everything dedupes.
    let session_path = project_directory.join("session.jsonl");
    let new_time = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    fs::File::open(&session_path)
        .unwrap()
        .set_modified(new_time)
        .unwrap();

    let second_scan = run_scan(&database, temp_root.path(), false).await.unwrap();
    assert_eq!(second_scan.events_inserted, 0);
    // 5 assistant lines parse into 5 candidate events; 4 get filtered by
    // (source, request_id), 1 (the error line w/o requestId) by
    // (source, content_hash). All are duplicates from the database's POV.
    assert_eq!(second_scan.events_duplicates, 5);
    assert_eq!(count_events(&database).await.unwrap(), 4);
}
