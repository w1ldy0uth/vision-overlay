//! Verified protocol 7.2: player-property baseline 7193/6, updates 22660/8;
//! friendship baseline AvatarDataNotify, updates 7234/15. Quantities are absolute.
use crate::profile_protocol::{entry, nested};
use anyhow::{Result, ensure};
use auto_artifactarium::r#gen::protos::{AvatarFetterInfo, PropValue};
use protobuf::Message;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
pub struct State {
    pub equipment: crate::equipment_state::State,
    character_baseline: bool,
    currencies: BTreeMap<u32, i64>,
    adventure: Option<Value>,
    resin: Option<(u32, Option<u32>)>,
    resin_baseline: bool,
    talents: BTreeMap<String, Value>,
    constellations: BTreeMap<String, Value>,
    // A notification proves one activation, not the full baseline of another element.
    other_constellations: BTreeMap<(String, u32), Value>,
    friends: BTreeMap<String, Value>,
    characters: BTreeMap<String, Value>,
    fight_props: BTreeMap<String, Value>,
    traveler_active: BTreeMap<String, Value>,
}

fn resin_observation(count: u32, next: Option<u32>, command: u16) -> (&'static str, Value) {
    (
        "currency_snapshot",
        json!({"item_id":106,"count":count,"next_recovery_unix_s":next,"command_id":command}),
    )
}

// Observed on 7.2: count in field 11, next recovery as Unix seconds in field 12.
fn resin_notify(body: &[u8]) -> Option<(u32, Option<u32>)> {
    use protobuf::CodedInputStream;
    if body.is_empty() || body.len() > 64 {
        return None;
    }
    let mut input = CodedInputStream::from_bytes(body);
    let (mut count, mut next) = (None, None);
    while let Some(tag) = input.read_raw_tag_or_eof().ok()? {
        match tag {
            88 if count.is_none() => count = Some(input.read_uint32().ok()?),
            96 if next.is_none() => next = Some(input.read_uint32().ok()?),
            88 | 96 => return None,
            _ => protobuf::rt::skip_field_for_tag(tag, &mut input).ok()?,
        }
    }
    if count.is_none() && next.is_none() {
        return None;
    }
    let count = count.unwrap_or(0);
    if count > 2000 || next.is_some_and(|n| n != 0 && n < 1_600_000_000) {
        return None;
    }
    Some((count, next.filter(|n| *n > 0 && count < 200)))
}

