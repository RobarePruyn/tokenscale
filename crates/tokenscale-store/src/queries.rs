//! Read-side query helpers for the dashboard.
//!
//! Each function in this module corresponds to one HTTP endpoint exposed by
//! `tokenscale-server`. SQL stays here; the server crate is pure transport.
//!
//! Date handling: timestamps are stored as ISO-8601 strings with millisecond
//! precision, so `date(occurred_at)` extracts the UTC YYYY-MM-DD portion and
//! the comparison `date(occurred_at) BETWEEN ?from AND ?to` works as expected
//! when both bounds are passed as `YYYY-MM-DD`. Both bounds are inclusive.
//!
//! Provider filtering: passing `"all"` (a sentinel agreed with the HTTP layer)
//! disables filtering; any other value joins through `sources.provider`.
//!
//! Phase 1 keeps these queries simple and indexed by `events(occurred_at)` /
//! `events(model, occurred_at)`. We will revisit if a query plan calls for a
//! materialized view or a pre-aggregated daily table.

use serde::Serialize;

use crate::error::Result;
use crate::Database;

/// Sentinel value the HTTP layer passes when the user has selected "all
/// providers" rather than a specific one. Kept here, not on the server side,
/// so SQL and HTTP stay in agreement about it.
pub const ALL_PROVIDERS: &str = "all";

// ----------------------------------------------------------------------------
// daily_usage
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DailyUsageFlatRow {
    /// `YYYY-MM-DD`, UTC.
    pub date: String,
    pub model: String,
    pub total_tokens: i64,
}

/// Sum of all token types per (UTC date, model). Returned in
/// `(date ASC, model ASC)` order so the server can group it into the nested
/// shape the frontend's stacked area chart wants without a second sort.
pub async fn daily_usage(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
) -> Result<Vec<DailyUsageFlatRow>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT
             date(events.occurred_at) AS day,
             events.model              AS model,
             SUM(events.input_tokens
                 + events.output_tokens
                 + events.cache_read_tokens
                 + events.cache_write_5m_tokens
                 + events.cache_write_1h_tokens) AS total_tokens
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ? AND ?
            AND (? = ? OR sources.provider = ?)
          GROUP BY day, events.model
          ORDER BY day ASC, events.model ASC",
    )
    .bind(from_date)
    .bind(to_date)
    .bind(provider_filter)
    .bind(ALL_PROVIDERS)
    .bind(provider_filter)
    .fetch_all(database.pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(|(date, model, total_tokens)| DailyUsageFlatRow {
            date,
            model,
            total_tokens,
        })
        .collect())
}

// ----------------------------------------------------------------------------
// daily_usage_breakdown — per-token-type rollup per (date, model)
// ----------------------------------------------------------------------------

/// One row per (UTC date, model) with per-token-type sums broken out.
/// Returned in `(date ASC, model ASC)` order so the server can group it
/// into the nested by-date shape without a second sort.
#[derive(Debug, Clone, Serialize)]
pub struct DailyUsageBreakdownRow {
    /// `YYYY-MM-DD`, UTC.
    pub date: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
}

/// Per-(date, model) sums broken out by token type. Successor to
/// `daily_usage`, which collapses all token types into a single total —
/// retained for the simpler historical callers.
pub async fn daily_usage_breakdown(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
) -> Result<Vec<DailyUsageBreakdownRow>> {
    // The 7-element tuple is the row shape sqlx::query_as binds to; defining
    // a struct with `derive(FromRow)` to get the same thing would require
    // moving sqlx imports into the public API. Local allow.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT
             date(events.occurred_at) AS day,
             events.model              AS model,
             COALESCE(SUM(events.input_tokens), 0),
             COALESCE(SUM(events.output_tokens), 0),
             COALESCE(SUM(events.cache_read_tokens), 0),
             COALESCE(SUM(events.cache_write_5m_tokens), 0),
             COALESCE(SUM(events.cache_write_1h_tokens), 0)
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ? AND ?
            AND (? = ? OR sources.provider = ?)
          GROUP BY day, events.model
          ORDER BY day ASC, events.model ASC",
    )
    .bind(from_date)
    .bind(to_date)
    .bind(provider_filter)
    .bind(ALL_PROVIDERS)
    .bind(provider_filter)
    .fetch_all(database.pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                date,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_5m_tokens,
                cache_write_1h_tokens,
            )| DailyUsageBreakdownRow {
                date,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_5m_tokens,
                cache_write_1h_tokens,
            },
        )
        .collect())
}

