//! Shared domain mutations and validation for offline import and HTTP delivery.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Action {
    Snapshot {
        collection: String,
        scope: String,
        rows: BTreeMap<String, Value>,
    },
    Set {
        collection: String,
        key: String,
        value: Option<Value>,
    },
    Invalidate {
        reason: String,
        error: bool,
    },
    Notice {
        topics: Vec<String>,
    },
}
impl Action {
    pub fn collection(&self) -> Option<&str> {
        match self {
            Self::Snapshot { collection, .. } | Self::Set { collection, .. } => Some(collection),
            _ => None,
        }
    }
}
pub struct Record {
    pub event: Value,
    pub sequence: i64,
    pub epoch: String,
    pub observed_ms: Option<i64>,
    pub actions: Vec<Action>,
}
pub fn collection(c: &str) -> Result<()> {
    ensure!(
        matches!(
            c,
            "material_stacks"
                | "weapons"
                | "artifacts"
                | "furniture"
                | "cosmetics"
                | "unknown_items"
                | "characters"
                | "traveler_depots"
                | "wallet"
                | "adventure"
                | "achievements"
                | "teams"
                | "player_profile"
                | "wish_history"
        ),
        "Unknown collection"
    );
    Ok(())
}
pub fn validate_entity(c: &str, key: &str, v: &Value) -> Result<()> {
    collection(c)?;
    ensure!(v.is_object(), "Entity must be an object");
    if c == "traveler_depots" {
        crate::legacy::decimal(key)?;
        crate::legacy::decimal(v["guid"].as_str().context("Missing Traveler GUID")?)?;
        let avatar = v["avatar_id"].as_u64().context("Missing Traveler avatar")?;
        let depot = v["skill_depot_id"]
            .as_u64()
            .context("Missing Traveler depot")?;
        ensure!(
            depot.to_string() == key
                && matches!(
                    (avatar, depot),
                    (10000005, 501..=508) | (10000007, 701..=708)
                ),
            "Traveler depot identity mismatch"
        );
        for field in ["skill_levels", "extra_skill_levels"] {
            let map = v[field].as_object().context("Missing Traveler skill map")?;
            ensure!(map.len() <= 64, "Oversized Traveler skills");
            if field == "skill_levels" {
                ensure!(!map.is_empty(), "Missing Traveler base skills");
            }
            for (id, level) in map {
                ensure!(
                    id.parse::<u32>().is_ok_and(|id| id > 0)
                        && level
                            .as_u64()
                            .is_some_and(|n| n <= 15 && (field != "skill_levels" || n > 0)),
                    "Invalid Traveler skill"
                );
            }
        }
        let ids = v["talent_ids"]
            .as_array()
            .context("Missing Traveler talents")?;
        let mut seen = std::collections::BTreeSet::new();
        ensure!(
            ids.len() <= 6
                && ids.iter().all(|id| id
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX) && seen.insert(n))),
            "Invalid Traveler constellations"
        );
    }
    if c == "player_profile" {
        ensure!(key == "player", "Invalid profile key");
        if let Some(picture) = v.get("profile_picture") {
            ensure!(
                picture["id"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= u32::MAX as u64)
                    && picture["frame_id"]
                        .as_u64()
                        .is_some_and(|n| n <= u32::MAX as u64),
                "Invalid profile decoration"
            );
        }
        crate::legacy::decimal(v["uid"].as_str().context("Missing profile UID")?)?;
        if let Some(name) = v.get("nickname") {
            ensure!(
                name.as_str().is_some_and(|s| s.len() <= 256),
                "Invalid nickname"
            );
        }
    }
    if c == "teams" {
        crate::legacy::decimal(key)?;
        ensure!(
            v["team_id"]
                .as_u64()
                .is_some_and(|id| id > 0 && id <= u64::from(u32::MAX) && id.to_string() == key),
            "Invalid team ID"
        );
        ensure!(
            v["name"].as_str().is_some_and(|s| s.len() <= 4096),
            "Invalid team name"
        );
        ensure!(
            v["is_active"].is_null() || v["is_active"].is_boolean(),
            "Invalid active team marker"
        );
        let members = v["members"].as_array().context("Missing team members")?;
        ensure!(members.len() <= 16, "Too many team members");
        let mut seen = std::collections::BTreeSet::new();
        for (index, member) in members.iter().enumerate() {
            let guid = member["guid"]
                .as_str()
                .context("Missing team member GUID")?;
            crate::legacy::decimal(guid)?;
            ensure!(
                seen.insert(guid) && member["position"].as_u64() == Some(index as u64 + 1),
                "Invalid team member order"
            );
        }
    }
    if matches!(
        c,
        "material_stacks"
            | "weapons"
            | "artifacts"
            | "furniture"
            | "cosmetics"
            | "unknown_items"
            | "characters"
    ) {
        crate::legacy::decimal(key)?;
        if let Some(guid) = v.get("guid") {
            ensure!(guid.as_str() == Some(key), "GUID mismatch");
        }
        let field = if c == "characters" {
            "avatar_id"
        } else {
            "item_id"
        };
        ensure!(
            v[field]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX)),
            "Invalid catalog identity"
        );
    }
    if c == "material_stacks" {
        ensure!(
            v["count"]
                .as_u64()
                .is_some_and(|n| n <= u64::from(u32::MAX)),
            "Invalid stack count"
        );
    }
    if c == "wallet" {
        ensure!(matches!(key, "201" | "202"), "Unsupported currency");
        let count = v["count"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .context("Invalid currency count")?;
        ensure!(key == "201" || count >= 0, "Negative Mora");
    }
    if c == "adventure" {
        ensure!(
            key == "player" && v["rank"].as_u64().is_some_and(|n| (1..=60).contains(&n)),
            "Invalid adventure state"
        );
    }
    if c == "achievements" {
        crate::legacy::decimal(key)?;
    }
    Ok(())
}
/// expected=null means creation; updates must check the previous fields explicitly.
pub fn merge(
    c: &str,
    key: &str,
    old: Option<&Value>,
    values: &Value,
    expected: &Value,
    delete: bool,
) -> Result<Option<Value>> {
    collection(c)?;
    if matches!(
        c,
        "material_stacks"
            | "weapons"
            | "artifacts"
            | "furniture"
            | "cosmetics"
            | "unknown_items"
            | "characters"
            | "achievements"
            | "teams"
            | "wish_history"
    ) {
        crate::legacy::decimal(key)?;
    }
    ensure!(values.is_object(), "Values must be an object");
    if expected.is_null() {
        ensure!(
            old.is_none(),
            "Missing previous-state assertion for an existing entity"
        );
    } else {
        let old = old.context("Expected entity missing")?;
        for (k, v) in expected.as_object().context("Expected must be an object")? {
            ensure!(old.get(k) == Some(v), "Previous-state mismatch");
        }
    }
    if delete {
        ensure!(
            values.as_object().unwrap().is_empty(),
            "Deleted entity has values"
        );
        return Ok(None);
    }
    ensure!(!values.as_object().unwrap().is_empty(), "Empty patch");
    let mut next = old
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    for (k, v) in values.as_object().unwrap() {
        next[k] = v.clone();
    }
    if let Some(old) = old {
        for field in ["item_id", "avatar_id", "guid", "uid", "team_id"] {
            if let Some(v) = old.get(field) {
                ensure!(next.get(field) == Some(v), "Entity identity cannot change");
            }
        }
    }
    validate_entity(c, key, &next)?;
    Ok(Some(next))
}
