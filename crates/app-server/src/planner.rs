use crate::ApiState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct GoalQuery {
    preset_id: Option<String>,
}

fn authorized(state: &ApiState, headers: &HeaderMap) -> bool {
    headers.get("origin").and_then(|v| v.to_str().ok())
        == Some(format!("http://127.0.0.1:{}", state.port).as_str())
        && headers.get("x-vision-control").is_some_and(|v| v == "1")
}

pub async fn presets(State(s): State<ApiState>, Path(id): Path<String>) -> Response {
    let Some(w) = s.writer else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match w.lock().await.planner_presets(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn save_preset(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&s, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(w) = s.writer else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match w.lock().await.save_planner_preset(&id, body).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => (StatusCode::CONFLICT, Json(json!({"error":{"message":"Не удалось сохранить пресет. Проверьте, свободно ли название и не достигнут ли лимит пресетов."}}))).into_response(),
    }
}

pub async fn list(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<GoalQuery>,
) -> Response {
    let Some(w) = s.writer else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match w
        .lock()
        .await
        .planner_goals(&id, query.preset_id.as_deref().unwrap_or("default"))
        .await
    {
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
    if !authorized(&s, &h) {
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
