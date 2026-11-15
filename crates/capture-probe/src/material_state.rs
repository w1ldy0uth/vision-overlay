//! Protocol 7.2: incoming 7608 Item counts and 24980 field-15 GUID deletions.
use auto_artifactarium::r#gen::protos::Item;
use protobuf::{CodedInputStream, Message};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Serialize)]
pub struct Stock {
    pub item_id: u32,
    pub guid: String,
    pub count: u32,
}

#[derive(Serialize)]
pub struct Change {
    pub removed: bool,
    pub new_entry: bool,
    pub item_id: u32,
    pub guid: String,
    pub previous_count: u32,
    pub count: u32,
    pub delta: i64,
}

#[derive(Default)]
pub struct State {
    ready: bool,
    materials: BTreeMap<u64, Stock>,
}

impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn snapshot(&mut self, items: &[Item]) {
        self.reset();
        for item in items.iter().filter(|item| item.has_material()) {
            self.materials.insert(
                item.guid,
                Stock {
                    item_id: item.item_id,
                    guid: item.guid.to_string(),
                    count: item.material().count,
                },
            );
        }
        self.ready = true;
    }

    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"snapshot_available":self.ready,"materials":self.materials.values().collect::<Vec<_>>(),
            "coverage":"material_item_updates_protocol_7_2","new_guids_supported":true,"deletion_supported":true,"zero_rows_retained":true})
    }

    pub fn apply(
        &mut self,
        incoming: bool,
        command: u16,
        body: &[u8],
    ) -> anyhow::Result<(Vec<Change>, usize)> {
        if !incoming || !matches!(command, 7608 | 24980) || !self.ready {
            return Ok((Vec::new(), 0));
        }
        anyhow::ensure!(body.len() <= 262144, "Material update exceeds size limit");
        if command == 24980 {
            return self.delete(body);
        }
        let updates = parse_updates(body)?;
        let mut changes = Vec::new();
        let mut ignored = 0;
        for item in updates {
            if !item.has_material() {
                if item.guid == 0
                    || item.item_id == 0
                    || (!item.has_equip()
                        && !item.has_furniture()
                        && crate::inventory::extended_stack(&item).is_none()
                        && crate::inventory::tps_accessories(&item).is_none())
                {
                    ignored += 1;
                }
                continue;
            }
            if item.guid == 0 || item.item_id == 0 {
                ignored += 1;
                continue;
            }
            let new_entry = !self.materials.contains_key(&item.guid);
            if new_entry && item.material().count == 0 {
                ignored += 1;
                continue;
            }
            let stock = self.materials.entry(item.guid).or_insert_with(|| Stock {
                item_id: item.item_id,
                guid: item.guid.to_string(),
                count: 0,
            });
            if !item.has_material() || item.item_id != stock.item_id {
                ignored += 1;
                continue;
            }
            let count = item.material().count;
            if stock.count == count {
                continue;
            }
            changes.push(Change {
                removed: false,
                new_entry,
                item_id: stock.item_id,
                guid: stock.guid.clone(),
                previous_count: stock.count,
                count,
                delta: i64::from(count) - i64::from(stock.count),
            });
            stock.count = count;
        }
        Ok((changes, ignored))
    }

    fn delete(&mut self, body: &[u8]) -> anyhow::Result<(Vec<Change>, usize)> {
        let guids = parse_deletions(body)?;
        let mut changes = Vec::new();
        let mut ignored = 0;
        // Deterministic order for a message containing multiple deletions.
        let mut guids: Vec<_> = guids.into_iter().collect();
        guids.sort_unstable();
        for guid in guids {
            let Some(stock) = self.materials.get_mut(&guid) else {
                ignored += 1;
                continue;
            };
            if stock.count == 0 {
                continue;
            }
            changes.push(Change {
                removed: true,
                new_entry: false,
                item_id: stock.item_id,
                guid: stock.guid.clone(),
                previous_count: stock.count,
                count: 0,
                delta: -i64::from(stock.count),
            });
            stock.count = 0;
        }
        Ok((changes, ignored))
    }
}

