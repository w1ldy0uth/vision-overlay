//! Imported API history is immutable account data, independent of capture snapshots.
use crate::Store;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sqlx::{Connection, Row};
use std::collections::BTreeMap;
use time::{OffsetDateTime, PrimitiveDateTime, format_description::well_known::Rfc3339};

fn text<'a>(v: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    let s = v[key].as_str().context("Missing history field")?;
    ensure!(s.len() <= max, "History field too long");
    Ok(s)
}

fn validate(v: &Value, uid: &str) -> Result<Value> {
    let id = text(v, "id", 32)?;
    ensure!(
        !id.is_empty() && !id.starts_with('0') && id.bytes().all(|c| c.is_ascii_digit()),
        "Invalid wish ID"
    );
    ensure!(
        v["uid"] == uid && v["count"] == "1",
        "History identity mismatch"
    );
    ensure!(
        matches!(
            v["gacha_type"].as_str(),
            Some("100" | "200" | "301" | "400" | "302" | "500")
        ),
        "Unsupported banner"
    );
    ensure!(
        matches!(v["rank_type"].as_str(), Some("3" | "4" | "5")),
        "Invalid rarity"
    );
    let timestamp = text(v, "time", 19)?;
    let format = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day] [hour]:[minute]:[second]",
    )?;
    PrimitiveDateTime::parse(timestamp, &format).context("Invalid wish date")?;
    text(v, "name", 512)?;
    for key in ["item_id", "item_type", "lang"] {
        ensure!(
            v[key].is_null() || v[key].as_str().is_some_and(|s| s.len() <= 256),
            "Invalid item metadata"
        );
    }
    // Explicit allowlist: imported files must never persist an authkey or URL.
    let mut row = serde_json::Map::new();
    for key in [
        "id",
        "uid",
        "gacha_type",
        "item_id",
        "count",
        "time",
        "name",
        "item_type",
        "rank_type",
        "lang",
    ] {
        row.insert(key.into(), v[key].clone());
    }
    Ok(Value::Object(row))
}
impl Store {
    pub async fn import_wishes(&mut self, profile: &str, archive: Value) -> Result<Value> {
        self.import_wishes_with_catalog(profile, archive, None)
            .await
    }
    pub async fn import_wishes_with_catalog(
        &mut self,
        profile: &str,
        archive: Value,
        catalog: Option<&Value>,
    ) -> Result<Value> {
        ensure!(
            archive["schema_version"] == 1
                && archive["source"] == "hoyoverse_wish_api"
                && archive["game_id"] == "genshin"
                && archive["account_namespace"] == "hk4e_global",
            "Unsupported wish archive"
        );
        let fetched = OffsetDateTime::parse(text(&archive, "fetched_at", 64)?, &Rfc3339)?
            .unix_timestamp_nanos()
            / 1_000_000;
        let fetched = i64::try_from(fetched)?;
        let now = crate::ingest::now_ms();
        ensure!(
            fetched > 0 && fetched <= now + 60_000,
            "Invalid import time"
        );
        let uid = text(&archive, "uid", 20)?;
        let region = text(&archive, "server_region", 32)?;
        let rows = archive["records"]
            .as_array()
            .context("Missing wish records")?;
        ensure!(rows.len() <= 100_000, "Too many wish records");
        let mut records = BTreeMap::new();
        for raw in rows {
            let v = validate(raw, uid)?;
            ensure!(
                records
                    .insert(v["id"].as_str().unwrap().to_owned(), v)
                    .is_none(),
                "Duplicate wish ID in archive"
            );
        }
        let streams = archive["streams"]
            .as_array()
            .context("Missing imported streams")?;
        let mut completed = std::collections::BTreeSet::new();
        for stream in streams {
            let banner = text(stream, "gacha_type", 3)?;
            ensure!(
                matches!(banner, "100" | "200" | "301" | "302" | "500") && completed.insert(banner),
                "Invalid imported stream"
            );
            ensure!(
                stream["pages"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 10000),
                "Invalid page count"
            );
        }
        ensure!(
            records
                .values()
                .all(|r| completed.contains(if r["gacha_type"] == "400" {
                    "301"
                } else {
                    r["gacha_type"].as_str().unwrap()
                })),
            "Record outside imported streams"
        );
        let complete =
            completed.len() == 5 && archive["coverage"]["all_available_pages_fetched"] == true;
        let mut ranges = serde_json::Map::new();
        for row in records.values() {
            let group = match row["gacha_type"].as_str().unwrap() {
                "301" | "400" => "characters",
                "302" => "weapons",
                "200" => "standard",
                "500" => "chronicled",
                _ => "beginners",
            };
            let id = row["id"].as_str().unwrap();
            let entry = ranges
                .entry(group.to_string())
                .or_insert_with(|| json!({"first":id,"last":id}));
            let before = entry["first"].as_str().unwrap().to_owned();
            let after = entry["last"].as_str().unwrap().to_owned();
            if (id.len(), id) < (before.len(), before.as_str()) {
                entry["first"] = json!(id);
            }
            if (id.len(), id) > (after.len(), after.as_str()) {
                entry["last"] = json!(id);
            }
        }
        let metadata = json!({"uid":uid,"server_region":region,"all_available_pages_fetched":complete,
            "ranges":ranges,"region_time_zone":archive["region_time_zone"].as_i64().filter(|n| (-12..=14).contains(n))});
        let hash = crate::digest(&serde_json::to_vec(
            &json!({"fetched_ms":fetched,"metadata":metadata,"records":records}),
        )?);
        let mut tx = self.connection.begin().await?;
        let identity = sqlx::query("SELECT uid,server_region FROM profiles WHERE profile_id=?")
            .bind(profile)
            .fetch_optional(&mut *tx)
            .await?
            .context("Profile not found")?;
        ensure!(
            identity.get::<String, _>("uid") == uid
                && identity.get::<String, _>("server_region") == region,
            "History belongs to another profile"
        );
        let mut added = 0;
        for (id, row) in &records {
            if let Some(old) = sqlx::query_scalar::<_, String>(
                "SELECT record_json FROM wish_records WHERE profile_id=? AND wish_id=?",
            )
            .bind(profile)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            {
                let old: Value = serde_json::from_str(&old)?;
                // Localized labels may change; stable gameplay facts must agree.
                for key in ["id", "uid", "gacha_type", "count", "time", "rank_type"] {
                    ensure!(
                        old[key] == row[key],
                        "Conflicting wish ID; import cancelled"
                    );
                }
                if !old["item_id"].is_null() && !row["item_id"].is_null() {
                    ensure!(
                        old["item_id"] == row["item_id"],
                        "Conflicting wish item; import cancelled"
                    );
                }
            } else {
                sqlx::query("INSERT INTO wish_records VALUES(?,?,?)")
                    .bind(profile)
                    .bind(id)
                    .bind(serde_json::to_string(row)?)
                    .execute(&mut *tx)
                    .await?;
                added += 1;
            }
        }
        let inserted = sqlx::query("INSERT OR IGNORE INTO wish_imports VALUES(?,?,?,?,?,?,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(profile)
            .bind(&hash)
            .bind(fetched)
            .bind(now)
            .bind(serde_json::to_string(&metadata)?)
            .bind(records.len() as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if inserted > 0 {
            let capture = uuid::Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
                .bind(&capture).bind(format!("wishes:{profile}:{hash}"))
                .bind("{}").bind("{\"mode\":\"system\"}").bind("wish-import-v1").execute(&mut *tx).await?;
            sqlx::query("INSERT INTO epochs VALUES(?,'0',?,'wish_import')")
                .bind(&capture)
                .bind(profile)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE store_meta SET revision=revision+1 WHERE singleton=1")
                .execute(&mut *tx)
                .await?;
            let revision: i64 =
                sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                    .fetch_one(&mut *tx)
                    .await?;
            let event = json!({"kind":"wish_history.imported","payload":{"added":added,"received":records.len()}});
            sqlx::query(
                "INSERT INTO events VALUES(?,1,'0',?,'wish_history.imported',?,?,'applied',?)",
            )
            .bind(&capture)
            .bind(profile)
            .bind(now)
            .bind(event.to_string())
            .bind(revision)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO change_log VALUES(?,?,1,?,?)")
                .bind(revision)
                .bind(&capture)
                .bind(profile)
                .bind("[\"wish_history\"]")
                .execute(&mut *tx)
                .await?;
        }
        crate::wish_live::reconcile(&mut tx, profile, &records, catalog).await?;
        tx.commit().await?;
        Ok(json!({"added":added,"received":records.len(),"all_available_pages_fetched":complete}))
    }

    pub async fn wishes(&mut self, profile: &str) -> Result<Value> {
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE profile_id=?")
            .bind(profile)
            .fetch_one(&mut self.connection)
            .await?;
        ensure!(exists == 1, "Profile not found");
        let rows: Vec<String> = sqlx::query_scalar("SELECT record_json FROM wish_records WHERE profile_id=? ORDER BY length(wish_id) DESC,wish_id DESC").bind(profile).fetch_all(&mut self.connection).await?;
        let records: Vec<Value> = rows
            .iter()
            .map(|s| serde_json::from_str(s))
            .collect::<std::result::Result<_, _>>()?;
        let latest = sqlx::query("SELECT fetched_ms,imported_ms,metadata_json FROM wish_imports WHERE profile_id=? ORDER BY fetched_ms DESC,imported_ms DESC LIMIT 1").bind(profile).fetch_optional(&mut self.connection).await?;
        let import = latest.map(|r| -> Result<Value> {Ok(json!({"fetched_ms":r.get::<i64,_>("fetched_ms").to_string(),"imported_ms":r.get::<i64,_>("imported_ms").to_string(),"metadata":serde_json::from_str::<Value>(r.get("metadata_json"))?}))}).transpose()?;
        let live = self.wish_live_rows(profile).await?;
        let hidden:i64=sqlx::query_scalar("SELECT count(*) FROM wish_live w WHERE w.profile_id=? AND w.api_wish_id IS NULL AND (w.eligible=0 OR NOT EXISTS(SELECT 1 FROM wish_sync_state s WHERE s.profile_id=w.profile_id AND s.capture_id=w.capture_id AND s.epoch=w.epoch))").bind(profile).fetch_one(&mut self.connection).await?;
        let sync = self.wish_sync_state(profile).await?;
        let status = sync["status"].as_str().unwrap_or(if import.is_some() {
            "needs_sync"
        } else {
            "not_imported"
        });
        Ok(
            json!({"records":records,"last_import":import,"sync":sync,"status":status,"live_enabled":self.wish_live_enabled(profile).await?,"live_records":live,"hidden_live_count":hidden}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive() -> Value {
        json!({"schema_version":1,"source":"hoyoverse_wish_api","game_id":"genshin","account_namespace":"hk4e_global","uid":"700000001","server_region":"os_euro","fetched_at":"2026-11-12T15:50:00Z","coverage":{"all_available_pages_fetched":true},"streams":[{"gacha_type":"301","pages":2}],"records":[{"id":"184467440737095516161234","uid":"700000001","gacha_type":"301","item_id":null,"count":"1","time":"2026-11-12 16:00:00","name":"Меч","item_type":"Оружие","rank_type":"3","lang":"ru-ru"}]})
    }
    async fn store() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(&temp.path().join("test.sqlite3"))
            .await
            .unwrap();
        for (id, uid, region) in [
            ("one", "700000001", "os_euro"),
            ("two", "800000001", "os_asia"),
        ] {
            sqlx::query("INSERT INTO profiles VALUES(?,'genshin','hk4e_global',?,?)")
                .bind(id)
                .bind(region)
                .bind(uid)
                .execute(&mut s.connection)
                .await
                .unwrap();
        }
        (temp, s)
    }
    #[tokio::test]
    async fn dedup_isolation_conflict_and_rollback() {
        let (_temp, mut s) = store().await;
        assert_eq!(s.import_wishes("one", archive()).await.unwrap()["added"], 1);
        assert_eq!(s.import_wishes("one", archive()).await.unwrap()["added"], 0);
        assert!(s.import_wishes("two", archive()).await.is_err());
        assert_eq!(s.wishes("two").await.unwrap()["records"], json!([]));
        let mut v = archive();
        v["records"][0]["rank_type"] = json!("5");
        let mut added = v["records"][0].clone();
        added["id"] = json!("1");
        v["records"].as_array_mut().unwrap().push(added);
        assert!(s.import_wishes("one", v).await.is_err());
        let stored = s.wishes("one").await.unwrap();
        assert_eq!(stored["records"].as_array().unwrap().len(), 1);
        assert_eq!(stored["records"][0]["rank_type"], "3");
        assert_eq!(
            stored["last_import"]["metadata"]["ranges"]["characters"]["first"],
            "184467440737095516161234"
        );
        assert_eq!(stored["status"], "needs_sync");
        assert_eq!(stored["live_enabled"], false);
    }
    #[tokio::test]
    async fn repeated_import_preserves_older_records_and_strips_secrets() {
        let (_temp, mut s) = store().await;
        let mut v = archive();
        v["records"][0]["authkey"] = json!("secret");
        s.import_wishes("one", v).await.unwrap();
        let mut v = archive();
        v["records"][0]["id"] = json!("9999999999999999999999999");
        s.import_wishes("one", v).await.unwrap();
        let data = s.wishes("one").await.unwrap();
        assert_eq!(data["records"].as_array().unwrap().len(), 2);
        assert_eq!(data["records"][0]["id"], "9999999999999999999999999");
        assert!(!data.to_string().contains("secret"));
        let mut v = archive();
        v["records"][0]["time"] = json!("2026-02-30 12:00:00");
        assert!(s.import_wishes("one", v).await.is_err());
    }
    #[tokio::test]
    async fn portable_backup_preserves_wishes() {
        let (temp, mut s) = store().await;
        s.import_wishes("one", archive()).await.unwrap();
        let path = temp.path().join("history.visionbackup");
        s.backup(
            &path,
            crate::backup::Settings::default(),
            json!({"schema_version":1}),
        )
        .await
        .unwrap();
        let validated = crate::backup::validate(&path).await.unwrap();
        assert_eq!(validated.manifest.database_schema_version, 8);
        let mut restored = Store::open(&temp.path().join("restored.sqlite3"))
            .await
            .unwrap();
        restored
            .restore(&validated, &temp.path().join("rollback.visionbackup"), &[])
            .await
            .unwrap();
        assert_eq!(
            restored.wishes("one").await.unwrap()["records"],
            s.wishes("one").await.unwrap()["records"]
        );
    }
}
