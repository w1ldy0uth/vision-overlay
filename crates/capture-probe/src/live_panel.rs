//! Read-only projection of the capture event stream for a local live panel.
use crate::event_format::{
    achievement_action, achievement_display, adventure_action, adventure_display, event_timestamp,
    item_action, item_description,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[derive(Deserialize)]
struct Material {
    item_id: u32,
    guid: String,
    count: u32,
}

struct Stock {
    id: u32,
    initial: u32,
    count: u32,
    updates: u64,
    updated_at: Option<String>,
    updated_sequence: u64,
}

pub struct Panel {
    pub revision: u64,
    pub status: String,
    pub detail: String,
    session: String,
    mode: String,
    ready: bool,
    tracking: bool,
    sequence: u64,
    stocks: HashMap<String, Stock>,
    items: BTreeMap<String, Value>,
    profile_session: String,
    trial_guids: HashSet<String>,
    achievement_session: String,
    achievements: BTreeMap<u64, Value>,
    achievement_times: BTreeMap<u64, Option<String>>,
    achievement_catalog: BTreeMap<String, Value>,
    adventure: Option<Value>,
    adventure_time: Option<String>,
    currencies: BTreeMap<u32, Value>,
    friends: BTreeMap<String, Value>,
    progress: BTreeMap<String, Value>,
    talents: BTreeMap<String, Value>,
    constellations: BTreeMap<String, Value>,
    equipment: BTreeMap<String, Value>,
    fight_props: BTreeMap<String, Value>,
    item_times: HashMap<String, Option<String>>,
    friend_times: HashMap<String, Option<String>>,
    history: VecDeque<Value>,
    total_changes: u64,
    issues: u64,
    names: HashMap<String, String>,
    skills: HashMap<String, String>,
}

impl Panel {
    pub fn new(names: HashMap<String, String>) -> Self {
        Self {
            revision: 1,
            status: "waiting".into(),
            detail: "Ожидание исходного снимка".into(),
            session: String::new(),
            mode: "live".into(),
            ready: false,
            tracking: false,
            sequence: 0,
            stocks: HashMap::new(),
            items: BTreeMap::new(),
            profile_session: String::new(),
            trial_guids: HashSet::new(),
            achievement_session: String::new(),
            achievements: BTreeMap::new(),
            achievement_times: BTreeMap::new(),
            achievement_catalog: BTreeMap::new(),
            adventure: None,
            adventure_time: None,
            currencies: BTreeMap::new(),
            friends: BTreeMap::new(),
            progress: BTreeMap::new(),
            talents: BTreeMap::new(),
            constellations: BTreeMap::new(),
            equipment: BTreeMap::new(),
            fight_props: BTreeMap::new(),
            item_times: HashMap::new(),
            friend_times: HashMap::new(),
            history: VecDeque::new(),
            total_changes: 0,
            issues: 0,
            names,
            skills: HashMap::new(),
        }
    }
    pub fn with_achievements(mut self, catalog: BTreeMap<String, Value>) -> Self {
        self.achievement_catalog = catalog;
        self
    }
    pub fn with_skills(mut self, skills: HashMap<String, String>) -> Self {
        self.skills = skills;
        self
    }
    pub fn unavailable(&mut self, status: &str, detail: &str) {
        if self.status != status || self.detail != detail {
            self.status = status.into();
            self.detail = detail.into();
            self.revision += 1;
        }
        self.ready = false;
    }
    pub fn stopped(&self) -> bool {
        matches!(self.status.as_str(), "stopped" | "error")
    }
    pub fn heartbeat_expired(&mut self) {
        if self.status == "active" {
            self.status = "stale".into();
            self.detail =
                "Нет событий захвата более 8 секунд. Последние количества могут устареть.".into();
            self.revision += 1;
        }
    }
    fn insert_character(&mut self, profile: &Value, timestamp: Option<String>) -> Result<()> {
        let guid = profile["guid"].as_str().context("Missing character GUID")?;
        let prop = |key: &str| {
            profile["props"][key]["ival"]
                .as_str()
                .and_then(|v| v.parse::<u32>().ok())
        };
        self.equipment
            .insert(guid.into(), profile["equip_guids"].clone());
        self.fight_props
            .insert(guid.into(), profile["fight_props"].clone());
        self.constellations.insert(guid.into(),json!({"avatar_id":profile["avatar_id"],"skill_depot_id":profile["skill_depot_id"],"talent_ids":profile["talent_ids"]}));
        self.talents.insert(guid.into(),json!({"skill_levels":profile["skill_levels"],"extra_skill_levels":profile["extra_skill_levels"]}));
        self.progress.insert(guid.into(),json!({"guid":guid,"avatar_id":profile["avatar_id"],"level":prop("4001"),"exp":prop("1001"),"ascension":prop("1002")}));
        self.friend_times.insert(guid.into(), timestamp.clone());
        if profile["friendship"].is_object() {
            let guid = profile["guid"].as_str().context("Missing character GUID")?;
            self.friends.insert(guid.to_owned(),json!({"guid":guid,"avatar_id":profile["avatar_id"],"level":profile["friendship"]["level"],"exp":profile["friendship"]["exp"]}));
            self.friend_times.insert(guid.into(), timestamp.clone());
        }
        Ok(())
    }
    pub fn apply(&mut self, event: &Value) -> Result<()> {
        ensure!(event["schema_version"] == 1, "Unsupported event schema");
        let sequence = event["sequence"]
            .as_u64()
            .context("Missing event sequence")?;
        ensure!(sequence > self.sequence, "Event sequence must increase");
        self.sequence = sequence;
        if self.ready && self.status == "stale" {
            self.status = "active".into();
            self.detail = "Учёт активен · получение событий возобновилось".into();
            self.revision += 1;
        }
        let kind = event["event"].as_str().context("Missing event kind")?;
        let data = &event["data"];
        let timestamp = event_timestamp(event);
        if kind == "achievement_snapshot" {
            let session = event["session"]
                .as_str()
                .context("Missing achievement session")?;
            let mut records = BTreeMap::new();
            for row in data["records"].as_array().context("Missing achievements")? {
                let id = row["achievement_id"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .context("Invalid achievement ID")?;
                ensure!(
                    records.insert(id, row.clone()).is_none(),
                    "Duplicate achievement"
                );
            }
            self.achievements = records;
            self.achievement_session = session.into();
            self.achievement_times = self
                .achievements
                .keys()
                .map(|id| (*id, timestamp.clone()))
                .collect();
            self.revision += 1;
            return Ok(());
        }
        if kind == "achievement_changed" {
            ensure!(
                self.ready && event["session"] == self.achievement_session,
                "Achievement change without matching active baseline"
            );
            let id = data["achievement_id"]
                .as_u64()
                .context("Missing achievement ID")?;
            ensure!(
                self.achievements.get(&id).unwrap_or(&Value::Null) == &data["previous"]
                    && data["current"]["achievement_id"] == id,
                "Inconsistent achievement history"
            );
            self.achievements.insert(id, data["current"].clone());
            self.achievement_times.insert(id, timestamp.clone());
            let catalog = self.achievement_catalog.get(&id.to_string());
            self.history.push_front(json!({"item_id":id,"achievement_id":id,"name":catalog.map(|c|&c["name"]),"previous_display":achievement_display(&data["previous"]),"current_display":achievement_display(&data["current"]),"action":achievement_action(data),"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            self.history.truncate(200);
            self.total_changes += 1;
            self.revision += 1;
            return Ok(());
        }
        if matches!(
            kind,
            "currency_snapshot" | "character_snapshot" | "adventure_snapshot"
        ) {
            let session = event["session"]
                .as_str()
                .context("Missing profile session")?;
            if self.profile_session != session {
                if self.achievement_session != session {
                    self.achievements.clear();
                    self.achievement_times.clear();
                    self.achievement_session.clear();
                }
                self.adventure = None;
                self.adventure_time = None;
                self.currencies.clear();
                self.friends.clear();
                self.progress.clear();
                self.talents.clear();
                self.constellations.clear();
                self.equipment.clear();
                self.fight_props.clear();
                self.trial_guids.clear();
                self.friend_times.clear();
                self.profile_session = session.into();
            }
            if kind == "adventure_snapshot" {
                ensure!(
                    data["current"]["rank"]
                        .as_u64()
                        .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX))
                        && data["current"]["exp"]
                            .as_u64()
                            .is_some_and(|n| n <= u64::from(u32::MAX)),
                    "Invalid adventure baseline"
                );
                self.adventure = Some(data["current"].clone());
                self.adventure_time = timestamp;
            } else if kind == "currency_snapshot" {
                let id = data["item_id"].as_u64().context("Missing currency ID")?;
                let count = data["count"]
                    .as_i64()
                    .context("Invalid currency snapshot")?;
                ensure!(
                    matches!(id, 201 | 202) && (id != 202 || count >= 0),
                    "Invalid currency snapshot"
                );
                self.currencies.insert(id as u32,json!({"item_id":id,"name":if id==201 {"Примогемы"}else{"Мора"},"initial_count":count,"count":count,"updated_at":timestamp}));
            } else {
                self.friends.clear();
                self.progress.clear();
                self.talents.clear();
                self.constellations.clear();
                self.equipment.clear();
                self.fight_props.clear();
                self.trial_guids.clear();
                self.friend_times.clear();
                for profile in data["profiles"]
                    .as_array()
                    .context("Missing character profiles")?
                {
                    if profile["trial_info_present"] == true {
                        self.trial_guids.insert(
                            profile["guid"]
                                .as_str()
                                .context("Missing trial GUID")?
                                .into(),
                        );
                        continue;
                    }
                    self.insert_character(profile, timestamp.clone())?;
                    self.equipment.insert(
                        profile["guid"].as_str().unwrap().into(),
                        profile["equip_guids"].clone(),
                    );
                }
            }
            self.revision += 1;
            return Ok(());
        }
        if matches!(
            kind,
            "adventure_progress_changed"
                | "constellation_changed"
                | "constellation_depot_changed"
                | "character_added"
                | "currency_changed"
                | "friendship_changed"
                | "character_progress_changed"
                | "fight_properties_changed"
                | "equipment_changed"
                | "talent_changed"
        ) {
            ensure!(
                (self.ready || (self.tracking && self.status == "waiting"))
                    && event["session"] == self.profile_session,
                "Profile change without matching active baseline"
            );
            if kind != "currency_changed"
                && data["guid"]
                    .as_str()
                    .is_some_and(|guid| self.trial_guids.contains(guid))
            {
                return Ok(());
            }
            if kind == "adventure_progress_changed" {
                ensure!(
                    self.adventure.as_ref() == Some(&data["previous"]),
                    "Inconsistent adventure history"
                );
                let old_rank = data["previous"]["rank"]
                    .as_u64()
                    .context("Missing previous Adventure Rank")?;
                let rank = data["current"]["rank"]
                    .as_u64()
                    .filter(|n| *n >= old_rank && *n <= u64::from(u32::MAX))
                    .context("Invalid Adventure Rank")?;
                ensure!(
                    data["current"]["exp"].is_null()
                        || data["current"]["exp"]
                            .as_u64()
                            .is_some_and(|n| n <= u64::from(u32::MAX)),
                    "Invalid Adventure EXP"
                );
                let delta = if rank == old_rank {
                    data["previous"]["exp"]
                        .as_u64()
                        .zip(data["current"]["exp"].as_u64())
                        .and_then(|(a, b)| b.checked_sub(a))
                } else {
                    None
                };
                ensure!(
                    data["exp_delta"] == json!(delta),
                    "Invalid Adventure EXP delta"
                );
                self.adventure = Some(data["current"].clone());
                self.adventure_time = timestamp.clone();
                self.history.push_front(json!({"item_id":if rank==old_rank{10014}else{10013},"name":if rank==old_rank{"Опыт приключений"}else{"Ранг приключений"},"previous_display":adventure_display(&data["previous"]),"current_display":adventure_display(&data["current"]),"action":adventure_action(data),"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "constellation_depot_changed" {
                let guid = data["guid"]
                    .as_str()
                    .context("Missing constellation GUID")?;
                let old = self
                    .constellations
                    .get(guid)
                    .context("Missing constellation baseline")?;
                ensure!(
                    old["avatar_id"] == data["avatar_id"]
                        && matches!(data["avatar_id"].as_u64(), Some(10000005 | 10000007))
                        && old["skill_depot_id"] != data["skill_depot_id"]
                        && data["current"]["completeness"] == "partial",
                    "Invalid Traveler constellation observation"
                );
                self.history.push_front(json!({"item_id":data["avatar_id"],"guid":guid,"name":self.names.get(&data["avatar_id"].to_string()),"previous_display":"—","current_display":"Активировано","action":"Созвездие другого элемента (нужен снимок элемента)","timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "constellation_changed" {
                let guid = data["guid"]
                    .as_str()
                    .context("Missing constellation GUID")?;
                let old = self
                    .constellations
                    .get(guid)
                    .context("Missing constellation baseline")?;
                ensure!(
                    old == &data["previous"]
                        && old["avatar_id"] == data["avatar_id"]
                        && data["current"]["avatar_id"] == data["avatar_id"]
                        && old["skill_depot_id"] == data["skill_depot_id"]
                        && data["current"]["skill_depot_id"] == data["skill_depot_id"],
                    "Inconsistent constellation identity or baseline"
                );
                let before = old["talent_ids"]
                    .as_array()
                    .context("Missing constellation list")?;
                let after = data["current"]["talent_ids"]
                    .as_array()
                    .context("Missing updated constellation list")?;
                ensure!(
                    after.len() <= 6
                        && after.len() == before.len() + 1
                        && after.starts_with(before)
                        && after.last() == Some(&data["talent_id"])
                        && !before.contains(&data["talent_id"])
                        && data["talent_id"]
                            .as_u64()
                            .is_some_and(|id| id > 0 && id <= u64::from(u32::MAX))
                        && data["previous_count"] == before.len()
                        && data["count"] == after.len(),
                    "Invalid constellation activation"
                );
                self.constellations
                    .insert(guid.into(), data["current"].clone());
                self.friend_times.insert(guid.into(), timestamp.clone());
                self.history.push_front(json!({"item_id":data["avatar_id"],"guid":guid,"name":self.names.get(&data["avatar_id"].to_string()),"previous_display":format!("C{}",data["previous_count"]),"current_display":format!("C{}",data["count"]),"action":"Созвездие активировано","timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "character_added" {
                let profile = &data["profile"];
                let guid = data["guid"]
                    .as_str()
                    .context("Missing added character GUID")?;
                ensure!(
                    profile["guid"] == guid
                        && profile["avatar_id"] == data["avatar_id"]
                        && profile["avatar_id"].as_u64().is_some_and(|id| id > 0),
                    "Invalid added character identity"
                );
                if profile["trial_info_present"] == true {
                    return Ok(());
                }
                if let Some(existing) = self.progress.get(guid) {
                    ensure!(
                        existing["avatar_id"] == profile["avatar_id"],
                        "Conflicting added character GUID"
                    );
                    return Ok(());
                }
                ensure!(
                    profile["props"]["4001"]["ival"]
                        .as_str()
                        .and_then(|s| s.parse::<u32>().ok())
                        .is_some_and(|n| n > 0),
                    "Invalid added character level"
                );
                self.insert_character(profile, timestamp.clone())?;
                self.history.push_front(json!({"item_id":data["avatar_id"],"guid":guid,"name":self.names.get(&data["avatar_id"].to_string()),"previous_display":"—","current_display":format!("Ур. {}",profile["props"]["4001"]["ival"].as_str().unwrap()),"action":"Получен персонаж","timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "currency_changed" {
                let id = data["item_id"].as_u64().context("Missing currency ID")?;
                ensure!(matches!(id, 201 | 202), "Unknown currency");
                let row = self
                    .currencies
                    .get_mut(&(id as u32))
                    .context("Missing currency baseline")?;
                ensure!(
                    row["count"] == data["previous_count"],
                    "Inconsistent currency history"
                );
                let count = data["count"].as_i64().context("Invalid currency count")?;
                ensure!(id != 202 || count >= 0, "Negative Mora balance");
                let before = data["previous_count"]
                    .as_i64()
                    .context("Invalid previous currency count")?;
                ensure!(
                    data["delta"].as_i64().map(i128::from)
                        == Some(i128::from(count) - i128::from(before)),
                    "Invalid currency delta"
                );
                row["count"] = json!(count);
                row["updated_at"] = json!(timestamp);
                self.history.push_front(json!({"item_id":id,"name":if id==201 {"Примогемы"}else{"Мора"},"previous_count":before,"count":count,"delta":data["delta"],"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "fight_properties_changed" {
                let guid = data["guid"]
                    .as_str()
                    .context("Missing fight property character")?;
                ensure!(
                    self.progress
                        .get(guid)
                        .is_some_and(|p| p["avatar_id"] == data["avatar_id"]),
                    "Fight property character mismatch"
                );
                let values = self
                    .fight_props
                    .get_mut(guid)
                    .context("Missing fight property baseline")?;
                let updates = data["current"]
                    .as_object()
                    .context("Missing fight properties")?;
                for (id, value) in updates {
                    ensure!(
                        values[id] == data["previous"][id]
                            && value.as_f64().is_some_and(f64::is_finite),
                        "Fight property previous mismatch"
                    );
                }
                if !values.is_object() {
                    *values = json!({});
                }
                for (id, value) in updates {
                    values[id] = value.clone();
                }
                self.friend_times.insert(guid.into(), timestamp.clone());
                self.history.push_front(json!({"item_id":data["avatar_id"],"name":self.names.get(&data["avatar_id"].to_string()),"action":"Боевые характеристики","previous_display":data["previous"].to_string(),"current_display":data["current"].to_string(),"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "equipment_changed" {
                let updates = data["updates"]
                    .as_array()
                    .context("Missing equipment updates")?;
                for update in updates {
                    let guid = update["guid"]
                        .as_str()
                        .context("Missing equipment character")?;
                    ensure!(
                        self.equipment.get(guid) == Some(&update["previous"])
                            && self
                                .progress
                                .get(guid)
                                .is_some_and(|p| p["avatar_id"] == update["avatar_id"]),
                        "Equipment previous mismatch"
                    );
                }
                for update in updates {
                    let guid = update["guid"].as_str().unwrap();
                    self.equipment
                        .insert(guid.into(), update["current"].clone());
                    self.friend_times.insert(guid.into(), timestamp.clone());
                }
                self.history.push_front(json!({"item_id":data["avatar_id"],"name":self.names.get(&data["avatar_id"].to_string()),"action":"Смена экипировки","previous_display":"Предыдущая экипировка","current_display":if data["item_guid"].is_null(){"Пустой слот".into()}else{self.names.get(&data["item_id"].to_string()).cloned().unwrap_or_else(||format!("Предмет ID {}",data["item_id"]))},"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "talent_changed" {
                let guid = data["guid"].as_str().context("Missing talent GUID")?;
                let profile = self
                    .progress
                    .get(guid)
                    .context("Missing talent character")?;
                ensure!(
                    profile["avatar_id"] == data["avatar_id"],
                    "Invalid talent character identity"
                );
                let skill = data["skill_id"]
                    .as_u64()
                    .filter(|id| *id > 0 && *id <= u64::from(u32::MAX))
                    .context("Invalid skill ID")?;
                let level = data["level"]
                    .as_u64()
                    .filter(|level| *level > 0 && *level <= u64::from(u32::MAX))
                    .context("Invalid talent level")?;
                let talents = self
                    .talents
                    .get_mut(guid)
                    .context("Missing talents baseline")?;
                let key = skill.to_string();
                ensure!(
                    talents["skill_levels"][&key].is_u64()
                        && talents["skill_levels"][&key] == data["previous_level"],
                    "Inconsistent talent history"
                );
                talents["skill_levels"][&key] = json!(level);
                self.friend_times.insert(guid.into(), timestamp.clone());
                let name = self
                    .skills
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| format!("Талант ID {skill}"));
                self.history.push_front(json!({"item_id":data["avatar_id"],"skill_id":skill,"name":self.names.get(&data["avatar_id"].to_string()),"previous_display":format!("Ур. {}",data["previous_level"]),"current_display":format!("Ур. {level}"),"action":format!("Талант: {name}"),"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else if kind == "character_progress_changed" {
                let guid = data["guid"].as_str().context("Missing character GUID")?;
                ensure!(
                    self.progress.get(guid) == Some(&data["previous"]),
                    "Inconsistent character progression history"
                );
                ensure!(
                    data["current"]["guid"] == guid
                        && data["current"]["avatar_id"] == data["avatar_id"],
                    "Invalid character progression identity"
                );
                self.progress.insert(guid.into(), data["current"].clone());
                self.friend_times.insert(guid.into(), timestamp.clone());
                let action = if data["previous"]["level"] != data["current"]["level"] {
                    format!(
                        "Уровень: {} → {}",
                        data["previous"]["level"], data["current"]["level"]
                    )
                } else if data["previous"]["ascension"] != data["current"]["ascension"] {
                    "Возвышение персонажа".into()
                } else {
                    data["exp_delta"]
                        .as_i64()
                        .map(|n| format!("Опыт персонажа: {n:+}"))
                        .unwrap_or_else(|| "Опыт персонажа обновлён".into())
                };
                self.history.push_front(json!({"item_id":data["avatar_id"],"name":self.names.get(&data["avatar_id"].to_string()),
                    "previous_display":format!("Ур. {} · {} опыта",data["previous"]["level"],data["previous"]["exp"]),
                    "current_display":format!("Ур. {} · {} опыта",data["current"]["level"],data["current"]["exp"]),"action":action,
                    "timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            } else {
                let guid = data["guid"].as_str().context("Missing friendship GUID")?;
                ensure!(
                    self.friends.get(guid) == Some(&data["previous"]),
                    "Inconsistent friendship history"
                );
                ensure!(
                    data["current"]["guid"] == guid
                        && data["current"]["avatar_id"] == data["avatar_id"],
                    "Invalid friendship identity"
                );
                self.friends.insert(guid.into(), data["current"].clone());
                self.friend_times.insert(guid.into(), timestamp.clone());
                self.history.push_front(json!({"item_id":data["avatar_id"],"name":self.names.get(&data["avatar_id"].to_string()),
                    "previous_display":format!("Ур. {} · {} опыта",data["previous"]["level"],data["previous"]["exp"]),
                    "current_display":format!("Ур. {} · {} опыта",data["current"]["level"],data["current"]["exp"]),
                    "action":data["exp_delta"].as_i64().map(|n|format!("Дружба: {n:+} опыта")).unwrap_or_else(||"Изменение уровня дружбы".into()),
                    "timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            }
            self.history.truncate(200);
            self.total_changes += 1;
            self.revision += 1;
            return Ok(());
        }
        if kind == "source_config" {
            self.tracking = data["track_materials"] == true;
            self.mode = data["mode"].as_str().unwrap_or("live").into();
            self.revision += 1;
            return Ok(());
        }
        if kind == "inventory_snapshot" {
            let materials: Vec<Material> = serde_json::from_value(data["materials"].clone())?;
            let mut stocks = HashMap::new();
            for material in materials {
                ensure!(
                    material.guid.parse::<u64>().is_ok_and(|n| n > 0) && material.item_id > 0,
                    "Invalid snapshot identity"
                );
                ensure!(
                    stocks
                        .insert(
                            material.guid,
                            Stock {
                                id: material.item_id,
                                initial: material.count,
                                count: material.count,
                                updates: 0,
                                updated_at: timestamp.clone(),
                                updated_sequence: sequence
                            }
                        )
                        .is_none(),
                    "Duplicate snapshot GUID"
                );
            }
            self.session = event["session"]
                .as_str()
                .context("Missing snapshot session")?
                .into();
            self.stocks = stocks;
            self.item_times.clear();
            for row in data["items"].as_array().into_iter().flatten() {
                if let Some(guid) = row["guid"].as_str() {
                    self.item_times.insert(guid.into(), timestamp.clone());
                }
            }
            self.items = data["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| {
                    (
                        row["guid"].as_str().unwrap_or_default().to_owned(),
                        row.clone(),
                    )
                })
                .collect();
            self.history.clear();
            self.total_changes = 0;
            self.issues = 0;
            self.ready = false;
            self.status = "waiting".into();
            self.detail = "Снимок получен. Ожидание подтверждения учёта".into();
            self.revision += 1;
            return Ok(());
        }
        if kind == "status" {
            let state = data["state"].as_str().unwrap_or("");
            match state {
                "snapshot_received_experimental_realtime_tracking"|"snapshot_received_offline_material_tracking" => {
                    ensure!(event["session"]==self.session && !self.stocks.is_empty(),"Tracking status without matching snapshot");
                    self.ready=true; self.status="active".into(); self.detail="Учёт активен · материалы обновляются автоматически".into(); self.revision+=1;
                }
                "stale" => self.unavailable("stale","Игровое соединение разорвано. Ожидание нового снимка"),
                "waiting_for_snapshot"|"waiting_for_session" => self.unavailable("waiting","Ожидание исходного снимка"),
                state if state.starts_with("realtime_tracking_paused") || state=="capture_incomplete_queue_overflow" => self.unavailable("stale","Учёт приостановлен. Последние количества могут устареть; проверьте терминал захвата"),
                "source_error"|"decoder_failed_key_recovery" => self.unavailable("error","Ошибка захвата. Проверьте терминал"),
                state if state.starts_with("stopped") && self.status!="error" => {
                    // A normal stop freezes the last state; an error must remain visible.
                        let detail=if self.ready {"Захват завершён. Показаны последние сохранённые количества"} else {"Захват завершён. Актуальность последних количеств не подтверждена"};
                        self.unavailable("stopped",detail);
                }
                _=>{}
            }
            return Ok(());
        }
        if kind == "item_changed" {
            ensure!(
                self.ready && event["session"] == self.session,
                "Item update without matching active snapshot"
            );
            let change = &data["change"];
            let guid = change["guid"].as_str().context("Missing item GUID")?;
            ensure!(
                guid.parse::<u64>().is_ok_and(|n| n > 0),
                "Invalid item GUID"
            );
            let previous = self.items.get(guid).cloned().unwrap_or(Value::Null);
            ensure!(
                previous == change["previous"],
                "Inconsistent equipment history"
            );
            let row = &change["item"];
            let removed = change["removed"] == true;
            ensure!(
                removed || (row["guid"] == guid && row["item_id"] == change["item_id"]),
                "Invalid equipment identity"
            );
            let before = previous["count"].as_u64().unwrap_or(0);
            let count = if removed {
                0
            } else {
                row["count"].as_u64().unwrap_or(1)
            };
            if removed {
                self.items.remove(guid);
                self.item_times.remove(guid);
            } else {
                self.items.insert(guid.to_owned(), row.clone());
                self.item_times.insert(guid.into(), timestamp.clone());
            }
            self.history.push_front(json!({"item_id":change["item_id"],"guid":guid,
                "name":self.names.get(&change["item_id"].to_string()),"previous_count":before,"count":count,
                "delta":count as i64-before as i64,"action":item_action(change),"previous_display":item_description(&previous),"current_display":item_description(row),
                "timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
            self.history.truncate(200);
            self.total_changes += 1;
            self.revision += 1;
            return Ok(());
        }
        if !matches!(
            kind,
            "material_changed" | "material_update_ignored" | "material_update_rejected"
        ) {
            return Ok(());
        }
        ensure!(
            event["session"] == self.session,
            "Material event belongs to another session"
        );
        if kind != "material_changed" {
            self.issues += 1;
            self.revision += 1;
            if kind == "material_update_rejected" {
                self.unavailable("stale", "Получено некорректное обновление материала");
            }
            return Ok(());
        }
        ensure!(self.ready, "Material change without an active snapshot");
        let change = &data["change"];
        let guid = change["guid"].as_str().context("Missing material GUID")?;
        let id = change["item_id"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .context("Invalid item ID")?;
        let count = change["count"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .context("Invalid count")?;
        let before = change["previous_count"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .context("Invalid previous count")?;
        ensure!(
            change["delta"] == i64::from(count) - i64::from(before),
            "Invalid quantity delta"
        );
        ensure!(
            change["removed"] != true || count == 0,
            "Invalid deletion quantity"
        );
        if let Some(stock) = self.stocks.get(guid) {
            ensure!(
                stock.id == id && stock.count == before && change["new_entry"] != true,
                "Inconsistent material history"
            );
        } else {
            ensure!(
                guid.parse::<u64>().is_ok_and(|n| n > 0)
                    && change["new_entry"] == true
                    && change["removed"] != true
                    && before == 0
                    && count > 0,
                "Unmarked material creation"
            );
        }
        let stock = self.stocks.entry(guid.into()).or_insert(Stock {
            id,
            initial: 0,
            count: 0,
            updates: 0,
            updated_at: None,
            updated_sequence: 0,
        });
        stock.count = count;
        stock.updates += 1;
        stock.updated_at = timestamp.clone();
        stock.updated_sequence = sequence;
        self.history.push_front(json!({"item_id":id,"name":self.names.get(&id.to_string()),"previous_count":before,"count":count,
            "delta":change["delta"],"timestamp":timestamp,"source_elapsed_ms":event["source_elapsed_ms"]}));
        self.history.truncate(200);
        self.total_changes += 1;
        self.revision += 1;
        Ok(())
    }
    pub fn report(&self) -> Value {
        // A recreated stack can have another GUID. The panel shows total owned by item ID.
        let mut totals = BTreeMap::<u32, (u64, u64, u64)>::new();
        let mut times = BTreeMap::<u32, (u64, Option<String>)>::new();
        for stock in self.stocks.values() {
            let total = totals.entry(stock.id).or_default();
            total.0 += u64::from(stock.initial);
            total.1 += u64::from(stock.count);
            total.2 += stock.updates;
            let latest = times.entry(stock.id).or_default();
            if stock.updated_sequence >= latest.0 {
                *latest = (stock.updated_sequence, stock.updated_at.clone());
            }
        }
        let rows:Vec<_>=totals.into_iter().map(|(id,(initial,count,updates))|json!({"item_id":id,"name":self.names.get(&id.to_string()),
            "initial_count":initial,"count":count,"delta":count as i64-initial as i64,"updates":updates,"updated_at":times.get(&id).map(|(_,stamp)|stamp)})).collect();
        let characters:Vec<_>=self.progress.values().map(|p| {
            let guid=p["guid"].as_str().unwrap_or_default();let friendship=self.friends.get(guid);
            let skills=self.talents.get(guid);
            let constellation=self.constellations.get(guid);
            let talents:Vec<_>=skills.and_then(|t|t["skill_levels"].as_object()).into_iter().flat_map(|levels|levels.iter()).map(|(id,level)|json!({"skill_id":id.parse::<u32>().ok(),"name":self.skills.get(id),"level":level})).collect();
            json!({"constellation":constellation.and_then(|c|c["talent_ids"].as_array()).map(Vec::len),"talent_ids":constellation.map(|c|&c["talent_ids"]),"talents":talents,"extra_skill_levels":skills.map(|t|&t["extra_skill_levels"]),"guid":guid,"avatar_id":p["avatar_id"],"name":self.names.get(&p["avatar_id"].to_string()),
                "fight_props":self.fight_props.get(guid),"equip_guids":self.equipment.get(guid),"character_level":p["level"],"character_exp":p["exp"],"ascension":p["ascension"],
                "level":friendship.map(|f|&f["level"]),"exp":friendship.map(|f|&f["exp"]),"updated_at":self.friend_times.get(guid)})
        }).collect();
        let achievements: Vec<_> = self
            .achievements
            .iter()
            .map(|(id, row)| {
                let mut row = row.clone();
                let catalog = self.achievement_catalog.get(&id.to_string());
                row["name"] = catalog
                    .map(|c| c["name"].clone())
                    .unwrap_or_else(|| json!(format!("Достижение ID {id}")));
                row["category"] = catalog
                    .map(|c| c["category"].clone())
                    .unwrap_or(Value::Null);
                row["progress_display"] = json!(achievement_display(&row));
                row["updated_at"] = json!(self.achievement_times.get(id));
                row
            })
            .collect();
        json!({"adventure":self.adventure.as_ref().map(|a|json!({"rank":a["rank"],"exp":a["exp"],"updated_at":self.adventure_time})),"achievements":achievements,"revision":self.revision,"status":self.status,"detail":self.detail,"session":self.session,"mode":self.mode,
            "rows":rows,"currencies":self.currencies.values().collect::<Vec<_>>(),"characters":characters,"items":self.items.values().map(|item| { let mut row=item.clone(); row["name"]=json!(self.names.get(&item["item_id"].to_string()));row["updated_at"]=json!(self.item_times.get(item["guid"].as_str().unwrap_or_default())); row }).collect::<Vec<_>>(),"history":self.history,"history_total":self.total_changes,"issues":self.issues,"tracking_enabled":self.tracking})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(n: u64, kind: &str, data: Value) -> Value {
        json!({"schema_version":1,"sequence":n,"event":kind,"session":"2","data":data})
    }
    fn ready() -> Panel {
        let mut panel = Panel::new(HashMap::new());
        panel
            .apply(&event(
                1,
                "inventory_snapshot",
                json!({"materials":[{"item_id":104122,"guid":"18446744073709551615","count":3}]}),
            ))
            .unwrap();
        panel
            .apply(&event(
                2,
                "status",
                json!({"state":"snapshot_received_experimental_realtime_tracking"}),
            ))
            .unwrap();
        panel
    }
    #[test]
    fn fight_property_patch_updates_report_preserving_other_totals() {
        let mut panel = ready();
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000137,"fight_props":{"2000":3700.0,"2001":464.0,"2002":211.0},"equip_guids":[]}]}))).unwrap();
        panel.apply(&event(4,"fight_properties_changed",json!({"guid":"42","avatar_id":10000137,"previous":{"2001":464.0,"40":null},"current":{"2001":787.0,"40":0.0}}))).unwrap();
        let stats = &panel.report()["characters"][0]["fight_props"];
        assert_eq!(stats["2001"], 787.0);
        assert_eq!(stats["2000"], 3700.0);
        assert_eq!(stats["40"], 0.0);
    }
    #[test]
    fn equipment_transfer_updates_both_rows_and_history() {
        let mut panel = ready();
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000114,"equip_guids":["101"]},{"guid":"43","avatar_id":10000089,"equip_guids":["202"]}]}))).unwrap();
        let d = json!({"guid":"42","avatar_id":10000114,"equip_type":2,"item_id":32524,"item_guid":"202","updates":[{"guid":"42","avatar_id":10000114,"previous":["101"],"current":["101","202"]},{"guid":"43","avatar_id":10000089,"previous":["202"],"current":[]}]});
        panel.apply(&event(4, "equipment_changed", d)).unwrap();
        assert_eq!(panel.equipment["42"], json!(["101", "202"]));
        assert_eq!(panel.equipment["43"], json!([]));
        assert_eq!(panel.report()["history"][0]["action"], "Смена экипировки");
        let mut foreign = event(5, "equipment_changed", json!({"updates":[]}));
        foreign["session"] = json!("other");
        assert!(panel.apply(&foreign).is_err());
    }
    #[test]
    fn adventure_projection_handles_split_rank_exp_and_clears_other_session() {
        let mut panel = ready();
        let initial = json!({"rank":11,"exp":1515});
        panel
            .apply(&event(3, "adventure_snapshot", json!({"current":initial})))
            .unwrap();
        let rank = json!({"rank":12,"exp":null});
        let mut e = event(
            4,
            "adventure_progress_changed",
            json!({"previous":initial,"current":rank,"exp_delta":null}),
        );
        e["timestamp"] = json!("2026-11-12T16:00:00+03:00");
        panel.apply(&e).unwrap();
        assert!(panel.report()["adventure"]["exp"].is_null());
        panel
            .apply(&event(
                5,
                "adventure_progress_changed",
                json!({"previous":rank,"current":{"rank":12,"exp":20},"exp_delta":null}),
            ))
            .unwrap();
        let report = panel.report();
        assert_eq!(report["adventure"]["rank"], 12);
        assert_eq!(report["adventure"]["exp"], 20);
        assert_eq!(report["history_total"], 2);
        assert_eq!(report["history"][1]["timestamp"], e["timestamp"]);
        let mut bad = event(
            6,
            "adventure_progress_changed",
            json!({"previous":{"rank":12,"exp":20},"current":{"rank":12,"exp":50},"exp_delta":999}),
        );
        assert!(panel.apply(&bad).is_err());
        assert_eq!(panel.report()["adventure"]["exp"], 20);
        bad["sequence"] = json!(7);
        bad["session"] = json!("other");
        assert!(panel.apply(&bad).is_err());
        let mut baseline = event(8, "currency_snapshot", json!({"item_id":201,"count":1}));
        baseline["session"] = json!("other");
        panel.apply(&baseline).unwrap();
        assert!(panel.report()["adventure"].is_null());
    }
    #[test]
    fn constellation_changes_update_current_roster_history_and_timestamp() {
        let mut panel = ready();
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[],"skill_levels":{"10382":1},"extra_skill_levels":{}}]}))).unwrap();
        let previous = json!({"avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[]});
        let current = json!({"avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[251]});
        let mut e = event(
            4,
            "constellation_changed",
            json!({"guid":"42","avatar_id":10000025,"talent_id":251,"skill_depot_id":2501,"previous_count":0,"count":1,"previous":previous,"current":current}),
        );
        e["timestamp"] = json!("2026-11-12T15:40:00+03:00");
        panel.apply(&e).unwrap();
        let r = panel.report();
        assert_eq!(r["characters"][0]["constellation"], 1);
        assert_eq!(r["characters"][0]["talent_ids"], json!([251]));
        assert_eq!(r["characters"][0]["talents"][0]["level"], 1);
        assert_eq!(r["history"][0]["previous_display"], "C0");
        assert_eq!(r["history"][0]["current_display"], "C1");
        assert_eq!(r["history"][0]["timestamp"], e["timestamp"]);
        e["sequence"] = json!(5);
        assert!(panel.apply(&e).is_err());
        assert_eq!(panel.report()["history_total"], 1);
        e["sequence"] = json!(6);
        e["session"] = json!("foreign");
        assert!(panel.apply(&e).is_err());
        panel
            .apply(&event(7, "character_snapshot", json!({"profiles":[]})))
            .unwrap();
        assert!(panel.report()["characters"].as_array().unwrap().is_empty());
    }
    #[test]
    fn achievements_update_history_display_counts_and_isolate_sessions() {
        let mut panel = ready();
        let old = json!({"achievement_id":82040,"status_code":1,"current_progress":3,"total_progress":4,"finish_unix":0});
        panel
            .apply(&event(
                3,
                "achievement_snapshot",
                json!({"records":[old.clone()]}),
            ))
            .unwrap();
        let done = json!({"achievement_id":82040,"status_code":2,"current_progress":null,"total_progress":4,"finish_unix":123});
        let mut change = event(
            4,
            "achievement_changed",
            json!({"achievement_id":82040,"previous":old,"current":done,"action":"completed"}),
        );
        change["timestamp"] = json!("2026-11-12T15:29:40+03:00");
        panel.apply(&change).unwrap();
        let report = panel.report();
        assert_eq!(
            report["achievements"][0]["progress_display"],
            "4 / 4 · Выполнено · награда не получена"
        );
        assert!(report["achievements"][0]["current_progress"].is_null());
        assert_eq!(report["history"][0]["timestamp"], change["timestamp"]);
        let mut claim = done.clone();
        claim["status_code"] = json!(3);
        panel.apply(&event(5,"achievement_changed",json!({"achievement_id":82040,"previous":done,"current":claim,"action":"reward_claimed"}))).unwrap();
        assert_eq!(
            panel.report()["history"][0]["action"],
            "Награда за достижение получена"
        );
        change["sequence"] = json!(6);
        change["session"] = json!("other");
        assert!(panel.apply(&change).is_err());
        let mut baseline = event(7, "currency_snapshot", json!({"item_id":201,"count":18}));
        baseline["session"] = json!("other");
        panel.apply(&baseline).unwrap();
        assert!(
            panel.report()["achievements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn added_character_preserves_roster_populates_fields_and_history_once() {
        let mut panel = ready();
        panel
            .apply(&event(
                3,
                "character_snapshot",
                json!({"profiles":[{"guid":"42","avatar_id":10000034}]}),
            ))
            .unwrap();
        let profile = json!({"guid":"99","avatar_id":10000025,"props":{"4001":{"ival":"1"},"1001":{"ival":"0"},"1002":{"ival":"0"}},"skill_levels":{"10715":1},"extra_skill_levels":{},"friendship":{"level":1,"exp":0}});
        let data = json!({"guid":"99","avatar_id":10000025,"profile":profile});
        let mut change = event(4, "character_added", data.clone());
        change["timestamp"] = json!("2026-11-12T15:22:36+03:00");
        panel.apply(&change).unwrap();
        let r = panel.report();
        assert_eq!(r["characters"].as_array().unwrap().len(), 2);
        let row = r["characters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["guid"] == "99")
            .unwrap();
        assert_eq!(row["character_level"], 1);
        assert_eq!(row["level"], 1);
        assert_eq!(row["talents"][0]["level"], 1);
        assert_eq!(row["updated_at"], change["timestamp"]);
        assert_eq!(r["history"][0]["action"], "Получен персонаж");
        panel
            .apply(&event(5, "character_added", data.clone()))
            .unwrap();
        assert_eq!(panel.report()["history_total"], 1);
        let mut foreign = event(6, "character_added", data);
        foreign["session"] = json!("different");
        assert!(panel.apply(&foreign).is_err());
        assert_eq!(panel.report()["characters"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn primogems_signed_balances_and_mora_have_independent_history() {
        let mut panel = ready();
        panel
            .apply(&event(
                3,
                "currency_snapshot",
                json!({"item_id":201,"count":99}),
            ))
            .unwrap();
        panel
            .apply(&event(
                4,
                "currency_snapshot",
                json!({"item_id":202,"count":1000}),
            ))
            .unwrap();
        let mut change = event(
            5,
            "currency_changed",
            json!({"item_id":201,"previous_count":99,"count":-1,"delta":-100}),
        );
        change["timestamp"] = json!("2026-11-12T13:05:00+03:00");
        panel.apply(&change).unwrap();
        let report = panel.report();
        assert_eq!(report["currencies"][0]["count"], -1);
        assert_eq!(report["currencies"][1]["count"], 1000);
        assert_eq!(report["history"][0]["name"], "Примогемы");
        assert_eq!(report["history"][0]["timestamp"], change["timestamp"]);
        assert!(
            panel
                .apply(&event(
                    6,
                    "currency_changed",
                    json!({"item_id":202,"previous_count":1000,"count":-1,"delta":-1001})
                ))
                .is_err()
        );
        assert_eq!(panel.report()["currencies"][1]["count"], 1000);
    }
    #[test]
    fn talents_update_character_rows_history_and_preserve_bonuses_without_name_collisions() {
        let mut panel = ready().with_skills(HashMap::from([(
            "10715".into(),
            "Проворство шакала".into(),
        )]));
        panel.names.insert("10715".into(), "Другой предмет".into());
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000071,"skill_levels":{"10715":5},"extra_skill_levels":{"7101":3}}]}))).unwrap();
        let mut change = event(
            4,
            "talent_changed",
            json!({"guid":"42","avatar_id":10000071,"skill_id":10715,"previous_level":5,"level":6}),
        );
        change["timestamp"] = json!("2026-11-12T13:05:00+03:00");
        panel.apply(&change).unwrap();
        let report = panel.report();
        assert_eq!(report["characters"][0]["talents"][0]["level"], 6);
        assert_eq!(
            report["characters"][0]["talents"][0]["name"],
            "Проворство шакала"
        );
        assert_eq!(report["characters"][0]["extra_skill_levels"]["7101"], 3);
        assert_eq!(report["history"][0]["action"], "Талант: Проворство шакала");
        assert_eq!(report["history"][0]["timestamp"], change["timestamp"]);
        assert!(panel.apply(&event(5,"talent_changed",json!({"guid":"42","avatar_id":10000071,"skill_id":10715,"previous_level":5,"level":7}))).is_err());
        assert_eq!(panel.report()["characters"][0]["talents"][0]["level"], 6);
    }
    #[test]
    fn legacy_snapshots_with_trial_profiles_are_filtered() {
        let mut panel = ready();
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[
            {"guid":"42","avatar_id":10000127,"trial_info_present":true,"friendship":{"level":1,"exp":0}},
            {"guid":"43","avatar_id":10000127,"avatar_type":3,"friendship":{"level":1,"exp":0}}
        ]}))).unwrap();
        panel.apply(&event(4,"friendship_changed",json!({"guid":"42","avatar_id":10000127,"previous":{"level":1,"exp":0},"current":{"level":1,"exp":1}}))).unwrap();
        panel.apply(&event(5,"character_progress_changed",json!({"guid":"42","avatar_id":10000127,"previous":{"level":90},"current":{"level":91}}))).unwrap();
        let report = panel.report();
        assert_eq!(report["characters"].as_array().unwrap().len(), 1);
        assert_eq!(report["characters"][0]["guid"], "43");
    }
    #[test]
    fn character_progression_updates_profile_history_and_timestamp_separately_from_friendship() {
        let mut panel = ready();
        panel.apply(&event(3,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000137,"props":{"4001":{"ival":"20"},"1001":{"ival":"0"},"1002":{"ival":"1"}},"friendship":{"level":2,"exp":100}}]}))).unwrap();
        let old = json!({"guid":"42","avatar_id":10000137,"level":20,"exp":0,"ascension":1});
        let new = json!({"guid":"42","avatar_id":10000137,"level":21,"exp":6125,"ascension":1});
        let mut changed = event(
            4,
            "character_progress_changed",
            json!({"guid":"42","avatar_id":10000137,"previous":old,"current":new,"exp_delta":null}),
        );
        changed["timestamp"] = json!("2026-11-12T04:05:35.123+03:00");
        panel.apply(&changed).unwrap();
        let report = panel.report();
        assert_eq!(report["characters"][0]["character_level"], 21);
        assert_eq!(report["characters"][0]["character_exp"], 6125);
        assert_eq!(report["characters"][0]["level"], 2);
        assert_eq!(report["characters"][0]["exp"], 100);
        assert_eq!(report["history"][0]["timestamp"], changed["timestamp"]);
        assert_eq!(report["history"][0]["action"], "Уровень: 20 → 21");
        panel.unavailable("stale", "test");
        assert!(
            panel
                .apply(&event(5, "character_progress_changed", json!({})))
                .is_err()
        );
    }
    #[test]
    fn exact_event_timestamp_reaches_history_and_inventory_without_using_poll_time() {
        let mut panel = ready();
        let stamp = "2026-11-12T03:41:55.469+03:00";
        let mut change = event(
            3,
            "material_changed",
            json!({"change":{"item_id":104122,"guid":"18446744073709551615","previous_count":3,"count":4,"delta":1}}),
        );
        change["timestamp"] = json!(stamp);
        change["observed_unix_ms"] = json!("0");
        panel.apply(&change).unwrap();
        let report = panel.report();
        assert_eq!(report["history"][0]["timestamp"], stamp);
        assert_eq!(report["rows"][0]["updated_at"], stamp);
        let mut next = event(
            4,
            "material_changed",
            json!({"change":{"item_id":104122,"guid":"42","previous_count":0,"count":1,"delta":1,"new_entry":true}}),
        );
        next["timestamp"] = json!("2026-11-12T03:41:56.000+03:00");
        panel.apply(&next).unwrap();
        assert_eq!(panel.report()["rows"][0]["updated_at"], next["timestamp"]);
    }
    #[test]
    fn profile_balances_and_friendship_changes_project_to_history_and_reset_on_session() {
        let mut panel = ready();
        panel
            .apply(&event(
                3,
                "currency_snapshot",
                json!({"item_id":202,"count":100}),
            ))
            .unwrap();
        panel
            .apply(&event(
                4,
                "currency_changed",
                json!({"item_id":202,"previous_count":100,"count":70,"delta":-30}),
            ))
            .unwrap();
        assert_eq!(panel.report()["currencies"][0]["count"], 70);
        panel.apply(&event(5,"character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000035,"friendship":{"level":3,"exp":1855}}]}))).unwrap();
        let old = json!({"guid":"42","avatar_id":10000035,"level":3,"exp":1855});
        let new = json!({"guid":"42","avatar_id":10000035,"level":3,"exp":1875});
        panel.apply(&event(6,"friendship_changed",json!({"guid":"42","avatar_id":10000035,"previous":old,"current":new,"exp_delta":20}))).unwrap();
        assert_eq!(panel.report()["characters"][0]["exp"], 1875);
        assert_eq!(panel.report()["history_total"], 2);
        assert_eq!(panel.report()["currencies"][0]["count"], 70);
        panel.unavailable("stale", "test");
        assert!(
            panel
                .apply(&event(7, "currency_changed", json!({})))
                .is_err()
        );
        let mut next = event(8, "currency_snapshot", json!({"item_id":202,"count":20}));
        next["session"] = json!("3");
        panel.apply(&next).unwrap();
        assert!(panel.report()["characters"].as_array().unwrap().is_empty());
    }
    #[test]
    fn equipment_creation_update_and_deletion_preserve_instance_history() {
        let mut panel = ready();
        let item = json!({"guid":"42","item_id":41533,"kind":"artifact","count":1,"level":0,"main_prop_id":13002});
        panel.apply(&event(3,"item_changed",json!({"change":{"guid":"42","item_id":41533,"new_entry":true,"removed":false,"previous":null,"item":item}}))).unwrap();
        assert_eq!(panel.report()["items"][0]["guid"], "42");
        let mut updated = item.clone();
        updated["level"] = json!(4);
        panel.apply(&event(4,"item_changed",json!({"change":{"guid":"42","item_id":41533,"new_entry":false,"removed":false,"previous":item,"item":updated}}))).unwrap();
        assert_eq!(panel.report()["items"][0]["level"], 4);
        panel.apply(&event(5,"item_changed",json!({"change":{"guid":"42","item_id":41533,"new_entry":false,"removed":true,"previous":updated,"item":null}}))).unwrap();
        assert!(panel.report()["items"].as_array().unwrap().is_empty());
        assert_eq!(panel.report()["history_total"], 3);
        assert_eq!(panel.report()["history"][0]["action"], "Удалён");
        panel.unavailable("stale", "test");
        assert!(
            panel
                .apply(&event(6, "item_changed", json!({"change":{}})))
                .is_err()
        );
    }
    #[test]
    fn deletion_creation_and_changed_guid_are_aggregated_by_id() {
        let mut panel = ready();
        panel.apply(&event(3,"material_changed",json!({"change":{"item_id":104122,"guid":"18446744073709551615","previous_count":3,"count":0,"delta":-3,"removed":true}}))).unwrap();
        panel.apply(&event(4,"material_changed",json!({"change":{"item_id":104122,"guid":"42","previous_count":0,"count":1,"delta":1,"new_entry":true}}))).unwrap();
        let report = panel.report();
        assert_eq!(report["rows"].as_array().unwrap().len(), 1);
        assert_eq!(report["rows"][0]["count"], 1);
        assert_eq!(report["rows"][0]["initial_count"], 3);
        assert_eq!(report["history_total"], 2);
        assert_eq!(report["history"][0]["count"], 1);
    }
    #[test]
    fn stale_history_is_retained_but_cannot_be_updated_and_new_snapshot_resets_it() {
        let mut panel = ready();
        panel
            .apply(&event(
                3,
                "status",
                json!({"state":"realtime_tracking_paused_capture_unreliable"}),
            ))
            .unwrap();
        assert_eq!(panel.status, "stale");
        assert!(
            panel
                .apply(&event(4, "material_changed", json!({"change":{}})))
                .is_err()
        );
        assert_eq!(panel.report()["rows"][0]["count"], 3);
        panel
            .apply(&event(
                5,
                "inventory_snapshot",
                json!({"materials":[{"item_id":100002,"guid":"9","count":42}]}),
            ))
            .unwrap();
        assert_eq!(panel.report()["history_total"], 0);
        assert_eq!(panel.report()["rows"][0]["count"], 42);
    }
    #[test]
    fn heartbeat_recovers_but_capture_pause_requires_a_new_snapshot() {
        let mut panel = ready();
        panel.heartbeat_expired();
        assert_eq!(panel.status, "stale");
        panel.apply(&event(3, "source_stats", json!({}))).unwrap();
        assert_eq!(panel.status, "active");
        panel
            .apply(&event(
                4,
                "status",
                json!({"state":"realtime_tracking_paused_capture_unreliable"}),
            ))
            .unwrap();
        panel.apply(&event(5, "source_stats", json!({}))).unwrap();
        assert_eq!(panel.status, "stale");
        panel
            .apply(&event(6, "status", json!({"state":"stopped"})))
            .unwrap();
        assert_eq!(panel.status, "stopped");
        assert!(panel.detail.contains("не подтверждена"));
    }
}
