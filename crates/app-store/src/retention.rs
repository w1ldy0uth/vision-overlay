//! Materialized profile checkpoints. Old/incomplete epochs never evict good ones.
use crate::Store;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const TABLES: &[&str] = &[
    "recovery_points",
    "retention_epochs",
    "retention_artifacts",
    "automatic_backups",
    "automatic_backup_captures",
    "retention_jobs",
    "retention_meta",
];
const REQUIRED: &[&str] = &[
    "material_stacks",
    "weapons",
    "artifacts",
    "furniture",
    "unknown_items",
    "characters",
    "wallet",
    "adventure",
    "achievements",
    "teams",
    "player_profile",
];
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub capture: Option<String>,
    pub pinned: bool,
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_evidence_and_unapplied_updates_are_preserved_correctly() {
        let mut frame = Frame::default();
        frame.apply(&json!({"kind":"state.snapshot","payload":{"collections":[{"name":"material_stacks","scope":"unverified","entities":[{"entity_key":"42","values":{"guid":"42","item_id":100011,"count":3}}]}]}}),1,Some(1000),true).unwrap();
        let deleted = json!({"kind":"state.batch","payload":{"operations":[{"collection":"material_stacks","entity_key":"42","action":"delete","values":{}}]}});
        frame.apply(&deleted, 2, Some(1001), false).unwrap();
        assert_eq!(frame.collections["material_stacks"]["42"]["count"], 3);
        frame.apply(&deleted, 3, Some(1002), true).unwrap();
        assert!(!frame.collections["material_stacks"].contains_key("42"));
        assert_eq!(frame.deletions["material_stacks"]["42"]["item_id"], 100011);
        frame.apply(&json!({"kind":"state.batch","payload":{"operations":[{"collection":"material_stacks","entity_key":"42","action":"set","values":{"guid":"42","item_id":100011,"count":1}}]}}),4,Some(1003),true).unwrap();
        assert!(frame.deletions["material_stacks"].is_empty());
    }
}
#[derive(Default)]
struct Frame {
    collections: BTreeMap<String, BTreeMap<String, Value>>,
    coverage: BTreeMap<String, Value>,
    deletions: BTreeMap<String, BTreeMap<String, Value>>,
    bad: bool,
    last: i64,
    sequence: i64,
}
impl Frame {
    fn apply(
        &mut self,
        event: &Value,
        sequence: i64,
        time: Option<i64>,
        applied: bool,
    ) -> Result<()> {
        let kind = event["kind"].as_str().unwrap_or("");
        if kind == "state.invalidated" {
            let reason = event["payload"]["reason"].as_str().unwrap_or("");
            if ["loss", "error", "rejected", "decoder"]
                .iter()
                .any(|s| reason.contains(s))
            {
                self.bad = true;
            }
        }
        if !applied {
            return Ok(());
        }
        if kind == "state.snapshot" {
            for collection in event["payload"]["collections"]
                .as_array()
                .context("Missing snapshot collections")?
            {
                let name = collection["name"]
                    .as_str()
                    .context("Missing collection name")?
                    .to_owned();
                let partial = collection["scope"] == "partial";
                let rows = self.collections.entry(name.clone()).or_default();
                if !partial {
                    rows.clear();
                }
                if collection["scope"] == "full" {
                    self.deletions.remove(&name);
                }
                for row in collection["entities"]
                    .as_array()
                    .context("Missing snapshot rows")?
                {
                    if let Some(deleted) = self.deletions.get_mut(&name) {
                        deleted.remove(row["entity_key"].as_str().context("Missing row key")?);
                    }
                    rows.insert(
                        row["entity_key"]
                            .as_str()
                            .context("Missing row key")?
                            .into(),
                        row["values"].clone(),
                    );
                }
                self.coverage.insert(name,json!({"scope":collection["scope"],"verified_fields":collection["verified_fields"],"value_format":event["payload"]["evidence"]["value_format"]}));
            }
        } else if kind == "state.batch" {
            for op in event["payload"]["operations"]
                .as_array()
                .context("Missing operations")?
            {
                let name = op["collection"].as_str().context("Missing collection")?;
                let rows = self
                    .collections
                    .get_mut(name)
                    .context("Patch without baseline")?;
                let key = op["entity_key"].as_str().context("Missing key")?;
                if op["action"] == "delete" {
                    let previous = rows.remove(key).unwrap_or(Value::Null);
                    self.deletions
                        .entry(name.into())
                        .or_default()
                        .insert(key.into(), previous);
                } else {
                    if let Some(deleted) = self.deletions.get_mut(name) {
                        deleted.remove(key);
                    }
                    let value = rows.entry(key.into()).or_insert_with(|| json!({}));
                    for (field, next) in op["values"].as_object().context("Invalid patch")? {
                        value[field] = next.clone();
                    }
                }
            }
        } else {
            return Ok(());
        }
        self.last = time.context("Missing observation time")?;
        self.sequence = sequence;
        Ok(())
    }
    fn complete(&self) -> bool {
        !self.bad
            && REQUIRED
                .iter()
                .all(|name| self.collections.contains_key(*name))
            && self.collections["wallet"].contains_key("201")
            && self.collections["wallet"].contains_key("202")
    }
}
impl Store {
    /// Internal maintenance queries must hold the same writer mutex as ingestion.
    pub fn maintenance_db(&mut self) -> &mut sqlx::SqliteConnection {
        &mut self.connection
    }

