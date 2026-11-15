use super::redact::Cleaner;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};
const TOTAL: u64 = 512 * 1024 * 1024;
pub struct Built {
    pub temp: tempfile::TempDir,
    pub path: PathBuf,
    pub size: u64,
    pub capture: String,
    pub manifest: Value,
}
fn reparse(m: &fs::Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn safe(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        !reparse(&fs::symlink_metadata(root)?),
        "Diagnostic root is a link"
    );
    ensure!(
        !relative.is_empty()
            && Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "Unsafe diagnostic path"
    );
    let base = root.canonicalize()?;
    let mut path = base.clone();
    for c in Path::new(relative).components() {
        path.push(c);
        if let Ok(m) = fs::symlink_metadata(&path) {
            ensure!(!reparse(&m), "Diagnostic link rejected");
        }
    }
    ensure!(path.starts_with(base), "Diagnostic path escaped root");
    Ok(path)
}
fn read_json(path: &Path) -> Result<Value> {
    ensure!(
        fs::metadata(path)?.len() <= 131072,
        "Diagnostic metadata too large"
    );
    let b = fs::read(path)?;
    Ok(serde_json::from_slice(
        b.strip_prefix(&[239, 187, 191]).unwrap_or(&b),
    )?)
}
struct Package {
    zip: ZipWriter<File>,
    entries: Vec<Value>,
    bytes: u64,
}
impl Package {
    fn add(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.bytes + bytes.len() as u64 <= TOTAL,
            "Diagnostic size limit"
        );
        self.zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )?;
        self.zip.write_all(bytes)?;
        self.bytes += bytes.len() as u64;
        self.entries.push(
            json!({"path":name,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))}),
        );
        Ok(())
    }
    fn json(&mut self, name: &str, mut value: Value, clean: &Cleaner) -> Result<()> {
        clean.value(&mut value);
        self.add(name, &serde_json::to_vec_pretty(&value)?)
    }
    fn log(
        &mut self,
        root: &Path,
        relative: &str,
        name: &str,
        limit: u64,
        clean: &Cleaner,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let path = safe(root, relative)?;
        if !path.is_file() {
            return Ok(());
        }
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        let skip = length.saturating_sub(limit);
        file.seek(SeekFrom::Start(skip))?;
        let mut bytes = Vec::new();
        file.take(length - skip).read_to_end(&mut bytes)?;
        if skip > 0 {
            warnings.push(format!("{name}: only the last {limit} bytes retained"));
            if let Some(i) = bytes.iter().position(|b| *b == b'\n') {
                bytes.drain(..=i);
            } else {
                bytes.clear();
            }
        }
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        if complete < bytes.len() {
            warnings.push(format!("{name}: unfinished last line excluded"));
        }
        bytes.truncate(complete);
        let mut text = String::new();
        for line in String::from_utf8_lossy(&bytes).lines() {
            if let Ok(mut v) = serde_json::from_str::<Value>(line) {
                clean.value(&mut v);
                text.push_str(&serde_json::to_string(&v)?);
            } else {
                text.push_str(&clean.text(line));
            }
            text.push('\n');
        }
        self.add(name, text.as_bytes())
    }
    fn raw(
        &mut self,
        name: &str,
        mut file: File,
        length: u64,
        closed: bool,
        timeline: &mut u64,
    ) -> Result<Value> {
        ensure!(length <= 64 * 1024 * 1024, "Raw segment too large");
        let mut input = (&mut file).take(length);
        let mut magic = [0; 8];
        input.read_exact(&mut magic)?;
        ensure!(&magic == b"VORAW001", "Unsupported raw recording");
        self.zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut put = |part: &[u8]| -> Result<()> {
            ensure!(
                self.bytes + bytes + part.len() as u64 <= TOTAL,
                "Diagnostic size limit"
            );
            self.zip.write_all(part)?;
            hash.update(part);
            bytes += part.len() as u64;
            Ok(())
        };
        put(&magic)?;
        let mut packets = 0u64;
        let mut first = None;
        let mut footer = false;
        let mut tail = 0;
        loop {
            let remaining = input.limit();
            if remaining < 12 {
                tail = remaining;
                break;
            }
            let mut header = [0u8; 12];
            input.read_exact(&mut header)?;
            let elapsed = u64::from_le_bytes(header[..8].try_into()?);
            let size = u32::from_le_bytes(header[8..].try_into()?);
            ensure!(
                elapsed >= *timeline && size <= 131072,
                "Invalid raw packet record"
            );
            if size == 0 {
                ensure!(input.limit() == 0, "Data after raw footer");
                *timeline = elapsed;
                footer = true;
                break;
            }
            if input.limit() < u64::from(size) {
                tail = 12 + input.limit();
                break;
            }
            let mut packet = vec![0; size as usize];
            input.read_exact(&mut packet)?;
            put(&header)?;
            put(&packet)?;
            first.get_or_insert(elapsed);
            *timeline = elapsed;
            packets += 1;
        }
        ensure!(!closed || footer, "Closed raw segment is incomplete");
        put(&timeline.to_le_bytes())?;
        put(&0u32.to_le_bytes())?;
        self.bytes += bytes;
        self.entries
            .push(json!({"path":name,"bytes":bytes,"sha256":format!("{:x}",hash.finalize())}));
        Ok(
            json!({"file":Path::new(name).file_name().unwrap().to_string_lossy(),"first_ms":first,"last_ms":first.map(|_|*timeline),"packets":packets,"bytes":bytes,"closed":true,"diagnostic_cut":{"source_bytes":length,"source_closed":closed,"synthetic_footer":!footer,"incomplete_tail_bytes":tail}}),
        )
    }
}
fn find_run(root: &Path, capture: &str) -> Result<Option<String>> {
    let mut matches = Vec::new();
    for e in fs::read_dir(root)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("realtime-")
            || name.len() > 128
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            continue;
        }
        let path = safe(root, &format!("{name}/delivery.json"))?;
        if path.is_file()
            && read_json(&path)
                .ok()
                .is_some_and(|v| v["capture_id"] == capture)
        {
            matches.push(name);
        }
    }
    ensure!(matches.len() <= 1, "Ambiguous capture directory");
    Ok(matches.pop())
}
type FrozenRaw = (Value, Vec<(String, File, u64, bool)>);
fn raw_files(root: &Path, run: &str) -> Result<Option<FrozenRaw>> {
    let path = safe(root, &format!("{run}/raw-ring/manifest.json"))?;
    if !path.is_file() {
        return Ok(None);
    }
    let manifest = read_json(&path)?;
    ensure!(
        manifest["schema_version"] == 1 && manifest["format"] == "VORAW001-ring",
        "Unsupported raw ring manifest"
    );
    let rows = manifest["segments"]
        .as_array()
        .context("Missing raw segments")?;
    ensure!(rows.len() <= 128, "Too many raw segments");
    let mut files = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for row in rows {
        let name = row["file"].as_str().context("Missing raw name")?;
        ensure!(
            name.len() == 22
                && name.starts_with("segment-")
                && name.ends_with(".voraw")
                && name[8..16].bytes().all(|b| b.is_ascii_digit())
                && names.insert(name),
            "Invalid raw segment name"
        );
        let path = safe(root, &format!("{run}/raw-ring/{name}"))?;
        let file = File::open(path).context("Raw segment rotated during export; retry")?;
        let len = file.metadata()?.len();
        total += len;
        ensure!(
            total <= 320 * 1024 * 1024,
            "Raw budget exceeds diagnostic limit"
        );
        files.push((name.into(), file, len, row["closed"] == true));
    }
    Ok(Some((manifest, files)))
}
fn control_logs(
    p: &mut Package,
    root: &Path,
    run: Option<&str>,
    clean: &Cleaner,
    warnings: &mut Vec<String>,
) -> Result<()> {
    p.log(
        root,
        "auto-capture/watcher.log",
        "logs/watcher.log",
        1024 * 1024,
        clean,
        warnings,
    )?;
    for relative in ["logs/server.jsonl", "logs/server.previous.jsonl"] {
        p.log(root, relative, relative, 4 * 1024 * 1024, clean, warnings)?;
    }
    let Some(run) = run else { return Ok(()) };
    let sessions = safe(root, "auto-capture/runs")?;
    if !sessions.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(sessions)?.take(2048) {
        let e = e?;
        let id = e.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        let path = safe(root, &format!("auto-capture/runs/{id}/session.json"))?;
        if !path.is_file() {
            continue;
        }
        if read_json(&path)
            .ok()
            .is_none_or(|v| v["run_directory"] != run)
        {
            continue;
        }
        for log in fs::read_dir(safe(root, "auto-capture")?)? {
            let log = log?;
            let name = log.file_name().to_string_lossy().into_owned();
            if name.contains(&format!("-{}.", &id[..8]))
                && (name.ends_with(".stdout.log") || name.ends_with(".stderr.log"))
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            {
                p.log(
                    root,
                    &format!("auto-capture/{name}"),
                    &format!("logs/{name}"),
                    4 * 1024 * 1024,
                    clean,
                    warnings,
                )?;
            }
        }
    }
    Ok(())
}
pub fn package(
    root: &Path,
    capture: &str,
    packets: bool,
    description: &str,
    mut facts: Value,
    state: Value,
    catalog: Option<Value>,
) -> Result<Built> {
    ensure!(uuid::Uuid::parse_str(capture).is_ok(), "Invalid capture ID");
    let clean = Cleaner::new(&root.canonicalize()?);
    let run = find_run(root, capture)?;
    let mut warnings = Vec::new();
    let frozen = if packets {
        run.as_deref()
            .map(|r| raw_files(root, r))
            .transpose()?
            .flatten()
    } else {
        None
    };
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("diagnostics.zip");
    let mut p = Package {
        zip: ZipWriter::new(File::create(&path)?),
        entries: Vec::new(),
        bytes: 0,
    };
    if facts["snapshots_truncated"] == true {
        warnings.push(
            "Database snapshot export is bounded to the newest 192 snapshots / 64 MiB".into(),
        );
    }
    if facts["events_truncated"] == true {
        warnings.push("Database event export is bounded to the newest 2000 events / 32 MiB".into());
    }
    p.json("database/snapshots.json", facts["snapshots"].take(), &clean)?;
    p.json("database/events.json", facts["events"].take(), &clean)?;
    p.json("database/capture.json", facts, &clean)?;
    p.json("application/state.json", state, &clean)?;
    p.json("application/environment.json",json!({"version":env!("CARGO_PKG_VERSION"),"build_commit":env!("VISION_BUILD_COMMIT"),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"description":description}),&clean)?;
    if let Some(c) = catalog {
        p.json("application/catalog.json", c, &clean)?;
    }
    if let Some(run) = &run {
        p.log(
            root,
            &format!("{run}/capture.jsonl"),
            "logs/capture.jsonl",
            32 * 1024 * 1024,
            &clean,
            &mut warnings,
        )?;
        for name in ["panel.stdout.log", "panel.stderr.log"] {
            p.log(
                root,
                &format!("{run}/{name}"),
                &format!("logs/{name}"),
                1024 * 1024,
                &clean,
                &mut warnings,
            )?;
        }
    } else {
        warnings.push("Capture directory was not retained; database facts are included".into());
    }
    control_logs(&mut p, root, run.as_deref(), &clean, &mut warnings)?;
    if let Ok(path) = safe(root, "auto-capture/status.json")
        && path.is_file()
        && let Ok(status) = read_json(&path)
    {
        p.json("application/capture-status.json", status, &clean)?;
    }
    let mut raw_info = json!({"requested":packets,"included":false});
    if let Some((mut manifest, files)) = frozen {
        let original_closed = manifest["closed"].clone();
        let mut timeline = 0;
        let mut rows = Vec::new();
        for (name, file, len, closed) in files {
            rows.push(p.raw(
                &format!("packets/raw-ring/{name}"),
                file,
                len,
                closed,
                &mut timeline,
            )?);
        }
        let replay = manifest["evicted_segments"] == 0
            && rows
                .first()
                .is_some_and(|r| r["file"] == "segment-00000000.voraw");
        manifest["segments"] = json!(rows);
        manifest["closed"] = json!(true);
        manifest["standalone_replay"] = json!(replay);
        manifest["replay_note"] = json!(if replay {
            "diagnostic_frozen_prefix_synthetic_end"
        } else {
            "evicted_prefix_key_and_kcp_context_may_be_missing"
        });
        manifest["diagnostic_export"] = json!({"source_closed":original_closed,"original_capture_unchanged":true,"cut_elapsed_ms":timeline});
        if !replay {
            warnings.push(
                "Raw prefix has been evicted; independent decoding may be unavailable".into(),
            );
        }
        raw_info = json!({"requested":true,"included":true,"segments":manifest["segments"].as_array().unwrap().len(),"evicted_segments":manifest["evicted_segments"],"standalone_replay":replay,"cut_elapsed_ms":timeline,"source_closed":original_closed});
        p.json("packets/raw-ring/manifest.json", manifest, &clean)?;
    } else if packets {
        warnings.push("No retained raw packet recording was found".into());
    }
    let readme = "Vision Overlay diagnostic archive\n\nContains facts for the selected capture, retained snapshots, recent sanitized application logs, and optional game packet recordings. No SQLite database, key files, collector tokens, browser cache, or wish auth URLs are copied. UID, inventory and character data remain. Raw packets are encrypted game traffic and are not anonymized; review before sharing. Nothing is uploaded automatically.\n\nDatabase facts are read in one transaction. Each file/raw segment is frozen independently; an active capture is not stopped. Incomplete JSONL lines and incomplete final packet records are excluded. Raw output uses synthetic closing footers, which do not mean the game session ended. Evicted prefixes cannot be recovered. External decoder keys are not bundled.\n\nmanifest.json lists SHA256 and sizes of every other file, limits and omissions. This archive is for analysis, not backup restore.\n";
    p.add("README.txt", readme.as_bytes())?;
    let manifest = json!({"schema_version":1,"kind":"vision_overlay_diagnostics","capture_id":capture,"created_unix_ms":app_store::ingest::now_ms().to_string(),"consistency":"database_transaction_and_independent_file_cuts","raw":raw_info,"warnings":warnings,"limits":{"uncompressed_bytes":TOTAL,"capture_log_tail_bytes":32*1024*1024,"database_snapshots":192,"database_events":2000},"privacy":{"account_data_included":true,"raw_packets_anonymized":false,"key_files_included":false,"automatic_upload":false},"files":p.entries});
    p.zip.start_file(
        "manifest.json",
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )?;
    serde_json::to_writer_pretty(&mut p.zip, &manifest)?;
    p.zip.finish()?.sync_all()?;
    let size = fs::metadata(&path)?.len();
    Ok(Built {
        temp,
        path,
        size,
        capture: capture.into(),
        manifest,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_is_allowlisted_and_raw_cut_contains_complete_records() {
        let root = tempfile::tempdir().unwrap();
        let capture = uuid::Uuid::new_v4().to_string();
        let run = "realtime-20261115-test";
        fs::create_dir_all(root.path().join(run).join("raw-ring")).unwrap();
        fs::write(
            root.path().join(run).join("delivery.json"),
            json!({"capture_id":capture}).to_string(),
        )
        .unwrap();
        fs::write(root.path().join("gi.json"), "DO_NOT_EXPORT").unwrap();
        fs::write(
            root.path().join(run).join("capture.jsonl"),
            "{\"event\":\"test\",\"data\":{\"authkey\":\"private-key\"}}\n{partial",
        )
        .unwrap();
        let ring = root.path().join(run).join("raw-ring");
        let mut bytes = b"VORAW001".to_vec();
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(3u32.to_le_bytes());
        bytes.extend([1, 2, 3]);
        bytes.extend([9, 9]);
        fs::write(ring.join("segment-00000000.voraw"), &bytes).unwrap();
        fs::write(ring.join("manifest.json"),json!({"schema_version":1,"format":"VORAW001-ring","closed":false,"evicted_segments":0,"segments":[{"file":"segment-00000000.voraw","closed":false}]}).to_string()).unwrap();
        let built = package(
            root.path(),
            &capture,
            true,
            "test",
            json!({"snapshots":[],"events":[]}),
            json!({}),
            None,
        )
        .unwrap();
        assert_eq!(
            fs::read(ring.join("segment-00000000.voraw")).unwrap(),
            bytes
        );
        let mut zip = zip::ZipArchive::new(File::open(&built.path).unwrap()).unwrap();
        assert!(zip.by_name("gi.json").is_err());
        let mut log = String::new();
        zip.by_name("logs/capture.jsonl")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert!(!log.contains("private-key"));
        assert!(!log.contains("partial"));
        let mut raw = Vec::new();
        zip.by_name("packets/raw-ring/segment-00000000.voraw")
            .unwrap()
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(raw.len(), 8 + 12 + 3 + 12);
        for row in built.manifest["files"].as_array().unwrap() {
            let mut bytes = Vec::new();
            zip.by_name(row["path"].as_str().unwrap())
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(row["sha256"], format!("{:x}", Sha256::digest(bytes)));
        }
    }
    #[test]
    fn path_traversal_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(safe(root.path(), "../gi.json").is_err());
        assert!(safe(root.path(), "/absolute").is_err());
    }
}
