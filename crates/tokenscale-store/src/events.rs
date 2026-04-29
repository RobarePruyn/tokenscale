//! Event writer — idempotent batched insert into the `events` table.
//!
//! The two unique partial indexes on `events` — `(source, request_id)` when
//! `request_id` is present, `(source, content_hash)` when it isn't — make
//! `INSERT OR IGNORE` the natural way to dedupe. SQLite returns
//! `rows_affected = 0` when the unique-index conflict fires, which we count
//! and surface so the caller can report "X new, Y duplicates skipped" on
//! re-scan.
//!
//! Inserts are wrapped in a single transaction per call. Callers should batch
//! per file (or per ingest cycle) to amortize commit costs.

use tokenscale_core::Event;
use tracing::debug;

use crate::error::Result;
use crate::Database;

/// Result of an `insert_events` call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InsertSummary {
    /// Rows that landed in the database.
    pub inserted: usize,
    /// Rows that hit a unique-index conflict — already in the database.
    pub skipped_duplicate: usize,
}

impl InsertSummary {
    pub fn merge(&mut self, other: Self) {
        self.inserted += other.inserted;
        self.skipped_duplicate += other.skipped_duplicate;
    }
}

const INSERT_SQL: &str = "
    INSERT OR IGNORE INTO events (
        source, occurred_at, model,
        input_tokens, output_tokens, cache_read_tokens,
        cache_write_5m_tokens, cache_write_1h_tokens,
        request_id, content_hash,
        session_id, project_id, workspace_id, api_key_id, raw
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
";

/// Insert each event idempotently. Duplicates (matched by either unique
/// partial index) are silently skipped and counted into the returned summary.
pub async fn insert_events(database: &Database, events: &[Event]) -> Result<InsertSummary> {
    if events.is_empty() {
        return Ok(InsertSummary::default());
    }

    let mut transaction = database.pool().begin().await?;
    let mut summary = InsertSummary::default();

    for event in events {
        let result = sqlx::query(INSERT_SQL)
            .bind(&event.source)
            .bind(event.occurred_at)
            .bind(&event.model)
            .bind(i64::try_from(event.input_tokens).unwrap_or(i64::MAX))
            .bind(i64::try_from(event.output_tokens).unwrap_or(i64::MAX))
            .bind(i64::try_from(event.cache_read_tokens).unwrap_or(i64::MAX))
            .bind(i64::try_from(event.cache_write_5m_tokens).unwrap_or(i64::MAX))
            .bind(i64::try_from(event.cache_write_1h_tokens).unwrap_or(i64::MAX))
            .bind(event.request_id.as_deref())
            .bind(event.content_hash.as_deref())
            .bind(event.session_id.as_deref())
            .bind(event.project_id.as_deref())
            .bind(event.workspace_id.as_deref())
            .bind(event.api_key_id.as_deref())
            .bind(event.raw.as_deref())
            .execute(&mut *transaction)
            .await?;

        if result.rows_affected() == 0 {
            summary.skipped_duplicate += 1;
        } else {
            summary.inserted += 1;
        }
    }

    transaction.commit().await?;
    debug!(?summary, count = events.len(), "insert_events committed");
    Ok(summary)
}

/// Total event count. Convenience for smoke tests and the `health` endpoint.
pub async fn count_events(database: &Database) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(database.pool())
        .await?;
    Ok(row.0)
}

/// Sanity-check that the seed `sources` rows are present. Returns the list
/// of `kind` values the database knows about. Useful for tests.
pub async fn list_source_kinds(database: &Database) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT kind FROM sources ORDER BY kind")
        .fetch_all(database.pool())
        .await?;
    Ok(rows.into_iter().map(|(k,)| k).collect())
}
