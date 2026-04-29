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

/// Time bucket granularity for the per-period chart query. The dashboard's
/// "Granularity" control maps to this; the SQL `GROUP BY` clause swaps
/// based on the variant.
///
/// All three variants produce `YYYY-MM-DD` bucket labels — the start date
/// of the bucket — so the frontend can render them on a single x-axis
/// formatter without special-casing each granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Day,
    Week,
    Month,
}

impl Granularity {
    /// Parse the value of the `?granularity=` query string. Falls back to
    /// `Day` for unknown or missing inputs so the API stays forgiving.
    #[must_use]
    pub fn parse_or_default(raw: Option<&str>) -> Self {
        match raw {
            Some("week") => Self::Week,
            Some("month") => Self::Month,
            _ => Self::Day,
        }
    }

    /// SQLite expression that produces the bucket label for one event row.
    /// Used in both `SELECT` (as the alias) and `GROUP BY`.
    ///
    /// - Day: `date(occurred_at)` → `2026-04-29`
    /// - Week: Monday of the week containing `occurred_at` → `2026-04-27`
    /// - Month: first of the month → `2026-04-01`
    fn bucket_expression(self) -> &'static str {
        match self {
            // SQLite's `weekday 0` modifier yields the next Sunday on or after
            // the input. Subtracting 6 days produces the Monday at the start
            // of that ISO-style week.
            Self::Week => "date(events.occurred_at, 'weekday 0', '-6 days')",
            Self::Month => "date(events.occurred_at, 'start of month')",
            Self::Day => "date(events.occurred_at)",
        }
    }
}

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

/// Per-(bucket, model) sums broken out by token type. Successor to
/// `daily_usage`, which collapses all token types into a single total —
/// retained for the simpler historical callers.
///
/// `project_filter` is empty for "all projects", non-empty for a specific
/// allow-list of `project_id` values (Claude Code's `cwd` strings). The
/// IN-clause is built dynamically with `sqlx::QueryBuilder` so each project
/// argument is properly parameterized.
///
/// `granularity` swaps the bucket size between day, ISO-week (starting
/// Monday), and calendar month. Bucket labels are always YYYY-MM-DD start
/// dates so the frontend can format them uniformly.
pub async fn daily_usage_breakdown(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
    project_filter: &[String],
    granularity: Granularity,
) -> Result<Vec<DailyUsageBreakdownRow>> {
    use sqlx::QueryBuilder;

    let bucket_expr = granularity.bucket_expression();

    let mut builder: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new("SELECT ");
    builder.push(bucket_expr);
    builder.push(
        " AS bucket,
             events.model              AS model,
             COALESCE(SUM(events.input_tokens), 0),
             COALESCE(SUM(events.output_tokens), 0),
             COALESCE(SUM(events.cache_read_tokens), 0),
             COALESCE(SUM(events.cache_write_5m_tokens), 0),
             COALESCE(SUM(events.cache_write_1h_tokens), 0)
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ",
    );
    builder.push_bind(from_date.to_owned());
    builder.push(" AND ");
    builder.push_bind(to_date.to_owned());
    builder.push(" AND (");
    builder.push_bind(provider_filter.to_owned());
    builder.push(" = ");
    builder.push_bind(ALL_PROVIDERS.to_owned());
    builder.push(" OR sources.provider = ");
    builder.push_bind(provider_filter.to_owned());
    builder.push(")");

    if !project_filter.is_empty() {
        builder.push(" AND events.project_id IN (");
        let mut separated = builder.separated(", ");
        for project in project_filter {
            separated.push_bind(project.clone());
        }
        separated.push_unseparated(")");
    }

    builder.push(" GROUP BY bucket, events.model ORDER BY bucket ASC, events.model ASC");

    // The 7-element tuple is the row shape sqlx binds to; defining a struct
    // with `derive(FromRow)` to get the same thing would force sqlx imports
    // into the public API. Local allow.
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, i64, i64, i64, i64, i64)> =
        builder.build_query_as().fetch_all(database.pool()).await?;

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
// list_models_in_window — models present in the window
// ----------------------------------------------------------------------------

/// One distinct `model` identifier in the window with its window-level
/// total, ordered by total tokens descending. Mirrors
/// `list_projects_with_totals` for the model dimension.
///
/// This query intentionally **does not** take a project filter — its
/// purpose is to drive the dashboard's Model chip list, which should
/// reflect "all models in this provider/window" regardless of what
/// the user has done with the project filter. Using project-filtered
/// model data for the chip list would mean clicking "Select none" on
/// projects causes the model chips to vanish, leaving the user no way
/// back.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSummaryRow {
    pub model: String,
    pub total_tokens: i64,
}

pub async fn list_models_in_window(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
) -> Result<Vec<ModelSummaryRow>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT
             events.model,
             COALESCE(SUM(events.input_tokens
                + events.output_tokens
                + events.cache_read_tokens
                + events.cache_write_5m_tokens
                + events.cache_write_1h_tokens), 0) AS total_tokens
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ? AND ?
            AND (? = ? OR sources.provider = ?)
          GROUP BY events.model
          HAVING total_tokens > 0
          ORDER BY total_tokens DESC, events.model ASC",
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
        .map(|(model, total_tokens)| ModelSummaryRow {
            model,
            total_tokens,
        })
        .collect())
}

// ----------------------------------------------------------------------------
// list_projects_with_totals — projects present in the window
// ----------------------------------------------------------------------------

/// One distinct `project_id` (Claude Code `cwd` path) with its window-level
/// rollup, ordered by total tokens descending so the dashboard can render
/// the busiest projects first.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummaryRow {
    pub project_id: String,
    pub event_count: i64,
    pub total_tokens: i64,
}

