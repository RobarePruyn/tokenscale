//! Audit queries — read-only inspection used by `tokenscale audit ...`
//! subcommands.
//!
//! v0.1.13 adds `pricing-launch-dates`, the Section 4 pre-release hard
//! gate. The dashboard's per-event time-anchored pricing returns `None`
//! when an event predates every `pricing.valid_from` row for its
//! `(provider, model)` pair (Decision D4 in the cost-time-anchoring
//! roadmap). Ship a release where that count is non-trivial and the
//! Cost (USD) view silently renders "—" for whole buckets — exactly the
//! v0.1.0-style regression the time-anchoring work is meant to prevent.
//!
//! The audit aggregates per (provider, model):
//!   * Total events seen in the DB.
//!   * Earliest event's `occurred_at` date.
//!   * Earliest `pricing.valid_from` for the pair, if any row exists.
//!   * Number of events whose date precedes the earliest valid_from
//!     (or whose pair has no pricing row at all).
//!
//! The CLI subcommand prints the table, prints the total pre-launch
//! count, and exits non-zero when that total is > 0 so CI / release
//! checklists can gate on it.

use serde::Serialize;

use crate::error::Result;
use crate::Database;

/// One row per `(provider, model)` pair present in `events`. A pair
/// with `events_pre_launch == 0` is healthy; any non-zero count means
/// the v0.1.13 time-anchored Cost (USD) view would render "—" for
/// those events.
///
/// `events_pre_launch` collapses two distinct failure modes — the CLI
/// pulls them apart for the release gate:
///   * `earliest_valid_from.is_some()` AND `events_pre_launch > 0`:
///     events on a priced model predate its earliest `valid_from`. This
///     is the wrong-launch-date regression v0.1.13 gates on.
///   * `earliest_valid_from.is_none()`: the model has no pricing row at
///     all (e.g. the `<synthetic>` admin-API aggregate). Every event
///     for that pair counts toward `events_pre_launch` because the
///     correlated-MIN subquery returns NULL. The CLI surfaces this as a
///     separate "unpriced model" bucket, NOT a gate failure.
#[derive(Debug, Clone, Serialize)]
pub struct PricingLaunchDateAuditRow {
    pub provider: String,
    pub model: String,
    pub events_total: i64,
    pub events_pre_launch: i64,
    /// `YYYY-MM-DD` of the earliest event for this pair. Always present
    /// (we wouldn't be in the result set if no events existed).
    pub earliest_event_date: String,
    /// `YYYY-MM-DD` of the earliest `pricing.valid_from` for this pair,
    /// or `None` when the model has no pricing row at all (the strictest
    /// failure mode — every event is pre-launch).
    pub earliest_valid_from: Option<String>,
}

