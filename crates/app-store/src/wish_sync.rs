//! A sync result is valid only for the live profile/session that initiated it.
use crate::Store;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, Row};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishContext {
    pub profile_id: String,
    pub capture_id: String,
    pub epoch: String,
    pub uid: String,
    pub server_region: String,
}

impl Store {
    pub async fn wish_context(&mut self) -> Result<Option<WishContext>> {
        let row=sqlx::query("SELECT a.profile_id,a.capture_id,a.epoch,p.uid,p.server_region FROM active_context a JOIN profiles p ON p.profile_id=a.profile_id JOIN live_leases l ON l.profile_id=a.profile_id AND l.capture_id=a.capture_id AND l.epoch=a.epoch WHERE l.expires_ms>?")
            .bind(crate::ingest::now_ms()).fetch_optional(&mut self.connection).await?;
        Ok(row.map(|r| WishContext {
            profile_id: r.get("profile_id"),
            capture_id: r.get("capture_id"),
            epoch: r.get("epoch"),
            uid: r.get("uid"),
            server_region: r.get("server_region"),
        }))
    }
    pub async fn set_wish_sync(
        &mut self,
        context: &WishContext,
        status: &str,
        fetched_ms: Option<i64>,
    ) -> Result<()> {
        ensure!(
            matches!(
                status,
                "requested" | "syncing" | "synced" | "needs_access" | "failed" | "cancelled"
            ),
            "Invalid sync state"
        );
        let now = crate::ingest::now_ms();
        let mut tx = self.connection.begin().await?;
        sqlx::query("INSERT INTO wish_sync_state VALUES(?,?,?,?,?,?) ON CONFLICT(profile_id) DO UPDATE SET capture_id=excluded.capture_id,epoch=excluded.epoch,status=excluded.status,updated_ms=excluded.updated_ms,fetched_ms=COALESCE(excluded.fetched_ms,wish_sync_state.fetched_ms)")
            .bind(&context.profile_id).bind(&context.capture_id).bind(&context.epoch).bind(status).bind(now).bind(fetched_ms).execute(&mut *tx).await?;
        // Same durable notification path as imported history; no captured event is forged.
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
            .bind(&id).bind(format!("wish-sync:{id}")).bind("{}").bind("{\"mode\":\"system\"}").bind("wish-sync-v1").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO epochs VALUES(?,'0',?,'wish_sync')")
            .bind(&id)
            .bind(&context.profile_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE store_meta SET revision=revision+1 WHERE singleton=1")
            .execute(&mut *tx)
            .await?;
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events VALUES(?,1,'0',?,'wish_history.sync',?,?,'applied',?)")
            .bind(&id)
            .bind(&context.profile_id)
            .bind(now)
            .bind(json!({"kind":"wish_history.sync","payload":{"status":status}}).to_string())
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO change_log VALUES(?,?,1,?,?)")
            .bind(revision)
            .bind(&id)
            .bind(&context.profile_id)
            .bind("[\"wish_history\"]")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn wish_sync_state(&mut self, profile: &str) -> Result<Value> {
        let row = sqlx::query("SELECT * FROM wish_sync_state WHERE profile_id=?")
            .bind(profile)
            .fetch_optional(&mut self.connection)
            .await?;
        let Some(row) = row else {
            return Ok(Value::Null);
        };
        let mut status: String = row.get("status");
        let context = self.wish_context().await?;
        let matches = context.is_some_and(|c| {
            c.profile_id == profile
                && c.capture_id == row.get::<String, _>("capture_id")
                && c.epoch == row.get::<String, _>("epoch")
        });
        if !matches && matches!(status.as_str(), "synced" | "syncing") {
            status = "needs_sync".into();
        }
        // Lease continuity alone cannot heal an explicitly invalidated domain.
        let invalid:i64=sqlx::query_scalar("SELECT count(*) FROM domain_state WHERE profile_id=? AND freshness IN ('stale','error')").bind(profile).fetch_one(&mut self.connection).await?;
        if invalid > 0 && status == "synced" {
            status = "needs_sync".into();
        }
        let hidden:i64=sqlx::query_scalar("SELECT count(*) FROM wish_live w WHERE w.profile_id=? AND w.api_wish_id IS NULL AND (w.eligible=0 OR NOT EXISTS(SELECT 1 FROM wish_sync_state s WHERE s.profile_id=w.profile_id AND s.capture_id=w.capture_id AND s.epoch=w.epoch))").bind(profile).fetch_one(&mut self.connection).await?;
        if hidden > 0 && status == "synced" {
            status = "needs_sync".into();
        }
        Ok(
            json!({"status":status,"updated_ms":row.get::<i64,_>("updated_ms").to_string(),"fetched_ms":row.get::<Option<i64>,_>("fetched_ms").map(|n|n.to_string())}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sync_is_scoped_to_live_session_and_not_restored_as_live() {
        let temp = tempfile::tempdir().unwrap();
        let mut s = Store::open(&temp.path().join("test.db")).await.unwrap();
        sqlx::query(
            "INSERT INTO profiles VALUES('p','genshin','hk4e_global','os_euro','700000001')",
        )
        .execute(&mut s.connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO captures VALUES('00000000-0000-0000-0000-000000000001','test','{}','{}','test',0,'system','now')",
        )
        .execute(&mut s.connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO epochs VALUES('00000000-0000-0000-0000-000000000001','1','p','login'),('00000000-0000-0000-0000-000000000001','2','p','login')",
        )
        .execute(&mut s.connection)
        .await
        .unwrap();
        sqlx::query("UPDATE active_context SET profile_id='p',capture_id='00000000-0000-0000-0000-000000000001',epoch='1'")
            .execute(&mut s.connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO live_leases VALUES('p','00000000-0000-0000-0000-000000000001','1',?)",
        )
        .bind(crate::ingest::now_ms() + 60_000)
        .execute(&mut s.connection)
        .await
        .unwrap();
        let c = s.wish_context().await.unwrap().unwrap();
        s.set_wish_sync(&c, "syncing", None).await.unwrap();
        assert_eq!(s.wish_sync_state("p").await.unwrap()["status"], "syncing");
        s.set_wish_sync(&c, "synced", Some(1000)).await.unwrap();
        assert_eq!(s.wish_sync_state("p").await.unwrap()["status"], "synced");
        sqlx::query("UPDATE active_context SET epoch='2'")
            .execute(&mut s.connection)
            .await
            .unwrap();
        sqlx::query("UPDATE live_leases SET epoch='2'")
            .execute(&mut s.connection)
            .await
            .unwrap();
        assert_eq!(
            s.wish_sync_state("p").await.unwrap()["status"],
            "needs_sync"
        );
        let c2 = s.wish_context().await.unwrap().unwrap();
        assert_ne!(c, c2);
        s.set_wish_sync(&c2, "requested", None).await.unwrap();
        assert_eq!(s.wish_sync_state("p").await.unwrap()["status"], "requested");
        s.set_wish_sync(&c2, "synced", Some(2000)).await.unwrap();
        sqlx::query("DELETE FROM live_leases")
            .execute(&mut s.connection)
            .await
            .unwrap();
        assert!(s.wish_context().await.unwrap().is_none());
        assert_eq!(
            s.wish_sync_state("p").await.unwrap()["status"],
            "needs_sync"
        );
        let path = temp.path().join("sync.visionbackup");
        s.backup(
            &path,
            crate::backup::Settings::default(),
            json!({"schema_version":1}),
        )
        .await
        .unwrap();
        let archive = crate::backup::validate(&path).await.unwrap();
        let mut restored = Store::open(&temp.path().join("restored.db")).await.unwrap();
        restored
            .restore(&archive, &temp.path().join("rollback.visionbackup"), &[])
            .await
            .unwrap();
        assert_eq!(
            restored.wish_sync_state("p").await.unwrap()["status"],
            "needs_sync"
        );
        assert_eq!(
            restored.wish_sync_state("p").await.unwrap()["fetched_ms"],
            "2000"
        );
    }
}
