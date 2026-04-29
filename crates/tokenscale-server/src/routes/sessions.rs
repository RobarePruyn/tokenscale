//! `GET /api/v1/sessions/recent` — most recently active sessions.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokenscale_store::recent_sessions;

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 25;
const MAX_LIMIT: i64 = 500;

#[derive(Deserialize)]
pub struct RecentSessionsParams {
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct RecentSessionsResponse {
    pub rows: Vec<SessionRow>,
}

#[derive(Serialize)]
pub struct SessionRow {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub first_event_at: String,
    pub last_event_at: String,
    pub event_count: i64,
    pub total_tokens: i64,
}

pub async fn recent_handler(
    State(state): State<AppState>,
    Query(params): Query<RecentSessionsParams>,
) -> Result<Json<RecentSessionsResponse>, ApiError> {
    let raw_limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    if raw_limit <= 0 {
        return Err(ApiError::BadRequest("limit must be > 0".to_owned()));
    }
    let clamped_limit = raw_limit.min(MAX_LIMIT);

    let rows = recent_sessions(&state.database, clamped_limit).await?;
    let rows = rows
        .into_iter()
        .map(|row| SessionRow {
            session_id: row.session_id,
            project_id: row.project_id,
            first_event_at: row.first_event_at,
            last_event_at: row.last_event_at,
            event_count: row.event_count,
            total_tokens: row.total_tokens,
        })
        .collect();
    Ok(Json(RecentSessionsResponse { rows }))
}
