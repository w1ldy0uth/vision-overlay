//! Login-triggered background sync. Network/cache access never holds the store lock.
use app_store::{Store, wish_sync::WishContext};
use serde::Deserialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, watch};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    python: Option<String>,
    interface_index: Option<u32>,
}
fn helper() -> Option<PathBuf> {
    std::env::current_exe().ok()?.ancestors().find_map(|p| {
        let path = p.join("scripts/sync_wish_history.py");
        path.is_file().then_some(path)
    })
}
async fn same(writer: &Arc<Mutex<Store>>, context: &WishContext) -> bool {
    writer
        .lock()
        .await
        .wish_context()
        .await
        .ok()
        .flatten()
        .as_ref()
        == Some(context)
}
async fn download(
    writer: &Arc<Mutex<Store>>,
    context: &WishContext,
    root: &Path,
    stop: &mut watch::Receiver<bool>,
) -> anyhow::Result<(String, Option<Value>)> {
    let helper = helper().ok_or_else(|| anyhow::anyhow!("Wish helper unavailable"))?;
    let config_path = root.join("wish-sync.json");
    let config: Config = if config_path.exists() {
        anyhow::ensure!(
            std::fs::metadata(&config_path)?.len() <= 4096,
            "Oversized sync config"
        );
        serde_json::from_slice(&std::fs::read(config_path)?)?
    } else {
        Config::default()
    };
    let dir = tempfile::tempdir()?;
    let embedded = std::env::current_exe().ok().and_then(|exe| {
        let path = exe.parent()?.join("python/python.exe");
        path.is_file().then_some(path)
    });
    let interpreter = config
        .python
        .as_ref()
        .map(PathBuf::from)
        .or(embedded)
        .unwrap_or_else(|| PathBuf::from("python"));
    let mut command = tokio::process::Command::new(interpreter);
    command
        .arg(helper)
        .arg("--uid")
        .arg(&context.uid)
        .arg("--region")
        .arg(&context.server_region)
        .arg("--directory")
        .arg(dir.path())
        .arg("--app-data")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(index) = config.interface_index {
        anyhow::ensure!((1..=0xffffff).contains(&index), "Invalid interface");
        command.arg("--interface-index").arg(index.to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let deadline = tokio::time::sleep(Duration::from_secs(180));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            result=child.wait()=>{anyhow::ensure!(result?.success(),"Wish helper failed");break;},
            _=&mut deadline=>{let _=child.kill().await;return Ok(("failed".into(),None));},
            _=stop.changed()=>{let _=child.kill().await;return Ok(("cancelled".into(),None));},
            _=tokio::time::sleep(Duration::from_secs(2))=>{
                if !same(writer,context).await {let _=child.kill().await;return Ok(("cancelled".into(),None));}
            }
        }
    }
    let result = dir.path().join("result.json");
    anyhow::ensure!(
        std::fs::metadata(&result)?.len() <= 1024,
        "Invalid sync result"
    );
    let result: Value = serde_json::from_slice(&std::fs::read(result)?)?;
    match result["status"].as_str() {
        Some("success") => {
            let path = dir.path().join("history.json");
            anyhow::ensure!(
                std::fs::metadata(&path)?.len() <= 32 * 1024 * 1024,
                "Oversized history"
            );
            Ok((
                "synced".into(),
                Some(serde_json::from_slice(&std::fs::read(path)?)?),
            ))
        }
        Some("needs_access") => Ok(("needs_access".into(), None)),
        Some("needs_network") => {
            crate::app_log!(
                "Wish history API is unreachable; check VPN or the configured outgoing interface"
            );
            Ok(("failed".into(), None))
        }
        _ => Ok(("failed".into(), None)),
    }
}
pub async fn run(
    writer: Arc<Mutex<Store>>,
    catalogs: Option<Arc<crate::catalog::Catalogs>>,
    root: PathBuf,
    mut stop: watch::Receiver<bool>,
) {
    let mut last = None;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! { _=stop.changed()=>break,_=tick.tick()=>{} }
        if *stop.borrow() {
            break;
        }
        let context = match writer.lock().await.wish_context().await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(context) = context else {
            last = None;
            continue;
        };
        let requested = writer
            .lock()
            .await
            .wish_sync_state(&context.profile_id)
            .await
            .ok()
            .is_some_and(|v| v["status"] == "requested");
        if last.as_ref() == Some(&context) && !requested {
            continue;
        }
        last = Some(context.clone());
        if writer
            .lock()
            .await
            .set_wish_sync(&context, "syncing", None)
            .await
            .is_err()
        {
            continue;
        }
        let (mut status, archive) = download(&writer, &context, &root, &mut stop)
            .await
            .unwrap_or_else(|_| ("failed".into(), None));
        let mut store = writer.lock().await;
        let current = store.wish_context().await.ok().flatten();
        if current.as_ref() != Some(&context) {
            status = "cancelled".into();
        }
        let catalog: Option<Value> = catalogs
            .as_ref()
            .and_then(|c| serde_json::from_slice(&c.payload().1).ok());
        let mut fetched = None;
        if status == "synced" {
            if let Some(archive) = archive {
                let complete = archive["coverage"]["all_available_pages_fetched"] == true
                    && archive["streams"].as_array().is_some_and(|s| s.len() == 5);
                if !complete {
                    status = "failed".into();
                } else {
                    fetched = archive["fetched_at"]
                        .as_str()
                        .and_then(|s| {
                            time::OffsetDateTime::parse(
                                s,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .ok()
                        })
                        .map(|d| (d.unix_timestamp_nanos() / 1_000_000) as i64);
                    if store
                        .import_wishes_with_catalog(&context.profile_id, archive, catalog.as_ref())
                        .await
                        .is_err()
                    {
                        status = "failed".into();
                        fetched = None;
                    }
                }
            } else {
                status = "failed".into();
            }
        }
        let _ = store.set_wish_sync(&context, &status, fetched).await;
    }
}
