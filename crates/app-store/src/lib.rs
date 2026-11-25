//! Persistent offline import. No packet capture, network service or live lease here.
pub mod backup;
pub mod core;
pub mod ingest;
pub mod legacy;
pub mod planner;
pub mod reader;
pub mod retention;
pub mod wish_live;
pub mod wish_sync;
pub mod wishes;
use anyhow::{Context, Result, ensure};
use legacy::{Action, Bindings};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, Row, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use uuid::Uuid;

pub(crate) const DATABASE_SCHEMA_VERSION: i64 = 9;
const ADAPTER: &str = "legacy-jsonl-import-v1";
pub struct Store {
    connection: SqliteConnection,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options).await?;
        let exists: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_one(&mut connection)
        .await?;
        if exists > 0 {
            let version: Option<i64> =
                sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
                    .fetch_one(&mut connection)
                    .await?;
            ensure!(
                version.unwrap_or(0) <= DATABASE_SCHEMA_VERSION,
                "Database schema is newer than this application"
            );
        }
        sqlx::migrate!("./migrations").run(&mut connection).await?;
        Ok(Self { connection })
    }
    pub async fn close(self) -> Result<()> {
        self.connection.close().await?;
        Ok(())
    }

    /// A whole finalized archive is one transaction. Failures publish no partial import.
    pub async fn import(&mut self, bytes: &[u8], bindings: &Bindings) -> Result<Value> {
        let archive = legacy::parse(bytes)?;
        for (epoch, identity) in bindings {
            legacy::decimal(epoch)?;
            identity.validate()?;
            if let Some(uid) = archive.login_uids.get(epoch) {
                ensure!(
                    uid == &identity.uid,
                    "Explicit profile differs from captured login UID"
                );
            }
        }
        ensure!(
            archive.data_epochs == bindings.keys().cloned().collect(),
            "Provide exactly one explicit binding for every epoch containing game data (and no extra bindings)"
        );
        let hash = digest(bytes);
        let binding_json = serde_json::to_string(bindings)?;
        let mut tx = self.connection.begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT capture_id,binding_json,adapter_revision FROM captures WHERE artifact_sha256=?",
        )
        .bind(&hash)
        .fetch_optional(&mut *tx)
        .await?
        {
            ensure!(
                row.get::<String, _>("binding_json") == binding_json,
                "This exact archive is already bound to different profiles"
            );
            ensure!(
                row.get::<String, _>("adapter_revision") == ADAPTER,
                "Archive belongs to another adapter revision; explicit rebuild is required"
            );
            return Ok(
                json!({"duplicate":true,"capture_id":row.get::<String,_>("capture_id"),"artifact_sha256":hash}),
            );
        }
        let capture = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO captures(capture_id,artifact_sha256,binding_json,source_json,adapter_revision,event_count,terminal_status) VALUES(?,?,?,?,?,?,?)")
            .bind(&capture).bind(&hash).bind(&binding_json).bind(serde_json::to_string(&archive.source)?).bind(ADAPTER).bind(archive.records.len() as i64).bind(&archive.terminal).execute(&mut *tx).await?;
        let mut profiles = BTreeMap::new();
        for (epoch, identity) in bindings {
            let id = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO profiles(profile_id,game_id,account_namespace,server_region,uid) VALUES(?,'genshin','hk4e_global',?,?) ON CONFLICT(game_id,account_namespace,server_region,uid) DO NOTHING")
                .bind(&id).bind(&identity.server_region).bind(&identity.uid).execute(&mut *tx).await?;
            let id:String=sqlx::query_scalar("SELECT profile_id FROM profiles WHERE game_id='genshin' AND account_namespace='hk4e_global' AND server_region=? AND uid=?").bind(&identity.server_region).bind(&identity.uid).fetch_one(&mut *tx).await?;
            let lease: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM live_leases WHERE profile_id=? AND expires_ms>?",
            )
            .bind(&id)
            .bind(ingest::now_ms())
            .fetch_one(&mut *tx)
            .await?;
            ensure!(
                lease == 0,
                "Profile currently has a live collector; stop capture before importing into it"
            );
            profiles.insert(epoch.clone(), id);
        }
        for epoch in archive
            .records
            .iter()
            .map(|r| r.epoch.clone())
            .collect::<BTreeSet<_>>()
        {
            sqlx::query("INSERT INTO epochs VALUES(?,?,?,?)")
                .bind(&capture)
                .bind(&epoch)
                .bind(profiles.get(&epoch))
                .bind(if profiles.contains_key(&epoch) {
                    "user_import"
                } else {
                    "identity_pending"
                })
                .execute(&mut *tx)
                .await?;
        }
        let mut revision: i64 =
            sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        // Whether each epoch/collection may update the saved view. Older archives stay queryable.
        let mut apply = BTreeMap::<(String, String), bool>::new();
        let mut mutation_events = 0u64;
        for record in &archive.records {
            let profile = profiles.get(&record.epoch).map(String::as_str);
            if apply_record(
                &mut tx,
                &capture,
                record,
                profile,
                &mut revision,
                &mut apply,
                false,
            )
            .await?
            {
                mutation_events += 1;
            }
        }
        sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"duplicate":false,"capture_id":capture,"artifact_sha256":hash,"events":archive.records.len(),"mutation_events":mutation_events,"revision":revision.to_string(),"profiles":profiles,"terminal_status":archive.terminal}),
        )
    }
    pub async fn profiles(&mut self) -> Result<Value> {
        let rows=sqlx::query("SELECT profile_id,game_id,account_namespace,server_region,uid FROM profiles ORDER BY server_region,uid").fetch_all(&mut self.connection).await?;
        Ok(json!(rows.iter().map(|r|json!({"profile_id":r.get::<String,_>("profile_id"),"game_id":r.get::<String,_>("game_id"),"account_namespace":r.get::<String,_>("account_namespace"),"server_region":r.get::<String,_>("server_region"),"uid":r.get::<String,_>("uid")})).collect::<Vec<_>>()))
    }
    pub async fn state(&mut self, profile: &str) -> Result<Value> {
        read_profile_state(&mut self.connection, profile).await
    }
    pub async fn history(&mut self, profile: &str, after: i64, limit: i64) -> Result<Value> {
        ensure!(
            after >= 0 && (1..=1000).contains(&limit),
            "Invalid history cursor/limit"
        );
        let rows=sqlx::query("SELECT revision,capture_id,source_sequence,topics_json FROM change_log WHERE profile_id=? AND revision>? ORDER BY revision LIMIT ?").bind(profile).bind(after).bind(limit).fetch_all(&mut self.connection).await?;
        Ok(json!(rows.iter().map(|r|json!({"revision":r.get::<i64,_>("revision").to_string(),"capture_id":r.get::<String,_>("capture_id"),"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"topics":serde_json::from_str::<Value>(r.get("topics_json")).unwrap()})).collect::<Vec<_>>()))
    }
}

async fn read_profile_state(connection: &mut SqliteConnection, profile: &str) -> Result<Value> {
    let mut tx = connection.begin().await?;
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE profile_id=?")
        .bind(profile)
        .fetch_one(&mut *tx)
        .await?;
    ensure!(exists == 1, "Unknown profile");
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
        .fetch_one(&mut *tx)
        .await?;
    let rows=sqlx::query("SELECT collection,entity_key,value_json,confirmed_in_baseline,revision,value_format FROM current_entities WHERE profile_id=? ORDER BY collection,entity_key").bind(profile).fetch_all(&mut *tx).await?;
    let mut collections: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut totals = BTreeMap::<u32, u64>::new();
    for r in rows {
        let collection: String = r.get("collection");
        let mut v: Value = serde_json::from_str(r.get("value_json"))?;
        let confirmed = r.get::<i64, _>("confirmed_in_baseline") == 1;
        if collection == "theater" {
            retain_theater_arcana(&mut tx, profile, &mut v).await?;
        }
        if collection == "material_stacks" && confirmed {
            let id = v["item_id"].as_u64().context("Stored item ID missing")? as u32;
            let count = v["count"].as_u64().context("Stored count missing")?;
            let total = totals.entry(id).or_default();
            *total = total
                .checked_add(count)
                .context("Material aggregate overflow")?;
        }
        collections.entry(collection).or_default().push(json!({"entity_key":r.get::<String,_>("entity_key"),"value_format":r.get::<String,_>("value_format"),"values":v,"confirmation":if confirmed{"observed"}else{"historical_unconfirmed"},"revision":r.get::<i64,_>("revision").to_string()}));
    }
    let domains=sqlx::query("SELECT collection,scope,freshness,reason,observed_ms,revision FROM domain_state WHERE profile_id=? ORDER BY collection").bind(profile).fetch_all(&mut *tx).await?;
    let domains:Vec<_>=domains.iter().map(|r|json!({"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope"),"freshness":r.get::<String,_>("freshness"),"reason":r.get::<String,_>("reason"),"observed_unix_ms":r.get::<i64,_>("observed_ms").to_string(),"revision":r.get::<i64,_>("revision").to_string()})).collect();
    let active: Option<String> =
        sqlx::query_scalar("SELECT profile_id FROM active_context WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(
        json!({"profile_id":profile,"revision":revision.to_string(),"mode":if active.as_deref()==Some(profile){"live"}else{"saved_offline"},"domains":domains,"collections":collections,"material_totals":totals.into_iter().map(|(id,count)|json!({"item_id":id,"count":count.to_string()})).collect::<Vec<_>>()}),
    )
}

// The latest run may report fewer Arcana than an earlier run in the same season.
async fn retain_theater_arcana(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    profile: &str,
    value: &mut Value,
) -> Result<()> {
    let reported = value["arcana"].as_i64().unwrap_or(0);
    if reported >= 2 {
        return Ok(());
    }
    let best: Option<i64> = sqlx::query_scalar(include_str!("theater_best_arcana.sql"))
        .bind(profile)
        .bind(value["schedule_id"].as_i64())
        .bind(value["starts_unix_s"].as_i64())
        .bind(value["ends_unix_s"].as_i64())
        .fetch_one(&mut **tx)
        .await?;
    // Older decoders also reported zero when the field was missing.
    let best = best.unwrap_or(0).max(reported);
    value["arcana"] = json!((best > 0).then_some(best));
    Ok(())
}

#[cfg(test)]
mod ingest_tests;
#[cfg(test)]
mod tests;

#[allow(clippy::too_many_arguments)]
async fn write_entity(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    profile: &str,
    collection: &str,
    key: &str,
    value: Option<&Value>,
    capture: &str,
    record: &legacy::Record,
    revision: i64,
) -> Result<()> {
    if let Some(value) = value {
        core::validate_entity(collection, key, value)?;
        let value_json = if collection == "theater" {
            let mut value = value.clone();
            retain_theater_arcana(tx, profile, &mut value).await?;
            serde_json::to_string(&value)?
        } else {
            serde_json::to_string(value)?
        };
        sqlx::query("INSERT INTO current_entities(profile_id,collection,entity_key,value_json,confirmed_in_baseline,capture_id,epoch,source_sequence,observed_ms,revision) VALUES(?,?,?,?,1,?,?,?,?,?) ON CONFLICT(profile_id,collection,entity_key) DO UPDATE SET value_json=excluded.value_json,confirmed_in_baseline=1,capture_id=excluded.capture_id,epoch=excluded.epoch,source_sequence=excluded.source_sequence,observed_ms=excluded.observed_ms,revision=excluded.revision")
            .bind(profile).bind(collection).bind(key).bind(value_json).bind(capture).bind(&record.epoch).bind(record.sequence).bind(record.observed_ms).bind(revision).execute(&mut **tx).await?;
    } else {
        sqlx::query(
            "DELETE FROM current_entities WHERE profile_id=? AND collection=? AND entity_key=?",
        )
        .bind(profile)
        .bind(collection)
        .bind(key)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

// Shared transactional projector. Validation happens in the respective source adapter.
async fn apply_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture: &str,
    record: &core::Record,
    profile: Option<&str>,
    revision: &mut i64,
    apply: &mut BTreeMap<(String, String), bool>,
    live: bool,
) -> Result<bool> {
    // Public social data must belong to the login-bound profile in both import and live delivery.
    for action in &record.actions {
        let values: Vec<&Value> = match action {
            Action::Snapshot {
                collection, rows, ..
            } if collection == "player_profile" => rows.values().collect(),
            Action::Set {
                collection,
                value: Some(value),
                ..
            } if collection == "player_profile" => vec![value],
            _ => vec![],
        };
        if !values.is_empty() {
            let id = profile.context("Profile data requires login identity")?;
            let uid: String = sqlx::query_scalar("SELECT uid FROM profiles WHERE profile_id=?")
                .bind(id)
                .fetch_one(&mut **tx)
                .await?;
            for value in values {
                ensure!(
                    value["uid"].as_str() == Some(uid.as_str()),
                    "Social profile UID does not match login"
                );
            }
        }
    }
    sqlx::query("INSERT INTO events(capture_id,source_sequence,epoch,profile_id,kind,observed_ms,event_json,projection_result) VALUES(?,?,?,?,?,?,?,'archived')")
                .bind(capture).bind(record.sequence).bind(&record.epoch).bind(profile).bind(record.event["kind"].as_str().or_else(||record.event["event"].as_str()).unwrap()).bind(record.observed_ms).bind(serde_json::to_string(&record.event)?).execute(&mut **tx).await?;
    let mut topics = BTreeSet::new();
    let next_revision = revision.checked_add(1).context("Revision overflow")?;
    for action in &record.actions {
        if let Action::Notice { topics: extra } = action {
            topics.extend(extra.iter().cloned());
            continue;
        }
        let Some(profile) = profile else { continue };
        if let Action::Snapshot {
            collection,
            scope,
            rows,
        } = action
        {
            sqlx::query("INSERT INTO snapshots(snapshot_id,capture_id,source_sequence,profile_id,collection,scope,snapshot_json) VALUES(?,?,?,?,?,?,?)")
                        .bind(Uuid::new_v4().to_string())
                        .bind(capture)
                        .bind(record.sequence)
                        .bind(profile)
                        .bind(collection)
                        .bind(scope)
                        .bind(serde_json::to_string(rows)?)
                        .execute(&mut **tx)
                        .await?;
            let key = (record.epoch.clone(), collection.clone());
            let first = !apply.contains_key(&key);
            if first {
                let previous: Option<i64> = sqlx::query_scalar(
                    "SELECT observed_ms FROM domain_state WHERE profile_id=? AND collection=?",
                )
                .bind(profile)
                .bind(collection)
                .fetch_optional(&mut **tx)
                .await?;
                apply.insert(
                    key.clone(),
                    previous.is_none_or(|t| record.observed_ms.is_some_and(|n| n >= t)),
                );
            }
            if !apply[&key] {
                continue;
            }
            if scope == "full" && !(collection == "theater" && rows.contains_key("current")) {
                sqlx::query("DELETE FROM current_entities WHERE profile_id=? AND collection=?")
                    .bind(profile)
                    .bind(collection)
                    .execute(&mut **tx)
                    .await?;
            } else if scope != "full" && (first || scope == "unverified") {
                sqlx::query("UPDATE current_entities SET confirmed_in_baseline=0 WHERE profile_id=? AND collection=?").bind(profile).bind(collection).execute(&mut **tx).await?;
            }
            for (key, value) in rows {
                write_entity(
                    tx,
                    profile,
                    collection,
                    key,
                    Some(value),
                    capture,
                    record,
                    next_revision,
                )
                .await?;
            }
            sqlx::query("INSERT INTO domain_state VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(profile_id,collection) DO UPDATE SET capture_id=excluded.capture_id,epoch=excluded.epoch,scope=excluded.scope,freshness=excluded.freshness,reason=excluded.reason,observed_ms=excluded.observed_ms,revision=excluded.revision")
                        .bind(profile).bind(collection).bind(capture).bind(&record.epoch).bind(scope).bind(if live {"synchronized"} else {"saved"}).bind(if live {"live"} else {"offline_import"}).bind(record.observed_ms.unwrap()).bind(next_revision).execute(&mut **tx).await?;
            topics.insert(collection.clone());
        } else if let Action::Set {
            collection,
            key,
            value,
        } = action
        {
            if !apply
                .get(&(record.epoch.clone(), collection.clone()))
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            write_entity(
                tx,
                profile,
                collection,
                key,
                value.as_ref(),
                capture,
                record,
                next_revision,
            )
            .await?;
            sqlx::query("UPDATE domain_state SET observed_ms=?,revision=? WHERE profile_id=? AND collection=?").bind(record.observed_ms).bind(next_revision).bind(profile).bind(collection).execute(&mut **tx).await?;
            topics.insert(collection.clone());
        } else if let Action::Invalidate { reason, error } = action {
            let affected:Vec<String>=sqlx::query_scalar("SELECT collection FROM domain_state WHERE profile_id=? AND capture_id=? AND epoch=?").bind(profile).bind(capture).bind(&record.epoch).fetch_all(&mut **tx).await?;
            for c in affected {
                if record.event["kind"] == "state.invalidated"
                    && !record.event["payload"]["collections"]
                        .as_array()
                        .is_some_and(|list| list.iter().any(|x| x == &c))
                {
                    continue;
                }
                sqlx::query("UPDATE domain_state SET freshness=?,reason=?,revision=? WHERE profile_id=? AND collection=? AND freshness!='error'").bind(if *error{"error"}else{"stale"}).bind(reason).bind(next_revision).bind(profile).bind(&c).execute(&mut **tx).await?;
                topics.insert(c);
            }
        }
    }
    // A successful terminal stop freezes a valid baseline as saved; a disconnect just
    // before game exit is expected. Decoder/capture errors remain errors/stale.
    if record.event["event"] == "status"
        && record.event["data"]["state"]
            .as_str()
            .is_some_and(|s| s.starts_with("stopped"))
    {
        let n=sqlx::query("UPDATE domain_state SET freshness='saved',reason=?,revision=? WHERE capture_id=? AND freshness!='error' AND reason IN ('offline_import','stale')")
                    .bind(record.event["data"]["state"].as_str().unwrap()).bind(next_revision).bind(capture).execute(&mut **tx).await?.rows_affected();
        if n > 0 {
            topics.insert("freshness".into());
        }
    }
    if live {
        topics.insert("capture_health".into());
    }
    // Capture registration itself is observable, even when there were no game facts.
    if record.sequence == 1 {
        topics.insert("captures".into());
    }
    if !topics.is_empty() {
        *revision = next_revision;

        sqlx::query("INSERT INTO change_log VALUES(?,?,?,?,?)")
            .bind(*revision)
            .bind(capture)
            .bind(record.sequence)
            .bind(profile)
            .bind(serde_json::to_string(&topics)?)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE events SET projection_result='applied',revision=? WHERE capture_id=? AND source_sequence=?").bind(*revision).bind(capture).bind(record.sequence).execute(&mut **tx).await?;
    }

    let format = if record.event["contract_version"] == 1
        && record.event["payload"]["evidence"]["value_format"] != "legacy_capture_v1"
    {
        "domain_v1"
    } else {
        "legacy_capture_v1"
    };
    sqlx::query(
        "UPDATE current_entities SET value_format=? WHERE capture_id=? AND source_sequence=?",
    )
    .bind(format)
    .bind(capture)
    .bind(record.sequence)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE snapshots SET value_format=? WHERE capture_id=? AND source_sequence=?")
        .bind(format)
        .bind(capture)
        .bind(record.sequence)
        .execute(&mut **tx)
        .await?;
    Ok(!topics.is_empty())
}
