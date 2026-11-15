//! Read-only, pooled API views and durable SSE cursors. No connection crosses a yield.
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{collections::BTreeMap, path::Path, time::Duration};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("Resource not found")]
    NotFound,
    #[error("State changed; restart pagination")]
    ViewChanged,
    #[error("Cursor is outside the retained change log")]
    ResyncRequired { earliest: String, current: String },
    #[error("Database read failed")]
    Database(#[from] sqlx::Error),
    #[error("Stored JSON is invalid")]
    Json(#[from] serde_json::Error),
    #[error("Stored state is invalid")]
    State(#[from] anyhow::Error),
}
type Result<T> = std::result::Result<T, ReadError>;
#[derive(Clone)]
pub struct Reader {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub collection: Option<String>,
    pub q: Option<String>,
    pub confirmed: Option<bool>,
    pub revision: Option<String>,
    pub kind: Option<String>,
    pub order: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u32,
    context: String,
    revision: String,
    last: Vec<String>,
}
fn encode(context: &str, revision: i64, last: Vec<String>) -> String {
    BASE64_URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            version: 1,
            context: context.into(),
            revision: revision.to_string(),
            last,
        })
        .unwrap(),
    )
}
fn page(
    q: &PageQuery,
    context: &str,
    revision: i64,
    mutable: bool,
    keys: usize,
) -> Result<(i64, Vec<String>)> {
    let limit = q.limit.unwrap_or(100);
    if !(1..=1000).contains(&limit) {
        return Err(ReadError::BadRequest("limit must be 1..1000"));
    }
    if q.q.as_ref().is_some_and(|s| s.len() > 200) {
        return Err(ReadError::BadRequest("Search query is too long"));
    }
    if let Some(r) = &q.revision
        && decimal(r)? != revision
    {
        return Err(ReadError::ViewChanged);
    }
    let Some(c) = &q.cursor else {
        return Ok((i64::from(limit), vec![String::new(); keys]));
    };
    if c.len() > 4096 {
        return Err(ReadError::BadRequest("Invalid cursor"));
    }
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(c)
        .map_err(|_| ReadError::BadRequest("Invalid cursor"))?;
    let c: Cursor =
        serde_json::from_slice(&bytes).map_err(|_| ReadError::BadRequest("Invalid cursor"))?;
    if c.version != 1 || c.context != context || c.last.len() != keys {
        return Err(ReadError::BadRequest("Cursor does not match this request"));
    }
    let cursor_revision = decimal(&c.revision)?;
    if cursor_revision > revision || (mutable && cursor_revision != revision) {
        return Err(ReadError::ViewChanged);
    }
    Ok((i64::from(limit), c.last))
}
pub fn decimal(s: &str) -> Result<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return Err(ReadError::BadRequest(
            "Expected a nonnegative decimal string",
        ));
    }
    s.parse()
        .map_err(|_| ReadError::BadRequest("Cursor exceeds supported range"))
}
fn uuid(s: &str) -> Result<()> {
    Uuid::parse_str(s).map_err(|_| ReadError::BadRequest("Invalid resource ID"))?;
    Ok(())
}
async fn revision(tx: &mut Transaction<'_, Sqlite>) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
            .fetch_one(&mut **tx)
            .await?,
    )
}
async fn require_profile(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<()> {
    uuid(id)?;
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE profile_id=?")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    if exists == 0 {
        return Err(ReadError::NotFound);
    }
    Ok(())
}
fn unsupported(
    q: &PageQuery,
    search: bool,
    collection: bool,
    confirmed: bool,
    kind: bool,
) -> Result<()> {
    if (!search && q.q.is_some())
        || (!collection && q.collection.is_some())
        || (!confirmed && q.confirmed.is_some())
        || (!kind && (q.kind.is_some() || q.order.is_some()))
    {
        return Err(ReadError::BadRequest(
            "Filter is not supported by this endpoint",
        ));
    }
    Ok(())
}
async fn profiles_in(tx: &mut Transaction<'_, Sqlite>) -> Result<Value> {
    let profiles=sqlx::query("SELECT profile_id,game_id,account_namespace,server_region,uid FROM profiles ORDER BY server_region,uid").fetch_all(&mut **tx).await?;
    let mut result = Vec::new();
    for row in profiles {
        let id: String = row.get("profile_id");
        let domains=sqlx::query("SELECT collection,scope,freshness,reason,observed_ms,revision FROM domain_state WHERE profile_id=? ORDER BY collection").bind(&id).fetch_all(&mut **tx).await?;
        result.push(json!({"profile_id":id,"game_id":row.get::<String,_>("game_id"),"account_namespace":row.get::<String,_>("account_namespace"),"server_region":row.get::<String,_>("server_region"),"uid":row.get::<String,_>("uid"),"domains":domains.iter().map(|r|json!({"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope"),"freshness":r.get::<String,_>("freshness"),"reason":r.get::<String,_>("reason"),"observed_unix_ms":r.get::<i64,_>("observed_ms").to_string(),"revision":r.get::<i64,_>("revision").to_string()})).collect::<Vec<_>>()}));
    }
    Ok(json!(result))
}

