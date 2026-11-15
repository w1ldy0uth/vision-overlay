use axum::{
    body::Body,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
pub async fn index() -> Response {
    asset("/index.html")
}
pub fn bundled_catalog() -> Option<serde_json::Value> {
    ASSETS
        .iter()
        .find(|(name, _, _)| *name == "/catalog.json")
        .and_then(|(_, _, bytes)| serde_json::from_slice(bytes).ok())
}
pub fn catalog_fallback() -> Response {
    asset("/catalog.json")
}
pub async fn fallback(uri: axum::http::Uri) -> Response {
    asset(uri.path())
}
fn asset(path: &str) -> Response {
    if let Some((_, mime, bytes)) = ASSETS.iter().find(|(name, _, _)| *name == path) {
        return Response::builder()
            .header(header::CONTENT_TYPE, *mime)
            .body(Body::from(*bytes))
            .unwrap();
    }
    if path == "/index.html" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Web interface not built. Run scripts/Build-Web.ps1 and rebuild vision-server.",
        )
            .into_response();
    }
    (
        StatusCode::NOT_FOUND,
        axum::Json(
            serde_json::json!({"error":{"code":"not_found","message":"Resource not found"}}),
        ),
    )
        .into_response()
}
