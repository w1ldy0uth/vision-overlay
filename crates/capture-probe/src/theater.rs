use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::profile_protocol::{nested, uint_fields};

pub const LOGIN_RESPONSE: u16 = 163;
pub const SCREEN_RESPONSE: u16 = 28867;

// PC 7.2 uses the same season block in the login and Theater screen responses.
pub fn snapshot(command: u16, body: &[u8]) -> Result<Value> {
    let season_field = match command {
        LOGIN_RESPONSE => 8,
        SCREEN_RESPONSE => 11,
        _ => anyhow::bail!("Unsupported Theater response"),
    };
    ensure!(body.len() <= 32768, "Oversized Theater response");
    let mut blocks = nested(body)?
        .into_iter()
        .filter(|(field, _)| *field == season_field);
    let (_, block) = blocks.next().context("Missing Theater season")?;
    ensure!(blocks.next().is_none(), "Duplicate Theater season");

    let fields = uint_fields(&block, &[1, 3, 5, 15, 963])?;
    let schedule = *fields
        .get(&1)
        .filter(|&&id| id > 0)
        .context("Missing Theater schedule")?;
    let starts = *fields.get(&963).context("Missing Theater start")?;
    let ends = *fields.get(&15).context("Missing Theater end")?;
    let acts = fields.get(&5).copied().unwrap_or(0);
    // This field can disappear in later responses within the same season.
    let arcana = fields.get(&3).copied().filter(|&count| count > 0);
    ensure!(
        starts >= 1_600_000_000
            && ends > starts
            && ends - starts <= 40 * 86400
            && acts <= 10
            && arcana.is_none_or(|count| count <= 2),
        "Invalid Theater season progress"
    );
    // Keep unknown claim layouts separate from the verified season progress.
    let echo_keys = echo_key_claims(&block).ok().flatten();
    Ok(
        json!({"schedule_id":schedule,"starts_unix_s":starts,"ends_unix_s":ends,
        "acts":acts,"arcana":arcana,"echo_keys":echo_keys}),
    )
}

fn echo_key_claims(block: &[u8]) -> Result<Option<u32>> {
    let mut claims = std::collections::BTreeMap::new();
    for (_, entry) in nested(block)?
        .into_iter()
        .filter(|(field, _)| *field == 335)
    {
        let pair = uint_fields(&entry, &[1, 2])?;
        if let Some(&id @ (6 | 13)) = pair.get(&1) {
            let status = pair.get(&2).copied().unwrap_or(0);
            ensure!(
                status <= 1 && claims.insert(id, status).is_none(),
                "Invalid Echo key claim"
            );
        }
    }
    Ok((!claims.is_empty()).then(|| claims.values().sum()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;

    fn response(field: u32, arcana: Option<u32>) -> Vec<u8> {
        let mut season = Vec::new();
        let mut output = CodedOutputStream::vec(&mut season);
        output.write_uint32(1, 31).unwrap();
        output.write_uint32(5, 10).unwrap();
        output.write_uint32(15, 1796093999).unwrap();
        output.write_uint32(963, 1793502000).unwrap();
        if let Some(count) = arcana {
            output.write_uint32(3, count).unwrap();
        }
        output.flush().unwrap();
        drop(output);

        let mut message = Vec::new();
        let mut output = CodedOutputStream::vec(&mut message);
        output.write_bytes(field, &season).unwrap();
        output.flush().unwrap();
        drop(output);
        message
    }

    #[test]
    fn arcana_are_optional_and_do_not_affect_act_progress() {
        for (command, field) in [(LOGIN_RESPONSE, 8), (SCREEN_RESPONSE, 11)] {
            for count in [None, Some(0), Some(1), Some(2)] {
                let result = snapshot(command, &response(field, count)).unwrap();
                assert_eq!(result["acts"], 10);
                assert_eq!(result["arcana"], json!(count.filter(|&count| count > 0)));
            }
            assert!(snapshot(command, &response(field, Some(3))).is_err());
        }
        assert!(snapshot(SCREEN_RESPONSE, &response(8, Some(2))).is_err());
    }
}
