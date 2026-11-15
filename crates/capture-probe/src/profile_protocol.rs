//! Bounded protobuf helpers shared by offline discovery and verified profile messages.
use anyhow::{Result, ensure};
use auto_artifactarium::r#gen::protos::{AvatarFetterInfo, PropValue};
use protobuf::{CodedInputStream, Message};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

/// Verified 7.2 incoming 6762: avatar GUID 6, partial map<uint32,float> 14.
/// Entries carry absolute values. An omitted protobuf float is zero, not deletion.
pub fn fight_properties(body: &[u8]) -> Result<(u64, BTreeMap<u32, f64>)> {
    ensure!(body.len() <= 65536, "Fight property update too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut guid = None;
    let mut props = BTreeMap::new();
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 4096, "Too many fight property fields");
        match tag {
            48 => {
                ensure!(guid.is_none(), "Duplicate fight property GUID");
                guid = Some(input.read_uint64()?);
            }
            114 => {
                let bytes = input.read_bytes()?;
                let mut entry = CodedInputStream::from_bytes(&bytes);
                let mut key = None;
                let mut value = None;
                while let Some(tag) = entry.read_raw_tag_or_eof()? {
                    match tag {
                        8 => {
                            ensure!(key.is_none(), "Duplicate fight property key");
                            key = Some(entry.read_uint32()?);
                        }
                        21 => {
                            ensure!(value.is_none(), "Duplicate fight property value");
                            value = Some(f64::from(entry.read_float()?));
                        }
                        _ => anyhow::bail!("Invalid fight property map entry"),
                    }
                }
                let key = key
                    .filter(|k| *k > 0)
                    .ok_or_else(|| anyhow::anyhow!("Missing fight property key"))?;
                let value = value.unwrap_or(0.0);
                ensure!(value.is_finite(), "Non-finite fight property");
                ensure!(
                    props.len() < 512 && props.insert(key, value).is_none(),
                    "Duplicate/oversized fight property map"
                );
            }
            _ => {
                ensure!(
                    !matches!(tag >> 3, 6 | 14),
                    "Invalid fight property wire type"
                );
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        }
    }
    Ok((
        guid.filter(|g| *g > 0)
            .ok_or_else(|| anyhow::anyhow!("Missing fight property GUID"))?,
        props,
    ))
}

