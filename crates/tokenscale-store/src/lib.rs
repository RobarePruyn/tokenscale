//! `tokenscale-store` — SQLite schema, migrations, and queries.
//!
//! All SQL lives in this crate. Other crates speak in terms of the domain
//! types defined in `tokenscale-core` and call typed query functions exposed
//! here.
//!
//! The migrations directory is `migrations/` at the workspace root (not
//! inside this crate) so that operators can inspect the schema without
//! spelunking into a Cargo target tree. `sqlx::migrate!` references it via a
//! relative path.
//!
//! Phase 1 uses sqlx's runtime-checked query API (`sqlx::query`,
//! `sqlx::query_as`) rather than the compile-time-checked macros. The
//! trade-off — losing compile-time SQL verification in exchange for not
//! requiring a `.sqlx/` cache and `cargo sqlx prepare` workflow — is
//! documented in `docs/decisions.md`. The graduation to query! macros is a
//! follow-up commit once the schema stabilizes.

mod database;
mod error;
mod events;
mod files;
mod queries;
mod subscriptions;

pub use database::Database;
pub use error::{Result, StoreError};
pub use events::{count_events, insert_events, list_source_kinds, InsertSummary};
pub use files::{get_file_state, upsert_file_state, FileState};
pub use queries::{
    daily_usage, daily_usage_breakdown, health_summary, list_models_in_window,
    list_projects_with_totals, recent_sessions, usage_by_model, DailyUsageBreakdownRow,
    DailyUsageFlatRow, Granularity, HealthSummary, ModelSummaryRow, ProjectSummaryRow,
    RecentSessionRow, UsageByModelRow, ALL_PROVIDERS,
};
pub use subscriptions::{
    delete_subscription, insert_subscription, list_subscriptions, Subscription,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tokenscale_core::Event;

    fn sample_event() -> Event {
        Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 0, 29, 54).unwrap(),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: 6,
            output_tokens: 136,
            cache_read_tokens: 16_410,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 8_837,
            request_id: Some("req_011CaFyK1b4pQUFLfXGAuJbw".to_owned()),
            content_hash: None,
            session_id: Some("455218e7-8747-410f-a4f3-11bf11c53cc6".to_owned()),
            project_id: Some(
                "/Users/Robare/Library/Mobile Documents/com~apple~CloudDocs/Dev/QTrial".to_owned(),
            ),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        }
    }

    #[tokio::test]
    async fn migrations_apply_and_seed_sources() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let kinds = list_source_kinds(&database).await?;
        assert_eq!(
            kinds,
            vec!["admin_api".to_owned(), "claude_code".to_owned()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn insert_events_is_idempotent_on_request_id() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let event = sample_event();

        let first = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(
            first,
            InsertSummary {
                inserted: 1,
                skipped_duplicate: 0
            }
        );

        let second = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(
            second,
            InsertSummary {
                inserted: 0,
                skipped_duplicate: 1
            }
        );

        assert_eq!(count_events(&database).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn insert_events_dedupes_on_content_hash_when_request_id_absent() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let mut event = sample_event();
        event.request_id = None;
        event.content_hash = Some("hash-aaa".to_owned());

        insert_events(&database, std::slice::from_ref(&event)).await?;
        let second = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(second.skipped_duplicate, 1);
        assert_eq!(count_events(&database).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn file_state_roundtrip() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let path = "/tmp/example.jsonl";
        assert!(get_file_state(&database, "claude_code", path)
            .await?
            .is_none());

        upsert_file_state(&database, "claude_code", path, 12_345).await?;
        let state = get_file_state(&database, "claude_code", path)
            .await?
            .unwrap();
        assert_eq!(state.mtime_ns, 12_345);
        assert_eq!(state.source, "claude_code");

        upsert_file_state(&database, "claude_code", path, 99_999).await?;
        let state = get_file_state(&database, "claude_code", path)
            .await?
            .unwrap();
        assert_eq!(state.mtime_ns, 99_999);
        Ok(())
    }
}
