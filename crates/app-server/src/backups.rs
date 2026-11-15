use crate::ApiState;
use app_store::backup::{self, Settings};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(json!({"error":{"message":message.to_string()}})),
    )
        .into_response()
}
fn allowed(s: &ApiState, h: &HeaderMap) -> bool {
    h.get("origin").and_then(|v| v.to_str().ok())
        == Some(format!("http://127.0.0.1:{}", s.port).as_str())
        && h.get("x-vision-control").is_some_and(|v| v == "1")
}
fn local(s: &ApiState) -> anyhow::Result<PathBuf> {
    Ok(s.automation_dir
        .as_deref()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("Backup storage not configured"))?
        .to_path_buf())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    settings: Settings,
    catalog: Value,
}
pub async fn export(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Json(request): Json<Export>,
) -> Response {
    if !allowed(&s, &headers) {
        return error(StatusCode::FORBIDDEN, "Use the application backup controls");
    }
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let writer = s
                .writer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Writer unavailable"))?;
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("export.visionbackup");
            let snapshot = writer.lock().await.backup_snapshot().await?;
            backup::package(snapshot, &path, request.settings, request.catalog).await?;
            Ok::<_, anyhow::Error>(fs::read(path)?)
        })
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!(e)));
    match result {
        Ok(bytes) => (
            [
                ("content-type", "application/octet-stream"),
                (
                    "content-disposition",
                    "attachment; filename=VisionOverlay.visionbackup",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
fn upload(bytes: &[u8]) -> anyhow::Result<(tempfile::TempDir, PathBuf)> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= backup::MAX_ARCHIVE,
        "Invalid archive size"
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("upload.visionbackup");
    fs::write(&path, bytes)?;
    Ok((temp, path))
}
pub async fn inspect(State(s): State<ApiState>, headers: HeaderMap, bytes: Bytes) -> Response {
    if !allowed(&s, &headers) {
        return error(StatusCode::FORBIDDEN, "Use the application backup controls");
    }
    let result=tokio::task::spawn_blocking(move || tokio::runtime::Handle::current().block_on(async move {
        let (_temp,path)=upload(&bytes)?;let archive=backup::validate(&path).await?;
        Ok::<_,anyhow::Error>(json!({"manifest":archive.manifest,"settings":archive.settings,"sha256":format!("{:x}",Sha256::digest(&bytes))}))
    })).await.unwrap_or_else(|e|Err(anyhow::anyhow!(e)));
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
fn existing_captures(root: &Path) -> Vec<String> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("realtime-"))
        .filter_map(|entry| fs::read(entry.path().join("delivery.json")).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter_map(|value| value["capture_id"].as_str().map(str::to_owned))
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .collect()
}
pub async fn restore(State(s): State<ApiState>, headers: HeaderMap, bytes: Bytes) -> Response {
    if !allowed(&s, &headers)
        || headers
            .get("x-vision-restore")
            .is_none_or(|v| v != "replace-all")
    {
        return error(
            StatusCode::FORBIDDEN,
            "Confirm replacement in the application",
        );
    }
    let status = crate::automation::status(s.automation_dir.as_deref());
    if status["capture_run_id"].is_string()
        || status["external_capture"] == true
        || matches!(
            status["capture_state"].as_str(),
            Some("starting" | "ready" | "stopping")
        )
    {
        return error(StatusCode::CONFLICT, "Stop capture before restoring");
    }
    let expected = headers
        .get("x-backup-sha256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if expected != format!("{:x}", Sha256::digest(&bytes)) {
        return error(
            StatusCode::BAD_REQUEST,
            "Inspect this backup before applying it",
        );
    }
    let result = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let local = local(&s)?;
            let (_temp, path) = upload(&bytes)?;
            let archive = backup::validate(&path).await?;
            let rollback_dir = local.join("restore-rollback");
            fs::create_dir_all(&rollback_dir)?;
            let rollback = rollback_dir.join(format!(
                "before-restore-{}.visionbackup",
                uuid::Uuid::new_v4()
            ));
            let blocked = existing_captures(&local);
            let writer = s
                .writer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Writer unavailable"))?;
            let mut writer = writer.lock().await;
            let status = crate::automation::status(s.automation_dir.as_deref());
            anyhow::ensure!(
                !status["capture_run_id"].is_string() && status["external_capture"] != true,
                "Capture started while checking backup; stop it first"
            );
            let mut response = writer.restore(&archive, &rollback, &blocked).await?;
            // Region assignments contain no credentials and are reconstructed from profile identity.
            let mut bindings = std::collections::BTreeMap::<String, Vec<String>>::new();
            for profile in archive.manifest.profiles.as_array().unwrap() {
                bindings
                    .entry(profile["uid"].as_str().unwrap().into())
                    .or_default()
                    .push(profile["server_region"].as_str().unwrap().into());
            }
            let regions: std::collections::BTreeMap<_, _> = bindings
                .into_iter()
                .filter(|(_, values)| values.len() == 1)
                .map(|(uid, values)| (uid, values[0].clone()))
                .collect();
            let regions_tmp = local.join(format!("regions-{}.tmp", uuid::Uuid::new_v4()));
            let config_result = fs::write(&regions_tmp, serde_json::to_vec_pretty(&regions)?)
                .and_then(|_| fs::rename(&regions_tmp, local.join("live-regions.json")));
            if config_result.is_err() {
                response["region_settings_warning"] =
                    json!("Configure server regions before starting capture");
            }
            Ok::<_, anyhow::Error>(response)
        })
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!(e)));
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