/// Observed 7.2 command 561: GUID field 5, map<uint32,uint64> field 3.
pub fn character_properties(body: &[u8]) -> Result<(u64, BTreeMap<u32, u64>)> {
    ensure!(body.len() <= 262144, "Character property update too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut guid = None;
    let mut props = BTreeMap::new();
    let mut count = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        count += 1;
        ensure!(count <= 4096, "Too many character property fields");
        match tag {
            40 => {
                ensure!(guid.is_none(), "Duplicate character GUID");
                guid = Some(input.read_uint64()?);
            }
            26 => {
                let bytes = input.read_bytes()?;
                let mut map = CodedInputStream::from_bytes(&bytes);
                let mut key = None;
                let mut value = None;
                while let Some(tag) = map.read_raw_tag_or_eof()? {
                    match tag {
                        8 => {
                            ensure!(key.is_none(), "Duplicate property key");
                            key = Some(map.read_uint32()?);
                        }
                        16 => {
                            ensure!(value.is_none(), "Duplicate property value");
                            value = Some(map.read_uint64()?);
                        }
                        _ => anyhow::bail!("Invalid character property map entry"),
                    }
                }
                let key = key.ok_or_else(|| anyhow::anyhow!("Missing property key"))?;
                // Proto3 map values default to zero when field 2 is omitted.
                ensure!(
                    props.insert(key, value.unwrap_or(0)).is_none(),
                    "Duplicate character property"
                );
            }
            _ => {
                ensure!(
                    !matches!(tag >> 3, 3 | 5),
                    "Invalid character property wire type"
                );
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        }
    }
    let guid = guid
        .filter(|guid| *guid > 0)
        .ok_or_else(|| anyhow::anyhow!("Missing character GUID"))?;
    Ok((guid, props))
}

/// Verified 7.2 incoming 22610: old level 3, new level 5, GUID 11, skill 13.
pub fn talent_change(body: &[u8]) -> Result<(u64, u32, u32, u32)> {
    ensure!(body.len() <= 65536, "Talent update too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut values = BTreeMap::new();
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 128, "Too many talent fields");
        let field = tag >> 3;
        if matches!(field, 3 | 5 | 11 | 13) {
            ensure!(
                tag & 7 == 0 && !values.contains_key(&field),
                "Invalid or duplicate talent field"
            );
            let value = if field == 11 {
                input.read_uint64()?
            } else {
                u64::from(input.read_uint32()?)
            };
            ensure!(value > 0, "Invalid zero talent field");
            values.insert(field, value);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    ensure!(values.len() == 4, "Missing talent fields");
    Ok((
        values[&11],
        values[&13] as u32,
        values[&3] as u32,
        values[&5] as u32,
    ))
}

/// Incoming 3996: activated talent 1, skill depot 7, character GUID 11.
/// The companion response 1528 is not another activation.
pub fn constellation_activation(body: &[u8]) -> Result<(u64, u32, u32)> {
    ensure!(body.len() <= 65536, "Constellation message too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut values = BTreeMap::new();
    let mut count = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        count += 1;
        ensure!(count <= 128, "Too many constellation fields");
        let field = tag >> 3;
        if matches!(field, 1 | 7 | 11) {
            ensure!(
                tag & 7 == 0 && !values.contains_key(&field),
                "Invalid or duplicate constellation field"
            );
            let value = if field == 11 {
                input.read_uint64()?
            } else {
                u64::from(input.read_uint32()?)
            };
            ensure!(value > 0, "Zero constellation identity");
            values.insert(field, value);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    ensure!(values.len() == 3, "Missing constellation fields");
    Ok((values[&11], values[&7] as u32, values[&1] as u32))
}

pub fn nested(body: &[u8]) -> Result<Vec<(u32, Vec<u8>)>> {
    ensure!(body.len() <= 2 * 1024 * 1024, "Profile message too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut values = Vec::new();
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 8192, "Too many profile fields");
        if tag & 7 == 2 {
            values.push((tag >> 3, input.read_bytes()?));
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    Ok(values)
}
pub fn entry(bytes: &[u8]) -> Result<(u64, Vec<u8>)> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut key = None;
    let mut value = None;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            8 => {
                ensure!(key.is_none(), "Duplicate map key");
                key = Some(input.read_uint64()?);
            }
            18 => {
                ensure!(value.is_none(), "Duplicate map value");
                value = Some(input.read_bytes()?);
            }
            _ => anyhow::bail!("Not a profile map entry"),
        }
    }
    Ok((
        key.ok_or_else(|| anyhow::anyhow!("Missing key"))?,
        value.ok_or_else(|| anyhow::anyhow!("Missing value"))?,
    ))
}
pub fn discover(body: &[u8], guids: &HashSet<u64>) -> Vec<Value> {
    let Ok(fields) = nested(body) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (field, bytes) in fields {
        let Ok((key, value)) = entry(&bytes) else {
            continue;
        };
        if key == 10016
            && let Ok(prop) = PropValue::parse_from_bytes(&value)
            && prop.type_ == 10016
            && prop.has_ival()
            && prop.ival() >= 0
        {
            result.push(json!({"kind":"mora","map_field":field,"value":prop.ival().to_string()}));
        }
        if guids.contains(&key)
            && let Ok(info) = AvatarFetterInfo::parse_from_bytes(&value)
            && (1..=10).contains(&info.exp_level)
            && info.exp_number < 10000
            && info.special_fields.unknown_fields().iter().next().is_none()
        {
            result.push(json!({"kind":"friendship","map_field":field,"guid":key.to_string(),"level":info.exp_level,"exp":info.exp_number}));
        }
    }
    result
}
