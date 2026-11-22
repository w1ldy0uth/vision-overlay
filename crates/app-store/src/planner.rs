//! User goals are not captured game facts and survive every account snapshot.
use crate::Store;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sqlx::{Connection, Row};

impl Store {
    pub async fn planner_goals(&mut self, profile: &str) -> Result<Value> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE profile_id=?")
            .bind(profile)
            .fetch_one(&mut self.connection)
            .await?;
        ensure!(count == 1, "Profile not found");
        let rows = sqlx::query("SELECT * FROM planner_goals WHERE profile_id=? ORDER BY goal_id")
            .bind(profile)
            .fetch_all(&mut self.connection)
            .await?;
        let goals=rows.into_iter().map(|r|Ok(json!({"goal_id":r.get::<String,_>("goal_id"),"revision":r.get::<i64,_>("revision").to_string(),"values":serde_json::from_str::<Value>(r.get("goal_json"))?}))).collect::<Result<Vec<_>>>()?;
        Ok(json!({"goals":goals}))
    }
    pub async fn save_planner_goal(&mut self, profile: &str, request: Value) -> Result<Value> {
        let obj = request.as_object().context("Missing request")?;
        ensure!(
            obj.keys()
                .all(|k| matches!(k.as_str(), "goal_id" | "revision" | "values" | "delete")),
            "Unknown goal field"
        );
        let id = match request["goal_id"].as_str() {
            Some(s) => {
                uuid::Uuid::parse_str(s)?;
                s.to_owned()
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        let expected = request["revision"]
            .as_str()
            .map(crate::reader::decimal)
            .transpose()?;
        let delete = request["delete"] == true;
        let v = &request["values"];
        let mut tx = self.connection.begin().await?;
        let profile_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM profiles WHERE profile_id=?")
                .bind(profile)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(profile_count == 1, "Profile not found");
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM planner_goals WHERE profile_id=? AND goal_id=?",
        )
        .bind(profile)
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
        ensure!(current == expected, "Goal changed; reload before editing");
        if delete {
            ensure!(current.is_some(), "Goal not found");
        } else {
            let fields = v.as_object().context("Missing goal")?;
            ensure!(
                fields.keys().all(|k| matches!(
                    k.as_str(),
                    "collection"
                        | "entity_key"
                        | "level"
                        | "ascension"
                        | "skill_depot_id"
                        | "talents"
                        | "priority"
                        | "equipped_weapon"
                )),
                "Unknown goal property"
            );
            let collection = v["collection"].as_str().context("Missing collection")?;
            ensure!(
                matches!(collection, "characters" | "weapons"),
                "Invalid goal collection"
            );
            let key = v["entity_key"].as_str().context("Missing entity")?;
            crate::legacy::decimal(key)?;
            ensure!(
                v["level"].as_u64().is_some_and(|n| (1..=100).contains(&n))
                    && v["ascension"].as_u64().is_some_and(|n| n <= 6)
                    && v["priority"].as_u64().is_some_and(|n| n <= 999),
                "Invalid goal target"
            );
            if let Some(weapon) = v.get("equipped_weapon") {
                let target = weapon
                    .as_object()
                    .context("Invalid equipped weapon target")?;
                ensure!(
                    collection == "characters"
                        && target.len() == 2
                        && weapon["level"]
                            .as_u64()
                            .is_some_and(|n| (1..=90).contains(&n))
                        && weapon["ascension"].as_u64().is_some_and(|n| {
                            let level = weapon["level"].as_u64().unwrap_or_default();
                            let caps = [20, 40, 50, 60, 70, 80];
                            let minimum = caps.iter().filter(|&&cap| cap < level).count() as u64;
                            n == minimum || (n == minimum + 1 && caps.contains(&level))
                        }),
                    "Invalid equipped weapon target"
                );
            }
            let duplicate:i64=sqlx::query_scalar("SELECT count(*) FROM planner_goals WHERE profile_id=? AND goal_id!=? AND json_extract(goal_json,'$.collection')=? AND json_extract(goal_json,'$.entity_key')=?").bind(profile).bind(&id).bind(collection).bind(key).fetch_one(&mut *tx).await?;
            ensure!(
                duplicate == 0,
                "An entity already has a goal; edit it instead"
            );
            let entity:String=sqlx::query_scalar("SELECT value_json FROM current_entities WHERE profile_id=? AND collection=? AND entity_key=?").bind(profile).bind(collection).bind(key).fetch_optional(&mut *tx).await?.context("Target entity not found")?;
            let entity: Value = serde_json::from_str(&entity)?;
            let talents = v["talents"].as_object().context("Missing talents")?;
            ensure!(talents.len() <= 6, "Too many talents");
            if collection == "weapons" {
                ensure!(
                    talents.is_empty() && v["skill_depot_id"].is_null(),
                    "Weapon has talents"
                );
            } else {
                let depot = v["skill_depot_id"]
                    .as_u64()
                    .context("Missing skill depot")?;
                let saved:Option<String>=sqlx::query_scalar("SELECT value_json FROM current_entities WHERE profile_id=? AND collection='traveler_depots' AND entity_key=?").bind(profile).bind(depot.to_string()).fetch_optional(&mut *tx).await?;
                let active = entity["skill_depot_id"].as_u64() == Some(depot);
                let skills = if active {
                    entity["skill_levels"].clone()
                } else {
                    let saved: Value =
                        serde_json::from_str(&saved.context("Unknown skill depot")?)?;
                    ensure!(saved["guid"] == key, "Traveler identity mismatch");
                    saved["skill_levels"].clone()
                };
                ensure!(
                    talents.iter().all(|(id, n)| skills.get(id).is_some()
                        && n.as_u64().is_some_and(|n| (1..=10).contains(&n))),
                    "Invalid base talent target"
                );
            }
            if current.is_none() {
                let count: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM planner_goals WHERE profile_id=?")
                        .bind(profile)
                        .fetch_one(&mut *tx)
                        .await?;
                ensure!(count < 100, "Goal limit reached");
            }
        }
        sqlx::query("UPDATE store_meta SET revision=revision+1 WHERE singleton=1")
            .execute(&mut *tx)
            .await?;
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta")
            .fetch_one(&mut *tx)
            .await?;
        if delete {
            sqlx::query("DELETE FROM planner_goals WHERE profile_id=? AND goal_id=?")
                .bind(profile)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO planner_goals VALUES(?,?,?,?) ON CONFLICT(profile_id,goal_id) DO UPDATE SET goal_json=excluded.goal_json,revision=excluded.revision").bind(profile).bind(&id).bind(v.to_string()).bind(revision).execute(&mut *tx).await?;
        }
        let capture = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))").bind(&capture).bind(format!("planner:{capture}")).bind("{}").bind("{\"mode\":\"system\"}").bind("planner-v1").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO epochs VALUES(?,'0',?,'user_goal')")
            .bind(&capture)
            .bind(profile)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events VALUES(?,1,'0',?,'planner.changed',?,?,'applied',?)")
            .bind(&capture)
            .bind(profile)
            .bind(crate::ingest::now_ms())
            .bind(
                json!({"kind":"planner.changed","payload":{"goal_id":id,"deleted":delete}})
                    .to_string(),
            )
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO change_log VALUES(?,?,1,?,?)")
            .bind(revision)
            .bind(&capture)
            .bind(profile)
            .bind("[\"planner\"]")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(json!({"goal_id":id,"revision":revision.to_string(),"deleted":delete}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn goals_are_profile_scoped_versioned_and_survive_portable_restore() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(&temp.path().join("test.db")).await.unwrap();
        let raw = [
            json!({"schema_version":1,"session":"0","sequence":1,"event":"source_config","data":{"mode":"live"}}),
            json!({"schema_version":1,"session":"1","sequence":2,"observed_unix_ms":"1000","event":"inventory_snapshot","data":{"completeness":"full","materials":[],"items":[{"guid":"42","item_id":11516,"kind":"weapon","level":1,"exp":0,"ascension":0}]}}),
            json!({"schema_version":1,"session":"1","sequence":3,"observed_unix_ms":"1001","event":"status","data":{"state":"stopped_game_exited"}}),
        ];
        let bytes = raw.iter().map(|v| format!("{v}\n")).collect::<String>();
        s.import(
            bytes.as_bytes(),
            &crate::legacy::Bindings::from([(
                "1".into(),
                crate::legacy::Identity {
                    uid: "700000001".into(),
                    server_region: "os_euro".into(),
                },
            )]),
        )
        .await
        .unwrap();
        let id = s.profiles().await.unwrap()[0]["profile_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let v = json!({"collection":"weapons","entity_key":"42","level":20,"ascension":0,"skill_depot_id":null,"talents":{},"priority":0});
        let result = s.save_planner_goal(&id, json!({"values":v})).await.unwrap();
        assert!(s.save_planner_goal(&id, json!({"values":v})).await.is_err());
        let goal = result["goal_id"].as_str().unwrap();
        let rev = result["revision"].as_str().unwrap();
        assert!(
            s.save_planner_goal(&id, json!({"goal_id":goal,"values":v}))
                .await
                .is_err()
        );
        assert!(
            s.save_planner_goal(
                "other",
                json!({"goal_id":goal,"revision":rev,"delete":true})
            )
            .await
            .is_err()
        );
        let path = temp.path().join("planner.visionbackup");
        s.backup(
            &path,
            crate::backup::Settings::default(),
            json!({"schema_version":1}),
        )
        .await
        .unwrap();
        let validated = crate::backup::validate(&path).await.unwrap();
        let mut restored = Store::open(&temp.path().join("restored.db")).await.unwrap();
        restored
            .restore(&validated, &temp.path().join("rollback.visionbackup"), &[])
            .await
            .unwrap();
        assert_eq!(
            s.planner_goals(&id).await.unwrap(),
            restored.planner_goals(&id).await.unwrap()
        );
        s.save_planner_goal(&id, json!({"goal_id":goal,"revision":rev,"delete":true}))
            .await
            .unwrap();
        assert_eq!(s.planner_goals(&id).await.unwrap()["goals"], json!([]));
    }
}
