//! Managed-file retention. Unknown historical folders are pinned on adoption.
use anyhow::{Context, Result, ensure};
use app_store::{Store, backup};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, watch};
use uuid::Uuid;

static CONTROL: Mutex<()> = Mutex::const_new(());
pub(crate) async fn diagnostic_guard() -> tokio::sync::MutexGuard<'static, ()> {
    CONTROL.lock().await
}
type Writer = Arc<Mutex<Store>>;
fn now() -> i64 {
    app_store::ingest::now_ms()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path) -> Result<Value> {
    ensure!(fs::metadata(path)?.len() <= 65536, "Metadata too large");
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
    )?)
}
fn reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn safe(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        !reparse(&fs::symlink_metadata(root)?),
        "Managed root cannot be a link/reparse point"
    );
    let base = root.canonicalize()?;
    let rel = Path::new(relative);
    ensure!(
        !relative.is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe relative artifact path"
    );
    let mut path = base.clone();
    for component in rel.components() {
        path.push(component);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            ensure!(!reparse(&meta), "Links/reparse points are protected");
        }
    }
    ensure!(path.starts_with(base), "Artifact escaped managed root");
    Ok(path)
}
fn inventory(path: &Path) -> Result<(u64, String, bool)> {
    fn visit(path: &Path, hash: &mut Sha256, size: &mut u64, manual: &mut bool) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        ensure!(!reparse(&meta), "Linked artifact is protected");
        hash.update(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes(),
        );
        hash.update(meta.len().to_le_bytes());
        if let Ok(t) = meta.modified() {
            hash.update(
                t.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_le_bytes(),
            );
        }
        if meta.is_dir() {
            let mut paths = fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            for p in paths {
                visit(&p, hash, size, manual)?;
            }
        } else {
            ensure!(meta.is_file(), "Unsupported artifact type");
            *size += meta.len();
            *manual |= path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("visionbackup"));
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut manual = false;
    visit(path, &mut hash, &mut bytes, &mut manual)?;
    Ok((bytes, format!("{:x}", hash.finalize()), manual))
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn delivery_complete(root: &Path, relative: &str, capture: &str) -> Result<()> {
    let dir = safe(root, relative)?;
    let descriptor = read(&dir.join("delivery.json"))?;
    ensure!(
        descriptor["schema_version"] == 1
            && descriptor["capture_id"] == capture
            && descriptor["log_file"] == "capture.jsonl",
        "Unknown delivery descriptor"
    );
    let ack = read(&dir.join("delivery/ack.json"))?;
    ensure!(
        ack["capture_id"] == capture,
        "ACK belongs to another capture"
    );
    let log = dir.join("capture.jsonl");
    ensure!(
        ack["source_offset"].as_u64() == Some(fs::metadata(&log)?.len()),
        "Journal is not fully acknowledged"
    );
    ensure!(
        fs::metadata(dir.join("delivery/outbox.jsonl"))?.len() == 0,
        "Pending outbox"
    );
    let mut file = fs::File::open(log)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(65536)))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let mut finalized = false;
    for line in tail
        .split(|byte| *byte == b'\n')
        .rev()
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
    {
        let event: Value = serde_json::from_slice(line)?;
        if event["event"] == "status" {
            finalized = matches!(
                event["data"]["state"].as_str(),
                Some("stopped" | "stopped_user" | "stopped_game_exited")
            );
            break;
        }
        ensure!(
            matches!(
                event["event"].as_str(),
                Some(
                    "decoder_warning_counts"
                        | "decoder_recovery_stats"
                        | "source_summary"
                        | "transport_summary"
                )
            ),
            "Capture has non-final data after its terminal status"
        );
    }
    ensure!(finalized, "Capture is not finalized");
    let manifest = dir.join("raw-ring/manifest.json");
    if manifest.exists() {
        let raw = read(&manifest)?;
        ensure!(
            raw["closed"] == true
                && raw["segments"].as_array().is_some_and(|segments| segments
                    .iter()
                    .all(|segment| segment["closed"] == true)),
            "Raw recording is open"
        );
    }
    Ok(())
}
fn active(root: &Path) -> Option<String> {
    read(&root.join("auto-capture/status.json"))
        .ok()
        .filter(|v| matches!(v["state"].as_str(), Some("starting" | "ready" | "stopping")))
        .and_then(|v| v["run_directory"].as_str().map(str::to_owned))
}
pub async fn sync(store: &mut Store, root: &Path) -> Result<()> {
    store.materialize_recovery_points().await?;
    let initialized: Option<i64> =
        sqlx::query_scalar("SELECT initialized FROM retention_meta WHERE singleton=1")
            .fetch_optional(store.maintenance_db())
            .await?;
    let current = active(root);
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("realtime-") {
            continue;
        }
        let path = match safe(root, &name) {
            Ok(path) => path,
            Err(_) => {
                let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()).to_string();
                store
                    .register_artifact(&id, "diagnostics", &name, None, true)
                    .await?;
                continue;
            }
        };
        if !path.is_dir() {
            continue;
        }
        let capture = read(&path.join("delivery.json"))
            .ok()
            .filter(|v| v["schema_version"] == 1 && v["log_file"] == "capture.jsonl")
            .and_then(|v| {
                v["capture_id"]
                    .as_str()
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .map(|id| id.to_string())
            });
        let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()).to_string();
        let historical = initialized.is_none() && current.as_deref() != Some(&name);
        store
            .register_artifact(
                &id,
                if capture.is_some() {
                    "capture"
                } else {
                    "diagnostics"
                },
                &name,
                capture.as_deref(),
                historical || capture.is_none(),
            )
            .await?;
        if historical && let Some(capture) = capture {
            let known: i64 = sqlx::query_scalar("SELECT count(*) FROM captures WHERE capture_id=?")
                .bind(&capture)
                .fetch_one(store.maintenance_db())
                .await?;
            if known > 0 {
                sqlx::query("INSERT OR IGNORE INTO automatic_backup_captures VALUES(?,NULL,'predates_retention')").bind(capture).execute(store.maintenance_db()).await?;
            }
        }
    }
    let auto_dir = safe(root, "automatic-backups")?;
    if auto_dir.is_dir() {
        for entry in fs::read_dir(auto_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".visionbackup") {
                continue;
            }
            let relative = format!("automatic-backups/{name}");
            let known: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM retention_artifacts WHERE relative_path=?",
            )
            .bind(&relative)
            .fetch_one(store.maintenance_db())
            .await?;
            if known == 0 {
                let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, relative.as_bytes()).to_string();
                store
                    .register_artifact(&id, "unknown_backup", &relative, None, true)
                    .await?;
            }
        }
    }
    sqlx::query("INSERT OR IGNORE INTO retention_meta VALUES(1,1)")
        .execute(store.maintenance_db())
        .await?;
    Ok(())
}
async fn preview_inner(store: &mut Store, root: &Path) -> Result<Value> {
    let current = active(root);
    let points = store.recovery_points().await?;
    let mut rows = vec![];
    let backups:Vec<String>=sqlx::query_scalar("SELECT backup_id FROM automatic_backups WHERE state='verified' ORDER BY created_ms DESC,backup_id DESC LIMIT 3").fetch_all(store.maintenance_db()).await?;
    for artifact in store.artifacts().await? {
        if artifact.state == "removed" {
            continue;
        }
        let mut reasons = store.artifact_protection(&artifact).await?;
        if artifact.kind == "automatic_backup" {
            reasons.retain(|s| s != "unregistered_diagnostics");
            if backups.contains(&artifact.id) {
                reasons.push("last_three_automatic_backups".into());
            }
        }
        if current.as_deref() == Some(&artifact.path) {
            reasons.push("active_capture".into());
        }
        let mut bytes = 0;
        let mut stamp = String::new();
        match safe(root, &artifact.path).and_then(|p| inventory(&p)) {
            Ok((size, fingerprint, manual)) => {
                bytes = size;
                stamp = fingerprint;
                if manual && artifact.kind != "automatic_backup" {
                    reasons.push("manual_backup_inside".into());
                }
            }
            Err(error) => {
                reasons.push(format!("missing_or_unsafe: {error}"));
            }
        }
        if let Some(id) = &artifact.capture
            && delivery_complete(root, &artifact.path, id).is_err()
        {
            reasons.push("delivery_or_recording_not_finalized".into());
        }
        if artifact.kind == "automatic_backup" {
            let expected: Option<String> = sqlx::query_scalar(
                "SELECT sha256 FROM automatic_backups WHERE backup_id=? AND state='verified'",
            )
            .bind(&artifact.id)
            .fetch_optional(store.maintenance_db())
            .await?;
            if expected.is_none_or(|hash| {
                safe(root, &artifact.path)
                    .and_then(|p| hash_file(&p))
                    .ok()
                    .as_deref()
                    != Some(&hash)
            }) {
                reasons.push("unverified_backup".into());
            }
        }
        rows.push(json!({"artifact_id":artifact.id,"kind":artifact.kind,"path":artifact.path,"capture_id":artifact.capture,"pinned":artifact.pinned,"bytes":bytes,"stamp":stamp,"protected_reasons":reasons,"eligible":reasons.is_empty()}));
    }
    let candidates: Vec<_> = rows
        .iter()
        .filter(|r| r["eligible"] == true)
        .cloned()
        .collect();
    let plan_id = digest(serde_json::to_string(&candidates)?.as_bytes());
    Ok(
        json!({"plan_id":plan_id,"points":points,"artifacts":rows,"eligible_count":candidates.len(),"eligible_bytes":candidates.iter().filter_map(|r|r["bytes"].as_u64()).sum::<u64>(),"policy":{"points_per_profile":3,"automatic_backups":3,"trigger":"completed_capture_with_changes","manual_backups_protected":true}}),
    )
}
pub async fn preview(writer: Writer, root: PathBuf) -> Result<Value> {
    let _control = CONTROL.lock().await;
    let mut store = writer.lock().await;
    sync(&mut store, &root).await?;
    preview_inner(&mut store, &root).await
}
pub async fn pin(writer: Writer, root: PathBuf, id: String, pinned: bool) -> Result<Value> {
    let _control = CONTROL.lock().await;
    let mut store = writer.lock().await;
    store.pin_artifact(&id, pinned).await?;
    preview_inner(&mut store, &root).await
}