    pub async fn materialize_recovery_points(&mut self) -> Result<usize> {
        let epochs:Vec<(String,String,String)>=sqlx::query_as("SELECT e.capture_id,e.epoch,e.profile_id FROM epochs e JOIN captures c USING(capture_id) LEFT JOIN retention_epochs r ON r.capture_id=e.capture_id AND r.epoch=e.epoch WHERE e.profile_id IS NOT NULL AND c.terminal_status IN ('stopped','stopped_user','stopped_game_exited') AND r.capture_id IS NULL ORDER BY c.imported_at,e.epoch").fetch_all(&mut self.connection).await?;
        let mut created = 0;
        for (capture, epoch, profile) in epochs {
            let rows=sqlx::query("SELECT source_sequence,observed_ms,event_json,projection_result FROM events WHERE capture_id=? AND epoch=? ORDER BY source_sequence").bind(&capture).bind(&epoch).fetch_all(&mut self.connection).await?;
            let mut frame = Frame::default();
            for row in rows {
                let event: Value = serde_json::from_str(row.get("event_json"))?;
                if frame
                    .apply(
                        &event,
                        row.get("source_sequence"),
                        row.get("observed_ms"),
                        row.get::<String, _>("projection_result") == "applied",
                    )
                    .is_err()
                {
                    frame.bad = true;
                    break;
                }
            }
            let complete = frame.complete();
            let mut tx = self.connection.begin().await?;
            if complete {
                let id = Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    format!("{capture}:{epoch}:{profile}").as_bytes(),
                )
                .to_string();
                sqlx::query("INSERT INTO recovery_points VALUES(?,?,?,?,?,?,?)").bind(id).bind(&profile).bind(&capture).bind(&epoch).bind(frame.last).bind(frame.sequence).bind(json!({"schema_version":1,"collections":frame.collections,"coverage":frame.coverage,"deletions":frame.deletions}).to_string()).execute(&mut *tx).await?;
                // Only successful complete checkpoints participate in the three-point limit.
                sqlx::query("DELETE FROM recovery_points WHERE profile_id=? AND point_id NOT IN (SELECT point_id FROM recovery_points WHERE profile_id=? ORDER BY observed_ms DESC,capture_id DESC,end_sequence DESC LIMIT 3)").bind(&profile).bind(&profile).execute(&mut *tx).await?;
                created += 1;
            }
            sqlx::query("INSERT INTO retention_epochs VALUES(?,?,?)")
                .bind(&capture)
                .bind(&epoch)
                .bind(if complete {
                    "materialized"
                } else {
                    "incomplete_or_legacy"
                })
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(created)
    }
    pub async fn recovery_points(&mut self) -> Result<Value> {
        let rows=sqlx::query("SELECT r.*,p.uid,p.server_region FROM recovery_points r JOIN profiles p USING(profile_id) ORDER BY p.uid,p.server_region,r.observed_ms DESC").fetch_all(&mut self.connection).await?;
        Ok(Value::Array(rows.iter().map(|r|json!({"point_id":r.get::<String,_>("point_id"),"profile_id":r.get::<String,_>("profile_id"),"uid":r.get::<String,_>("uid"),"server_region":r.get::<String,_>("server_region"),"capture_id":r.get::<String,_>("capture_id"),"epoch":r.get::<String,_>("epoch"),"observed_ms":r.get::<i64,_>("observed_ms").to_string(),"end_sequence":r.get::<i64,_>("end_sequence").to_string()})).collect()))
    }
    pub async fn meaningful_hash(&mut self) -> Result<String> {
        let mut hash = Sha256::new();
        for r in sqlx::query("SELECT p.uid,p.server_region,e.collection,e.entity_key,e.value_json FROM current_entities e JOIN profiles p USING(profile_id) ORDER BY p.uid,p.server_region,e.collection,e.entity_key").fetch_all(&mut self.connection).await? {
            for name in ["uid","server_region","collection","entity_key","value_json"] {let value:String=r.get(name);hash.update((value.len() as u64).to_le_bytes());hash.update(value.as_bytes());}
        }
        for value in
            sqlx::query_scalar::<_, String>("SELECT settings_json FROM portable_preferences")
                .fetch_all(&mut self.connection)
                .await?
        {
            hash.update(value.as_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub async fn artifacts(&mut self) -> Result<Vec<Artifact>> {
        Ok(
            sqlx::query("SELECT * FROM retention_artifacts ORDER BY relative_path")
                .fetch_all(&mut self.connection)
                .await?
                .iter()
                .map(|r| Artifact {
                    id: r.get("artifact_id"),
                    kind: r.get("kind"),
                    path: r.get("relative_path"),
                    capture: r.get("capture_id"),
                    pinned: r.get::<i64, _>("pinned") != 0,
                    state: r.get("state"),
                })
                .collect(),
        )
    }
    pub async fn register_artifact(
        &mut self,
        id: &str,
        kind: &str,
        path: &str,
        capture: Option<&str>,
        pinned: bool,
    ) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO retention_artifacts(artifact_id,kind,relative_path,capture_id,pinned) VALUES(?,?,?,?,?)").bind(id).bind(kind).bind(path).bind(capture).bind(pinned).execute(&mut self.connection).await?;
        Ok(())
    }
    pub async fn pin_artifact(&mut self, id: &str, pinned: bool) -> Result<()> {
        ensure!(
            sqlx::query(
                "UPDATE retention_artifacts SET pinned=? WHERE artifact_id=? AND state='present'"
            )
            .bind(pinned)
            .bind(id)
            .execute(&mut self.connection)
            .await?
            .rows_affected()
                == 1,
            "Unknown artifact"
        );
        Ok(())
    }
    pub async fn artifact_protection(&mut self, a: &Artifact) -> Result<Vec<String>> {
        let mut reasons = vec![];
        if a.pinned {
            reasons.push("pinned".into());
        }
        if a.state != "present" {
            reasons.push(a.state.clone());
        }
        if let Some(capture) = &a.capture {
            let terminal: Option<String> =
                sqlx::query_scalar("SELECT terminal_status FROM captures WHERE capture_id=?")
                    .bind(capture)
                    .fetch_optional(&mut self.connection)
                    .await?;
            if !terminal.is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "stopped" | "stopped_user" | "stopped_game_exited"
                )
            }) {
                reasons.push("unfinished_or_error_capture".into());
            }
            let refs:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM current_entities WHERE capture_id=?)+(SELECT count(*) FROM domain_state WHERE capture_id=?)+(SELECT count(*) FROM recovery_points WHERE capture_id=?)").bind(capture).bind(capture).bind(capture).fetch_one(&mut self.connection).await?;
            if refs > 0 {
                reasons.push("recovery_or_current_state_dependency".into());
            }
            let open: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM live_checkpoints WHERE capture_id=? AND closed=0",
            )
            .bind(capture)
            .fetch_one(&mut self.connection)
            .await?;
            if open > 0 {
                reasons.push("open_delivery_checkpoint".into());
            }
        } else {
            reasons.push("unregistered_diagnostics".into());
        }
        Ok(reasons)
    }
}
