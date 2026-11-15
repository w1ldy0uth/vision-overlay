use crate::{ApiState, maintenance};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
fn root(state: &ApiState) -> anyhow::Result<std::path::PathBuf> {
    Ok(state
        .automation_dir
        .as_deref()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| anyhow::anyhow!("Maintenance is not configured"))?
        .to_path_buf())
}
fn allowed(state: &ApiState, headers: &HeaderMap) -> bool {
    headers.get("origin").and_then(|v| v.to_str().ok())
        == Some(format!("http://127.0.0.1:{}", state.port).as_str())
        && headers.get("x-vision-control").is_some_and(|v| v == "1")
}
fn error(error: impl ToString) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({"error":{"message":error.to_string()}})),
    )
        .into_response()
}
pub async fn preview(State(state): State<ApiState>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let root = root(&state)?;
            let writer = state
                .writer
                .ok_or_else(|| anyhow::anyhow!("Writer unavailable"))?;
            maintenance::preview(writer, root).await
        })
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => error(e),
        Err(e) => error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    artifact_id: uuid::Uuid,
    pinned: bool,
}
pub async fn pin(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<Pin>,
) -> Response {
    if !allowed(&state, &headers) {
        return (StatusCode::FORBIDDEN, "Use the application controls").into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let root = root(&state)?;
            let writer = state
                .writer
                .ok_or_else(|| anyhow::anyhow!("Writer unavailable"))?;
            maintenance::pin(
                writer,
                root,
                request.artifact_id.to_string(),
                request.pinned,
            )
            .await
        })
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => error(e),
        Err(e) => error(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Apply {
    plan_id: String,
}
pub async fn apply(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<Apply>,
) -> Response {
    if !allowed(&state, &headers) {
        return (StatusCode::FORBIDDEN, "Use the application controls").into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let root = root(&state)?;
            let writer = state
                .writer
                .ok_or_else(|| anyhow::anyhow!("Writer unavailable"))?;
            maintenance::apply(writer, root, request.plan_id).await
        })
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => error(e),
        Err(e) => error(e),
    }
}
