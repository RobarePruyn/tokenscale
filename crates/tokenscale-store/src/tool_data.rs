//! Phase 1.5 / v0.1.17: tool-use, tool-result, and file-snapshot
//! inserts + query primitives.
//!
//! `insert_tool_data` is the bulk INSERT path. INSERT OR IGNORE on
//! the (source, *_id) UNIQUE indexes makes re-scans idempotent —
//! same posture as `events::insert_events`.
//!
//! `count_tool_use_orphans` reports the orphan-count baseline
//! (Addition 1) — `tool_use` rows whose `tool_use_id` has no matching
//! `tool_result` in the same source. Phase 0 baseline: 5 on the
//! maintainer's data (interrupted sessions, expected). Surfaces
//! upstream schema drift cheaply if the number ever climbs.
//!
//! Phase 2 (`list_session_bash_calls`) and Phase 3
//! (`list_session_file_edits`) query primitives ship as **stubs**.
//! Their function signatures are committed against so Phase 2/3 can
//! build, but v0.1.17 does not surface them in any handler — see
//! the scoping doc §4.

use serde::Serialize;
use tokenscale_core::{FileSnapshot, ToolResult, ToolUse};
use tracing::debug;

use crate::error::Result;
use crate::Database;

/// Result of an `insert_tool_data` call. Tracks per-table inserts so
/// the scan-summary line can break the count down (a Phase 1.5
/// rescan lands `tool_uses_inserted = 0` against an unchanged file
/// — INSERT OR IGNORE silently dedups).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolDataInsertSummary {
    pub tool_uses_inserted: usize,
    pub tool_results_inserted: usize,
    pub file_snapshots_inserted: usize,
}

const INSERT_TOOL_USE_SQL: &str = "
    INSERT OR IGNORE INTO tool_uses (
        tool_use_id, parent_event_uuid, source, tool_name,
        input_json, occurred_at, session_id, project_id
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
";

const INSERT_TOOL_RESULT_SQL: &str = "
    INSERT OR IGNORE INTO tool_results (
        tool_use_id, parent_event_uuid, source, content,
        occurred_at, session_id
    ) VALUES (?, ?, ?, ?, ?, ?)
";

const INSERT_FILE_SNAPSHOT_SQL: &str = "
    INSERT OR IGNORE INTO file_snapshots (
        snapshot_message_id, source, file_path, backup_file_name,
        version, backup_time, is_snapshot_update, session_id, project_id
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
";

/// Bulk-insert tool-use / tool-result / file-snapshot rows. Single
/// transaction; INSERT OR IGNORE dedups rescans via the (source, *_id)
/// UNIQUE indexes. Same idempotency posture as `insert_events`.
pub async fn insert_tool_data(
    database: &Database,
    tool_uses: &[ToolUse],
    tool_results: &[ToolResult],
    file_snapshots: &[FileSnapshot],
) -> Result<ToolDataInsertSummary> {
    if tool_uses.is_empty() && tool_results.is_empty() && file_snapshots.is_empty() {
        return Ok(ToolDataInsertSummary::default());
    }

    let mut transaction = database.pool().begin().await?;
    let mut summary = ToolDataInsertSummary::default();

    for tu in tool_uses {
        let result = sqlx::query(INSERT_TOOL_USE_SQL)
            .bind(&tu.tool_use_id)
            .bind(&tu.parent_event_uuid)
            .bind(&tu.source)
            .bind(&tu.tool_name)
            .bind(&tu.input_json)
            .bind(tu.occurred_at)
            .bind(tu.session_id.as_deref())
            .bind(tu.project_id.as_deref())
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() > 0 {
            summary.tool_uses_inserted += 1;
        }
    }

    for tr in tool_results {
        let result = sqlx::query(INSERT_TOOL_RESULT_SQL)
            .bind(&tr.tool_use_id)
            .bind(&tr.parent_event_uuid)
            .bind(&tr.source)
            .bind(&tr.content)
            .bind(tr.occurred_at)
            .bind(tr.session_id.as_deref())
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() > 0 {
            summary.tool_results_inserted += 1;
        }
    }

    for fs in file_snapshots {
        let result = sqlx::query(INSERT_FILE_SNAPSHOT_SQL)
            .bind(&fs.snapshot_message_id)
            .bind(&fs.source)
            .bind(&fs.file_path)
            .bind(fs.backup_file_name.as_deref())
            .bind(fs.version)
            .bind(fs.backup_time)
            .bind(i64::from(fs.is_snapshot_update))
            .bind(fs.session_id.as_deref())
            .bind(fs.project_id.as_deref())
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() > 0 {
            summary.file_snapshots_inserted += 1;
        }
    }

    transaction.commit().await?;
    debug!(?summary, "insert_tool_data committed");
    Ok(summary)
}

/// v0.1.18 / Issue #6 fix: delete every tool_use for a source. Used
/// by `--rebuild` to wipe the slate before a full re-parse, paired
/// with the matching deletes for tool_results and file_snapshots
/// (also in this module) plus session_commits (in commit_data.rs).
/// v0.1.17 shipped `--rebuild` without these; the resulting bug
/// retained stale rows across rebuilds. See Issue #6.
pub async fn delete_tool_uses_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM tool_uses WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// v0.1.18 / Issue #6 fix: companion to `delete_tool_uses_for_source`.
pub async fn delete_tool_results_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM tool_results WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// v0.1.18 / Issue #6 fix: companion to `delete_tool_uses_for_source`.
pub async fn delete_file_snapshots_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM file_snapshots WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Phase 1.5 Addition 1: orphan-count baseline. Counts `tool_use`
/// rows (in the given source) whose `tool_use_id` has no matching
/// `tool_result`. Phase 0 baseline on maintainer data: 5. Surfaces
/// schema drift cheaply if the number ever climbs.
pub async fn count_tool_use_orphans(database: &Database, source: &str) -> Result<usize> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tool_uses tu
         WHERE tu.source = ?
           AND NOT EXISTS (
             SELECT 1 FROM tool_results tr
              WHERE tr.source = tu.source
                AND tr.tool_use_id = tu.tool_use_id
           )",
    )
    .bind(source)
    .fetch_one(database.pool())
    .await?;
    Ok(usize::try_from(row.0).unwrap_or(0))
}

