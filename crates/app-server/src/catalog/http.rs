use super::{Bundle, Catalogs, LIMIT, build};
use crate::ApiState;
use axum::{
    Json,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
fn get(s: &ApiState) -> anyhow::Result<Arc<Catalogs>> {
    s.catalogs
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Catalog storage unavailable"))
}
fn allowed(s: &ApiState, h: &HeaderMap) -> bool {
    h.get("origin").and_then(|v| v.to_str().ok())
        == Some(format!("http://127.0.0.1:{}", s.port).as_str())
        && h.get("x-vision-control").is_some_and(|v| v == "1")
}
fn error(code: StatusCode, e: impl std::fmt::Display) -> Response {
    (code, Json(json!({"error":{"message":e.to_string()}}))).into_response()
}
pub async fn status(State(s): State<ApiState>) -> Response {
    match get(&s) {
        Ok(c) => Json(c.status()).into_response(),
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}
pub async fn blob(State(s): State<ApiState>) -> Response {
    match get(&s) {
        Ok(c) => {
            let (id, bytes) = c.payload();
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .header("x-catalog-version", id)
                .body(Body::from(bytes.as_ref().clone()))
                .unwrap()
        }
        Err(_) => crate::web::catalog_fallback(),
    }
}
pub async fn export(State(s): State<ApiState>) -> Response {
    match get(&s) {
        Ok(c) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=VisionOverlay.visioncatalog",
                ),
            ],
            c.export().as_ref().clone(),
        )
            .into_response(),
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub revision: Option<String>,
}
pub async fn prepare(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(request): Json<Prepare>,
) -> Response {
    if !allowed(&s, &h) {
        return error(
            StatusCode::FORBIDDEN,
            "Use the catalog controls in this application",
        );
    }
    let c = match get(&s) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let Ok(_guard) = c.operation.try_lock() else {
        return error(StatusCode::CONFLICT, "Another catalog operation is running");
    };
    match build::download(request.revision)
        .await
        .and_then(|b| c.stage(b))
    {
        Ok(_) => Json(c.status()).into_response(),
        Err(e) => {
            c.fail(&e);
            error(StatusCode::BAD_REQUEST, format!("{e:#}"))
        }
    }
}
pub async fn import(State(s): State<ApiState>, h: HeaderMap, bytes: Bytes) -> Response {
    if !allowed(&s, &h) {
        return error(
            StatusCode::FORBIDDEN,
            "Use the catalog controls in this application",
        );
    }
    let c = match get(&s) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let Ok(_guard) = c.operation.try_lock() else {
        return error(StatusCode::CONFLICT, "Another catalog operation is running");
    };
    let result = (|| {
        anyhow::ensure!(bytes.len() <= LIMIT, "Bundle size limit");
        let b: Bundle = serde_json::from_slice(&bytes)?;
        c.stage(b)
    })();
    match result {
        Ok(_) => Json(c.status()).into_response(),
        Err(e) => {
            c.fail(&e);
            error(StatusCode::BAD_REQUEST, format!("{e:#}"))
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activate {
    pub id: String,
    pub expected_active: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rollback {
    pub expected_active: String,
}
pub async fn activate(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(r): Json<Activate>,
) -> Response {
    mutate(s, h, move |c| c.activate(&r.id, &r.expected_active))
}
pub async fn rollback(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(r): Json<Rollback>,
) -> Response {
    mutate(s, h, move |c| c.rollback(&r.expected_active))
}
fn mutate(s: ApiState, h: HeaderMap, f: impl FnOnce(&Catalogs) -> anyhow::Result<()>) -> Response {
    if !allowed(&s, &h) {
        return error(
            StatusCode::FORBIDDEN,
            "Use the catalog controls in this application",
        );
    }
    let c = match get(&s) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let Ok(_guard) = c.operation.try_lock() else {
        return error(StatusCode::CONFLICT, "Another catalog operation is running");
    };
    match f(&c) {
        Ok(()) => Json(c.status()).into_response(),
        Err(e) => {
            c.fail(&e);
            error(StatusCode::CONFLICT, format!("{e:#}"))
        }
    }
}
