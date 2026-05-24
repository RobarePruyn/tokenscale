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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InsertSummary {
    /// Rows that landed in the database.
    pub inserted: usize,
    /// Rows that hit a unique-index conflict — already in the database.
    /// Includes uuid-collision skips counted in `uuid_duplicate_indices`
    /// below, plus the v0.1.0-era request_id / content_hash dedup hits.
    /// (Strict bookkeeping note: when a uuid duplicate is caught by the
    /// pre-check, the INSERT is skipped entirely so it doesn't double-count
    /// here — `skipped_duplicate` increments only via INSERT OR IGNORE's
    /// `rows_affected = 0` path.)
    pub skipped_duplicate: usize,
    /// v0.1.16: indices (into the input `events` slice) of events that
    /// were skipped due to a uuid collision detected by the pre-check.
    /// Caller uses these to emit per-event warning logs with whatever
    /// per-event provenance (JSONL file path + line number) the caller
    /// tracked at parse time. See `docs/roadmap-1b-ii-parser-captures.md`
    /// § 3 + I1.5 (Option A — chosen for smaller-diff and clean
    /// separation between store and ingest concerns).
    pub uuid_duplicate_indices: Vec<usize>,
}

impl InsertSummary {
    pub fn merge(&mut self, other: Self) {
        self.inserted += other.inserted;
        self.skipped_duplicate += other.skipped_duplicate;
        // Indices are caller-relative (positions in THAT call's events
        // slice) so merging across calls would conflate origins.
        // Callers that merge summaries should drain indices first.
        self.uuid_duplicate_indices
            .extend(other.uuid_duplicate_indices);
    }
}

const INSERT_SQL: &str = "
    INSERT OR IGNORE INTO events (
        source, occurred_at, model,
        input_tokens, output_tokens, cache_read_tokens,
        cache_write_5m_tokens, cache_write_1h_tokens,
        request_id, content_hash,
        session_id, project_id, workspace_id, api_key_id,
        uuid, parent_uuid, git_branch,
        raw
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
";

/// v0.1.16 pre-check: lookup by (source, uuid) and return the
/// existing row's request_id (if any). Used to distinguish
/// "rescan of an unchanged file" (same uuid AND same request_id —
/// normal, silent dedup via INSERT OR IGNORE on the request_id
/// index) from "CC behavior change" (same uuid, DIFFERENT
/// request_id — exceptional, loud).
///
/// The SQL UNIQUE constraint remains as the final defense — even if
/// the pre-check passes (e.g. two parallel scans inserting the same
/// uuid simultaneously), `INSERT OR IGNORE` would catch the race.
const UUID_PRECHECK_SQL: &str =
    "SELECT request_id FROM events WHERE source = ? AND uuid = ? LIMIT 1";

/// Insert each event idempotently. Duplicates (matched by either unique
/// partial index) are silently skipped and counted into the returned
/// summary. uuid-collision skips are surfaced separately via
/// `uuid_duplicate_indices` so callers can emit per-event warnings
/// with their own per-event provenance metadata.
pub async fn insert_events(database: &Database, events: &[Event]) -> Result<InsertSummary> {
    if events.is_empty() {
        return Ok(InsertSummary::default());
    }

    let mut transaction = database.pool().begin().await?;
    let mut summary = InsertSummary::default();

    for (index, event) in events.iter().enumerate() {
        // v0.1.16 pre-check: catch GENUINE uuid duplicates (same uuid,
        // DIFFERENT request_id) BEFORE INSERT OR IGNORE so the caller
        // can distinguish them from request_id rescans (same uuid AND
        // same request_id, which is just "we re-scanned an unchanged
        // file"). The latter is normal and noisy; the former is the
        // exceptional CC-behavior-change signal Phase 0 flagged as the
        // load-bearing correctness concern.
        //
        // Both cases would also be caught by INSERT OR IGNORE on
        // either the (source, request_id) or (source, uuid) UNIQUE
        // index, but `rows_affected = 0` doesn't tell us WHICH
        // constraint fired. The pre-check is the cost of
        // distinguishability.
        if let Some(uuid) = event.uuid.as_deref() {
            let existing: Option<(Option<String>,)> = sqlx::query_as(UUID_PRECHECK_SQL)
                .bind(&event.source)
                .bind(uuid)
                .fetch_optional(&mut *transaction)
                .await?;
            if let Some((existing_request_id,)) = existing {
                if existing_request_id == event.request_id {
                    // Same uuid AND same request_id → same event,
                    // just being re-ingested. Fall through to the
                    // INSERT OR IGNORE path so the dedup counts via
                    // the existing skipped_duplicate channel.
                } else {
                    // Same uuid, DIFFERENT request_id → exceptional.
                    // The defense activates: skip + queue for the
                    // caller's warning log.
                    summary.uuid_duplicate_indices.push(index);
                    continue;
                }
            }
        }

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
            .bind(event.uuid.as_deref())
            .bind(event.parent_uuid.as_deref())
            .bind(event.git_branch.as_deref())
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
    debug!(
        inserted = summary.inserted,
        skipped_duplicate = summary.skipped_duplicate,
        uuid_duplicates = summary.uuid_duplicate_indices.len(),
        count = events.len(),
        "insert_events committed"
    );
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
