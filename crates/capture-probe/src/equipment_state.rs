//! Verified incoming 7.2 command 5724: item 9, avatar GUID 10, item GUID 11, slot 12.
//! Ownership changes are emitted together so a GUID never belongs to two characters.
use anyhow::{Context, Result, ensure};
use protobuf::CodedInputStream;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn slot(name: &str) -> Option<u32> {
    Some(match name {
        "EQUIP_BRACER" => 1,
        "EQUIP_NECKLACE" => 2,
        "EQUIP_SHOES" => 3,
        "EQUIP_RING" => 4,
        "EQUIP_DRESS" => 5,
        "EQUIP_WEAPON" => 6,
        _ => return None,
    })
}
/// Read only a verified active catalog; standalone probes can use the raw source table.
pub fn load_types(local: &Path) -> Result<BTreeMap<u32, u32>> {
    let pointer = local.join("catalogs/active.json");
    if pointer.exists() {
        let p: Value = serde_json::from_slice(&std::fs::read(pointer)?)?;
        let id = p["current"].as_str().context("Missing catalog ID")?;
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid catalog ID"
        );
        let bytes = std::fs::read(local.join("catalogs/versions").join(format!("{id}.json")))?;
        ensure!(
            bytes.len() <= 32 * 1024 * 1024 && format!("{:x}", Sha256::digest(&bytes)) == id,
            "Invalid equipment catalog checksum"
        );
        let bundle: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            bundle["manifest"]["format_version"] == 1 && bundle["manifest"]["game"] == "genshin",
            "Unsupported equipment catalog"
        );
        return Ok(bundle["payload"]["items"]
            .as_object()
            .context("Missing equipment definitions")?
            .iter()
            .filter_map(|(id, row)| Some((id.parse().ok()?, slot(row["equipType"].as_str()?)?)))
            .collect());
    }
    let table: Value =
        serde_json::from_slice(&std::fs::read(local.join("ReliquaryExcelConfigData.json"))?)?;
    Ok(table
        .as_array()
        .context("Missing artifact source table")?
        .iter()
        .filter_map(|r| {
            Some((
                u32::try_from(r["id"].as_u64()?).ok()?,
                slot(r["equipType"].as_str()?)?,
            ))
        })
        .collect())
}