// ----------------------------------------------------------------------------
// usage_by_model
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UsageByModelRow {
    pub model: String,
    pub event_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
}

pub async fn usage_by_model(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
) -> Result<Vec<UsageByModelRow>> {
    let rows: Vec<(String, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT
             events.model,
             COUNT(*)                            AS event_count,
             COALESCE(SUM(events.input_tokens), 0),
             COALESCE(SUM(events.output_tokens), 0),
             COALESCE(SUM(events.cache_read_tokens), 0),
             COALESCE(SUM(events.cache_write_5m_tokens), 0),
             COALESCE(SUM(events.cache_write_1h_tokens), 0)
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ? AND ?
            AND (? = ? OR sources.provider = ?)
          GROUP BY events.model
          ORDER BY (COALESCE(SUM(events.input_tokens), 0)
                    + COALESCE(SUM(events.output_tokens), 0)
                    + COALESCE(SUM(events.cache_read_tokens), 0)
                    + COALESCE(SUM(events.cache_write_5m_tokens), 0)
                    + COALESCE(SUM(events.cache_write_1h_tokens), 0)) DESC",
    )
    .bind(from_date)
    .bind(to_date)
    .bind(provider_filter)
    .bind(ALL_PROVIDERS)
    .bind(provider_filter)
    .fetch_all(database.pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                model,
                event_count,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_5m_tokens,
                cache_write_1h_tokens,
            )| UsageByModelRow {
                model,
                event_count,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_5m_tokens,
                cache_write_1h_tokens,
            },
        )
        .collect())
}

// ----------------------------------------------------------------------------
// recent_sessions
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RecentSessionRow {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub first_event_at: String,
    pub last_event_at: String,
    pub event_count: i64,
    pub total_tokens: i64,
}

pub async fn recent_sessions(database: &Database, limit: i64) -> Result<Vec<RecentSessionRow>> {
    // The 6-element tuple is the row shape sqlx::query_as binds to; refactoring
    // it into a struct just to satisfy `type_complexity` would add ceremony
    // without any clarity win, so it's silenced locally.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Option<String>, Option<String>, String, String, i64, i64)> = sqlx::query_as(
        "SELECT
             session_id,
             project_id,
             MIN(occurred_at) AS first_event_at,
             MAX(occurred_at) AS last_event_at,
             COUNT(*)         AS event_count,
             SUM(input_tokens + output_tokens + cache_read_tokens
                 + cache_write_5m_tokens + cache_write_1h_tokens) AS total_tokens
           FROM events
          WHERE session_id IS NOT NULL
          GROUP BY session_id, project_id
          ORDER BY last_event_at DESC
          LIMIT ?",
    )
    .bind(limit)
    .fetch_all(database.pool())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(session_id, project_id, first_event_at, last_event_at, event_count, total_tokens)| {
                RecentSessionRow {
                    session_id,
                    project_id,
                    first_event_at,
                    last_event_at,
                    event_count,
                    total_tokens,
                }
            },
        )
        .collect())
}

// ----------------------------------------------------------------------------
// health summary
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct HealthSummary {
    pub total_events: i64,
    pub providers: Vec<String>,
}

