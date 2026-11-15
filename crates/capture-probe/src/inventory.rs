use auto_artifactarium::r#gen::protos::Item;
use serde_json::{Value, json};

/// Equipment instances retain their GUID; equal item IDs must not be merged.
pub fn non_materials(items: &[Item]) -> Vec<Value> {
    items.iter().filter(|item| !item.has_material()).map(|item| {
        let mut row=json!({"item_id":item.item_id,"guid":item.guid.to_string(),"kind":"unknown","count":null});
        if item.has_furniture() {
            row["kind"]=json!("furniture"); row["count"]=json!(item.furniture().count);
        } else if item.has_equip() {
            let equip=item.equip(); row["locked"]=json!(equip.is_locked);
            row["count"]=json!(1);
            if equip.has_weapon() {
                let weapon=equip.weapon();
                row["kind"]=json!("weapon");row["level"]=json!(weapon.level);
                row["ascension"]=json!(weapon.promote_level);
                row["exp"]=json!(weapon.exp);
                row["refinement"]=json!(weapon.affix_map.values().next().copied().unwrap_or_default().checked_add(1));
            } else if equip.has_reliquary() {
                let artifact=equip.reliquary();
                row["kind"]=json!("artifact");row["level"]=json!(artifact.level.checked_sub(1));
                row["main_prop_id"]=json!(artifact.main_prop_id);
                row["exp"]=json!(artifact.exp);
                row["ascension"]=json!(artifact.promote_level);
                row["starred"]=json!(artifact.starred);
                row["append_prop_ids"]=json!(artifact.append_prop_id_list);
                row["unactivated_prop_ids"]=json!(artifact.unactivated_prop_id_list);
            }
        }
        if row["kind"]=="unknown" && let Some((kind,count))=extended_stack(item) {
            row["kind"]=json!(kind); row["count"]=json!(count);
        }
        if row["kind"]=="unknown" && let Some(accessories)=tps_accessories(item) {
            row["kind"]=json!("tps_weapon"); row["count"]=json!(1);
            row["accessory_ids"]=json!(accessories);
        }
        if row["kind"]=="unknown" {
            row["unknown_details"]=json!(item.special_fields.unknown_fields().iter().map(|(field,value)|match value {
                protobuf::UnknownValueRef::LengthDelimited(bytes)=>json!({"field":field,"length":bytes.len(),"numeric_fields":crate::diagnostics::Diagnostics::default().structure(0,bytes)}),
                _=>json!({"field":field,"wire_type":"non_message"}),
            }).collect::<Vec<_>>());
        }
        row
    }).collect()
}

/// Observed 7.2 Item details: field 8 Beyond material, field 9 hall facility.
/// Both carry absolute uint32 count at nested field 1. Field 10 carries TPS weapon details.
pub fn extended_stack(item: &Item) -> Option<(&'static str, u32)> {
    use protobuf::CodedInputStream;
    let mut result = None;
    for (field, value) in item.special_fields.unknown_fields().iter() {
        if !matches!(field, 8 | 9) {
            continue;
        }
        if result.is_some() {
            return None;
        }
        let protobuf::UnknownValueRef::LengthDelimited(bytes) = value else {
            return None;
        };
        if bytes.len() > 65536 {
            return None;
        }
        let mut input = CodedInputStream::from_bytes(bytes);
        let mut count = None;
        let mut fields = 0;
        while let Some(tag) = input.read_raw_tag_or_eof().ok()? {
            fields += 1;
            if fields > 1024 {
                return None;
            }
            if tag == 8 {
                if count.is_some() {
                    return None;
                }
                count = Some(input.read_uint32().ok()?);
            } else {
                if tag >> 3 == 1 {
                    return None;
                }
                protobuf::rt::skip_field_for_tag(tag, &mut input).ok()?;
            }
        }
        result = Some((
            if field == 8 {
                "beyond_material"
            } else {
                "beyond_facility"
            },
            count.unwrap_or(0),
        ));
    }
    result
}

