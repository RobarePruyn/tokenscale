//! `GET /api/v1/projects` — distinct `project_id` values present in the data
//! window, with a per-project rollup. Used to populate the project-filter
//! chip list in the dashboard.
//!
//! Same window/provider params as `/api/v1/usage/daily` so the chip list
//! reflects exactly what's in the chart.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokenscale_store::{list_projects_with_totals, ALL_PROVIDERS};

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_WINDOW_DAYS: i64 = 30;

#[derive(Debug, Deserialize)]
pub struct ProjectsParams {
    /// `YYYY-MM-DD`. Defaults to 30 days ago (UTC).
    pub from: Option<String>,
    /// `YYYY-MM-DD`. Defaults to today (UTC).
    pub to: Option<String>,
    /// `all` (default) or a specific provider slug.
    pub provider: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Serialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub event_count: i64,
    pub total_tokens: i64,
}

pub async fn list_handler(
    State(state): State<AppState>,
    Query(params): Query<ProjectsParams>,
) -> Result<Json<ProjectsResponse>, ApiError> {
    use chrono::{Duration, Utc};

    let to_date = params
        .to
        .unwrap_or_else(|| Utc::now().date_naive().to_string());
    let from_date = params.from.unwrap_or_else(|| {
        (Utc::now().date_naive() - Duration::days(DEFAULT_WINDOW_DAYS)).to_string()
    });
    chrono::NaiveDate::parse_from_str(&from_date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest(format!("expected YYYY-MM-DD, got {from_date:?}")))?;
    chrono::NaiveDate::parse_from_str(&to_date, "%Y-%m-%d")
        .map_err(|_| ApiError::BadRequest(format!("expected YYYY-MM-DD, got {to_date:?}")))?;
    let provider = params.provider.unwrap_or_else(|| ALL_PROVIDERS.to_owned());

    let rows = list_projects_with_totals(&state.database, &from_date, &to_date, &provider).await?;
    let projects = rows
        .into_iter()
        .map(|row| ProjectSummary {
            project_id: row.project_id,
            event_count: row.event_count,
            total_tokens: row.total_tokens,
        })
        .collect();

    Ok(Json(ProjectsResponse { projects }))
}
