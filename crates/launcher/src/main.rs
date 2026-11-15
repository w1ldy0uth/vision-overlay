#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[cfg(windows)]
mod windows;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    schema_version: u32,
    version: String,
    #[serde(default, rename = "build_commit")]
    _build_commit: Option<String>,
}
pub struct Layout {
    root: PathBuf,
    data: PathBuf,
    components: PathBuf,
}
const DISPATCH_SHA256: &str = "e0e1fcbfb6aa5d727367a60574b7688a4da14abe12c5a3bdad3a7fc87c694d18";
impl Layout {
    fn open(root: &Path) -> Result<Self> {
        let package: Package = serde_json::from_slice(&fs::read(root.join("portable.json"))?)?;
        ensure!(
            package.schema_version == 1 && !package.version.is_empty(),
            "Unsupported portable package"
        );
        let layout = Self {
            root: root.to_path_buf(),
            data: root.join("data"),
            components: root.join("components"),
        };
        for path in [
            "vision-server.exe",
            "capture-probe.exe",
            "python/python.exe",
            "scripts/Configure-AutoCapture.ps1",
            "scripts/Runtime.Paths.ps1",
            "scripts/sync_wish_history.py",
            "reference/gi.json",
        ] {
            ensure!(
                layout.components.join(path).is_file(),
                "Package component missing: {path}"
            );
        }
        Ok(layout)
    }
    fn initialize(&self) -> Result<()> {
        let keys = self.bundled_keys()?;
        fs::create_dir_all(self.data.join("logs"))?;
        // Deliberately copy only packaged reference data. Never read another installation.
        for name in [
            "materials.ru.json",
            "avatars.ru.json",
            "achievements.ru.json",
        ] {
            let target = self.data.join(name);
            if !target.exists() {
                fs::copy(self.components.join("reference").join(name), target)?;
            }
        }
        let key_file = self.data.join("gi.json");
        if !key_file.exists() {
            fs::write(key_file, keys)?;
        }
        Ok(())
    }
    fn bundled_keys(&self) -> Result<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let path = self.components.join("reference/gi.json");
        ensure!(
            fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
            "Bundled dispatch file too large"
        );
        let bytes = fs::read(path)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == DISPATCH_SHA256,
            "Bundled dispatch file is damaged; extract the complete application archive again"
        );
        validate_keys(&bytes)?;
        Ok(bytes)
    }
    fn port(&self) -> Result<u16> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Settings {
            port: u16,
        }
        let path = self.data.join("launcher.json");
        let port = if path.exists() {
            serde_json::from_slice::<Settings>(&fs::read(path)?)?.port
        } else {
            18789
        };
        ensure!(port >= 1024, "Invalid launcher port");
        Ok(port)
    }
}
fn validate_keys(bytes: &[u8]) -> Result<()> {
    use base64::Engine;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "Key file is too large");
    let keys: std::collections::HashMap<u16, String> =
        serde_json::from_slice(bytes).context("Expected compatible dispatch-key JSON")?;
    ensure!(!keys.is_empty(), "Key file is empty");
    for value in keys.values() {
        ensure!(
            !base64::prelude::BASE64_STANDARD.decode(value)?.is_empty(),
            "Empty key"
        );
    }
    Ok(())
}
fn run() -> Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("Vision Overlay {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let layout = Layout::open(exe.parent().context("Application directory missing")?)?;
    if std::env::args().any(|a| a == "--check") {
        println!("Portable components OK; port {}", layout.port()?);
        return Ok(());
    }
    layout.initialize().with_context(|| format!("Не удалось подготовить папку данных {}. Распакуйте переносимую версию в доступную для записи папку пользователя, например Apps/VisionOverlay.", layout.data.display()))?;
    #[cfg(windows)]
    {
        windows::run(layout)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Windows x64 is required")
    }
}
fn main() {
    if let Err(e) = run() {
        #[cfg(windows)]
        windows::error(&format!("{e:#}"));
        #[cfg(not(windows))]
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_initialization_is_isolated_and_preserves_existing_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Portable folder");
        fs::create_dir_all(root.join("components/reference")).unwrap();
        fs::write(
            root.join("portable.json"),
            r#"{"schema_version":1,"version":"test"}"#,
        )
        .unwrap();
        for path in [
            "vision-server.exe",
            "capture-probe.exe",
            "python/python.exe",
            "scripts/Configure-AutoCapture.ps1",
            "scripts/Runtime.Paths.ps1",
            "scripts/sync_wish_history.py",
        ] {
            let path = root.join("components").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "fixture").unwrap();
        }
        for name in [
            "materials.ru.json",
            "avatars.ru.json",
            "achievements.ru.json",
        ] {
            fs::write(root.join("components/reference").join(name), "reference").unwrap();
        }
        let bundled = include_bytes!("../../../packaging/dispatch/gi.json");
        fs::write(root.join("components/reference/gi.json"), bundled).unwrap();
        let layout = Layout::open(&root).unwrap();
        layout.initialize().unwrap();
        fs::write(layout.data.join("materials.ru.json"), "existing").unwrap();
        layout.initialize().unwrap();
        assert_eq!(
            fs::read_to_string(layout.data.join("materials.ru.json")).unwrap(),
            "existing"
        );
        assert_eq!(fs::read(layout.data.join("gi.json")).unwrap(), bundled);
        fs::write(layout.data.join("gi.json"), "custom-existing").unwrap();
        layout.initialize().unwrap();
        assert_eq!(
            fs::read_to_string(layout.data.join("gi.json")).unwrap(),
            "custom-existing"
        );
        fs::write(root.join("components/reference/gi.json"), b"{}").unwrap();
        assert!(layout.initialize().is_err());
        assert!(!layout.data.join("vision-overlay.sqlite3").exists());
        assert_eq!(layout.port().unwrap(), 18789);
        fs::write(layout.data.join("launcher.json"), r#"{"port":0}"#).unwrap();
        assert!(layout.port().is_err());
    }
    #[test]
    fn malformed_or_empty_key_files_are_rejected_without_exporting_values() {
        assert!(validate_keys(br#"{"1":"AQID"}"#).is_ok());
        for b in [
            b"{}".as_slice(),
            br#"{"1":""}"#,
            br#"{"1":"%%%"}"#,
            b"not-json",
        ] {
            assert!(validate_keys(b).is_err());
        }
    }
}
