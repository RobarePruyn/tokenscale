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
        .route("/api/v1/projects", get(routes::projects::list_handler))
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
    use std::sync::Arc;
    use tokenscale_core::PricingFile;
    use tokenscale_store::Database;
    use tower::util::ServiceExt;

    /// Production-flavored pricing fixture covering the four Anthropic
    /// models present in our live data. Used by tests that exercise the
    /// billable-total computation.
    const TEST_PRICING_TOML: &str = r#"
schema_version = 1
file_status = "production"

[providers.anthropic]
display_name = "Anthropic"

[providers.anthropic.models."claude-opus-4-7"]
display_name = "Claude Opus 4.7"
valid_from = "2026-04-28"
input_usd_per_mtok = 15.00
output_usd_per_mtok = 75.00
cache_read_usd_per_mtok = 1.50
cache_write_5m_multiplier = 1.25
cache_write_1h_multiplier = 2.00
source_url = "https://example.test"
source_accessed_at = "2026-04-28"
"#;

    fn test_pricing() -> Arc<PricingFile> {
        Arc::new(PricingFile::parse(TEST_PRICING_TOML).unwrap())
    }

    async fn build_test_app() -> axum::Router {
        let database = Database::open_in_memory_for_tests().await.unwrap();
        build_router(AppState::new(database, test_pricing()))
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
        // Pricing block from the test fixture.
        assert_eq!(body["pricing"]["schema_version"], 1);
        assert_eq!(body["pricing"]["file_status"], "production");
        assert_eq!(body["pricing"]["model_count"], 1);
        assert_eq!(body["pricing"]["needs_review"], false);
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
    async fn daily_usage_filters_out_zero_total_models_and_returns_breakdown() {
        // Two models — one with real usage, one whose entire window is 0
        // tokens (the `<synthetic>` case Claude Code emits). The zero-total
        // model should not appear in the response. The real one should
        // come back with a per-token-type breakdown and a non-null
        // billable_total since the test pricing fixture covers it.
        use chrono::{TimeZone, Utc};
        use tokenscale_core::Event;
        use tokenscale_store::insert_events;

        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |model: &str, request_id: &str, input: u64, output: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: model.to_owned(),
            input_tokens: input,
            output_tokens: output,
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
                make_event("claude-opus-4-7", "r1", 1_000, 100),
                make_event("<synthetic>", "r2", 0, 0),
                make_event("<synthetic>", "r3", 0, 0),
            ],
        )
        .await
        .unwrap();

        let app = build_router(AppState::new(database, test_pricing()));
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

        // Synthetic gone, opus visible.
        assert_eq!(body["models"], serde_json::json!(["claude-opus-4-7"]));
        assert_eq!(body["modelsWithoutPricing"], serde_json::json!([]));
        assert_eq!(
            body["tokenTypes"],
            serde_json::json!([
                "input",
                "output",
                "cache_read",
                "cache_write_5m",
                "cache_write_1h"
            ])
        );

        // Per-token-type breakdown is present.
        let opus = &body["rows"][0]["byModel"]["claude-opus-4-7"];
        assert_eq!(opus["input"], 1_000);
        assert_eq!(opus["output"], 100);
        assert_eq!(opus["cache_read"], 0);
        assert_eq!(opus["cache_write_5m"], 0);
        assert_eq!(opus["cache_write_1h"], 0);
        // Billable total = 1000*1.0 + 100*5.0 = 1500
        assert_eq!(opus["billable_total"], 1_500);
    }

    #[tokio::test]
    async fn projects_endpoint_returns_distinct_projects_with_totals() {
        use chrono::{TimeZone, Utc};
        use tokenscale_core::Event;
        use tokenscale_store::insert_events;

        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |project: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some(project.to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        insert_events(
            &database,
            &[
                make_event("/proj/big", "r1", 1_000),
                make_event("/proj/small", "r2", 10),
            ],
        )
        .await
        .unwrap();

        let app = build_router(AppState::new(database, test_pricing()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/projects?from=2026-04-21&to=2026-04-21")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        // Sorted by total tokens desc.
        assert_eq!(body["projects"][0]["project_id"], "/proj/big");
        assert_eq!(body["projects"][0]["total_tokens"], 1_000);
        assert_eq!(body["projects"][1]["project_id"], "/proj/small");
    }

    #[tokio::test]
    async fn daily_usage_filters_by_project_query_param() {
        use chrono::{TimeZone, Utc};
        use tokenscale_core::Event;
        use tokenscale_store::insert_events;

        let database = Database::open_in_memory_for_tests().await.unwrap();
        let make_event = |project: &str, request_id: &str, tokens: u64| Event {
            source: "claude_code".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
            model: "claude-opus-4-7".to_owned(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            request_id: Some(request_id.to_owned()),
            content_hash: None,
            session_id: Some("s".to_owned()),
            project_id: Some(project.to_owned()),
            workspace_id: None,
            api_key_id: None,
            raw: None,
        };
        insert_events(
            &database,
            &[
                make_event("/proj/alpha", "r1", 100),
                make_event("/proj/beta", "r2", 200),
            ],
        )
        .await
        .unwrap();

        let app = build_router(AppState::new(database, test_pricing()));

        // Filter to alpha only.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=2026-04-21&to=2026-04-21&project=/proj/alpha")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let opus = &body["rows"][0]["byModel"]["claude-opus-4-7"];
        assert_eq!(opus["input"], 100);

        // Filter to both via comma-separated list.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=2026-04-21&to=2026-04-21&project=/proj/alpha,/proj/beta")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let opus = &body["rows"][0]["byModel"]["claude-opus-4-7"];
        assert_eq!(opus["input"], 300);
    }

    #[tokio::test]
    async fn daily_usage_omits_billable_total_for_unpriced_models() {
        // A model present in the data but absent from the pricing file
        // should still appear in the response (its raw counts are
        // meaningful) but with no billable_total. The model's name lands
        // in modelsWithoutPricing so the dashboard can flag it.
        use chrono::{TimeZone, Utc};
        use tokenscale_core::Event;
        use tokenscale_store::insert_events;

        let database = Database::open_in_memory_for_tests().await.unwrap();
        insert_events(
            &database,
            &[Event {
                source: "claude_code".to_owned(),
                occurred_at: Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap(),
                model: "claude-future-9-9".to_owned(),
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
                request_id: Some("r-future".to_owned()),
                content_hash: None,
                session_id: Some("s".to_owned()),
                project_id: Some("/p".to_owned()),
                workspace_id: None,
                api_key_id: None,
                raw: None,
            }],
        )
        .await
        .unwrap();

        let app = build_router(AppState::new(database, test_pricing()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/usage/daily?from=2026-04-21&to=2026-04-21")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        let row = &body["rows"][0]["byModel"]["claude-future-9-9"];
        assert_eq!(row["input"], 100);
        // billable_total is absent rather than null in JSON via skip_serializing_if.
        assert!(row.get("billable_total").is_none());
        assert_eq!(
            body["modelsWithoutPricing"],
            serde_json::json!(["claude-future-9-9"])
        );
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
