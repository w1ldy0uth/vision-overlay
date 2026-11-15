//! Research-only protocol7.2 achievement baseline observations.
use anyhow::{Result, ensure};
use protobuf::CodedInputStream;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub fn parse(body: &[u8]) -> Result<Value> {
    parse_records(body, 9).map(|records| json!({"records":records,"command_id":4164,"interpretation":"research_achievement_fields_not_applied_to_account_state"}))
}

pub fn parse_records(body: &[u8], field: u32) -> Result<Vec<Value>> {
    ensure!(
        body.len() <= 2 * 1024 * 1024,
        "Achievement snapshot too large"
    );
    let entries = crate::profile_protocol::nested(body)?;
    let mut records = Vec::new();
    let mut ids = HashSet::new();
    for (_, bytes) in entries.into_iter().filter(|(number, _)| *number == field) {
        ensure!(
            records.len() < 10000 && bytes.len() <= 4096,
            "Achievement research limit"
        );
        let mut input = CodedInputStream::from_bytes(&bytes);
        let mut fields = BTreeMap::new();
        let mut count = 0;
        while let Some(tag) = input.read_raw_tag_or_eof()? {
            count += 1;
            ensure!(count <= 128, "Too many achievement fields");
            if tag & 7 == 0 {
                ensure!(
                    !fields.contains_key(&(tag >> 3)),
                    "Duplicate achievement scalar"
                );
                fields.insert(tag >> 3, input.read_uint64()?);
            } else {
                ensure!(
                    !matches!(tag >> 3, 4 | 6 | 7 | 8 | 9),
                    "Invalid achievement wire type"
                );
                protobuf::rt::skip_field_for_tag(tag, &mut input)?;
            }
        }
        let id = *fields
            .get(&9)
            .ok_or_else(|| anyhow::anyhow!("Missing achievement ID"))?;
        ensure!(
            id > 0 && id <= u64::from(u32::MAX) && ids.insert(id),
            "Invalid or duplicate achievement ID"
        );
        records.push(json!({"achievement_id":id,"status_code":fields.get(&7).copied().unwrap_or(0),"total_progress":fields.get(&8).copied().unwrap_or(0),"current_progress":fields.get(&6).copied(),"finish_unix":fields.get(&4).copied().unwrap_or(0),"raw_scalar_fields":fields}));
    }
    ensure!(!records.is_empty(), "No achievement records");
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    fn snapshot(rows: &[Vec<(u32, u64)>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut out = CodedOutputStream::vec(&mut bytes);
            for fields in rows {
                let mut record = Vec::new();
                {
                    let mut out = CodedOutputStream::vec(&mut record);
                    for (field, value) in fields {
                        out.write_uint64(*field, *value).unwrap();
                    }
                    out.flush().unwrap();
                }
                out.write_bytes(9, &record).unwrap();
            }
            out.flush().unwrap();
        }
        bytes
    }
    #[test]
    fn completed_and_partial_records_keep_current_and_total_separate() {
        let body = snapshot(&[
            vec![(9, 80001), (7, 3), (8, 1), (4, 1607179091)],
            vec![(9, 82215), (7, 1), (8, 5), (6, 4)],
        ]);
        let result = parse(&body).unwrap();
        let rows = result["records"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["achievement_id"], 80001);
        assert_eq!(rows[0]["status_code"], 3);
        assert!(rows[0]["current_progress"].is_null());
        assert_eq!(rows[0]["finish_unix"], 1607179091);
        assert_eq!(rows[1]["current_progress"], 4);
        assert_eq!(rows[1]["total_progress"], 5);
        assert_eq!(rows[1]["finish_unix"], 0);
        assert_eq!(
            parse(&snapshot(&[vec![(9, 82215), (7, 1), (8, 5), (6, 0)]])).unwrap()["records"][0]["current_progress"],
            0
        );
    }
    #[test]
    fn duplicate_ids_fields_missing_ids_and_malformed_messages_are_rejected() {
        assert!(parse(&snapshot(&[vec![(9, 80001)], vec![(9, 80001)]])).is_err());
        assert!(parse(&snapshot(&[vec![(9, 80001), (9, 80002)]])).is_err());
        assert!(parse(&snapshot(&[vec![(9, 0)]])).is_err());
        assert!(parse(&snapshot(&[vec![(7, 3)]])).is_err());
        assert!(parse(&[74, 255]).is_err());
        assert!(parse(&[]).is_err());
        let mut body = snapshot(&[vec![(9, 80001)]]);
        body.push(255);
        assert!(parse(&body).is_err());
    }
}
