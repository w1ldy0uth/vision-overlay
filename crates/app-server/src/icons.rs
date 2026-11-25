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
fn format(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() > LIMIT {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("png", "image/png"));
    }
    if bytes.len() >= 20
        && bytes.starts_with(b"RIFF")
        && matches!(&bytes[8..16], b"WEBPVP8 " | b"WEBPVP8L" | b"WEBPVP8X")
        && u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize == bytes.len() - 8
    {
        return Some(("webp", "image/webp"));
    }
    None
}
fn image(bytes: Vec<u8>, mime: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(bytes))
        .unwrap()
}

fn cached(root: &FsPath, name: &str) -> Option<Response> {
    for extension in ["png", "webp"] {
        if let Ok(bytes) = std::fs::read(root.join(format!("{name}.{extension}")))
            && let Some((_, mime)) = format(&bytes)
        {
            return Some(image(bytes, mime));
        }
    }
    None
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
    if let Some(response) = cached(&root, &name) {
        return response;
    }
    if FAILURES
        .lock()
        .unwrap()
        .get(&name)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    {
        if let Some(response) = last_good(&base, &name) {
            return response;
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
    if let Some(response) = cached(&root, &name) {
        return response;
    }
    let mut result = Err(anyhow::anyhow!("Icon unavailable"));
    // Project Amber includes inventory icons missing from the other mirrors.
    let mut urls = vec![
        format!("https://enka.network/ui/{name}.png"),
        format!("https://gi.yatta.moe/assets/UI/{name}.png"),
        format!(
            "https://raw.githubusercontent.com/PathOfGenshin/resources/main/resources/gi/Sprite/{name}.png"
        ),
    ];
    if name.starts_with("UI_MonsterIcon_") {
        urls.push(format!("https://static.nanoka.cc/assets/gi/{name}.webp"));
    }
    // The ingame database serves additional item thumbnails by numeric item ID.
    if let Some(id) = name
        .strip_prefix("UI_ItemIcon_")
        .and_then(|suffix| suffix.parse::<u32>().ok())
    {
        urls.push(format!(
            "https://cdn.th.gl/genshin-impact/icons/db/item_{id}.webp"
        ));
    }
    for url in urls {
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
            let (extension, mime) =
                format(&bytes).ok_or_else(|| anyhow::anyhow!("Invalid icon format"))?;
            Ok::<_, anyhow::Error>((bytes, extension, mime))
        }
        .await;
        if result.is_ok() {
            break;
        }
    }
    match result {
        Ok((bytes, extension, mime)) => {
            FAILURES.lock().unwrap().remove(&name);
            let file = root.join(format!("{name}.{extension}"));
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
            image(bytes, mime)
        }
        Err(_) => {
            let mut failures = FAILURES.lock().unwrap();
            failures.retain(|_, t| t.elapsed() < Duration::from_secs(60));
            if failures.len() >= 512 {
                failures.clear();
            }
            failures.insert(name.clone(), Instant::now());
            drop(failures);
            if let Some(response) = last_good(&base, &name) {
                return response;
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

fn last_good(root: &FsPath, name: &str) -> Option<Response> {
    if let Some(response) = cached(root, name) {
        return Some(response);
    }
    for entry in std::fs::read_dir(root).ok()?.flatten().take(128) {
        let key = entry.file_name();
        let key = key.to_string_lossy();
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if let Some(response) = cached(&entry.path(), name) {
            return Some(response);
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
        assert!(format(b"<html>not an image</html>").is_none());
        assert_eq!(format(b"\x89PNG\r\n\x1a\n"), Some(("png", "image/png")));
        assert_eq!(
            format(b"RIFF\x0c\0\0\0WEBPVP8L\0\0\0\0"),
            Some(("webp", "image/webp"))
        );
        assert!(format(b"RIFF\x10\0\0\0WEBPVP8L\0\0\0\0").is_none());
    }
}