/// Item.10 TPS weapon, repeated uint32 accessory IDs at nested field 2.
/// The GUID represents one weapon instance, not a stack of accessories.
pub fn tps_accessories(item: &Item) -> Option<Vec<u32>> {
    use protobuf::CodedInputStream;
    let mut result = None;
    for (field, value) in item.special_fields.unknown_fields().iter() {
        if field != 10 {
            continue;
        }
        if result.is_some() {
            return None;
        }
        let protobuf::UnknownValueRef::LengthDelimited(bytes) = value else {
            return None;
        };
        if bytes.len() > 65536 {
            return None;
        }
        let mut input = CodedInputStream::from_bytes(bytes);
        let mut ids = Vec::new();
        let mut fields = 0;
        while let Some(tag) = input.read_raw_tag_or_eof().ok()? {
            fields += 1;
            if fields > 1024 {
                return None;
            }
            match tag {
                16 => ids.push(input.read_uint32().ok()?),
                18 => {
                    let packed = input.read_bytes().ok()?;
                    let mut packed = CodedInputStream::from_bytes(&packed);
                    while !packed.eof().ok()? {
                        if ids.len() >= 1024 {
                            return None;
                        }
                        ids.push(packed.read_uint32().ok()?);
                    }
                }
                _ => {
                    if tag >> 3 == 2 {
                        return None;
                    }
                    protobuf::rt::skip_field_for_tag(tag, &mut input).ok()?;
                }
            }
        }
        result = Some(ids);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tps_weapons_preserve_accessories_packed_and_unpacked_without_treating_them_as_quantity() {
        let mut item = Item::new();
        item.item_id = 224001;
        item.guid = 4;
        item.special_fields
            .mut_unknown_fields()
            .add_length_delimited(10, vec![18, 6, 0xf9, 0xd9, 13, 0xfa, 0xd9, 13, 16, 7]);
        let row = non_materials(&[item.clone()]).remove(0);
        assert_eq!(row["kind"], "tps_weapon");
        assert_eq!(row["count"], 1);
        assert_eq!(row["accessory_ids"], json!([224505, 224506, 7]));
        item.special_fields.clear();
        item.special_fields
            .mut_unknown_fields()
            .add_length_delimited(10, vec![18, 1, 0xff]);
        assert!(tps_accessories(&item).is_none());
        assert_eq!(non_materials(&[item])[0]["kind"], "unknown");
    }
    #[test]
    fn extended_stacks_preserve_counts_and_unknown_details_are_not_assumed_owned_cosmetics() {
        let mut material = Item::new();
        material.item_id = 250005;
        material.guid = 1;
        material
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(8, vec![8, 0xd8, 4]);
        let mut facility = Item::new();
        facility.item_id = 351101;
        facility.guid = 2;
        facility
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(9, vec![8, 1]);
        let mut unknown = Item::new();
        unknown.item_id = 224001;
        unknown.guid = 3;
        unknown
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(11, vec![18, 3, 0xf9, 0xd9, 13]);
        let rows = non_materials(&[material.clone(), facility, unknown]);
        assert_eq!(rows[0]["kind"], "beyond_material");
        assert_eq!(rows[0]["count"], 600);
        assert_eq!(rows[1]["kind"], "beyond_facility");
        assert_eq!(rows[1]["count"], 1);
        assert_eq!(rows[2]["kind"], "unknown");
        assert!(rows[2]["count"].is_null());
        assert_eq!(rows[2]["unknown_details"][0]["field"], 11);
        material
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(8, vec![8, 1]);
        assert!(extended_stack(&material).is_none());
        let mut malformed = Item::new();
        malformed
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(8, vec![8, 0xff]);
        assert!(extended_stack(&malformed).is_none());
        let mut zero = Item::new();
        zero.special_fields
            .mut_unknown_fields()
            .add_length_delimited(8, vec![]);
        assert_eq!(extended_stack(&zero), Some(("beyond_material", 0)));
    }
    #[test]
    fn preserves_distinct_instances_and_converts_display_levels() {
        let mut first = Item::new();
        first.item_id = 11101;
        first.guid = u64::MAX;
        let equip = first.mut_equip();
        equip.is_locked = true;
        let weapon = equip.mut_weapon();
        weapon.level = 90;
        weapon.promote_level = 6;
        weapon.affix_map.insert(1, 4);
        let mut second = first.clone();
        second.guid = u64::MAX - 1;
        let mut artifact = Item::new();
        artifact.item_id = 20002;
        artifact.guid = 3;
        artifact.mut_equip().mut_reliquary().level = 21;
        artifact.mut_equip().mut_reliquary().main_prop_id = 10001;
        artifact.mut_equip().mut_reliquary().append_prop_id_list = vec![501031, 501032];
        let mut material = Item::new();
        material.item_id = 100051;
        material.mut_material().count = 1;
        let mut furniture = Item::new();
        furniture.guid = 4;
        furniture.mut_furniture().count = 3;
        let rows = non_materials(&[first, second, artifact, material, furniture]);
        assert_eq!(rows.len(), 4);
        assert_ne!(rows[0]["guid"], rows[1]["guid"]);
        assert_eq!(rows[0]["guid"], u64::MAX.to_string());
        assert_eq!(rows[0]["refinement"], 5);
        assert_eq!(rows[2]["level"], 20);
        assert_eq!(rows[2]["main_prop_id"], 10001);
        assert_eq!(rows[2]["append_prop_ids"], json!([501031, 501032]));
        assert_eq!(rows[3]["count"], 3);
    }
}
