//! `_ingest_file_state` accessors — track last-seen mtimes per ingested file.
//!
//! The point of this table is making `tokenscale scan` cheap on re-run. The
//! ingest layer stat()s each candidate file, compares to the stored mtime,
//! and only opens the file if it has changed.
//!
//! `mtime_ns` is stored as i64 nanoseconds since the unix epoch — wide enough
//! for any realistic past or future and trivially comparable.

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub source: String,
    pub file_path: String,
    pub mtime_ns: i64,
    pub last_scanned_at: DateTime<Utc>,
}

/// Look up the stored state for a file, if any.
pub async fn get_file_state(
    database: &Database,
    source: &str,
    file_path: &str,
) -> Result<Option<FileState>> {
    let row: Option<(String, String, i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT source, file_path, mtime_ns, last_scanned_at
           FROM _ingest_file_state
          WHERE source = ? AND file_path = ?",
    )
    .bind(source)
    .bind(file_path)
    .fetch_optional(database.pool())
    .await?;

    Ok(
        row.map(|(source, file_path, mtime_ns, last_scanned_at)| FileState {
            source,
            file_path,
            mtime_ns,
            last_scanned_at,
        }),
    )
}

/// Record that we scanned `file_path` at `mtime_ns` just now. Insert or
/// replace — re-scanning the same file is the common case.
pub async fn upsert_file_state(
    database: &Database,
    source: &str,
    file_path: &str,
    mtime_ns: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO _ingest_file_state (source, file_path, mtime_ns, last_scanned_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (source, file_path) DO UPDATE SET
            mtime_ns        = excluded.mtime_ns,
            last_scanned_at = excluded.last_scanned_at",
    )
    .bind(source)
    .bind(file_path)
    .bind(mtime_ns)
    .bind(Utc::now())
    .execute(database.pool())
    .await?;
    Ok(())
}
