//! Ordered authenticated-producer delivery. ACKs are durable, not a queue receipt.
use crate::{
    Store,
    core::{self, Action, Record},
    digest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, Row};
use std::collections::{BTreeMap, BTreeSet};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Invalid {
        code: &'static str,
        message: &'static str,
        expected: Option<String>,
    },
    #[error("Storage unavailable")]
    Storage(#[from] anyhow::Error),
    #[error("Storage unavailable")]
    Sql(#[from] sqlx::Error),
}
fn invalid(code: &'static str, message: &'static str) -> Error {
    Error::Invalid {
        code,
        message,
        expected: None,
    }
}
type Result<T> = std::result::Result<T, Error>;
pub fn now_ms() -> i64 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis())
    .min(i64::MAX as u128) as i64
}
fn number(s: &str, positive: bool) -> Result<i64> {
    let n = crate::reader::decimal(s)
        .map_err(|_| invalid("invalid_event", "Invalid decimal counter"))?;
    if positive && n == 0 {
        return Err(invalid("invalid_event", "Counter must be positive"));
    }
    Ok(n)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileKey {
    pub game_id: String,
    pub account_namespace: String,
    pub server_region: String,
    pub uid: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub producer: String,
    pub producer_version: String,
    pub protocol_version: String,
    pub adapter_revision: String,
    pub mode: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub contract_version: u32,
    pub capture_id: String,
    pub source_sequence: String,
    pub epoch: Option<String>,
    pub profile: Option<ProfileKey>,
    pub source: Source,
    pub observed_at: Option<String>,
    pub source_elapsed_ms: Option<String>,
    pub kind: String,
    pub payload: Value,
}
impl Envelope {
    pub fn parse(v: Value) -> Result<Self> {
        for key in [
            "contract_version",
            "capture_id",
            "source_sequence",
            "epoch",
            "profile",
            "source",
            "observed_at",
            "source_elapsed_ms",
            "kind",
            "payload",
        ] {
            if v.get(key).is_none() {
                return Err(invalid("invalid_event", "Missing envelope field"));
            }
        }
        let e: Self =
            serde_json::from_value(v).map_err(|_| invalid("invalid_event", "Invalid envelope"))?;
        if e.contract_version != 1
            || Uuid::parse_str(&e.capture_id).is_err()
            || e.source.mode != "live"
        {
            return Err(invalid(
                "unsupported_version",
                "Unsupported source contract",
            ));
        }
        number(&e.source_sequence, true)?;
        if let Some(epoch) = &e.epoch {
            number(epoch, true)?;
        }
        if let Some(n) = &e.source_elapsed_ms {
            number(n, false)?;
        }
        for s in [
            &e.source.producer,
            &e.source.producer_version,
            &e.source.protocol_version,
            &e.source.adapter_revision,
        ] {
            if s.is_empty() || s.len() > 128 {
                return Err(invalid("invalid_event", "Invalid source metadata"));
            }
        }
        e.observed_ms()?;
        if let Some(p) = &e.profile {
            if p.game_id != "genshin" || p.account_namespace != "hk4e_global" {
                return Err(invalid(
                    "identity_conflict",
                    "Unsupported profile namespace",
                ));
            }
            crate::legacy::Identity {
                uid: p.uid.clone(),
                server_region: p.server_region.clone(),
            }
            .validate()
            .map_err(|_| invalid("identity_conflict", "Invalid UID or region"))?;
        }
        if !e.payload.is_object() {
            return Err(invalid("invalid_event", "Payload must be an object"));
        }
        Ok(e)
    }
    pub fn observed_ms(&self) -> Result<Option<i64>> {
        self.observed_at
            .as_ref()
            .map(|s| {
                let t = OffsetDateTime::parse(s, &Rfc3339)
                    .map_err(|_| invalid("invalid_event", "Invalid observed_at"))?;
                if t.offset() != time::UtcOffset::UTC {
                    return Err(invalid("invalid_event", "observed_at must be UTC"));
                }
                i64::try_from(t.unix_timestamp_nanos() / 1_000_000)
                    .map_err(|_| invalid("invalid_event", "Time exceeds range"))
            })
            .transpose()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Collection {
    name: String,
    scope: String,
    verified_fields: Vec<String>,
    live_fields: Vec<String>,
    entities: Vec<Entity>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entity {
    entity_key: String,
    values: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    snapshot_id: String,
    collections: Vec<Collection>,
    evidence: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Op {
    collection: String,
    entity_key: String,
    action: String,
    values: Value,
    expected: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    operations: Vec<Op>,
    evidence: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invalidation {
    collections: Vec<String>,
    reason: String,
    requires_snapshot: bool,
}
fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T> {
    serde_json::from_value(v.clone()).map_err(|_| invalid("invalid_event", "Invalid event payload"))
}
fn fields(v: &Value, allowed: &[&str]) -> Result<()> {
    if v.as_object()
        .unwrap()
        .keys()
        .any(|k| !allowed.contains(&k.as_str()))
        || allowed.iter().any(|k| v.get(k).is_none())
    {
        return Err(invalid(
            "invalid_event",
            "Unexpected or missing payload field",
        ));
    }
    Ok(())
}
fn reason(v: &Value) -> Result<&str> {
    let r = v["reason"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or_else(|| invalid("invalid_event", "Invalid reason"))?;
    Ok(r)
}
fn value_format(evidence: &Value) -> Result<()> {
    if evidence
        .get("value_format")
        .is_some_and(|s| s != "legacy_capture_v1" && s != "domain_v1")
    {
        return Err(invalid("invalid_event", "Unsupported value format"));
    }
    Ok(())
}

impl Store {
    pub async fn ingest(&mut self, capture: &str, values: Vec<Value>) -> Result<Value> {
        if self.capture_blocked(capture).await? {
            return Err(invalid(
                "capture_restored",
                "Old capture cannot resume after restore",
            ));
        }
        if values.is_empty() || values.len() > 256 {
            return Err(invalid("invalid_batch", "Batch must contain 1..256 events"));
        }
        let mut envelopes = Vec::new();
        for v in values {
            let e = Envelope::parse(v)?;
            if e.kind == "capture.opened" {
                let barrier: Option<i64> = sqlx::query_scalar(
                    "SELECT restored_unix_ms FROM restore_barrier WHERE singleton=1",
                )
                .fetch_optional(&mut self.connection)
                .await?;
                if let Some(barrier) = barrier
                    && e.observed_ms()?.is_none_or(|time| time < barrier)
                {
                    return Err(invalid(
                        "capture_restored",
                        "Capture predates restored installation; start a fresh capture",
                    ));
                }
            }
            if e.capture_id != capture {
                return Err(invalid(
                    "identity_conflict",
                    "Capture ID differs from route",
                ));
            }
            envelopes.push(e);
        }
        let mut tx = self.connection.begin().await?;
        let mut revision: i64 =
            sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        let mut results = Vec::new();
        for e in envelopes {
            let seq = number(&e.source_sequence, true)?;
            let raw = serde_json::to_value(&e).unwrap();
            let hash = digest(&serde_json::to_vec(&raw).unwrap());
            let now = now_ms();
            if let Some(row)=sqlx::query("SELECT event_hash,result_json FROM delivery_results WHERE capture_id=? AND source_sequence=?").bind(capture).bind(seq).fetch_optional(&mut *tx).await?{
                if row.get::<String,_>("event_hash")!=hash{return Err(invalid("event_conflict","Delivery key has different payload"));}
                let mut ack:Value=serde_json::from_str(row.get("result_json")).map_err(|_|invalid("storage_error","Stored ACK is invalid"))?;ack["duplicate"]=json!(true);results.push(ack);continue;
            }
            let checkpoint=sqlx::query("SELECT last_sequence,current_epoch,closed FROM live_checkpoints WHERE capture_id=?").bind(capture).fetch_optional(&mut *tx).await?;
            let last = checkpoint
                .as_ref()
                .map(|r| r.get::<i64, _>("last_sequence"))
                .unwrap_or(0);
            if seq != last + 1 {
                return Err(Error::Invalid {
                    code: "sequence_gap",
                    message: "Events must be contiguous",
                    expected: Some((last + 1).to_string()),
                });
            }
            if checkpoint
                .as_ref()
                .is_some_and(|r| r.get::<i64, _>("closed") != 0)
            {
                return Err(invalid("capture_closed", "Capture is already closed"));
            }
            let current_epoch = checkpoint
                .as_ref()
                .and_then(|r| r.get::<Option<String>, _>("current_epoch"));
            let mut profile = None;
            let mut actions = Vec::new();
            let mut rejected = false;
            if seq == 1 {
                if e.kind != "capture.opened" || e.epoch.is_some() || e.profile.is_some() {
                    return Err(invalid(
                        "invalid_event",
                        "Capture must start with capture.opened",
                    ));
                }
                fields(&e.payload, &["configuration"])?;
                sqlx::query("INSERT INTO captures(capture_id,artifact_sha256,binding_json,source_json,adapter_revision,event_count,terminal_status) VALUES(?,?,?, ?,?,0,'running')")
                    .bind(capture).bind(format!("live:{capture}")).bind("{}").bind(serde_json::to_string(&e.source).unwrap()).bind(&e.source.adapter_revision).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO epochs VALUES(?,'0',NULL,'capture_scope')")
                    .bind(capture)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("INSERT INTO live_checkpoints(capture_id,last_sequence,last_contact_ms) VALUES(?,0,?)").bind(capture).bind(now).execute(&mut *tx).await?;
            } else {
                let original: String =
                    sqlx::query_scalar("SELECT source_json FROM captures WHERE capture_id=?")
                        .bind(capture)
                        .fetch_one(&mut *tx)
                        .await?;
                if serde_json::from_str::<Source>(&original).ok() != Some(e.source.clone()) {
                    return Err(invalid(
                        "source_conflict",
                        "Source metadata changed during capture",
                    ));
                }
                if e.kind == "session.opened" {
                    if e.profile.is_some() || e.epoch.is_none() {
                        return Err(invalid("invalid_event", "Invalid session open"));
                    }
                    fields(&e.payload, &["reason"])?;
                    reason(&e.payload)?;
                    if let Some(old) = &current_epoch {
                        close_epoch(&mut tx, capture, old, revision + 1).await?;
                    }
                    let epoch = e.epoch.as_deref().unwrap();
                    let maximum: Option<i64> = sqlx::query_scalar(
                        "SELECT max(CAST(epoch AS INTEGER)) FROM epochs WHERE capture_id=?",
                    )
                    .bind(capture)
                    .fetch_one(&mut *tx)
                    .await?;
                    if number(epoch, true)? <= maximum.unwrap_or(0) {
                        return Err(invalid("epoch_closed", "Epoch is not new"));
                    }
                    sqlx::query("INSERT INTO epochs VALUES(?,?,NULL,'identity_pending')")
                        .bind(capture)
                        .bind(epoch)
                        .execute(&mut *tx)
                        .await?;
                    sqlx::query("UPDATE live_checkpoints SET current_epoch=? WHERE capture_id=?")
                        .bind(epoch)
                        .bind(capture)
                        .execute(&mut *tx)
                        .await?;
                    actions.push(Action::Notice {
                        topics: vec!["active_profile".into(), "freshness".into()],
                    });
                } else if e.kind == "capture.closed" {
                    fields(&e.payload, &["reason"])?;
                    let r = reason(&e.payload)?;
                    if !matches!(r, "game_exited" | "user" | "deadline" | "source_end") {
                        return Err(invalid("invalid_event", "Unknown close reason"));
                    }
                    if e.epoch != current_epoch {
                        return Err(invalid(
                            "epoch_closed",
                            "Close epoch differs from current epoch",
                        ));
                    }
                    if let Some(old) = &current_epoch {
                        profile = bound_profile(&mut tx, capture, old).await?;
                        validate_profile(&mut tx, &e, profile.as_deref()).await?;
                        close_epoch(&mut tx, capture, old, revision + 1).await?;
                    } else if e.profile.is_some() {
                        return Err(invalid(
                            "identity_conflict",
                            "Closed session has no profile binding",
                        ));
                    }
                    sqlx::query("UPDATE live_checkpoints SET closed=1,current_epoch=NULL WHERE capture_id=?").bind(capture).execute(&mut *tx).await?;
                    sqlx::query("UPDATE captures SET terminal_status=CASE WHEN terminal_status='source_error' OR terminal_status LIKE 'decoder_failed%' THEN terminal_status ELSE ? END WHERE capture_id=?").bind(format!("stopped_{r}")).bind(capture).execute(&mut *tx).await?;
                    actions.push(Action::Notice {
                        topics: vec![
                            "captures".into(),
                            "active_profile".into(),
                            "freshness".into(),
                        ],
                    });
                } else {
                    if e.epoch != current_epoch {
                        return Err(invalid(
                            "epoch_closed",
                            "Event belongs to a closed or different epoch",
                        ));
                    }
                    if let Some(epoch) = &e.epoch {
                        profile = bound_profile(&mut tx, capture, epoch).await?;
                    }
                    if e.kind == "identity.confirmed" {
                        let epoch = e
                            .epoch
                            .as_deref()
                            .ok_or_else(|| invalid("invalid_event", "Identity requires epoch"))?;
                        if profile.is_some() {
                            return Err(invalid("identity_conflict", "Epoch is already bound"));
                        }
                        let key = e.profile.as_ref().ok_or_else(|| {
                            invalid("identity_pending", "Profile identity is required")
                        })?;
                        fields(&e.payload, &["evidence"])?;
                        let proof = &e.payload["evidence"];
                        if proof["login_verified"] != true
                            || !matches!(
                                proof["uid_source"].as_str(),
                                Some("login_packet" | "user_import")
                            )
                            || !matches!(
                                proof["region_source"].as_str(),
                                Some(
                                    "verified_protocol_field"
                                        | "user_confirmed_session"
                                        | "user_import"
                                        | "uid_prefix"
                                )
                            )
                            || (proof["region_source"] == "uid_prefix"
                                && crate::legacy::global_region_from_uid(&key.uid)
                                    != Some(key.server_region.as_str()))
                        {
                            return Err(invalid(
                                "identity_pending",
                                "Explicit identity evidence is required",
                            ));
                        }
                        let id = Uuid::new_v4().to_string();
                        sqlx::query("INSERT INTO profiles VALUES(?,'genshin','hk4e_global',?,?) ON CONFLICT(game_id,account_namespace,server_region,uid) DO NOTHING").bind(&id).bind(&key.server_region).bind(&key.uid).execute(&mut *tx).await?;
                        let id:String=sqlx::query_scalar("SELECT profile_id FROM profiles WHERE game_id='genshin' AND account_namespace='hk4e_global' AND server_region=? AND uid=?").bind(&key.server_region).bind(&key.uid).fetch_one(&mut *tx).await?;
                        if let Some(r)=sqlx::query("SELECT capture_id,epoch,expires_ms FROM live_leases WHERE profile_id=?").bind(&id).fetch_optional(&mut *tx).await?
                            && r.get::<i64,_>("expires_ms")>now && (r.get::<String,_>("capture_id")!=capture||r.get::<String,_>("epoch")!=epoch){return Err(invalid("lease_conflict","Profile already has a live collector"));}
                        sqlx::query("INSERT INTO live_leases VALUES(?,?,?,?) ON CONFLICT(profile_id) DO UPDATE SET capture_id=excluded.capture_id,epoch=excluded.epoch,expires_ms=excluded.expires_ms").bind(&id).bind(capture).bind(epoch).bind(now+15000).execute(&mut *tx).await?;
                        sqlx::query("UPDATE epochs SET profile_id=?,evidence=? WHERE capture_id=? AND epoch=?").bind(&id).bind(serde_json::to_string(proof).unwrap()).bind(capture).bind(epoch).execute(&mut *tx).await?;
                        sqlx::query("UPDATE domain_state SET freshness='awaiting_snapshot',reason='new_live_epoch',revision=? WHERE profile_id=?").bind(revision+1).bind(&id).execute(&mut *tx).await?;
                        profile = Some(id);
                        actions.push(Action::Notice {
                            topics: vec!["profiles".into(), "freshness".into()],
                        });
                    } else {
                        validate_profile(&mut tx, &e, profile.as_deref()).await?;
                        if let Some(id) = &profile {
                            let epoch = e.epoch.as_deref().unwrap();
                            let owner: Option<(String, String)> = sqlx::query_as(
                                "SELECT capture_id,epoch FROM live_leases WHERE profile_id=?",
                            )
                            .bind(id)
                            .fetch_optional(&mut *tx)
                            .await?;
                            if owner != Some((capture.into(), epoch.into())) {
                                return Err(invalid(
                                    "lease_conflict",
                                    "Profile lease belongs to another collector",
                                ));
                            }
                            sqlx::query("UPDATE live_leases SET expires_ms=? WHERE profile_id=?")
                                .bind(now + 15000)
                                .bind(id)
                                .execute(&mut *tx)
                                .await?;
                            if e.observed_ms()?.is_some_and(|t| t >= now - 8000) {
                                let restored=sqlx::query("UPDATE domain_state SET freshness='synchronized',reason='live',revision=? WHERE profile_id=? AND capture_id=? AND epoch=? AND freshness='stale' AND reason IN ('backend_restarted','lease_expired','source_lag')")
                                    .bind(revision+1).bind(id).bind(capture).bind(epoch).execute(&mut *tx).await?.rows_affected();
                                if restored > 0 {
                                    actions.push(Action::Notice {
                                        topics: vec!["freshness".into()],
                                    });
                                }
                            }
                        }
                        match e.kind.as_str() {
                            "state.snapshot" => {
                                let id = profile.as_deref().ok_or_else(|| {
                                    invalid("identity_pending", "Snapshot requires bound identity")
                                })?;
                                let s: Snapshot = decode(&e.payload)?;
                                value_format(&s.evidence)?;
                                if Uuid::parse_str(&s.snapshot_id).is_err()
                                    || s.collections.is_empty()
                                {
                                    return Err(invalid(
                                        "invalid_event",
                                        "Invalid snapshot identity",
                                    ));
                                }
                                let mut names = BTreeSet::new();
                                for c in s.collections {
                                    core::collection(&c.name).map_err(|_| {
                                        invalid("invalid_event", "Unknown collection")
                                    })?;
                                    if !names.insert(c.name.clone())
                                        || !matches!(
                                            c.scope.as_str(),
                                            "full" | "partial" | "unverified"
                                        )
                                        || c.live_fields
                                            .iter()
                                            .any(|f| !c.verified_fields.contains(f))
                                    {
                                        return Err(invalid("invalid_event", "Invalid coverage"));
                                    }
                                    let mut rows = BTreeMap::new();
                                    for r in c.entities {
                                        if c.name == "player_profile"
                                            && r.values["uid"].as_str()
                                                != e.profile.as_ref().map(|p| p.uid.as_str())
                                        {
                                            return Err(invalid(
                                                "identity_conflict",
                                                "Social profile UID does not match login",
                                            ));
                                        }
                                        core::validate_entity(&c.name, &r.entity_key, &r.values)
                                            .map_err(|_| {
                                                invalid("invalid_event", "Invalid snapshot entity")
                                            })?;
                                        if rows.insert(r.entity_key, r.values).is_some() {
                                            return Err(invalid(
                                                "invalid_event",
                                                "Duplicate snapshot entity",
                                            ));
                                        }
                                    }
                                    actions.push(Action::Snapshot {
                                        collection: c.name,
                                        scope: c.scope,
                                        rows,
                                    });
                                }
                                let _ = id;
                            }
                            "state.batch" => {
                                let id = profile.as_deref().ok_or_else(|| {
                                    invalid("identity_pending", "Changes require bound identity")
                                })?;
                                let b: Batch = decode(&e.payload)?;
                                value_format(&b.evidence)?;
                                if b.operations.is_empty() {
                                    return Err(invalid("invalid_event", "Empty operation batch"));
                                }
                                let mut seen = BTreeSet::new();
                                for op in b.operations {
                                    core::collection(&op.collection).map_err(|_| {
                                        invalid("invalid_event", "Unknown collection")
                                    })?;
                                    if !seen.insert((op.collection.clone(), op.entity_key.clone()))
                                        || !matches!(op.action.as_str(), "set" | "delete")
                                    {
                                        return Err(invalid(
                                            "invalid_event",
                                            "Invalid/duplicate operation",
                                        ));
                                    }
                                    let ready:Option<(String,String,String,String)>=sqlx::query_as("SELECT capture_id,epoch,freshness,reason FROM domain_state WHERE profile_id=? AND collection=?").bind(id).bind(&op.collection).fetch_optional(&mut *tx).await?;
                                    if !ready.is_some_and(|(c, p, f, r)| {
                                        c == capture
                                            && Some(p) == e.epoch
                                            && (matches!(f.as_str(), "synchronized" | "saved")
                                                || (f == "stale"
                                                    && matches!(
                                                        r.as_str(),
                                                        "backend_restarted"
                                                            | "lease_expired"
                                                            | "source_lag"
                                                    )))
                                    }) {
                                        rejected = true;
                                        break;
                                    }
                                    let old:Option<String>=sqlx::query_scalar("SELECT value_json FROM current_entities WHERE profile_id=? AND collection=? AND entity_key=? AND confirmed_in_baseline=1 AND capture_id=? AND epoch=?").bind(id).bind(&op.collection).bind(&op.entity_key).bind(capture).bind(e.epoch.as_deref()).fetch_optional(&mut *tx).await?;
                                    let old = old
                                        .map(|s| serde_json::from_str::<Value>(&s))
                                        .transpose()
                                        .map_err(|_| {
                                            invalid("storage_error", "Invalid stored entity")
                                        })?;
                                    match core::merge(
                                        &op.collection,
                                        &op.entity_key,
                                        old.as_ref(),
                                        &op.values,
                                        &op.expected,
                                        op.action == "delete",
                                    ) {
                                        Ok(value) => actions.push(Action::Set {
                                            collection: op.collection,
                                            key: op.entity_key,
                                            value,
                                        }),
                                        Err(_) => {
                                            rejected = true;
                                            break;
                                        }
                                    }
                                }
                                if rejected {
                                    actions = vec![Action::Invalidate {
                                        reason: "rejected_patch_requires_snapshot".into(),
                                        error: true,
                                    }];
                                }
                            }
                            "state.invalidated" => {
                                let i: Invalidation = decode(&e.payload)?;
                                if profile.is_none()
                                    || !i.requires_snapshot
                                    || i.collections.is_empty()
                                    || i.reason.is_empty()
                                {
                                    return Err(invalid("invalid_event", "Invalid invalidation"));
                                }
                                for c in &i.collections {
                                    core::collection(c).map_err(|_| {
                                        invalid("invalid_event", "Unknown collection")
                                    })?;
                                }
                                let error = i.reason == "source_error"
                                    || i.reason.starts_with("decoder_failed");
                                actions.push(Action::Invalidate {
                                    reason: i.reason,
                                    error,
                                });
                            }
                            "session.closed" => {
                                fields(&e.payload, &["reason"])?;
                                reason(&e.payload)?;
                                close_epoch(
                                    &mut tx,
                                    capture,
                                    e.epoch.as_deref().ok_or_else(|| {
                                        invalid("epoch_closed", "No session to close")
                                    })?,
                                    revision + 1,
                                )
                                .await?;
                                sqlx::query("UPDATE live_checkpoints SET current_epoch=NULL WHERE capture_id=?").bind(capture).execute(&mut *tx).await?;
                                actions.push(Action::Notice {
                                    topics: vec!["active_profile".into(), "freshness".into()],
                                });
                            }
                            "wish.observed" => {
                                crate::wish_live::validate(&e.payload).map_err(|_| {
                                    invalid("invalid_event", "Invalid wish observation")
                                })?;
                                if e.observed_ms()?.is_none() {
                                    return Err(invalid("invalid_event", "Wish needs observed_at"));
                                }
                                actions.push(Action::Notice {
                                    topics: vec!["wish_history".into()],
                                });
                            }
                            "diagnostic.notice" => {
                                fields(&e.payload, &["code", "details"])?;
                                if !e.payload["code"].is_string()
                                    || !e.payload["details"].is_object()
                                {
                                    return Err(invalid("invalid_event", "Invalid diagnostic"));
                                }
                                if e.payload["code"] == "wish_tracking_rejected" {
                                    sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE profile_id=?").bind(profile.as_deref()).execute(&mut *tx).await?;
                                    actions.push(Action::Notice {
                                        topics: vec!["wish_history".into()],
                                    });
                                }
                                if e.payload["code"] == "capture_error" {
                                    let state = e.payload["details"]["state"].as_str().ok_or_else(
                                        || invalid("invalid_event", "Missing capture error state"),
                                    )?;
                                    if state != "source_error"
                                        && !state.starts_with("decoder_failed")
                                    {
                                        return Err(invalid(
                                            "invalid_event",
                                            "Unsupported capture error state",
                                        ));
                                    }
                                    sqlx::query(
                                        "UPDATE captures SET terminal_status=? WHERE capture_id=?",
                                    )
                                    .bind(state)
                                    .bind(capture)
                                    .execute(&mut *tx)
                                    .await?;
                                }
                            }
                            _ => return Err(invalid("unsupported_event", "Unknown event kind")),
                        }
                    }
                }
            }
            let observed = e.observed_ms()?;
            if actions
                .iter()
                .any(|a| matches!(a, Action::Snapshot { .. } | Action::Set { .. }))
                && observed.is_none()
            {
                return Err(invalid("invalid_event", "Game facts need observed_at"));
            }
            let record = Record {
                event: raw,
                sequence: seq,
                epoch: e.epoch.clone().unwrap_or_else(|| "0".into()),
                observed_ms: observed,
                actions,
            };
            let before = revision;
            let mut applicability = BTreeMap::new();
            for a in &record.actions {
                if let Some(c) = a.collection() {
                    let same:i64=sqlx::query_scalar("SELECT count(*) FROM domain_state WHERE profile_id=? AND collection=? AND capture_id=? AND epoch=?").bind(profile.as_deref()).bind(c).bind(capture).bind(&record.epoch).fetch_one(&mut *tx).await?;
                    if same > 0 {
                        applicability.insert((record.epoch.clone(), c.into()), true);
                    }
                }
            }
            crate::apply_record(
                &mut tx,
                capture,
                &record,
                profile.as_deref(),
                &mut revision,
                &mut applicability,
                true,
            )
            .await
            .map_err(Error::Storage)?;
            if e.kind == "wish.observed" {
                crate::wish_live::observe(
                    &mut tx,
                    profile
                        .as_deref()
                        .ok_or_else(|| invalid("identity_pending", "Missing wish profile"))?,
                    capture,
                    &record.epoch,
                    seq,
                    record.observed_ms.unwrap(),
                    &e.payload,
                )
                .await
                .map_err(Error::Storage)?;
            }
            if e.kind == "state.invalidated" {
                sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE profile_id=?")
                    .bind(profile.as_deref())
                    .execute(&mut *tx)
                    .await?;
            }
            if e.observed_ms()?.is_some_and(|t| t < now - 8000) {
                let changed=sqlx::query("UPDATE domain_state SET freshness='stale',reason='source_lag',revision=? WHERE capture_id=? AND epoch=? AND freshness='synchronized'")
                    .bind(if revision==before{revision+1}else{revision}).bind(capture).bind(&record.epoch).execute(&mut *tx).await?.rows_affected();
                if changed > 0 {
                    sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE profile_id=?")
                        .bind(profile.as_deref())
                        .execute(&mut *tx)
                        .await?;
                    append_topics(
                        &mut tx,
                        capture,
                        seq,
                        profile.as_deref(),
                        &mut revision,
                        before,
                        &["freshness"],
                    )
                    .await?;
                }
            }
            if let Some(id) = &profile {
                let ready:i64=sqlx::query_scalar("SELECT count(*) FROM domain_state WHERE profile_id=? AND capture_id=? AND epoch=? AND freshness='synchronized'").bind(id).bind(capture).bind(&record.epoch).fetch_one(&mut *tx).await?;
                if ready > 0 && !matches!(e.kind.as_str(), "capture.closed" | "session.closed") {
                    let changed=sqlx::query("UPDATE active_context SET profile_id=?,capture_id=?,epoch=? WHERE singleton=1 AND (profile_id IS NOT ? OR capture_id IS NOT ? OR epoch IS NOT ?)").bind(id).bind(capture).bind(&record.epoch).bind(id).bind(capture).bind(&record.epoch).execute(&mut *tx).await?.rows_affected();
                    if changed > 0 {
                        append_topics(
                            &mut tx,
                            capture,
                            seq,
                            profile.as_deref(),
                            &mut revision,
                            before,
                            &["active_profile"],
                        )
                        .await?;
                    }
                } else if ready == 0 {
                    let changed=sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE capture_id=? AND epoch=?").bind(capture).bind(&record.epoch).execute(&mut *tx).await?.rows_affected();
                    if changed > 0 {
                        append_topics(
                            &mut tx,
                            capture,
                            seq,
                            profile.as_deref(),
                            &mut revision,
                            before,
                            &["active_profile"],
                        )
                        .await?;
                    }
                }
            }
            let ack = json!({"source_sequence":e.source_sequence,"revision":revision.to_string(),"outcome":if rejected{"rejected"}else{"accepted"},"resync_required":rejected,"duplicate":false});
            sqlx::query("INSERT INTO delivery_results VALUES(?,?,?,?)")
                .bind(capture)
                .bind(seq)
                .bind(hash)
                .bind(ack.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE events SET projection_result=? WHERE capture_id=? AND source_sequence=?",
            )
            .bind(if rejected { "rejected" } else { "applied" })
            .bind(capture)
            .bind(seq)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE live_checkpoints SET last_sequence=?,last_contact_ms=? WHERE capture_id=?",
            )
            .bind(seq)
            .bind(now)
            .bind(capture)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE captures SET event_count=? WHERE capture_id=?")
                .bind(seq)
                .bind(capture)
                .execute(&mut *tx)
                .await?;
            results.push(ack);
        }
        sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"capture_id":capture,"acked_sequence":results.last().map(|r|r["source_sequence"].clone()),"revision":revision.to_string(),"results":results}),
        )
    }

    /// Called only at backend startup, never by a CLI reader/importer opening the DB.
    pub async fn recover_live(&mut self) -> Result<()> {
        self.expire_context(true).await
    }
    pub async fn fail_delivery(&mut self, capture: &str) -> Result<Value> {
        if self.capture_blocked(capture).await? {
            return Ok(json!({"ignored":true}));
        }
        let mut tx = self.connection.begin().await?;
        let changed=sqlx::query("UPDATE live_checkpoints SET closed=1,current_epoch=NULL WHERE capture_id=? AND closed=0").bind(capture).execute(&mut *tx).await?.rows_affected();
        let mut revision: i64 =
            sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        if changed > 0 {
            sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE capture_id=?")
                .bind(capture)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE captures SET terminal_status='delivery_error' WHERE capture_id=?")
                .bind(capture)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE domain_state SET freshness='error',reason='delivery_gap',revision=? WHERE capture_id=?").bind(revision+1).bind(capture).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM live_leases WHERE capture_id=?")
                .bind(capture)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE capture_id=?").bind(capture).execute(&mut *tx).await?;
            let system = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))").bind(&system).bind(format!("system:{system}")).bind("{}").bind("{\"mode\":\"system\"}").bind("lifecycle-v1").execute(&mut *tx).await?;
            sqlx::query("INSERT INTO epochs VALUES(?,'0',NULL,'delivery_failure')")
                .bind(&system)
                .execute(&mut *tx)
                .await?;
            let record = Record {
                event: json!({"contract_version":1,"kind":"diagnostic.notice","payload":{"code":"delivery_gap","details":{"capture_id":capture}}}),
                sequence: 1,
                epoch: "0".into(),
                observed_ms: Some(now_ms()),
                actions: vec![Action::Notice {
                    topics: vec!["freshness".into(), "active_profile".into()],
                }],
            };
            crate::apply_record(
                &mut tx,
                &system,
                &record,
                None,
                &mut revision,
                &mut BTreeMap::new(),
                true,
            )
            .await
            .map_err(Error::Storage)?;
            sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
                .bind(revision)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(json!({"capture_id":capture,"closed":true,"revision":revision.to_string()}))
    }
    pub async fn expire_leases(&mut self) -> Result<()> {
        self.expire_context(false).await
    }
    async fn expire_context(&mut self, restart: bool) -> Result<()> {
        let mut tx = self.connection.begin().await?;
        let now = now_ms();
        let why = if restart {
            "backend_restarted"
        } else {
            "lease_expired"
        };
        let changed = if restart {
            sqlx::query("UPDATE domain_state SET freshness='stale',reason='backend_restarted' WHERE freshness IN ('synchronized','awaiting_snapshot')").execute(&mut *tx).await?.rows_affected()
        } else {
            sqlx::query("UPDATE domain_state SET freshness='stale',reason='lease_expired' WHERE freshness IN ('synchronized','awaiting_snapshot') AND profile_id IN (SELECT profile_id FROM live_leases WHERE expires_ms<?)").bind(now).execute(&mut *tx).await?.rows_affected()
        };
        if restart {
            sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE status='synced'")
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE wish_sync_state SET status='failed' WHERE profile_id IN (SELECT profile_id FROM live_leases WHERE expires_ms<?)").bind(now).execute(&mut *tx).await?;
        }
        let active = if restart {
            sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE profile_id IS NOT NULL").execute(&mut *tx).await?.rows_affected()
        } else {
            sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE profile_id IN (SELECT profile_id FROM live_leases WHERE expires_ms<?)").bind(now).execute(&mut *tx).await?.rows_affected()
        };
        if changed + active > 0 {
            let capture = Uuid::new_v4().to_string();
            let mut revision: i64 =
                sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                    .fetch_one(&mut *tx)
                    .await?;
            sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))").bind(&capture).bind(format!("system:{capture}")).bind("{}").bind("{\"mode\":\"system\"}").bind("lifecycle-v1").execute(&mut *tx).await?;
            sqlx::query("INSERT INTO epochs VALUES(?,'0',NULL,'backend_lifecycle')")
                .bind(&capture)
                .execute(&mut *tx)
                .await?;
            let record = Record {
                event: json!({"contract_version":1,"kind":"diagnostic.notice","payload":{"code":why,"details":{} }}),
                sequence: 1,
                epoch: "0".into(),
                observed_ms: Some(now),
                actions: vec![Action::Notice {
                    topics: vec!["freshness".into(), "active_profile".into()],
                }],
            };
            crate::apply_record(
                &mut tx,
                &capture,
                &record,
                None,
                &mut revision,
                &mut BTreeMap::new(),
                true,
            )
            .await
            .map_err(Error::Storage)?;
            sqlx::query("UPDATE domain_state SET revision=? WHERE reason=? AND freshness='stale'")
                .bind(revision)
                .bind(why)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
                .bind(revision)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

async fn bound_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture: &str,
    epoch: &str,
) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT profile_id FROM epochs WHERE capture_id=? AND epoch=?")
            .bind(capture)
            .bind(epoch)
            .fetch_optional(&mut **tx)
            .await?
            .flatten(),
    )
}
async fn validate_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    e: &Envelope,
    id: Option<&str>,
) -> Result<()> {
    match (id, &e.profile) {
        (None, None) => Ok(()),
        (Some(id), Some(p)) => {
            let stored: Option<(String, String)> =
                sqlx::query_as("SELECT server_region,uid FROM profiles WHERE profile_id=?")
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await?;
            if stored == Some((p.server_region.clone(), p.uid.clone())) {
                Ok(())
            } else {
                Err(invalid(
                    "identity_conflict",
                    "Event profile differs from epoch binding",
                ))
            }
        }
        _ => Err(invalid(
            "identity_conflict",
            "Event profile differs from epoch binding",
        )),
    }
}
async fn close_epoch(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture: &str,
    epoch: &str,
    revision: i64,
) -> Result<()> {
    sqlx::query("UPDATE domain_state SET freshness='saved',reason='session_closed',revision=? WHERE profile_id IN (SELECT profile_id FROM live_leases WHERE capture_id=? AND epoch=?) AND freshness IN ('synchronized','awaiting_snapshot')").bind(revision).bind(capture).bind(epoch).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM live_leases WHERE capture_id=? AND epoch=?")
        .bind(capture)
        .bind(epoch)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE capture_id=? AND epoch=?").bind(capture).bind(epoch).execute(&mut **tx).await?;
    Ok(())
}
async fn append_topics(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture: &str,
    seq: i64,
    profile: Option<&str>,
    revision: &mut i64,
    before: i64,
    topics: &[&str],
) -> Result<()> {
    if *revision == before {
        *revision = revision
            .checked_add(1)
            .ok_or_else(|| invalid("storage_error", "Revision overflow"))?;
        sqlx::query("INSERT INTO change_log VALUES(?,?,?,?,?)")
            .bind(*revision)
            .bind(capture)
            .bind(seq)
            .bind(profile)
            .bind(json!(topics).to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE events SET revision=? WHERE capture_id=? AND source_sequence=?")
            .bind(*revision)
            .bind(capture)
            .bind(seq)
            .execute(&mut **tx)
            .await?;
    } else {
        let current: String =
            sqlx::query_scalar("SELECT topics_json FROM change_log WHERE revision=?")
                .bind(*revision)
                .fetch_one(&mut **tx)
                .await?;
        let mut list: Vec<String> = serde_json::from_str(&current).unwrap();
        for t in topics {
            if !list.iter().any(|s| s == t) {
                list.push(t.to_string());
            }
        }
        sqlx::query("UPDATE change_log SET topics_json=? WHERE revision=?")
            .bind(json!(list).to_string())
            .bind(*revision)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
