use crate::profile_protocol::uint_fields;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

// Current PC 7.2 TowerAllDataRsp. Field 11 describes the previous schedule;
// the current repeating floors are identified by their opening times in field 2.
pub const COMMAND: u16 = 5207;

pub fn snapshot(body: &[u8]) -> Result<Value> {
    use crate::profile_protocol::nested;
    ensure!(body.len() <= 32768, "Oversized Abyss response");
    let header = uint_fields(body, &[5, 6, 9, 257])?;
    ensure!(
        header.get(&9).copied().unwrap_or(0) == 0,
        "Abyss request failed"
    );
    let schedule = *header
        .get(&6)
        .filter(|&&id| id > 0)
        .context("Missing Abyss cycle")?;
    let starts = *header.get(&257).context("Missing Abyss start")?;
    let ends = *header.get(&5).context("Missing Abyss end")?;
    ensure!(
        starts >= 1_600_000_000 && ends > starts && ends - starts <= 40 * 86400,
        "Invalid Abyss cycle dates"
    );

    let nested_fields = nested(body)?;
    let mut seasonal = BTreeSet::new();
    for (_, entry) in nested_fields.iter().filter(|(field, _)| *field == 2) {
        let pair = uint_fields(entry, &[1, 2])?;
        if pair
            .get(&2)
            .is_some_and(|&opens| (starts..=ends).contains(&opens))
        {
            let id = *pair
                .get(&1)
                .filter(|&&id| id > 0)
                .context("Missing Abyss floor")?;
            ensure!(seasonal.insert(id), "Duplicate seasonal floor");
        }
    }
    ensure!(seasonal.len() == 4, "Unexpected set of Abyss floors");
    let mut records = BTreeMap::new();
    for (_, record) in nested_fields.iter().filter(|(field, _)| *field == 14) {
        let id = *uint_fields(record, &[2])?
            .get(&2)
            .context("Missing Abyss record floor")?;
        if !seasonal.contains(&id) {
            continue;
        }
        let mut chambers = BTreeMap::new();
        for (_, entry) in nested(record)?
            .into_iter()
            .filter(|(field, _)| *field == 11)
        {
            let pair = uint_fields(&entry, &[1, 2])?;
            let chamber = *pair
                .get(&1)
                .filter(|&&id| id > 0)
                .context("Missing Abyss chamber")?;
            let stars = pair.get(&2).copied().unwrap_or(0);
            ensure!(
                stars <= 3 && chambers.insert(chamber, stars).is_none(),
                "Invalid Abyss chamber result"
            );
        }
        ensure!(
            chambers.len() <= 3 && records.insert(id, chambers).is_none(),
            "Invalid Abyss floor record"
        );
    }
    let mut stars = 0;
    let mut reached = None;
    for (index, id) in seasonal.into_iter().enumerate() {
        if let Some(chambers) = records.get(&id).filter(|chambers| !chambers.is_empty()) {
            stars += chambers.values().sum::<u32>();
            reached = Some((9 + index, chambers.len()));
        }
    }
    Ok(
        json!({"schedule_id":schedule,"starts_unix_s":starts,"ends_unix_s":ends,"stars":stars,
        "floor":reached.map(|(floor,_)|floor),"chamber":reached.map(|(_,chamber)|chamber)}),
    )
}
