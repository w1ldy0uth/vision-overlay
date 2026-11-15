//! Level EXP and material-based Mora costs from the same pinned Excel revision.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct Ratio {
    numerator: u32,
    denominator: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Leveling {
    schema_version: u32,
    avatar_exp: BTreeMap<u32, u32>,
    weapon_exp: BTreeMap<u32, BTreeMap<u32, u32>>,
    avatar_materials: BTreeMap<u32, u32>,
    weapon_materials: BTreeMap<u32, u32>,
    avatar_mora_per_exp: Ratio,
    weapon_mora_per_exp: BTreeMap<u32, Ratio>,
    weapon_return_materials: Vec<u32>,
}
fn number(v: &Value) -> Result<u32> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .context("Invalid positive level integer")
}
fn ratio(s: &str) -> Result<Ratio> {
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    ensure!(
        fraction.len() <= 6
            && !whole.is_empty()
            && whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit()),
        "Unsupported cost ratio"
    );
    let denominator = 10u32.pow(fraction.len() as u32);
    let numerator = whole
        .parse::<u32>()?
        .checked_mul(denominator)
        .and_then(|n| n.checked_add(fraction.parse::<u32>().unwrap_or(0)))
        .context("Cost ratio overflow")?;
    ensure!(
        numerator > 0 && numerator <= denominator,
        "Invalid cost ratio"
    );
    let mut a = numerator;
    let mut b = denominator;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    Ok(Ratio {
        numerator: numerator / a,
        denominator: denominator / a,
    })
}
pub fn compile(tables: &BTreeMap<String, Value>) -> Result<Value> {
    let rows = |name: &str| {
        tables
            .get(name)
            .and_then(Value::as_array)
            .with_context(|| format!("Missing leveling table {name}"))
    };
    let mut avatar_exp = BTreeMap::new();
    for r in rows("AvatarLevel")? {
        ensure!(
            avatar_exp
                .insert(number(&r["level"])?, number(&r["exp"])?)
                .is_none(),
            "Duplicate avatar EXP level"
        );
    }
    let mut weapon_exp: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    for r in rows("WeaponLevel")? {
        let level = number(&r["level"])?;
        let exps = r["requiredExps"]
            .as_array()
            .filter(|v| v.len() == 5)
            .context("Unsupported weapon EXP rarities")?;
        for (index, exp) in exps.iter().enumerate() {
            ensure!(
                weapon_exp
                    .entry(index as u32 + 1)
                    .or_default()
                    .insert(level, number(exp)?)
                    .is_none(),
                "Duplicate weapon EXP level"
            );
        }
    }
    let mut avatar_materials = BTreeMap::new();
    let mut weapon_materials = BTreeMap::new();
    for r in rows("Material")? {
        let (target, op) = match r["materialType"].as_str() {
            Some("MATERIAL_EXP_FRUIT") => (&mut avatar_materials, "ITEM_USE_ADD_EXP"),
            Some("MATERIAL_WEAPON_EXP_STONE") => (&mut weapon_materials, "ITEM_USE_ADD_WEAPON_EXP"),
            _ => continue,
        };
        let actions: Vec<_> = r["itemUse"]
            .as_array()
            .context("Missing EXP item actions")?
            .iter()
            .filter(|a| a["useOp"] == op)
            .collect();
        ensure!(actions.len() == 1, "Ambiguous EXP item action");
        let exp = actions[0]["useParam"][0]
            .as_str()
            .context("Missing EXP item amount")?
            .parse::<u32>()?;
        ensure!(
            exp > 0 && target.insert(number(&r["id"])?, exp).is_none(),
            "Invalid/duplicate EXP material"
        );
    }
    let constants = rows("ConstValue")?;
    let coefficients = |name: &str| -> Result<Vec<Ratio>> {
        let matching: Vec<_> = constants.iter().filter(|r| r["name"] == name).collect();
        ensure!(matching.len() == 1, "Missing/duplicate level cost constant");
        matching[0]["value"]
            .as_array()
            .context("Missing level cost coefficients")?
            .iter()
            .filter(|v| v.as_str() != Some(""))
            .map(|v| ratio(v.as_str().context("Invalid level coefficient")?))
            .collect()
    };
    let avatar_rates = coefficients("CONST_VALUE_AVATAR_UPGRADE_COST_RATIO")?;
    ensure!(
        !avatar_rates.is_empty() && avatar_rates.iter().all(|r| r == &avatar_rates[0]),
        "Unsupported varying avatar material rates"
    );
    let weapon_rates = coefficients("CONST_VALUE_WEAPON_LEVELUP_COST")?;
    ensure!(weapon_rates.len() == 5, "Missing weapon rarity rates");
    let return_rows: Vec<_> = constants
        .iter()
        .filter(|r| r["name"] == "CONST_VALUE_UPGRADE_WEAPON_RETURN_MATERIAL_ID")
        .collect();
    ensure!(
        return_rows.len() == 1,
        "Missing weapon return material constant"
    );
    let weapon_return_materials = return_rows[0]["value"]
        .as_array()
        .context("Missing return material IDs")?
        .iter()
        .filter(|v| v.as_str() != Some(""))
        .map(|v| {
            v.as_str()
                .context("Invalid return material ID")?
                .parse::<u32>()
                .map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!(Leveling {
        schema_version: 1,
        avatar_exp,
        weapon_exp,
        avatar_materials,
        weapon_materials,
        avatar_mora_per_exp: avatar_rates[0].clone(),
        weapon_mora_per_exp: weapon_rates
            .into_iter()
            .enumerate()
            .map(|(i, r)| (i as u32 + 1, r))
            .collect(),
        weapon_return_materials,
    }))
}
pub fn validate(payload: &Value) -> Result<()> {
    let Some(value) = payload.get("leveling") else {
        return Ok(());
    };
    let l: Leveling = serde_json::from_value(value.clone()).context("Invalid leveling schema")?;
    ensure!(
        l.schema_version == 1 && serde_json::to_value(&l)? == *value,
        "Invalid leveling version/keys"
    );
    let levels = |rows: &BTreeMap<u32, u32>| -> Result<()> {
        ensure!(
            !rows.is_empty() && rows.len() <= 100,
            "Invalid EXP level count"
        );
        for (i, (level, exp)) in rows.iter().enumerate() {
            ensure!(
                *level as usize == i + 1 && *exp > 0 && *exp <= 100_000_000,
                "Invalid/gapped EXP levels"
            );
        }
        Ok(())
    };
    let valid_ratio = |r: &Ratio| {
        r.numerator > 0
            && r.denominator > 0
            && r.numerator <= r.denominator
            && r.denominator <= 1_000_000
    };
    levels(&l.avatar_exp)?;
    ensure!(
        valid_ratio(&l.avatar_mora_per_exp)
            && l.weapon_exp.len() == 5
            && l.weapon_mora_per_exp.len() == 5,
        "Invalid leveling rates/rarities"
    );
    for rarity in 1..=5 {
        levels(l.weapon_exp.get(&rarity).context("Missing weapon rarity")?)?;
        ensure!(
            valid_ratio(
                l.weapon_mora_per_exp
                    .get(&rarity)
                    .context("Missing weapon rate")?
            ),
            "Invalid weapon EXP rate"
        );
    }
    for items in [&l.avatar_materials, &l.weapon_materials] {
        ensure!(
            !items.is_empty() && items.len() <= 16,
            "Invalid EXP materials"
        );
        for (id, exp) in items {
            ensure!(
                *exp > 0 && *exp <= 1_000_000 && payload["materials"].get(id.to_string()).is_some(),
                "Invalid EXP material link"
            );
        }
    }
    let mut last = u32::MAX;
    let mut seen = std::collections::BTreeSet::new();
    ensure!(
        !l.weapon_return_materials.is_empty(),
        "Missing weapon return denominations"
    );
    for id in &l.weapon_return_materials {
        let exp = *l
            .weapon_materials
            .get(id)
            .context("Broken weapon return material link")?;
        ensure!(
            exp < last && seen.insert(id),
            "Unordered/duplicate return denominations"
        );
        last = exp;
    }
    // Every supported ascension cap must have all preceding EXP transitions.
    let p = &payload["progression"];
    for (key, exps) in [
        ("avatarPromotions", &l.avatar_exp),
        ("weaponPromotions", &l.weapon_exp[&5]),
    ] {
        for stages in p[key]
            .as_object()
            .context("Missing progression links")?
            .values()
        {
            for step in stages
                .as_object()
                .context("Invalid promotion path")?
                .values()
            {
                let cap = number(&step["maxLevel"])?;
                ensure!(
                    cap <= exps.len() as u32 + 1,
                    "EXP table does not cover ascension caps"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> BTreeMap<String, Value> {
        serde_json::from_str(include_str!("../../tests/fixtures/leveling-source.json")).unwrap()
    }
    fn payload() -> Value {
        let tables = fixture();
        let materials: serde_json::Map<_, _> = tables["Material"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["id"].to_string(), json!({})))
            .collect();
        json!({"leveling":compile(&tables).unwrap(),"materials":materials,"progression":{
            "avatarPromotions":{"1":{"6":{"maxLevel":90}}},"weaponPromotions":{"1":{"6":{"maxLevel":90}}}
        }})
    }
    #[test]
    fn source_fixture_matches_observed_levels_rates_and_weapon_refund_denominations() {
        let p = payload();
        validate(&p).unwrap();
        let l = &p["leveling"];
        assert_eq!(l["avatarExp"]["22"], 18000);
        assert_eq!(l["weaponExp"]["4"]["20"], 10500);
        assert_eq!(
            l["avatarMoraPerExp"],
            json!({"numerator":1,"denominator":5})
        );
        assert_eq!(
            l["weaponMoraPerExp"]["1"],
            json!({"numerator":1,"denominator":10})
        );
        assert_eq!(l["weaponReturnMaterials"], json!([104013, 104012, 104011]));
    }
    #[test]
    fn invalid_source_rows_and_broken_bundle_links_are_rejected() {
        let mut tables = fixture();
        tables.get_mut("WeaponLevel").unwrap()[0]["requiredExps"] = json!([1, 2]);
        assert!(compile(&tables).is_err());
        for mutation in 0..5 {
            let mut p = payload();
            match mutation {
                0 => {
                    p["leveling"]["avatarExp"]
                        .as_object_mut()
                        .unwrap()
                        .remove("22");
                }
                1 => p["leveling"]["weaponMoraPerExp"]["1"]["denominator"] = json!(0),
                2 => {
                    p["materials"].as_object_mut().unwrap().clear();
                }
                3 => p["progression"]["avatarPromotions"]["1"]["6"]["maxLevel"] = json!(100),
                _ => p["leveling"]["weaponReturnMaterials"] = json!([104011, 104013]),
            }
            assert!(validate(&p).is_err(), "mutation {mutation}");
        }
    }
    #[test]
    fn decimal_ratios_are_exact_and_invalid_values_are_rejected() {
        assert_eq!(
            ratio("0.2").unwrap(),
            Ratio {
                numerator: 1,
                denominator: 5
            }
        );
        assert_eq!(
            ratio("0.1").unwrap(),
            Ratio {
                numerator: 1,
                denominator: 10
            }
        );
        for invalid in ["0", "-0.2", "NaN", "0.0000001", "2", "", "0.x"] {
            assert!(ratio(invalid).is_err());
        }
    }
    #[test]
    fn legacy_catalogs_remain_readable_but_partial_leveling_is_rejected() {
        validate(&json!({})).unwrap();
        assert!(validate(&json!({"leveling":{"schemaVersion":1}})).is_err());
    }
}
