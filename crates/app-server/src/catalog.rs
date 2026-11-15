//! Immutable catalog bundles and a single atomic activation pointer. No account writes.
pub mod build;
mod character_details;
mod crafting;
mod equipment_details;
pub mod http;
mod leveling;
mod player_profile;
mod progression;
mod talents;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::Mutex;

pub const LIMIT: usize = 32 * 1024 * 1024;
pub const SOURCE: &str = "https://gitlab.com/Dimbreath/animegamedata2";
/// Release bootstrap must include all currently shipped product features.
/// Older catalogs remain readable for recovery, but cannot seed a new portable build.
pub fn portable_payload(bundle: &Bundle) -> Result<Vec<u8>> {
    bundle.validate()?;
    for (section, table) in [
        ("characterDetails", "skills"),
        ("characterDetails", "passives"),
        ("characterDetails", "constellations"),
        ("equipmentDetails", "weapons"),
        ("equipmentDetails", "sets"),
        ("progression", "avatarPaths"),
        ("progression", "talentLevels"),
        ("progression", "weaponPaths"),
        ("leveling", "avatarExp"),
        ("leveling", "weaponExp"),
        ("playerProfile", "pictures"),
        ("crafting", "recipes"),
    ] {
        let value = &bundle.payload[section][table];
        ensure!(
            value.as_object().is_some_and(|v| !v.is_empty())
                || value.as_array().is_some_and(|v| !v.is_empty()),
            "Portable catalog missing required table: {section}.{table}"
        );
    }
    Ok(serde_json::to_vec(&bundle.payload)?)
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn revision(s: &str) -> Result<()> {
    ensure!(
        hex(s, 40),
        "Expected a full lowercase catalog revision (40 hex digits)"
    );
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format_version: u32,
    pub game: String,
    pub language: String,
    pub value_format: String,
    pub revision: String,
    pub source: String,
    pub created_at: String,
    pub payload_sha256: String,
    pub source_files: BTreeMap<String, String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub manifest: Manifest,
    pub payload: Value,
}
impl Bundle {
    pub fn new(
        payload: Value,
        revision: &str,
        source: &str,
        source_files: BTreeMap<String, String>,
    ) -> Result<Self> {
        let bundle = Self {
            manifest: Manifest {
                format_version: 1,
                game: "genshin".into(),
                language: "ru".into(),
                value_format: "legacy_capture_v1".into(),
                revision: revision.into(),
                source: source.into(),
                created_at: OffsetDateTime::now_utc().format(&Rfc3339)?,
                payload_sha256: hash(&serde_json::to_vec(&payload)?),
                source_files,
            },
            payload,
        };
        bundle.validate()?;
        Ok(bundle)
    }
    pub fn validate(&self) -> Result<()> {
        let m = &self.manifest;
        ensure!(
            m.format_version == 1
                && m.game == "genshin"
                && m.language == "ru"
                && m.value_format == "legacy_capture_v1",
            "Incompatible catalog format/game/language"
        );
        revision(&m.revision)?;
        ensure!(
            matches!(m.source.as_str(), SOURCE | "bundled"),
            "Unsupported catalog source"
        );
        OffsetDateTime::parse(&m.created_at, &Rfc3339)?;
        ensure!(
            m.source_files.values().all(|v| hex(v, 64)),
            "Invalid source file hashes"
        );
        let bytes = serde_json::to_vec(&self.payload)?;
        ensure!(
            bytes.len() <= LIMIT && hash(&bytes) == m.payload_sha256,
            "Catalog checksum mismatch"
        );
        validate_payload(&self.payload, &m.revision)
    }
}
fn object<'a>(v: &'a Value, k: &str) -> Result<&'a serde_json::Map<String, Value>> {
    v[k].as_object()
        .filter(|v| !v.is_empty())
        .with_context(|| format!("Missing catalog map: {k}"))
}
fn positive_id(s: &str) -> bool {
    !s.starts_with('0') && s.parse::<u32>().is_ok_and(|n| n > 0)
}
fn text(s: &Value) -> bool {
    s.as_str()
        .is_some_and(|s| !s.is_empty() && s.len() <= 16384)
}
fn icon(s: &Value) -> bool {
    s.is_null()
        || s.as_str().is_some_and(|s| {
            s.len() <= 160 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
pub fn validate_payload(v: &Value, rev: &str) -> Result<()> {
    talents::validate(v)?;
    crafting::validate(v)?;
    player_profile::validate(v)?;
    character_details::validate(v)?;
    equipment_details::validate(v)?;
    progression::validate(v)?;
    leveling::validate(v)?;
    for key in [
        "items",
        "names",
        "rarities",
        "avatarNames",
        "avatarIcons",
        "skillNames",
        "materials",
        "achievements",
    ] {
        let map = object(v, key)?;
        ensure!(
            map.len() <= 100000 && map.keys().all(|id| positive_id(id)),
            "Invalid IDs in {key}"
        );
    }
    for key in ["names", "avatarNames", "skillNames"] {
        ensure!(
            object(v, key)?.values().all(text),
            "Invalid localized text: {key}"
        );
    }
    for key in ["items", "avatars", "achievements"] {
        ensure!(v["revisions"][key] == rev, "Mixed catalog revisions: {key}");
    }
    for (id, item) in object(v, "items")? {
        ensure!(
            item.is_object() && icon(&item["icon"]),
            "Invalid item/icon {id}"
        );
        ensure!(
            item["rank"].is_null()
                || item["rank"]
                    .as_u64()
                    .is_some_and(|n| n <= u64::from(u32::MAX)),
            "Invalid item rank"
        );
    }
    for (id, rarity) in object(v, "rarities")? {
        ensure!(
            rarity.as_u64().is_some_and(|n| (1..=5).contains(&n))
                && (v["items"].get(id).is_some() || v["names"].get(id).is_some()),
            "Invalid rarity/link {id}"
        );
    }
    for (id, d) in object(v, "materials")? {
        ensure!(
            d["itemType"]
                .as_str()
                .is_some_and(|s| s.starts_with("ITEM_"))
                && v["items"].get(id).is_some(),
            "Invalid material/link {id}"
        );
    }
    for (id, s) in object(v, "avatarIcons")? {
        ensure!(
            icon(s) && v["avatarNames"].get(id).is_some(),
            "Invalid character/icon link {id}"
        );
    }
    for (id, a) in object(v, "achievements")? {
        let root = a["chain_id"]
            .as_str()
            .context("Missing achievement chain")?;
        ensure!(
            text(&a["name"])
                && a["stage"].as_u64().is_some_and(|n| (1..=10).contains(&n))
                && v["achievements"][root]["goal_id"] == a["goal_id"],
            "Invalid achievement/link {id}"
        );
        for field in ["excluded", "is_disused"] {
            ensure!(
                a.get(field).is_none_or(Value::is_boolean),
                "Invalid achievement classification {id}"
            );
        }
    }
    for key in ["mainProps", "affixes", "relicLevels"] {
        let list = v[key]
            .as_array()
            .filter(|a| !a.is_empty())
            .with_context(|| format!("Missing stat table: {key}"))?;
        ensure!(list.len() <= 100000, "Stat table size limit");
        let mut ids = std::collections::BTreeSet::new();
        for entry in list {
            ensure!(entry.is_object(), "Invalid stat entry");
            if key != "relicLevels" {
                ensure!(
                    entry["id"].as_u64().is_some_and(|n| n > 0)
                        && ids.insert(entry["id"].to_string())
                        && entry["propType"]
                            .as_str()
                            .is_some_and(|p| p.starts_with("FIGHT_PROP_")),
                    "Invalid/duplicate stat identity"
                );
            } else {
                ensure!(
                    entry["level"]
                        .as_u64()
                        .is_some_and(|n| (1..=100).contains(&n))
                        && entry["addProps"].is_array()
                        && ids.insert(format!("{}:{}", entry["rank"], entry["level"])),
                    "Invalid/duplicate artifact level"
                );
            }
        }
    }
    ensure!(
        v["inventoryClassification"]["schemaVersion"] == 1,
        "Unsupported inventory classification format"
    );
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    current: String,
    previous: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    base_active: String,
    bundle: Bundle,
}
struct Loaded {
    id: String,
    bundle: Bundle,
    bytes: Arc<Vec<u8>>,
    payload: Arc<Vec<u8>>,
}
impl Loaded {
    fn new(bundle: Bundle) -> Result<Self> {
        bundle.validate()?;
        let bytes = serde_json::to_vec(&bundle)?;
        ensure!(bytes.len() <= LIMIT, "Bundle exceeds size limit");
        Ok(Self {
            id: hash(&bytes),
            payload: Arc::new(serde_json::to_vec(&bundle.payload)?),
            bundle,
            bytes: Arc::new(bytes),
        })
    }
    fn info(&self) -> Value {
        json!({"id":self.id,"manifest":self.bundle.manifest,"counts":counts(&self.bundle.payload)})
    }
}
fn counts(v: &Value) -> Value {
    let achievements = v["achievements"].as_object();
    let total = achievements.map_or(0, |v| v.len());
    let classified =
        achievements.is_some_and(|rows| rows.values().all(|r| r["is_disused"].is_boolean()));
    let excluded = achievements.map_or(0, |rows| {
        rows.values()
            .filter(|r| r["excluded"] == true || r["is_disused"] == true)
            .count()
    });
    json!({"items":v["items"].as_object().map_or(0,|v|v.len()),"characters":v["avatarNames"].as_object().map_or(0,|v|v.len()),"achievements":total,"achievements_playable":classified.then_some(total-excluded),"achievements_excluded":classified.then_some(excluded),"avatar_promotion_paths":v["progression"]["avatarPromotions"].as_object().map(|v|v.len()),"weapon_promotion_paths":v["progression"]["weaponPromotions"].as_object().map(|v|v.len()),"talent_cost_groups":v["progression"]["talentLevels"].as_object().map(|v|v.len())})
}
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| equivalent(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, a)| y.get(k).is_some_and(|b| equivalent(a, b)))
        }
        _ => a == b,
    }
}
fn candidate_info(base: &str, next: &Loaded, active: &Loaded) -> Value {
    json!({"base_active":base,"version":next.info(),"unchanged":next.bundle.manifest.payload_sha256==active.bundle.manifest.payload_sha256,"changes":changes(&active.bundle.payload,&next.bundle.payload)})
}
fn changes(old: &Value, new: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "names",
        "items",
        "materials",
        "avatarNames",
        "skillNames",
        "achievements",
    ] {
        let a = old[key].as_object().unwrap();
        let b = new[key].as_object().unwrap();
        let equal = |x: &Value, y: &Value| {
            if key == "materials" {
                ["itemType", "materialType", "systemType"]
                    .iter()
                    .all(|k| x[*k] == y[*k])
            } else {
                x == y
            }
        };
        let added = b.keys().filter(|k| !a.contains_key(*k)).count();
        let removed = a.keys().filter(|k| !b.contains_key(*k)).count();
        let changed = a
            .iter()
            .filter(|(k, v)| b.get(*k).is_some_and(|next| !equal(v, next)))
            .count();
        out.insert(
            key.into(),
            json!({"added":added,"removed":removed,"changed":changed}),
        );
    }
    out.insert(
        "stat_tables_changed".into(),
        json!(
            ["mainProps", "affixes", "relicLevels", "equipmentDetails"]
                .iter()
                .any(|k| !equivalent(&old[*k], &new[*k]))
        ),
    );
    out.insert(
        "talent_links_changed".into(),
        json!(
            [
                "skillMetadata",
                "skillDepots",
                "avatarMetadata",
                "characterDetails"
            ]
            .iter()
            .any(|k| !equivalent(&old[*k], &new[*k]))
        ),
    );
    out.insert(
        "progression_changed".into(),
        json!(!equivalent(&old["progression"], &new["progression"])),
    );
    out.insert(
        "profile_presentation_changed".into(),
        json!(!equivalent(&old["playerProfile"], &new["playerProfile"])),
    );
    out.insert(
        "crafting_changed".into(),
        json!(!equivalent(&old["crafting"], &new["crafting"])),
    );
    out.insert(
        "leveling_changed".into(),
        json!(!equivalent(&old["leveling"], &new["leveling"])),
    );
    Value::Object(out)
}
struct State {
    current: Arc<Loaded>,
    previous: Option<Arc<Loaded>>,
    error: Option<String>,
    candidate: Option<Value>,
}
type Recovery = (Arc<Loaded>, Option<Arc<Loaded>>, Option<String>);
pub struct Catalogs {
    root: PathBuf,
    state: RwLock<State>,
    pub operation: Mutex<()>,
}
fn read(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(path)?.len() <= LIMIT as u64,
        "Catalog file exceeds size limit"
    );
    Ok(fs::read(path)?)
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        for attempt in 0..6 {
            match fs::rename(&temporary, path) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < 5 => {
                    std::thread::sleep(std::time::Duration::from_millis(30))
                }
                Err(e) => return Err(e.into()),
            }
        }
        anyhow::bail!("Atomic catalog replacement failed")
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
impl Catalogs {
    pub fn open(root: PathBuf, payload: Value) -> Result<Self> {
        let rev = payload["revisions"]["items"]
            .as_str()
            .context("Bundled catalog has no revision")?
            .to_owned();
        let seed = Arc::new(Loaded::new(Bundle::new(
            payload,
            &rev,
            "bundled",
            BTreeMap::new(),
        )?)?);
        let manager = Self {
            root,
            state: RwLock::new(State {
                current: seed,
                previous: None,
                error: None,
                candidate: None,
            }),
            operation: Mutex::new(()),
        };
        if let Err(error) = fs::create_dir_all(manager.root.join("versions")) {
            manager.fail(&error.into());
            return Ok(manager);
        }
        let active = manager.root.join("active.json");
        if active.exists() || manager.root.join("last-good.json").exists() {
            match manager.recover() {
                Ok((current, previous, recovered)) => {
                    let mut s = manager.state.write().unwrap();
                    s.current = current;
                    s.previous = previous;
                    s.error = recovered;
                }
                Err(e) => {
                    manager.state.write().unwrap().error = Some(format!(
                        "Using bundled catalog after recovery failure: {e:#}"
                    ))
                }
            }
        } else {
            let initial = (|| -> Result<()> {
                let s = manager.state.read().unwrap();
                manager.save(&s.current)?;
                let p = Pointer {
                    current: s.current.id.clone(),
                    previous: None,
                };
                atomic(&active, &serde_json::to_vec(&p)?)?;
                atomic(
                    &manager.root.join("last-good.json"),
                    &serde_json::to_vec(&p)?,
                )?;
                Ok(())
            })();
            if let Err(error) = initial {
                manager.fail(&error);
            }
        }
        let current = manager.state.read().unwrap().current.clone();
        if let Err(error) = manager.save(&current) {
            manager.fail(&error);
        }
        if let Ok(bytes) = read(&manager.root.join("staged.json")) {
            let staged = (|| -> Result<Value> {
                let candidate: Candidate = serde_json::from_slice(&bytes)?;
                let checked = Loaded::new(candidate.bundle)?;
                Ok(candidate_info(&candidate.base_active, &checked, &current))
            })();
            match staged {
                Ok(info) => manager.state.write().unwrap().candidate = Some(info),
                Err(error) => manager.fail(&error),
            }
        }
        Ok(manager)
    }
    fn load(&self, id: &str) -> Result<Arc<Loaded>> {
        ensure!(hex(id, 64), "Invalid catalog version ID");
        let bytes = read(&self.root.join("versions").join(format!("{id}.json")))?;
        ensure!(hash(&bytes) == id, "Catalog version checksum mismatch");
        Ok(Arc::new(Loaded::new(serde_json::from_slice(&bytes)?)?))
    }
    fn recover(&self) -> Result<Recovery> {
        for name in ["active.json", "last-good.json"] {
            if let Ok(p) =
                read(&self.root.join(name)).and_then(|b| Ok(serde_json::from_slice::<Pointer>(&b)?))
            {
                if let Ok(current) = self.load(&p.current) {
                    let previous = p.previous.as_deref().and_then(|id| self.load(id).ok());
                    return Ok((
                        current,
                        previous,
                        (name != "active.json")
                            .then(|| "Recovered the last working catalog pointer".into()),
                    ));
                }
                if let Some(id) = p.previous
                    && let Ok(previous) = self.load(&id)
                {
                    return Ok((
                        previous,
                        None,
                        Some("Active catalog damaged; using the previous verified version".into()),
                    ));
                }
            }
        }
        anyhow::bail!("No verified runtime version")
    }
    fn save(&self, loaded: &Loaded) -> Result<()> {
        let p = self
            .root
            .join("versions")
            .join(format!("{}.json", loaded.id));
        if p.exists() {
            ensure!(hash(&read(&p)?) == loaded.id, "Existing version damaged");
        } else {
            atomic(&p, &loaded.bytes)?;
        }
        Ok(())
    }
    pub fn status(&self) -> Value {
        let s = self.state.read().unwrap();
        json!({"current":s.current.info(),"previous":s.previous.as_ref().map(|v|v.info()),"candidate":s.candidate,"last_error":s.error,"busy":self.operation.try_lock().is_err()})
    }