pub(crate) fn parse_updates(body: &[u8]) -> anyhow::Result<Vec<Item>> {
    anyhow::ensure!(body.len() <= 262144, "Item update exceeds size limit");
    let mut input = CodedInputStream::from_bytes(body);
    let mut updates = Vec::new();
    let mut seen = HashSet::new();
    // Parse the complete envelope before mutating: malformed tails must not partly apply.
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        if tag >> 3 == 1 {
            anyhow::ensure!(tag == 10, "Unexpected wire type for Item list");
            let item = Item::parse_from_bytes(&input.read_bytes()?)?;
            anyhow::ensure!(updates.len() < 4096, "Too many material update entries");
            anyhow::ensure!(seen.insert(item.guid), "Duplicate GUID in update envelope");
            updates.push(item);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    Ok(updates)
}
pub(crate) fn parse_deletions(body: &[u8]) -> anyhow::Result<HashSet<u64>> {
    anyhow::ensure!(body.len() <= 262144, "Deletion exceeds size limit");
    let mut input = CodedInputStream::from_bytes(body);
    let mut guids = HashSet::new();
    let mut add = |guid| -> anyhow::Result<()> {
        anyhow::ensure!(
            guid > 0 && guids.len() < 4096 && guids.insert(guid),
            "Invalid or duplicate deletion GUID"
        );
        Ok(())
    };
    // Read the whole envelope before applying any deletion.
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            120 => add(input.read_uint64()?)?, // protobuf also permits unpacked repeated uint64
            122 => {
                let bytes = input.read_bytes()?;
                let mut packed = CodedInputStream::from_bytes(&bytes);
                while !packed.eof()? {
                    add(packed.read_uint64()?)?;
                }
            }
            _ => {
                anyhow::ensure!(tag >> 3 != 15, "Unexpected deletion GUID wire type");
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        }
    }
    Ok(guids)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deletion(guids: &[u64], packed: bool) -> Vec<u8> {
        let mut body = Vec::new();
        {
            let mut out = protobuf::CodedOutputStream::vec(&mut body);
            if packed {
                let mut values = Vec::new();
                {
                    let mut values_out = protobuf::CodedOutputStream::vec(&mut values);
                    for guid in guids {
                        values_out.write_uint64_no_tag(*guid).unwrap();
                    }
                    values_out.flush().unwrap();
                }
                out.write_bytes(15, &values).unwrap();
            } else {
                for guid in guids {
                    out.write_uint64(15, *guid).unwrap();
                }
            }
            out.flush().unwrap();
        }
        body
    }
    #[test]
    fn deletion_zeroes_known_material_and_is_idempotent_and_session_scoped() {
        for packed in [true, false] {
            let mut state = State::default();
            state.snapshot(&[item(3)]);
            let body = deletion(&[999, 1000], packed);
            assert!(state.apply(false, 24980, &body).unwrap().0.is_empty());
            let (changes, ignored) = state.apply(true, 24980, &body).unwrap();
            assert_eq!(ignored, 1);
            assert_eq!(changes.len(), 1);
            assert!(changes[0].removed);
            assert!(!changes[0].new_entry);
            assert_eq!(changes[0].previous_count, 3);
            assert_eq!(changes[0].count, 0);
            assert_eq!(changes[0].delta, -3);
            assert_eq!(state.materials[&999].count, 0);
            assert!(!state.materials.contains_key(&1000));
            assert!(state.apply(true, 24980, &body).unwrap().0.is_empty());
            let mut created = item(1);
            created.guid = 2000;
            assert!(state.apply(true, 7608, &envelope(&[created])).unwrap().0[0].new_entry);
            assert_eq!(
                state
                    .materials
                    .values()
                    .map(|stock| stock.count)
                    .sum::<u32>(),
                1
            );
            state.reset();
            assert!(state.apply(true, 24980, &body).unwrap().0.is_empty());
        }
    }
    #[test]
    fn malformed_deletion_does_not_partly_zero_inventory() {
        let mut state = State::default();
        state.snapshot(&[item(3)]);
        let mut truncated = deletion(&[999], true);
        truncated.extend([122, 255]);
        for body in [
            truncated,
            deletion(&[999, 999], true),
            deletion(&[999, 0], false),
        ] {
            assert!(state.apply(true, 24980, &body).is_err());
            assert_eq!(state.materials[&999].count, 3);
        }
    }
    fn item(count: u32) -> Item {
        let mut item = Item::new();
        item.item_id = 100011;
        item.guid = 999;
        item.mut_material().count = count;
        item
    }
    fn envelope(items: &[Item]) -> Vec<u8> {
        let mut result = Vec::new();
        {
            let mut out = protobuf::CodedOutputStream::vec(&mut result);
            for item in items {
                out.write_message(1, item).unwrap();
            }
            out.flush().unwrap();
        }
        result
    }
    #[test]
    fn absolute_pickups_spend_and_repeated_delivery() {
        let mut state = State::default();
        state.snapshot(&[item(315)]);
        for (count, delta) in [(316, 1), (317, 1), (316, -1)] {
            let body = envelope(&[item(count)]);
            let (changes, ignored) = state.apply(true, 7608, &body).unwrap();
            assert_eq!(ignored, 0);
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].delta, delta);
            assert_eq!(state.materials[&999].count, count);
            assert!(state.apply(true, 7608, &body).unwrap().0.is_empty());
        }
    }
    #[test]
    fn requires_snapshot_direction_command_and_current_session() {
        let mut state = State::default();
        let body = envelope(&[item(316)]);
        assert!(state.apply(true, 7608, &body).unwrap().0.is_empty());
        state.snapshot(&[item(315)]);
        assert!(state.apply(false, 7608, &body).unwrap().0.is_empty());
        assert!(state.apply(true, 5662, &body).unwrap().0.is_empty());
        state.reset();
        assert!(state.apply(true, 7608, &body).unwrap().0.is_empty());
        state.snapshot(&[item(42)]);
        assert_eq!(state.materials[&999].count, 42);
    }
    #[test]
    fn ignores_mismatched_items_and_accepts_explicit_zero() {
        let mut state = State::default();
        state.snapshot(&[item(315)]);
        let mut mismatch = item(8);
        mismatch.item_id = 1;
        for value in [mismatch, Item::new()] {
            let (changes, ignored) = state.apply(true, 7608, &envelope(&[value])).unwrap();
            assert!(changes.is_empty());
            assert_eq!(ignored, 1);
        }
        state.apply(true, 7608, &envelope(&[item(0)])).unwrap();
        assert_eq!(state.materials[&999].count, 0);
    }
    #[test]
    fn learns_new_material_guid_from_update_and_deduplicates_delivery() {
        let mut state = State::default();
        state.snapshot(&[item(315)]);
        let mut crafted = item(1);
        crafted.guid = 1000;
        crafted.item_id = 113082;
        let body = envelope(&[crafted.clone()]);
        let (changes, ignored) = state.apply(true, 7608, &body).unwrap();
        assert_eq!(ignored, 0);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].new_entry);
        assert_eq!(changes[0].previous_count, 0);
        assert_eq!(changes[0].delta, 1);
        assert!(state.apply(true, 7608, &body).unwrap().0.is_empty());
        crafted.mut_material().count = 2;
        let (changes, _) = state.apply(true, 7608, &envelope(&[crafted])).unwrap();
        assert!(!changes[0].new_entry);
        assert_eq!(changes[0].previous_count, 1);
        state.snapshot(&[item(42)]);
        assert!(!state.materials.contains_key(&1000));
        state.reset();
        assert!(state.apply(true, 7608, &body).unwrap().0.is_empty());
    }
    #[test]
    fn malformed_or_duplicate_envelopes_are_atomic() {
        let mut state = State::default();
        state.snapshot(&[item(315)]);
        let mut body = envelope(&[item(316)]);
        body.extend([10, 255]);
        assert!(state.apply(true, 7608, &body).is_err());
        assert!(
            state
                .apply(true, 7608, &envelope(&[item(316), item(317)]))
                .is_err()
        );
        assert_eq!(state.materials[&999].count, 315);
    }
}
