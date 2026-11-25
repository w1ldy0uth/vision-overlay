//! Validate a complete legacy file before any game facts are committed.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub uid: String,
    pub server_region: String,
}
impl Identity {
    pub fn validate(&self) -> Result<()> {
        decimal(&self.uid)?;
        ensure!((6..=20).contains(&self.uid.len()), "Invalid UID length");
        ensure!(
            matches!(
                self.server_region.as_str(),
                "os_euro" | "os_asia" | "os_usa" | "os_cht"
            ),
            "Unknown server region"
        );
        Ok(())
    }
}
pub type Bindings = BTreeMap<String, Identity>;
/// Global-service UID prefixes. Do not infer Chinese/internal or unknown formats.
pub fn global_region_from_uid(uid: &str) -> Option<&'static str> {
    if !uid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let prefix = match uid.len() {
        9 => &uid[..1],
        10 => &uid[..2],
        _ => return None,
    };
    match prefix {
        "6" => Some("os_usa"),
        "7" => Some("os_euro"),
        "8" | "18" => Some("os_asia"),
        "9" => Some("os_cht"),
        _ => None,
    }
}
pub fn decimal(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0'),
        "Expected canonical positive decimal ID"
    );
    ensure!(s.parse::<u64>().is_ok(), "ID exceeds u64");
    Ok(())
}
fn guid(v: &Value) -> Result<String> {
    let s = v.as_str().context("GUID must be a string")?;
    decimal(s)?;
    Ok(s.into())
}
fn catalog_id(v: &Value) -> Result<()> {
    ensure!(
        v.as_u64()
            .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX)),
        "Invalid catalog ID"
    );
    Ok(())
}
fn stack(v: &Value) -> Result<()> {
    guid(&v["guid"])?;
    catalog_id(&v["item_id"])?;
    ensure!(
        v["count"]
            .as_u64()
            .is_some_and(|n| n <= u64::from(u32::MAX)),
        "Invalid stack count"
    );
    Ok(())
}
fn item_collection(v: &Value) -> &'static str {
    match v["kind"].as_str() {
        Some("weapon") => "weapons",
        Some("artifact") => "artifacts",
        Some("furniture") => "furniture",
        _ => "unknown_items",
    }
}

pub use crate::core::{Action, Record};
pub struct Archive {
    pub records: Vec<Record>,
    pub source: Value,
    pub terminal: String,
    pub data_epochs: BTreeSet<String>,
    pub login_uids: BTreeMap<String, String>,
}
type Entities = BTreeMap<String, Value>;
#[derive(Default, Clone)]
pub struct Projection {
    collections: BTreeMap<String, Entities>,
    blocked: BTreeSet<String>,
}
impl Projection {
    pub fn collection_names(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }
    pub fn entity(&self, c: &str, key: &str) -> Option<&Value> {
        self.collections.get(c).and_then(|r| r.get(key))
    }

