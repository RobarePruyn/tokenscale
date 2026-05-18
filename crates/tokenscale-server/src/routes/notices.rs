//! `/api/v1/notices` — list active in-app release notices and record
//! dismissals.
//!
//! See [`crate::notices`] for the notice catalog and persistence shape.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::notices::ActiveNotice;
use crate::state::AppState;

#[derive(Serialize)]
pub struct NoticesResponse {
    pub notices: Vec<ActiveNotice>,
}

/// `GET /api/v1/notices` — return the set of release notices that are
/// currently active (not past expiry, not already dismissed).
pub async fn list_handler(State(state): State<AppState>) -> Json<NoticesResponse> {
    let dismissal_state = state.dismissal_store.load().await;
    Json(NoticesResponse {
        notices: crate::notices::active_notices(&dismissal_state),
    })
}

/// `POST /api/v1/notices/{id}/dismiss` — record dismissal of a notice.
/// Idempotent. Returns 204 No Content on success.
pub async fn dismiss_handler(
    State(state): State<AppState>,
    Path(notice_id): Path<String>,
) -> StatusCode {
    // Only allow dismissing notices the binary actually knows about — refuse
    // requests for arbitrary IDs to keep the dismissal-state file from
    // accumulating cruft from a misbehaving / outdated frontend.
    if !crate::notices::NOTICES.iter().any(|n| n.id == notice_id) {
        return StatusCode::NOT_FOUND;
    }
    match state.dismissal_store.dismiss(&notice_id).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(err) => {
            tracing::error!(error = %err, notice_id = %notice_id, "failed to persist dismissal");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
