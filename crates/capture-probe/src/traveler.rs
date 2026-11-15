//! Verified 7.2 AvatarSkillDepotChangeNotify (25666). Independent of catalog language.
use anyhow::{Context, Result, ensure};
use protobuf::CodedInputStream;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn owned(id: &Value) -> bool {
    matches!(id.as_u64(), Some(10000005 | 10000007))
}
pub fn subset(profile: &Value) -> Value {
    json!({"skill_depot_id":profile["skill_depot_id"],"skill_levels":profile["skill_levels"],"talent_ids":profile["talent_ids"],"extra_skill_levels":profile["extra_skill_levels"]})
}
pub fn depot_for(avatar: u64, depot: u32) -> bool {
    match avatar {
        10000005 => (501..=508).contains(&depot),
        10000007 => (701..=708).contains(&depot),
        _ => false,
    }
}
pub fn guid(body: &[u8]) -> Result<String> {
    ensure!(body.len() <= 65536, "Depot message too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut value = None;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        if tag >> 3 == 14 {
            ensure!(tag == 112 && value.is_none(), "Invalid depot GUID field");
            value = Some(input.read_uint64()?);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    Ok(value
        .filter(|n| *n > 0)
        .context("Missing depot GUID")?
        .to_string())
}
fn pair(bytes: &[u8]) -> Result<(u32, u32)> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut k = None;
    let mut v = None;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            8 => {
                ensure!(k.is_none(), "Duplicate skill key");
                k = Some(input.read_uint32()?);
            }
            16 => {
                ensure!(v.is_none(), "Duplicate skill value");
                v = Some(input.read_uint32()?);
            }
            _ => anyhow::bail!("Invalid depot map entry"),
        }
    }
    Ok((
        k.filter(|n| *n > 0).context("Missing skill key")?,
        v.unwrap_or(0),
    ))
}
pub fn parse(body: &[u8]) -> Result<(String, Value)> {
    ensure!(body.len() <= 65536, "Depot message too large");
    let mut input = CodedInputStream::from_bytes(body);
    let mut guid = None;
    let mut depot = None;
    let mut skills = BTreeMap::new();
    let mut extra = BTreeMap::new();
    let mut talents = Vec::new();
    let mut talent_seen = false;
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 512, "Too many depot fields");
        match tag {
            112 => {
                ensure!(guid.is_none(), "Duplicate depot GUID");
                guid = Some(input.read_uint64()?);
            }
            56 => {
                ensure!(depot.is_none(), "Duplicate depot ID");
                depot = Some(input.read_uint32()?);
            }
            18 | 42 => {
                let (id, level) = pair(&input.read_bytes()?)?;
                let map = if tag == 18 { &mut skills } else { &mut extra };
                ensure!(
                    map.len() < 64 && map.insert(id, level).is_none(),
                    "Duplicate/oversized depot map"
                );
                ensure!(
                    if tag == 18 {
                        (1..=10).contains(&level)
                    } else {
                        level <= 10
                    },
                    "Invalid depot skill level"
                );
            }
            106 => {
                ensure!(!talent_seen, "Duplicate packed talents");
                talent_seen = true;
                let bytes = input.read_bytes()?;
                let mut packed = CodedInputStream::from_bytes(&bytes);
                while !packed.eof()? {
                    ensure!(talents.len() < 6, "Too many depot talents");
                    talents.push(packed.read_uint32()?);
                }
            }
            104 => {
                ensure!(talents.len() < 6, "Too many depot talents");
                talents.push(input.read_uint32()?);
            }
            _ => {
                ensure!(
                    !matches!(tag >> 3, 2 | 5 | 7 | 13 | 14),
                    "Invalid depot wire type"
                );
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        }
    }
    ensure!(!skills.is_empty(), "Missing full depot skills");
    let mut ids = BTreeSet::new();
    ensure!(
        talents.iter().all(|n| *n > 0 && ids.insert(*n)),
        "Invalid depot talents"
    );
    let guid = guid
        .filter(|n| *n > 0)
        .context("Missing depot GUID")?
        .to_string();
    let depot = depot.filter(|n| *n > 0).context("Missing depot ID")?;
    Ok((
        guid,
        json!({"skill_depot_id":depot,"skill_levels":skills,"talent_ids":talents,"extra_skill_levels":extra}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixtures() -> Vec<Vec<u8>> {
        let values: Vec<String> =
            serde_json::from_str(include_str!("../tests/fixtures/traveler-switch.json")).unwrap();
        values
            .iter()
            .map(|s| {
                s.as_bytes()
                    .chunks(2)
                    .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                    .collect()
            })
            .collect()
    }
    #[test]
    fn switch_talent_constellation_duplicate_and_reset_do_not_mix_sets() {
        use protobuf::CodedOutputStream;
        let messages = fixtures();
        let (_, cryo) = parse(&messages[1]).unwrap();
        let mut profile = cryo.clone();
        profile["guid"] = json!("42");
        profile["avatar_id"] = json!(10000005);
        profile["props"] = json!({"4001":{"ival":"90"},"1001":{"ival":"0"},"1002":{"ival":"6"}});
        profile["equip_guids"] = json!(["100"]);
        let mut state = crate::profile_state::State::default();
        state.snapshot(&[profile.clone()]);
        let first = state.apply(true, 25666, &messages[0]).unwrap();
        assert_eq!(first[0].1["current"]["skill_depot_id"], 504);
        assert!(state.apply(true, 25666, &messages[0]).unwrap().is_empty());
        let second = state.apply(true, 25666, &messages[1]).unwrap();
        assert_eq!(second[0].1["previous"]["skill_levels"]["10067"], 4);
        assert_eq!(second[0].1["current"]["skill_levels"]["10127"], 6);
        let mut body = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut body);
            out.write_uint32(3, 6).unwrap();
            out.write_uint32(5, 7).unwrap();
            out.write_uint64(11, 42).unwrap();
            out.write_uint32(13, 10127).unwrap();
            out.flush().unwrap();
        }
        let update = state.apply(true, 22610, &body).unwrap();
        assert_eq!(update[0].0, "traveler_element_changed");
        assert_eq!(update[0].1["current"]["skill_levels"]["10127"], 7);
        state.apply(true, 25666, &messages[0]).unwrap();
        assert!(state.apply(false, 25666, &messages[1]).unwrap().is_empty());
        state.reset();
        assert!(state.apply(true, 25666, &messages[1]).unwrap().is_empty());
        profile["talent_ids"] = json!([]);
        profile["extra_skill_levels"] = json!({});
        state.snapshot(&[profile]);
        let mut activation = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut activation);
            out.write_uint32(1, 81).unwrap();
            out.write_uint32(7, 505).unwrap();
            out.write_uint64(11, 42).unwrap();
            out.flush().unwrap();
        }
        let activated = state.apply(true, 3996, &activation).unwrap();
        assert_eq!(activated[0].0, "traveler_element_changed");
        assert_eq!(activated[0].1["current"]["talent_ids"], json!([81]));
        assert!(state.apply(true, 3996, &activation).unwrap().is_empty());
    }
    #[test]
    fn observed_anemo_and_cryo_fields_decode_full_sets() {
        let body = [
            vec![18, 6, 8, 191, 145, 6, 16, 4],
            vec![18, 5, 8, 211, 78, 16, 4],
            vec![18, 5, 8, 212, 78, 16, 4],
            vec![106, 6, 71, 72, 73, 74, 75, 76],
            vec![56, 248, 3],
            vec![112, 129, 232, 130, 128, 135, 180, 230, 177, 42],
        ]
        .concat();
        let (guid, v) = parse(&body).unwrap();
        assert_eq!(v["skill_depot_id"], 504);
        assert_eq!(v["skill_levels"]["10067"], 4);
        assert_eq!(v["talent_ids"].as_array().unwrap().len(), 6);
        assert!(!guid.is_empty());
        let mut bad = body.clone();
        bad.extend([56, 249, 3]);
        assert!(parse(&bad).is_err());
        assert!(parse(&body[..body.len() - 1]).is_err());
        assert!(depot_for(10000007, 705));
        assert!(!depot_for(10000007, 505));
    }
    #[test]
    fn newly_added_traveler_can_switch_without_waiting_for_another_login_snapshot() {
        use auto_artifactarium::r#gen::protos::{AvatarInfo, PropValue};
        use protobuf::{CodedOutputStream, Message};
        let mut avatar = AvatarInfo::new();
        avatar.avatar_id = 10000005;
        avatar.guid = 42;
        avatar.skill_depot_id = 505;
        avatar.skill_level_map.insert(10127, 6);
        let mut level = PropValue::new();
        level.type_ = 4001;
        level.set_ival(1);
        avatar.prop_map.insert(4001, level);
        let mut body = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut body);
            out.write_bytes(4, &avatar.write_to_bytes().unwrap())
                .unwrap();
            out.flush().unwrap();
        }
        let mut state = crate::profile_state::State::default();
        state.snapshot(&[]);
        assert_eq!(
            state.apply(true, 3231, &body).unwrap()[0].0,
            "character_added"
        );
        let changed = state.apply(true, 25666, &fixtures()[0]).unwrap();
        assert_eq!(changed[0].0, "traveler_element_changed");
        assert_eq!(changed[0].1["current"]["skill_depot_id"], 504);
    }
}
