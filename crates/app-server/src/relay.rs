//! Ordinary-user delivery worker. The elevated probe only writes its existing local log.
use anyhow::{Context, Result, bail, ensure};
use app_store::{
    core::Action,
    ingest::{Envelope, ProfileKey, Source},
    legacy::Projection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::watch;
use uuid::Uuid;

pub const OUTBOX_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema_version: u32,
    pub capture_id: String,
    pub log_file: String,
}
#[derive(Serialize, Deserialize, Default)]
struct Checkpoint {
    capture_id: String,
    acked_sequence: u64,
    source_offset: u64,
    source_sha256: String,
    event_sha256: String,
}
#[derive(Serialize, Deserialize, Clone)]
struct Pending {
    source_offset: u64,
    source_sha256: String,
    event_sha256: String,
    events: Vec<Value>,
}
#[derive(Clone)]
pub struct Normalizer {
    capture_id: String,
    sequence: u64,
    legacy_sequence: u64,
    loss_count: u64,
    epoch: Option<String>,
    profile: Option<ProfileKey>,
    projection: Projection,
    source: Source,
    pub closed: bool,
}
impl Normalizer {
    pub fn new(id: &str) -> Result<Self> {
        Uuid::parse_str(id)?;
        Ok(Self {
            capture_id: id.into(),
            sequence: 0,
            legacy_sequence: 0,
            loss_count: 0,
            epoch: None,
            profile: None,
            projection: Projection::default(),
            source: Source {
                producer: "vision-relay".into(),
                producer_version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: "7.2".into(),
                adapter_revision: "legacy-live-v1".into(),
                mode: "live".into(),
            },
            closed: false,
        })
    }
    fn event(&mut self, raw: &Value, kind: &str, payload: Value) -> Result<Value> {
        self.sequence += 1;
        let observed_at = raw["observed_unix_ms"]
            .as_str()
            .and_then(|s| s.parse::<i128>().ok())
            .map(|n| {
                OffsetDateTime::from_unix_timestamp_nanos(n * 1_000_000)?
                    .format(&Rfc3339)
                    .map_err(anyhow::Error::from)
            })
            .transpose()?;
        let e = Envelope {
            contract_version: 1,
            capture_id: self.capture_id.clone(),
            source_sequence: self.sequence.to_string(),
            epoch: self.epoch.clone(),
            profile: self.profile.clone(),
            source: self.source.clone(),
            observed_at,
            source_elapsed_ms: raw["source_elapsed_ms"].as_u64().map(|n| n.to_string()),
            kind: kind.into(),
            payload,
        };
        Ok(serde_json::to_value(e)?)
    }
    pub fn feed(&mut self, raw: &Value, regions: &BTreeMap<String, String>) -> Result<Vec<Value>> {
        ensure!(raw["schema_version"] == 1, "Unsupported probe journal");
        let sequence = raw["sequence"].as_u64().context("Missing probe sequence")?;
        ensure!(sequence == self.legacy_sequence + 1, "Gap in probe journal");
        let kind = raw["event"].as_str().context("Missing probe event")?;
        let data = &raw["data"];
        // Preserve explicit mappings (including legacy event hashes), infer only known
        // global prefixes from the UID verified by the probe's login packet.
        let identity = if kind == "login_identity" {
            let uid = data["uid"].as_str().context("Invalid login UID")?;
            let (region, region_source) = regions
                .get(uid)
                .map(|region| (region.as_str(), "user_import"))
                .or_else(|| {
                    app_store::legacy::global_region_from_uid(uid)
                        .map(|region| (region, "uid_prefix"))
                })
                .with_context(|| format!("Identity pending: configure region for UID {uid}"))?;
            let key = ProfileKey {
                game_id: "genshin".into(),
                account_namespace: "hk4e_global".into(),
                uid: uid.into(),
                server_region: region.into(),
            };
            app_store::legacy::Identity {
                uid: key.uid.clone(),
                server_region: key.server_region.clone(),
            }
            .validate()?;
            Some((key, region_source))
        } else {
            None
        };
        if self.closed {
            self.legacy_sequence = sequence;
            return Ok(vec![]);
        }
        if sequence == 1 {
            ensure!(
                kind == "source_config" && data["mode"] == "live",
                "Only a fresh live log can be relayed"
            );
            self.legacy_sequence = sequence;
            let mut configuration = json!({"raw_recording":data["recording_mode"],"decoder_revision":data["decoder_revision"]});
            if let Some(value) = data.get("wish_tracking") {
                configuration["wish_tracking"] = value.clone();
            }
            return Ok(vec![self.event(
                raw,
                "capture.opened",
                json!({"configuration":configuration}),
            )?]);
        }
        let epoch = raw["session"].as_str().context("Missing probe session")?;
        let epoch_number = epoch.parse::<u64>()?;
        ensure!(epoch_number.to_string() == epoch, "Invalid probe epoch");
        if kind.ends_with("_snapshot")
            || kind.ends_with("_changed")
            || kind == "character_added"
            || kind == "wish_completed"
        {
            ensure!(
                identity.is_some()
                    || (self.profile.is_some() && self.epoch.as_deref() == Some(epoch)),
                "Identity pending: probe must emit login_identity before game data"
            );
        }
        let mut out = Vec::new();
        if epoch_number > 0 && self.epoch.as_deref() != Some(epoch) {
            if let Some(old) = &self.epoch {
                ensure!(
                    epoch_number > old.parse::<u64>()?,
                    "Probe epoch went backwards"
                );
            }
            self.epoch = Some(epoch.into());
            self.profile = None;
            self.projection = Projection::default();
            out.push(self.event(raw, "session.opened", json!({"reason":"probe_handshake"}))?);
        }
        if let Some((key, region_source)) = identity {
            ensure!(
                self.profile.as_ref().is_none_or(|old| old == &key),
                "Login identity changed within epoch"
            );
            if self.profile.is_none() {
                self.profile = Some(key);
                out.push(self.event(raw,"identity.confirmed",json!({"evidence":{"uid_source":"login_packet","region_source":region_source,"login_verified":true}}))?);
            }
        }
        if kind == "wish_completed" {
            app_store::wish_live::validate(data)?;
            out.push(self.event(raw, "wish.observed", data.clone())?);
            self.legacy_sequence = sequence;
            return Ok(out);
        }
        if kind == "wish_tracking_rejected" {
            out.push(self.event(
                raw,
                "diagnostic.notice",
                json!({"code":"wish_tracking_rejected","details":{}}),
            )?);
            self.legacy_sequence = sequence;
            return Ok(out);
        }
        let is_change = kind.ends_with("_changed") || kind == "character_added";
        let before = if is_change {
            Some(self.projection.clone())
        } else {
            None
        };
        let mut actions = self.projection.apply(kind, data)?;
        if matches!(kind, "source_stats" | "source_summary") {
            let loss = [
                &data["dropped_queue_full"],
                &data["realtime_backend"]["callback_queue_drops"],
                &data["realtime_backend"]["missed_read"],
                &data["realtime_backend"]["missed_write"],
                &data["realtime_backend"]["invalid_descriptors"],
            ]
            .into_iter()
            .try_fold(0u64, |n, v| n.checked_add(v.as_u64().unwrap_or(0)))
            .context("Capture loss counter overflow")?;
            if loss > self.loss_count {
                self.projection.apply("status", &json!({"state":"stale"}))?;
                actions.push(Action::Invalidate {
                    reason: "capture_loss".into(),
                    error: false,
                });
            }
            self.loss_count = self.loss_count.max(loss);
        }
        if actions
            .iter()
            .any(|a| matches!(a, Action::Snapshot { .. } | Action::Set { .. }))
        {
            ensure!(
                self.profile.is_some(),
                "Identity pending: probe must emit login_identity before game data"
            );
        }
        let mut collections = Vec::new();
        let mut operations = Vec::new();
        for action in actions {
            match action {
                Action::Snapshot {
                    collection,
                    scope,
                    rows,
                } => {
                    let fields: BTreeSet<String> = rows
                        .values()
                        .filter_map(Value::as_object)
                        .flat_map(|o| o.keys().cloned())
                        .collect();
                    let live: Vec<String> = fields
                        .iter()
                        .filter(|k| match collection.as_str() {
                            "player_profile" => k.as_str() == "world_level",
                            "characters" => {
                                matches!(k.as_str(), "friendship" | "skill_levels" | "talent_ids")
                            }
                            _ => !matches!(k.as_str(), "unknown_details"),
                        })
                        .cloned()
                        .collect();
                    collections.push(json!({"name":collection,"scope":scope,"verified_fields":fields,"live_fields":live,"entities":rows.into_iter().map(|(k,v)|json!({"entity_key":k,"values":v})).collect::<Vec<_>>()}));
                }
                Action::Set {
                    collection,
                    key,
                    value,
                } => {
                    let expected = before
                        .as_ref()
                        .and_then(|p| p.entity(&collection, &key))
                        .cloned()
                        .unwrap_or(Value::Null);
                    operations.push(json!({"collection":collection,"entity_key":key,"action":if value.is_some(){"set"}else{"delete"},"values":value.unwrap_or(json!({})),"expected":expected}));
                }
                Action::Invalidate { reason, .. } => {
                    let names = self.projection.collection_names();
                    if self.profile.is_some() && !names.is_empty() {
                        out.push(self.event(
                            raw,
                            "state.invalidated",
                            json!({"collections":names,"reason":reason,"requires_snapshot":true}),
                        )?);
                    }
                }
                Action::Notice { .. } => {}
            }
        }
        let evidence =
            json!({"value_format":"legacy_capture_v1","legacy_sequence":sequence.to_string()});
        if !collections.is_empty() {
            let snapshot = Uuid::new_v5(
                &Uuid::parse_str(&self.capture_id)?,
                format!("snapshot:{sequence}").as_bytes(),
            );
            out.push(self.event(raw,"state.snapshot",json!({"snapshot_id":snapshot.to_string(),"collections":collections,"evidence":evidence}))?);
        }
        if !operations.is_empty() {
            out.push(self.event(
                raw,
                "state.batch",
                json!({"operations":operations,"evidence":evidence}),
            )?);
        }
        if matches!(
            kind,
            "account_update_rejected"
                | "achievement_update_rejected"
                | "profile_update_rejected"
                | "material_update_rejected"
        ) {
            out.push(self.event(raw, "diagnostic.notice", json!({"code":"capture_update_rejected","details":{"event":kind,"command_id":data["command_id"].as_u64()}}))?);
        }
        if kind == "status" {
            let state = data["state"].as_str().unwrap_or("");
            if state == "source_error" || state.starts_with("decoder_failed") {
                out.push(self.event(
                    raw,
                    "diagnostic.notice",
                    json!({"code":"capture_error","details":{"state":state}}),
                )?);
            }
            if state.starts_with("stopped") || state == "source_error" {
                let reason = match state {
                    "stopped_game_exited" => "game_exited",
                    "stopped_duration_limit" => "deadline",
                    "stopped" => "user",
                    _ => "source_end",
                };
                out.push(self.event(raw, "capture.closed", json!({"reason":reason}))?);
                self.closed = true;
            }
        }
        // Every raw row advances durable source provenance, but only periodic source metrics
        // become heartbeats. They never contain packet payloads, keys or login tokens.
        if matches!(kind, "source_stats" | "source_summary") && !self.closed {
            out.push(self.event(raw,"diagnostic.notice",json!({"code":"collector_heartbeat","details":{"acquired_packets":data["acquired_packets"],"dropped_queue_full":data["dropped_queue_full"]}}))?);
        }
        self.legacy_sequence = sequence;
        Ok(out)
    }
}

