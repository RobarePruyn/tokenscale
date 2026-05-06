//! `_ingest_file_state` accessors — track last-seen `(mtime, len)` per
//! ingested file.
//!
//! The point of this table is making `tokenscale scan` cheap on re-run.
//! The ingest layer stat()s each candidate file, compares against the
//! stored tuple, and only opens the file if either field changed.
//!
//! Why `(mtime_ns, len)` instead of mtime alone: cloud-synced filesystems
//! (iCloud, Dropbox, OneDrive) sometimes preserve mtime across syncs but
//! mutate bytes — or vice versa. Comparing the size alongside mtime
//! catches both flavors of drift at the cost of one extra integer per
//! row.

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub source: String,
    pub file_path: String,
    pub mtime_ns: i64,
    /// File size in bytes at last scan. Compared alongside `mtime_ns`
    /// for cloud-FS robustness — see module docs.
    pub len: i64,
    pub last_scanned_at: DateTime<Utc>,
}

/// Look up the stored state for a file, if any.
pub async fn get_file_state(
    database: &Database,
    source: &str,
    file_path: &str,
) -> Result<Option<FileState>> {
    let row: Option<(String, String, i64, i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT source, file_path, mtime_ns, len, last_scanned_at
           FROM _ingest_file_state
          WHERE source = ? AND file_path = ?",
    )
    .bind(source)
    .bind(file_path)
    .fetch_optional(database.pool())
    .await?;

    Ok(
        row.map(|(source, file_path, mtime_ns, len, last_scanned_at)| FileState {
            source,
            file_path,
            mtime_ns,
            len,
            last_scanned_at,
        }),
    )
}

/// Record that we scanned `file_path` at `(mtime_ns, len)` just now.
/// Insert or replace — re-scanning the same file is the common case.
pub async fn upsert_file_state(
    database: &Database,
    source: &str,
    file_path: &str,
    mtime_ns: i64,
    len: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO _ingest_file_state (source, file_path, mtime_ns, len, last_scanned_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (source, file_path) DO UPDATE SET
            mtime_ns        = excluded.mtime_ns,
            len             = excluded.len,
            last_scanned_at = excluded.last_scanned_at",
    )
    .bind(source)
    .bind(file_path)
    .bind(mtime_ns)
    .bind(len)
    .bind(Utc::now())
    .execute(database.pool())
    .await?;
    Ok(())
}

/// Delete every `_ingest_file_state` row for a given source. Next scan
/// will treat every file as new (Layer 1 mtime+len skip is bypassed),
/// but Layer 2 dedup at the events table prevents double-inserts —
/// `--rescan` is therefore safe to run any time.
pub async fn clear_file_state_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM _ingest_file_state WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Most recent `last_scanned_at` across all files for a given source.
/// Drives the dashboard's "data freshness" indicator: if this is N
/// seconds ago, the user is looking at data at least that old.
/// Returns `None` when no files have been scanned yet.
pub async fn most_recent_scan_at(
    database: &Database,
    source: &str,
) -> Result<Option<DateTime<Utc>>> {
    let row: Option<(Option<DateTime<Utc>>,)> =
        sqlx::query_as("SELECT MAX(last_scanned_at) FROM _ingest_file_state WHERE source = ?")
            .bind(source)
            .fetch_optional(database.pool())
            .await?;
    Ok(row.and_then(|tuple| tuple.0))
}

/// Delete every event for a given source. Used by `--rebuild` to wipe
/// the slate before a full re-parse — only appropriate when the user
/// has actively decided that prior ingest output is wrong (parser bug,
/// schema migration). Pair with `clear_file_state_for_source` so the
/// subsequent scan re-parses every file.
pub async fn delete_events_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM events WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}