// ---------------------------------------------------------------
// Phase 2 / Phase 3 query primitive stubs
// ---------------------------------------------------------------
//
// Function signatures committed against so Phase 2 + Phase 3 can
// build. v0.1.17 does not surface these in any handler. Bodies are
// real (Phase 2/3 would need them) but cheap to ship now while the
// schema is fresh.

/// One Bash tool-use call in a session, joined with its result.
/// Phase 2 (Tier 1 commit attribution) filters on this for
/// `git commit` invocations and captures the resulting SHA from
/// `result_content`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionBashCallRow {
    pub tool_use_id: String,
    /// The Bash `command` field, extracted from `input_json`.
    /// Stored on `tool_uses.input_json` as a verbatim JSON object;
    /// Phase 2 query primitive extracts the `command` string at
    /// query time.
    pub command: Option<String>,
    /// Raw stdout from the tool_result, when one is present.
    /// `None` for orphan tool_uses (interrupted sessions).
    pub result_content: Option<String>,
    pub occurred_at: String,
    pub parent_event_uuid: String,
}

/// Phase 2 query primitive: list every Bash call in a session, with
/// its result (or NULL for orphans). Caller filters / parses for
/// `git commit` etc.
pub async fn list_session_bash_calls(
    database: &Database,
    session_id: &str,
) -> Result<Vec<SessionBashCallRow>> {
    let raw: Vec<(String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT
             tu.tool_use_id,
             tu.input_json,
             tr.content       AS result_content,
             tu.occurred_at,
             tu.parent_event_uuid
           FROM tool_uses tu
      LEFT JOIN tool_results tr
             ON tr.source = tu.source
            AND tr.tool_use_id = tu.tool_use_id
          WHERE tu.session_id = ?
            AND tu.tool_name = 'Bash'
          ORDER BY tu.occurred_at ASC",
    )
    .bind(session_id)
    .fetch_all(database.pool())
    .await?;

    Ok(raw
        .into_iter()
        .map(|(tool_use_id, input_json, result_content, occurred_at, parent_event_uuid)| {
            // Extract the `command` field from input_json on the fly.
            // Bash inputs always carry it; Edit / Write etc. would
            // have other fields. Defensive `None` if parse fails.
            let command = serde_json::from_str::<serde_json::Value>(&input_json)
                .ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str().map(str::to_owned)));
            SessionBashCallRow {
                tool_use_id,
                command,
                result_content,
                occurred_at,
                parent_event_uuid,
            }
        })
        .collect())
}