    pub fn fail(&self, e: &anyhow::Error) {
        self.state.write().unwrap().error = Some(format!("{e:#}"));
    }
    pub fn payload(&self) -> (String, Arc<Vec<u8>>) {
        let s = self.state.read().unwrap();
        (s.current.id.clone(), s.current.payload.clone())
    }
    pub fn export(&self) -> Arc<Vec<u8>> {
        self.state.read().unwrap().current.bytes.clone()
    }
    pub fn stage(&self, bundle: Bundle) -> Result<Value> {
        let checked = Loaded::new(bundle.clone())?;
        let current = self.state.read().unwrap().current.clone();
        ensure!(
            checked.bundle.payload["names"].as_object().unwrap().len()
                >= current.bundle.payload["names"].as_object().unwrap().len() / 2,
            "Suspiciously incomplete catalog: too many item names removed"
        );
        atomic(
            &self.root.join("staged.json"),
            &serde_json::to_vec(&Candidate {
                base_active: current.id.clone(),
                bundle,
            })?,
        )?;
        let mut state = self.state.write().unwrap();
        state.error = None;
        state.candidate = Some(candidate_info(&current.id, &checked, &current));
        Ok(checked.info())
    }
    fn switch(&self, next: Arc<Loaded>, expected: &str) -> Result<()> {
        let mut s = self.state.write().unwrap();
        ensure!(
            s.current.id == expected,
            "Active catalog changed; review the candidate again"
        );
        if next.id == s.current.id {
            return Ok(());
        }
        self.save(&next)?;
        atomic(
            &self.root.join("last-good.json"),
            &serde_json::to_vec(&Pointer {
                current: s.current.id.clone(),
                previous: s.previous.as_ref().map(|v| v.id.clone()),
            })?,
        )?;
        atomic(
            &self.root.join("active.json"),
            &serde_json::to_vec(&Pointer {
                current: next.id.clone(),
                previous: Some(s.current.id.clone()),
            })?,
        )?;
        s.previous = Some(s.current.clone());
        s.current = next;
        s.error = None;
        Ok(())
    }
    pub fn activate(&self, id: &str, expected: &str) -> Result<()> {
        let candidate: Candidate = serde_json::from_slice(&read(&self.root.join("staged.json"))?)?;
        ensure!(
            candidate.base_active == expected,
            "Candidate belongs to another active version"
        );
        let checked = Arc::new(Loaded::new(candidate.bundle)?);
        ensure!(checked.id == id, "Candidate changed; review it again");
        self.switch(checked, expected)?;
        let _ = fs::remove_file(self.root.join("staged.json"));
        self.state.write().unwrap().candidate = None;
        Ok(())
    }
    pub fn rollback(&self, expected: &str) -> Result<()> {
        let old = self
            .state
            .read()
            .unwrap()
            .previous
            .clone()
            .context("No previous catalog version")?;
        let verified = self.load(&old.id)?;
        self.switch(verified, expected)
    }
}
