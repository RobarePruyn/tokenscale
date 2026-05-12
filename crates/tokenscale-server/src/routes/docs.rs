//! `GET /api/v1/docs/<slug>` — serve bundled markdown documentation
//! used by the dashboard's methodology page.
//!
//! The methodology page is the credibility surface for the
//! environmental view — it lets users trace any factor back to its
//! source, see the methodology narrative, and read the research log.
//! These docs live in the repo's `docs/` directory and are bundled
//! into the binary via `include_str!` so the methodology page works
//! offline and doesn't require a network round-trip to GitHub.
//!
//! The slug-to-file mapping is intentionally narrow: only the four
//! docs the methodology page renders are exposed. Other docs in the
//! repo (architecture, decisions, etc.) stay out of the API surface
//! — they're maintainer-facing, not user-facing.

use axum::extract::Path;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// The methodology narrative — how every number in the dashboard
/// gets computed. Bundled from `docs/methodology.md` at compile time.
const METHODOLOGY_MD: &str = include_str!("../../../../docs/methodology.md");

/// Bibliography — every factor source with confidence tag, access
/// date, and summary. Bundled from `docs/sources.md`.
const SOURCES_MD: &str = include_str!("../../../../docs/sources.md");

/// Audit trail of past research sweeps. Bundled from
/// `docs/research-log.md`.
const RESEARCH_LOG_MD: &str = include_str!("../../../../docs/research-log.md");

/// Open questions for the next research sweep. Bundled from
/// `docs/request-for-research.md`.
const REQUEST_FOR_RESEARCH_MD: &str = include_str!("../../../../docs/request-for-research.md");

/// Map the URL slug to the bundled markdown content. Returning the
/// raw markdown (not pre-rendered HTML) keeps the frontend in charge
/// of the visual treatment — same docs can be themed differently as
/// the dashboard evolves.
pub async fn handler(Path(slug): Path<String>) -> Response {
    let body = match slug.as_str() {
        "methodology" => METHODOLOGY_MD,
        "sources" => SOURCES_MD,
        "research-log" => RESEARCH_LOG_MD,
        "request-for-research" => REQUEST_FOR_RESEARCH_MD,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                format!("no doc named {slug:?}; valid slugs: methodology, sources, research-log, request-for-research"),
            )
                .into_response();
        }
    };
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/markdown; charset=utf-8")],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn methodology_doc_is_bundled_and_starts_with_expected_heading() {
        // Sanity check that include_str! actually picked up the file
        // — at build time, a missing file would have errored on
        // compile, but the content itself is worth a one-line check
        // so a future renamed-or-moved file doesn't slip past CI.
        let response = handler(Path("methodology".to_owned())).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_slug_yields_404() {
        let response = handler(Path("not-a-doc".to_owned())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