/// One Edit/Write tool-use call in a session.
/// Phase 3 (Tier 2 edit-survival) filters on this for the files CC
/// touched, then `git blame`s the current tree to measure how much
/// survived.
#[derive(Debug, Clone, Serialize)]
pub struct SessionFileEditRow {
    pub tool_use_id: String,
    /// Either "Edit" or "Write".
    pub tool_name: String,
    /// The `file_path` field, extracted from `input_json`.
    pub file_path: Option<String>,
    pub occurred_at: String,
}

/// Phase 3 query primitive: list every Edit/Write call in a session
/// with the file path the tool touched.
pub async fn list_session_file_edits(
    database: &Database,
    session_id: &str,
) -> Result<Vec<SessionFileEditRow>> {
    let raw: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT
             tu.tool_use_id,
             tu.tool_name,
             tu.input_json,
             tu.occurred_at
           FROM tool_uses tu
          WHERE tu.session_id = ?
            AND tu.tool_name IN ('Edit', 'Write')
          ORDER BY tu.occurred_at ASC",
    )
    .bind(session_id)
    .fetch_all(database.pool())
    .await?;

    Ok(raw
        .into_iter()
        .map(|(tool_use_id, tool_name, input_json, occurred_at)| {
            let file_path = serde_json::from_str::<serde_json::Value>(&input_json)
                .ok()
                .and_then(|v| v.get("file_path").and_then(|p| p.as_str().map(str::to_owned)));
            SessionFileEditRow {
                tool_use_id,
                tool_name,
                file_path,
                occurred_at,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn fixture_tool_use(tool_use_id: &str, tool_name: &str, session_id: &str) -> ToolUse {
        ToolUse {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "parent-uuid".to_owned(),
            source: "claude_code".to_owned(),
            tool_name: tool_name.to_owned(),
            input_json: r#"{"command":"ls","description":"list"}"#.to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap(),
            session_id: Some(session_id.to_owned()),
            project_id: Some("/repo".to_owned()),
        }
    }

    fn fixture_tool_result(tool_use_id: &str, session_id: &str, content: &str) -> ToolResult {
        ToolResult {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "parent-uuid".to_owned(),
            source: "claude_code".to_owned(),
            content: content.to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 1).unwrap(),
            session_id: Some(session_id.to_owned()),
        }
    }

    fn fixture_file_snapshot(snapshot_message_id: &str, file_path: &str) -> FileSnapshot {
        FileSnapshot {
            snapshot_message_id: snapshot_message_id.to_owned(),
            source: "claude_code".to_owned(),
            file_path: file_path.to_owned(),
            backup_file_name: None,
            version: 1,
            backup_time: Utc.with_ymd_and_hms(2026, 5, 24, 12, 0, 0).unwrap(),
            is_snapshot_update: false,
            session_id: Some("sess-1".to_owned()),
            project_id: Some("/repo".to_owned()),
        }
    }

    #[tokio::test]
    async fn insert_tool_data_lands_three_record_kinds() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        let summary = insert_tool_data(
            &db,
            &[
                fixture_tool_use("toolu_A", "Bash", "sess-1"),
                fixture_tool_use("toolu_B", "Edit", "sess-1"),
            ],
            &[fixture_tool_result("toolu_A", "sess-1", "OK")],
            &[fixture_file_snapshot("uuid-A1", "foo.rs")],
        )
        .await
        .unwrap();
        assert_eq!(summary.tool_uses_inserted, 2);
        assert_eq!(summary.tool_results_inserted, 1);
        assert_eq!(summary.file_snapshots_inserted, 1);
    }

    #[tokio::test]
    async fn insert_tool_data_is_idempotent_on_rescan() {
        // Same rows inserted twice. First lands; second is a no-op
        // via INSERT OR IGNORE on the (source, *_id) UNIQUE indexes.
        let db = Database::open_in_memory_for_tests().await.unwrap();
        let tu = vec![fixture_tool_use("toolu_A", "Bash", "sess-1")];
        let tr = vec![fixture_tool_result("toolu_A", "sess-1", "OK")];
        let fs = vec![fixture_file_snapshot("uuid-A1", "foo.rs")];

        let first = insert_tool_data(&db, &tu, &tr, &fs).await.unwrap();
        assert_eq!(first.tool_uses_inserted, 1);
        assert_eq!(first.tool_results_inserted, 1);
        assert_eq!(first.file_snapshots_inserted, 1);

        let second = insert_tool_data(&db, &tu, &tr, &fs).await.unwrap();
        assert_eq!(second.tool_uses_inserted, 0);
        assert_eq!(second.tool_results_inserted, 0);
        assert_eq!(second.file_snapshots_inserted, 0);
    }

    #[tokio::test]
    async fn count_tool_use_orphans_returns_unmatched_count() {
        // Two tool_uses, one tool_result → one orphan.
        let db = Database::open_in_memory_for_tests().await.unwrap();
        insert_tool_data(
            &db,
            &[
                fixture_tool_use("toolu_A", "Bash", "sess-1"),
                fixture_tool_use("toolu_ORPHAN", "Bash", "sess-1"),
            ],
            &[fixture_tool_result("toolu_A", "sess-1", "OK")],
            &[],
        )
        .await
        .unwrap();
        let orphans = count_tool_use_orphans(&db, "claude_code").await.unwrap();
        assert_eq!(orphans, 1);
    }

    #[tokio::test]
    async fn list_session_bash_calls_joins_through_tool_use_id() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        insert_tool_data(
            &db,
            &[
                fixture_tool_use("toolu_A", "Bash", "sess-1"),
                fixture_tool_use("toolu_ORPHAN", "Bash", "sess-1"),
                fixture_tool_use("toolu_E", "Edit", "sess-1"), // not Bash
            ],
            &[fixture_tool_result("toolu_A", "sess-1", "result-of-A")],
            &[],
        )
        .await
        .unwrap();
        let bash_calls = list_session_bash_calls(&db, "sess-1").await.unwrap();
        assert_eq!(bash_calls.len(), 2, "Edit row must NOT appear; both Bash rows do");
        // The matched one has result_content; the orphan has None.
        let matched = bash_calls.iter().find(|c| c.tool_use_id == "toolu_A").unwrap();
        assert_eq!(matched.result_content.as_deref(), Some("result-of-A"));
        let orphan = bash_calls.iter().find(|c| c.tool_use_id == "toolu_ORPHAN").unwrap();
        assert!(orphan.result_content.is_none(), "orphan tool_use has no result");
        // command extracted from input_json
        assert_eq!(matched.command.as_deref(), Some("ls"));
    }

    // ----------------------------------------------------------------
    // § 7 release gate: aggregation regression. The new tool_data
    // tables must NOT leak into events aggregation. Same window,
    // same SQL, same numbers — before and after tool_data exists.
    // ----------------------------------------------------------------

    #[tokio::test]
    async fn aggregate_impact_by_bucket_numbers_unchanged_by_tool_data_inserts() {
        use crate::aggregate_impact_by_bucket;
        use crate::queries::{Granularity, ALL_PROVIDERS};
        use crate::impact_query::ImpactQueryFactors;
        use crate::{insert_events, sync_environmental_factors, sync_pricing};
        use tokenscale_core::{EnvironmentalFactorsFile, Event, PricingFile};

        const PROD_TOML: &str = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2026-01-01"
source_doc = "test"
wh_per_mtok_input = 0.5
wh_per_mtok_output = 2.0
wh_per_mtok_cache_read = 0.05
wh_per_mtok_cache_write_5m = 0.5
wh_per_mtok_cache_write_1h = 0.5
uncertainty_range_pct = 35
confidence = "secondary"
[grid_factors."us-east-1"]
display_name = "AWS US East"
valid_from = "2026-01-01"
source_accessed_at = "2026-01-01"
co2e_kg_per_kwh = 0.30
water_l_per_kwh = 0.20
pue = 1.15
egrid_subregion = "SRVC"
egrid_subregion_full_name = "SERC Virginia/Carolina"
"#;

        const PRICING_TOML: &str = r#"
schema_version = 1
file_status = "production"
[providers.anthropic]
display_name = "Anthropic"
[providers.anthropic.models."claude-sonnet-4-6"]
display_name = "Claude Sonnet 4.6"
valid_from = "2025-09-01"
input_usd_per_mtok = 3.00
output_usd_per_mtok = 15.00
cache_read_usd_per_mtok = 0.30
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "x"
source_accessed_at = "2026-05-24"
"#;

        let db = Database::open_in_memory_for_tests().await.unwrap();
        sync_environmental_factors(&db, &EnvironmentalFactorsFile::parse(PROD_TOML).unwrap())
            .await
            .unwrap();
        sync_pricing(&db, &PricingFile::parse(PRICING_TOML).unwrap())
            .await
            .unwrap();

        let event = Event {
            source: "claude_code".to_owned(),
            occurred_at: chrono::Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: "claude-sonnet-4-6".to_owned(),
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some("req-test".to_owned()),
            content_hash: None,
            session_id: Some("sess-test".to_owned()),
            project_id: Some("/repo".to_owned()),
            workspace_id: None,
            api_key_id: None,
            uuid: Some("uuid-test".to_owned()),
            parent_uuid: None,
            git_branch: None,
            raw: None,
        };
        insert_events(&db, std::slice::from_ref(&event)).await.unwrap();

        let factors = ImpactQueryFactors {
            region: "us-east-1",
            fallback_pue: 1.15,
            fallback_wue_l_per_kwh: Some(0.15),
        };
        let before = aggregate_impact_by_bucket(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], Granularity::Day, &factors,
        )
        .await
        .unwrap();

        // Now insert a bunch of tool_data referencing this same event.
        insert_tool_data(
            &db,
            &[
                fixture_tool_use("toolu_A", "Bash", "sess-test"),
                fixture_tool_use("toolu_B", "Edit", "sess-test"),
                fixture_tool_use("toolu_C", "Write", "sess-test"),
            ],
            &[
                fixture_tool_result("toolu_A", "sess-test", "result-A"),
                fixture_tool_result("toolu_B", "sess-test", "result-B"),
            ],
            &[fixture_file_snapshot("uuid-test", "/repo/foo.rs")],
        )
        .await
        .unwrap();

        let after = aggregate_impact_by_bucket(
            &db, "2026-04-01", "2026-04-30",
            ALL_PROVIDERS, &[], Granularity::Day, &factors,
        )
        .await
        .unwrap();

        // Same row count.
        assert_eq!(before.len(), after.len(), "row count must not change");
        assert_eq!(before.len(), 1);

        let b = &before[0];
        let a = &after[0];

        // Every field that participates in cost / impact aggregation
        // is identical. If the new tables ever leak into the
        // aggregation path, one of these asserts fires.
        assert_eq!(b.input_tokens, a.input_tokens);
        assert_eq!(b.output_tokens, a.output_tokens);
        assert_eq!(b.cache_read_tokens, a.cache_read_tokens);
        assert_eq!(b.cache_write_5m_tokens, a.cache_write_5m_tokens);
        assert_eq!(b.cache_write_1h_tokens, a.cache_write_1h_tokens);
        assert!((b.energy_wh - a.energy_wh).abs() < 1e-9);
        assert!((b.facility_wh - a.facility_wh).abs() < 1e-9);
        assert_eq!(b.cost_usd_total, a.cost_usd_total);
        assert_eq!(b.events_count, a.events_count);
        assert_eq!(b.events_missing_pricing, a.events_missing_pricing);
    }

    #[tokio::test]
    async fn list_session_file_edits_filters_to_edit_and_write() {
        let db = Database::open_in_memory_for_tests().await.unwrap();
        // Use input_json that carries file_path.
        let mut edit = fixture_tool_use("toolu_E", "Edit", "sess-1");
        edit.input_json = r#"{"file_path":"/repo/foo.rs","old_string":"x","new_string":"y"}"#.to_owned();
        let mut write = fixture_tool_use("toolu_W", "Write", "sess-1");
        write.input_json = r#"{"file_path":"/repo/new.rs","content":"…"}"#.to_owned();
        insert_tool_data(
            &db,
            &[
                fixture_tool_use("toolu_B", "Bash", "sess-1"), // filtered out
                edit,
                write,
            ],
            &[],
            &[],
        )
        .await
        .unwrap();
        let edits = list_session_file_edits(&db, "sess-1").await.unwrap();
        assert_eq!(edits.len(), 2);
        let file_paths: Vec<&str> = edits
            .iter()
            .filter_map(|e| e.file_path.as_deref())
            .collect();
        assert!(file_paths.contains(&"/repo/foo.rs"));
        assert!(file_paths.contains(&"/repo/new.rs"));
    }
}
