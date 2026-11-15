use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub fn compile(tables: &BTreeMap<String, Value>, texts: &Map<String, Value>) -> Result<Value> {
    let rows = |name: &str| {
        tables
            .get(name)
            .and_then(Value::as_array)
            .context("Missing character presentation source")
    };
    let links = regex::Regex::new(r"\{/?LINK[^}]*\}").unwrap();
    let localized = |r: &Value, key: &str| -> Value {
        texts
            .get(&r[key].to_string())
            .and_then(Value::as_str)
            .map(|s| {
                let text = super::build::display(s).replace("{NON_BREAK_SPACE}", " ");
                json!(links.replace_all(&text, "").into_owned())
            })
            .unwrap_or(Value::Null)
    };
    let entry = |r: &Value, icon: &str| json!({"name":localized(r,"nameTextMapHash"),"description":localized(r,"descTextMapHash"),"icon":r[icon]});
    let mut skills = Map::new();
    for r in rows("AvatarSkill")? {
        skills.insert(r["id"].to_string(), entry(r, "skillIcon"));
    }
    let mut passives = Map::new();
    for r in rows("ProudSkill")? {
        if r["level"] == 1 && r["proudSkillType"] == 2 {
            passives.insert(r["proudSkillGroupId"].to_string(), entry(r, "icon"));
        }
    }
    let mut constellations = Map::new();
    for r in rows("AvatarTalent")? {
        constellations.insert(r["talentId"].to_string(), entry(r, "icon"));
    }
    let mut depots = Map::new();
    for r in rows("AvatarSkillDepot")? {
        let passive:Vec<_>=r["inherentProudSkillOpens"].as_array().into_iter().flatten().filter(|v|v["proudSkillGroupId"].as_u64().is_some_and(|n|n>0)).map(|v|json!({"groupId":v["proudSkillGroupId"],"requiredAscension":v["needAvatarPromoteLevel"].as_u64().unwrap_or(0)})).collect();
        depots.insert(r["id"].to_string(), json!({"passives":passive}));
    }
    Ok(
        json!({"schemaVersion":1,"skills":skills,"passives":passives,"constellations":constellations,"depots":depots}),
    )
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(p) = payload.get("characterDetails") else {
        return Ok(());
    };
    ensure!(
        p["schemaVersion"] == 1,
        "Unsupported character presentation metadata"
    );
    for key in ["skills", "passives", "constellations"] {
        let map = p[key]
            .as_object()
            .context("Missing character presentation map")?;
        ensure!(map.len() <= 20000, "Oversized character presentation map");
        for (id, r) in map {
            ensure!(
                super::positive_id(id) && super::icon(&r["icon"]),
                "Invalid character presentation icon"
            );
            for field in ["name", "description"] {
                ensure!(
                    r[field].is_null() || r[field].as_str().is_some_and(|s| s.len() <= 131072),
                    "Invalid talent text"
                );
            }
        }
    }
    let depots = p["depots"].as_object().context("Missing passive links")?;
    ensure!(depots.len() <= 4096, "Oversized passive links");
    for (id, r) in depots {
        ensure!(super::positive_id(id), "Invalid depot");
        let passives = r["passives"].as_array().context("Invalid passive list")?;
        ensure!(passives.len() <= 16, "Oversized passive list");
        for passive in passives {
            ensure!(
                passive["groupId"].as_u64().is_some_and(|n| n > 0)
                    && passive["requiredAscension"]
                        .as_u64()
                        .is_some_and(|n| n <= 6),
                "Invalid passive requirement"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_preserves_depot_links_unlock_requirements_and_plain_descriptions() {
        let tables = BTreeMap::from([
            (
                "AvatarSkill".into(),
                json!([{"id":11,"nameTextMapHash":1,"descTextMapHash":2,"skillIcon":"Skill_A_01"}]),
            ),
            (
                "ProudSkill".into(),
                json!([{"level":1,"proudSkillType":2,"proudSkillGroupId":21,"nameTextMapHash":1,"descTextMapHash":2,"icon":"UI_Talent_S_Test"}]),
            ),
            (
                "AvatarTalent".into(),
                json!([{"talentId":31,"nameTextMapHash":1,"descTextMapHash":2,"icon":"UI_Talent_S_Test"}]),
            ),
            (
                "AvatarSkillDepot".into(),
                json!([{"id":1,"inherentProudSkillOpens":[{"proudSkillGroupId":21,"needAvatarPromoteLevel":4},{}]}]),
            ),
        ]);
        let texts = Map::from_iter([
            ("1".into(), json!("Название")),
            (
                "2".into(),
                json!("{LINK#N123}<color=#FF0000>Описание</color>{/LINK}"),
            ),
        ]);
        let p = compile(&tables, &texts).unwrap();
        assert_eq!(p["skills"]["11"]["description"], "Описание");
        assert_eq!(
            p["depots"]["1"]["passives"],
            json!([{"groupId":21,"requiredAscension":4}])
        );
        let mut payload = json!({"characterDetails":p});
        validate(&payload).unwrap();
        payload["characterDetails"]["skills"]["11"]["icon"] = json!("https://example.com/icon");
        assert!(validate(&payload).is_err());
    }
}
