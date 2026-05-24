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

mod audit;
mod billing;
mod database;
mod error;
mod events;
mod factors_lookup;
mod factors_sync;
mod files;
mod impact_query;
mod pricing_lookup;
mod pricing_sync;
mod queries;
mod sessions_query;
mod subscriptions;

pub use audit::{audit_pricing_launch_dates, PricingLaunchDateAuditRow};
pub use billing::{
    delete_billing_charges_by_source, insert_billing_charges, list_billing_charges_in_window,
    sum_billing_charges_in_window, BillingChargeInsertSummary, BillingChargeRow,
};
pub use database::Database;
pub use error::{Result, StoreError};
pub use events::{count_events, insert_events, list_source_kinds, InsertSummary};
pub use factors_lookup::{lookup_environmental_factors, lookup_grid_factors};
pub use factors_sync::{sync_environmental_factors, FactorsSyncSummary};
pub use pricing_lookup::lookup_pricing;
pub use pricing_sync::{sync_pricing, PricingSyncSummary};
pub use impact_query::{aggregate_impact_by_bucket, ImpactByBucketRow, ImpactQueryFactors};
pub use files::{
    clear_file_state_for_source, delete_events_for_source, get_file_state, most_recent_scan_at,
    upsert_file_state, FileState,
};
pub use queries::{
    daily_usage, daily_usage_breakdown, health_summary, list_models_in_window,
    list_projects_with_totals, recent_sessions, usage_by_model, DailyUsageBreakdownRow,
    DailyUsageFlatRow, Granularity, HealthSummary, ModelSummaryRow, ProjectSummaryRow,
    RecentSessionRow, UsageByModelRow, ALL_PROVIDERS,
};
pub use sessions_query::{list_sessions_with_totals, SessionSummaryRow, DEFAULT_SESSION_LIMIT};
pub use subscriptions::{
    delete_subscription, insert_subscription, list_subscriptions, update_subscription, Subscription,
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
            uuid: None,
            parent_uuid: None,
            git_branch: None,
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
                skipped_duplicate: 0,
                uuid_duplicate_indices: vec![],
            }
        );

        let second = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(
            second,
            InsertSummary {
                inserted: 0,
                skipped_duplicate: 1,
                uuid_duplicate_indices: vec![],
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

        upsert_file_state(&database, "claude_code", path, 12_345, 1_024).await?;
        let state = get_file_state(&database, "claude_code", path)
            .await?
            .unwrap();
        assert_eq!(state.mtime_ns, 12_345);
        assert_eq!(state.len, 1_024);
        assert_eq!(state.source, "claude_code");

        upsert_file_state(&database, "claude_code", path, 99_999, 2_048).await?;
        let state = get_file_state(&database, "claude_code", path)
            .await?
            .unwrap();
        assert_eq!(state.mtime_ns, 99_999);
        assert_eq!(state.len, 2_048);
        Ok(())
    }

    #[tokio::test]
    async fn clear_file_state_resets_only_target_source() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        upsert_file_state(&database, "claude_code", "/a", 1, 10).await?;
        upsert_file_state(&database, "admin_api", "/b", 2, 20).await?;

        let removed = clear_file_state_for_source(&database, "claude_code").await?;
        assert_eq!(removed, 1);
        assert!(get_file_state(&database, "claude_code", "/a")
            .await?
            .is_none());
        assert!(get_file_state(&database, "admin_api", "/b")
            .await?
            .is_some());
        Ok(())
    }

    #[tokio::test]
    async fn delete_events_for_source_only_touches_target_source() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;
        let event = sample_event();
        insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(count_events(&database).await?, 1);

        let deleted = delete_events_for_source(&database, "claude_code").await?;
        assert_eq!(deleted, 1);
        assert_eq!(count_events(&database).await?, 0);

        // Different source — no-op.
        insert_events(&database, std::slice::from_ref(&event)).await?;
        let deleted = delete_events_for_source(&database, "admin_api").await?;
        assert_eq!(deleted, 0);
        assert_eq!(count_events(&database).await?, 1);
        Ok(())
    }

    // ----------------------------------------------------------------
    // v0.1.16 ingest tests (Phase 1B-ii):
    // - uuid pre-check distinguishes rescan from CC behavior change
    // - NULL uuids do not conflict (partial index excludes NULL)
    // - UNIQUE backstops the pre-check (race coverage)
    // - per-row UNIQUE violation doesn't abort the ingest run
    // - cross-source non-collision (Addition 3): same uuid, different
    //   source → both land
    // - regression: NULL-uuid historical events query cleanly
    // ----------------------------------------------------------------

    fn event_with_uuid_and_request_id(uuid: &str, request_id: &str) -> Event {
        Event {
            request_id: Some(request_id.to_owned()),
            uuid: Some(uuid.to_owned()),
            ..sample_event()
        }
    }

    #[tokio::test]
    async fn duplicate_uuid_different_request_id_is_skipped_with_warning() -> Result<()> {
        let database = Database::open_in_memory_for_tests().await?;

        // Land the first event.
        let first = event_with_uuid_and_request_id("U-shared", "req-1");
        let summary_1 = insert_events(&database, std::slice::from_ref(&first)).await?;
        assert_eq!(summary_1.inserted, 1);
        assert_eq!(summary_1.skipped_duplicate, 0);
        assert!(summary_1.uuid_duplicate_indices.is_empty());

        // Same uuid, DIFFERENT request_id → the exceptional case.
        // Pre-check catches it, adds to uuid_duplicate_indices,
        // skipped_duplicate stays at 0 (the INSERT never ran).
        let second = event_with_uuid_and_request_id("U-shared", "req-2-different");
        let summary_2 = insert_events(&database, std::slice::from_ref(&second)).await?;
        assert_eq!(summary_2.inserted, 0);
        assert_eq!(summary_2.skipped_duplicate, 0);
        assert_eq!(summary_2.uuid_duplicate_indices, vec![0]);

        assert_eq!(count_events(&database).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_uuid_same_request_id_is_a_silent_rescan_not_a_uuid_collision() -> Result<()>
    {
        // The rescan case: insert the SAME event twice (same uuid AND
        // same request_id). This is "I re-scanned an unchanged file"
        // and must count via `skipped_duplicate` (silent, expected),
        // NOT via `uuid_duplicate_indices` (loud, exceptional).
        // Smoke-found regression — caught when v0.1.16 pre-check first
        // landed and the scan tests started failing because every
        // rescan was being miscounted as a uuid collision.
        let database = Database::open_in_memory_for_tests().await?;
        let event = event_with_uuid_and_request_id("U-rescan", "req-rescan");

        let summary_1 = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(summary_1.inserted, 1);
        assert_eq!(summary_1.skipped_duplicate, 0);

        let summary_2 = insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(summary_2.inserted, 0);
        assert_eq!(summary_2.skipped_duplicate, 1);
        assert!(summary_2.uuid_duplicate_indices.is_empty(),
            "rescan must NOT count as uuid collision");

        Ok(())
    }

    #[tokio::test]
    async fn two_events_with_null_uuid_both_land() -> Result<()> {
        // Partial UNIQUE index excludes NULL → multiple NULL-uuid
        // events coexist (and were the entire historical state
        // pre-v0.1.16). Different request_ids so the request_id
        // UNIQUE doesn't dedup them either.
        let database = Database::open_in_memory_for_tests().await?;
        let a = Event {
            request_id: Some("req-a".to_owned()),
            uuid: None,
            ..sample_event()
        };
        let b = Event {
            request_id: Some("req-b".to_owned()),
            uuid: None,
            ..sample_event()
        };

        let summary = insert_events(&database, &[a, b]).await?;
        assert_eq!(summary.inserted, 2, "NULL uuids must not conflict");
        assert_eq!(summary.skipped_duplicate, 0);
        assert!(summary.uuid_duplicate_indices.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn unique_violation_does_not_abort_ingest_run() -> Result<()> {
        // Batch of three: first lands, second is a uuid-collision
        // (same uuid different request_id), third lands. The middle
        // skip must NOT propagate as Err; the third must still
        // succeed. Per § 4 test #9.
        let database = Database::open_in_memory_for_tests().await?;
        let one = event_with_uuid_and_request_id("U-1", "req-A");
        insert_events(&database, std::slice::from_ref(&one)).await?;

        let batch = vec![
            event_with_uuid_and_request_id("U-2", "req-B"),
            event_with_uuid_and_request_id("U-1", "req-C"), // collision (uuid)
            event_with_uuid_and_request_id("U-3", "req-D"),
        ];
        let summary = insert_events(&database, &batch).await?;
        assert_eq!(summary.inserted, 2, "first and third must land");
        assert_eq!(summary.uuid_duplicate_indices, vec![1]);
        assert_eq!(count_events(&database).await?, 3);
        Ok(())
    }

    #[tokio::test]
    async fn same_uuid_different_source_both_land() -> Result<()> {
        // Addition 3: the (source, uuid) UNIQUE key — NOT (uuid)
        // alone — means two events with the same uuid but different
        // sources both land. Pins the scoping doc § 1 choice
        // against a future refactor that might drop `source` from
        // the index.
        let database = Database::open_in_memory_for_tests().await?;

        let claude = Event {
            source: "claude_code".to_owned(),
            request_id: Some("req-claude".to_owned()),
            uuid: Some("shared-uuid".to_owned()),
            ..sample_event()
        };
        // Use the OTHER seeded source from the initial migration —
        // `admin_api` is in `sources.kind` per the seed-sources test.
        let admin = Event {
            source: "admin_api".to_owned(),
            request_id: Some("req-admin".to_owned()),
            uuid: Some("shared-uuid".to_owned()),
            ..sample_event()
        };

        let summary = insert_events(&database, &[claude, admin]).await?;
        assert_eq!(
            summary.inserted, 2,
            "same uuid, different sources must both land — (source, uuid) UNIQUE key"
        );
        assert!(
            summary.uuid_duplicate_indices.is_empty(),
            "no uuid collision across distinct sources"
        );
        assert_eq!(count_events(&database).await?, 2);
        Ok(())
    }

    #[tokio::test]
    async fn null_uuid_event_round_trips_through_queries() -> Result<()> {
        // § 4 test #10 — regression for pre-v0.1.16 historical
        // events (NULL uuid). Must continue to query/aggregate
        // cleanly. The sample_event() helper has uuid: None by
        // default, so this is the natural historical shape.
        let database = Database::open_in_memory_for_tests().await?;
        let event = sample_event();
        insert_events(&database, std::slice::from_ref(&event)).await?;
        assert_eq!(count_events(&database).await?, 1);

        // Aggregate via daily_usage to confirm NULL-uuid rows feed
        // into query paths without panic / parse error.
        let rows = daily_usage(&database, "2026-04-01", "2026-04-30", ALL_PROVIDERS).await?;
        let total: i64 = rows.iter().map(|r| r.total_tokens).sum();
        assert!(total > 0, "NULL-uuid event must contribute to daily totals");
        Ok(())
    }
}
