//! Explicit constellation actions → skill depot slot → skill → proud group.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

pub fn archive(bytes: &[u8]) -> Result<(Value, BTreeMap<String, String>)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(zip.len() <= 512, "Too many talent config files");
    let mut configs = Map::new();
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let parts: Vec<_> = name.split('/').collect();
        ensure!(
            parts.len() == 5
                && parts[1..4] == ["BinOutput", "Talent", "AvatarTalents"]
                && parts[4].ends_with(".json")
                && !parts.contains(&".."),
            "Unexpected talent archive path"
        );
        ensure!(entry.size() <= 2 * 1024 * 1024, "Talent config too large");
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut data)?;
        total += data.len();
        ensure!(
            data.len() <= 2 * 1024 * 1024 && total <= 16 * 1024 * 1024,
            "Talent archive size limit"
        );
        let source = parts[1..].join("/");
        ensure!(
            files.insert(source, super::hash(&data)).is_none(),
            "Duplicate talent archive path"
        );
        let v: Value = serde_json::from_slice(&data)?;
        for (key, actions) in v.as_object().context("Invalid talent config")? {
            let selected: Vec<_> = actions
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["$type"] == "AddTalentExtraLevel")
                .cloned()
                .collect();
            if selected.is_empty() {
                continue;
            }
            if let Some(old) = configs.get(key) {
                ensure!(
                    old == &json!(selected),
                    "Conflicting talent action definitions"
                );
            }
            configs.insert(key.clone(), json!(selected));
        }
    }
    ensure!(
        !configs.is_empty(),
        "Talent archive has no bonus definitions"
    );
    Ok((json!(configs), files))
}

pub fn compile(tables: &BTreeMap<String, Value>) -> Result<(Value, Value, Value)> {
    let rows = |name: &str| {
        tables
            .get(name)
            .and_then(Value::as_array)
            .with_context(|| format!("Missing talent table {name}"))
    };
    let skill_rows = rows("AvatarSkill")?;
    let talent_rows = rows("AvatarTalent")?;
    let talents: BTreeMap<_, _> = talent_rows
        .iter()
        .filter_map(|r| Some((r["talentId"].as_u64()?, r)))
        .collect();
    let configs = &tables["AvatarTalentConfigs"];
    let skills: Map<_, _> = skill_rows
        .iter()
        .map(|r| {
            (
                r["id"].to_string(),
                json!({"proudSkillGroupId":r["proudSkillGroupId"],"element":r["costElemType"]}),
            )
        })
        .collect();
    let mut depots = Map::new();
    for row in rows("AvatarSkillDepot")? {
        let id = row["id"].as_u64().context("Missing depot ID")?;
        let mut bonuses = Map::new();
        for (index, tid) in row["talents"].as_array().into_iter().flatten().enumerate() {
            let Some(talent) = tid.as_u64().and_then(|tid| talents.get(&tid)) else {
                continue;
            };
            let Some(config) = talent["openConfig"]
                .as_str()
                .and_then(|k| configs.get(k))
                .and_then(Value::as_array)
            else {
                continue;
            };
            let mut links = Vec::new();
            for action in config {
                if action["extraLevel"] != 3 || action["talentType"] != "AvatarSkill" {
                    continue;
                }
                ensure!(
                    matches!(index, 2 | 4),
                    "Unexpected constellation bonus position"
                );
                let skill = match action["talentIndex"].as_u64() {
                    Some(9) => row["energySkill"].clone(),
                    Some(1) => row["skills"][0].clone(),
                    Some(2) => row["skills"][1].clone(),
                    _ => anyhow::bail!("Unsupported constellation skill index"),
                };
                let Some(group) = skills
                    .get(&skill.to_string())
                    .and_then(|s| s["proudSkillGroupId"].as_u64())
                    .filter(|g| *g > 0)
                else {
                    continue;
                };
                links.push(json!({"skillId":skill,"proudSkillGroupId":group,"levels":3,"constellation":index+1}));
            }
            if !links.is_empty() {
                bonuses.insert(tid.to_string(), json!(links));
            }
        }
        depots.insert(id.to_string(),json!({"skills":row["skills"],"energySkill":row["energySkill"],"attackModeSkill":row["attackModeSkill"],"talents":row["talents"],"constellationBonuses":bonuses}));
    }
    Ok((json!(skills), json!(depots), avatar_links(rows("Avatar")?)))
}

pub fn avatar_links(rows: &[Value]) -> Value {
    let avatars:Map<_,_>=rows.iter().map(|r| {
        let mut ids:Vec<_>=r["candSkillDepotIds"].as_array().into_iter().flatten().cloned().collect();
        if r["skillDepotId"].as_u64().is_some_and(|id|id>0) && !ids.contains(&r["skillDepotId"]) {ids.push(r["skillDepotId"].clone());}
        (r["id"].to_string(),json!({"weaponType":r["weaponType"],"skillDepotId":r["skillDepotId"],"skillDepotIds":ids,"initialWeapon":r["initialWeapon"],"useType":r["useType"],"qualityType":r["qualityType"]}))
    }).collect();
    json!(avatars)
}