    fn inventory_contains(&self, key: &str) -> bool {
        [
            "material_stacks",
            "weapons",
            "artifacts",
            "furniture",
            "unknown_items",
        ]
        .iter()
        .any(|c| {
            self.collections
                .get(*c)
                .is_some_and(|rows| rows.contains_key(key))
        })
    }
    fn snapshot(
        &mut self,
        name: &str,
        scope: &str,
        rows: Entities,
        actions: &mut Vec<Action>,
    ) -> Result<()> {
        self.collections.insert(name.into(), rows.clone());
        self.blocked.remove(name);
        // A fresh inventory/character base alone does not repair other collections.
        actions.push(Action::Snapshot {
            collection: name.into(),
            scope: scope.into(),
            rows,
        });
        Ok(())
    }
    fn old(&self, c: &str, k: &str) -> Result<&Value> {
        self.collections
            .get(c)
            .and_then(|r| r.get(k))
            .context("Update without matching baseline entity")
    }
    fn set(
        &mut self,
        c: &str,
        k: String,
        value: Option<Value>,
        actions: &mut Vec<Action>,
    ) -> Result<()> {
        ensure!(
            !self.blocked.contains(c),
            "Update after invalidation requires a fresh session/base"
        );
        let rows = self
            .collections
            .get_mut(c)
            .context("Update without collection baseline")?;
        if let Some(v) = &value {
            rows.insert(k.clone(), v.clone());
        } else {
            rows.remove(&k);
        }
        actions.push(Action::Set {
            collection: c.into(),
            key: k,
            value,
        });
        Ok(())
    }
    pub fn apply(&mut self, kind: &str, d: &Value) -> Result<Vec<Action>> {
        let mut out = Vec::new();
        match kind {
            "player_profile_snapshot" | "player_profile_changed" => {
                let value = &d["profile"];
                crate::core::validate_entity("player_profile", "player", value)?;
                if kind == "player_profile_snapshot" {
                    self.snapshot(
                        "player_profile",
                        "unverified",
                        BTreeMap::from([("player".into(), value.clone())]),
                        &mut out,
                    )?;
                } else {
                    ensure!(
                        self.old("player_profile", "player")? == &d["previous"],
                        "Profile previous mismatch"
                    );
                    ensure!(d["previous"]["uid"] == value["uid"], "Profile UID changed");
                    self.set(
                        "player_profile",
                        "player".into(),
                        Some(value.clone()),
                        &mut out,
                    )?;
                }
            }
            "team_snapshot" => {
                let mut rows = BTreeMap::new();
                for value in d["teams"].as_array().context("Missing teams")? {
                    let key = value["team_id"]
                        .as_u64()
                        .context("Missing team ID")?
                        .to_string();
                    crate::core::validate_entity("teams", &key, value)?;
                    ensure!(
                        rows.insert(key, value.clone()).is_none(),
                        "Duplicate team ID"
                    );
                }
                self.snapshot("teams", "full", rows, &mut out)?;
            }
            "team_changed" => {
                let mut seen = BTreeSet::new();
                for change in d["changes"].as_array().context("Missing team changes")? {
                    let value = &change["team"];
                    let key = value["team_id"]
                        .as_u64()
                        .context("Missing team ID")?
                        .to_string();
                    ensure!(seen.insert(key.clone()), "Duplicate team change");
                    crate::core::validate_entity("teams", &key, value)?;
                    ensure!(
                        self.old("teams", &key)? == &change["previous"],
                        "Team previous mismatch"
                    );
                    self.set("teams", key, Some(value.clone()), &mut out)?;
                }
            }
            "inventory_snapshot" => {
                let scope = match d["completeness"].as_str() {
                    Some("full") => "full",
                    _ => "unverified",
                };
                let mut groups: BTreeMap<&str, Entities> = [
                    "material_stacks",
                    "weapons",
                    "artifacts",
                    "furniture",
                    "unknown_items",
                ]
                .into_iter()
                .map(|s| (s, BTreeMap::new()))
                .collect();
                let mut seen = BTreeSet::new();
                for v in d["materials"].as_array().context("Missing materials")? {
                    stack(v)?;
                    let key = guid(&v["guid"])?;
                    ensure!(seen.insert(key.clone()), "Duplicate inventory GUID");
                    groups
                        .get_mut("material_stacks")
                        .unwrap()
                        .insert(key, v.clone());
                }
                for v in d["items"].as_array().context("Missing items")? {
                    catalog_id(&v["item_id"])?;
                    let key = guid(&v["guid"])?;
                    ensure!(seen.insert(key.clone()), "Duplicate inventory GUID");
                    groups
                        .get_mut(item_collection(v))
                        .unwrap()
                        .insert(key, v.clone());
                }
                let materials_full = d["material_completeness"] == "full";
                if materials_full {
                    ensure!(
                        d["synthetic"] == false
                            && d["command_id"] == 20892
                            && d["material_coverage"] == "player_store_notify_7_2"
                            && d["recognized_item_count"].as_u64() == Some(seen.len() as u64),
                        "Unsupported material coverage evidence"
                    );
                }
                for (c, rows) in groups {
                    self.snapshot(
                        c,
                        if c == "material_stacks" && materials_full {
                            "full"
                        } else {
                            scope
                        },
                        rows,
                        &mut out,
                    )?;
                }
            }
            "material_changed" => {
                ensure!(
                    d["quantity_semantics"] == "absolute",
                    "Unsupported material semantics"
                );
                let v = &d["change"];
                let k = guid(&v["guid"])?;
                catalog_id(&v["item_id"])?;
                let count = v["count"]
                    .as_u64()
                    .filter(|n| *n <= u64::from(u32::MAX))
                    .context("Invalid count")?;
                let previous = if v["new_entry"] == true {
                    ensure!(
                        !self.inventory_contains(&k),
                        "Inventory GUID already exists in another collection"
                    );
                    ensure!(
                        self.collections
                            .get("material_stacks")
                            .is_some_and(|r| !r.contains_key(&k)),
                        "New GUID already exists"
                    );
                    0
                } else {
                    let old = self.old("material_stacks", &k)?;
                    ensure!(old["item_id"] == v["item_id"], "Material identity changed");
                    old["count"].as_u64().context("Invalid old count")?
                };
                ensure!(
                    v["previous_count"] == previous
                        && v["delta"].as_i64().map(i128::from)
                            == Some(i128::from(count) - i128::from(previous)),
                    "Material previous/delta mismatch"
                );
                ensure!(
                    v["removed"] != true || count == 0,
                    "Deletion has nonzero count"
                );
                let next = if v["removed"] == true {
                    None
                } else {
                    Some(json!({"guid":k,"item_id":v["item_id"],"count":count}))
                };
                self.set("material_stacks", k, next, &mut out)?;
            }
            "item_changed" => {
                let ch = &d["change"];
                let k = guid(&ch["guid"])?;
                let previous = &ch["previous"];
                let next = &ch["item"];
                let c = item_collection(if next.is_null() { previous } else { next });
                if ch["new_entry"] == true {
                    ensure!(
                        !self.inventory_contains(&k),
                        "Inventory GUID already exists in another collection"
                    );
                    ensure!(
                        previous.is_null()
                            && self.collections.get(c).is_some_and(|r| !r.contains_key(&k)),
                        "Conflicting item creation"
                    );
                } else {
                    ensure!(previous["item_id"] == ch["item_id"], "Item ID mismatch");
                    ensure!(self.old(c, &k)? == previous, "Item previous mismatch");
                }
                if ch["removed"] == true {
                    ensure!(next.is_null(), "Deleted item has value");
                    self.set(c, k, None, &mut out)?;
                } else {
                    ensure!(
                        guid(&next["guid"])? == k && next["item_id"] == ch["item_id"],
                        "Item identity mismatch"
                    );
                    catalog_id(&next["item_id"])?;
                    self.set(c, k, Some(next.clone()), &mut out)?;
                }
            }
            "fight_properties_changed" => {
                let key = guid(&d["guid"])?;
                let mut character = self.old("characters", &key)?.clone();
                ensure!(
                    character["avatar_id"] == d["avatar_id"],
                    "Fight property character mismatch"
                );
                let before = d["previous"]
                    .as_object()
                    .context("Missing previous fight properties")?;
                let updates = d["current"]
                    .as_object()
                    .filter(|m| !m.is_empty() && m.len() <= 512)
                    .context("Invalid fight property changes")?;
                ensure!(
                    before.len() == updates.len(),
                    "Fight property keys mismatch"
                );
                for (id, value) in updates {
                    decimal(id)?;
                    ensure!(
                        id.parse::<u32>().is_ok()
                            && value.as_f64().is_some_and(f64::is_finite)
                            && before.get(id) == Some(&character["fight_props"][id]),
                        "Fight property previous/value mismatch"
                    );
                }
                if !character["fight_props"].is_object() {
                    character["fight_props"] = json!({});
                }
                for (id, value) in updates {
                    character["fight_props"][id] = value.clone();
                }
                character["fight_props_live"] = json!(true);
                self.set("characters", key, Some(character), &mut out)?;
            }
            "equipment_changed" => {
                let updates = d["updates"]
                    .as_array()
                    .filter(|v| !v.is_empty() && v.len() <= 2)
                    .context("Invalid equipment updates")?;
                let mut next = BTreeMap::new();
                for u in updates {
                    let key = guid(&u["guid"])?;
                    let mut character = self.old("characters", &key)?.clone();
                    ensure!(
                        character["avatar_id"] == u["avatar_id"]
                            && character["equip_guids"] == u["previous"],
                        "Equipment previous mismatch"
                    );
                    let ids = u["current"]
                        .as_array()
                        .filter(|v| v.len() <= 6)
                        .context("Invalid equipment GUID list")?;
                    let mut seen = BTreeSet::new();
                    for id in ids {
                        let id = guid(id)?;
                        ensure!(
                            seen.insert(id.clone())
                                && (self.entity("weapons", &id).is_some()
                                    || self.entity("artifacts", &id).is_some()),
                            "Invalid equipment reference"
                        );
                    }
                    character["equip_guids"] = u["current"].clone();
                    ensure!(
                        next.insert(key, character).is_none(),
                        "Duplicate equipment character"
                    );
                }
                let mut owners = BTreeSet::new();
                for (key, old) in self
                    .collections
                    .get("characters")
                    .context("Missing character baseline")?
                {
                    for id in next.get(key).unwrap_or(old)["equip_guids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        ensure!(owners.insert(guid(id)?), "Equipment has multiple owners");
                    }
                }
                for (key, character) in next {
                    self.set("characters", key, Some(character), &mut out)?;
                }
            }
            "character_snapshot" => {
                let mut rows = Entities::new();
                for v in d["profiles"].as_array().context("Missing characters")? {
                    if v["trial_info_present"] == true {
                        continue;
                    }
                    let k = guid(&v["guid"])?;
                    catalog_id(&v["avatar_id"])?;
                    ensure!(
                        rows.insert(k, v.clone()).is_none(),
                        "Duplicate character GUID"
                    );
                }
                self.snapshot("characters", "unverified", rows, &mut out)?;
            }
            "traveler_element_baseline" | "traveler_element_changed" => {
                ensure!(
                    !self.blocked.contains("characters"),
                    "Traveler data after invalidation"
                );
                let k = guid(&d["guid"])?;
                let mut character = self.old("characters", &k)?.clone();
                ensure!(
                    character["avatar_id"] == d["avatar_id"]
                        && matches!(character["avatar_id"].as_u64(), Some(10000005 | 10000007)),
                    "Traveler identity mismatch"
                );
                let subset = |v: &Value| json!({"skill_depot_id":v["skill_depot_id"],"skill_levels":v["skill_levels"],"talent_ids":v["talent_ids"],"extra_skill_levels":v["extra_skill_levels"]});
                if kind == "traveler_element_baseline" {
                    ensure!(
                        subset(&character) == d["current"],
                        "Traveler baseline mismatch"
                    );
                } else {
                    ensure!(
                        subset(&character) == d["previous"],
                        "Traveler previous set mismatch"
                    );
                }
                let mut depot = d["current"].clone();
                depot["guid"] = json!(k);
                depot["avatar_id"] = d["avatar_id"].clone();
                let id = depot["skill_depot_id"]
                    .as_u64()
                    .context("Missing depot")?
                    .to_string();
                crate::core::validate_entity("traveler_depots", &id, &depot)?;
                self.collections
                    .entry("traveler_depots".into())
                    .or_default()
                    .insert(id.clone(), depot.clone());
                self.blocked.remove("traveler_depots");
                out.push(Action::Snapshot {
                    collection: "traveler_depots".into(),
                    scope: "partial".into(),
                    rows: BTreeMap::from([(id, depot)]),
                });
                if kind == "traveler_element_changed" {
                    ensure!(
                        !self.blocked.contains("characters"),
                        "Traveler change after invalidation"
                    );
                    for field in [
                        "skill_depot_id",
                        "skill_levels",
                        "talent_ids",
                        "extra_skill_levels",
                    ] {
                        character[field] = d["current"][field].clone();
                    }
                    self.collections
                        .get_mut("characters")
                        .unwrap()
                        .insert(k.clone(), character.clone());
                    // Both collections form one normalized snapshot and one SQLite transaction.
                    out.push(Action::Snapshot {
                        collection: "characters".into(),
                        scope: "partial".into(),
                        rows: BTreeMap::from([(k, character)]),
                    });
                }
            }
            "character_added" => {
                let v = &d["profile"];
                if v["trial_info_present"] != true {
                    let k = guid(&v["guid"])?;
                    catalog_id(&v["avatar_id"])?;
                    ensure!(
                        v["guid"] == d["guid"] && v["avatar_id"] == d["avatar_id"],
                        "Character identity mismatch"
                    );
                    ensure!(
                        self.collections
                            .get("characters")
                            .is_some_and(|r| !r.contains_key(&k)),
                        "Duplicate added character"
                    );
                    self.set("characters", k, Some(v.clone()), &mut out)?;
                }
            }
            "currency_snapshot" | "currency_changed" => {
                let id = d["item_id"].as_u64().context("Missing currency")?;
                ensure!(matches!(id, 106 | 201 | 202), "Unsupported currency");
                let k = id.to_string();
                let n = d["count"].as_i64().context("Invalid currency count")?;
                // Legacy permits negative primogems; retain the actual observation explicitly.
                ensure!(id == 201 || n >= 0, "Negative Mora");
                let mut value = json!({"item_id":id,"count":n.to_string()});
                if id == 106 {
                    value["next_recovery_unix_s"] = d["next_recovery_unix_s"].clone();
                }
                if kind == "currency_changed" {
                    let old = self.old("wallet", &k)?["count"]
                        .as_str()
                        .and_then(|v| v.parse::<i64>().ok())
                        .context("Missing currency baseline")?;
                    ensure!(
                        d["previous_count"] == old
                            && d["delta"].as_i64().map(i128::from)
                                == Some(i128::from(n) - i128::from(old)),
                        "Currency previous/delta mismatch"
                    );
                    self.set("wallet", k, Some(value), &mut out)?;
                } else {
                    self.collections
                        .entry("wallet".into())
                        .or_default()
                        .insert(k.clone(), value.clone());
                    out.push(Action::Snapshot {
                        collection: "wallet".into(),
                        scope: "partial".into(),
                        rows: BTreeMap::from([(k, value)]),
                    });
                }
            }
            "onslaught_snapshot" | "onslaught_exalted_snapshot" => {
                let collection = if kind == "onslaught_snapshot" {
                    "onslaught"
                } else {
                    "onslaught_exalted"
                };
                let key = "current";
                crate::core::validate_entity(collection, key, d)?;
                self.snapshot(
                    collection,
                    "full",
                    BTreeMap::from([(key.into(), d.clone())]),
                    &mut out,
                )?;
            }
            "theater_snapshot" => {
                let key = "current";
                crate::core::validate_entity("theater", key, d)?;
                self.snapshot(
                    "theater",
                    "full",
                    BTreeMap::from([(key.into(), d.clone())]),
                    &mut out,
                )?;
            }
            "abyss_snapshot" => {
                let key = "current";
                crate::core::validate_entity("abyss", key, d)?;
                self.snapshot(
                    "abyss",
                    "full",
                    BTreeMap::from([(key.into(), d.clone())]),
                    &mut out,
                )?;
            }
            "adventure_snapshot" | "adventure_progress_changed" => {
                let v = &d["current"];
                ensure!(
                    v.is_object() && v["rank"].as_u64().is_some_and(|n| n > 0 && n <= 60),
                    "Invalid adventure data"
                );
                if kind == "adventure_snapshot" {
                    self.snapshot(
                        "adventure",
                        "partial",
                        BTreeMap::from([("player".into(), v.clone())]),
                        &mut out,
                    )?;
                } else {
                    ensure!(
                        self.old("adventure", "player")? == &d["previous"],
                        "Adventure previous mismatch"
                    );
                    self.set("adventure", "player".into(), Some(v.clone()), &mut out)?;
                }
            }
            "achievement_snapshot" => {
                let mut rows = Entities::new();
                for v in d["records"].as_array().context("Missing achievements")? {
                    catalog_id(&v["achievement_id"])?;
                    ensure!(
                        rows.insert(v["achievement_id"].to_string(), v.clone())
                            .is_none(),
                        "Duplicate achievement"
                    );
                }
                self.snapshot("achievements", "unverified", rows, &mut out)?;
            }
            "achievement_changed" => {
                let k = d["achievement_id"].to_string();
                let old = self.collections.get("achievements").and_then(|r| r.get(&k));
                ensure!(
                    old.unwrap_or(&Value::Null) == &d["previous"],
                    "Achievement previous mismatch"
                );
                ensure!(
                    d["current"]["achievement_id"] == d["achievement_id"],
                    "Achievement identity mismatch"
                );
                self.set("achievements", k, Some(d["current"].clone()), &mut out)?;
            }
            "constellation_depot_changed" => {
                let k = guid(&d["guid"])?;
                let mut v = self.old("characters", &k)?.clone();
                catalog_id(&d["skill_depot_id"])?;
                catalog_id(&d["talent_id"])?;
                let depot = d["skill_depot_id"].to_string();
                ensure!(
                    v["avatar_id"] == d["avatar_id"]
                        && matches!(v["avatar_id"].as_u64(), Some(10000005 | 10000007))
                        && v["skill_depot_id"] != d["skill_depot_id"]
                        && v["constellations_by_depot"][&depot] == d["previous"],
                    "Traveler constellation depot mismatch"
                );
                let before = if d["previous"].is_null() {
                    Vec::new()
                } else {
                    d["previous"]["observed_talent_ids"]
                        .as_array()
                        .context("Missing observed constellations")?
                        .clone()
                };
                let after = d["current"]["observed_talent_ids"]
                    .as_array()
                    .context("Missing updated constellations")?;
                ensure!(
                    d["current"]["completeness"] == "partial"
                        && after.len() <= 6
                        && after.len() == before.len() + 1
                        && after.starts_with(&before)
                        && after.last() == Some(&d["talent_id"])
                        && !before.contains(&d["talent_id"]),
                    "Invalid partial constellation activation"
                );
                if v["constellations_by_depot"].is_null() {
                    v["constellations_by_depot"] = json!({});
                }
                v["constellations_by_depot"][depot] = d["current"].clone();
                self.set("characters", k, Some(v), &mut out)?;
            }
            "friendship_changed"
            | "character_progress_changed"
            | "talent_changed"
            | "constellation_changed" => {
                let k = guid(&d["guid"])?;
                let mut v = self.old("characters", &k)?.clone();
                ensure!(
                    v["avatar_id"] == d["avatar_id"],
                    "Character identity mismatch"
                );
                if kind == "friendship_changed" {
                    ensure!(
                        v["friendship"]["level"] == d["previous"]["level"]
                            && v["friendship"]["exp"] == d["previous"]["exp"]
                            && d["previous"]["guid"] == k
                            && d["current"]["guid"] == k
                            && d["current"]["avatar_id"] == v["avatar_id"],
                        "Friendship previous mismatch"
                    );
                    v["friendship"] =
                        json!({"level":d["current"]["level"],"exp":d["current"]["exp"]});
                } else if kind == "talent_changed" {
                    let id = d["skill_id"].as_u64().context("Invalid skill")?.to_string();
                    ensure!(
                        v["skill_levels"][&id] == d["previous_level"] && d["level"].is_u64(),
                        "Talent previous mismatch"
                    );
                    v["skill_levels"][id] = d["level"].clone();
                } else if kind == "constellation_changed" {
                    ensure!(
                        v["talent_ids"] == d["previous"]["talent_ids"]
                            && v["skill_depot_id"] == d["skill_depot_id"],
                        "Constellation previous mismatch"
                    );
                    v["talent_ids"] = d["current"]["talent_ids"].clone();
                } else {
                    for (prop, field) in [("4001", "level"), ("1001", "exp"), ("1002", "ascension")]
                    {
                        let previous = v["props"][prop]["ival"]
                            .as_str()
                            .and_then(|s| s.parse::<u64>().ok());
                        ensure!(
                            previous.map(|n| json!(n)).unwrap_or(Value::Null)
                                == d["previous"][field],
                            "Character previous mismatch"
                        );
                        v["props"][prop]["ival"] = if d["current"][field].is_null() {
                            Value::Null
                        } else {
                            json!(d["current"][field].to_string())
                        };
                    }
                }
                self.set("characters", k, Some(v), &mut out)?;
            }
            "status" => {
                let s = d["state"].as_str().context("Missing status")?;
                if s.contains("paused")
                    || s.contains("overflow")
                    || s == "stale"
                    || s == "source_error"
                    || s.starts_with("decoder_failed")
                {
                    self.blocked.extend(self.collections.keys().cloned());
                    out.push(Action::Invalidate {
                        reason: s.into(),
                        error: s == "source_error" || s.starts_with("decoder_failed"),
                    });
                }
            }
            "account_update_rejected"
            | "achievement_update_rejected"
            | "profile_update_rejected"
            | "material_update_rejected" => {
                self.blocked.extend(self.collections.keys().cloned());
                out.push(Action::Invalidate {
                    reason: kind.into(),
                    error: false,
                });
            }
            "source_config"
            | "login_identity"
            | "transport_config"
            | "transport_stats"
            | "transport_summary"
            | "source_stats"
            | "source_summary"
            | "decoder_warning_counts"
            | "decoder_recovery_stats"
            | "decoder_recovery_context"
            | "game_process_watch"
            | "material_state"
            | "unsupported_body_encoding" => {}
            s if s.ends_with("_candidate") || s.ends_with("_summary") => {}
            _ => bail!("Unsupported legacy event kind: {kind}"),
        }
        Ok(out)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Archive> {
    ensure!(
        bytes.len() <= 512 * 1024 * 1024,
        "Archive exceeds 512 MiB limit"
    );
    let text = std::str::from_utf8(bytes)?.trim_start_matches('\u{feff}');
    ensure!(text.ends_with('\n'), "Archive has an incomplete final line");
    let mut records = Vec::new();
    let mut projections: BTreeMap<String, Projection> = BTreeMap::new();
    let mut data_epochs = BTreeSet::new();
    let mut login_uids = BTreeMap::new();
    let mut source = Value::Null;
    let mut terminal = String::new();
    let mut closed = false;
    let mut last_loss_count = 0u64;
    let mut previous_epoch = 0u64;
    for (line_no, line) in text.lines().enumerate() {
        ensure!(
            !line.is_empty() && line.len() <= 16 * 1024 * 1024,
            "Empty/oversize JSONL line {}",
            line_no + 1
        );
        let e: Value = serde_json::from_str(line)
            .with_context(|| format!("Invalid JSON at line {}", line_no + 1))?;
        ensure!(e["schema_version"] == 1, "Unsupported legacy schema");
        let sequence = e["sequence"]
            .as_i64()
            .filter(|n| *n > 0)
            .context("Invalid sequence")?;
        ensure!(
            sequence == records.len() as i64 + 1,
            "Sequence gap or duplicate"
        );
        let epoch = e["session"].as_str().context("Missing epoch")?.to_owned();
        let epoch_number = epoch.parse::<u64>().context("Invalid epoch")?;
        ensure!(
            epoch_number.to_string() == epoch && epoch_number >= previous_epoch,
            "Invalid/decreasing epoch"
        );
        previous_epoch = epoch_number;
        let kind = e["event"].as_str().context("Missing event kind")?;
        if kind == "login_identity" {
            let uid = e["data"]["uid"].as_str().context("Invalid login UID")?;
            decimal(uid)?;
            if let Some(old) = login_uids.insert(epoch.clone(), uid.into()) {
                ensure!(old == uid, "Conflicting login UID in epoch");
            }
        }
        ensure!(
            kind != "source_config" || sequence == 1,
            "Repeated source_config in one archive"
        );
        if sequence == 1 {
            ensure!(kind == "source_config", "Expected source_config first");
            source = e["data"].clone();
            ensure!(
                source["mode"] == "live",
                "Only completed live JSONL can update saved state; replay needs a separate verified provenance adapter"
            );
        }
        let observed_ms = match e.get("observed_unix_ms") {
            Some(Value::String(s)) => Some(s.parse::<i64>().context("Invalid observed time")?),
            Some(Value::Null) | None => None,
            _ => bail!("Observed time must be a string"),
        };
        if kind == "status" {
            terminal = e["data"]["state"]
                .as_str()
                .context("Invalid terminal status")?
                .into();
        }
        let mut actions = projections
            .entry(epoch.clone())
            .or_default()
            .apply(kind, &e["data"])
            .with_context(|| format!("Invalid {kind} at source sequence {sequence}"))?;
        if matches!(kind, "source_stats" | "source_summary") {
            let data = &e["data"];
            let loss_count = [
                &data["dropped_queue_full"],
                &data["realtime_backend"]["callback_queue_drops"],
                &data["realtime_backend"]["missed_read"],
                &data["realtime_backend"]["missed_write"],
                &data["realtime_backend"]["invalid_descriptors"],
            ]
            .into_iter()
            .try_fold(0u64, |n, v| n.checked_add(v.as_u64().unwrap_or(0)))
            .context("Capture loss counter overflow")?;
            if loss_count > last_loss_count {
                let p = projections.get_mut(&epoch).unwrap();
                p.blocked.extend(p.collections.keys().cloned());
                actions.push(Action::Invalidate {
                    reason: "capture_loss".into(),
                    error: false,
                });
            }
            last_loss_count = last_loss_count.max(loss_count);
        }
        if !actions.is_empty() && !matches!(actions.as_slice(), [Action::Invalidate { .. }]) {
            ensure!(!closed, "Game facts after terminal capture status");
            ensure!(epoch_number > 0, "Game data in epoch zero");
            ensure!(
                observed_ms.is_some_and(|t| t >= 0),
                "Game data lacks observed time"
            );
            data_epochs.insert(epoch.clone());
        }
        if kind == "status"
            && (terminal.starts_with("stopped")
                || terminal == "source_error"
                || terminal.starts_with("decoder_failed"))
        {
            closed = true;
        }
        records.push(Record {
            event: e,
            sequence,
            epoch,
            observed_ms,
            actions,
        });
    }
    ensure!(!records.is_empty(), "Empty archive");
    ensure!(
        terminal.starts_with("stopped")
            || terminal == "source_error"
            || terminal.starts_with("decoder_failed"),
        "Capture is not finalized"
    );
    Ok(Archive {
        records,
        source,
        terminal,
        data_epochs,
        login_uids,
    })
}
