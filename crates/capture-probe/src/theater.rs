use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::profile_protocol::{nested, uint_fields};

pub const COMMAND: u16 = 163;

// PC 7.2 login response. Use the current season block, excluding dated history lists.
pub fn snapshot(body: &[u8]) -> Result<Value> {
    ensure!(body.len() <= 32768, "Oversized Theater response");
    let mut blocks = nested(body)?.into_iter().filter(|(field, _)| *field == 8);
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
    let arcana = fields.get(&3).copied().unwrap_or(0);
    ensure!(
        starts >= 1_600_000_000
            && ends > starts
            && ends - starts <= 40 * 86400
            && acts <= 10
            && arcana <= 2,
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
