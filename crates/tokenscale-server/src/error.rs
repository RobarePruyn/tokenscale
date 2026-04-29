//! HTTP error type. Wraps `tokenscale-store::StoreError` and serializes a
//! consistent JSON shape.
//!
//! Internal errors map to `500 Internal Server Error`; user-input problems
//! (bad date format, etc.) map to `400 Bad Request`. The dashboard reads
//! `{ "error": { "code": "...", "message": "..." } }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("store error: {0}")]
    Store(#[from] tokenscale_store::StoreError),

    #[error("bad request: {0}")]
    BadRequest(String),
}

#[derive(Serialize)]
struct ApiErrorEnvelope<'a> {
    error: ApiErrorBody<'a>,
}

#[derive(Serialize)]
struct ApiErrorBody<'a> {
    code: &'a str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::Store(error) => {
                error!(?error, "store error in HTTP handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "an internal error occurred — see server logs".to_owned(),
                )
            }
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message.clone()),
        };
        (
            status,
            Json(ApiErrorEnvelope {
                error: ApiErrorBody { code, message },
            }),
        )
            .into_response()
    }
}