/// Per-(provider, model) audit of pre-launch events: events whose
/// `occurred_at` precedes every `pricing.valid_from` row for their pair.
/// Used by the `tokenscale audit pricing-launch-dates` subcommand as the
/// v0.1.13 release-gate query.
///
/// Returns an empty vec when the DB has no events, which the caller
/// should treat as "zero pre-launch events" (the gate passes — but the
/// gate is also vacuous on a freshly-initialised DB).
pub async fn audit_pricing_launch_dates(
    database: &Database,
) -> Result<Vec<PricingLaunchDateAuditRow>> {
    // The correlated subquery `(SELECT MIN(valid_from) FROM pricing
    // WHERE provider = X AND model = Y)` returns NULL when no row
    // matches — IS NULL covers the "no pricing row at all" failure
    // mode, while the date comparison covers "every pricing row's
    // valid_from is after the event date". The ELSE 0 in the CASE is
    // the healthy branch.
    let rows: Vec<PricingLaunchDateAuditRow> = sqlx::query_as(
        "SELECT
             sources.provider AS provider,
             events.model     AS model,
             COUNT(*)         AS events_total,
             SUM(CASE
                     WHEN (SELECT MIN(valid_from)
                             FROM pricing
                            WHERE provider = sources.provider
                              AND model    = COALESCE(ma.canonical, events.model)) IS NULL
                       THEN 1
                     WHEN date(events.occurred_at) <
                          (SELECT MIN(valid_from)
                             FROM pricing
                            WHERE provider = sources.provider
                              AND model    = COALESCE(ma.canonical, events.model))
                       THEN 1
                     ELSE 0
                 END)         AS events_pre_launch,
             MIN(date(events.occurred_at)) AS earliest_event_date,
             (SELECT MIN(valid_from)
                FROM pricing
               WHERE provider = sources.provider
                 AND model    = COALESCE(ma.canonical, events.model)) AS earliest_valid_from
           FROM events
           JOIN sources ON sources.kind = events.source
           -- D1: map alias model IDs to their canonical row key. events.model
           -- stays raw for grouping/display; only resolution uses the mapping.
           LEFT JOIN model_aliases ma
                  ON ma.provider = sources.provider
                 AND ma.raw = events.model
          GROUP BY sources.provider, events.model
          ORDER BY sources.provider, events.model",
    )
    .fetch_all(database.pool())
    .await?;

    Ok(rows)
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for PricingLaunchDateAuditRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            provider: row.try_get("provider")?,
            model: row.try_get("model")?,
            events_total: row.try_get("events_total")?,
            events_pre_launch: row.try_get("events_pre_launch")?,
            earliest_event_date: row.try_get("earliest_event_date")?,
            earliest_valid_from: row.try_get("earliest_valid_from")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tokenscale_core::{Event, PricingFile};

    use crate::{insert_events, sync_pricing};

    const SINGLE_ROW_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-sonnet-4-6"]
display_name              = "Claude Sonnet 4.6"
valid_from                = "2026-03-01"
input_usd_per_mtok        = 3.00
output_usd_per_mtok       = 15.00
cache_read_usd_per_mtok   = 0.30
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url                = "https://example"
source_accessed_at        = "2026-05-18"
"#;

    fn event(model: &str, day: u32, request_id: &str) -> Event {
        Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, day, 12, 0, 0).unwrap(),
            model: model.to_owned(),
            input_tokens: 100,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: None,
            project_id: None,
            workspace_id: None,
            api_key_id: None,
            uuid: None,
            parent_uuid: None,
            git_branch: None,
            raw: None,
        }
    }

    #[tokio::test]
    async fn empty_db_returns_no_rows() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn pair_with_no_pricing_row_at_all_is_all_pre_launch() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        // Sync the single-row pricing — but use a DIFFERENT model in
        // the event so there's no matching row at all.
        let pricing = PricingFile::parse(SINGLE_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();
        insert_events(&database, &[event("claude-haiku-99", 21, "r-1")])
            .await
            .unwrap();

        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.model, "claude-haiku-99");
        assert_eq!(row.events_total, 1);
        assert_eq!(row.events_pre_launch, 1);
        assert_eq!(row.earliest_event_date, "2026-04-21");
        assert!(row.earliest_valid_from.is_none());
    }

    #[tokio::test]
    async fn event_predating_valid_from_is_counted_pre_launch() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(SINGLE_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();
        // valid_from = 2026-03-01; an event in February → pre-launch.
        let pre = Event {
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 15, 12, 0, 0).unwrap(),
            ..event("claude-sonnet-4-6", 15, "r-pre")
        };
        insert_events(&database, &[pre]).await.unwrap();

        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.events_total, 1);
        assert_eq!(row.events_pre_launch, 1);
        assert_eq!(row.earliest_valid_from.as_deref(), Some("2026-03-01"));
    }

    #[tokio::test]
    async fn event_on_or_after_valid_from_is_not_pre_launch() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(SINGLE_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();
        // Two events on Apr 21 — both well after valid_from 2026-03-01.
        insert_events(
            &database,
            &[
                event("claude-sonnet-4-6", 21, "r-1"),
                event("claude-sonnet-4-6", 22, "r-2"),
            ],
        )
        .await
        .unwrap();

        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.events_total, 2);
        assert_eq!(row.events_pre_launch, 0);
    }

    #[tokio::test]
    async fn mixed_pre_and_post_launch_split_correctly() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(SINGLE_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();
        // One pre-launch (Feb 15), one post (Apr 21).
        let pre = Event {
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 15, 12, 0, 0).unwrap(),
            ..event("claude-sonnet-4-6", 15, "r-pre")
        };
        let post = event("claude-sonnet-4-6", 21, "r-post");
        insert_events(&database, &[pre, post]).await.unwrap();

        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.events_total, 2);
        assert_eq!(row.events_pre_launch, 1);
        assert_eq!(row.earliest_event_date, "2026-02-15");
    }

    #[tokio::test]
    async fn boundary_event_at_valid_from_is_post_launch() {
        // valid_from <= event_date passes; the audit's `date(event) <
        // valid_from` boundary check uses strict less-than, mirroring
        // the lookup_pricing semantics.
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let pricing = PricingFile::parse(SINGLE_ROW_TOML).unwrap();
        sync_pricing(&database, &pricing).await.unwrap();
        let on_boundary = Event {
            occurred_at: Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap(),
            ..event("claude-sonnet-4-6", 1, "r-boundary")
        };
        insert_events(&database, &[on_boundary]).await.unwrap();

        let rows = audit_pricing_launch_dates(&database).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].events_pre_launch, 0);
    }
}
