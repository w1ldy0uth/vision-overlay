use super::redact::Cleaner;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
static ROOT: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());
pub fn init(root: &Path) -> anyhow::Result<()> {
    let dir = root.join("logs");
    fs::create_dir_all(&dir)?;
    let _ = ROOT.set(root.to_path_buf());
    record("Application server started");
    Ok(())
}
pub fn record(message: &str) {
    let Some(root) = ROOT.get() else {
        return;
    };
    let Ok(_guard) = LOCK.lock() else {
        return;
    };
    let path = root.join("logs/server.jsonl");
    if fs::metadata(&path).is_ok_and(|m| m.len() >= 4 * 1024 * 1024) {
        let old = root.join("logs/server.previous.jsonl");
        if old.exists() && fs::remove_file(&old).is_err() {
            return;
        }
        if fs::rename(&path, &old).is_err() {
            return;
        }
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let msg = Cleaner::new(root).text(message);
        let v = serde_json::json!({"observed_unix_ms":app_store::ingest::now_ms().to_string(),"component":"vision-server","message":msg});
        let _ = writeln!(file, "{v}");
    }
}
