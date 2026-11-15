//! Initial protocol observations. Property meanings are retained as numeric keys.
use anyhow::{Result, ensure};
use auto_artifactarium::r#gen::protos::{AvatarDataNotify, AvatarInfo};
use protobuf::Message;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub fn parse(body: &[u8]) -> Result<Vec<Value>> {
    ensure!(
        body.len() <= 2 * 1024 * 1024,
        "Character research command exceeds size limit"
    );
    let packet = AvatarDataNotify::parse_from_bytes(body)?;
    ensure!(
        !packet.avatar_list.is_empty() && packet.avatar_list.len() <= 1024,
        "Character list is empty or exceeds research limit"
    );
    normalize(packet.avatar_list)
}

/// Verified incoming 3231: exactly one AvatarInfo at field 4.
pub fn added(body: &[u8]) -> Result<Value> {
    use protobuf::CodedInputStream;
    ensure!(
        body.len() <= 262144,
        "Character addition exceeds size limit"
    );
    let mut input = CodedInputStream::from_bytes(body);
    let mut avatar = None;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        if tag >> 3 == 4 {
            ensure!(
                tag & 7 == 2 && avatar.is_none(),
                "Invalid or duplicate added character"
            );
            avatar = Some(AvatarInfo::parse_from_bytes(&input.read_bytes()?)?);
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    let avatar = avatar.ok_or_else(|| anyhow::anyhow!("Missing added character"))?;
    let level = avatar
        .prop_map
        .get(&4001)
        .ok_or_else(|| anyhow::anyhow!("Missing character level"))?;
    ensure!(
        level.type_ == 4001
            && level.has_ival()
            && level.ival() > 0
            && level.ival() <= i64::from(u32::MAX),
        "Invalid character level"
    );
    Ok(normalize(vec![avatar])?.remove(0))
}

fn normalize(avatars: Vec<AvatarInfo>) -> Result<Vec<Value>> {
    let mut guids = HashSet::new();
    for avatar in &avatars {
        ensure!(
            avatar.avatar_id > 0 && avatar.guid > 0 && guids.insert(avatar.guid),
            "Invalid or duplicate character GUID"
        );
    }
    let profiles=avatars.into_iter().map(|avatar| {
        let props:BTreeMap<_,_>=avatar.prop_map.iter().map(|(id,prop)|(*id,json!({"type":prop.type_,"val":prop.val.to_string(),
            "ival":if prop.has_ival(){Some(prop.ival().to_string())}else{None},"fval":if prop.has_fval(){Some(prop.fval())}else{None}}))).collect();
        let fight_props:BTreeMap<_,_>=avatar.fight_prop_map.into_iter().collect();
        let skill_levels:BTreeMap<_,_>=avatar.skill_level_map.into_iter().collect();
        let extra_skill_levels:BTreeMap<_,_>=avatar.proud_skill_extra_level_map.into_iter().collect();
        let friendship=avatar.fetter_info.as_ref().map(|info|json!({"level":info.exp_level,"exp":info.exp_number}));
        let trial_info_present=avatar.special_fields.unknown_fields().iter().any(|(field,value)|field==9 && matches!(value,protobuf::UnknownValueRef::LengthDelimited(_)));
        json!({"avatar_id":avatar.avatar_id,"guid":avatar.guid.to_string(),"props":props,"fight_props":fight_props,
            "equip_guids":avatar.equip_guid_list.iter().map(u64::to_string).collect::<Vec<_>>(),
            "talent_ids":avatar.talent_id_list,"skill_levels":skill_levels,"extra_skill_levels":extra_skill_levels,
            "skill_depot_id":avatar.skill_depot_id,"avatar_type":avatar.avatar_type,"life_state":avatar.life_state,
            "friendship":friendship,"trial_info_present":trial_info_present})
    }).collect();
    Ok(profiles)
}

/// Research-only extraction of the team map at AvatarDataNotify field5 (upstream field14 is outdated).
/// Empty saved teams are records and must not be dropped.
pub fn parse_teams(body: &[u8], profiles: &[Value]) -> Result<Value> {
    use protobuf::CodedInputStream;
    ensure!(
        body.len() <= 2 * 1024 * 1024,
        "Team snapshot exceeds research limit"
    );
    let mut input = CodedInputStream::from_bytes(body);
    let mut teams = BTreeMap::new();
    let mut scalars = BTreeMap::<u32, Vec<String>>::new();
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 8192, "Too many team snapshot fields");
        if tag >> 3 == 5 {
            ensure!(tag & 7 == 2, "Invalid team map wire type");
            let bytes = input.read_bytes()?;
            let (id, bytes) = crate::profile_protocol::entry(&bytes)?;
            ensure!(
                id > 0 && id <= u64::from(u32::MAX) && !teams.contains_key(&id),
                "Invalid or duplicate team slot"
            );
            let mut team_input = CodedInputStream::from_bytes(&bytes);
            let mut avatar_guid_list = Vec::new();
            let mut name = String::new();
            let mut name_seen = false;
            let mut team_fields = 0;
            while let Some(tag) = team_input.read_raw_tag_or_eof()? {
                team_fields += 1;
                ensure!(team_fields <= 128, "Too many team fields");
                if tag == 98 {
                    let packed = team_input.read_bytes()?;
                    let mut packed = CodedInputStream::from_bytes(&packed);
                    while !packed.eof()? {
                        ensure!(avatar_guid_list.len() < 16, "Too many members");
                        avatar_guid_list.push(packed.read_uint64()?);
                    }
                } else if tag == 96 {
                    ensure!(avatar_guid_list.len() < 16, "Too many members");
                    avatar_guid_list.push(team_input.read_uint64()?);
                } else if tag == 18 {
                    ensure!(!name_seen, "Duplicate team name");
                    name_seen = true;
                    name = team_input.read_string()?;
                    ensure!(name.len() <= 4096, "Team name too long");
                } else if matches!(tag >> 3, 2 | 12) {
                    anyhow::bail!("Invalid team field wire type");
                } else {
                    protobuf::rt::skip_field_for_tag(tag, &mut team_input)?;
                }
            }
            let mut member_guids = HashSet::new();
            ensure!(
                avatar_guid_list
                    .iter()
                    .all(|guid| *guid > 0 && member_guids.insert(*guid)),
                "Invalid or duplicate team member GUID"
            );
            let members:Vec<_>=avatar_guid_list.iter().enumerate().map(|(index,guid)| {
                let profile=profiles.iter().find(|p|p["guid"].as_str()==Some(&guid.to_string()));
                json!({"position":index+1,"guid":guid.to_string(),"avatar_id":profile.map(|p|&p["avatar_id"]),"trial_info_present":profile.map(|p|&p["trial_info_present"])})
            }).collect();
            teams.insert(id, json!({"team_id":id,"name":name,"members":members}));
        } else if tag & 7 == 0 {
            scalars
                .entry(tag >> 3)
                .or_default()
                .push(input.read_uint64()?.to_string());
        } else {
            protobuf::rt::skip_field_for_tag(tag, &mut input)?;
        }
    }
    Ok(
        json!({"teams":teams.into_values().collect::<Vec<_>>(),"top_level_scalar_fields":scalars,"interpretation":"research_team_map_not_applied_to_account_state"}),
    )
}

