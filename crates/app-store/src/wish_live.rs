//! Captured rewards have local identities. Only the history API owns wish IDs.
use crate::Store;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sqlx::{Row, Sqlite, Transaction};
pub fn group(s: &str) -> &str {
    if matches!(s, "301" | "400") {
        "characters"
    } else {
        s
    }
}
fn same_group(a: &str, b: &str) -> bool {
    group(a) == group(b)
}
pub fn validate(v: &Value) -> Result<()> {
    let fields = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Missing wish payload"))?;
    ensure!(
        fields
            .keys()
            .all(|k| matches!(k.as_str(), "gacha_type" | "gacha_id" | "results")),
        "Unexpected wish field"
    );
    ensure!(
        matches!(
            v["gacha_type"].as_str(),
            Some("100" | "200" | "301" | "400" | "302" | "500")
        ),
        "Unsupported wish type"
    );
    crate::legacy::decimal(
        v["gacha_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing banner"))?,
    )?;
    let results = v["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing wish results"))?;
    ensure!(matches!(results.len(), 1 | 10), "Invalid wish batch");
    for r in results {
        ensure!(
            r.as_object().is_some_and(|r| r.len() == 2)
                && r["count"] == 1
                && r["item_id"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= u32::MAX as u64),
            "Invalid wish reward"
        );
    }
    Ok(())
}
pub async fn observe(
    tx: &mut Transaction<'_, Sqlite>,
    profile: &str,
    capture: &str,
    epoch: &str,
    seq: i64,
    ms: i64,
    v: &Value,
) -> Result<()> {
    validate(v)?;
    let enabled:i64=sqlx::query_scalar("SELECT count(*) FROM wish_sync_state s JOIN live_leases l USING(profile_id) WHERE s.profile_id=? AND s.capture_id=? AND s.epoch=? AND s.status='synced' AND s.fetched_ms<=? AND l.capture_id=s.capture_id AND l.epoch=s.epoch AND l.expires_ms>? AND EXISTS(SELECT 1 FROM events e WHERE e.capture_id=s.capture_id AND e.source_sequence=1 AND json_extract(e.event_json,'$.payload.configuration.wish_tracking')=1) AND NOT EXISTS(SELECT 1 FROM domain_state d WHERE d.profile_id=s.profile_id AND d.freshness IN ('stale','error')) AND NOT EXISTS(SELECT 1 FROM wish_live w WHERE w.profile_id=s.profile_id AND (w.eligible=0 OR w.capture_id<>s.capture_id OR w.epoch<>s.epoch) AND w.api_wish_id IS NULL)")
        .bind(profile).bind(capture).bind(epoch).bind(ms).bind(crate::ingest::now_ms()).fetch_one(&mut **tx).await?;
    // Retries are deduplicated by the native capture/sequence/result key.
    let eligible = enabled > 0 && ms >= crate::ingest::now_ms() - 8000;
    let history:Vec<String>=sqlx::query_scalar("SELECT record_json FROM wish_records WHERE profile_id=? ORDER BY length(wish_id) DESC,wish_id DESC").bind(profile).fetch_all(&mut **tx).await?;
    let kind = v["gacha_type"].as_str().unwrap();
    let mut anchor = "0".to_owned();
    for raw in history {
        let r: Value = serde_json::from_str(&raw)?;
        if same_group(kind, r["gacha_type"].as_str().unwrap_or("")) {
            anchor = r["id"].as_str().unwrap().into();
            break;
        }
    }
    for (i, r) in v["results"].as_array().unwrap().iter().enumerate() {
        let id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("{capture}:{seq}:{i}").as_bytes(),
        )
        .to_string();
        sqlx::query("INSERT INTO wish_live VALUES(?,?,?,?,?,?,?,?,?,?,?,?,NULL)")
            .bind(profile)
            .bind(id)
            .bind(capture)
            .bind(epoch)
            .bind(seq)
            .bind(i as i64)
            .bind(ms)
            .bind(kind)
            .bind(v["gacha_id"].as_str())
            .bind(r["item_id"].as_u64().unwrap() as i64)
            .bind(&anchor)
            .bind(i64::from(eligible))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
fn numeric_after(a: &str, b: &str) -> bool {
    (a.len(), a) > (b.len(), b)
}
fn reward_matches(api: &Value, item: i64, catalog: Option<&Value>) -> bool {
    if let Some(id) = api["item_id"].as_str().filter(|s| !s.is_empty()) {
        return id.parse::<i64>().ok() == Some(item);
    }
    if !api["lang"].as_str().is_some_and(|s| s.starts_with("ru")) {
        return false;
    }
    let Some(c) = catalog else { return false };
    let id = item.to_string();
    let names = if item >= 10000000 {
        &c["avatarNames"]
    } else {
        &c["names"]
    };
    let Some(name) = names[&id].as_str() else {
        return false;
    };
    if api["name"] != name {
        return false;
    }
    names
        .as_object()
        .is_some_and(|m| m.values().filter(|v| v.as_str() == Some(name)).count() == 1)
}
fn time_matches(api: &Value, observed: i64, region: &str) -> bool {
    let zone = match region {
        "os_euro" => 1,
        "os_usa" => -5,
        "os_asia" | "os_cht" => 8,
        _ => return false,
    };
    let Ok(format) = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day] [hour]:[minute]:[second]",
    ) else {
        return false;
    };
    let Some(s) = api["time"].as_str() else {
        return false;
    };
    let Ok(date) = time::PrimitiveDateTime::parse(s, &format) else {
        return false;
    };
    let ms = (date.assume_utc().unix_timestamp_nanos() / 1_000_000) as i64 - zone * 3600000;
    (observed - ms).abs() <= 3000
}
pub async fn reconcile(
    tx: &mut Transaction<'_, Sqlite>,
    profile: &str,
    imported: &std::collections::BTreeMap<String, Value>,
    catalog: Option<&Value>,
) -> Result<()> {
    let region: String =
        sqlx::query_scalar("SELECT server_region FROM profiles WHERE profile_id=?")
            .bind(profile)
            .fetch_one(&mut **tx)
            .await?;
    let rows=sqlx::query("SELECT * FROM wish_live WHERE profile_id=? AND api_wish_id IS NULL ORDER BY observed_ms,source_sequence,result_index").bind(profile).fetch_all(&mut **tx).await?;
    for r in rows {
        let kind: String = r.get("gacha_type");
        let anchor: String = r.get("anchor_id");
        let item: i64 = r.get("item_id");
        let observed: i64 = r.get("observed_ms");
        let used: Vec<String> = sqlx::query_scalar(
            "SELECT api_wish_id FROM wish_live WHERE profile_id=? AND api_wish_id IS NOT NULL",
        )
        .bind(profile)
        .fetch_all(&mut **tx)
        .await?;
        let mut newer: Vec<_> = imported
            .values()
            .filter(|api| {
                same_group(&kind, api["gacha_type"].as_str().unwrap_or(""))
                    && numeric_after(api["id"].as_str().unwrap(), &anchor)
                    && !used.iter().any(|id| api["id"] == *id)
            })
            .collect();
        newer.sort_by(|a, b| {
            let a = a["id"].as_str().unwrap();
            let b = b["id"].as_str().unwrap();
            (a.len(), a).cmp(&(b.len(), b))
        });
        // Proven continuity supplies order, including ten identical rewards in one second.
        // A delayed API prefix consumes only that prefix, leaving later captures pending.
        let candidates: Vec<_> = if r.get::<i64, _>("eligible") == 1 {
            newer
                .first()
                .copied()
                .into_iter()
                .filter(|api| {
                    reward_matches(api, item, catalog) && time_matches(api, observed, &region)
                })
                .collect()
        } else {
            newer
                .iter()
                .copied()
                .filter(|api| {
                    reward_matches(api, item, catalog) && time_matches(api, observed, &region)
                })
                .collect()
        };
        if candidates.len() == 1 {
            sqlx::query("UPDATE wish_live SET api_wish_id=? WHERE live_id=?")
                .bind(candidates[0]["id"].as_str().unwrap())
                .bind(r.get::<String, _>("live_id"))
                .execute(&mut **tx)
                .await?;
        } else if !newer.is_empty() {
            sqlx::query(
                "UPDATE wish_live SET eligible=0 WHERE profile_id=? AND api_wish_id IS NULL",
            )
            .bind(profile)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}
impl Store {
    pub async fn wish_live_enabled(&mut self, profile: &str) -> Result<bool> {
        let Some(c) = self.wish_context().await? else {
            return Ok(false);
        };
        if c.profile_id != profile {
            return Ok(false);
        }
        let sync = self.wish_sync_state(profile).await?;
        if sync["status"] != "synced" {
            return Ok(false);
        }
        let supported:i64=sqlx::query_scalar("SELECT count(*) FROM events WHERE capture_id=? AND source_sequence=1 AND json_extract(event_json,'$.payload.configuration.wish_tracking')=1").bind(c.capture_id).fetch_one(&mut self.connection).await?;
        Ok(supported == 1)
    }
    pub async fn wish_live_rows(&mut self, profile: &str) -> Result<Vec<Value>> {
        let enabled = self.wish_live_enabled(profile).await?;
        if !enabled {
            return Ok(vec![]);
        }
        let c = self.wish_context().await?.unwrap();
        let rows=sqlx::query("SELECT * FROM wish_live WHERE profile_id=? AND capture_id=? AND epoch=? AND eligible=1 AND api_wish_id IS NULL ORDER BY observed_ms DESC,source_sequence DESC,result_index DESC").bind(profile).bind(c.capture_id).bind(c.epoch).fetch_all(&mut self.connection).await?;
        Ok(rows.iter().map(|r|json!({"live_id":r.get::<String,_>("live_id"),"item_id":r.get::<i64,_>("item_id").to_string(),"gacha_type":r.get::<String,_>("gacha_type"),"gacha_id":r.get::<String,_>("gacha_id"),"observed_ms":r.get::<i64,_>("observed_ms").to_string(),"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"result_index":r.get::<i64,_>("result_index"),"anchor_id":r.get::<String,_>("anchor_id")})).collect())
    }
}
