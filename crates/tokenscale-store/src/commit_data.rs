//! Phase 2 / v0.1.18: session_commits insert + query primitives.
//!
//! `process_session_commits` is the bulk-insert path. Called from the
//! scan loop after `insert_tool_data` succeeds. Walks every Bash
//! tool_use for the source, applies `commit_extract::extract_session_commit`
//! to each, resolves the cd target or project_id to a git toplevel,
//! and INSERT OR IGNOREs into `session_commits`. Idempotent per the
//! `(source, tool_use_id)` UNIQUE index.
//!
//! `list_session_commits` is the query primitive consumed by the
//! `GET /api/v1/sessions/:id/commits` handler. /tmp testing commits
//! are filtered by default (D4a); the handler opts back in via the
//! `include_testing` flag.
//!
//! `delete_session_commits_for_source` is the destructive helper
//! called by `--rebuild` per Issue #6's wipe-set extension.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use tokenscale_core::{resolve_to_git_toplevel, RecoverySource, ToolResult, ToolUse};
use tracing::debug;

use crate::commit_extract::{extract_session_commit, is_testing_project};
use crate::error::Result;
use crate::Database;

/// Summary of a `process_session_commits` run. Mirrors the per-table
/// inserted-count shape of `ToolDataInsertSummary`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCommitInsertSummary {
    /// Rows that landed (`INSERT OR IGNORE` non-conflict).
    pub session_commits_inserted: usize,
    /// Rows skipped because the `(source, tool_use_id)` UNIQUE
    /// constraint already had a row (idempotent rescan).
    pub session_commits_duplicates: usize,
    /// Bash tool_uses that were inspected.
    pub bash_tool_uses_scanned: usize,
    /// Bash tool_uses that the shlex token-walk filter rejected as
    /// meta-mentions (probe scripts, sqlite queries, etc.).
    pub bash_tool_uses_filtered_out: usize,
}

