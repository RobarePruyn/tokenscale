//! `tokenscale-server` — axum HTTP server.
//!
//! Exposes the tokenscale REST API and serves the embedded React dashboard.
//!
//! Phase 1 endpoints:
//!
//! - `GET /api/v1/health`
//! - `GET /api/v1/usage/daily?from=&to=&provider=`
//! - `GET /api/v1/usage/by-model?from=&to=&provider=`
//! - `GET /api/v1/sessions/recent?limit=`
//!
//! The `provider` query parameter accepts `all` (default) or a specific
//! provider slug. Even though v1 has only `anthropic`, the parameter is
//! present from day one so the API surface does not change in v2.
//!
//! Static-asset serving: production builds embed `frontend/dist/` into the
//! binary at compile time via `rust-embed`. The wiring is in `embed.rs`.

mod embed;
mod error;
mod routes;
mod state;

use std::net::SocketAddr;
use tracing::info;

pub use error::ApiError;
pub use state::AppState;

/// Build the axum router with all Phase 1 routes wired in. Exposed so tests
/// can drive it via `tower::ServiceExt::oneshot` without binding a port.
pub fn build_router(state: AppState) -> axum::Router {
    use axum::routing::get;
    use axum::Router;
    use tower_http::trace::TraceLayer;

    Router::new()
        .route("/api/v1/health", get(routes::health::handler))
        .route("/api/v1/usage/daily", get(routes::usage::daily_handler))
        .route(
            "/api/v1/usage/by-model",
            get(routes::usage::by_model_handler),
        )
        .route(
            "/api/v1/sessions/recent",
            get(routes::sessions::recent_handler),
        )
        .fallback(embed::static_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind to `bind_address` and serve until the process is signaled. Designed
/// to be called from the CLI's `tokenscale serve`.
pub async fn serve(state: AppState, bind_address: SocketAddr) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    info!(address = %bind_address, "tokenscale server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tokenscale_store::Database;
    use tower::util::ServiceExt;

    async fn build_test_app() -> axum::Router {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        build_router(AppState::new(database))
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok() {
        let app = build_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["total_events"], 0);
        assert_eq!(body["providers"], serde_json::json!(["anthropic"]));
    }

    #[tokio::test]
    async fn daily_usage_endpoint_with_no_data_returns_empty_rows() {
        let app = build_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=2026-04-20&to=2026-04-22")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["rows"], serde_json::json!([]));
        assert_eq!(body["models"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn daily_usage_rejects_bad_date_format() {
        let app = build_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=yesterday&to=today")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn daily_usage_filters_out_zero_total_models() {
        // Two models — one with real usage, one whose entire window is 0
        // tokens (the `<synthetic>` case Claude Code emits). The zero-total
        // model should not appear in the response.
        use chrono::{TimeZone, Utc};
        use tokenscale_core::Event;
        use tokenscale_store::insert_events;

        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |model: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: model.to_owned(),
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
                make_event("claude-opus-4-7", "r1", 1_000),
                make_event("<synthetic>", "r2", 0),
                make_event("<synthetic>", "r3", 0),
            ],
        )
        .await
        .unwrap();

        let app = build_router(AppState::new(database));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=2026-04-21&to=2026-04-21")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["models"], serde_json::json!(["claude-opus-4-7"]));
        // The single row should also have only the visible model in byModel.
        assert_eq!(body["rows"][0]["byModel"]["claude-opus-4-7"], 1_000);
        assert!(body["rows"][0]["byModel"].get("<synthetic>").is_none());
    }

    #[tokio::test]
    async fn recent_sessions_endpoint_with_no_data_returns_empty_rows() {
        let app = build_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions/recent?limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["rows"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn recent_sessions_rejects_zero_limit() {
        let app = build_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sessions/recent?limit=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