/// Observed team rename/edit/switch envelopes; research only, no state mutation.
pub fn team_command(command: u16, body: &[u8], guids: &HashSet<u64>) -> Result<Option<Value>> {
    if !matches!(command, 6665 | 27798 | 29481 | 28798 | 8831 | 28691) {
        return Ok(None);
    }
    ensure!(body.len() <= 65536, "Team command exceeds research limit");
    let mut diagnostics = crate::diagnostics::Diagnostics::default();
    diagnostics.targets = guids.clone();
    let numeric = diagnostics.structure(command, body);
    let mut text = BTreeMap::new();
    for (field, bytes) in crate::profile_protocol::nested(body)? {
        if let Ok(value) = std::str::from_utf8(&bytes)
            && value.len() <= 4096
            && !value.chars().any(char::is_control)
        {
            text.insert(field, value.to_owned());
        }
    }
    let updates = if command == 29481 {
        use protobuf::CodedOutputStream;
        let mut remapped = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut remapped);
            for (_, entry) in crate::profile_protocol::nested(body)?
                .into_iter()
                .filter(|(field, _)| *field == 15)
            {
                out.write_bytes(5, &entry)?;
            }
            out.flush()?;
        }
        parse_teams(&remapped, &[])?.get("teams").cloned()
    } else {
        None
    };
    Ok(Some(
        json!({"command_id":command,"numeric_fields":numeric,"top_level_text_fields":text,"team_updates":updates,"interpretation":"research_team_command_not_applied"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn team_snapshot(teams: &[(u32, &str, Vec<u64>)]) -> Vec<u8> {
        use protobuf::CodedOutputStream;
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            for (id, name, guids) in teams {
                let mut team = Vec::new();
                {
                    let mut out = CodedOutputStream::vec(&mut team);
                    if !name.is_empty() {
                        out.write_string(2, name).unwrap();
                    }
                    let mut packed = Vec::new();
                    {
                        let mut out = CodedOutputStream::vec(&mut packed);
                        for guid in guids {
                            out.write_raw_varint64(*guid).unwrap();
                        }
                        out.flush().unwrap();
                    }
                    if !packed.is_empty() {
                        out.write_bytes(12, &packed).unwrap();
                    }
                    out.flush().unwrap();
                }
                let mut entry = Vec::new();
                {
                    let mut out = CodedOutputStream::vec(&mut entry);
                    out.write_uint32(1, *id).unwrap();
                    out.write_bytes(2, &team).unwrap();
                    out.flush().unwrap();
                }
                out.write_bytes(5, &entry).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    #[test]
    fn team_commands_capture_rename_full_updates_empty_team_and_switch_ack() {
        use protobuf::CodedOutputStream;
        let mut rename = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut rename);
            out.write_uint32(8, 10).unwrap();
            out.write_string(13, "V0 test").unwrap();
            out.flush().unwrap();
        }
        let result = team_command(27798, &rename, &HashSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(result["top_level_text_fields"]["13"], "V0 test");
        for members in [vec![u64::MAX], vec![]] {
            let mut update = team_snapshot(&[(10, "V0 test", members.clone())]);
            update[0] = 122; // map field5 -> field15, same one-byte tag.
            let result = team_command(29481, &update, &HashSet::from([u64::MAX]))
                .unwrap()
                .unwrap();
            assert_eq!(result["team_updates"][0]["team_id"], 10);
            assert_eq!(result["team_updates"][0]["name"], "V0 test");
            assert_eq!(
                result["team_updates"][0]["members"]
                    .as_array()
                    .unwrap()
                    .len(),
                members.len()
            );
        }
        let result = team_command(8831, &[16, 2], &HashSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(result["numeric_fields"][0]["path"], "2");
        assert_eq!(result["numeric_fields"][0]["value"], "2");
        assert!(team_command(1, &[255], &HashSet::new()).unwrap().is_none());
        assert!(team_command(29481, &[122, 255], &HashSet::new()).is_err());
    }
    #[test]
    fn teams_preserve_empty_slots_names_order_and_unknown_member_guids() {
        let body = team_snapshot(&[(10, "", vec![]), (1, "Отряд <1>", vec![u64::MAX, 42])]);
        let result = parse_teams(
            &body,
            &[json!({"guid":u64::MAX.to_string(),"avatar_id":10000071,"trial_info_present":false})],
        )
        .unwrap();
        assert_eq!(result["teams"].as_array().unwrap().len(), 2);
        assert_eq!(result["teams"][0]["name"], "Отряд <1>");
        assert_eq!(
            result["teams"][0]["members"][0]["guid"],
            u64::MAX.to_string()
        );
        assert_eq!(result["teams"][0]["members"][0]["avatar_id"], 10000071);
        assert_eq!(result["teams"][0]["members"][1]["guid"], "42");
        assert!(result["teams"][0]["members"][1]["avatar_id"].is_null());
        assert_eq!(result["teams"][1]["team_id"], 10);
        assert_eq!(result["teams"][1]["name"], "");
        assert!(result["teams"][1]["members"].as_array().unwrap().is_empty());
    }
    #[test]
    fn team_research_rejects_duplicate_slots_members_and_damaged_envelopes() {
        assert!(parse_teams(&team_snapshot(&[(1, "", vec![]), (1, "", vec![])]), &[]).is_err());
        assert!(parse_teams(&team_snapshot(&[(1, "", vec![42, 42])]), &[]).is_err());
        assert!(parse_teams(&team_snapshot(&[(0, "", vec![])]), &[]).is_err());
        assert!(parse_teams(&team_snapshot(&[(1, "", vec![0])]), &[]).is_err());
        let mut bad = team_snapshot(&[(1, "", vec![42])]);
        bad.push(0xff);
        assert!(parse_teams(&bad, &[]).is_err());
        assert!(parse_teams(&[40, 1], &[]).is_err());
        assert!(parse_teams(&[42, 3, 8, 1, 18], &[]).is_err());
    }
    #[test]
    fn trial_marker_requires_message_field_and_raw_profiles_remain_available() {
        let mut packet = AvatarDataNotify::new();
        let mut avatar = auto_artifactarium::r#gen::protos::AvatarInfo::new();
        avatar.avatar_id = 10000127;
        avatar.guid = 1;
        avatar
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(9, vec![]);
        packet.avatar_list.push(avatar.clone());
        avatar.guid = 2;
        avatar.special_fields.clear();
        avatar.special_fields.mut_unknown_fields().add_varint(9, 1);
        packet.avatar_list.push(avatar);
        let profiles = parse(&packet.write_to_bytes().unwrap()).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["trial_info_present"], true);
        assert_eq!(profiles[1]["trial_info_present"], false);
    }
    #[test]
    fn preserves_guid_and_raw_fields_without_inventing_missing_levels() {
        let mut packet = AvatarDataNotify::new();
        let mut avatar = auto_artifactarium::r#gen::protos::AvatarInfo::new();
        avatar.avatar_id = 10000002;
        avatar.guid = u64::MAX;
        avatar.equip_guid_list.push(u64::MAX - 1);
        avatar.skill_level_map.insert(10001, 6);
        avatar.proud_skill_extra_level_map.insert(1, 3);
        avatar.fight_prop_map.insert(2000, 12345.5);
        packet.avatar_list.push(avatar);
        let profiles = parse(&packet.write_to_bytes().unwrap()).unwrap();
        assert_eq!(profiles[0]["guid"], u64::MAX.to_string());
        assert_eq!(profiles[0]["equip_guids"][0], (u64::MAX - 1).to_string());
        assert!(profiles[0]["props"].as_object().unwrap().is_empty());
        assert_eq!(profiles[0]["skill_levels"]["10001"], 6);
        assert_eq!(profiles[0]["fight_props"]["2000"], 12345.5);
    }
    #[test]
    fn rejects_empty_malformed_and_duplicate_guid_but_allows_same_id_different_guids() {
        assert!(parse(&[]).is_err());
        assert!(parse(&[50, 255]).is_err());
        let mut packet = AvatarDataNotify::new();
        let mut avatar = auto_artifactarium::r#gen::protos::AvatarInfo::new();
        avatar.avatar_id = 10000127;
        avatar.guid = 1;
        packet.avatar_list = vec![avatar.clone(), avatar];
        assert!(parse(&packet.write_to_bytes().unwrap()).is_err());
        packet.avatar_list[1].guid = 2;
        assert_eq!(parse(&packet.write_to_bytes().unwrap()).unwrap().len(), 2);
    }
}
