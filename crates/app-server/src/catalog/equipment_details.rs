use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
pub fn compile(tables: &BTreeMap<String, Value>, texts: &Map<String, Value>) -> Result<Value> {
    let rows = |n: &str| {
        tables
            .get(n)
            .and_then(Value::as_array)
            .context("Missing equipment source")
    };
    let links = regex::Regex::new(r"\{/?LINK[^}]*\}")?;
    let text = |r: &Value, k: &str| {
        texts
            .get(&r[k].to_string())
            .and_then(Value::as_str)
            .map(|s| {
                links
                    .replace_all(&super::build::display(s), "")
                    .replace("{NON_BREAK_SPACE}", " ")
            })
    };
    let mut affixes: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for r in rows("EquipAffix")? {
        affixes.entry(r["id"].to_string()).or_default().insert(
            (r["level"].as_u64().unwrap_or(0) + 1).to_string(),
            json!({"name":text(r,"nameTextMapHash"),"description":text(r,"descTextMapHash")}),
        );
    }
    let mut weapons = Map::new();
    for r in rows("Weapon")? {
        let props:Vec<_>=r["weaponProp"].as_array().into_iter().flatten().filter(|p|p["propType"].as_str().is_some_and(|s|s!="FIGHT_PROP_NONE")).map(|p|json!({"type":p["propType"],"initial":p["initValue"].as_f64().unwrap_or(0.0),"curve":p["type"]})).collect();
        let effect = r["skillAffix"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|id| affixes.get(&id.to_string()));
        weapons.insert(
            r["id"].to_string(),
            json!({"promotionId":r["weaponPromoteId"],"props":props,"effects":effect}),
        );
    }
    let mut curves = Map::new();
    for r in rows("WeaponCurve")? {
        let values: Map<_, _> = r["curveInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    p["type"].as_str().unwrap_or_default().into(),
                    json!({"arith":p["arith"],"value":p["value"]}),
                )
            })
            .collect();
        curves.insert(r["level"].to_string(), json!(values));
    }
    let mut promotions: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for r in rows("WeaponPromote")? {
        let props: Map<_, _> = r["addProps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some((
                    p["propType"].as_str()?.into(),
                    json!(p["value"].as_f64().unwrap_or(0.0)),
                ))
            })
            .collect();
        promotions
            .entry(r["weaponPromoteId"].to_string())
            .or_default()
            .insert(
                r["promoteLevel"].as_u64().unwrap_or(0).to_string(),
                json!({"maxLevel":r["unlockMaxLevel"],"props":props}),
            );
    }
    let mut sets = Map::new();
    for r in rows("ReliquarySet")? {
        let effects = affixes.get(&r["equipAffixId"].to_string());
        let bonuses:Vec<_>=r["setNeedNum"].as_array().into_iter().flatten().enumerate().map(|(i,n)|json!({"pieces":n,"description":effects.and_then(|e|e.get(&(i+1).to_string())).map(|e|&e["description"])})).collect();
        sets.insert(
            r["setId"].to_string(),
            json!({"name":effects.and_then(|e|e.get("1")).map(|e|&e["name"]),"bonuses":bonuses}),
        );
    }
    let relics: Map<_, _> = rows("Reliquary")?
        .iter()
        .filter(|r| r["setId"].as_u64().is_some_and(|n| n > 0))
        .map(|r| (r["id"].to_string(), r["setId"].clone()))
        .collect();
    Ok(
        json!({"schemaVersion":1,"weapons":weapons,"curves":curves,"promotions":promotions,"sets":sets,"relicSets":relics}),
    )
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(p) = payload.get("equipmentDetails") else {
        return Ok(());
    };
    ensure!(p["schemaVersion"] == 1, "Unsupported equipment metadata");
    for key in ["weapons", "curves", "promotions", "sets", "relicSets"] {
        let rows = p[key]
            .as_object()
            .context("Missing equipment metadata map")?;
        ensure!(
            rows.len() <= 20000 && rows.keys().all(|k| super::positive_id(k)),
            "Invalid equipment metadata IDs"
        );
    }
    for w in p["weapons"].as_object().unwrap().values() {
        ensure!(
            w["promotionId"].as_u64().is_some_and(|n| n > 0),
            "Invalid weapon promotion link"
        );
        let props = w["props"].as_array().context("Invalid weapon properties")?;
        ensure!(props.len() <= 4, "Too many weapon properties");
        for prop in props {
            ensure!(
                prop["type"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("FIGHT_PROP_"))
                    && prop["curve"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("GROW_CURVE_"))
                    && prop["initial"]
                        .as_f64()
                        .is_some_and(|n| n.is_finite() && n >= 0.0),
                "Invalid weapon property"
            );
        }
        if let Some(e) = w["effects"].as_object() {
            for (rank, v) in e {
                ensure!(
                    rank.parse::<u32>().is_ok_and(|n| (1..=5).contains(&n)),
                    "Invalid refinement"
                );
                validate_text(v)?;
            }
        }
    }
    for levels in p["curves"].as_object().unwrap().values() {
        for c in levels.as_object().context("Invalid curves")?.values() {
            ensure!(
                c["arith"] == "ARITH_MULTI"
                    && c["value"]
                        .as_f64()
                        .is_some_and(|n| n.is_finite() && n > 0.0),
                "Invalid weapon curve"
            );
        }
    }
    for path in p["promotions"].as_object().unwrap().values() {
        for (stage, r) in path.as_object().context("Invalid promotion path")? {
            ensure!(
                stage.parse::<u32>().is_ok_and(|n| n <= 6)
                    && r["maxLevel"].as_u64().is_some_and(|n| n > 0 && n <= 100),
                "Invalid promotion stage"
            );
            for v in r["props"]
                .as_object()
                .context("Invalid promotion stats")?
                .values()
            {
                ensure!(
                    v.as_f64().is_some_and(|n| n.is_finite()),
                    "Invalid promotion stat"
                );
            }
        }
    }
    for set in p["sets"].as_object().unwrap().values() {
        let bonuses = set["bonuses"]
            .as_array()
            .context("Invalid artifact bonuses")?;
        ensure!(bonuses.len() <= 4, "Too many artifact bonuses");
        for bonus in bonuses {
            ensure!(
                bonus["pieces"]
                    .as_u64()
                    .is_some_and(|n| (1..=5).contains(&n)),
                "Invalid artifact threshold"
            );
            validate_text(bonus)?;
        }
    }
    for id in p["relicSets"].as_object().unwrap().values() {
        ensure!(
            id.as_u64()
                .is_some_and(|n| p["sets"].get(n.to_string()).is_some()),
            "Unknown artifact set"
        );
    }
    Ok(())
}
fn validate_text(v: &Value) -> Result<()> {
    for k in ["name", "description"] {
        ensure!(
            v[k].is_null() || v[k].as_str().is_some_and(|s| s.len() <= 131072),
            "Invalid equipment text"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equipment_sources_keep_refinement_and_set_threshold_links() {
        let tables = BTreeMap::from([
            (
                "Weapon".into(),
                json!([{"id":15402,"weaponPromoteId":15402,"weaponProp":[{"propType":"FIGHT_PROP_BASE_ATTACK","initValue":42.401,"type":"GROW_CURVE_ATTACK_201"}],"skillAffix":[115402,0]}]),
            ),
            (
                "WeaponCurve".into(),
                json!([{"level":90,"curveInfos":[{"type":"GROW_CURVE_ATTACK_201","arith":"ARITH_MULTI","value":8.349}]}]),
            ),
            (
                "WeaponPromote".into(),
                json!([{"weaponPromoteId":15402,"promoteLevel":6,"unlockMaxLevel":90,"addProps":[{"propType":"FIGHT_PROP_BASE_ATTACK","value":155.6}]}]),
            ),
            (
                "EquipAffix".into(),
                json!([{"id":115402,"level":4,"nameTextMapHash":1,"descTextMapHash":2},{"id":215003,"nameTextMapHash":3,"descTextMapHash":4},{"id":215003,"level":1,"nameTextMapHash":3,"descTextMapHash":5}]),
            ),
            ("Reliquary".into(), json!([{"id":77310,"setId":15003}])),
            (
                "ReliquarySet".into(),
                json!([{"setId":15003,"equipAffixId":215003,"setNeedNum":[2,4]}]),
            ),
        ]);
        let texts = Map::from_iter([
            ("1".into(), json!("Песнь")),
            ("2".into(), json!("<color=#FF0000>48%</color>")),
            ("3".into(), json!("Ансамбль")),
            ("4".into(), json!("80 МС")),
            ("5".into(), json!("35%")),
        ]);
        let p = compile(&tables, &texts).unwrap();
        assert_eq!(p["weapons"]["15402"]["effects"]["5"]["description"], "48%");
        assert_eq!(
            p["sets"]["15003"]["bonuses"][1],
            json!({"pieces":4,"description":"35%"})
        );
        let mut payload = json!({"equipmentDetails":p});
        validate(&payload).unwrap();
        payload["equipmentDetails"]["relicSets"]["77310"] = json!(999);
        assert!(validate(&payload).is_err());
    }
}
