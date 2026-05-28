//! `GET /api/v1/sessions/{session_id}/commits` — Phase 2 / v0.1.18.
//!
//! Tier 1 commit attribution surface. Returns every captured commit
//! for the session, each row carrying the verbatim SHA, the cd target,
//! the resolved project, the recovery_source enum, and two diagnostic
//! booleans (`output_head_truncated_by_command`, `is_amend`). The
//! handler adds the query-time `sha_resolves_in_tree` check per D3:
//! batched `git cat-file --batch-check` per (project_resolved) group.
//!
//! /tmp testing commits are filtered by default per D4a; pass
//! `?include_testing=true` to surface them.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use tokenscale_store::{list_session_commits, SessionCommitRow};
use tracing::debug;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SessionCommitsParams {
    /// Opt-in to surface /tmp testing commits. Default false per D4a.
    #[serde(default)]
    pub include_testing: bool,
}

#[derive(Serialize)]
pub struct SessionCommitsResponse {
    pub session_id: String,
    pub commits: Vec<SessionCommitView>,
}

/// API-facing shape. Mirrors `SessionCommitRow` but adds the
/// `sha_resolves_in_tree` field filled in by the handler.
#[derive(Serialize)]
pub struct SessionCommitView {
    pub tool_use_id: String,
    pub session_id: String,
    pub sha: Option<String>,
    pub sha_resolves_in_tree: Option<bool>,
    pub cd_target_raw: Option<String>,
    pub project_resolved: String,
    pub recovery_source: String,
    pub output_head_truncated_by_command: bool,
    pub is_amend: bool,
    pub occurred_at: String,
}

pub async fn handler(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<SessionCommitsParams>,
) -> Result<Json<SessionCommitsResponse>, ApiError> {
    let rows = list_session_commits(&state.database, &session_id, params.include_testing).await?;

    // Group SHAs by project_resolved so one git invocation per project
    // checks all of that project's SHAs. Captures the "rows that have
    // a sha" subset; rows with sha=None bypass the resolution check.
    let resolutions = resolve_shas_batched(&rows);

    let commits = rows
        .into_iter()
        .map(|row| {
            let resolves = row
                .sha
                .as_deref()
                .map(|s| resolutions.get(s).copied().unwrap_or(false));
            SessionCommitView {
                tool_use_id: row.tool_use_id,
                session_id: row.session_id,
                sha: row.sha,
                sha_resolves_in_tree: resolves,
                cd_target_raw: row.cd_target_raw,
                project_resolved: row.project_resolved,
                recovery_source: row.recovery_source,
                output_head_truncated_by_command: row.output_head_truncated_by_command,
                is_amend: row.is_amend,
                occurred_at: row.occurred_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(SessionCommitsResponse {
        session_id,
        commits,
    }))
}

/// Batch-resolve SHAs against their project trees. Returns a map from
/// SHA to "resolves in current tree" boolean. SHAs not present in the
/// map are treated as "does not resolve" by the caller.
///
/// One `git -C <project> cat-file --batch-check` invocation per
/// unique `project_resolved`; SHAs for that project are written to
/// stdin (one per line). The output is one line per input SHA:
/// `<sha> <type> <size>` for found objects, `<sha> missing` for not.
fn resolve_shas_batched(rows: &[SessionCommitRow]) -> HashMap<String, bool> {
    // Group SHAs by project. Only consider rows with a sha.
    let mut by_project: HashMap<&str, Vec<&str>> = HashMap::new();
    for row in rows {
        if let Some(sha) = row.sha.as_deref() {
            by_project
                .entry(row.project_resolved.as_str())
                .or_default()
                .push(sha);
        }
    }

    let mut resolutions: HashMap<String, bool> = HashMap::new();
    for (project, shas) in by_project {
        let resolved = cat_file_batch_check(project, &shas);
        for (sha, ok) in resolved {
            // If the same SHA appears under different projects (rare
            // but possible: a cherry-pick to a different repo), the
            // OR of resolutions is the correct answer (the SHA exists
            // somewhere reachable).
            let entry = resolutions.entry(sha).or_insert(false);
            *entry = *entry || ok;
        }
    }
    resolutions
}

/// Spawn `git -C <project> cat-file --batch-check` with the given
/// SHAs on stdin. Returns (sha, resolves) pairs for each input SHA.
/// On any spawn or read failure, every SHA defaults to "not resolved"
/// (consistent with the existing cwd_resolver fail-soft posture).
fn cat_file_batch_check(project: &str, shas: &[&str]) -> Vec<(String, bool)> {
    if shas.is_empty() {
        return Vec::new();
    }
    debug!(project, sha_count = shas.len(), "cat-file --batch-check spawn");

    let Ok(mut child) = Command::new("git")
        .arg("-C")
        .arg(project)
        .arg("cat-file")
        .arg("--batch-check")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        // git binary missing, or project path doesn't exist, etc.
        // All SHAs default to unresolved.
        return shas.iter().map(|s| ((*s).to_owned(), false)).collect();
    };

    // Write SHAs to stdin, one per line. Drop stdin so the child sees EOF.
    if let Some(mut stdin) = child.stdin.take() {
        for sha in shas {
            let _ = writeln!(stdin, "{sha}");
        }
        drop(stdin);
    }

    let Ok(output) = child.wait_with_output() else {
        return shas.iter().map(|s| ((*s).to_owned(), false)).collect();
    };

    // Parse stdout: one line per input SHA. Format is either
    // `<sha> <type> <size>` (found) or `<sha> missing` (not found).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut by_sha: HashMap<String, bool> = HashMap::new();
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else { continue };
        let second = parts.next().unwrap_or("");
        // "missing" means not found; anything else means found and
        // git is reporting the object type.
        by_sha.insert(sha.to_owned(), second != "missing");
    }

    // Return one entry per input SHA so callers can attribute by
    // input order if needed.
    shas.iter()
        .map(|s| {
            let resolved = by_sha.get(*s).copied().unwrap_or(false);
            ((*s).to_owned(), resolved)
        })
        .collect()
}
