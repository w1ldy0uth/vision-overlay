//! Build the same presentation model as the bundled catalog from one pinned source revision.
use super::{Bundle, SOURCE, hash, revision};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
    time::Duration,
};

pub const TABLES: &[&str] = &[
    "Material",
    "Weapon",
    "Reliquary",
    "HomeWorldFurniture",
    "BydMaterial",
    "BeyondHallFacility",
    "TpsWeapon",
    "Avatar",
    "AvatarSkill",
    "Achievement",
    "AchievementGoal",
    "ReliquaryMainProp",
    "ReliquaryLevel",
    "ReliquaryAffix",
    "AvatarSkillDepot",
    "AvatarTalent",
    "AvatarPromote",
    "WeaponPromote",
    "ProudSkill",
    "AvatarLevel",
    "WeaponLevel",
    "ConstValue",
    "Combine",
    "PlayerLevel",
    "ProfilePicture",
    "ProfileFrame",
    "WeaponCurve",
    "EquipAffix",
    "ReliquarySet",
];

/// Extend a verified same-revision package from a local checkout of source
/// tables, avoiding a second download of localization and unrelated tables.
pub fn extend_leveling(mut base: Bundle, sources: BTreeMap<String, Vec<u8>>) -> Result<Bundle> {
    base.validate()?;
    ensure!(
        base.payload.get("progression").is_some(),
        "Base package has no progression paths"
    );
    let mut tables = BTreeMap::new();
    for name in [
        "AvatarLevel",
        "WeaponLevel",
        "Material",
        "ConstValue",
        "Avatar",
        "AvatarSkill",
    ] {
        let bytes = sources.get(name).context("Missing leveling source file")?;
        ensure!(bytes.len() <= 32 * 1024 * 1024, "Source file size limit");
        let path = format!("ExcelBinOutput/{name}ExcelConfigData.json");
        let digest = hash(bytes);
        if let Some(old) = base.manifest.source_files.get(&path) {
            ensure!(
                old == &digest,
                "Source table differs from the base revision: {name}"
            );
        }
        base.manifest.source_files.insert(path, digest);
        tables.insert(name.into(), serde_json::from_slice(bytes)?);
    }
    base.payload["leveling"] = super::leveling::compile(&tables)?;
    base.payload["avatarMetadata"] = super::talents::avatar_links(
        tables["Avatar"]
            .as_array()
            .context("Invalid avatar source")?,
    );
    for row in tables["AvatarSkill"]
        .as_array()
        .context("Invalid skill source")?
    {
        let id = row["id"].to_string();
        let target = base.payload["skillMetadata"]
            .get_mut(&id)
            .context("Missing skill metadata link")?;
        ensure!(
            target["proudSkillGroupId"] == row["proudSkillGroupId"],
            "Skill metadata differs from source"
        );
        target["element"] = row["costElemType"].clone();
    }
    Bundle::new(
        base.payload,
        &base.manifest.revision,
        &base.manifest.source,
        base.manifest.source_files,
    )
}
/// Enrich an existing verified package using its exact pinned revision.
pub fn extend_crafting(mut base: Bundle, bytes: Vec<u8>) -> Result<Bundle> {
    base.validate()?;
    ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "Combine source file size limit"
    );
    let key = "ExcelBinOutput/CombineExcelConfigData.json";
    let digest = hash(&bytes);
    ensure!(
        base.manifest
            .source_files
            .get(key)
            .is_none_or(|old| old == &digest),
        "Combine source differs from base"
    );
    base.payload["crafting"] = super::crafting::compile(&serde_json::from_slice(&bytes)?)?;
    base.manifest.source_files.insert(key.into(), digest);
    Bundle::new(
        base.payload,
        &base.manifest.revision,
        &base.manifest.source,
        base.manifest.source_files,
    )
}
pub fn extend_player_profile(
    mut base: Bundle,
    sources: BTreeMap<String, Vec<u8>>,
) -> Result<Bundle> {
    base.validate()?;
    let mut tables = BTreeMap::new();
    for name in ["PlayerLevel", "ProfilePicture", "ProfileFrame"] {
        let bytes = sources.get(name).context("Missing profile source")?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, "Profile source too large");
        let key = format!("ExcelBinOutput/{name}ExcelConfigData.json");
        let digest = hash(bytes);
        ensure!(
            base.manifest
                .source_files
                .get(&key)
                .is_none_or(|v| v == &digest),
            "Profile source differs from base"
        );
        base.manifest.source_files.insert(key, digest);
        tables.insert(name.into(), serde_json::from_slice(bytes)?);
    }
    base.payload["playerProfile"] = super::player_profile::compile(&tables)?;
    Bundle::new(
        base.payload,
        &base.manifest.revision,
        &base.manifest.source,
        base.manifest.source_files,
    )
}
pub fn extend_character_details(
    mut base: Bundle,
    sources: BTreeMap<String, Vec<u8>>,
) -> Result<Bundle> {
    base.validate()?;
    let mut tables = BTreeMap::new();
    let mut texts = Map::new();
    for name in [
        "AvatarSkill",
        "AvatarSkillDepot",
        "AvatarTalent",
        "ProudSkill",
    ] {
        let path = format!("ExcelBinOutput/{name}ExcelConfigData.json");
        let bytes = sources.get(&path).context("Missing character table")?;
        ensure!(
            bytes.len() <= 32 * 1024 * 1024
                && base.manifest.source_files.get(&path) == Some(&hash(bytes)),
            "Character table differs from pinned source"
        );
        tables.insert(name.into(), serde_json::from_slice(bytes)?);
    }
    let needed: BTreeSet<_> = tables
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .flat_map(|r| ["nameTextMapHash", "descTextMapHash"].map(|k| r[k].to_string()))
        .collect();
    for prefix in ["TextMapRU", "TextMap_MediumRU"] {
        for part in 0..2 {
            let path = format!("TextMap/{prefix}_{part}.json");
            let bytes = sources.get(&path).context("Missing localization")?;
            ensure!(
                bytes.len() <= 64 * 1024 * 1024
                    && base.manifest.source_files.get(&path) == Some(&hash(bytes)),
                "Localization differs from pinned source"
            );
            texts.extend(select_texts(bytes, &needed)?);
        }
    }
    base.payload["characterDetails"] = super::character_details::compile(&tables, &texts)?;
    Bundle::new(
        base.payload,
        &base.manifest.revision,
        &base.manifest.source,
        base.manifest.source_files,
    )
}
pub fn extend_equipment_details(
    mut base: Bundle,
    sources: BTreeMap<String, Vec<u8>>,
) -> Result<Bundle> {
    base.validate()?;
    let mut tables = BTreeMap::new();
    let mut texts = Map::new();
    for name in [
        "Weapon",
        "WeaponCurve",
        "WeaponPromote",
        "EquipAffix",
        "Reliquary",
        "ReliquarySet",
    ] {
        let path = format!("ExcelBinOutput/{name}ExcelConfigData.json");
        let bytes = sources.get(&path).context("Missing equipment source")?;
        let digest = hash(bytes);
        ensure!(
            bytes.len() <= 32 * 1024 * 1024
                && base
                    .manifest
                    .source_files
                    .get(&path)
                    .is_none_or(|v| *v == digest),
            "Equipment source differs from pinned revision"
        );
        base.manifest.source_files.insert(path, digest);
        tables.insert(name.into(), serde_json::from_slice(bytes)?);
    }
    let needed: BTreeSet<_> = tables
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .flat_map(|r| ["nameTextMapHash", "descTextMapHash"].map(|k| r[k].to_string()))
        .collect();
    for prefix in ["TextMapRU", "TextMap_MediumRU"] {
        for part in 0..2 {
            let path = format!("TextMap/{prefix}_{part}.json");
            let bytes = sources.get(&path).context("Missing localization")?;
            ensure!(
                bytes.len() <= 64 * 1024 * 1024
                    && base.manifest.source_files.get(&path) == Some(&hash(bytes)),
                "Localization differs from pinned revision"
            );
            texts.extend(select_texts(bytes, &needed)?);
        }
    }
    base.payload["equipmentDetails"] = super::equipment_details::compile(&tables, &texts)?;
    Bundle::new(
        base.payload,
        &base.manifest.revision,
        &base.manifest.source,
        base.manifest.source_files,
    )
}
static MALE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{M#([^{}]*)\}").unwrap());
static FEMALE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{F#[^{}]*\}").unwrap());
static MARKUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?(?:color|size|b|i)(?:=[^>]*)?>").unwrap());
pub(super) fn display(s: &str) -> String {
    let m = MALE.replace_all(s, "$1");
    let f = FEMALE.replace_all(&m, "");
    MARKUP
        .replace_all(f.strip_prefix('#').unwrap_or(&f), "")
        .replace("\\n", "\n")
}
pub fn compile(
    rev: &str,
    tables: &BTreeMap<String, Value>,
    texts: &Map<String, Value>,
    files: BTreeMap<String, String>,
) -> Result<Bundle> {
    revision(rev)?;
    let table = |name: &str| -> Result<&Vec<Value>> {
        tables
            .get(name)
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty())
            .with_context(|| format!("Missing source table {name}"))
    };
    let localized = |r: &Value, field: &str| {
        texts
            .get(&r[field].to_string())
            .and_then(Value::as_str)
            .map(display)
    };
    let mut names = Map::new();
    let mut items = Map::new();
    let mut rarities = Map::new();
    let mut materials = Map::new();
    for name in &TABLES[..7] {
        let mut seen = BTreeSet::new();
        for r in table(name)? {
            let id = r["id"]
                .as_u64()
                .filter(|v| *v > 0 && *v <= u64::from(u32::MAX))
                .context("Invalid source item ID")?
                .to_string();
            ensure!(seen.insert(id.clone()), "Duplicate source item ID");
            if let Some(text) = localized(r, "nameTextMapHash").filter(|s| !s.is_empty()) {
                names.insert(id.clone(), json!(text));
            }
            let rarity = r["rankLevel"].as_u64().unwrap_or(0);
            if (1..=5).contains(&rarity) {
                rarities.insert(id.clone(), json!(rarity));
            }
            items.insert(id.clone(),json!({"icon":r.get("itemIcon").or_else(||r.get("icon")),"rank":r["rank"],"rarity":rarity,"typeLabel":localized(r,"typeDescTextMapHash"),"weaponType":r["weaponType"],"equipType":r["equipType"],"setId":r["setId"]}));
            if *name == "Material" {
                materials.insert(id,json!({"itemType":r["itemType"],"materialType":r["materialType"],"systemType":r.get("systemType").or_else(||r.get("JDDNMAMMEPO")),"rankLevel":r["rankLevel"]}));
            }
        }
    }
    // Individually verified missing rarity fields, independent of language/name ordering.
    for id in ["120861", "121266"] {
        if items.contains_key(id) {
            rarities.insert(id.into(), json!(1));
        }
    }
    let mut avatar_names = Map::new();
    let mut avatar_icons = Map::new();
    let mut skill_names = Map::new();
    for r in table("Avatar")? {
        let id = r["id"].as_u64().context("Missing avatar ID")?.to_string();
        avatar_names.insert(
            id.clone(),
            json!(
                localized(r, "nameTextMapHash")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("Персонаж {id}"))
            ),
        );
        avatar_icons.insert(id, r["iconName"].clone());
    }
    for r in table("AvatarSkill")? {
        if let Some(text) = localized(r, "nameTextMapHash").filter(|s| !s.is_empty()) {
            skill_names.insert(r["id"].to_string(), json!(text));
        }
    }
    let goals: BTreeMap<_, _> = table("AchievementGoal")?
        .iter()
        .map(|r| (r["id"].as_u64().unwrap_or(0).to_string(), r))
        .collect();
    let source: BTreeMap<_, _> = table("Achievement")?
        .iter()
        .map(|r| (r["id"].to_string(), r))
        .collect();
    ensure!(
        source.len() == table("Achievement")?.len(),
        "Duplicate achievement ID"
    );
    let mut achievements = Map::new();
    for (id, r) in &source {
        let mut root = *r;
        let mut seen = BTreeSet::new();
        while let Some(previous) = root["preStageAchievementId"].as_u64().filter(|v| *v > 0) {
            ensure!(
                seen.insert(root["id"].to_string()),
                "Cyclic achievement chain"
            );
            let prev = *source
                .get(&previous.to_string())
                .context("Broken achievement chain")?;
            ensure!(
                prev["goalId"].as_u64().unwrap_or(0) == root["goalId"].as_u64().unwrap_or(0),
                "Achievement chain crosses categories"
            );
            root = prev;
        }
        let goal_id = r["goalId"].as_u64().unwrap_or(0);
        let goal = goals
            .get(&goal_id.to_string())
            .copied()
            .unwrap_or(&Value::Null);
        let mut description = localized(r, "descTextMapHash").unwrap_or_default();
        if !r["progress"].is_null() {
            description = description.replace("{param0}", &r["progress"].to_string());
        }
        achievements.insert(id.clone(),json!({"name":localized(r,"titleTextMapHash").unwrap_or_else(||format!("Достижение ID {id}")),"description":description,"goal_id":goal_id,"category":localized(goal,"nameTextMapHash").unwrap_or_else(||"Без категории".into()),"category_order":goal["orderId"].as_u64().unwrap_or(999999),"order":r["orderId"].as_u64().unwrap_or(r["id"].as_u64().context("Invalid achievement ID")?),"chain_order":root["orderId"].as_u64().unwrap_or(root["id"].as_u64().unwrap()),"chain_id":root["id"].to_string(),"stage":seen.len()+1,"is_disused":r["isDisuse"].as_bool().unwrap_or(false),"excluded":id=="84517" || r["isDisuse"]==true}));
    }
    ensure!(
        names.len() > 100 && avatar_names.len() > 10 && achievements.len() > 100,
        "Source localization is incomplete"
    );
    let (skill_metadata, skill_depots, avatar_metadata) = super::talents::compile(tables)?;
    let progression = super::progression::compile(tables)?;
    let leveling = super::leveling::compile(tables)?;
    Bundle::new(
        json!({"equipmentDetails":super::equipment_details::compile(tables,texts)?,"characterDetails":super::character_details::compile(tables,texts)?,"playerProfile":super::player_profile::compile(tables)?,"crafting":super::crafting::compile(tables.get("Combine").context("Missing Combine source")?)?,"leveling":leveling,"progression":progression,"skillMetadata":skill_metadata,"skillDepots":skill_depots,"avatarMetadata":avatar_metadata,"items":items,"names":names,"rarities":rarities,"avatarNames":avatar_names,"avatarIcons":avatar_icons,"skillNames":skill_names,"achievements":achievements,"materials":materials,"mainProps":table("ReliquaryMainProp")?,"affixes":table("ReliquaryAffix")?,"relicLevels":table("ReliquaryLevel")?,"revisions":{"items":rev,"avatars":rev,"achievements":rev},"inventoryClassification":{"schemaVersion":1,"revision":rev,"source":"MaterialExcelConfigData"}}),
        rev,
        SOURCE,
        files,
    )
}
pub async fn download(requested: Option<String>) -> Result<Bundle> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("VisionOverlay/0.1 catalog updater")
        .build()?;
    let rev = if let Some(rev) = requested {
        revision(&rev)?;
        rev
    } else {
        let bytes=fetch(&client,"https://gitlab.com/api/v4/projects/Dimbreath%2Fanimegamedata2/repository/commits?per_page=1",1024*1024).await?;
        let v: Value = serde_json::from_slice(&bytes)?;
        v[0]["id"]
            .as_str()
            .context("Source did not return a pinned revision")?
            .into()
    };
    revision(&rev)?;
    let mut texts = Map::new();
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    let mut tables = BTreeMap::new();
    for name in TABLES {
        let path = format!("ExcelBinOutput/{name}ExcelConfigData.json");
        let bytes = fetch(
            &client,
            &format!("{SOURCE}/-/raw/{rev}/{path}"),
            32 * 1024 * 1024,
        )
        .await?;
        total += bytes.len();
        ensure!(
            total <= 256 * 1024 * 1024,
            "Source download budget exceeded"
        );
        files.insert(path, hash(&bytes));
        tables.insert((*name).into(), serde_json::from_slice(&bytes)?);
    }
    let archive_path = "BinOutput/Talent/AvatarTalents.zip";
    let bytes=fetch(&client,&format!("https://gitlab.com/api/v4/projects/Dimbreath%2Fanimegamedata2/repository/archive.zip?sha={rev}&path=BinOutput%2FTalent%2FAvatarTalents"),8*1024*1024).await?;
    total += bytes.len();
    files.insert(archive_path.into(), hash(&bytes));
    let (configs, hashes) =
        tokio::task::spawn_blocking(move || super::talents::archive(&bytes)).await??;
    files.extend(hashes);
    tables.insert("AvatarTalentConfigs".into(), configs);
    let needed: BTreeSet<String> = tables
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .flat_map(|row| {
            [
                "nameTextMapHash",
                "titleTextMapHash",
                "descTextMapHash",
                "typeDescTextMapHash",
            ]
            .into_iter()
            .filter_map(|key| row.get(key).map(Value::to_string))
        })
        .collect();
    for prefix in ["TextMapRU", "TextMap_MediumRU"] {
        for part in 0..2 {
            let path = format!("TextMap/{prefix}_{part}.json");
            let bytes = fetch(
                &client,
                &format!("{SOURCE}/-/raw/{rev}/{path}"),
                64 * 1024 * 1024,
            )
            .await?;
            total += bytes.len();
            ensure!(
                total <= 256 * 1024 * 1024,
                "Source download budget exceeded"
            );
            files.insert(path, hash(&bytes));
            let wanted = needed.clone();
            let selected =
                tokio::task::spawn_blocking(move || select_texts(&bytes, &wanted)).await??;
            texts.extend(selected);
        }
    }
    // Parsing/validation is CPU work; do not block the live delivery runtime.
    tokio::task::spawn_blocking(move || compile(&rev, &tables, &texts, files)).await?
}
async fn fetch(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut r = client.get(url).send().await?.error_for_status()?;
    ensure!(
        r.content_length().is_none_or(|n| n <= limit as u64),
        "Source file size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = r.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= limit, "Source file size limit");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Localization pools are large. Keep only hashes referenced by the downloaded tables.
fn select_texts(bytes: &[u8], needed: &BTreeSet<String>) -> Result<Map<String, Value>> {
    struct Pick<'a>(&'a BTreeSet<String>);
    impl<'de> serde::de::Visitor<'de> for Pick<'_> {
        type Value = Map<String, Value>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a localization map")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut selected = Map::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if self.0.contains(&key) {
                    selected.insert(key, Value::String(value));
                }
            }
            Ok(selected)
        }
    }
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let selected = serde::Deserializer::deserialize_map(&mut parser, Pick(needed))?;
    parser.end()?;
    Ok(selected)
}