const INSERT_SESSION_COMMIT_SQL: &str = "
    INSERT OR IGNORE INTO session_commits (
        source, tool_use_id, session_id, sha, cd_target_raw,
        project_resolved, recovery_source,
        output_head_truncated_by_command, is_amend, occurred_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
";

/// Row shape returned by the bulk-join SELECT inside
/// `process_session_commits`. Aliased so the sqlx tuple's complexity
/// stays out of the function signature (clippy's `type_complexity`
/// flags the inline form).
type BashToolUseWithResultRow = (
    String,            // tool_use_id
    String,            // parent_event_uuid
    String,            // tool_name (always 'Bash' due to WHERE)
    String,            // input_json
    DateTime<Utc>,     // occurred_at
    Option<String>,    // session_id
    Option<String>,    // project_id
    Option<String>,    // result_content
    Option<DateTime<Utc>>, // result_occurred_at
    Option<String>,    // result_parent_event_uuid
);

/// Bulk-insert session_commits rows by walking every Bash tool_use
/// for the source, applying the commit-extraction logic, and
/// resolving cd target / project_id to git toplevel via a per-cwd
/// cache (so each unique cwd shells out only once per run).
#[allow(clippy::too_many_lines)] // Cohesive single transaction; splitting would obscure flow.
pub async fn process_session_commits(
    database: &Database,
    source: &str,
) -> Result<SessionCommitInsertSummary> {
    let mut summary = SessionCommitInsertSummary::default();

    // Pull every Bash tool_use for the source along with its result
    // (if any). The LEFT JOIN puts NULL in the content column for
    // orphans, which `extract_session_commit` handles as the
    // recovery_source=None case.
    let rows: Vec<BashToolUseWithResultRow> = sqlx::query_as(
        "SELECT
             tu.tool_use_id,
             tu.parent_event_uuid,
             tu.tool_name,
             tu.input_json,
             tu.occurred_at,
             tu.session_id,
             tu.project_id,
             tr.content,
             tr.occurred_at,
             tr.parent_event_uuid
           FROM tool_uses tu
      LEFT JOIN tool_results tr
             ON tr.source = tu.source AND tr.tool_use_id = tu.tool_use_id
          WHERE tu.source = ?
            AND tu.tool_name = 'Bash'",
    )
    .bind(source)
    .fetch_all(database.pool())
    .await?;

    // Per-cwd cache so the git rev-parse shellout runs once per
    // unique cwd, not once per row. The maintainer corpus has 27
    // sessions / 68 distinct project_ids, so the savings are real.
    let mut resolve_cache: HashMap<String, String> = HashMap::new();
    let mut resolve = |cwd: &str| -> String {
        if let Some(cached) = resolve_cache.get(cwd) {
            return cached.clone();
        }
        let resolved = resolve_to_git_toplevel(cwd);
        resolve_cache.insert(cwd.to_owned(), resolved.clone());
        resolved
    };

    let mut transaction = database.pool().begin().await?;

    for (
        tool_use_id, parent_event_uuid, tool_name, input_json, occurred_at,
        session_id, project_id, result_content, result_occurred_at, result_parent_event_uuid,
    ) in rows
    {
        summary.bash_tool_uses_scanned += 1;

        let tu = ToolUse {
            tool_use_id,
            parent_event_uuid,
            source: source.to_owned(),
            tool_name,
            input_json,
            occurred_at,
            session_id: session_id.clone(),
            project_id: project_id.clone(),
        };
        let tr = result_content.map(|c| ToolResult {
            tool_use_id: tu.tool_use_id.clone(),
            parent_event_uuid: result_parent_event_uuid.unwrap_or_default(),
            source: source.to_owned(),
            content: c,
            occurred_at: result_occurred_at.unwrap_or(occurred_at),
            session_id: session_id.clone(),
        });

        // Resolve the project. Per D4: prefer cd target when present,
        // fall back to project_id. We do the cd extraction up front
        // here (cheap) rather than after `extract_session_commit`
        // because the extraction itself doesn't know which cwd to use
        // for `project_resolved`.
        let command = serde_json::from_str::<serde_json::Value>(&tu.input_json)
            .ok()
            .and_then(|v| {
                v.get("command")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let cd_target = crate::commit_extract::extract_cd_target(&command);
        let raw_cwd_for_resolution = cd_target
            .as_deref()
            .or(project_id.as_deref())
            .unwrap_or("");
        // If neither cd target nor project_id is present, the
        // resolution falls through to empty string, which then maps to
        // empty in resolve_to_git_toplevel's failure path. The schema
        // requires project_resolved NOT NULL; use a placeholder.
        let project_resolved = if raw_cwd_for_resolution.is_empty() {
            "<unresolved>".to_owned()
        } else {
            resolve(raw_cwd_for_resolution)
        };

        let Some(sc) = extract_session_commit(&tu, tr.as_ref(), project_resolved) else {
            summary.bash_tool_uses_filtered_out += 1;
            continue;
        };

        let result = sqlx::query(INSERT_SESSION_COMMIT_SQL)
            .bind(&sc.source)
            .bind(&sc.tool_use_id)
            .bind(&sc.session_id)
            .bind(sc.sha.as_deref())
            .bind(sc.cd_target_raw.as_deref())
            .bind(&sc.project_resolved)
            .bind(sc.recovery_source.as_str())
            .bind(i64::from(sc.output_head_truncated_by_command))
            .bind(i64::from(sc.is_amend))
            .bind(sc.occurred_at)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() > 0 {
            summary.session_commits_inserted += 1;
        } else {
            summary.session_commits_duplicates += 1;
        }
    }

    transaction.commit().await?;
    debug!(?summary, "process_session_commits committed");
    Ok(summary)
}

/// One captured commit returned by `list_session_commits`. Mirrors
/// `SessionCommit` but adds the API-facing JSON shape (snake_case
/// fields; the SHA resolution boolean is filled in by the handler at
/// query time, not by this primitive).
#[derive(Debug, Clone, Serialize)]
pub struct SessionCommitRow {
    pub tool_use_id: String,
    pub session_id: String,
    pub sha: Option<String>,
    pub cd_target_raw: Option<String>,
    pub project_resolved: String,
    pub recovery_source: String,
    pub output_head_truncated_by_command: bool,
    pub is_amend: bool,
    pub occurred_at: DateTime<Utc>,
}

/// Row shape returned by the `list_session_commits` SELECT. Same
/// type-alias treatment as `BashToolUseWithResultRow`.
type SessionCommitDbRow = (
    String,                // tool_use_id
    String,                // session_id
    Option<String>,        // sha
    Option<String>,        // cd_target_raw
    String,                // project_resolved
    String,                // recovery_source
    i64,                   // output_head_truncated_by_command
    i64,                   // is_amend
    DateTime<Utc>,         // occurred_at
);

/// Phase 2 query primitive: list every session_commit for a session,
/// ordered by occurred_at ascending. /tmp testing commits are filtered
/// by default per D4a; pass `include_testing = true` to surface them.
pub async fn list_session_commits(
    database: &Database,
    session_id: &str,
    include_testing: bool,
) -> Result<Vec<SessionCommitRow>> {
    let raw: Vec<SessionCommitDbRow> = sqlx::query_as(
        "SELECT
             tool_use_id,
             session_id,
             sha,
             cd_target_raw,
             project_resolved,
             recovery_source,
             output_head_truncated_by_command,
             is_amend,
             occurred_at
           FROM session_commits
          WHERE session_id = ?
          ORDER BY occurred_at ASC",
    )
    .bind(session_id)
    .fetch_all(database.pool())
    .await?;

    let rows: Vec<SessionCommitRow> = raw
        .into_iter()
        .filter(|(_, _, _, _, project_resolved, _, _, _, _)| {
            include_testing || !is_testing_project(project_resolved)
        })
        .map(|(tool_use_id, session_id, sha, cd_target_raw, project_resolved,
               recovery_source, output_head_truncated, is_amend, occurred_at)| {
            SessionCommitRow {
                tool_use_id,
                session_id,
                sha,
                cd_target_raw,
                project_resolved,
                recovery_source,
                output_head_truncated_by_command: output_head_truncated != 0,
                is_amend: is_amend != 0,
                occurred_at,
            }
        })
        .collect();

    Ok(rows)
}

/// Delete every session_commit for a source. Used by `--rebuild` per
/// Issue #6's wipe-set extension. The companion deletes for
/// `tool_uses`, `tool_results`, and `file_snapshots` live in
/// `tool_data.rs`. The CLI groups all four deletes under a single
/// destructive WARN log.
pub async fn delete_session_commits_for_source(database: &Database, source: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM session_commits WHERE source = ?")
        .bind(source)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Helper used by the impact-aggregation regression test (§9 smoke
/// gate item). Confirms session_commits never leak into cost or
/// environmental aggregation by counting only what's in the table.
pub async fn count_session_commits(database: &Database, source: &str) -> Result<usize> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_commits WHERE source = ?")
            .bind(source)
            .fetch_one(database.pool())
            .await?;
    Ok(usize::try_from(row.0).unwrap_or(0))
}

/// Resolve the recovery_source TEXT back to the enum. Convenience
/// for handlers / consumers that want the typed value.
#[must_use]
pub fn parse_recovery_source(value: &str) -> RecoverySource {
    RecoverySource::from_db_str(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_data::insert_tool_data;
    use chrono::TimeZone;
    use tokenscale_core::{ToolResult, ToolUse};

    fn sample_tool_use(command: &str, tool_use_id: &str, session_id: &str) -> ToolUse {
        ToolUse {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "parent-uuid".to_owned(),
            source: "claude_code".to_owned(),
            tool_name: "Bash".to_owned(),
            input_json: serde_json::json!({ "command": command }).to_string(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 0).unwrap(),
            session_id: Some(session_id.to_owned()),
            // Use a non-tmp project_id so the /tmp filter doesn't drop
            // these in default tests.
            project_id: Some("/home/dev/sample-repo".to_owned()),
        }
    }

    fn sample_tool_result(tool_use_id: &str, content: &str, session_id: &str) -> ToolResult {
        ToolResult {
            tool_use_id: tool_use_id.to_owned(),
            parent_event_uuid: "user-uuid".to_owned(),
            source: "claude_code".to_owned(),
            content: content.to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 5, 26, 0, 0, 1).unwrap(),
            session_id: Some(session_id.to_owned()),
        }
    }

    #[tokio::test]
    async fn process_session_commits_inserts_extracted_rows() -> Result<()> {
        let db = Database::open_in_memory_for_tests().await?;

        // Three Bash tool_uses: one real commit, one non-commit Bash, one meta-mention.
        let real = sample_tool_use(
            r#"git commit -m "msg""#,
            "toolu_real",
            "sess-A",
        );
        let non_commit = sample_tool_use("git status", "toolu_noncommit", "sess-A");
        let meta = sample_tool_use("grep 'git commit' file.txt", "toolu_meta", "sess-A");

        let real_result = sample_tool_result("toolu_real", "[main abc1234] subject", "sess-A");
        let non_commit_result = sample_tool_result("toolu_noncommit", "on branch main", "sess-A");
        let meta_result = sample_tool_result("toolu_meta", "no matches", "sess-A");

        insert_tool_data(
            &db,
            &[real, non_commit, meta],
            &[real_result, non_commit_result, meta_result],
            &[],
        )
        .await?;

        let summary = process_session_commits(&db, "claude_code").await?;
        assert_eq!(summary.bash_tool_uses_scanned, 3);
        assert_eq!(summary.bash_tool_uses_filtered_out, 2,
            "non-commit + meta-mention must both fail the shlex filter");
        assert_eq!(summary.session_commits_inserted, 1);

        Ok(())
    }

    #[tokio::test]
    async fn process_session_commits_is_idempotent_on_rescan() -> Result<()> {
        let db = Database::open_in_memory_for_tests().await?;
        let tu = sample_tool_use(r#"git commit -m "msg""#, "toolu_a", "sess-A");
        let tr = sample_tool_result("toolu_a", "[main abc1234] subj", "sess-A");
        insert_tool_data(&db, &[tu], &[tr], &[]).await?;

        let first = process_session_commits(&db, "claude_code").await?;
        assert_eq!(first.session_commits_inserted, 1);
        assert_eq!(first.session_commits_duplicates, 0);

        let second = process_session_commits(&db, "claude_code").await?;
        assert_eq!(second.session_commits_inserted, 0,
            "rescan must not re-insert");
        assert_eq!(second.session_commits_duplicates, 1);

        assert_eq!(count_session_commits(&db, "claude_code").await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn list_session_commits_filters_tmp_by_default() -> Result<()> {
        let db = Database::open_in_memory_for_tests().await?;

        // A real commit in a real project + a /tmp testing exercise.
        let real = ToolUse {
            project_id: Some("/home/dev/real-repo".to_owned()),
            ..sample_tool_use(r#"git commit -m "msg""#, "toolu_real", "sess-B")
        };
        let tmp = ToolUse {
            project_id: Some("/tmp/wt-test".to_owned()),
            ..sample_tool_use(
                r#"cd /tmp/wt-test && git commit -m "wt""#,
                "toolu_tmp",
                "sess-B",
            )
        };
        let real_r = sample_tool_result("toolu_real", "[main abc1234] s", "sess-B");
        let tmp_r = sample_tool_result("toolu_tmp", "[main def5678] t", "sess-B");
        insert_tool_data(&db, &[real, tmp], &[real_r, tmp_r], &[]).await?;
        process_session_commits(&db, "claude_code").await?;

        // include_testing=false: only the real commit.
        let default_view = list_session_commits(&db, "sess-B", false).await?;
        assert_eq!(default_view.len(), 1,
            "/tmp commits filtered by default per D4a");
        assert_eq!(default_view[0].tool_use_id, "toolu_real");

        // include_testing=true: both surface.
        let full_view = list_session_commits(&db, "sess-B", true).await?;
        assert_eq!(full_view.len(), 2);

        Ok(())
    }

    #[tokio::test]
    async fn delete_session_commits_for_source_only_touches_target_source() -> Result<()> {
        let db = Database::open_in_memory_for_tests().await?;
        let tu = sample_tool_use(r#"git commit -m "msg""#, "toolu_x", "sess-C");
        let tr = sample_tool_result("toolu_x", "[main abc1234] subj", "sess-C");
        insert_tool_data(&db, &[tu], &[tr], &[]).await?;
        process_session_commits(&db, "claude_code").await?;
        assert_eq!(count_session_commits(&db, "claude_code").await?, 1);

        let deleted = delete_session_commits_for_source(&db, "claude_code").await?;
        assert_eq!(deleted, 1);
        assert_eq!(count_session_commits(&db, "claude_code").await?, 0);

        // Different source: no-op.
        let deleted = delete_session_commits_for_source(&db, "admin_api").await?;
        assert_eq!(deleted, 0);
        Ok(())
    }

    #[tokio::test]
    async fn process_session_commits_captures_orphan_with_none_recovery() -> Result<()> {
        let db = Database::open_in_memory_for_tests().await?;
        // Bash tool_use with no matching tool_result — orphan.
        let tu = sample_tool_use(r#"git commit -m "msg""#, "toolu_orph", "sess-O");
        insert_tool_data(&db, &[tu], &[], &[]).await?;
        process_session_commits(&db, "claude_code").await?;

        let rows = list_session_commits(&db, "sess-O", false).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sha, None);
        assert_eq!(rows[0].recovery_source, "none");
        Ok(())
    }

    // ----------------------------------------------------------------
    // §9 release gate: aggregation regression. session_commits must
    // NOT leak into events cost/impact aggregation. Same window,
    // same SQL, same numbers, before and after session_commits exists.
    // Mirrors the v0.1.17 §7 test pattern for tool_data; defends the
    // separation between tool-attribution tables and the events
    // aggregation path.
    // ----------------------------------------------------------------
    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Cohesive: factor + pricing TOML + setup + assert in one place.
    async fn aggregate_impact_by_bucket_numbers_unchanged_by_session_commits_inserts() {
        use crate::aggregate_impact_by_bucket;
        use crate::impact_query::ImpactQueryFactors;
        use crate::queries::{Granularity, ALL_PROVIDERS};
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
[[grid_factors."us-east-1"]]
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
        sync_environmental_factors(
            &db,
            &EnvironmentalFactorsFile::parse(PROD_TOML).unwrap(),
        )
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
            &db,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors,
        )
        .await
        .unwrap();

        // Insert tool_uses for the same session, then run
        // process_session_commits to populate session_commits.
        let tu = sample_tool_use(
            r#"git commit -m "msg""#,
            "toolu_agg_test",
            "sess-test",
        );
        let tr = sample_tool_result("toolu_agg_test", "[main abc1234] subj", "sess-test");
        insert_tool_data(&db, &[tu], &[tr], &[]).await.unwrap();
        process_session_commits(&db, "claude_code").await.unwrap();
        assert_eq!(
            count_session_commits(&db, "claude_code").await.unwrap(),
            1,
            "precondition: session_commits row should exist"
        );

        let after = aggregate_impact_by_bucket(
            &db,
            "2026-04-01",
            "2026-04-30",
            ALL_PROVIDERS,
            &[],
            Granularity::Day,
            &factors,
        )
        .await
        .unwrap();

        // Same row count, same numbers. If session_commits leak into
        // the events aggregation path, one of these asserts fires.
        assert_eq!(before.len(), after.len(), "row count must not change");
        assert_eq!(before.len(), 1);

        let b = &before[0];
        let a = &after[0];
        assert_eq!(b.input_tokens, a.input_tokens);
        assert_eq!(b.output_tokens, a.output_tokens);
        assert_eq!(b.cache_read_tokens, a.cache_read_tokens);
        assert_eq!(b.cache_write_5m_tokens, a.cache_write_5m_tokens);
        assert_eq!(b.cache_write_1h_tokens, a.cache_write_1h_tokens);
        assert!((b.energy_wh.unwrap() - a.energy_wh.unwrap()).abs() < 1e-9);
        assert!((b.facility_wh.unwrap() - a.facility_wh.unwrap()).abs() < 1e-9);
        assert_eq!(b.cost_usd_total, a.cost_usd_total);
        assert_eq!(b.events_count, a.events_count);
        assert_eq!(b.events_missing_pricing, a.events_missing_pricing);
    }
}
