mod archive;
mod logging;
mod redact;
use crate::ApiState;
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
pub use logging::{init as init_logging, record};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
static BUILD: Mutex<()> = Mutex::const_new(());
struct Download {
    temp: tempfile::TempDir,
    path: std::path::PathBuf,
    size: u64,
    filename: String,
    created: Instant,
}
static DOWNLOADS: LazyLock<Mutex<BTreeMap<String, Download>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    capture_id: String,
    #[serde(default)]
    include_packets: bool,
    #[serde(default)]
    description: String,
}
fn error(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({"error":{"message":message}}))).into_response()
}
pub async fn export(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(request): Json<ExportRequest>,
) -> Response {
    if h.get("origin").and_then(|v| v.to_str().ok())
        != Some(format!("http://127.0.0.1:{}", s.port).as_str())
        || h.get("x-vision-control").is_none_or(|v| v != "1")
    {
        return error(
            StatusCode::FORBIDDEN,
            "Use the application diagnostic controls",
        );
    }
    if uuid::Uuid::parse_str(&request.capture_id).is_err() || request.description.len() > 8192 {
        return error(
            StatusCode::BAD_REQUEST,
            "Invalid capture or issue description",
        );
    }
    let Ok(_build) = BUILD.try_lock() else {
        return error(
            StatusCode::CONFLICT,
            "Another diagnostic export is in progress",
        );
    };
    {
        let mut pending = DOWNLOADS.lock().await;
        pending.retain(|_, v| v.created.elapsed() < Duration::from_secs(900));
        if pending.len() >= 2 {
            return error(
                StatusCode::CONFLICT,
                "Download the prepared archives before creating another",
            );
        }
    }
    let Some(root) = s
        .automation_dir
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
    else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Diagnostic storage not configured",
        );
    };
    let _retention = crate::maintenance::diagnostic_guard().await;
    let facts = match s.reader.diagnostic_capture(&request.capture_id).await {
        Ok(v) => v,
        Err(e) => return crate::ApiError::from(e).into_response(),
    };
    let mut state = match s.reader.state().await {
        Ok(v) => v,
        Err(e) => return crate::ApiError::from(e).into_response(),
    };
    let profiles: Vec<_> = facts["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["profile_id"].as_str())
        .collect();
    if let Some(rows) = state["profiles"].as_array_mut() {
        rows.retain(|p| {
            p["profile_id"]
                .as_str()
                .is_some_and(|id| profiles.contains(&id))
        });
    }
    let catalog = s.catalogs.as_ref().map(|c| c.status());
    let result = tokio::task::spawn_blocking(move || {
        archive::package(
            &root,
            &request.capture_id,
            request.include_packets,
            &request.description,
            facts,
            state,
            catalog,
        )
    })
    .await;
    match result {
        Ok(Ok(built)) => {
            let id = uuid::Uuid::new_v4().to_string();
            let reply = json!({"download_url":format!("/api/v1/diagnostics/download/{id}"),"filename":format!("VisionOverlay-diagnostics-{}.zip",built.capture),"bytes":built.size.to_string(),"manifest":built.manifest});
            DOWNLOADS.lock().await.insert(
                id.clone(),
                Download {
                    temp: built.temp,
                    path: built.path,
                    size: built.size,
                    filename: format!("VisionOverlay-diagnostics-{}.zip", built.capture),
                    created: Instant::now(),
                },
            );
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(900)).await;
                DOWNLOADS.lock().await.remove(&id);
            });
            Json(reply).into_response()
        }
        Ok(Err(e)) => {
            crate::app_log!("Diagnostic export failed: {e:#}");
            error(
                StatusCode::BAD_REQUEST,
                "Could not package this capture; retry without raw packets or select another capture",
            )
        }
        Err(e) => {
            crate::app_log!("Diagnostic export worker failed: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Diagnostic export failed",
            )
        }
    }
}
pub async fn download(Path(id): Path<String>) -> Response {
    let Some(prepared) = DOWNLOADS.lock().await.remove(&id) else {
        return error(
            StatusCode::NOT_FOUND,
            "Diagnostic download is unavailable or already used",
        );
    };
    if prepared.created.elapsed() > Duration::from_secs(900) {
        return error(
            StatusCode::GONE,
            "Diagnostic download expired; export again",
        );
    }
    let file = match tokio::fs::File::open(&prepared.path).await {
        Ok(v) => v,
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Diagnostic download unavailable",
            );
        }
    };
    let length = prepared.size;
    let stream = async_stream::stream! {use tokio::io::AsyncReadExt;let _temp=prepared.temp;let mut file=file;let mut buffer=vec![0;65536];loop{match file.read(&mut buffer).await{Ok(0)=>break,Ok(n)=>yield Ok::<Bytes,std::io::Error>(Bytes::copy_from_slice(&buffer[..n])),Err(e)=>{yield Err(e);break;}}}};
    (
        [
            ("content-type", "application/zip".to_owned()),
            (
                "content-disposition",
                format!("attachment; filename={}", prepared.filename),
            ),
            ("content-length", length.to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}
#[macro_export]
macro_rules! app_log{($($arg:tt)*)=>{{let message=format!($($arg)*);eprintln!("{message}");$crate::diagnostics::record(&message);}};}

pub async fn shutdown_cleanup() {
    DOWNLOADS.lock().await.clear();
    record("Application server stopped");
}
