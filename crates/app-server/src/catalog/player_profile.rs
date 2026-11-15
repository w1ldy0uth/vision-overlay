use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub fn compile(tables: &BTreeMap<String, Value>) -> Result<Value> {
    let rows = |name: &str| {
        tables
            .get(name)
            .and_then(Value::as_array)
            .context("Missing profile source table")
    };
    let mut exp = BTreeMap::new();
    for r in rows("PlayerLevel")? {
        let rank = r["level"].as_u64().context("Missing Adventure Rank")?;
        if rank == 60 {
            ensure!(r["exp"].is_null(), "Unsupported max rank EXP");
            continue;
        }
        ensure!(
            (1..60).contains(&rank) && exp.insert(rank.to_string(), r["exp"].clone()).is_none(),
            "Invalid Adventure Rank source"
        );
    }
    let mut pictures = BTreeMap::new();
    for r in rows("ProfilePicture")? {
        let avatar = r["unlockParam"]
            .as_u64()
            .filter(|n| (10000000..20000000).contains(n));
        ensure!(
            pictures
                .insert(
                    r["id"].to_string(),
                    json!({"icon":r["iconPath"],"avatarId":avatar})
                )
                .is_none(),
            "Duplicate profile picture"
        );
    }
    let mut frames = BTreeMap::new();
    for r in rows("ProfileFrame")? {
        ensure!(frames.insert(r["id"].to_string(),json!({"icon":r["iconPath"],"iconPathHash":r["iconPathHash"].as_u64().map(|n|n.to_string())})).is_none(),"Duplicate profile frame");
    }
    Ok(json!({"schemaVersion":1,"adventureExp":exp,"pictures":pictures,"frames":frames}))
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(p) = payload.get("playerProfile") else {
        return Ok(());
    };
    ensure!(p["schemaVersion"] == 1, "Unsupported profile metadata");
    let exp = p["adventureExp"]
        .as_object()
        .context("Missing Adventure Rank EXP")?;
    ensure!(
        exp.len() == 59
            && (1..60).all(|r| exp
                .get(&r.to_string())
                .is_some_and(|v| v.as_u64().is_some_and(|n| n > 0 && n <= 1000000))),
        "Invalid Adventure Rank table"
    );
    for key in ["pictures", "frames"] {
        let rows = p[key]
            .as_object()
            .context("Missing profile decoration map")?;
        ensure!(rows.len() <= 4096, "Oversized profile decoration map");
        for (id, r) in rows {
            ensure!(
                super::positive_id(id) && super::icon(&r["icon"]),
                "Invalid profile icon"
            );
            if key == "pictures" {
                if let Some(avatar) = r["avatarId"].as_u64() {
                    ensure!(
                        payload["avatarNames"].get(avatar.to_string()).is_some(),
                        "Unknown profile picture avatar"
                    );
                }
            } else {
                ensure!(
                    r["iconPathHash"]
                        .as_str()
                        .is_some_and(|v| v.parse::<u64>().is_ok()),
                    "Invalid profile frame hash"
                );
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_tables_preserve_frame_hash_precision_and_reject_gaps_and_external_urls() {
        let levels: Vec<_> = (1..=60)
            .map(|n| {
                if n < 60 {
                    json!({"level":n,"exp":n*100})
                } else {
                    json!({"level":60})
                }
            })
            .collect();
        let tables = BTreeMap::from([
            ("PlayerLevel".into(), json!(levels)),
            (
                "ProfilePicture".into(),
                json!([{"id":10400,"iconPath":"UI_AvatarIcon_Flins_Circle","unlockParam":10000120}]),
            ),
            (
                "ProfileFrame".into(),
                json!([{"id":100017,"iconPathHash":7362869993646398554u64}]),
            ),
        ]);
        let p = compile(&tables).unwrap();
        assert_eq!(p["frames"]["100017"]["iconPathHash"], "7362869993646398554");
        let mut payload = json!({"playerProfile":p,"avatarNames":{"10000120":"Флинс"}});
        validate(&payload).unwrap();
        payload["playerProfile"]["adventureExp"]
            .as_object_mut()
            .unwrap()
            .remove("11");
        assert!(validate(&payload).is_err());
        payload["playerProfile"] = compile(&tables).unwrap();
        payload["playerProfile"]["pictures"]["10400"]["icon"] = json!("https://example.com/icon");
        assert!(validate(&payload).is_err());
    }
}