pub fn load_regions(path: &Path) -> Result<BTreeMap<String, String>> {
    ensure!(
        fs::metadata(path)?.len() < 65536,
        "Region config exceeds limit"
    );
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
    )?)
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    // Windows readers/antivirus may temporarily deny replacement. Keep the old
    // durable file and retry the same rename; never delete the destination first.
    retry_replace(|| fs::rename(&temp, path))
        .with_context(|| format!("Replace delivery file {}", path.display()))?;
    Ok(())
}
fn transient_file_error(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}
fn retry_replace(mut replace: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    for attempt in 0..8 {
        match replace() {
            Err(error) if transient_file_error(&error) && attempt < 7 => {
                std::thread::sleep(Duration::from_millis(10 << attempt.min(4)));
            }
            result => return result,
        }
    }
    unreachable!()
}
fn hash(h: &Sha256) -> String {
    format!("{:x}", h.clone().finalize())
}
pub async fn watch(
    root: PathBuf,
    regions: PathBuf,
    token: String,
    port: u16,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut started = BTreeSet::new();
    let mut workers = Vec::new();
    let mut poll = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){break;}},_=poll.tick()=>{
            if let Ok(entries)=fs::read_dir(&root){for entry in entries.flatten(){let dir=entry.path();if !entry.file_name().to_string_lossy().starts_with("realtime-"){continue;}
                let descriptor=dir.join("delivery.json");if !descriptor.is_file()||started.contains(&dir){continue;}started.insert(dir.clone());
                let(root,regions,token,rx)=(root.clone(),regions.clone(),token.clone(),shutdown.clone());workers.push(tokio::spawn(async move{
                    let mut attempts=0;
                    let outcome=loop {
                        let result=run(&root,&dir,&regions,&token,port,rx.clone()).await;
                        let retry=result.as_ref().err().is_some_and(|error|error.chain().any(|cause|cause.downcast_ref::<std::io::Error>().is_some_and(transient_file_error)));
                        if !retry || attempts>=5 {break result;}
                        attempts+=1;
                        crate::app_log!("Temporary delivery file access failure for {}; replaying durable ACK (attempt {attempts}/5)",dir.file_name().unwrap_or_default().to_string_lossy());
                        let mut stop=rx.clone();
                        tokio::select!{_=tokio::time::sleep(Duration::from_secs(1))=>{},_=stop.changed()=>{return;}}
                        if *rx.borrow(){return;}
                    };
                    if let Err(error)=outcome{crate::app_log!("Delivery blocked for {}: {error:#}",dir.file_name().unwrap_or_default().to_string_lossy());
                    // Quarantine an already accepted capture. If the backend is offline,
                    // its lease expiry still prevents an indefinitely live state.
                    if let Ok(bytes)=fs::read(dir.join("delivery.json"))
                        && let Ok(desc)=serde_json::from_slice::<Descriptor>(&bytes)
                        && Uuid::parse_str(&desc.capture_id).is_ok()
                        && let Ok(client)=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(3)).build(){
                        let _=client.post(format!("http://127.0.0.1:{port}/internal/v1/captures/{}/delivery-failed",desc.capture_id)).bearer_auth(token).send().await;
                    }
                }}));
            }}
        }}
    }
    for worker in workers {
        let _ = worker.await;
    }
}
pub async fn run(
    root: &Path,
    dir: &Path,
    regions_path: &Path,
    token: &str,
    port: u16,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let canonical_root = root.canonicalize()?;
    let canonical_dir = dir.canonicalize()?;
    ensure!(
        canonical_dir.starts_with(&canonical_root),
        "Capture path escaped watch root"
    );
    let desc: Descriptor = serde_json::from_slice(&fs::read(dir.join("delivery.json"))?)?;
    ensure!(
        desc.schema_version == 1 && desc.log_file == "capture.jsonl",
        "Unsupported delivery descriptor"
    );
    Uuid::parse_str(&desc.capture_id)?;
    let source = dir.join("capture.jsonl");
    let journal_dir = dir.join("delivery");
    fs::create_dir_all(&journal_dir)?;
    ensure!(
        journal_dir.canonicalize()?.starts_with(&canonical_dir),
        "Journal escaped capture directory"
    );
    let ack_path = journal_dir.join("ack.json");
    let outbox = journal_dir.join("outbox.jsonl");
    let checkpoint: Checkpoint = if ack_path.exists() {
        serde_json::from_slice(&fs::read(&ack_path)?)?
    } else {
        Checkpoint {
            capture_id: desc.capture_id.clone(),
            ..Default::default()
        }
    };
    ensure!(
        checkpoint.capture_id == desc.capture_id,
        "Checkpoint capture ID mismatch"
    );
    // The immutable source prefix rebuilds converter state. Unacknowledged wire bytes are
    // deterministic and can be regenerated after a process crash, including an ACK lost in transit.
    let mut normalizer = Normalizer::new(&desc.capture_id)?;
    let mut offset = 0u64;
    let mut source_hash = Sha256::new();
    let mut event_hash = Sha256::new();
    let mut verified = checkpoint.source_offset == 0;
    let mut pending: VecDeque<Pending> = VecDeque::new();
    let mut pending_bytes = 0usize;
    let mut initialized = false;
    let mut last_warning = String::new();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(3))
        .build()?;
    let url = format!(
        "http://127.0.0.1:{port}/internal/v1/captures/{}/events",
        desc.capture_id
    );
    let mut poll = tokio::time::interval(Duration::from_millis(250));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){return Ok(());}},_=poll.tick()=>{}}
        if !source.exists() {
            continue;
        }
        ensure!(
            source.canonicalize()?.starts_with(&canonical_dir),
            "Source log escaped capture directory"
        );
        ensure!(
            fs::metadata(&source)?.len() >= offset.max(checkpoint.source_offset),
            "Source journal truncated: fresh capture required"
        );
        if !initialized && verified {
            atomic(&outbox, b"")?;
            initialized = true;
        }
        if pending_bytes < OUTBOX_LIMIT - 32 * 1024 * 1024 {
            let mut file = File::open(&source)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut reader = BufReader::new(file);
            for _ in 0..if verified { 256 } else { 4096 } {
                let mut line = Vec::new();
                let len = reader
                    .by_ref()
                    .take(16 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)?;
                if len == 0 || line.last() != Some(&b'\n') {
                    break;
                }
                ensure!(len <= 16 * 1024 * 1024, "Probe line exceeds delivery limit");
                let raw: Value = serde_json::from_slice(if offset == 0 {
                    line.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&line)
                } else {
                    &line
                })?;
                let regions = match load_regions(regions_path) {
                    Ok(regions) => regions,
                    Err(_) => {
                        if last_warning != "Region configuration unavailable" {
                            crate::app_log!(
                                "Region configuration unavailable; delivery paused until corrected"
                            );
                            last_warning = "Region configuration unavailable".into();
                        }
                        break;
                    }
                };
                let events = match normalizer.feed(&raw, &regions) {
                    Ok(e) => {
                        last_warning.clear();
                        e
                    }
                    Err(error) => {
                        let warning = error.to_string();
                        if warning.contains("Identity pending") {
                            if warning != last_warning {
                                crate::app_log!("{warning}");
                                last_warning = warning;
                            }
                            break;
                        }
                        return Err(error);
                    }
                };
                source_hash.update(&line);
                for e in &events {
                    event_hash.update(serde_json::to_vec(e)?);
                    event_hash.update(b"\n");
                }
                offset += len as u64;
                if offset <= checkpoint.source_offset {
                    if offset == checkpoint.source_offset {
                        ensure!(
                            hash(&source_hash) == checkpoint.source_sha256
                                && hash(&event_hash) == checkpoint.event_sha256
                                && normalizer.sequence == checkpoint.acked_sequence,
                            "Source/configuration changed before durable ACK: fresh capture required"
                        );
                        verified = true;
                        atomic(&outbox, b"")?;
                        initialized = true;
                    }
                    continue;
                }
                ensure!(verified, "Checkpoint is not on a source record boundary");
                let record = Pending {
                    source_offset: offset,
                    source_sha256: hash(&source_hash),
                    event_sha256: hash(&event_hash),
                    events,
                };
                let encoded = serde_json::to_vec(&record)?;
                // A single source line is at most16MiB; pause source reads at the budget.
                ensure!(
                    encoded.len() < 32 * 1024 * 1024,
                    "Normalized event exceeds request limit"
                );
                let mut f = OpenOptions::new().create(true).append(true).open(&outbox)?;
                f.write_all(&encoded)?;
                f.write_all(b"\n")?;
                f.sync_data()?;
                pending_bytes += encoded.len() + 1;
                pending.push_back(record);
                if pending_bytes >= OUTBOX_LIMIT - 32 * 1024 * 1024 {
                    break;
                }
            }
        }
        // Send in source order, never blocking packet acquisition or capture JSONL writes.
        for _ in 0..64 {
            let Some(front) = pending.front() else { break };
            let accepted = if front.events.is_empty() {
                true
            } else {
                match client
                    .post(&url)
                    .bearer_auth(token)
                    .json(&json!({"events":front.events}))
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        let body: Value = response.json().await.context("Invalid delivery ACK")?;
                        ensure!(
                            body["acked_sequence"]
                                == front.events.last().unwrap()["source_sequence"],
                            "Backend acknowledged a different sequence"
                        );
                        ensure!(
                            !body["results"]
                                .as_array()
                                .context("Missing ACK results")?
                                .iter()
                                .any(|r| r["resync_required"] == true),
                            "Backend rejected a patch: fresh capture and snapshot required"
                        );
                        true
                    }
                    Ok(response) if response.status().is_client_error() => {
                        let code = response.status();
                        bail!("Backend rejected delivery ({code}); journal retained");
                    }
                    _ => false,
                }
            };
            if !accepted {
                break;
            }
            let record = pending.pop_front().unwrap();
            let acked = record
                .events
                .last()
                .and_then(|e| e["source_sequence"].as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    normalizer
                        .sequence
                        .saturating_sub(pending.iter().map(|p| p.events.len() as u64).sum::<u64>())
                });
            let state = Checkpoint {
                capture_id: desc.capture_id.clone(),
                acked_sequence: acked,
                source_offset: record.source_offset,
                source_sha256: record.source_sha256,
                event_sha256: record.event_sha256,
            };
            atomic(&ack_path, &serde_json::to_vec(&state)?)?;
            let mut bytes = Vec::new();
            for p in &pending {
                serde_json::to_writer(&mut bytes, p)?;
                bytes.push(b'\n');
            }
            atomic(&outbox, &bytes)?;
            pending_bytes = bytes.len();
        }
        if normalizer.closed && pending.is_empty() {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod file_retry_tests {
    use super::*;
    #[test]
    fn permanent_replace_error_is_not_retried() {
        let mut calls = 0;
        let result = retry_replace(|| {
            calls += 1;
            Err(std::io::Error::from_raw_os_error(2))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
    #[cfg(windows)]
    #[test]
    fn temporary_sharing_denial_retries_but_is_bounded() {
        let mut calls = 0;
        retry_replace(|| {
            calls += 1;
            if calls < 3 {
                Err(std::io::Error::from_raw_os_error(32))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(calls, 3);
        calls = 0;
        assert!(
            retry_replace(|| {
                calls += 1;
                Err(std::io::Error::from_raw_os_error(5))
            })
            .is_err()
        );
        assert_eq!(calls, 8);
    }
    #[cfg(windows)]
    #[test]
    fn atomic_replace_survives_a_real_windows_reader_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ack.json");
        atomic(&path, b"old").unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let unlock = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            drop(reader);
        });
        atomic(&path, b"new").unwrap();
        unlock.join().unwrap();
        assert_eq!(fs::read(path).unwrap(), b"new");
    }
}
