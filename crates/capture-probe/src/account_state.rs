//! Verified account-owned profile and saved-team state. Social UID never selects an account.
use anyhow::{Context, Result, ensure};
use protobuf::{CodedInputStream, CodedOutputStream};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    uid: Option<String>,
    profile: Option<Value>,
    teams: Option<BTreeMap<u64, Value>>,
}
impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn apply(&mut self, command: u16, body: &[u8]) -> Result<Vec<(&'static str, Value)>> {
        let mut out = Vec::new();
        if command == 27808 {
            if let Some(v) = crate::player_research::parse(command, body)? {
                let uid = v["uid"].as_str().context("Missing login UID")?;
                ensure!(
                    self.uid.as_deref().is_none_or(|old| old == uid),
                    "Account changed without session reset"
                );
                self.uid = Some(uid.into());
            }
            return Ok(out);
        }
        if matches!(command, 7193 | 22660 | 9233) {
            let Some(uid) = &self.uid else { return Ok(out) };
            let previous = self.profile.clone();
            let mut next = previous.clone().unwrap_or_else(|| json!({"uid":uid}));
            if let Some(v) = crate::player_research::parse(command, body)? {
                if command == 9233 {
                    for social in v["social_records"]
                        .as_array()
                        .context("Missing social records")?
                    {
                        if social["uid"].as_str() != Some(uid) {
                            continue;
                        }
                        if let Some(id) = social["profile_picture_raw"]["3"]
                            .as_u64()
                            .filter(|n| *n > 0 && *n <= u32::MAX as u64)
                        {
                            let frame = social["profile_picture_raw"]["4"].as_u64().unwrap_or(0);
                            ensure!(frame <= u32::MAX as u64, "Invalid profile frame");
                            next["profile_picture"] = json!({"id":id,"frame_id":frame});
                        }
                        for field in [
                            "nickname",
                            "signature",
                            "birthday",
                            "name_card_id",
                            "show_avatars",
                            "show_name_card_ids",
                            "achievement_count",
                        ] {
                            if !social[field].is_null() {
                                next[field] = social[field].clone();
                            }
                        }
                    }
                } else {
                    if let Some(nickname) = v["top_level_text_fields"].get("8") {
                        next["nickname"] = nickname.clone();
                    }
                    if let Some(world) = v["properties"].get("10019") {
                        next["world_level"] = world["value"].clone();
                    }
                }
            }
            if next.as_object().unwrap().len() > 1 && previous.as_ref() != Some(&next) {
                out.push((
                    if previous.is_some() {
                        "player_profile_changed"
                    } else {
                        "player_profile_snapshot"
                    },
                    json!({"previous":previous,"profile":next,"command_id":command}),
                ));
                self.profile = Some(next);
            }
        }
        if command == 20143 {
            let parsed = crate::avatar_research::parse_teams(body, &[])?;
            let mut teams = BTreeMap::new();
            for team in parsed["teams"].as_array().context("Missing teams")? {
                let mut team = team.clone();
                team["is_active"] = Value::Null;
                teams.insert(team["team_id"].as_u64().unwrap(), team);
            }
            ensure!(!teams.is_empty(), "Missing saved-team baseline");
            out.push((
                "team_snapshot",
                json!({"teams":teams.values().collect::<Vec<_>>(),"command_id":command}),
            ));
            self.teams = Some(teams);
        } else if matches!(command, 27798 | 29481 | 8831) {
            let Some(teams) = &mut self.teams else {
                return Ok(out);
            };
            let mut next = teams.clone();
            if command == 29481 {
                let mut remapped = Vec::new();
                {
                    let mut writer = CodedOutputStream::vec(&mut remapped);
                    for (field, entry) in crate::profile_protocol::nested(body)? {
                        if field == 15 {
                            writer.write_bytes(5, &entry)?;
                        }
                    }
                    writer.flush()?;
                }
                let parsed = crate::avatar_research::parse_teams(&remapped, &[])?;
                for team in parsed["teams"].as_array().unwrap() {
                    let id = team["team_id"].as_u64().unwrap();
                    let old = next.get(&id).context("Unknown team slot")?;
                    let mut team = team.clone();
                    team["is_active"] = old["is_active"].clone();
                    next.insert(id, team);
                }
            } else {
                ensure!(body.len() <= 65536, "Team update too large");
                let mut input = CodedInputStream::from_bytes(body);
                let mut id = None;
                let mut name = None;
                let mut count = 0;
                while let Some(tag) = input.read_raw_tag_or_eof()? {
                    count += 1;
                    ensure!(count <= 128, "Too many team fields");
                    if tag >> 3 == if command == 8831 { 2 } else { 8 } {
                        ensure!(tag & 7 == 0 && id.is_none(), "Invalid team ID");
                        id = Some(u64::from(input.read_uint32()?));
                    } else if command == 27798 && tag >> 3 == 13 {
                        ensure!(tag & 7 == 2 && name.is_none(), "Invalid team name");
                        let text = input.read_string()?;
                        ensure!(text.len() <= 4096, "Team name too long");
                        name = Some(text);
                    } else {
                        protobuf::rt::skip_field_for_tag(tag, &mut input)?;
                    }
                }
                let id = id.context("Missing team ID")?;
                ensure!(next.contains_key(&id), "Unknown team slot");
                if command == 8831 {
                    for (key, team) in &mut next {
                        team["is_active"] = json!(*key == id);
                    }
                } else {
                    next.get_mut(&id).unwrap()["name"] = json!(name.unwrap_or_default());
                }
            }
            let changes: Vec<_> = next
                .iter()
                .filter(|(id, value)| teams.get(id) != Some(*value))
                .map(|(id, value)| json!({"previous":teams[id],"team":value}))
                .collect();
            if !changes.is_empty() {
                out.push((
                    "team_changed",
                    json!({"changes":changes,"command_id":command}),
                ));
                *teams = next;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire(f: impl FnOnce(&mut CodedOutputStream<'_>)) -> Vec<u8> {
        let mut bytes = vec![];
        {
            let mut w = CodedOutputStream::vec(&mut bytes);
            f(&mut w);
            w.flush().unwrap();
        }
        bytes
    }
    #[test]
    fn foreign_social_card_cannot_overwrite_login_or_own_profile() {
        let mut state = State::default();
        state
            .apply(27808, &wire(|w| w.write_uint32(9, 700000001).unwrap()))
            .unwrap();
        state
            .apply(7193, &wire(|w| w.write_string(8, "Owner").unwrap()))
            .unwrap();
        let social = wire(|w| {
            w.write_uint32(1, 800000001).unwrap();
            w.write_string(2, "Other").unwrap();
        });
        assert!(
            state
                .apply(9233, &wire(|w| w.write_bytes(2, &social).unwrap()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(state.profile.as_ref().unwrap()["nickname"], "Owner");
        state.reset();
        assert!(
            state
                .apply(7193, &wire(|w| w.write_string(8, "Unbound").unwrap()))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn own_social_decoration_preserves_picture_and_frame_ids() {
        let mut state = State::default();
        state
            .apply(27808, &wire(|w| w.write_uint32(9, 700000001).unwrap()))
            .unwrap();
        let picture = wire(|w| {
            w.write_uint32(3, 10400).unwrap();
            w.write_uint32(4, 100017).unwrap();
        });
        let social = wire(|w| {
            w.write_uint32(1, 700000001).unwrap();
            w.write_bytes(25, &picture).unwrap();
        });
        state
            .apply(9233, &wire(|w| w.write_bytes(2, &social).unwrap()))
            .unwrap();
        assert_eq!(
            state.profile.as_ref().unwrap()["profile_picture"],
            json!({"id":10400,"frame_id":100017})
        );
    }
    #[test]
    fn rename_empty_slot_switch_and_remove_last_member() {
        let mut state = State::default();
        let entry = wire(|w| {
            w.write_uint32(1, 10).unwrap();
            w.write_bytes(2, &[]).unwrap();
        });
        let snapshot = state
            .apply(20143, &wire(|w| w.write_bytes(5, &entry).unwrap()))
            .unwrap();
        assert_eq!(snapshot[0].1["teams"][0]["members"], json!([]));
        assert!(snapshot[0].1["teams"][0]["is_active"].is_null());
        let renamed = state
            .apply(
                27798,
                &wire(|w| {
                    w.write_uint32(8, 10).unwrap();
                    w.write_string(13, "Test").unwrap();
                }),
            )
            .unwrap();
        assert_eq!(renamed[0].1["changes"][0]["team"]["name"], "Test");
        state.apply(8831, &[16, 10]).unwrap();
        let member_team = wire(|w| {
            w.write_string(2, "Test").unwrap();
            w.write_uint64(12, u64::MAX).unwrap();
        });
        let member_entry = wire(|w| {
            w.write_uint32(1, 10).unwrap();
            w.write_bytes(2, &member_team).unwrap();
        });
        let added = state
            .apply(29481, &wire(|w| w.write_bytes(15, &member_entry).unwrap()))
            .unwrap();
        assert_eq!(
            added[0].1["changes"][0]["team"]["members"][0]["guid"],
            u64::MAX.to_string()
        );
        let cleared = state
            .apply(29481, &wire(|w| w.write_bytes(15, &entry).unwrap()))
            .unwrap();
        assert_eq!(cleared[0].1["changes"][0]["team"]["members"], json!([]));
        assert_eq!(cleared[0].1["changes"][0]["team"]["is_active"], true);
        assert!(state.apply(8831, &[16, 2]).is_err());
    }
}
