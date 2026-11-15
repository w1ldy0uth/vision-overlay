//! Fixed-origin UI assets, independent of account data and inventory freshness.
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    path::{Path as FsPath, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

static FAILURES: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static REQUESTS: Semaphore = Semaphore::const_new(4);
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("VisionOverlay/0.1 (local UI icon cache)")
        .build()
        .expect("icon HTTP client")
});
const LIMIT: usize = 1024 * 1024;
fn valid_name(name: &str) -> bool {
    name.len() <= 160
        && (name.starts_with("UI_") || name.starts_with("Skill_"))
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn png(bytes: &[u8]) -> bool {
    bytes.len() <= LIMIT && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}
fn image(bytes: Vec<u8>) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(bytes))
        .unwrap()
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconQuery {
    v: Option<String>,
    retry: Option<u8>,
}
pub async fn icon(
    State(state): State<crate::ApiState>,
    Path(name): Path<String>,
    Query(query): Query<IconQuery>,
) -> Response {
    if !valid_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if query.retry.is_some_and(|n| n > 2) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let base = state
        .automation_dir
        .as_deref()
        .and_then(FsPath::parent)
        .map(|p| p.join("icon-cache"))
        .unwrap_or_else(|| PathBuf::from("local-data/icon-cache"));
    let mut root = base.clone();
    if let Some(version) = query.v {
        if version.len() != 64 || !version.bytes().all(|b| b.is_ascii_hexdigit()) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        root.push(version);
    }
    let file = root.join(format!("{name}.png"));
    if let Ok(bytes) = std::fs::read(&file)
        && png(&bytes)
    {
        return image(bytes);
    }
    if FAILURES
        .lock()
        .unwrap()
        .get(&name)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    {
        if let Some(bytes) = last_good(&base, &name) {
            return image(bytes);
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [
                (header::RETRY_AFTER, "60"),
                (header::CACHE_CONTROL, "no-store"),
            ],
        )
            .into_response();
    }
    let Ok(_permit) = REQUESTS.acquire().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if let Ok(bytes) = std::fs::read(&file)
        && png(&bytes)
    {
        return image(bytes);
    }
    let mut result = Err(anyhow::anyhow!("Icon unavailable"));
    // Both providers publish UI assets publicly. The second also works where
    // the first denies automated clients; older mirrors can legitimately lack new icons.
    for url in [
        format!("https://enka.network/ui/{name}.png"),
        format!(
            "https://raw.githubusercontent.com/PathOfGenshin/resources/main/resources/gi/Sprite/{name}.png"
        ),
    ] {
        result = async {
            let mut response = CLIENT.get(url).send().await?.error_for_status()?;
            if response.content_length().is_some_and(|n| n > LIMIT as u64) {
                anyhow::bail!("Icon size limit");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                anyhow::ensure!(bytes.len() + chunk.len() <= LIMIT, "Icon size limit");
                bytes.extend_from_slice(&chunk);
            }
            anyhow::ensure!(png(&bytes), "Invalid icon format");
            Ok::<_, anyhow::Error>(bytes)
        }
        .await;
        if result.is_ok() {
            break;
        }
    }
    match result {
        Ok(bytes) => {
            FAILURES.lock().unwrap().remove(&name);
            // Cache failure must never prevent showing a successfully loaded icon.
            if let Some(parent) = file.parent()
                && std::fs::create_dir_all(parent).is_ok()
            {
                let temporary = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
                if std::fs::write(&temporary, &bytes).is_ok() {
                    let _ = std::fs::rename(&temporary, &file);
                    let _ = std::fs::remove_file(&temporary);
                }
            }
            image(bytes)
        }
        Err(_) => {
            let mut failures = FAILURES.lock().unwrap();
            failures.retain(|_, t| t.elapsed() < Duration::from_secs(60));
            if failures.len() >= 512 {
                failures.clear();
            }
            failures.insert(name.clone(), Instant::now());
            drop(failures);
            if let Some(bytes) = last_good(&base, &name) {
                return image(bytes);
            }
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [
                    (header::RETRY_AFTER, "60"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
            )
                .into_response()
        }
    }
}

fn last_good(root: &FsPath, name: &str) -> Option<Vec<u8>> {
    let filename = format!("{name}.png");
    let direct = root.join(&filename);
    if let Ok(bytes) = std::fs::read(direct)
        && png(&bytes)
    {
        return Some(bytes);
    }
    for entry in std::fs::read_dir(root).ok()?.flatten().take(128) {
        let key = entry.file_name();
        let key = key.to_string_lossy();
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join(&filename))
            && png(&bytes)
        {
            return Some(bytes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icon_names_cannot_select_paths_or_remote_hosts() {
        assert!(valid_name("UI_ItemIcon_100011"));
        for s in [
            "../secret",
            "UI_../secret",
            "https://example.com",
            "UI_x.png",
            "UI_x?token=x",
            "UI_x%2Fsecret",
        ] {
            assert!(!valid_name(s));
        }
        assert!(!valid_name(&format!("UI_{}", "a".repeat(161))));
        assert!(!png(b"<html>not a PNG</html>"));
    }
}
