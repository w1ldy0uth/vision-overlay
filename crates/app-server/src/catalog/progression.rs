//! Costs describe entry into a target ascension/base talent level, never a total.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Material {
    id: u32,
    count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Promotion {
    required_level: u32,
    max_level: u32,
    required_adventure_rank: u32,
    mora: u32,
    materials: Vec<Material>,
    materials_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Talent {
    required_ascension: u32,
    mora: u32,
    materials: Vec<Material>,
}
type Paths<T> = BTreeMap<u32, BTreeMap<u32, T>>;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Progression {
    schema_version: u32,
    avatar_paths: BTreeMap<u32, u32>,
    weapon_paths: BTreeMap<u32, u32>,
    avatar_promotions: Paths<Promotion>,
    weapon_promotions: Paths<Promotion>,
    talent_levels: Paths<Talent>,
}
fn uint(row: &Value, field: &str) -> Result<u32> {
    row.get(field).map_or(Ok(0), |v| {
        v.as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .with_context(|| format!("Invalid progression integer: {field}"))
    })
}
fn materials(row: &Value) -> Result<Vec<Material>> {
    let mut out = Vec::new();
    for item in row["costItems"]
        .as_array()
        .context("Missing progression materials")?
    {
        let id = uint(item, "id")?;
        let count = uint(item, "count")?;
        // Excel uses ID 0 for a disabled slot, sometimes with a leftover count
        // (ordinary Traveler has no boss material in this slot).
        if id == 0 {
            continue;
        }
        ensure!(id > 0 && count > 0, "Incomplete material cost");
        out.push(Material { id, count });
    }
    Ok(out)
}
pub fn compile(tables: &BTreeMap<String, Value>) -> Result<Value> {
    let rows = |name: &str| {
        tables
            .get(name)
            .and_then(Value::as_array)
            .with_context(|| format!("Missing progression source: {name}"))
    };
    let promotions = |table: &str, field: &str, coin: &str| -> Result<Paths<Promotion>> {
        let mut paths: Paths<Promotion> = BTreeMap::new();
        for row in rows(table)? {
            let path = uint(row, field)?;
            let stage = uint(row, "promoteLevel")?;
            let costs = materials(row)?;
            let has_count = row["costItems"]
                .as_array()
                .context("Missing costs")?
                .iter()
                .map(|c| uint(c, "count"))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|count| count > 0);
            let materials_available = !costs.is_empty() || !has_count;
            ensure!(path > 0, "Missing promotion path ID");
            ensure!(
                paths
                    .entry(path)
                    .or_default()
                    .insert(
                        stage,
                        Promotion {
                            required_level: 0,
                            max_level: uint(row, "unlockMaxLevel")?,
                            required_adventure_rank: uint(row, "requiredPlayerLevel")?,
                            mora: uint(row, coin)?,
                            materials: costs,
                            materials_available,
                        }
                    )
                    .is_none(),
                "Duplicate promotion stage"
            );
        }
        for stages in paths.values_mut() {
            let mut previous_cap = 0;
            for step in stages.values_mut() {
                step.required_level = previous_cap;
                previous_cap = step.max_level;
            }
        }
        Ok(paths)
    };
    let links = |name: &str, field: &str| -> Result<BTreeMap<u32, u32>> {
        let mut links = BTreeMap::new();
        for row in rows(name)? {
            let id = uint(row, "id")?;
            let path = uint(row, field)?;
            ensure!(
                id > 0 && path > 0 && links.insert(id, path).is_none(),
                "Invalid/duplicate progression link"
            );
        }
        Ok(links)
    };
    let mut talent_levels: Paths<Talent> = BTreeMap::new();
    for row in rows("ProudSkill")? {
        if uint(row, "proudSkillType")? != 3 {
            continue;
        }
        let level = uint(row, "level")?;
        let mora = uint(row, "coinCost")?;
        let costs = materials(row)?;
        // Level 1 is the initial state; zero-cost 11–15 are stat definitions.
        if mora == 0 && costs.is_empty() {
            continue;
        }
        ensure!((2..=10).contains(&level), "Unsupported paid talent level");
        let group = uint(row, "proudSkillGroupId")?;
        ensure!(
            group > 0
                && talent_levels
                    .entry(group)
                    .or_default()
                    .insert(
                        level,
                        Talent {
                            required_ascension: uint(row, "breakLevel")?,
                            mora,
                            materials: costs,
                        }
                    )
                    .is_none(),
            "Duplicate talent cost"
        );
    }
    Ok(json!(Progression {
        schema_version: 1,
        avatar_paths: links("Avatar", "avatarPromoteId")?,
        weapon_paths: links("Weapon", "weaponPromoteId")?,
        avatar_promotions: promotions("AvatarPromote", "avatarPromoteId", "scoinCost")?,
        weapon_promotions: promotions("WeaponPromote", "weaponPromoteId", "coinCost")?,
        talent_levels,
    }))
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(value) = payload.get("progression") else {
        return Ok(());
    };
    let p: Progression =
        serde_json::from_value(value.clone()).context("Invalid progression schema")?;
    ensure!(
        serde_json::to_value(&p)? == *value,
        "Noncanonical progression keys"
    );
    ensure!(p.schema_version == 1, "Unsupported progression schema");
    let costs = |costs: &[Material]| -> Result<()> {
        let mut ids = BTreeSet::new();
        ensure!(costs.len() <= 16, "Too many cost materials");
        for cost in costs {
            ensure!(
                cost.id > 0
                    && cost.count > 0
                    && ids.insert(cost.id)
                    && payload["materials"].get(cost.id.to_string()).is_some(),
                "Invalid/duplicate material cost link"
            );
        }
        Ok(())
    };
    for (links, paths, entities) in [
        (
            &p.avatar_paths,
            &p.avatar_promotions,
            &payload["avatarMetadata"],
        ),
        (&p.weapon_paths, &p.weapon_promotions, &payload["items"]),
    ] {
        ensure!(
            !links.is_empty() && links.len() <= 10000 && !paths.is_empty() && paths.len() <= 10000,
            "Empty/oversized promotion catalog"
        );
        for (id, path) in links {
            ensure!(
                *id > 0 && entities.get(id.to_string()).is_some() && paths.contains_key(path),
                "Broken promotion path link"
            );
        }
        for (path, stages) in paths {
            ensure!(
                *path > 0 && (2..=7).contains(&stages.len()),
                "Invalid promotion path length"
            );
            let mut previous_cap = 0;
            let mut previous_rank = 0;
            for (index, (stage, step)) in stages.iter().enumerate() {
                ensure!(
                    *stage as usize == index
                        && step.required_level == previous_cap
                        && step.max_level > previous_cap
                        && step.max_level <= 100
                        && step.required_adventure_rank >= previous_rank
                        && step.required_adventure_rank <= 60,
                    "Invalid/gapped promotion requirements"
                );
                costs(&step.materials)?;
                ensure!(
                    step.materials_available || (index > 0 && step.materials.is_empty()),
                    "Inconsistent material availability"
                );
                if index == 0 {
                    ensure!(
                        step.mora == 0 && step.materials.is_empty(),
                        "Initial stage has a cost"
                    );
                }
                previous_cap = step.max_level;
                previous_rank = step.required_adventure_rank;
            }
        }
    }
    ensure!(
        !p.talent_levels.is_empty() && p.talent_levels.len() <= 10000,
        "Invalid talent cost catalog"
    );
    let groups: BTreeSet<u64> = payload["skillMetadata"]
        .as_object()
        .context("Missing skill links")?
        .values()
        .filter_map(|s| s["proudSkillGroupId"].as_u64())
        .collect();
    for (group, levels) in &p.talent_levels {
        ensure!(
            groups.contains(&u64::from(*group)) && levels.len() == 9,
            "Missing talent skill link/levels"
        );
        let mut previous = 0;
        for (index, (level, step)) in levels.iter().enumerate() {
            ensure!(
                *level as usize == index + 2
                    && step.required_ascension >= previous
                    && step.required_ascension <= 6
                    && step.mora > 0
                    && !step.materials.is_empty(),
                "Invalid talent cost/requirements"
            );
            costs(&step.materials)?;
            previous = step.required_ascension;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tables() -> BTreeMap<String, Value> {
        let costs = json!([{"id":101,"count":3},{},{}]);
        let talents: Vec<_> = (1..=15)
            .map(|level| {
                json!({
                    "proudSkillType":3,"proudSkillGroupId":232,"level":level,
                    "breakLevel":if level > 1 {2} else {0},
                    "coinCost":if (2..=10).contains(&level) {12500} else {0},
                    "costItems":if (2..=10).contains(&level) {costs.clone()} else {json!([{}])}
                })
            })
            .collect();
        BTreeMap::from([
            ("Avatar".into(), json!([{"id":2,"avatarPromoteId":7}])),
            ("Weapon".into(), json!([{"id":15,"weaponPromoteId":9}])),
            (
                "AvatarPromote".into(),
                json!([
                    {"avatarPromoteId":7,"unlockMaxLevel":20,"costItems":[{}]},
                    {"avatarPromoteId":7,"promoteLevel":1,"unlockMaxLevel":40,"requiredPlayerLevel":15,"scoinCost":20000,"costItems":costs}
                ]),
            ),
            (
                "WeaponPromote".into(),
                json!([
                    {"weaponPromoteId":9,"unlockMaxLevel":20,"costItems":[{}]},
                    {"weaponPromoteId":9,"promoteLevel":1,"unlockMaxLevel":40,"requiredPlayerLevel":15,"coinCost":5000,"costItems":costs}
                ]),
            ),
            ("ProudSkill".into(), json!(talents)),
        ])
    }
    fn payload(t: &BTreeMap<String, Value>) -> Value {
        json!({"progression":compile(t).unwrap(),"materials":{"101":{}},
            "avatarMetadata":{"2":{}},"items":{"15":{}},
            "skillMetadata":{"10018":{"proudSkillGroupId":232}}})
    }
    #[test]
    fn target_stage_costs_keep_mora_fields_distinct_and_exclude_stat_only_talents() {
        let v = payload(&tables());
        validate(&v).unwrap();
        let p = &v["progression"];
        assert_eq!(p["avatarPromotions"]["7"]["1"]["mora"], 20000);
        assert_eq!(p["weaponPromotions"]["9"]["1"]["mora"], 5000);
        assert_eq!(p["avatarPromotions"]["7"]["1"]["requiredLevel"], 20);
        assert_eq!(p["avatarPromotions"]["7"]["1"]["maxLevel"], 40);
        assert_eq!(p["talentLevels"]["232"].as_object().unwrap().len(), 9);
        assert!(p["talentLevels"]["232"].get("11").is_none());
        assert_eq!(p["talentLevels"]["232"]["2"]["requiredAscension"], 2);
    }
    #[test]
    fn malformed_costs_duplicates_and_new_unsupported_paid_levels_fail_closed() {
        for mutation in 0..4 {
            let mut t = tables();
            match mutation {
                0 => t
                    .get_mut("AvatarPromote")
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"avatarPromoteId":7,"promoteLevel":1,"costItems":[]})),
                1 => t.get_mut("ProudSkill").unwrap()[10]["coinCost"] = json!(1),
                2 => t.get_mut("WeaponPromote").unwrap()[1]["costItems"][0]["count"] = json!(0),
                _ => t.get_mut("AvatarPromote").unwrap()[1]["scoinCost"] = json!(-1),
            }
            assert!(compile(&t).is_err(), "mutation {mutation}");
        }
    }
    #[test]
    fn broken_links_and_gapped_or_inconsistent_requirements_reject_package() {
        for mutation in 0..6 {
            let mut v = payload(&tables());
            match mutation {
                0 => v["progression"]["avatarPaths"]["2"] = json!(999),
                1 => v["progression"]["avatarPromotions"]["7"]["1"]["requiredLevel"] = json!(21),
                2 => {
                    v["materials"].as_object_mut().unwrap().clear();
                }
                3 => {
                    v["skillMetadata"].as_object_mut().unwrap().clear();
                }
                4 => {
                    v["progression"]["talentLevels"]["232"]
                        .as_object_mut()
                        .unwrap()
                        .remove("4");
                }
                _ => {
                    v["progression"]["weaponPromotions"]["9"]["1"]["materials"] =
                        json!([{"id":101,"count":1},{"id":101,"count":2}])
                }
            }
            assert!(validate(&v).is_err(), "mutation {mutation}");
        }
    }
    #[test]
    fn older_packages_are_readable_but_partial_new_sections_are_rejected() {
        validate(&json!({})).unwrap();
        assert!(validate(&json!({"progression":{"schemaVersion":1}})).is_err());
    }
}