#[derive(Default)]
pub struct State {
    pub types: BTreeMap<u32, u32>,
    profiles: BTreeMap<String, Value>,
    items: BTreeMap<String, Value>,
}
impl State {
    pub fn reset(&mut self) {
        self.profiles.clear();
        self.items.clear();
    }
    pub fn snapshot_profiles(&mut self, profiles: &[Value]) {
        self.profiles = profiles
            .iter()
            .filter(|p| p["trial_info_present"] != true)
            .map(|p| {
                (
                    p["guid"].as_str().unwrap().to_owned(),
                    json!({"avatar_id":p["avatar_id"],"equip_guids":p["equip_guids"]}),
                )
            })
            .collect();
    }
    pub fn added_profile(&mut self, p: &Value) {
        self.profiles.insert(
            p["guid"].as_str().unwrap().into(),
            json!({"avatar_id":p["avatar_id"],"equip_guids":p["equip_guids"]}),
        );
    }
    pub fn snapshot_items(&mut self, items: &[Value]) {
        self.items.clear();
        for item in items {
            self.item(item);
        }
    }
    pub fn item(&mut self, item: &Value) {
        if let Some(guid) = item["guid"].as_str() {
            self.items.insert(guid.into(), item.clone());
        }
    }
    fn item_slot(&self, guid: &str) -> Result<u32> {
        let item = self
            .items
            .get(guid)
            .context("Equipment GUID missing from inventory")?;
        if item["kind"] == "weapon" {
            return Ok(6);
        }
        let id = item["item_id"]
            .as_u64()
            .context("Missing equipment item ID")?;
        self.types
            .get(&(u32::try_from(id)?))
            .copied()
            .context("Equipment slot missing from catalog")
    }
    pub fn apply(&mut self, body: &[u8]) -> Result<Vec<(&'static str, Value)>> {
        let (avatar, equip, item_id, equip_slot) = parse(body)?;
        let guid = avatar.to_string();
        let Some(target) = self.profiles.get(&guid) else {
            return Ok(vec![]);
        };
        let incoming = if equip == 0 {
            None
        } else {
            Some(equip.to_string())
        };
        if let Some(key) = &incoming {
            let item = self
                .items
                .get(key)
                .context("Equipped item missing from inventory")?;
            ensure!(
                item["item_id"] == item_id && self.item_slot(key)? == equip_slot,
                "Equipped item/slot mismatch"
            );
        }
        let mut updates = Vec::new();
        for (key, profile) in &self.profiles {
            let old = profile["equip_guids"]
                .as_array()
                .context("Missing equipment baseline")?;
            let mut next = Vec::new();
            for id in old {
                let id_str = id.as_str().context("Invalid equipment baseline GUID")?;
                if key == &guid && self.item_slot(id_str)? == equip_slot {
                    continue;
                }
                if incoming.as_deref() == Some(id_str) {
                    continue;
                }
                next.push(id.clone());
            }
            if key == &guid
                && let Some(id) = &incoming
            {
                next.push(json!(id));
            }
            // The baseline list has no ordering semantics. Preserve it for no-op notifications.
            if old
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                == next
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            {
                continue;
            }
            updates.push(
                json!({"guid":key,"avatar_id":profile["avatar_id"],"previous":old,"current":next}),
            );
        }
        if updates.is_empty() {
            return Ok(vec![]);
        }
        let result = json!({"command_id":5724,"guid":guid,"avatar_id":target["avatar_id"],"equip_type":equip_slot,"item_id":item_id,"item_guid":incoming,"updates":updates});
        for update in &updates {
            self.profiles
                .get_mut(update["guid"].as_str().unwrap())
                .unwrap()["equip_guids"] = update["current"].clone();
        }
        Ok(vec![("equipment_changed", result)])
    }
}
fn parse(body: &[u8]) -> Result<(u64, u64, u32, u32)> {
    ensure!(body.len() <= 65536, "Equipment notification too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut fields = BTreeMap::new();
    let mut count = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        count += 1;
        ensure!(count <= 256, "Too many equipment fields");
        let field = tag >> 3;
        if matches!(field, 9..=12) {
            ensure!(
                tag & 7 == 0 && !fields.contains_key(&field),
                "Duplicate/invalid equipment field"
            );
            fields.insert(field, input.read_uint64()?);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    let avatar = fields
        .get(&10)
        .copied()
        .filter(|v| *v > 0)
        .context("Missing equipment avatar GUID")?;
    let equip = fields.get(&11).copied().unwrap_or(0);
    let id = u32::try_from(fields.get(&9).copied().unwrap_or(0))?;
    let slot = u32::try_from(fields.get(&12).copied().unwrap_or(0))?;
    ensure!(
        (1..=6).contains(&slot) && (equip == 0) == (id == 0),
        "Invalid equipment slot/identity"
    );
    ensure!(slot != 6 || equip != 0, "Weapon cannot be unequipped");
    Ok((avatar, equip, id, slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn notify(avatar: u64, equip: u64, id: u32, slot: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            if id > 0 {
                out.write_uint32(9, id).unwrap();
            }
            out.write_uint64(10, avatar).unwrap();
            if equip > 0 {
                out.write_uint64(11, equip).unwrap();
            }
            out.write_uint32(12, slot).unwrap();
            out.flush().unwrap();
        }
        bytes
    }
    fn state() -> State {
        let mut s = State::default();
        s.types.insert(40524, 2);
        s.types.insert(32524, 2);
        s.snapshot_items(&[
            json!({"guid":"101","item_id":11517,"kind":"weapon"}),
            json!({"guid":"102","item_id":11401,"kind":"weapon"}),
            json!({"guid":"103","item_id":11402,"kind":"weapon"}),
            json!({"guid":"201","item_id":40524,"kind":"artifact"}),
            json!({"guid":"202","item_id":32524,"kind":"artifact"}),
        ]);
        s.snapshot_profiles(&[
            json!({"guid":"42","avatar_id":10000114,"equip_guids":["201","101"]}),
            json!({"guid":"43","avatar_id":10000089,"equip_guids":["202","102"]}),
        ]);
        s
    }
    #[test]
    fn weapon_replacement_transfer_and_server_swap_are_exclusive_and_idempotent() {
        let mut s = state();
        assert_eq!(s.apply(&notify(42, 103, 11402, 6)).unwrap().len(), 1);
        let transfer = s.apply(&notify(42, 102, 11401, 6)).unwrap();
        assert_eq!(transfer[0].1["updates"].as_array().unwrap().len(), 2);
        assert_eq!(s.profiles["43"]["equip_guids"], json!(["202"]));
        s.apply(&notify(43, 103, 11402, 6)).unwrap();
        assert_eq!(s.profiles["43"]["equip_guids"], json!(["202", "103"]));
        assert!(s.apply(&notify(43, 103, 11402, 6)).unwrap().is_empty());
    }
    #[test]
    fn artifact_removal_transfer_empty_source_and_all_five_slots() {
        let mut s = state();
        let removed = s.apply(&notify(42, 0, 0, 2)).unwrap();
        assert!(removed[0].1["item_guid"].is_null());
        assert!(s.apply(&notify(42, 0, 0, 2)).unwrap().is_empty());
        assert_eq!(
            s.apply(&notify(42, 202, 32524, 2)).unwrap()[0].1["updates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(s.profiles["43"]["equip_guids"], json!(["102"]));
        for slot in 1..=5 {
            let id = 50000 + slot;
            s.types.insert(id, slot);
            s.item(&json!({"guid":(300+slot).to_string(),"item_id":id,"kind":"artifact"}));
            s.apply(&notify(42, u64::from(300 + slot), id, slot))
                .unwrap();
        }
        assert_eq!(s.profiles["42"]["equip_guids"].as_array().unwrap().len(), 6);
    }
    #[test]
    fn invalid_or_unknown_items_do_not_partially_change_either_owner() {
        let mut s = state();
        let before = s.profiles.clone();
        for body in [
            notify(42, 202, 32524, 3),
            notify(42, 999, 32524, 2),
            notify(42, 0, 0, 6),
            notify(42, 202, 11401, 2),
        ] {
            assert!(s.apply(&body).is_err());
            assert_eq!(s.profiles, before);
        }
        let mut duplicate = notify(42, 202, 32524, 2);
        duplicate.extend([80, 42]);
        assert!(s.apply(&duplicate).is_err());
        assert_eq!(s.profiles, before);
    }
    #[test]
    fn session_reset_preserves_catalog_but_discards_owners_and_inventory() {
        let mut s = state();
        s.reset();
        assert_eq!(s.types.get(&40524), Some(&2));
        assert!(s.apply(&notify(42, 202, 32524, 2)).unwrap().is_empty());
        let max = u64::MAX;
        assert_eq!(
            parse(&notify(max, max - 1, 11517, 6)).unwrap(),
            (max, max - 1, 11517, 6)
        );
    }
}
