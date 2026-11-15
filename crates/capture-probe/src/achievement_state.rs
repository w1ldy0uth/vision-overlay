//! Verified 7.2 achievement baseline 4164/9 and absolute updates 4301/13.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    ready: bool,
    records: BTreeMap<u64, Value>,
}
impl State {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn apply(
        &mut self,
        incoming: bool,
        command: u16,
        body: &[u8],
    ) -> Result<Vec<(&'static str, Value)>> {
        if !incoming || !matches!(command, 4164 | 4301) || (command == 4301 && !self.ready) {
            return Ok(vec![]);
        }
        let rows =
            crate::achievement_research::parse_records(body, if command == 4164 { 9 } else { 13 })?;
        for row in &rows {
            ensure!(
                row["status_code"]
                    .as_u64()
                    .is_some_and(|s| (1..=3).contains(&s)),
                "Invalid achievement status"
            );
            for key in ["total_progress", "finish_unix"] {
                ensure!(
                    row[key].as_u64().is_some_and(|n| n <= u64::from(u32::MAX)),
                    "Invalid achievement counter"
                );
            }
            ensure!(
                row["current_progress"].is_null()
                    || row["current_progress"]
                        .as_u64()
                        .is_some_and(|n| n <= u64::from(u32::MAX)),
                "Invalid achievement progress"
            );
        }
        if command == 4164 {
            self.records = rows
                .iter()
                .map(|r| (r["achievement_id"].as_u64().unwrap(), r.clone()))
                .collect();
            self.ready = true;
            return Ok(vec![(
                "achievement_snapshot",
                json!({"records":rows,"command_id":command}),
            )]);
        }
        // Validate the whole batch before mutating any record.
        for row in &rows {
            if let Some(old) = self.records.get(&row["achievement_id"].as_u64().unwrap()) {
                ensure!(
                    row["status_code"].as_u64() >= old["status_code"].as_u64(),
                    "Achievement status regressed"
                );
            }
        }
        let mut events = vec![];
        for row in rows {
            let id = row["achievement_id"].as_u64().unwrap();
            let previous = self.records.get(&id).cloned();
            if previous.as_ref() == Some(&row) {
                continue;
            }
            let action = match row["status_code"].as_u64() {
                Some(3) if previous.as_ref().is_none_or(|p| p["status_code"] != 3) => {
                    "reward_claimed"
                }
                Some(2) if previous.as_ref().is_none_or(|p| p["status_code"] != 2) => "completed",
                _ => "progress",
            };
            self.records.insert(id, row.clone());
            events.push(("achievement_changed",json!({"achievement_id":id,"previous":previous,"current":row,"action":action,"command_id":command})));
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn message(field: u32, rows: &[(u32, u32, Option<u32>, u32)]) -> Vec<u8> {
        let mut bytes = vec![];
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            for (id, status, progress, total) in rows {
                let mut record = vec![];
                {
                    let mut r = CodedOutputStream::vec(&mut record);
                    r.write_uint32(9, *id).unwrap();
                    r.write_uint32(7, *status).unwrap();
                    r.write_uint32(8, *total).unwrap();
                    if let Some(n) = progress {
                        r.write_uint32(6, *n).unwrap();
                    }
                    r.flush().unwrap();
                }
                out.write_bytes(field, &record).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    #[test]
    fn progress_completion_claim_and_duplicates_preserve_missing_progress() {
        let mut state = State::default();
        assert!(
            state
                .apply(true, 4301, &message(13, &[(82040, 1, Some(1), 4)]))
                .unwrap()
                .is_empty()
        );
        assert!(
            state
                .apply(false, 4164, &message(9, &[(82040, 1, None, 4)]))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            state
                .apply(true, 4164, &message(9, &[(82040, 1, None, 4)]))
                .unwrap()[0]
                .0,
            "achievement_snapshot"
        );
        for (status, progress, action) in [
            (1, Some(1), "progress"),
            (1, Some(3), "progress"),
            (2, None, "completed"),
            (3, None, "reward_claimed"),
            (3, Some(10), "progress"),
        ] {
            let body = message(13, &[(82040, status, progress, 4)]);
            let e = state.apply(true, 4301, &body).unwrap();
            assert_eq!(e.len(), 1);
            assert_eq!(e[0].1["action"], action);
            assert_eq!(e[0].1["current"]["current_progress"], json!(progress));
            assert!(state.apply(true, 4301, &body).unwrap().is_empty());
        }
        state.reset();
        assert!(
            state
                .apply(true, 4301, &message(13, &[(82040, 3, None, 4)]))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn batches_are_atomic_and_new_ids_are_not_assigned_fake_previous_values() {
        let mut state = State::default();
        state
            .apply(
                true,
                4164,
                &message(9, &[(80037, 1, Some(58), 100), (82040, 3, None, 4)]),
            )
            .unwrap();
        for body in [
            message(13, &[(80037, 1, Some(59), 100), (82040, 1, Some(1), 4)]),
            message(13, &[(80037, 1, Some(59), 100), (80037, 1, Some(60), 100)]),
            message(13, &[(80037, 4, Some(59), 100)]),
            vec![106, 255],
        ] {
            assert!(state.apply(true, 4301, &body).is_err());
            assert_eq!(state.records[&80037]["current_progress"], 58);
        }
        let e = state
            .apply(true, 4301, &message(13, &[(80038, 1, Some(59), 200)]))
            .unwrap();
        assert!(e[0].1["previous"].is_null());
        assert!(
            state
                .apply(true, 4164, &message(9, &[(1, 4, None, 1)]))
                .is_err()
        );
        assert!(state.records.contains_key(&82040));
    }
}
