use crate::ApiState;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

fn error(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({"error":{"message":message}}))).into_response()
}
pub async fn sync(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if headers.get("origin").and_then(|v| v.to_str().ok())
        != Some(format!("http://127.0.0.1:{}", s.port).as_str())
        || !headers.get("x-vision-control").is_some_and(|v| v == "1")
    {
        return error(StatusCode::FORBIDDEN, "Use the application wish controls");
    }
    let Some(writer) = &s.writer else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Writer unavailable");
    };
    let mut store = writer.lock().await;
    let Ok(Some(context)) = store.wish_context().await else {
        return error(
            StatusCode::CONFLICT,
            "Войдите в игру и дождитесь определения профиля.",
        );
    };
    if context.profile_id != id {
        return error(
            StatusCode::CONFLICT,
            "Синхронизация доступна для активного профиля игры.",
        );
    }
    if store
        .wish_sync_state(&id)
        .await
        .ok()
        .is_some_and(|v| matches!(v["status"].as_str(), Some("syncing" | "requested")))
    {
        return (StatusCode::ACCEPTED, Json(json!({"queued":true}))).into_response();
    }
    match store.set_wish_sync(&context, "requested", None).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"queued":true}))).into_response(),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Не удалось запланировать синхронизацию.",
        ),
    }
}
pub async fn list(State(s): State<ApiState>, Path(id): Path<String>) -> Response {
    let Some(writer) = &s.writer else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Writer unavailable");
    };
    match writer.lock().await.wishes(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => error(
            StatusCode::BAD_REQUEST,
            "Unable to read wish history for this profile",
        ),
    }
}
pub async fn import(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if headers.get("origin").and_then(|v| v.to_str().ok())
        != Some(format!("http://127.0.0.1:{}", s.port).as_str())
        || !headers.get("x-vision-control").is_some_and(|v| v == "1")
    {
        return error(StatusCode::FORBIDDEN, "Use the application wish controls");
    }
    let Some(writer) = &s.writer else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Writer unavailable");
    };
    let catalog: Option<Value> = s
        .catalogs
        .as_ref()
        .and_then(|c| serde_json::from_slice(&c.payload().1).ok());
    match writer
        .lock()
        .await
        .import_wishes_with_catalog(&id, v, catalog.as_ref())
        .await
    {
        Ok(v) => Json(v).into_response(),
        // Input is untrusted. Never reflect fields/URLs from uploaded files.
        Err(e) => {
            crate::app_log!("Wish import rejected: {e}");
            error(
                StatusCode::BAD_REQUEST,
                "Импорт отклонён: проверьте формат архива, профиль и отсутствие конфликтующих записей. Данные не изменены.",
            )
        }
    }
}
