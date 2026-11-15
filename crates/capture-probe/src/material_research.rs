//! Structural observations only: no command semantics or stock mutations are assumed.
use auto_artifactarium::r#gen::protos::Item;
use protobuf::Message;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct Observation {
    pub path: String,
    pub item_id: u32,
    pub guid: String,
    pub count: u32,
    pub snapshot_count: u32,
    pub differs_from_snapshot: bool,
}

pub type KnownMaterials = HashMap<u64, (u32, u32)>;

pub fn scan(bytes: &[u8], known: &KnownMaterials) -> Vec<Observation> {
    if bytes.len() > 262144 || known.is_empty() {
        return Vec::new();
    }
    let mut budget = 8192;
    let mut output = Vec::new();
    walk(bytes, "body", 0, known, &mut budget, &mut output);
    output
}

fn varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Some(value);
        }
    }
    None
}

fn walk(
    bytes: &[u8],
    path: &str,
    depth: usize,
    known: &KnownMaterials,
    budget: &mut usize,
    output: &mut Vec<Observation>,
) {
    if depth > 8 || *budget == 0 || output.len() >= 256 {
        return;
    }
    *budget -= 1;
    if let Ok(item) = Item::parse_from_bytes(bytes)
        && let Some(&(id, count)) = known.get(&item.guid)
        && item.item_id == id
        && item.has_material()
    {
        let value = item.material().count;
        output.push(Observation {
            path: path.into(),
            item_id: id,
            guid: item.guid.to_string(),
            count: value,
            snapshot_count: count,
            differs_from_snapshot: value != count,
        });
        return;
    }
    let mut cursor = 0;
    let mut occurrences = HashMap::<u64, usize>::new();
    while cursor < bytes.len() && *budget > 0 {
        *budget -= 1;
        let Some(tag) = varint(bytes, &mut cursor) else {
            return;
        };
        let field = tag >> 3;
        if field == 0 || field > 536870911 {
            return;
        }
        match tag & 7 {
            0 => {
                if varint(bytes, &mut cursor).is_none() {
                    return;
                }
            }
            1 | 5 => {
                let size = if tag & 7 == 1 { 8 } else { 4 };
                let Some(end) = cursor.checked_add(size) else {
                    return;
                };
                if end > bytes.len() {
                    return;
                }
                cursor = end;
            }
            2 => {
                let Some(size) = varint(bytes, &mut cursor).and_then(|n| usize::try_from(n).ok())
                else {
                    return;
                };
                let Some(end) = cursor.checked_add(size) else {
                    return;
                };
                let Some(nested) = bytes.get(cursor..end) else {
                    return;
                };
                let index = occurrences.entry(field).or_default();
                let next_path = format!("{path}.{field}[{index}]");
                *index += 1;
                walk(nested, &next_path, depth + 1, known, budget, output);
                cursor = end;
            }
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_matching_guid_id_and_material_detail_and_preserves_repeats() {
        let mut item = Item::new();
        item.item_id = 100051;
        item.guid = 999;
        item.mut_material().count = 3;
        let bytes = item.write_to_bytes().unwrap();
        let mut wrapped = Vec::new();
        for _ in 0..2 {
            wrapped.extend([18, bytes.len() as u8]);
            wrapped.extend(&bytes);
        }
        let known = HashMap::from([(999, (100051, 2))]);
        let rows = scan(&wrapped, &known);
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].path, rows[1].path);
        assert!(rows[0].differs_from_snapshot);
        assert_eq!(rows[0].count, 3);
        assert!(scan(&bytes, &HashMap::from([(999, (100018, 2))])).is_empty());
        assert!(scan(&bytes, &HashMap::from([(998, (100051, 2))])).is_empty());
        item.mut_material().count = 0;
        assert_eq!(scan(&item.write_to_bytes().unwrap(), &known)[0].count, 0);
    }
    #[test]
    fn malformed_fields_do_not_panic_or_create_stock_changes() {
        let known = HashMap::from([(1, (1, 1))]);
        for bytes in [&[8, 128][..], &[18, 255, 255][..], &[0][..]] {
            assert!(scan(bytes, &known).is_empty());
        }
        assert!(scan(&vec![1; 262145], &known).is_empty());
    }
}