async fn commit_retirement(
    store: &mut Store,
    id: &str,
    job: &str,
    capture: Option<&str>,
) -> Result<()> {
    let mut tx = store.maintenance_db().begin().await?;
    if let Some(capture) = capture {
        let refs:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM current_entities WHERE capture_id=?)+(SELECT count(*) FROM domain_state WHERE capture_id=?)+(SELECT count(*) FROM recovery_points WHERE capture_id=?)").bind(capture).bind(capture).bind(capture).fetch_one(&mut *tx).await?;
        ensure!(refs == 0, "Capture became protected");
        for statement in [
            "DELETE FROM snapshots WHERE capture_id=?",
            "DELETE FROM change_log WHERE capture_id=?",
            "DELETE FROM delivery_results WHERE capture_id=?",
            "DELETE FROM events WHERE capture_id=?",
        ] {
            sqlx::query(statement)
                .bind(capture)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("UPDATE retention_artifacts SET state='removed' WHERE artifact_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE automatic_backups SET state='removed' WHERE backup_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE retention_jobs SET state='committed' WHERE job_id=?")
        .bind(job)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
fn remove_quarantine(root: &Path, relative: &str) -> Result<()> {
    ensure!(
        relative.starts_with(".retention-trash/"),
        "Unexpected trash path"
    );
    let path = safe(root, relative)?;
    if !path.exists() {
        return Ok(());
    }
    inventory(&path)?;
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}
async fn repair_jobs(store: &mut Store, root: &Path) -> Result<()> {
    let rows = sqlx::query("SELECT * FROM retention_jobs WHERE state IN ('prepared','committed')")
        .fetch_all(store.maintenance_db())
        .await?;
    for row in rows {
        let id: String = row.get("job_id");
        let state: String = row.get("state");
        let manifest: Value = serde_json::from_str(row.get("manifest_json"))?;
        let relative = manifest["path"].as_str().context("Invalid job path")?;
        let trash = manifest["trash"].as_str().context("Invalid job trash")?;
        if state == "committed" {
            remove_quarantine(root, trash)?;
        } else {
            let source = safe(root, relative)?;
            let staged = safe(root, trash)?;
            if staged.exists() {
                ensure!(
                    !source.exists(),
                    "Cannot roll back cleanup: destination exists"
                );
                fs::rename(staged, source)?;
            }
        }
        sqlx::query("UPDATE retention_jobs SET state=? WHERE job_id=?")
            .bind(if state == "committed" {
                "finished"
            } else {
                "rolled_back"
            })
            .bind(id)
            .execute(store.maintenance_db())
            .await?;
    }
    Ok(())
}
async fn retire(store: &mut Store, root: &Path, row: &Value) -> Result<u64> {
    let path = row["path"].as_str().context("Missing path")?;
    let id = row["artifact_id"].as_str().context("Missing artifact ID")?;
    if let Some(capture) = row["capture_id"].as_str() {
        delivery_complete(root, path, capture)?;
        let directory = safe(root, path)?;
        let ack = read(&directory.join("delivery/ack.json"))?;
        ensure!(
            ack["source_sha256"].as_str()
                == Some(hash_file(&directory.join("capture.jsonl"))?).as_deref(),
            "Source checksum differs from final ACK"
        );
        let checkpoint: Option<(i64, i64)> =
            sqlx::query_as("SELECT last_sequence,closed FROM live_checkpoints WHERE capture_id=?")
                .bind(capture)
                .fetch_optional(store.maintenance_db())
                .await?;
        ensure!(
            checkpoint == Some((ack["acked_sequence"].as_i64().unwrap_or(-1), 1)),
            "Delivery checkpoint changed"
        );
    }
    let job = Uuid::new_v4().to_string();
    let trash = format!(".retention-trash/{job}");
    let source = safe(root, path)?;
    let destination = safe(root, &trash)?;
    fs::create_dir_all(destination.parent().unwrap())?;
    ensure!(!destination.exists(), "Trash collision");
    let (_, stamp, _) = inventory(&source)?;
    ensure!(row["stamp"] == stamp, "Artifact changed since preview");
    sqlx::query("INSERT INTO retention_jobs VALUES(?,'prepared',?)")
        .bind(&job)
        .bind(json!({"path":path,"trash":trash,"artifact_id":id}).to_string())
        .execute(store.maintenance_db())
        .await?;
    fs::rename(&source, &destination)?;
    if let Err(error) = commit_retirement(store, id, &job, row["capture_id"].as_str()).await {
        ensure!(
            !source.exists(),
            "Cannot restore failed cleanup: destination exists"
        );
        fs::rename(&destination, &source)?;
        sqlx::query("UPDATE retention_jobs SET state='rolled_back' WHERE job_id=?")
            .bind(&job)
            .execute(store.maintenance_db())
            .await?;
        return Err(error);
    }
    remove_quarantine(root, &trash)?;
    sqlx::query("UPDATE retention_jobs SET state='finished' WHERE job_id=?")
        .bind(job)
        .execute(store.maintenance_db())
        .await?;
    Ok(row["bytes"].as_u64().unwrap_or(0))
}
pub async fn apply(writer: Writer, root: PathBuf, plan: String) -> Result<Value> {
    let _control = CONTROL.lock().await;
    let mut store = writer.lock().await;
    sync(&mut store, &root).await?;
    repair_jobs(&mut store, &root).await?;
    let fresh = preview_inner(&mut store, &root).await?;
    ensure!(
        fresh["plan_id"] == plan,
        "Cleanup plan changed; refresh the preview"
    );
    let mut bytes = 0;
    let mut count = 0;
    for row in fresh["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["eligible"] == true)
    {
        bytes += retire(&mut store, &root, row).await?;
        count += 1;
    }
    Ok(
        json!({"removed":count,"freed_bytes":bytes,"preview":preview_inner(&mut store,&root).await?}),
    )
}
fn catalog_metadata(root: &Path) -> Value {
    let mut sources = serde_json::Map::new();
    for (key, name) in [
        ("items", "materials.ru.json"),
        ("avatars", "avatars.ru.json"),
        ("achievements", "achievements.ru.json"),
    ] {
        let value = fs::metadata(root.join(name))
            .ok()
            .filter(|m| m.len() <= 64 * 1024 * 1024)
            .and_then(|_| fs::read(root.join(name)).ok())
            .and_then(|b| {
                serde_json::from_slice::<Value>(b.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&b))
                    .ok()
            })
            .and_then(|v| v.get("revision").cloned())
            .unwrap_or(Value::Null);
        sources.insert(key.into(), value);
    }
    json!({"schema_version":1,"sources":sources,"reason":"completed_capture"})
}
async fn auto_backup(writer: Writer, root: &Path) -> Result<()> {
    let mut store = writer.lock().await;
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM live_leases WHERE expires_ms>?")
        .bind(now())
        .fetch_one(store.maintenance_db())
        .await?;
    if active > 0 {
        return Ok(());
    }
    for row in sqlx::query(
        "SELECT backup_id,relative_path,sha256 FROM automatic_backups WHERE state='verified'",
    )
    .fetch_all(store.maintenance_db())
    .await?
    {
        let path: String = row.get("relative_path");
        let expected: String = row.get("sha256");
        if safe(root, &path)
            .and_then(|path| hash_file(&path))
            .ok()
            .as_deref()
            != Some(&expected)
        {
            sqlx::query("UPDATE automatic_backups SET state='invalid' WHERE backup_id=?")
                .bind(row.get::<String, _>("backup_id"))
                .execute(store.maintenance_db())
                .await?;
        }
    }
    let artifacts = store.artifacts().await?;
    for artifact in artifacts
        .into_iter()
        .filter(|a| a.kind == "capture" && a.state == "present")
    {
        let Some(capture) = artifact.capture else {
            continue;
        };
        let processed: i64 =
            sqlx::query_scalar("SELECT count(*) FROM automatic_backup_captures WHERE capture_id=?")
                .bind(&capture)
                .fetch_one(store.maintenance_db())
                .await?;
        if processed > 0
            || store.capture_blocked(&capture).await?
            || delivery_complete(root, &artifact.path, &capture).is_err()
        {
            continue;
        }
        let terminal: Option<String> =
            sqlx::query_scalar("SELECT terminal_status FROM captures WHERE capture_id=?")
                .bind(&capture)
                .fetch_optional(store.maintenance_db())
                .await?;
        if !terminal.is_some_and(|s| {
            matches!(
                s.as_str(),
                "stopped" | "stopped_user" | "stopped_game_exited"
            )
        }) {
            continue;
        }
        let ack = read(&safe(root, &artifact.path)?.join("delivery/ack.json"))?;
        let closed: Option<(i64, i64)> =
            sqlx::query_as("SELECT last_sequence,closed FROM live_checkpoints WHERE capture_id=?")
                .bind(&capture)
                .fetch_optional(store.maintenance_db())
                .await?;
        if closed != Some((ack["acked_sequence"].as_i64().unwrap_or(-1), 1)) {
            continue;
        }
        ensure!(
            ack["source_sha256"].as_str()
                == Some(hash_file(
                    &safe(root, &artifact.path)?.join("capture.jsonl")
                )?)
                .as_deref(),
            "Finalized source checksum differs from ACK"
        );
        if store.profiles().await?.as_array().unwrap().is_empty() {
            continue;
        }
        let coverage:(i64,i64)=sqlx::query_as("SELECT count(*),coalesce(sum(CASE WHEN result='materialized' THEN 0 ELSE 1 END),0) FROM retention_epochs WHERE capture_id=?").bind(&capture).fetch_one(store.maintenance_db()).await?;
        if coverage.0 == 0 || coverage.1 > 0 {
            sqlx::query("INSERT OR IGNORE INTO automatic_backup_captures VALUES(?,NULL,'incomplete_capture')").bind(&capture).execute(store.maintenance_db()).await?;
            continue;
        }
        let hash = store.meaningful_hash().await?;
        let latest:Option<(String,String)>=sqlx::query_as("SELECT backup_id,meaningful_hash FROM automatic_backups WHERE state='verified' ORDER BY created_ms DESC,backup_id DESC LIMIT 1").fetch_optional(store.maintenance_db()).await?;
        if latest
            .as_ref()
            .is_some_and(|(_, previous)| previous == &hash)
        {
            sqlx::query("INSERT INTO automatic_backup_captures VALUES(?,?,'unchanged')")
                .bind(&capture)
                .bind(latest.unwrap().0)
                .execute(store.maintenance_db())
                .await?;
            continue;
        }
        let saved_settings: Option<String> =
            sqlx::query_scalar("SELECT settings_json FROM portable_preferences WHERE singleton=1")
                .fetch_optional(store.maintenance_db())
                .await?;
        let settings = saved_settings
            .map(|value| serde_json::from_str::<backup::Settings>(&value))
            .transpose()?
            .unwrap_or_default();
        let snapshot = store.backup_snapshot().await?;
        drop(store);
        let id = Uuid::new_v4().to_string();
        let relative = format!("automatic-backups/{id}.visionbackup");
        let path = safe(root, &relative)?;
        fs::create_dir_all(path.parent().unwrap())?;
        backup::package(snapshot, &path, settings, catalog_metadata(root)).await?;
        let sha = hash_file(&path)?;
        store = writer.lock().await;
        if store.capture_blocked(&capture).await? {
            continue;
        }
        let mut tx = store.maintenance_db().begin().await?;
        sqlx::query("INSERT INTO automatic_backups VALUES(?,?,?,?,?,'verified')")
            .bind(&id)
            .bind(&relative)
            .bind(sha)
            .bind(hash)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO automatic_backup_captures VALUES(?,?,'created')")
            .bind(&capture)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO retention_artifacts(artifact_id,kind,relative_path,pinned) VALUES(?,'automatic_backup',?,0)").bind(id).bind(relative).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    // Rotate only managed automatic copies, after a new copy passed full validation.
    let plan = preview_inner(&mut store, root).await?;
    for row in plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "automatic_backup" && r["eligible"] == true)
    {
        retire(&mut store, root, row).await?;
    }
    Ok(())
}
pub async fn cycle(writer: Writer, root: PathBuf) -> Result<()> {
    let _control = CONTROL.lock().await;
    {
        let mut store = writer.lock().await;
        repair_jobs(&mut store, &root).await?;
        sync(&mut store, &root).await?;
    }
    auto_backup(writer, &root).await
}
pub async fn run(writer: Writer, root: PathBuf, mut shutdown: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){break;}},_=tick.tick()=>{}}
        let writer = writer.clone();
        let root = root.clone();
        match tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(cycle(writer, root))
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::app_log!("Maintenance deferred: {error}"),
            Err(error) => crate::app_log!("Maintenance worker failed: {error}"),
        }
    }
}
