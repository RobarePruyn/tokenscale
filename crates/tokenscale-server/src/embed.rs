//! Static-asset handler — serves the embedded React dashboard.
//!
//! Build behavior:
//!
//! - **Release builds** embed `frontend/dist/` into the binary at compile
//!   time via `rust-embed`. The folder must exist at compile time; if
//!   `npm run build` hasn't been run, the build fails with a clear error.
//! - **Debug builds** read from disk at runtime — no recompile needed when
//!   the frontend changes. If the folder is absent, the handler serves a
//!   small placeholder page pointing the developer at the right command.
//!
//! Routing model: SPA-friendly. Any path that doesn't match an embedded
//! asset falls back to `index.html` so client-side router paths
//! (`/dashboard/foo`) reach the SPA. The API routes are registered above
//! this fallback in `build_router`, so they never reach this handler.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// All files under `frontend/dist/`. The path is relative to this crate's
/// `Cargo.toml`, two levels up to the workspace root.
#[derive(RustEmbed)]
#[folder = "../../frontend/dist/"]
struct FrontendDist;

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidate_path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = FrontendDist::get(candidate_path) {
        return serve_embedded_file(candidate_path, &file.data);
    }

    // SPA fallback: any unknown path serves index.html. Lets a future React
    // Router handle `/something` without server-side awareness.
    if let Some(file) = FrontendDist::get("index.html") {
        return serve_embedded_file("index.html", &file.data);
    }

    placeholder_page()
}

fn serve_embedded_file(path: &str, bytes: &[u8]) -> Response {
    let mime_type = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type.as_ref())
        .body(Body::from(bytes.to_vec()))
        .unwrap_or_else(|_| internal_error_response())
}

fn placeholder_page() -> Response {
    const PLACEHOLDER_HTML: &str = "<!doctype html>
<html lang=\"en\"><head><meta charset=\"utf-8\"><title>tokenscale</title>
<style>body{font:16px/1.5 system-ui,-apple-system,sans-serif;max-width:640px;margin:3rem auto;padding:0 1rem;color:#0f172a}code{background:#f1f5f9;padding:2px 6px;border-radius:4px}</style>
</head><body>
<h1>tokenscale — dashboard not built</h1>
<p>The Rust API is running, but the frontend assets at <code>frontend/dist/</code> are absent or empty.</p>
<p>To use the dashboard:</p>
<ol>
<li>From <code>frontend/</code>, run <code>npm install</code> (once) then <code>npm run build</code>.</li>
<li>Rebuild the binary: <code>cargo build --release</code>.</li>
<li>Or for iterative frontend work: <code>npm run dev</code> from <code>frontend/</code> — Vite proxies API calls back to this server.</li>
</ol>
<p>The API itself is reachable directly — try <code><a href=\"/api/v1/health\">/api/v1/health</a></code>.</p>
</body></html>";
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(PLACEHOLDER_HTML))
        .unwrap_or_else(|_| internal_error_response())
}

fn internal_error_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
}
