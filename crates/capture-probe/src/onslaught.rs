use anyhow::{Result, ensure};
use protobuf::CodedInputStream;
use serde_json::{Value, json};

use crate::profile_protocol::{nested, uint_fields};

pub const EXALTED_RESPONSE: u16 = 20472;

#[derive(Default)]
pub struct State {
    schedule: Option<(u32, u32, u32)>,
    record: Option<(u32, u32, u32)>,
}

impl State {
    pub fn exalted_snapshot(&self, body: &[u8]) -> Result<Option<Value>> {
        let Some((schedule, starts, ends)) = self.schedule else {
            return Ok(None);
        };
        // Observed when opening «Вознесённые»: field 3 contains packed avatar IDs.
        let mut lists = nested(body)?.into_iter().filter(|(field, _)| *field == 3);
        let Some((_, packed)) = lists.next() else {
            return Ok(None);
        };
        ensure!(lists.next().is_none(), "Duplicate Onslaught roster field");
        let mut input = CodedInputStream::from_bytes(&packed);
        let mut avatars = Vec::new();
        while !input.eof()? {
            let id = input.read_uint64()?;
            ensure!(
                (10_000_001..11_000_000).contains(&id)
                    && avatars.len() < 16
                    && !avatars.contains(&id),
                "Invalid Onslaught roster"
            );
            avatars.push(id);
        }
        if avatars.is_empty() {
            return Ok(None);
        }
        Ok(Some(json!({
            "schedule_id": schedule, "starts_unix_s": starts, "ends_unix_s": ends,
            "avatar_ids": avatars
        })))
    }

    pub fn apply(&mut self, command: u16, body: &[u8], uid: &str) -> Result<Option<Value>> {
        let before = self.snapshot();
        match command {
            23766 => {
                // ActivityScheduleInfoNotify, activity 5269 is Stygian Onslaught.
                let mut schedule = None;
                for (_, entry) in nested(body)?.into_iter().filter(|(field, _)| *field == 4) {
                    let fields = uint_fields(&entry, &[4, 5, 10, 11, 12])?;
                    if fields.get(&5) != Some(&5269) || fields.get(&12) != Some(&1) {
                        continue;
                    }
                    if let (Some(&id), Some(&starts), Some(&ends)) =
                        (fields.get(&10), fields.get(&4), fields.get(&11))
                    {
                        ensure!(
                            id > 0
                                && starts >= 1_600_000_000
                                && ends > starts
                                && ends - starts <= 60 * 86400,
                            "Invalid Onslaught cycle"
                        );
                        ensure!(
                            schedule.replace((id, starts, ends)).is_none(),
                            "Duplicate Onslaught activity"
                        );
                    }
                }
                if let Some(schedule) = schedule {
                    self.schedule = Some(schedule);
                }
            }
            9233 => {
                // Social detail can also describe another player. Only accept this login's UID.
                for (_, entry) in nested(body)?.into_iter().filter(|(field, _)| *field == 2) {
                    let fields = uint_fields(&entry, &[1, 37, 38, 39])?;
                    if fields.get(&1).is_none_or(|id| id.to_string() != uid) {
                        continue;
                    }
                    if let (Some(&id), Some(&difficulty), Some(&seconds)) =
                        (fields.get(&39), fields.get(&37), fields.get(&38))
                    {
                        ensure!(
                            id > 0
                                && (1..=6).contains(&difficulty)
                                && (1..=86400).contains(&seconds),
                            "Invalid Onslaught result"
                        );
                        self.record = Some((id, difficulty, seconds));
                    }
                }
            }
            _ => return Ok(None),
        }
        let after = self.snapshot();
        Ok(after.filter(|value| Some(value) != before.as_ref()))
    }

    fn snapshot(&self) -> Option<Value> {
        let (schedule, starts, ends) = self.schedule?;
        let (record, difficulty, seconds) = self.record?;
        (schedule == record).then(|| {
            json!({
                "schedule_id": schedule, "starts_unix_s": starts, "ends_unix_s": ends,
                "difficulty": difficulty, "seconds": seconds
            })
        })
    }
}
