//! Loopback read API, durable SSE and authenticated transactional collector ingestion.
mod automation;
mod backups;
pub mod catalog;
pub mod diagnostics;
mod icons;
pub mod maintenance;
mod maintenance_api;
mod planner;
pub mod relay;
mod web;
pub mod wish_sync;
mod wishes;
use app_store::reader::{PageQuery, ReadError, Reader, decimal};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
pub use web::bundled_catalog;

#[derive(Clone)]
pub struct ApiState {
    pub reader: Reader,
    pub port: u16,
    pub shutdown: watch::Receiver<bool>,
    pub writer: Option<Arc<Mutex<app_store::Store>>>,
    pub token: Option<Arc<str>>,
    pub automation_dir: Option<std::path::PathBuf>,
    pub catalogs: Option<Arc<catalog::Catalogs>>,
}
pub struct ApiError(ReadError);
impl From<ReadError> for ApiError {
    fn from(e: ReadError) -> Self {
        Self(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, extra) = match self.0 {
            ReadError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                message,
                json!({}),
            ),
            ReadError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "Resource not found",
                json!({}),
            ),
            ReadError::ViewChanged => (
                StatusCode::CONFLICT,
                "view_changed",
                "State changed; restart pagination",
                json!({}),
            ),
            ReadError::ResyncRequired { earliest, current } => (
                StatusCode::CONFLICT,
                "resync_required",
                "Reload state and reconnect from its revision",
                json!({"earliest_available_after":earliest,"current_revision":current}),
            ),
            error => {
                crate::app_log!("API read failed: {error}");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "storage_unavailable",
                    "Stored data is temporarily unavailable",
                    json!({}),
                )
            }
        };
        (status,Json(json!({"error":{"code":code,"message":message,"retryable":status==StatusCode::SERVICE_UNAVAILABLE,"expected_sequence":null,"details":extra}}))).into_response()
    }
}
type ApiResult = Result<Json<Value>, ApiError>;
fn query(q: Result<Query<PageQuery>, QueryRejection>) -> Result<PageQuery, ApiError> {
    q.map(|q| q.0)
        .map_err(|_| ReadError::BadRequest("Invalid query parameters").into())
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/",get(web::index))
        .route("/icons/{name}",get(icons::icon))
        .route("/catalog.json",get(catalog::http::blob))
        .route("/api/v1/catalog",get(catalog::http::status))
        .route("/api/v1/catalog/export",get(catalog::http::export))
        .route("/api/v1/catalog/prepare",post(catalog::http::prepare).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/catalog/import",post(catalog::http::import).layer(DefaultBodyLimit::max(catalog::LIMIT)))
        .route("/api/v1/catalog/activate",post(catalog::http::activate).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/catalog/rollback",post(catalog::http::rollback).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/state",get(state_view))
        .route("/api/v1/maintenance",get(maintenance_api::preview))
        .route("/api/v1/maintenance/pin",post(maintenance_api::pin).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/maintenance/apply",post(maintenance_api::apply).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/backups/export",post(backups::export).layer(DefaultBodyLimit::max(131072)))
        .route("/api/v1/backups/inspect",post(backups::inspect).layer(DefaultBodyLimit::max(app_store::backup::MAX_ARCHIVE)))
        .route("/api/v1/backups/restore",post(backups::restore).layer(DefaultBodyLimit::max(app_store::backup::MAX_ARCHIVE)))
        .route("/api/v1/capture/automation",get(automation_view))
        .route("/api/v1/capture/control",post(capture_control).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/profiles",get(profiles))
        .route("/api/v1/profiles/{id}",get(profile))
        .route("/api/v1/profiles/{id}/inventory",get(inventory))
        .route("/api/v1/profiles/{id}/inventory/totals",get(material_totals))
        .route("/api/v1/profiles/{id}/characters",get(characters))
        .route("/api/v1/profiles/{id}/events",get(history))
        .route("/api/v1/profiles/{id}/planner-presets",get(planner::presets).post(planner::save_preset).layer(DefaultBodyLimit::max(4096)))
        .route("/api/v1/profiles/{id}/goals",get(planner::list).post(planner::save).layer(DefaultBodyLimit::max(16384)))
        .route("/api/v1/profiles/{id}/wishes/sync",post(wishes::sync).layer(DefaultBodyLimit::max(1024)))
        .route("/api/v1/profiles/{id}/wishes",get(wishes::list).post(wishes::import).layer(DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/api/v1/diagnostics/export",post(diagnostics::export).layer(DefaultBodyLimit::max(16384)))
        .route("/api/v1/diagnostics/download/{id}",get(diagnostics::download))
        .route("/api/v1/captures",get(captures))
        .route("/api/v1/captures/{id}",get(capture))
        .route("/api/v1/captures/{id}/snapshots",get(snapshots))
        .route("/api/v1/snapshots/{id}",get(snapshot))
        .route("/api/v1/events",get(events))
        .route("/internal/v1/captures/{id}/events",post(ingest))
        .route("/internal/v1/captures/{id}/delivery-failed",post(delivery_failed))
        .layer(DefaultBodyLimit::max(32*1024*1024))
        .fallback(web::fallback)
        .method_not_allowed_fallback(||async{(StatusCode::METHOD_NOT_ALLOWED,Json(json!({"error":{"code":"method_not_allowed","message":"This API is read-only","retryable":false,"expected_sequence":null}})))})
        .layer(middleware::from_fn_with_state(state.clone(),boundary))
        .with_state(state)
}
async fn boundary(State(s): State<ApiState>, req: Request, next: Next) -> Response {
    let host = format!("127.0.0.1:{}", s.port);
    let origin = format!("http://{host}");
    let valid = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        == Some(host.as_str())
        && req
            .headers()
            .get(header::ORIGIN)
            .is_none_or(|v| v.to_str().ok() == Some(origin.as_str()))
        && req
            .headers()
            .get("sec-fetch-site")
            .is_none_or(|v| v == "same-origin" || v == "none");
    let authorized = if req.uri().path().starts_with("/internal/") {
        match (&s.token, &s.writer) {
            (Some(token), Some(_)) => {
                let actual = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .unwrap_or("");
                actual.len() == token.len()
                    && actual
                        .bytes()
                        .zip(token.bytes())
                        .fold(0u8, |n, (a, b)| n | (a ^ b))
                        == 0
            }
            _ => false,
        }
    } else {
        true
    };
    let web_resource =
        !req.uri().path().starts_with("/api/") && !req.uri().path().starts_with("/internal/");
    let mut response = if valid && !authorized {
        (StatusCode::UNAUTHORIZED,Json(json!({"error":{"code":"unauthorized","message":"Collector authentication is required","retryable":false}}))).into_response()
    } else if valid {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN,Json(json!({"error":{"code":"origin_not_allowed","message":"Use the canonical loopback origin","retryable":false,"expected_sequence":null}}))).into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        (if web_resource {"default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' https://act-webstatic.hoyoverse.com/map_manage/ https://upload-static.hoyoverse.com/map_manage/ https://act.hoyoverse.com/map_manage/ https://uploadstatic.hoyoverse.com/ys-obc/ https://act-upload.hoyoverse.com/map_manage/; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"} else {"default-src 'none'; frame-ancestors 'none'; base-uri 'none'"})
            .parse()
            .unwrap(),
    );
    response
}
async fn state_view(State(s): State<ApiState>) -> ApiResult {
    let mut v = s.reader.state().await?;
    v["capabilities"]["http_ingest"] = json!(s.writer.is_some());
    v["capabilities"]["live_collector"] = json!(!v["active_game_profile_id"].is_null());
    Ok(Json(v))
}
async fn automation_view(State(s): State<ApiState>) -> Json<Value> {
    Json(automation::status(s.automation_dir.as_deref()))
}
async fn capture_control(
    State(s): State<ApiState>,
    headers: HeaderMap,
    body: Result<Json<automation::ControlRequest>, JsonRejection>,
) -> Response {
    let origin = format!("http://127.0.0.1:{}", s.port);
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(origin.as_str())
        || headers.get("x-vision-control").is_none_or(|v| v != "1")
    {
        return (StatusCode::FORBIDDEN,Json(json!({"error":{"code":"origin_not_allowed","message":"Use the application capture controls"}}))).into_response();
    }
    let Json(req) = match body {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":{"code":"invalid_request"}})),
            )
                .into_response();
        }
    };
    match automation::request(s.automation_dir.as_deref(), req) {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(automation::ControlError(status, code)) => {
            (status, Json(json!({"error":{"code":code}}))).into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IngestBatch {
    events: Vec<Value>,
}
async fn delivery_failed(State(s): State<ApiState>, Path(id): Path<String>) -> Response {
    let Some(writer) = s.writer else {
        return ApiError(ReadError::NotFound).into_response();
    };
    match writer.lock().await.fail_delivery(&id).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"code":"storage_unavailable","retryable":true}})),
        )
            .into_response(),
    }
}
async fn ingest(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    body: Result<Json<IngestBatch>, JsonRejection>,
) -> Response {
    let Some(writer) = s.writer else {
        return ApiError(ReadError::NotFound).into_response();
    };
    let batch=match body{Ok(v)=>v.0,Err(_)=>return (StatusCode::BAD_REQUEST,Json(json!({"error":{"code":"invalid_batch","message":"Invalid JSON event batch","retryable":false}}))).into_response()};
    match writer.lock().await.ingest(&id, batch.events).await {
        Ok(ack) => Json(ack).into_response(),
        Err(app_store::ingest::Error::Invalid {
            code,
            message,
            expected,
        }) => {
            let status = if matches!(
                code,
                "sequence_gap"
                    | "event_conflict"
                    | "identity_conflict"
                    | "lease_conflict"
                    | "epoch_closed"
                    | "capture_closed"
                    | "source_conflict"
            ) {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status,Json(json!({"error":{"code":code,"message":message,"expected_sequence":expected,"retryable":false}}))).into_response()
        }
        Err(error) => {
            crate::app_log!("Collector transaction failed: {error}");
            (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"code":"storage_unavailable","message":"Event batch was not acknowledged; retry safely","retryable":true}}))).into_response()
        }
    }
}
async fn profiles(State(s): State<ApiState>) -> ApiResult {
    Ok(Json(s.reader.profiles().await?))
}
async fn profile(State(s): State<ApiState>, Path(id): Path<String>) -> ApiResult {
    Ok(Json(s.reader.profile(&id).await?))
}
async fn inventory(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.entities(&id, query(q)?, false).await?))
}
async fn material_totals(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.material_totals(&id, query(q)?).await?))
}
async fn characters(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.entities(&id, query(q)?, true).await?))
}
async fn history(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.history(&id, query(q)?).await?))
}
async fn captures(
    State(s): State<ApiState>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.captures(query(q)?).await?))
}
async fn capture(State(s): State<ApiState>, Path(id): Path<String>) -> ApiResult {
    Ok(Json(s.reader.capture(&id).await?))
}
async fn snapshots(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult {
    Ok(Json(s.reader.snapshots(&id, query(q)?).await?))
}
async fn snapshot(State(s): State<ApiState>, Path(id): Path<String>) -> ApiResult {
    Ok(Json(s.reader.snapshot(&id).await?))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct EventQuery {
    after: Option<String>,
}
async fn events(
    State(s): State<ApiState>,
    headers: HeaderMap,
    q: Result<Query<EventQuery>, QueryRejection>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let q = q.map_err(|_| ReadError::BadRequest("Invalid SSE query"))?.0;
    let header = headers
        .get("last-event-id")
        .map(|h| {
            h.to_str()
                .map(str::to_owned)
                .map_err(|_| ReadError::BadRequest("Invalid Last-Event-ID"))
        })
        .transpose()?;
    let after = header
        .or(q.after)
        .ok_or(ReadError::BadRequest("after or Last-Event-ID is required"))?;
    let mut cursor = decimal(&after)?;
    // Validate before sending HTTP 200. Read again inside the stream to cover the race.
    s.reader.changes(cursor, 1).await?;
    let stream = async_stream::stream! {
        let mut shutdown=s.shutdown.clone();
        let mut tick=tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *shutdown.borrow(){break;}
            tokio::select!{
                changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){break;}},
                _=tick.tick()=>{
                    match s.reader.changes(cursor,100).await {
                        Ok(batch)=>{
                            for change in batch{
                                let next=change["revision"].as_str().unwrap();
                                cursor=decimal(next).expect("Stored revision is i64");
                                let event=Event::default().event("change").id(next).data(serde_json::to_string(&change).unwrap());
                                yield Ok(event);
                            }
                        }
                        Err(ReadError::ResyncRequired{earliest,current})=>{
                            yield Ok(Event::default().event("resync_required").data(json!({"earliest_available_after":earliest,"current_revision":current}).to_string()));
                            break;
                        }
                        Err(error)=>{
                            crate::app_log!("SSE storage read failed: {error}");
                            yield Ok(Event::default().event("stream_error").data("{\"code\":\"storage_unavailable\"}"));
                            break;
                        }
                    }
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("transport-only"),
    ))
}
