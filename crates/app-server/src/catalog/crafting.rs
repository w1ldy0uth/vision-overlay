//! Only default deterministic tier upgrades. Conversions/unlocked recipes are separate.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub fn compile(rows: &Value) -> Result<Value> {
    let mut recipes = BTreeMap::new();
    for r in rows.as_array().context("Missing Combine source")? {
        if !matches!(r["combineType"].as_u64(), Some(1 | 2 | 3 | 5))
            || r["recipeType"] != "RECIPE_TYPE_COMBINE"
            || r["isDefaultShow"] != true
        {
            continue;
        }
        let input: Vec<_> = r["materialItems"]
            .as_array()
            .context("Missing combine ingredients")?
            .iter()
            .filter(|i| i.get("id").is_some())
            .collect();
        ensure!(
            input.len() == 1
                && input[0]["count"] == 3
                && r["resultItemCount"] == 1
                && r["dropId"].is_null(),
            "Unsupported tier recipe shape"
        );
        ensure!(
            r.get("scoinCost").is_none_or(|v| v.as_u64().is_some())
                && r.get("playerLevel").is_none_or(|v| v.as_u64().is_some()),
            "Invalid source crafting cost/rank"
        );
        let result = r["resultItemId"]
            .as_u64()
            .context("Missing combine result")?;
        ensure!(recipes.insert(result.to_string(),json!({"recipeId":r["combineId"],"inputId":input[0]["id"],"inputCount":3,"outputCount":1,"mora":r["scoinCost"].as_u64().unwrap_or(0),"requiredAdventureRank":r["playerLevel"].as_u64().unwrap_or(0)})).is_none(),"Ambiguous tier recipe");
    }
    Ok(json!({"schemaVersion":1,"recipes":recipes}))
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(c) = payload.get("crafting") else {
        return Ok(());
    };
    ensure!(c["schemaVersion"] == 1, "Unsupported crafting schema");
    let recipes = c["recipes"]
        .as_object()
        .context("Missing crafting recipes")?;
    ensure!(recipes.len() <= 1000, "Too many crafting recipes");
    let mut ids = BTreeSet::new();
    for (id, r) in recipes {
        ensure!(
            super::positive_id(id) && payload["materials"].get(id).is_some(),
            "Invalid crafting result link"
        );
        let input = r["inputId"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= u32::MAX as u64)
            .context("Invalid crafting input")?
            .to_string();
        ensure!(
            payload["materials"].get(&input).is_some()
                && input != *id
                && r["inputCount"] == 3
                && r["outputCount"] == 1,
            "Invalid crafting input link/count"
        );
        ensure!(
            r["recipeId"]
                .as_u64()
                .is_some_and(|n| n > 0 && ids.insert(n))
                && r["mora"].as_u64().is_some_and(|n| n <= 1000000)
                && r["requiredAdventureRank"].as_u64().is_some_and(|n| n <= 60),
            "Invalid crafting cost/requirement"
        );
        let mut seen = BTreeSet::new();
        let mut next = id.to_owned();
        while let Some(recipe) = recipes.get(&next) {
            ensure!(
                seen.insert(next) && seen.len() <= 16,
                "Cyclic/deep crafting path"
            );
            next = recipe["inputId"].to_string();
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tier_recipes_exclude_conversion_and_validate_links_cycles_and_costs() {
        let row = json!({"combineId":1,"combineType":5,"recipeType":"RECIPE_TYPE_COMBINE","isDefaultShow":true,"materialItems":[{"id":1,"count":3},{},{}],"resultItemId":2,"resultItemCount":1,"scoinCost":300,"playerLevel":25});
        let mut conversion = row.clone();
        conversion["recipeType"] = json!("RECIPE_TYPE_CONVERT");
        let c = compile(&json!([row.clone(), conversion])).unwrap();
        assert_eq!(c["recipes"].as_object().unwrap().len(), 1);
        let p = json!({"crafting":c,"materials":{"1":{},"2":{}}});
        validate(&p).unwrap();
        let mut bad = p.clone();
        bad["crafting"]["recipes"]["2"]["inputId"] = json!(99);
        assert!(validate(&bad).is_err());
        let mut bad = p.clone();
        bad["crafting"]["recipes"]["1"] = json!({"recipeId":2,"inputId":2,"inputCount":3,"outputCount":1,"mora":1,"requiredAdventureRank":1});
        assert!(validate(&bad).is_err());
        let mut bad = row.clone();
        bad["resultItemCount"] = json!(2);
        assert!(compile(&json!([bad])).is_err());
        assert!(compile(&json!([row.clone(), row])).is_err());
    }
}
