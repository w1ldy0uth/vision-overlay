//! Non-stackable equipment and furniture updates, keyed by exact GUID.
use crate::{
    inventory,
    material_state::{parse_deletions, parse_updates},
};
use auto_artifactarium::r#gen::protos::Item;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    ready: bool,
    items: BTreeMap<String, Value>,
}
impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn snapshot(&mut self, items: &[Item]) {
        self.items = inventory::non_materials(items)
            .into_iter()
            .map(|row| (row["guid"].as_str().unwrap().to_owned(), row))
            .collect();
        self.ready = true;
    }
    pub fn apply(
        &mut self,
        incoming: bool,
        command: u16,
        body: &[u8],
    ) -> anyhow::Result<Vec<Value>> {
        if !incoming || !self.ready || !matches!(command, 7608 | 24980) {
            return Ok(Vec::new());
        }
        let mut changes = Vec::new();
        if command == 7608 {
            let updates = parse_updates(body)?;
            for item in &updates {
                if let Some(old) = self.items.get(&item.guid.to_string()) {
                    anyhow::ensure!(
                        old["item_id"] == item.item_id && !item.has_material(),
                        "Item identity changed"
                    );
                }
            }
            for item in updates
                .iter()
                .filter(|item| !item.has_material() && item.item_id > 0 && item.guid > 0)
            {
                let row = inventory::non_materials(std::slice::from_ref(item)).remove(0);
                let guid = item.guid.to_string();
                if self.items.get(&guid) == Some(&row) {
                    continue;
                }
                if let Some(old) = self.items.get(&guid) {
                    anyhow::ensure!(old["item_id"] == item.item_id, "Item identity changed");
                }
                let previous = self.items.insert(guid.clone(), row.clone());
                changes.push(json!({"guid":guid,"item_id":item.item_id,"removed":false,"new_entry":previous.is_none(),"previous":previous,"item":row}));
            }
        } else {
            let mut guids: Vec<_> = parse_deletions(body)?.into_iter().collect();
            guids.sort_unstable();
            for guid in guids {
                if let Some(previous) = self.items.remove(&guid.to_string()) {
                    changes.push(json!({"guid":guid.to_string(),"item_id":previous["item_id"],"removed":true,"new_entry":false,"previous":previous,"item":null}));
                }
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::{CodedOutputStream, Message};
    fn update(items: &[Item]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            for item in items {
                out.write_bytes(1, &item.write_to_bytes().unwrap()).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    #[test]
    fn refinement_change_without_level_exp_or_ascension_change_is_a_single_event() {
        let mut weapon = Item::new();
        weapon.item_id = 12405;
        weapon.guid = 42;
        weapon.mut_equip().mut_weapon().level = 1;
        weapon.mut_equip().mut_weapon().affix_map.insert(112405, 0);
        let mut state = State::default();
        state.snapshot(&[weapon.clone()]);
        weapon.mut_equip().mut_weapon().affix_map.insert(112405, 1);
        let body = update(&[weapon]);
        let changes = state.apply(true, 7608, &body).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["previous"]["refinement"], 1);
        assert_eq!(changes[0]["item"]["refinement"], 2);
        assert_eq!(changes[0]["previous"]["level"], changes[0]["item"]["level"]);
        assert!(state.apply(true, 7608, &body).unwrap().is_empty());
    }
    #[test]
    fn exp_changes_without_a_level_change_are_events_and_duplicates_are_suppressed() {
        let mut item = Item::new();
        item.guid = 42;
        item.item_id = 42523;
        item.mut_equip().mut_reliquary().level = 20;
        item.mut_equip().mut_reliquary().exp = 4500;
        let mut state = State::default();
        state.snapshot(std::slice::from_ref(&item));
        item.mut_equip().mut_reliquary().exp = 18360;
        let body = update(&[item]);
        let changes = state.apply(true, 7608, &body).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["previous"]["exp"], 4500);
        assert_eq!(changes[0]["item"]["exp"], 18360);
        assert_eq!(changes[0]["item"]["level"], 19);
        assert!(state.apply(true, 7608, &body).unwrap().is_empty());
    }
    #[test]
    fn new_artifacts_with_same_id_remain_separate_and_updates_are_idempotent() {
        let mut state = State::default();
        state.snapshot(&[]);
        let mut item = Item::new();
        item.guid = u64::MAX;
        item.item_id = 77544;
        item.mut_equip().mut_reliquary().level = 1;
        item.mut_equip().mut_reliquary().main_prop_id = 10001;
        let mut second = item.clone();
        second.guid -= 1;
        let body = update(&[item.clone(), second]);
        assert!(state.apply(false, 7608, &body).unwrap().is_empty());
        let changes = state.apply(true, 7608, &body).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["item"]["main_prop_id"], 10001);
        assert!(state.apply(true, 7608, &body).unwrap().is_empty());
        item.mut_equip().mut_reliquary().level = 5;
        assert_eq!(
            state.apply(true, 7608, &update(&[item])).unwrap()[0]["item"]["level"],
            4
        );
        let mut broken = body.clone();
        broken.push(10);
        broken.push(255);
        assert!(state.apply(true, 7608, &broken).is_err());
        assert_eq!(state.items.len(), 2);
        let mut deletion = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut deletion);
            out.write_uint64(15, u64::MAX).unwrap();
            out.flush().unwrap();
        }
        let removed = state.apply(true, 24980, &deletion).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0]["removed"], true);
        assert!(state.apply(true, 24980, &deletion).unwrap().is_empty());
        state.reset();
        assert!(state.apply(true, 7608, &body).unwrap().is_empty());
    }
}