pub fn validate(v: &Value) -> Result<()> {
    let keys = ["skillMetadata", "skillDepots", "avatarMetadata"];
    if keys.iter().all(|k| v.get(*k).is_none()) {
        return Ok(());
    }
    let skills = v["skillMetadata"]
        .as_object()
        .context("Missing skill metadata")?;
    let depots = v["skillDepots"]
        .as_object()
        .context("Missing skill depots")?;
    let avatars = v["avatarMetadata"]
        .as_object()
        .context("Missing avatar metadata")?;
    for (id, skill) in skills {
        ensure!(
            id.parse::<u32>().is_ok_and(|n| n > 0)
                && (skill["proudSkillGroupId"].is_null()
                    || skill["proudSkillGroupId"]
                        .as_u64()
                        .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX))),
            "Invalid skill metadata"
        );
    }
    for (id, depot) in depots {
        ensure!(
            id.parse::<u32>().is_ok_and(|n| n > 0),
            "Invalid skill depot ID"
        );
        for (tid, bonuses) in depot["constellationBonuses"]
            .as_object()
            .context("Missing constellation links")?
        {
            let talent = tid.parse::<u32>()?;
            let ids = depot["talents"]
                .as_array()
                .context("Missing depot talents")?;
            let position = ids
                .iter()
                .position(|v| v.as_u64() == Some(u64::from(talent)))
                .context("Bonus outside skill depot")?
                + 1;
            for bonus in bonuses
                .as_array()
                .context("Invalid constellation actions")?
            {
                let key = bonus["skillId"]
                    .as_u64()
                    .context("Missing bonus skill")?
                    .to_string();
                let in_depot = depot["energySkill"] == bonus["skillId"]
                    || depot["skills"]
                        .as_array()
                        .is_some_and(|s| s.contains(&bonus["skillId"]));
                ensure!(
                    in_depot
                        && bonus["levels"] == 3
                        && matches!(position, 3 | 5)
                        && bonus["constellation"] == position
                        && skills
                            .get(&key)
                            .is_some_and(|s| s["proudSkillGroupId"] == bonus["proudSkillGroupId"]),
                    "Invalid constellation/skill link"
                );
            }
        }
    }
    for (id, a) in avatars {
        ensure!(
            a["qualityType"].is_null()
                || matches!(
                    a["qualityType"].as_str(),
                    Some("QUALITY_PURPLE" | "QUALITY_ORANGE" | "QUALITY_ORANGE_SP")
                ),
            "Invalid avatar quality"
        );
        if let Some(ids) = a.get("skillDepotIds") {
            let ids = ids.as_array().context("Invalid avatar depot links")?;
            let mut seen = std::collections::BTreeSet::new();
            for depot in ids {
                let key = depot
                    .as_u64()
                    .filter(|n| *n > 0)
                    .context("Invalid avatar depot ID")?
                    .to_string();
                ensure!(
                    depots.contains_key(&key) && seen.insert(key),
                    "Broken/duplicate avatar depot link"
                );
            }
        }
        ensure!(
            v["avatarNames"].get(id).is_some()
                && (a["weaponType"].is_null() || a["weaponType"].is_string()),
            "Invalid avatar metadata link"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compilation_links_c3_and_c5_by_actions_instead_of_assuming_skill_order() {
        let tables = BTreeMap::from([
            (
                "AvatarSkill".into(),
                json!([{"id":11,"proudSkillGroupId":101},{"id":12,"proudSkillGroupId":102},{"id":19,"proudSkillGroupId":109}]),
            ),
            (
                "AvatarTalent".into(),
                json!([{"talentId":3,"openConfig":"Hero_C3"},{"talentId":5,"openConfig":"Hero_C5"}]),
            ),
            (
                "AvatarSkillDepot".into(),
                json!([{"id":1,"skills":[11,12],"energySkill":19,"talents":[1,2,3,4,5,6]}]),
            ),
            (
                "Avatar".into(),
                json!([{"id":10000001,"weaponType":"WEAPON_SWORD_ONE_HAND","skillDepotId":1}]),
            ),
            (
                "AvatarTalentConfigs".into(),
                json!({"Hero_C3":[{"talentType":"AvatarSkill","talentIndex":9,"extraLevel":3}],"Hero_C5":[{"talentType":"AvatarSkill","talentIndex":2,"extraLevel":3}]}),
            ),
        ]);
        let (skills, depots, avatars) = compile(&tables).unwrap();
        assert_eq!(depots["1"]["constellationBonuses"]["3"][0]["skillId"], 19);
        assert_eq!(depots["1"]["constellationBonuses"]["5"][0]["skillId"], 12);
        let mut payload = json!({"skillMetadata":skills,"skillDepots":depots,"avatarMetadata":avatars,"avatarNames":{"10000001":"Герой"}});
        validate(&payload).unwrap();
        payload["skillDepots"]["1"]["constellationBonuses"]["3"][0]["skillId"] = json!(12);
        assert!(validate(&payload).is_err());
    }
    #[test]
    fn archive_extracts_only_explicit_bonus_actions_and_rejects_paths() {
        use std::io::Write;
        fn zip(path: &str) -> Vec<u8> {
            let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
            z.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(br#"{"Hero_C3":[{"$type":"AddTalentExtraLevel","talentType":"AvatarSkill","talentIndex":9,"extraLevel":3}],"Other":[{"$type":"AddAbility"}]}"#).unwrap();
            z.finish().unwrap().into_inner()
        }
        let (configs, files) = archive(&zip(
            "root/BinOutput/Talent/AvatarTalents/ConfigTalent_Hero.json",
        ))
        .unwrap();
        assert_eq!(configs["Hero_C3"][0]["extraLevel"], 3);
        assert!(configs.get("Other").is_none());
        assert_eq!(files.len(), 1);
        assert!(archive(&zip("root/../../AvatarTalents/ConfigTalent_Hero.json")).is_err());
    }
}