#[cfg(test)]
mod fight_tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn packet(guid: u64, values: &[(u32, Option<f32>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_uint64(6, guid).unwrap();
            for (key, value) in values {
                let mut entry = Vec::new();
                {
                    let mut e = CodedOutputStream::vec(&mut entry);
                    e.write_uint32(1, *key).unwrap();
                    if let Some(v) = value {
                        e.write_float(2, *v).unwrap();
                    }
                    e.flush().unwrap();
                }
                out.write_bytes(14, &entry).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    fn state() -> State {
        let mut state = State::default();
        state.snapshot(&[json!({"guid":"18446744073709551615","avatar_id":10000137,"equip_guids":[],"fight_props":{"2000":3700.0,"2001":464.0,"2002":211.0,"40":0.2},"props":{}})]);
        state
    }
    #[test]
    fn partial_totals_merge_exact_values_clear_zero_and_preserve_unmentioned_stats() {
        let mut s = state();
        let body = packet(
            u64::MAX,
            &[(2001, Some(467.25)), (40, None), (28, Some(91.92999))],
        );
        let event = s.apply(true, 6762, &body).unwrap();
        assert_eq!(event.len(), 1);
        assert_eq!(event[0].1["previous"]["40"], 0.2);
        assert_eq!(event[0].1["current"]["40"], 0.0);
        assert_eq!(
            s.fight_props[&u64::MAX.to_string()]["values"]["2000"],
            3700.0
        );
        assert_eq!(
            s.fight_props[&u64::MAX.to_string()]["values"]["2001"],
            467.25
        );
        assert_eq!(event[0].1["current"]["28"], f64::from(91.92999_f32));
        assert!(event[0].1["previous"]["28"].is_null());
        assert!(s.apply(true, 6762, &body).unwrap().is_empty());
        assert!(
            s.apply(
                true,
                6762,
                &packet(u64::MAX, &[(1010, Some(1.0)), (4, Some(99.0))])
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            s.fight_props[&u64::MAX.to_string()]["values"]["2000"],
            3700.0
        );
        assert!(s.apply(false, 6762, b"invalid").unwrap().is_empty());
    }
    #[test]
    fn invalid_float_duplicate_or_truncated_packet_never_changes_state() {
        let mut s = state();
        let before = s.fight_props.clone();
        for body in [
            packet(u64::MAX, &[(2001, Some(787.0)), (22, Some(f32::NAN))]),
            packet(u64::MAX, &[(2001, Some(f32::INFINITY))]),
            packet(u64::MAX, &[(2001, Some(1.0)), (2001, Some(2.0))]),
            vec![48, 255],
        ] {
            assert!(s.apply(true, 6762, &body).is_err());
            assert_eq!(s.fight_props, before);
        }
    }
    #[test]
    fn reset_and_trial_profiles_cannot_receive_an_owned_update() {
        let mut s = state();
        s.reset();
        assert!(
            s.apply(true, 6762, &packet(u64::MAX, &[(2001, Some(1.0))]))
                .unwrap()
                .is_empty()
        );
        s.snapshot(&[json!({"guid":"42","avatar_id":10000137,"trial_info_present":true,"fight_props":{"2001":1.0}})]);
        assert!(
            s.apply(true, 6762, &packet(42, &[(2001, Some(2.0))]))
                .unwrap()
                .is_empty()
        );
    }
}

impl State {
    pub fn reset(&mut self) {
        self.equipment.reset();
        let equipment = std::mem::take(&mut self.equipment);
        *self = Self::default();
        self.equipment = equipment;
    }
    pub fn snapshot(&mut self, profiles: &[Value]) {
        self.fight_props = profiles
            .iter()
            .filter(|p| p["trial_info_present"] != true)
            .map(|p| {
                (
                    p["guid"].as_str().unwrap().to_owned(),
                    json!({"avatar_id":p["avatar_id"],"values":p["fight_props"]}),
                )
            })
            .collect();
        self.equipment.snapshot_profiles(profiles);
        self.other_constellations.clear();
        self.traveler_active = profiles
            .iter()
            .filter(|p| p["trial_info_present"] != true && crate::traveler::owned(&p["avatar_id"]))
            .map(|p| {
                (
                    p["guid"].as_str().unwrap().to_owned(),
                    crate::traveler::subset(p),
                )
            })
            .collect();
        self.character_baseline = true;
        self.constellations=profiles.iter().filter(|p|p["trial_info_present"]!=true).map(|p|(p["guid"].as_str().unwrap().to_owned(),json!({"avatar_id":p["avatar_id"],"skill_depot_id":p["skill_depot_id"],"talent_ids":p["talent_ids"]}))).collect();
        self.talents = profiles
            .iter()
            .filter(|p| p["trial_info_present"] != true)
            .map(|p| {
                (
                    p["guid"].as_str().unwrap().to_owned(),
                    json!({"avatar_id":p["avatar_id"],"skill_levels":p["skill_levels"]}),
                )
            })
            .collect();
        self.characters=profiles.iter().filter(|p|p["trial_info_present"]!=true).map(|p| {
            let property=|key:&str|p["props"][key]["ival"].as_str().and_then(|v|v.parse::<u32>().ok());
            (p["guid"].as_str().unwrap().to_owned(),json!({"guid":p["guid"],"avatar_id":p["avatar_id"],"level":property("4001"),"exp":property("1001"),"ascension":property("1002")}))
        }).collect();
        self.friends=profiles.iter().filter(|p|p["trial_info_present"]!=true && p["friendship"].is_object()).map(|p| (p["guid"].as_str().unwrap().to_owned(),
            json!({"avatar_id":p["avatar_id"],"guid":p["guid"],"level":p["friendship"]["level"],"exp":p["friendship"]["exp"]}))).collect();
    }
    pub fn apply(
        &mut self,
        incoming: bool,
        command: u16,
        body: &[u8],
    ) -> Result<Vec<(&'static str, Value)>> {
        if incoming && command == 24057 {
            // ResinChangeNotify is independent of the player-property map.
            // A schema mismatch here must not pause unrelated inventory tracking.
            let Some((count, next)) = resin_notify(body) else {
                return Ok(Vec::new());
            };
            let before = self.resin.replace((count, next));
            return if self.resin_baseline && before != self.resin {
                Ok(vec![resin_observation(count, next, command)])
            } else {
                Ok(Vec::new())
            };
        }
        if !incoming
            || !matches!(
                command,
                7193 | 22660 | 7234 | 561 | 22610 | 3231 | 3996 | 5724 | 6762 | 25666
            )
        {
            return Ok(Vec::new());
        }
        if command == 25666 {
            let guid = crate::traveler::guid(body)?;
            let Some(previous) = self.traveler_active.get(&guid) else {
                return Ok(vec![]);
            };
            let (_, current) = crate::traveler::parse(body)?;
            let avatar = self.constellations[&guid]["avatar_id"].clone();
            ensure!(
                crate::traveler::depot_for(
                    avatar.as_u64().unwrap(),
                    current["skill_depot_id"].as_u64().unwrap() as u32
                ),
                "Traveler depot belongs to another avatar"
            );
            if *previous == current {
                return Ok(vec![]);
            }
            let previous = previous.clone();
            self.traveler_active.insert(guid.clone(), current.clone());
            self.constellations.insert(guid.clone(),json!({"avatar_id":avatar,"skill_depot_id":current["skill_depot_id"],"talent_ids":current["talent_ids"]}));
            self.talents.insert(
                guid.clone(),
                json!({"avatar_id":avatar,"skill_levels":current["skill_levels"]}),
            );
            return Ok(vec![(
                "traveler_element_changed",
                json!({"guid":guid,"avatar_id":avatar,"previous":previous,"current":current,"command_id":command}),
            )]);
        }
        if command == 6762 {
            let (guid, updates) = crate::profile_protocol::fight_properties(body)?;
            let Some(profile) = self.fight_props.get_mut(&guid.to_string()) else {
                return Ok(vec![]);
            };
            let mut before = serde_json::Map::new();
            let mut after = serde_json::Map::new();
            for (id, value) in updates {
                // Build totals, secondary stats and elemental bonuses/resistances only.
                // Current HP, energy and operational counters do not describe a build.
                if !matches!(id,20..=56 | 2000..=2002) {
                    continue;
                }
                let key = id.to_string();
                if profile["values"][&key].as_f64() == Some(value) {
                    continue;
                }
                before.insert(key.clone(), profile["values"][&key].clone());
                after.insert(key, json!(value));
            }
            if after.is_empty() {
                return Ok(vec![]);
            }
            if !profile["values"].is_object() {
                profile["values"] = json!({});
            }
            for (key, value) in &after {
                profile["values"][key] = value.clone();
            }
            return Ok(vec![(
                "fight_properties_changed",
                json!({"guid":guid.to_string(),"avatar_id":profile["avatar_id"],"command_id":6762,"previous":before,"current":after}),
            )]);
        }
        if command == 5724 {
            return self.equipment.apply(body);
        }
        if command == 3996 {
            let (guid, depot, talent) = crate::profile_protocol::constellation_activation(body)?;
            let key = guid.to_string();
            let Some(profile) = self.constellations.get_mut(&key) else {
                return Ok(vec![]);
            };
            if profile["skill_depot_id"] != depot
                && matches!(profile["avatar_id"].as_u64(), Some(10000005 | 10000007))
            {
                let entry = self
                    .other_constellations
                    .entry((key.clone(), depot))
                    .or_insert(Value::Null);
                let previous = entry.clone();
                let mut ids = entry["observed_talent_ids"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if ids.contains(&json!(talent)) {
                    return Ok(vec![]);
                }
                ensure!(ids.len() < 6, "Constellation limit exceeded");
                ids.push(json!(talent));
                *entry = json!({"observed_talent_ids":ids,"completeness":"partial"});
                return Ok(vec![(
                    "constellation_depot_changed",
                    json!({
                        "guid":key,"avatar_id":profile["avatar_id"],"skill_depot_id":depot,
                        "talent_id":talent,"previous":previous,"current":entry,"command_id":command
                    }),
                )]);
            }
            let Some(ids) = profile["talent_ids"].as_array() else {
                return Ok(vec![]);
            };
            ensure!(
                profile["skill_depot_id"] == depot,
                "Constellation skill depot mismatch"
            );
            ensure!(
                ids.len() <= 6
                    && ids.iter().all(|id| id
                        .as_u64()
                        .is_some_and(|n| n > 0 && n <= u64::from(u32::MAX)))
                    && ids
                        .iter()
                        .filter_map(Value::as_u64)
                        .collect::<HashSet<_>>()
                        .len()
                        == ids.len(),
                "Invalid constellation baseline"
            );
            if ids.contains(&json!(talent)) {
                return Ok(vec![]);
            }
            ensure!(ids.len() < 6, "Constellation limit exceeded");
            let previous = profile.clone();
            let previous_count = ids.len();
            profile["talent_ids"]
                .as_array_mut()
                .unwrap()
                .push(json!(talent));
            if let Some(active) = self.traveler_active.get_mut(&key) {
                let before = active.clone();
                active["talent_ids"] = profile["talent_ids"].clone();
                return Ok(vec![(
                    "traveler_element_changed",
                    json!({"guid":key,"avatar_id":profile["avatar_id"],"previous":before,"current":active,"command_id":command}),
                )]);
            }
            return Ok(vec![(
                "constellation_changed",
                json!({"guid":key,"avatar_id":profile["avatar_id"],"talent_id":talent,"skill_depot_id":depot,"previous_count":previous_count,"count":previous_count+1,"previous":previous,"current":profile,"command_id":command}),
            )]);
        }
        if command == 3231 {
            if !self.character_baseline {
                return Ok(Vec::new());
            }
            let profile = crate::avatar_research::added(body)?;
            if profile["trial_info_present"] == true {
                return Ok(Vec::new());
            }
            let guid = profile["guid"].as_str().unwrap();
            if let Some(existing) = self.characters.get(guid) {
                ensure!(
                    existing["avatar_id"] == profile["avatar_id"],
                    "Conflicting character GUID"
                );
                return Ok(Vec::new());
            }
            ensure!(
                !self
                    .characters
                    .values()
                    .any(|p| p["avatar_id"] == profile["avatar_id"]),
                "Duplicate owned character ID"
            );
            let mut added = Self::default();
            added.snapshot(std::slice::from_ref(&profile));
            self.equipment.added_profile(&profile);
            self.characters.extend(added.characters);
            self.fight_props.extend(added.fight_props);
            self.talents.extend(added.talents);
            self.constellations.extend(added.constellations);
            self.traveler_active.extend(added.traveler_active);
            self.friends.extend(added.friends);
            return Ok(vec![(
                "character_added",
                json!({"guid":guid,"avatar_id":profile["avatar_id"],"profile":profile,"command_id":command}),
            )]);
        }
        if command == 22610 {
            return self.apply_talent(body);
        }
        if command == 561 {
            return self.apply_character(body);
        }
        let map_field = match command {
            7193 => 6,
            22660 => 8,
            _ => 15,
        };
        let fields = nested(body)?;
        let mut seen = HashSet::new();
        let mut currencies = Vec::new();
        let mut resin_count = None;
        let mut adventure_props = BTreeMap::new();
        let mut friends = Vec::new();
        for (_, bytes) in fields.into_iter().filter(|(field, _)| *field == map_field) {
            let (key, value) = entry(&bytes)?;
            ensure!(seen.insert(key), "Duplicate profile map key");
            if command != 7234 && matches!(key, 10015 | 10016) {
                let prop = PropValue::parse_from_bytes(&value)?;
                ensure!(
                    prop.type_ == key as u32
                        && prop.has_ival()
                        && (key != 10016 || prop.ival() >= 0),
                    "Invalid currency property"
                );
                let id = if key == 10015 { 201 } else { 202 };
                let count = prop.ival();
                let delta = if command == 22660 {
                    self.currencies
                        .get(&id)
                        .map(|old| {
                            count
                                .checked_sub(*old)
                                .ok_or_else(|| anyhow::anyhow!("Currency delta overflow"))
                        })
                        .transpose()?
                } else {
                    None
                };
                currencies.push((id, count, delta));
            } else if command != 7234 && key == 10020 {
                if let Ok(prop) = PropValue::parse_from_bytes(&value)
                    && prop.type_ == 10020
                    && !prop.has_fval()
                {
                    let count = if prop.has_ival() {
                        prop.ival()
                    } else {
                        prop.val
                    };
                    resin_count = u32::try_from(count).ok().filter(|count| *count <= 2000);
                }
            } else if command != 7234 && matches!(key, 10013 | 10014) {
                let prop = PropValue::parse_from_bytes(&value)?;
                let number = if prop.has_ival() {
                    prop.ival()
                } else {
                    prop.val
                };
                ensure!(
                    prop.type_ == key as u32
                        && !prop.has_fval()
                        && number >= 0
                        && number <= i64::from(u32::MAX)
                        && (key != 10013 || number > 0),
                    "Invalid Adventure Rank property"
                );
                adventure_props.insert(key, number as u32);
            } else if command == 7234 && self.friends.contains_key(&key.to_string()) {
                let info = AvatarFetterInfo::parse_from_bytes(&value)?;
                ensure!(
                    (1..=10).contains(&info.exp_level),
                    "Invalid friendship level"
                );
                friends.push((key.to_string(), info));
            }
        }
        let next_adventure = if command == 7193 {
            adventure_props
                .get(&10013)
                .zip(adventure_props.get(&10014))
                .map(|(rank, exp)| json!({"rank":rank,"exp":exp}))
        } else if command == 22660 && !adventure_props.is_empty() {
            self.adventure.as_ref().map(|old| {
                let mut next = old.clone();
                if let Some(rank) = adventure_props.get(&10013) {
                    next["rank"] = json!(rank);
                    if next["rank"] != old["rank"] {
                        next["exp"] = Value::Null;
                    }
                }
                if let Some(exp) = adventure_props.get(&10014) {
                    next["exp"] = json!(exp);
                }
                next
            })
        } else {
            None
        };
        if let (Some(old), Some(next)) = (&self.adventure, &next_adventure)
            && command == 22660
        {
            ensure!(
                next["rank"].as_u64() >= old["rank"].as_u64(),
                "Adventure Rank regressed"
            );
        }
        // All entries are validated before any mutation.
        let mut events = Vec::new();
        let previous_resin = self.resin;
        if let Some(count) = resin_count {
            let next = previous_resin
                .filter(|(old, _)| *old == count)
                .and_then(|(_, next)| next);
            self.resin = Some((count, next));
        }
        if command == 7193 {
            self.resin_baseline = true;
        }
        if self.resin_baseline
            && (command == 7193 || previous_resin != self.resin)
            && let Some((count, next)) = self.resin
        {
            events.push(resin_observation(count, next, command));
        }
        for (id, count, delta) in currencies {
            if command == 7193 {
                self.currencies.insert(id, count);
                events.push((
                    "currency_snapshot",
                    json!({"item_id":id,"count":count,"command_id":command}),
                ));
            } else if let Some(delta) = delta
                && delta != 0
            {
                let previous = self.currencies.insert(id, count).unwrap();
                events.push(("currency_changed",json!({"item_id":id,"previous_count":previous,"count":count,"delta":delta,"command_id":command})));
            }
        }
        if command == 7193 {
            self.adventure = next_adventure;
            if let Some(current) = &self.adventure {
                events.push((
                    "adventure_snapshot",
                    json!({"current":current,"command_id":command}),
                ));
            }
        } else if let Some(next) = next_adventure
            && self.adventure.as_ref() != Some(&next)
        {
            let previous = self.adventure.replace(next.clone()).unwrap();
            // EXP resets across ranks. Missing and decreasing values do not imply loss.
            let exp_delta = if previous["rank"] == next["rank"] {
                previous["exp"]
                    .as_u64()
                    .zip(next["exp"].as_u64())
                    .and_then(|(old, new)| new.checked_sub(old))
            } else {
                None
            };
            events.push(("adventure_progress_changed",json!({"previous":previous,"current":next,"exp_delta":exp_delta,"command_id":command})));
        }
        for (guid, info) in friends {
            let old = self.friends.get_mut(&guid).unwrap();
            if old["level"] == info.exp_level && old["exp"] == info.exp_number {
                continue;
            }
            let previous = old.clone();
            old["level"] = json!(info.exp_level);
            old["exp"] = json!(info.exp_number);
            // exp is progress within a level. A level transition is not a loss of experience.
            let exp_delta = if previous["level"] == info.exp_level {
                Some(i64::from(info.exp_number) - previous["exp"].as_i64().unwrap())
            } else {
                None
            };
            events.push(("friendship_changed",json!({"guid":guid,"avatar_id":old["avatar_id"],"previous":previous,"current":old,"exp_delta":exp_delta,"command_id":command})));
        }
        Ok(events)
    }
    fn apply_talent(&mut self, body: &[u8]) -> Result<Vec<(&'static str, Value)>> {
        let (guid, skill, previous, level) = crate::profile_protocol::talent_change(body)?;
        let Some(profile) = self.talents.get_mut(&guid.to_string()) else {
            return Ok(Vec::new());
        };
        let key = skill.to_string();
        let Some(current) = profile["skill_levels"][&key].as_u64() else {
            return Ok(Vec::new());
        };
        if current == u64::from(level) {
            return Ok(Vec::new());
        }
        ensure!(
            current == u64::from(previous),
            "Inconsistent talent baseline"
        );
        profile["skill_levels"][&key] = json!(level);
        if let Some(active) = self.traveler_active.get_mut(&guid.to_string()) {
            let before = active.clone();
            active["skill_levels"] = profile["skill_levels"].clone();
            return Ok(vec![(
                "traveler_element_changed",
                json!({"guid":guid.to_string(),"avatar_id":profile["avatar_id"],"previous":before,"current":active,"command_id":22610}),
            )]);
        }
        Ok(vec![(
            "talent_changed",
            json!({"guid":guid.to_string(),"avatar_id":profile["avatar_id"],"skill_id":skill,"previous_level":previous,"level":level,"command_id":22610}),
        )])
    }
    fn apply_character(&mut self, body: &[u8]) -> Result<Vec<(&'static str, Value)>> {
        let (guid, props) = crate::profile_protocol::character_properties(body)?;
        let Some(old) = self.characters.get_mut(&guid.to_string()) else {
            return Ok(Vec::new());
        };
        let mut next = old.clone();
        for (key, field) in [(4001, "level"), (1001, "exp"), (1002, "ascension")] {
            if let Some(value) = props.get(&key) {
                ensure!(
                    *value <= u64::from(u32::MAX) && (key != 4001 || *value > 0),
                    "Invalid character progression property"
                );
                next[field] = json!(*value);
            }
        }
        if next == *old {
            return Ok(Vec::new());
        }
        let previous = old.clone();
        let exp_delta = if previous["level"] == next["level"] {
            previous["exp"]
                .as_i64()
                .zip(next["exp"].as_i64())
                .map(|(a, b)| b - a)
        } else {
            None
        };
        *old = next.clone();
        Ok(vec![(
            "character_progress_changed",
            json!({"guid":guid.to_string(),"avatar_id":next["avatar_id"],"previous":previous,"current":next,"exp_delta":exp_delta,"command_id":561}),
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn envelope(field: u32, entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            for (key, value) in entries {
                let mut entry = Vec::new();
                {
                    let mut map = CodedOutputStream::vec(&mut entry);
                    map.write_uint64(1, *key).unwrap();
                    map.write_bytes(2, value).unwrap();
                    map.flush().unwrap();
                }
                out.write_bytes(field, &entry).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    fn mora(field: u32, count: i64) -> Vec<u8> {
        let mut prop = PropValue::new();
        prop.type_ = 10016;
        prop.set_ival(count);
        prop.val = count;
        envelope(field, &[(10016, prop.write_to_bytes().unwrap())])
    }
    fn friend(level: u32, exp: u32) -> Vec<u8> {
        let mut info = AvatarFetterInfo::new();
        info.exp_level = level;
        info.exp_number = exp;
        envelope(15, &[(u64::MAX, info.write_to_bytes().unwrap())])
    }
    fn progress(guid: u64, props: &[(u32, u64)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_uint64(5, guid).unwrap();
            for (key, value) in props {
                let mut entry = Vec::new();
                {
                    let mut map = CodedOutputStream::vec(&mut entry);
                    map.write_uint32(1, *key).unwrap();
                    if *value != 0 {
                        map.write_uint64(2, *value).unwrap();
                    }
                    map.flush().unwrap();
                }
                out.write_bytes(3, &entry).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    fn progression_state() -> State {
        let mut state = State::default();
        state.snapshot(&[json!({"guid":u64::MAX.to_string(),"avatar_id":10000137,"props":{"4001":{"ival":"20"},"1001":{"ival":"0"},"1002":{"ival":"1"}}})]);
        state
    }
    #[test]
    fn resin_notification_keeps_the_timer_across_matching_balance_observations() {
        fn update(count: i64) -> Vec<u8> {
            let mut property = PropValue::new();
            property.type_ = 10020;
            property.set_ival(count);
            envelope(8, &[(10020, property.write_to_bytes().unwrap())])
        }
        let mut packet = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut packet);
            out.write_uint32(11, 148).unwrap();
            out.write_uint32(12, 1_795_364_749).unwrap();
            out.flush().unwrap();
        }
        let mut state = State::default();
        assert!(state.apply(true, 24057, &packet).unwrap().is_empty());
        let initial = state.apply(true, 7193, &gems(6, 10)).unwrap();
        assert_eq!(initial[0].1["next_recovery_unix_s"], 1_795_364_749);
        assert!(state.apply(true, 22660, &update(148)).unwrap().is_empty());
        let spent = state.apply(true, 22660, &update(88)).unwrap();
        assert_eq!(spent[0].1["count"], 88);
        assert!(spent[0].1["next_recovery_unix_s"].is_null());
    }

    fn gems(field: u32, count: i64) -> Vec<u8> {
        let mut prop = PropValue::new();
        prop.type_ = 10015;
        prop.val = count;
        prop.set_ival(count);
        envelope(field, &[(10015, prop.write_to_bytes().unwrap())])
    }
    fn talent(guid: u64, skill: u32, old: u32, new: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_uint32(3, old).unwrap();
            out.write_uint32(5, new).unwrap();
            out.write_uint64(11, guid).unwrap();
            out.write_uint32(13, skill).unwrap();
            out.flush().unwrap();
        }
        bytes
    }
    fn addition(guid: u64, id: u32, trial: bool) -> Vec<u8> {
        use auto_artifactarium::r#gen::protos::AvatarInfo;
        let mut avatar = AvatarInfo::new();
        avatar.avatar_id = id;
        avatar.guid = guid;
        for (key, value) in [(4001, 1), (1001, 0), (1002, 0)] {
            let mut p = PropValue::new();
            p.type_ = key;
            p.val = value;
            p.set_ival(value);
            avatar.prop_map.insert(key, p);
        }
        avatar.skill_level_map.insert(10715, 1);
        let mut f = AvatarFetterInfo::new();
        f.exp_level = 1;
        avatar.fetter_info = Some(f).into();
        if trial {
            avatar
                .special_fields
                .mut_unknown_fields()
                .add_length_delimited(9, vec![]);
        }
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_bytes(4, &avatar.write_to_bytes().unwrap())
                .unwrap();
            out.flush().unwrap();
        }
        bytes
    }
    fn constellation(guid: u64, depot: u32, talent_id: u32) -> Vec<u8> {
        let mut bytes = vec![];
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_uint32(1, talent_id).unwrap();
            out.write_uint32(7, depot).unwrap();
            out.write_uint64(11, guid).unwrap();
            out.flush().unwrap();
        }
        bytes
    }
    fn adventure(field: u32, entries: &[(u32, i64)]) -> Vec<u8> {
        let values: Vec<_> = entries
            .iter()
            .map(|(key, value)| {
                let mut p = PropValue::new();
                p.type_ = *key;
                p.val = *value;
                p.set_ival(*value);
                (*key as u64, p.write_to_bytes().unwrap())
            })
            .collect();
        envelope(field, &values)
    }
    #[test]
    fn adventure_rank_then_exp_are_separate_without_fake_loss_and_reset_on_session() {
        let mut state = State::default();
        assert!(
            state
                .apply(true, 22660, &adventure(8, &[(10014, 845)]))
                .unwrap()
                .is_empty()
        );
        assert!(
            state
                .apply(false, 7193, &adventure(6, &[(10013, 11), (10014, 815)]))
                .unwrap()
                .is_empty()
        );
        let baseline = state
            .apply(true, 7193, &adventure(6, &[(10013, 11), (10014, 815)]))
            .unwrap();
        assert_eq!(baseline[0].0, "adventure_snapshot");
        let e = state
            .apply(true, 22660, &adventure(8, &[(10014, 1515)]))
            .unwrap();
        assert_eq!(e[0].1["exp_delta"], 700);
        assert!(
            state
                .apply(true, 22660, &adventure(8, &[(10014, 1515)]))
                .unwrap()
                .is_empty()
        );
        let rank = state
            .apply(true, 22660, &adventure(8, &[(10013, 12)]))
            .unwrap();
        assert_eq!(rank[0].1["previous"]["exp"], 1515);
        assert_eq!(rank[0].1["current"]["rank"], 12);
        assert!(rank[0].1["current"]["exp"].is_null());
        assert!(rank[0].1["exp_delta"].is_null());
        let exp = state
            .apply(true, 22660, &adventure(8, &[(10014, 20)]))
            .unwrap();
        assert_eq!(exp[0].1["current"]["exp"], 20);
        assert!(exp[0].1["exp_delta"].is_null());
        let exp = state
            .apply(true, 22660, &adventure(8, &[(10014, 50)]))
            .unwrap();
        assert_eq!(exp[0].1["exp_delta"], 30);
        state.reset();
        assert!(
            state
                .apply(true, 22660, &adventure(8, &[(10013, 13), (10014, 0)]))
                .unwrap()
                .is_empty()
        );
        let e = state
            .apply(true, 7193, &adventure(6, &[(10013, 60), (10014, 0)]))
            .unwrap();
        assert_eq!(e[0].1["current"], json!({"rank":60,"exp":0}));
    }
    #[test]
    fn adventure_combined_updates_and_invalid_batches_do_not_partially_mutate_currencies() {
        let mut state = State::default();
        state
            .apply(
                true,
                7193,
                &[adventure(6, &[(10013, 11), (10014, 1515)]), gems(6, 18)].concat(),
            )
            .unwrap();
        for values in [
            vec![(10013, 0)],
            vec![(10013, -1)],
            vec![(10014, -1)],
            vec![(10014, i64::from(u32::MAX) + 1)],
            vec![(10013, 10)],
            vec![(10014, 20), (10014, 30)],
        ] {
            assert!(
                state
                    .apply(true, 22660, &[gems(8, 23), adventure(8, &values)].concat())
                    .is_err()
            );
            assert_eq!(state.currencies[&201], 18);
            assert_eq!(state.adventure, Some(json!({"rank":11,"exp":1515})));
        }
        let e = state
            .apply(true, 22660, &adventure(8, &[(10014, 20), (10013, 12)]))
            .unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].1["current"], json!({"rank":12,"exp":20}));
        assert!(e[0].1["exp_delta"].is_null());
        // A lower raw EXP value is retained, but not labeled as a loss.
        let e = state
            .apply(true, 22660, &adventure(8, &[(10014, 0)]))
            .unwrap();
        assert!(e[0].1["exp_delta"].is_null());
    }
    #[test]
    fn traveler_other_element_activation_preserves_full_baseline_and_deduplicates() {
        for avatar in [10000005, 10000007] {
            let mut state = State::default();
            let baseline = json!({"guid":"42","avatar_id":avatar,"skill_depot_id":505,"talent_ids":[81,82,83,84,85,86],"skill_levels":{"100544":6}});
            state.snapshot(std::slice::from_ref(&baseline));
            let original = state.constellations["42"].clone();
            let body = constellation(42, 504, 31);
            let events = state.apply(true, 3996, &body).unwrap();
            assert_eq!(events[0].0, "constellation_depot_changed");
            assert!(events[0].1["previous"].is_null());
            assert_eq!(
                events[0].1["current"],
                json!({"observed_talent_ids":[31],"completeness":"partial"})
            );
            assert!(events[0].1["count"].is_null());
            assert!(state.apply(true, 3996, &body).unwrap().is_empty());
            let next = state
                .apply(true, 3996, &constellation(42, 504, 32))
                .unwrap();
            assert_eq!(next[0].1["previous"], events[0].1["current"]);
            assert_eq!(state.constellations["42"], original);
            assert_eq!(
                state.talents["42"]["skill_levels"],
                baseline["skill_levels"]
            );
            for id in 33..=36 {
                state
                    .apply(true, 3996, &constellation(42, 504, id))
                    .unwrap();
            }
            assert!(
                state
                    .apply(true, 3996, &constellation(42, 504, 37))
                    .is_err()
            );
            state.snapshot(&[baseline]);
            assert_eq!(state.apply(true, 3996, &body).unwrap().len(), 1);
            state.reset();
            assert!(state.apply(true, 3996, &body).unwrap().is_empty());
        }
    }
    #[test]
    fn constellation_activation_is_once_per_guid_and_does_not_change_base_skill_levels() {
        let mut state = State::default();
        let body = constellation(u64::MAX, 2501, 253);
        assert!(state.apply(true, 3996, &body).unwrap().is_empty());
        state.snapshot(&[json!({"guid":u64::MAX.to_string(),"avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[251,252],"skill_levels":{"10382":6},"extra_skill_levels":{}})]);
        assert!(state.apply(false, 3996, &body).unwrap().is_empty());
        let e = state.apply(true, 3996, &body).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, "constellation_changed");
        assert_eq!(e[0].1["previous_count"], 2);
        assert_eq!(e[0].1["count"], 3);
        assert_eq!(e[0].1["current"]["talent_ids"], json!([251, 252, 253]));
        assert_eq!(
            state.talents[&u64::MAX.to_string()]["skill_levels"]["10382"],
            6
        );
        assert!(state.apply(true, 3996, &body).unwrap().is_empty());
        assert!(state.apply(true, 1528, &body).unwrap().is_empty());
        state.reset();
        assert!(state.apply(true, 3996, &body).unwrap().is_empty());
    }
    #[test]
    fn constellation_activation_validates_identity_payload_and_six_slot_limit() {
        let mut state = State::default();
        state.snapshot(&[json!({"guid":"42","avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[]}),json!({"guid":"43","avatar_id":10000025,"skill_depot_id":2501,"talent_ids":[],"trial_info_present":true})]);
        assert!(
            state
                .apply(true, 3996, &constellation(43, 2501, 251))
                .unwrap()
                .is_empty()
        );
        let good = constellation(42, 2501, 251);
        for body in [
            vec![],
            vec![10, 0],
            constellation(0, 2501, 251),
            constellation(42, 0, 251),
            constellation(42, 2501, 0),
            [good.clone(), good.clone()].concat(),
            constellation(42, 3401, 251),
        ] {
            assert!(state.apply(true, 3996, &body).is_err());
            assert_eq!(state.constellations["42"]["talent_ids"], json!([]));
        }
        for id in 251..=256 {
            assert_eq!(
                state
                    .apply(true, 3996, &constellation(42, 2501, id))
                    .unwrap()
                    .len(),
                1
            );
        }
        assert!(
            state
                .apply(true, 3996, &constellation(42, 2501, 257))
                .is_err()
        );
        assert_eq!(
            state.constellations["42"]["talent_ids"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
    }
    #[test]
    fn character_addition_requires_baseline_is_idempotent_and_initializes_all_tracking() {
        let body = addition(u64::MAX, 10000025, false);
        let mut state = State::default();
        assert!(state.apply(true, 3231, &body).unwrap().is_empty());
        state.snapshot(&[json!({"guid":"42","avatar_id":10000034})]);
        state.apply(true, 7193, &gems(6, 18)).unwrap();
        assert!(state.apply(false, 3231, &body).unwrap().is_empty());
        let events = state.apply(true, 3231, &body).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "character_added");
        assert_eq!(events[0].1["guid"], u64::MAX.to_string());
        assert_eq!(state.characters.len(), 2);
        assert_eq!(state.currencies[&201], 18);
        assert!(state.apply(true, 3231, &body).unwrap().is_empty());
        assert_eq!(
            state
                .apply(true, 22610, &talent(u64::MAX, 10715, 1, 2))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(state.apply(true, 7234, &friend(1, 10)).unwrap().len(), 1);
        assert_eq!(
            state
                .apply(true, 561, &progress(u64::MAX, &[(4001, 2)]))
                .unwrap()
                .len(),
            1
        );
        // A delayed repeat must not reset progression, friendship or talents.
        assert!(state.apply(true, 3231, &body).unwrap().is_empty());
        assert_eq!(state.characters[&u64::MAX.to_string()]["level"], 2);
        assert_eq!(
            state.talents[&u64::MAX.to_string()]["skill_levels"]["10715"],
            2
        );
        state.reset();
        assert!(state.apply(true, 3231, &body).unwrap().is_empty());
    }
    #[test]
    fn character_addition_rejects_damaged_duplicate_and_conflicting_identity_without_mutation() {
        let mut state = State::default();
        state.snapshot(&[]);
        assert!(
            state
                .apply(true, 3231, &addition(44, 10000025, true))
                .unwrap()
                .is_empty()
        );
        assert!(state.characters.is_empty());
        let body = addition(42, 10000025, false);
        for malformed in [
            vec![],
            vec![32, 1],
            [body.clone(), body.clone()].concat(),
            addition(0, 10000025, false),
            addition(42, 0, false),
        ] {
            assert!(state.apply(true, 3231, &malformed).is_err());
            assert!(state.characters.is_empty());
        }
        state.apply(true, 3231, &body).unwrap();
        assert!(
            state
                .apply(true, 3231, &addition(42, 10000034, false))
                .is_err()
        );
        assert!(
            state
                .apply(true, 3231, &addition(43, 10000025, false))
                .is_err()
        );
        assert_eq!(state.characters.len(), 1);
    }
    #[test]
    fn primogem_baseline_gain_spend_zero_negative_duplicates_and_reset() {
        let mut state = State::default();
        assert!(state.apply(true, 22660, &gems(8, 99)).unwrap().is_empty());
        assert!(state.apply(false, 7193, &gems(6, 99)).unwrap().is_empty());
        let baseline = state.apply(true, 7193, &gems(6, 99)).unwrap();
        assert_eq!(baseline[0].0, "currency_snapshot");
        assert_eq!(baseline[0].1["item_id"], 201);
        for (count, delta) in [(109, 10), (159, 50), (0, -159), (-10, -10), (0, 10)] {
            let events = state.apply(true, 22660, &gems(8, count)).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].1["count"], count);
            assert_eq!(events[0].1["delta"], delta);
            assert!(
                state
                    .apply(true, 22660, &gems(8, count))
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(!state.currencies.contains_key(&202));
        state.reset();
        assert!(state.apply(true, 22660, &gems(8, 50)).unwrap().is_empty());
    }
    #[test]
    fn currencies_validate_whole_envelope_before_mutation_and_reject_overflow() {
        let mut state = State::default();
        state.apply(true, 7193, &gems(6, 99)).unwrap();
        let mut bytes = gems(8, 109);
        bytes.extend(mora(8, -1));
        assert!(state.apply(true, 22660, &bytes).is_err());
        assert_eq!(state.currencies[&201], 99);
        let mut duplicate = gems(8, 109);
        duplicate.extend(gems(8, 119));
        assert!(state.apply(true, 22660, &duplicate).is_err());
        assert_eq!(state.currencies[&201], 99);
        state.apply(true, 7193, &gems(6, i64::MAX)).unwrap();
        assert!(state.apply(true, 22660, &gems(8, i64::MIN)).is_err());
        assert_eq!(state.currencies[&201], i64::MAX);
        let mut malformed = gems(8, 100);
        malformed.push(0xff);
        assert!(state.apply(true, 22660, &malformed).is_err());
        assert_eq!(state.currencies[&201], i64::MAX);
    }
    #[test]
    fn talent_updates_only_known_nontrial_baselines_and_deduplicates_notifications() {
        let mut state = State::default();
        let bytes = talent(42, 10715, 5, 6);
        assert!(state.apply(true, 22610, &bytes).unwrap().is_empty());
        state.snapshot(&[
            json!({"guid":"42","avatar_id":10000071,"skill_levels":{"10715":5}}),
            json!({"guid":"43","avatar_id":10000127,"trial_info_present":true,"skill_levels":{"10715":5}})]);
        assert!(state.apply(false, 22610, &bytes).unwrap().is_empty());
        assert!(state.apply(true, 21869, &bytes).unwrap().is_empty());
        assert!(
            state
                .apply(true, 22610, &talent(43, 10715, 5, 6))
                .unwrap()
                .is_empty()
        );
        assert!(
            state
                .apply(true, 22610, &talent(42, 999, 5, 6))
                .unwrap()
                .is_empty()
        );
        let events = state.apply(true, 22610, &bytes).unwrap();
        assert_eq!(events[0].0, "talent_changed");
        assert_eq!(events[0].1["avatar_id"], 10000071);
        assert_eq!(events[0].1["level"], 6);
        assert!(state.apply(true, 22610, &bytes).unwrap().is_empty());
        assert_eq!(
            state
                .apply(true, 22610, &talent(42, 10715, 6, 7))
                .unwrap()
                .len(),
            1
        );
        assert!(state.apply(true, 22610, &talent(42, 10715, 5, 8)).is_err());
        assert_eq!(state.talents["42"]["skill_levels"]["10715"], 7);
        state.reset();
        assert!(
            state
                .apply(true, 22610, &talent(42, 10715, 7, 8))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn malformed_talent_wire_types_duplicates_missing_fields_and_zero_do_not_mutate() {
        let mut state = State::default();
        state.snapshot(&[json!({"guid":"42","avatar_id":10000071,"skill_levels":{"10715":5}})]);
        let valid = talent(42, 10715, 5, 6);
        let mut duplicate = valid.clone();
        duplicate.extend([24, 5]);
        let mut malformed = valid.clone();
        malformed.push(0xff);
        for bytes in [
            vec![],
            duplicate,
            malformed,
            talent(0, 10715, 5, 6),
            talent(42, 10715, 0, 6),
            vec![26, 0],
        ] {
            assert!(state.apply(true, 22610, &bytes).is_err());
            assert_eq!(state.talents["42"]["skill_levels"]["10715"], 5);
        }
    }
    #[test]
    fn trials_are_excluded_without_filtering_unmarked_character_types() {
        let mut state = State::default();
        state.snapshot(&[
            json!({"guid":"42","avatar_id":10000127,"trial_info_present":true,"friendship":{"level":1,"exp":0}}),
            json!({"guid":"43","avatar_id":10000127,"avatar_type":3,"friendship":{"level":1,"exp":0}}),
        ]);
        assert!(!state.characters.contains_key("42"));
        assert!(!state.friends.contains_key("42"));
        assert!(state.characters.contains_key("43"));
        assert!(state.friends.contains_key("43"));
        assert!(
            state
                .apply(true, 561, &progress(42, &[(4001, 91)]))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn character_exp_level_partial_updates_zero_duplicates_and_reset() {
        let mut state = progression_state();
        assert!(
            state
                .apply(false, 561, &progress(u64::MAX, &[(1001, 1000)]))
                .unwrap()
                .is_empty()
        );
        let first = state
            .apply(true, 561, &progress(u64::MAX, &[(1001, 1000)]))
            .unwrap();
        assert_eq!(first[0].0, "character_progress_changed");
        assert_eq!(first[0].1["current"]["level"], 20);
        assert_eq!(first[0].1["exp_delta"], 1000);
        assert!(
            state
                .apply(true, 561, &progress(u64::MAX, &[(1001, 1000)]))
                .unwrap()
                .is_empty()
        );
        let next = state
            .apply(true, 561, &progress(u64::MAX, &[(1001, 6125), (4001, 21)]))
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].1["current"]["level"], 21);
        assert_eq!(next[0].1["current"]["exp"], 6125);
        assert!(next[0].1["exp_delta"].is_null());
        assert_eq!(next[0].1["current"]["ascension"], 1);
        assert_eq!(
            state
                .apply(true, 561, &progress(u64::MAX, &[(1001, 0)]))
                .unwrap()[0]
                .1["current"]["exp"],
            0
        );
        assert!(
            state
                .apply(true, 561, &progress(42, &[(4001, 22)]))
                .unwrap()
                .is_empty()
        );
        state.reset();
        assert!(
            state
                .apply(true, 561, &progress(u64::MAX, &[(4001, 22)]))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn malformed_and_invalid_character_updates_do_not_partly_apply() {
        let mut state = progression_state();
        let mut bytes = progress(u64::MAX, &[(4001, 21)]);
        bytes.extend([26, 255]);
        assert!(state.apply(true, 561, &bytes).is_err());
        assert!(
            state
                .apply(
                    true,
                    561,
                    &progress(u64::MAX, &[(4001, 21), (1001, u64::MAX)])
                )
                .is_err()
        );
        assert!(
            state
                .apply(true, 561, &progress(u64::MAX, &[(1001, 1), (1001, 2)]))
                .is_err()
        );
        assert!(state.apply(true, 561, &progress(0, &[(4001, 21)])).is_err());
        assert_eq!(state.characters[&u64::MAX.to_string()]["level"], 20);
        assert_eq!(state.characters[&u64::MAX.to_string()]["exp"], 0);
    }
    #[test]
    fn mora_baseline_absolute_gain_spend_zero_duplicate_and_session_reset() {
        let mut state = State::default();
        assert!(state.apply(true, 22660, &mora(8, 100)).unwrap().is_empty());
        assert!(state.apply(false, 7193, &mora(6, 100)).unwrap().is_empty());
        assert_eq!(
            state.apply(true, 7193, &mora(6, 100)).unwrap()[0].0,
            "currency_snapshot"
        );
        let gained = state.apply(true, 22660, &mora(8, 120)).unwrap();
        assert_eq!(gained[0].1["delta"], 20);
        assert!(state.apply(true, 22660, &mora(8, 120)).unwrap().is_empty());
        assert_eq!(
            state.apply(true, 22660, &mora(8, 90)).unwrap()[0].1["delta"],
            -30
        );
        assert_eq!(
            state.apply(true, 22660, &mora(8, 0)).unwrap()[0].1["delta"],
            -90
        );
        assert!(state.apply(true, 22660, &mora(8, -1)).is_err());
        assert_eq!(state.currencies.get(&202), Some(&0));
        state.reset();
        assert!(state.apply(true, 22660, &mora(8, 50)).unwrap().is_empty());
    }
    #[test]
    fn profile_envelopes_validate_before_mutation() {
        let mut state = State::default();
        state.apply(true, 7193, &mora(6, 100)).unwrap();
        let mut broken = mora(8, 110);
        broken.extend([66, 255]);
        assert!(state.apply(true, 22660, &broken).is_err());
        assert_eq!(state.currencies.get(&202), Some(&100));
        let mut prop = PropValue::new();
        prop.type_ = 10016;
        prop.set_ival(9);
        let bytes = prop.write_to_bytes().unwrap();
        let duplicate = envelope(8, &[(10016, bytes.clone()), (10016, bytes)]);
        assert!(state.apply(true, 22660, &duplicate).is_err());
        assert_eq!(state.currencies.get(&202), Some(&100));
    }
    #[test]
    fn friendship_gain_level_transition_duplicates_unknown_guid_and_missing_baseline() {
        let mut state = State::default();
        assert!(
            state
                .apply(true, 7234, &friend(3, 1875))
                .unwrap()
                .is_empty()
        );
        state.snapshot(&[json!({"guid":u64::MAX.to_string(),"avatar_id":10000035,"friendship":{"level":3,"exp":1855}})]);
        let gain = state.apply(true, 7234, &friend(3, 1875)).unwrap();
        assert_eq!(gain[0].1["exp_delta"], 20);
        assert!(
            state
                .apply(true, 7234, &friend(3, 1875))
                .unwrap()
                .is_empty()
        );
        let level = state.apply(true, 7234, &friend(4, 5)).unwrap();
        assert!(level[0].1["exp_delta"].is_null());
        assert_eq!(level[0].1["current"]["level"], 4);
        let mut broken = friend(4, 25);
        broken.extend([122, 255]);
        assert!(state.apply(true, 7234, &broken).is_err());
        assert_eq!(state.friends[&u64::MAX.to_string()]["exp"], 5);
        assert!(state.apply(true, 7234, &friend(11, 5)).is_err());
        state.reset();
        assert!(state.apply(true, 7234, &friend(4, 25)).unwrap().is_empty());
    }
}