pub async fn health_summary(database: &Database) -> Result<HealthSummary> {
    let total_events = crate::events::count_events(database).await?;
    let providers: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT provider FROM sources ORDER BY provider")
            .fetch_all(database.pool())
            .await?;
    Ok(HealthSummary {
        total_events,
        providers: providers.into_iter().map(|(p,)| p).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insert_events;
    use chrono::Utc;
    use tokenscale_core::Event;

    fn make_event(date: &str, model: &str, request_id: &str, total_tokens: u64) -> Event {
        let occurred_at = chrono::DateTime::parse_from_rfc3339(date)
            .unwrap()
            .with_timezone(&Utc);
        Event {
            source: "claude_code".to_owned(),
            occurred_at,
            model: model.to_owned(),
            input_tokens: total_tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("sess1".to_owned()),
            project_id: Some("/p".to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        }
    }

    async fn fixture_database() -> Database {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let events = vec![
            make_event("2026-04-20T01:00:00Z", "claude-opus-4-7", "r1", 100),
            make_event("2026-04-20T02:00:00Z", "claude-opus-4-7", "r2", 50),
            make_event("2026-04-20T03:00:00Z", "claude-sonnet-4-6", "r3", 30),
            make_event("2026-04-21T01:00:00Z", "claude-opus-4-7", "r4", 200),
            make_event("2026-04-21T02:00:00Z", "claude-sonnet-4-6", "r5", 70),
            make_event("2026-04-22T01:00:00Z", "claude-haiku-4-5", "r6", 5),
        ];
        insert_events(&database, &events).await.unwrap();
        database
    }

    #[tokio::test]
    async fn daily_usage_groups_by_date_and_model() {
        let database = fixture_database().await;
        let rows = daily_usage(&database, "2026-04-20", "2026-04-22", ALL_PROVIDERS)
            .await
            .unwrap();

        assert_eq!(rows.len(), 5);
        let by_key: std::collections::HashMap<(String, String), i64> = rows
            .iter()
            .map(|r| ((r.date.clone(), r.model.clone()), r.total_tokens))
            .collect();
        assert_eq!(
            by_key[&("2026-04-20".to_owned(), "claude-opus-4-7".to_owned())],
            150
        );
        assert_eq!(
            by_key[&("2026-04-20".to_owned(), "claude-sonnet-4-6".to_owned())],
            30
        );
        assert_eq!(
            by_key[&("2026-04-21".to_owned(), "claude-opus-4-7".to_owned())],
            200
        );
        assert_eq!(
            by_key[&("2026-04-22".to_owned(), "claude-haiku-4-5".to_owned())],
            5
        );
    }

    #[tokio::test]
    async fn daily_usage_filters_by_date_window() {
        let database = fixture_database().await;
        let rows = daily_usage(&database, "2026-04-21", "2026-04-21", ALL_PROVIDERS)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.date == "2026-04-21"));
    }

    #[tokio::test]
    async fn daily_usage_filters_by_provider() {
        let database = fixture_database().await;
        let rows = daily_usage(&database, "2026-04-20", "2026-04-22", "anthropic")
            .await
            .unwrap();
        assert_eq!(rows.len(), 5);

        let zero_rows = daily_usage(&database, "2026-04-20", "2026-04-22", "openai")
            .await
            .unwrap();
        assert!(zero_rows.is_empty());
    }

    #[tokio::test]
    async fn daily_usage_breakdown_returns_per_token_type_sums() {
        let database = fixture_database().await;
        let rows = daily_usage_breakdown(&database, "2026-04-20", "2026-04-22", ALL_PROVIDERS)
            .await
            .unwrap();
        // 5 (date, model) groups same as daily_usage above.
        assert_eq!(rows.len(), 5);
        // Every row's input_tokens equals the total in the fixture (since
        // the fixture only sets input_tokens; everything else is zero).
        let opus_apr20 = rows
            .iter()
            .find(|r| r.date == "2026-04-20" && r.model == "claude-opus-4-7")
            .unwrap();
        assert_eq!(opus_apr20.input_tokens, 150);
        assert_eq!(opus_apr20.output_tokens, 0);
        assert_eq!(opus_apr20.cache_read_tokens, 0);
    }

    #[tokio::test]
    async fn usage_by_model_orders_by_total_descending() {
        let database = fixture_database().await;
        let rows = usage_by_model(&database, "2026-04-20", "2026-04-22", ALL_PROVIDERS)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].model, "claude-opus-4-7");
        assert_eq!(rows[0].event_count, 3);
        assert_eq!(rows[0].input_tokens, 350);
        assert_eq!(rows.last().unwrap().model, "claude-haiku-4-5");
    }

    #[tokio::test]
    async fn recent_sessions_returns_one_row_per_session() {
        let database = fixture_database().await;
        let rows = recent_sessions(&database, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id.as_deref(), Some("sess1"));
        assert_eq!(rows[0].event_count, 6);
        assert_eq!(rows[0].total_tokens, 455);
    }

    #[tokio::test]
    async fn health_summary_reports_event_count_and_providers() {
        let database = fixture_database().await;
        let summary = health_summary(&database).await.unwrap();
        assert_eq!(summary.total_events, 6);
        assert_eq!(summary.providers, vec!["anthropic".to_owned()]);
    }

    #[tokio::test]
    async fn health_summary_on_empty_database_reports_zero() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let summary = health_summary(&database).await.unwrap();
        assert_eq!(summary.total_events, 0);
        assert_eq!(summary.providers, vec!["anthropic".to_owned()]);
    }
}
