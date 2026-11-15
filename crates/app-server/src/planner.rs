use crate::ApiState;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
pub async fn list(State(s): State<ApiState>, Path(id): Path<String>) -> Response {
    let Some(w) = s.writer else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match w.lock().await.planner_goals(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
pub async fn save(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if h.get("origin").and_then(|v| v.to_str().ok())
        != Some(format!("http://127.0.0.1:{}", s.port).as_str())
        || !h.get("x-vision-control").is_some_and(|v| v == "1")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(w) = s.writer else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match w.lock().await.save_planner_goal(&id,v).await {
        Ok(v)=>Json(v).into_response(),
        Err(_)=>(StatusCode::CONFLICT,Json(json!({"error":{"message":"Не удалось сохранить цель: данные изменились или цель некорректна. Обновите список и проверьте параметры."}}))).into_response(),
    }
}
