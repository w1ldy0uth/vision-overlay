//! Offline research of public player identity and Adventure Rank properties.
//! Login packet handling exports only UID; credential-bearing fields are skipped.
use anyhow::{Result, ensure};
use auto_artifactarium::r#gen::protos::PropValue;
use protobuf::{CodedInputStream, Message};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub fn parse(command: u16, body: &[u8]) -> Result<Option<Value>> {
    if !matches!(command, 27808 | 7193 | 22660 | 9233) {
        return Ok(None);
    }
    ensure!(
        body.len() <= 2 * 1024 * 1024,
        "Player research message too large"
    );
    if command == 9233 {
        return social(body).map(Some);
    }
    let mut input = CodedInputStream::from_bytes(body);
    let mut uid = None;
    let mut properties = BTreeMap::new();
    let mut texts = BTreeMap::new();
    let mut scalar = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut count = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        count += 1;
        ensure!(count <= 8192, "Too many player fields");
        let field = tag >> 3;
        if command == 27808 {
            if field == 9 {
                ensure!(tag & 7 == 0 && uid.is_none(), "Invalid UID field");
                let value = input.read_uint32()?;
                ensure!(value > 0, "Missing UID");
                uid = Some(value.to_string());
            } else {
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        } else if field == if command == 7193 { 6 } else { 8 } {
            ensure!(tag & 7 == 2, "Invalid property map wire type");
            let bytes = input.read_bytes()?;
            let (key, bytes) = crate::profile_protocol::entry(&bytes)?;
            ensure!(seen.insert(key), "Duplicate player property");
            if matches!(key, 10013 | 10014 | 10019 | 10039 | 10040) {
                let prop = PropValue::parse_from_bytes(&bytes)?;
                ensure!(
                    prop.type_ == key as u32 && !prop.has_fval(),
                    "Invalid player property value"
                );
                properties.insert(key,json!({"value":if prop.has_ival(){prop.ival()}else{prop.val},"ival_present":prop.has_ival(),"raw_val":prop.val}));
            }
        } else if command == 7193 && tag & 7 == 2 {
            let bytes = input.read_bytes()?;
            if bytes.len() <= 256
                && let Ok(value) = String::from_utf8(bytes)
                && !value.chars().any(char::is_control)
            {
                texts.insert(field, value);
            }
        } else if command == 7193 && tag & 7 == 0 {
            scalar.insert(field, input.read_uint64()?.to_string());
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    if uid.is_none() && properties.is_empty() && texts.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        json!({"command_id":command,"uid":uid,"properties":properties,"top_level_text_fields":texts,"top_level_scalar_fields":scalar,"interpretation":"research_player_fields_not_applied_to_account_state"}),
    ))
}

