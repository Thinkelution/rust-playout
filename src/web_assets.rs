//! Frontend assets are part of the executable, independent of its working directory.
include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

pub async fn serve(uri: axum::http::Uri) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match asset(path) {
        Some((mime, bytes)) => (
            [("content-type", mime), ("cache-control", "no-cache")],
            bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}