pub async fn list_projects_with_totals(
    database: &Database,
    from_date: &str,
    to_date: &str,
    provider_filter: &str,
) -> Result<Vec<ProjectSummaryRow>> {
    let rows: Vec<(Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT
             events.project_id,
             COUNT(*) AS event_count,
             COALESCE(SUM(events.input_tokens
                + events.output_tokens
                + events.cache_read_tokens
                + events.cache_write_5m_tokens
                + events.cache_write_1h_tokens), 0) AS total_tokens
           FROM events
           JOIN sources ON sources.kind = events.source
          WHERE date(events.occurred_at) BETWEEN ? AND ?
            AND (? = ? OR sources.provider = ?)
            AND events.project_id IS NOT NULL
          GROUP BY events.project_id
          ORDER BY total_tokens DESC",
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
        .filter_map(|(project_id, event_count, total_tokens)| {
            project_id.map(|project_id| ProjectSummaryRow {
                project_id,
                event_count,
                total_tokens,
            })
        })
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
        let rows = daily_usage_breakdown(
            &database,
            "2026-04-20",
            "2026-04-22",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
        )
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
    async fn daily_usage_breakdown_filters_by_project() {
        // Build a fixture with two projects, then filter to one.
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |project: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::DateTime::parse_from_rfc3339("2026-04-20T01:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some(project.to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        insert_events(
            &database,
            &[
                make_event("/proj/alpha", "r1", 100),
                make_event("/proj/beta", "r2", 200),
                make_event("/proj/alpha", "r3", 50),
            ],
        )
        .await
        .unwrap();

        // No filter — both projects, total 350.
        let no_filter = daily_usage_breakdown(
            &database,
            "2026-04-20",
            "2026-04-20",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
        )
        .await
        .unwrap();
        assert_eq!(no_filter.len(), 1);
        assert_eq!(no_filter[0].input_tokens, 350);

        // Filter to alpha only — total 150.
        let alpha_only = daily_usage_breakdown(
            &database,
            "2026-04-20",
            "2026-04-20",
            ALL_PROVIDERS,
            &["/proj/alpha".to_owned()],
            Granularity::Day,
        )
        .await
        .unwrap();
        assert_eq!(alpha_only.len(), 1);
        assert_eq!(alpha_only[0].input_tokens, 150);

        // Filter to both explicitly — same as no filter.
        let both = daily_usage_breakdown(
            &database,
            "2026-04-20",
            "2026-04-20",
            ALL_PROVIDERS,
            &["/proj/alpha".to_owned(), "/proj/beta".to_owned()],
            Granularity::Day,
        )
        .await
        .unwrap();
        assert_eq!(both[0].input_tokens, 350);
    }

    #[tokio::test]
    async fn daily_usage_breakdown_buckets_by_week() {
        // Insert events on three consecutive days that all fall in the same
        // ISO week (Mon-Sun). Weekly granularity should collapse them into
        // one row whose `date` is the Monday at the start of that week.
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |day: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::DateTime::parse_from_rfc3339(&format!("{day}T12:00:00Z"))
                .unwrap()
                .with_timezone(&chrono::Utc),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some("/p".to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        // 2026-04-20 is a Monday, 21 = Tue, 22 = Wed — all same ISO week.
        insert_events(
            &database,
            &[
                make_event("2026-04-20", "r1", 100),
                make_event("2026-04-21", "r2", 200),
                make_event("2026-04-22", "r3", 50),
            ],
        )
        .await
        .unwrap();

        let rows = daily_usage_breakdown(
            &database,
            "2026-04-20",
            "2026-04-26",
            ALL_PROVIDERS,
            &[],
            Granularity::Week,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2026-04-20"); // Monday of the week
        assert_eq!(rows[0].input_tokens, 350);
    }

    #[tokio::test]
    async fn daily_usage_breakdown_buckets_by_month() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |day: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::DateTime::parse_from_rfc3339(&format!("{day}T12:00:00Z"))
                .unwrap()
                .with_timezone(&chrono::Utc),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some("/p".to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        insert_events(
            &database,
            &[
                make_event("2026-04-01", "r1", 100),
                make_event("2026-04-15", "r2", 200),
                make_event("2026-05-01", "r3", 50),
            ],
        )
        .await
        .unwrap();

        let rows = daily_usage_breakdown(
            &database,
            "2026-04-01",
            "2026-05-31",
            ALL_PROVIDERS,
            &[],
            Granularity::Month,
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2026-04-01");
        assert_eq!(rows[0].input_tokens, 300);
        assert_eq!(rows[1].date, "2026-05-01");
        assert_eq!(rows[1].input_tokens, 50);
    }

    #[tokio::test]
    async fn list_projects_with_totals_orders_by_largest_first() {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |project: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::DateTime::parse_from_rfc3339("2026-04-20T01:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some(project.to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        insert_events(
            &database,
            &[
                make_event("/proj/small", "r1", 10),
                make_event("/proj/big", "r2", 1000),
                make_event("/proj/medium", "r3", 100),
            ],
        )
        .await
        .unwrap();

        let projects =
            list_projects_with_totals(&database, "2026-04-20", "2026-04-20", ALL_PROVIDERS)
                .await
                .unwrap();
        assert_eq!(projects.len(), 3);
        assert_eq!(projects[0].project_id, "/proj/big");
        assert_eq!(projects[0].total_tokens, 1000);
        assert_eq!(projects[2].project_id, "/proj/small");
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