impl Reader {
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }
    pub async fn close(&self) {
        self.pool.close().await;
    }
    pub async fn state(&self) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let profiles = profiles_in(&mut tx).await?;
        let context =
            sqlx::query("SELECT profile_id,capture_id,epoch FROM active_context WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        let active: Option<String> = context.get("profile_id");
        let session = if active.is_some() {
            Some(format!(
                "{}:{}",
                context.get::<String, _>("capture_id"),
                context.get::<String, _>("epoch")
            ))
        } else {
            None
        };
        tx.commit().await?;
        Ok(
            json!({"api_version":1,"revision":rev.to_string(),"mode":if active.is_some(){"live"}else{"saved_offline"},"active_game_profile_id":active,"active_game_session":session,"profiles":profiles,"capabilities":{"live_collector":false,"http_ingest":false,"durable_sse":true}}),
        )
    }
    pub async fn profiles(&self) -> Result<Value> {
        let mut v = self.state().await?;
        v.as_object_mut().unwrap().remove("capabilities");
        Ok(v)
    }
    pub async fn profile(&self, id: &str) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        require_profile(&mut tx, id).await?;
        tx.commit().await?;
        let mut conn = self.pool.acquire().await?;
        Ok(super::read_profile_state(&mut conn, id).await?)
    }
    pub async fn entities(&self, profile: &str, q: PageQuery, characters: bool) -> Result<Value> {
        unsupported(&q, true, !characters, true, false)?;
        if q.collection.as_ref().is_some_and(|c| {
            !matches!(
                c.as_str(),
                "material_stacks"
                    | "weapons"
                    | "artifacts"
                    | "furniture"
                    | "cosmetics"
                    | "unknown_items"
            )
        }) {
            return Err(ReadError::BadRequest("Unknown inventory collection"));
        }
        let mut tx = self.pool.begin().await?;
        require_profile(&mut tx, profile).await?;
        let rev = revision(&mut tx).await?;
        let collection = if characters {
            Some("characters")
        } else {
            q.collection.as_deref()
        };
        let context = serde_json::to_string(&json!([
            "entities",
            profile,
            characters,
            collection,
            q.q,
            q.confirmed
        ]))?;
        let (limit, last) = page(&q, &context, rev, true, 2)?;
        let kind = if characters {
            "characters"
        } else {
            "inventory"
        };
        let search = q.q.as_deref().unwrap_or("");
        let confirmed = q.confirmed.map(i64::from);
        let rows=sqlx::query("SELECT collection,entity_key,value_json,confirmed_in_baseline,revision,value_format FROM current_entities WHERE profile_id=? AND ((?='characters' AND collection='characters') OR (?='inventory' AND collection IN ('material_stacks','weapons','artifacts','furniture','cosmetics','unknown_items'))) AND (? IS NULL OR collection=?) AND (? IS NULL OR confirmed_in_baseline=?) AND (?='' OR instr(entity_key,?)>0 OR instr(COALESCE(CAST(json_extract(value_json,'$.item_id') AS TEXT),CAST(json_extract(value_json,'$.avatar_id') AS TEXT),''),?)>0) AND (collection>? OR (collection=? AND entity_key>?)) ORDER BY collection,entity_key LIMIT ?")
            .bind(profile).bind(kind).bind(kind).bind(collection).bind(collection).bind(confirmed).bind(confirmed).bind(search).bind(search).bind(search).bind(&last[0]).bind(&last[0]).bind(&last[1]).bind(limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut equipment = BTreeMap::new();
        if characters {
            for r in sqlx::query("SELECT entity_key,collection,value_json,confirmed_in_baseline FROM current_entities WHERE profile_id=? AND collection IN ('weapons','artifacts','unknown_items')").bind(profile).fetch_all(&mut *tx).await?{
                equipment.insert(r.get::<String,_>("entity_key"),json!({"resolved":true,"collection":r.get::<String,_>("collection"),"confirmation":if r.get::<i64,_>("confirmed_in_baseline")==1{"observed"}else{"historical_unconfirmed"},"values":serde_json::from_str::<Value>(r.get("value_json"))?}));
            }
        }
        let mut items = Vec::new();
        let mut next = None;
        for r in rows.into_iter().take(limit as usize) {
            let value: Value = serde_json::from_str(r.get("value_json"))?;
            let col: String = r.get("collection");
            let key: String = r.get("entity_key");
            if more {
                next = Some(encode(&context, rev, vec![col.clone(), key.clone()]));
            }
            let mut v = json!({"collection":col,"entity_key":key,"values":value,"value_format":r.get::<String,_>("value_format"),"confirmation":if r.get::<i64,_>("confirmed_in_baseline")==1{"observed"}else{"historical_unconfirmed"},"revision":r.get::<i64,_>("revision").to_string()});
            if characters {
                v["equipment"]=json!(value.get("equip_guids").or_else(||value.get("equipment_guids")).unwrap_or(&Value::Null).as_array().into_iter().flatten().filter_map(Value::as_str).map(|guid|json!({"guid":guid,"reference":equipment.get(guid).cloned().unwrap_or(json!({"resolved":false}))})).collect::<Vec<_>>());
            }
            items.push(v);
        }
        let domains=sqlx::query("SELECT collection,scope,freshness,reason,observed_ms FROM domain_state WHERE profile_id=? AND (? IS NULL OR collection=?) ORDER BY collection").bind(profile).bind(collection).bind(collection).fetch_all(&mut *tx).await?;
        let domains:Vec<_>=domains.iter().filter(|r|if characters{r.get::<String,_>("collection")=="characters"}else{matches!(r.get::<String,_>("collection").as_str(),"material_stacks"|"weapons"|"artifacts"|"furniture"|"cosmetics"|"unknown_items")}).map(|r|json!({"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope"),"freshness":r.get::<String,_>("freshness"),"reason":r.get::<String,_>("reason"),"observed_unix_ms":r.get::<i64,_>("observed_ms").to_string()})).collect();
        let active: Option<String> =
            sqlx::query_scalar("SELECT profile_id FROM active_context WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"profile_id":profile,"mode":if active.as_deref()==Some(profile){"live"}else{"saved_offline"},"domains":domains,"items":items,"next_cursor":next}),
        )
    }
    pub async fn history(&self, profile: &str, q: PageQuery) -> Result<Value> {
        unsupported(&q, false, false, false, true)?;
        let mut tx = self.pool.begin().await?;
        require_profile(&mut tx, profile).await?;
        let rev = revision(&mut tx).await?;
        let descending = match q.order.as_deref() {
            None | Some("asc") => false,
            Some("desc") => true,
            _ => return Err(ReadError::BadRequest("Unknown history order")),
        };
        let context = serde_json::to_string(&json!(["history", profile, q.kind, descending]))?;
        let (limit, last) = page(&q, &context, rev, false, 1)?;
        let after = if last[0].is_empty() {
            if descending { i64::MAX } else { 0 }
        } else {
            decimal(&last[0])?
        };
        let rows=sqlx::query("SELECT c.revision,c.capture_id,c.source_sequence,c.topics_json,e.kind,e.epoch,e.observed_ms,e.projection_result,CASE WHEN e.kind LIKE '%snapshot' THEN NULL ELSE e.event_json END AS event_json FROM change_log c JOIN events e ON e.capture_id=c.capture_id AND e.source_sequence=c.source_sequence WHERE c.profile_id=? AND ((?=0 AND c.revision>?) OR (?=1 AND c.revision<?)) AND (? IS NULL OR e.kind=?) ORDER BY CASE WHEN ?=0 THEN c.revision END ASC,CASE WHEN ?=1 THEN c.revision END DESC LIMIT ?")
            .bind(profile).bind(descending).bind(after).bind(descending).bind(after).bind(q.kind.as_deref()).bind(q.kind.as_deref()).bind(descending).bind(descending).bind(limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut next = None;
        let mut items = Vec::new();
        for r in rows.into_iter().take(limit as usize) {
            let n: i64 = r.get("revision");
            if more {
                next = Some(encode(&context, rev, vec![n.to_string()]));
            }
            let body = r
                .get::<Option<String>, _>("event_json")
                .map(|s| serde_json::from_str::<Value>(&s))
                .transpose()?;
            items.push(json!({"revision":n.to_string(),"capture_id":r.get::<String,_>("capture_id"),"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"epoch":r.get::<String,_>("epoch"),"kind":r.get::<String,_>("kind"),"observed_unix_ms":r.get::<Option<i64>,_>("observed_ms").map(|n|n.to_string()),"topics":serde_json::from_str::<Value>(r.get("topics_json"))?,"data":body.as_ref().map(|b|if b["contract_version"]==1{&b["payload"]}else{&b["data"]}),"snapshot_payload_omitted":body.is_none(),"projection_result":r.get::<String,_>("projection_result")}));
        }
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"profile_id":profile,"items":items,"next_cursor":next}),
        )
    }
    pub async fn material_totals(&self, profile: &str, q: PageQuery) -> Result<Value> {
        unsupported(&q, true, false, false, false)?;
        let mut tx = self.pool.begin().await?;
        require_profile(&mut tx, profile).await?;
        let rev = revision(&mut tx).await?;
        let context = serde_json::to_string(&json!(["material_totals", profile, q.q]))?;
        let (limit, last) = page(&q, &context, rev, true, 1)?;
        let after = if last[0].is_empty() {
            0
        } else {
            decimal(&last[0])?
        };
        let search = q.q.as_deref().unwrap_or("");
        let rows=sqlx::query("SELECT CAST(json_extract(value_json,'$.item_id') AS INTEGER) AS item_id,SUM(CAST(json_extract(value_json,'$.count') AS INTEGER)) AS count FROM current_entities WHERE profile_id=? AND collection='material_stacks' AND confirmed_in_baseline=1 AND (?='' OR instr(CAST(json_extract(value_json,'$.item_id') AS TEXT),?)>0) GROUP BY item_id HAVING item_id>? ORDER BY item_id LIMIT ?")
            .bind(profile).bind(search).bind(search).bind(after).bind(limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut items = Vec::new();
        let mut next = None;
        for r in rows.into_iter().take(limit as usize) {
            let id: i64 = r.get("item_id");
            if more {
                next = Some(encode(&context, rev, vec![id.to_string()]));
            }
            items.push(json!({"item_id":id,"count":r.get::<i64,_>("count").to_string()}));
        }
        let domain=sqlx::query("SELECT scope,freshness,reason,observed_ms FROM domain_state WHERE profile_id=? AND collection='material_stacks'")
            .bind(profile).fetch_optional(&mut *tx).await?.map(|r|json!({"scope":r.get::<String,_>("scope"),"freshness":r.get::<String,_>("freshness"),"reason":r.get::<String,_>("reason"),"observed_unix_ms":r.get::<i64,_>("observed_ms").to_string()}));
        let active: Option<String> =
            sqlx::query_scalar("SELECT profile_id FROM active_context WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"profile_id":profile,"mode":if active.as_deref()==Some(profile){"live"}else{"saved_offline"},"domain":domain,"aggregation":"observed_stacks_only_not_proof_of_complete_inventory","items":items,"next_cursor":next}),
        )
    }
    pub async fn captures(&self, q: PageQuery) -> Result<Value> {
        unsupported(&q, false, false, false, false)?;
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let (limit, last) = page(&q, "captures", rev, false, 1)?;
        let rows=sqlx::query("SELECT capture_id,artifact_sha256,adapter_revision,event_count,terminal_status,imported_at FROM captures WHERE capture_id>? ORDER BY capture_id LIMIT ?").bind(&last[0]).bind(limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut items = Vec::new();
        let mut next = None;
        for r in rows.into_iter().take(limit as usize) {
            let id: String = r.get("capture_id");
            if more {
                next = Some(encode("captures", rev, vec![id.clone()]));
            }
            items.push(json!({"capture_id":id,"artifact_sha256":if r.get::<String,_>("artifact_sha256").starts_with("live:")||r.get::<String,_>("artifact_sha256").starts_with("system:"){None}else{Some(r.get::<String,_>("artifact_sha256"))},"adapter_revision":r.get::<String,_>("adapter_revision"),"event_count":r.get::<i64,_>("event_count").to_string(),"terminal_status":r.get::<String,_>("terminal_status"),"imported_at":r.get::<String,_>("imported_at")}));
        }
        tx.commit().await?;
        Ok(json!({"revision":rev.to_string(),"items":items,"next_cursor":next}))
    }
    pub async fn capture(&self, id: &str) -> Result<Value> {
        uuid(id)?;
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let r=sqlx::query("SELECT source_json,adapter_revision,event_count,terminal_status,imported_at FROM captures WHERE capture_id=?").bind(id).fetch_optional(&mut *tx).await?.ok_or(ReadError::NotFound)?;
        let epochs=sqlx::query("SELECT epoch,profile_id,evidence FROM epochs WHERE capture_id=? ORDER BY length(epoch),epoch").bind(id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"capture_id":id,"source":serde_json::from_str::<Value>(r.get("source_json"))?,"adapter_revision":r.get::<String,_>("adapter_revision"),"event_count":r.get::<i64,_>("event_count").to_string(),"terminal_status":r.get::<String,_>("terminal_status"),"imported_at":r.get::<String,_>("imported_at"),"epochs":epochs.iter().map(|r|json!({"epoch":r.get::<String,_>("epoch"),"profile_id":r.get::<Option<String>,_>("profile_id"),"identity_evidence":r.get::<String,_>("evidence")})).collect::<Vec<_>>()}),
        )
    }
    /// Bounded, transaction-consistent facts for one diagnostic capture only.
    pub async fn diagnostic_capture(&self, id: &str) -> Result<Value> {
        uuid(id)?;
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let c=sqlx::query("SELECT source_json,adapter_revision,event_count,terminal_status,imported_at FROM captures WHERE capture_id=?").bind(id).fetch_optional(&mut *tx).await?.ok_or(ReadError::NotFound)?;
        let profiles=sqlx::query("SELECT DISTINCT p.profile_id,p.server_region,p.uid FROM profiles p JOIN epochs e ON p.profile_id=e.profile_id WHERE e.capture_id=? ORDER BY p.profile_id").bind(id).fetch_all(&mut *tx).await?;
        let mut truncated = false;
        let mut bytes = 0usize;
        let mut snapshots = Vec::new();
        {
            let mut rows=sqlx::query("SELECT snapshot_id,source_sequence,profile_id,collection,scope,snapshot_json,value_format FROM snapshots WHERE capture_id=? ORDER BY source_sequence DESC LIMIT 193").bind(id).fetch(&mut *tx);
            while let Some(r) = rows.try_next().await? {
                if snapshots.len() >= 192 {
                    truncated = true;
                    break;
                }
                let raw: String = r.get("snapshot_json");
                bytes += raw.len();
                if bytes > 64 * 1024 * 1024 {
                    truncated = true;
                    break;
                }
                snapshots.push(json!({"snapshot_id":r.get::<String,_>("snapshot_id"),"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"profile_id":r.get::<String,_>("profile_id"),"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope"),"value_format":r.get::<String,_>("value_format"),"entities":serde_json::from_str::<Value>(&raw)?}));
            }
        }
        let mut events_truncated = false;
        let mut events = Vec::new();
        let mut event_bytes = 0usize;
        {
            let mut rows=sqlx::query("SELECT source_sequence,event_json,projection_result FROM events WHERE capture_id=? ORDER BY source_sequence DESC LIMIT 2001").bind(id).fetch(&mut *tx);
            while let Some(r) = rows.try_next().await? {
                if events.len() >= 2000 {
                    events_truncated = true;
                    break;
                }
                let raw: String = r.get("event_json");
                event_bytes += raw.len();
                if event_bytes > 32 * 1024 * 1024 {
                    events_truncated = true;
                    break;
                }
                events.push(json!({"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"projection_result":r.get::<String,_>("projection_result"),"event":serde_json::from_str::<Value>(&raw)?}));
            }
        }
        events.reverse();
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"capture":{"capture_id":id,"source":serde_json::from_str::<Value>(c.get("source_json"))?,"adapter_revision":c.get::<String,_>("adapter_revision"),"event_count":c.get::<i64,_>("event_count").to_string(),"terminal_status":c.get::<String,_>("terminal_status"),"imported_at":c.get::<String,_>("imported_at")},"profiles":profiles.iter().map(|r|json!({"profile_id":r.get::<String,_>("profile_id"),"server_region":r.get::<String,_>("server_region"),"uid":r.get::<String,_>("uid")})).collect::<Vec<_>>(),"snapshots":snapshots,"snapshots_truncated":truncated,"events":events,"events_truncated":events_truncated}),
        )
    }
    pub async fn snapshots(&self, capture: &str, q: PageQuery) -> Result<Value> {
        uuid(capture)?;
        unsupported(&q, false, true, false, false)?;
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM captures WHERE capture_id=?")
            .bind(capture)
            .fetch_one(&mut *tx)
            .await?;
        if exists == 0 {
            return Err(ReadError::NotFound);
        }
        let context = serde_json::to_string(&json!(["snapshots", capture, q.collection]))?;
        let (limit, last) = page(&q, &context, rev, false, 1)?;
        let rows=sqlx::query("SELECT snapshot_id,source_sequence,profile_id,collection,scope FROM snapshots WHERE capture_id=? AND snapshot_id>? AND (? IS NULL OR collection=?) ORDER BY snapshot_id LIMIT ?").bind(capture).bind(&last[0]).bind(q.collection.as_deref()).bind(q.collection.as_deref()).bind(limit+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > limit as usize;
        let mut items = Vec::new();
        let mut next = None;
        for r in rows.into_iter().take(limit as usize) {
            let id: String = r.get("snapshot_id");
            if more {
                next = Some(encode(&context, rev, vec![id.clone()]));
            }
            items.push(json!({"snapshot_id":id,"capture_id":capture,"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"profile_id":r.get::<String,_>("profile_id"),"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope")}));
        }
        tx.commit().await?;
        Ok(json!({"revision":rev.to_string(),"items":items,"next_cursor":next}))
    }
    pub async fn snapshot(&self, id: &str) -> Result<Value> {
        uuid(id)?;
        let mut tx = self.pool.begin().await?;
        let rev = revision(&mut tx).await?;
        let r=sqlx::query("SELECT snapshot_id,capture_id,source_sequence,profile_id,collection,scope,snapshot_json,value_format FROM snapshots WHERE snapshot_id=?").bind(id).fetch_optional(&mut *tx).await?.ok_or(ReadError::NotFound)?;
        tx.commit().await?;
        Ok(
            json!({"revision":rev.to_string(),"snapshot_id":id,"capture_id":r.get::<String,_>("capture_id"),"source_sequence":r.get::<i64,_>("source_sequence").to_string(),"profile_id":r.get::<String,_>("profile_id"),"collection":r.get::<String,_>("collection"),"scope":r.get::<String,_>("scope"),"value_format":r.get::<String,_>("value_format"),"entities":serde_json::from_str::<Value>(r.get("snapshot_json"))?}),
        )
    }
    /// Returns only committed rows; the same transaction validates the cursor and reads data.
    pub async fn changes(&self, after: i64, limit: i64) -> Result<Vec<Value>> {
        if after < 0 || !(1..=1000).contains(&limit) {
            return Err(ReadError::BadRequest("Invalid change cursor"));
        }
        let mut tx = self.pool.begin().await?;
        let latest = revision(&mut tx).await?;
        let earliest: Option<i64> = sqlx::query_scalar("SELECT min(revision) FROM change_log")
            .fetch_one(&mut *tx)
            .await?;
        let floor = earliest.map(|n| n - 1).unwrap_or(latest);
        let resync = || ReadError::ResyncRequired {
            earliest: floor.to_string(),
            current: latest.to_string(),
        };
        if after < floor || after > latest {
            return Err(resync());
        }
        let rows=sqlx::query("SELECT c.revision,c.profile_id,c.capture_id,c.topics_json,e.epoch FROM change_log c JOIN events e ON e.capture_id=c.capture_id AND e.source_sequence=c.source_sequence WHERE c.revision>? ORDER BY c.revision LIMIT ?").bind(after).bind(limit).fetch_all(&mut *tx).await?;
        let mut expected = after;
        let mut result = Vec::new();
        for r in rows {
            let n: i64 = r.get("revision");
            if expected.checked_add(1) != Some(n) {
                return Err(resync());
            }
            expected = n;
            result.push(json!({"revision":n.to_string(),"profile_id":r.get::<Option<String>,_>("profile_id"),"capture_id":r.get::<String,_>("capture_id"),"epoch":r.get::<String,_>("epoch"),"topics":serde_json::from_str::<Value>(r.get("topics_json"))?}));
        }
        if result.is_empty() && after < latest {
            return Err(resync());
        }
        tx.commit().await?;
        Ok(result)
    }
}