fn scalars(bytes: &[u8]) -> Result<BTreeMap<u32, u64>> {
    ensure!(bytes.len() <= 65536, "Social nested message too large");
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut result = BTreeMap::new();
    let mut count = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        count += 1;
        ensure!(count <= 256, "Too many nested social fields");
        if tag & 7 == 0 {
            ensure!(
                !result.contains_key(&(tag >> 3)),
                "Duplicate nested social scalar"
            );
            result.insert(tag >> 3, input.read_uint64()?);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    Ok(result)
}
fn social(body: &[u8]) -> Result<Value> {
    let mut records = Vec::new();
    for (_, bytes) in crate::profile_protocol::nested(body)?
        .into_iter()
        .filter(|(field, _)| *field == 2)
    {
        ensure!(
            records.len() < 256 && bytes.len() <= 65536,
            "Too many social records"
        );
        let mut input = CodedInputStream::from_bytes(&bytes);
        let mut scalar = BTreeMap::new();
        let mut text = BTreeMap::new();
        let mut birthday = None;
        let mut picture = None;
        let mut avatars = Vec::new();
        let mut cards = Vec::new();
        let mut count = 0;
        while let Some(tag) = input.read_raw_tag_or_eof()? {
            count += 1;
            ensure!(count <= 256, "Too many social fields");
            let field = tag >> 3;
            match tag {
                18 | 42 => {
                    ensure!(!text.contains_key(&field), "Duplicate social text");
                    let value = input.read_string()?;
                    ensure!(value.len() <= 1024, "Social text too long");
                    text.insert(field, value);
                }
                50 => {
                    ensure!(birthday.is_none(), "Duplicate birthday");
                    birthday = Some(scalars(&input.read_bytes()?)?);
                }
                178 => {
                    ensure!(avatars.len() < 32, "Too many showcase characters");
                    let values = scalars(&input.read_bytes()?)?;
                    avatars.push(json!({"avatar_id":values.get(&1),"level":values.get(&2),"raw_scalar_fields":values}));
                }
                186 => {
                    let packed = input.read_bytes()?;
                    let mut packed = CodedInputStream::from_bytes(&packed);
                    while !packed.eof()? {
                        ensure!(cards.len() < 256, "Too many showcase name cards");
                        cards.push(packed.read_uint32()?);
                    }
                }
                184 => {
                    ensure!(cards.len() < 256, "Too many showcase name cards");
                    cards.push(input.read_uint32()?);
                }
                202 => {
                    ensure!(picture.is_none(), "Duplicate profile picture");
                    picture = Some(scalars(&input.read_bytes()?)?);
                }
                _ if tag & 7 == 0 => {
                    ensure!(!scalar.contains_key(&field), "Duplicate social scalar");
                    scalar.insert(field, input.read_uint64()?);
                }
                _ => {
                    ensure!(
                        !matches!(field, 1 | 2 | 3 | 5 | 6 | 7 | 14 | 18 | 22 | 23 | 25),
                        "Invalid social field wire type"
                    );
                    protobuf::rt::skip_field_for_tag(tag, &mut input)?;
                }
            }
        }
        let uid = scalar
            .get(&1)
            .filter(|id| **id > 0 && **id <= u64::from(u32::MAX))
            .ok_or_else(|| anyhow::anyhow!("Invalid social UID"))?
            .to_string();
        records.push(json!({"uid":uid,"nickname":text.get(&2),"signature":text.get(&5),"adventure_rank":scalar.get(&3),"world_level":scalar.get(&7),"name_card_id":scalar.get(&14),"achievement_count":scalar.get(&18),"birthday":birthday.as_ref().map(|b|json!({"month":b.get(&1),"day":b.get(&2)})),"show_avatars":avatars,"show_name_card_ids":cards,"profile_picture_raw":picture,"raw_scalar_fields":scalar}));
    }
    Ok(
        json!({"command_id":9233,"social_records":records,"interpretation":"public_social_profile_not_login_identity"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn write(action: impl FnOnce(&mut CodedOutputStream<'_>)) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            action(&mut out);
            out.flush().unwrap();
        }
        bytes
    }
    fn property(field: u32, key: u32, value: i64, ival: bool) -> Vec<u8> {
        let mut prop = PropValue::new();
        prop.type_ = key;
        prop.val = value;
        if ival {
            prop.set_ival(value);
        }
        let entry = write(|o| {
            o.write_uint32(1, key).unwrap();
            o.write_bytes(2, &prop.write_to_bytes().unwrap()).unwrap();
        });
        write(|o| o.write_bytes(field, &entry).unwrap())
    }
    #[test]
    fn login_extracts_only_uid_and_skips_credentials() {
        let body = write(|o| {
            o.write_uint32(9, 700000001).unwrap();
            o.write_string(10, "secret-not-for-export").unwrap();
        });
        let result = parse(27808, &body).unwrap().unwrap();
        assert_eq!(result["uid"], "700000001");
        assert!(!result.to_string().contains("secret-not-for-export"));
        let duplicate = write(|o| {
            o.write_uint32(9, 1).unwrap();
            o.write_uint32(9, 2).unwrap();
        });
        assert!(parse(27808, &duplicate).is_err());
        assert!(parse(27808, &[74, 1, 65]).is_err());
        assert!(parse(27808, &write(|o| o.write_uint32(9, 0).unwrap())).is_err());
    }
    #[test]
    fn rank_and_exp_preserve_zero_partial_updates_and_type_mismatch() {
        let mut body = property(6, 10013, 60, true);
        body.extend(property(6, 10014, 0, false));
        body.extend(write(|o| o.write_string(8, "Тестовый профиль").unwrap()));
        let result = parse(7193, &body).unwrap().unwrap();
        assert_eq!(result["properties"]["10013"]["value"], 60);
        assert_eq!(result["properties"]["10014"]["value"], 0);
        assert_eq!(result["top_level_text_fields"]["8"], "Тестовый профиль");
        let update = parse(22660, &property(8, 10014, 1000, true))
            .unwrap()
            .unwrap();
        assert_eq!(update["properties"]["10014"]["value"], 1000);
        assert!(update["properties"]["10013"].is_null());
        let mut duplicate = property(6, 10013, 60, true);
        duplicate.extend(property(6, 10013, 59, true));
        assert!(parse(7193, &duplicate).is_err());
        let bad = write(|o| {
            o.write_bytes(
                6,
                &write(|o| {
                    o.write_uint32(1, 10013).unwrap();
                    o.write_bytes(2, &{
                        let mut p = PropValue::new();
                        p.type_ = 10014;
                        p.set_ival(60);
                        p.write_to_bytes().unwrap()
                    })
                    .unwrap();
                }),
            )
            .unwrap()
        });
        assert!(parse(7193, &bad).is_err());
        assert!(parse(7193, &[50, 255]).is_err());
        assert!(parse(1, &[255]).unwrap().is_none());
    }
    #[test]
    fn social_profiles_have_their_own_uid_and_ordered_showcase() {
        let social = write(|o| {
            o.write_uint32(1, 710000001).unwrap();
            o.write_string(2, "Другой профиль").unwrap();
            o.write_string(5, "Подпись").unwrap();
            o.write_uint32(3, 45).unwrap();
            o.write_bytes(
                6,
                &write(|o| {
                    o.write_uint32(1, 3).unwrap();
                    o.write_uint32(2, 7).unwrap();
                }),
            )
            .unwrap();
            for id in [10000071, 10000002] {
                o.write_bytes(
                    22,
                    &write(|o| {
                        o.write_uint32(1, id).unwrap();
                        o.write_uint32(2, 90).unwrap();
                    }),
                )
                .unwrap();
            }
            o.write_bytes(
                23,
                &write(|o| {
                    o.write_raw_varint32(210303).unwrap();
                    o.write_raw_varint32(210001).unwrap();
                }),
            )
            .unwrap();
        });
        let result = parse(9233, &write(|o| o.write_bytes(2, &social).unwrap()))
            .unwrap()
            .unwrap();
        let p = &result["social_records"][0];
        assert!(result["uid"].is_null());
        assert_eq!(p["uid"], "710000001");
        assert_eq!(p["birthday"]["month"], 3);
        assert_eq!(p["show_avatars"][0]["avatar_id"], 10000071);
        assert_eq!(p["show_avatars"][1]["avatar_id"], 10000002);
        assert_eq!(p["show_name_card_ids"], json!([210303, 210001]));
        assert!(parse(9233, &[18, 255]).is_err());
    }
}
