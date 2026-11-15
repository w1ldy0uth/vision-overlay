use crate::{
    Store,
    ingest::{Envelope, Error, ProfileKey, Source},
    reader::Reader,
};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[tokio::test]
async fn public_game_session_changes_on_same_account_login_and_clears_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let mut s = Store::open(&path).await.unwrap();
    let mut p = Producer::new();
    let start = p.start(profile());
    s.ingest(&p.id, start).await.unwrap();
    let reader = Reader::open(&path).await.unwrap();
    let before = reader.state().await.unwrap();
    assert_eq!(before["active_game_session"], format!("{}:1", p.id));
    let mut login = p.login("2", profile());
    login.push(p.snapshot(10));
    s.ingest(&p.id, login).await.unwrap();
    let after = reader.state().await.unwrap();
    assert_eq!(
        before["active_game_profile_id"],
        after["active_game_profile_id"]
    );
    assert_eq!(after["active_game_session"], format!("{}:2", p.id));
    let close = p.event("capture.closed", json!({"reason":"game_exited"}));
    s.ingest(&p.id, vec![close]).await.unwrap();
    assert!(reader.state().await.unwrap()["active_game_session"].is_null());
    reader.close().await;
}

fn wish_archive(records: Vec<Value>) -> Value {
    json!({"schema_version":1,"source":"hoyoverse_wish_api","game_id":"genshin","account_namespace":"hk4e_global","uid":"123456789","server_region":"os_euro","fetched_at":OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),"coverage":{"all_available_pages_fetched":true},"region_time_zone":1,"streams":[{"gacha_type":"100","pages":1},{"gacha_type":"200","pages":1},{"gacha_type":"301","pages":1},{"gacha_type":"302","pages":1},{"gacha_type":"500","pages":1}],"records":records})
}
fn wish_api(id: usize, time: &str) -> Value {
    json!({"id":id.to_string(),"uid":"123456789","gacha_type":"200","item_id":"","count":"1","time":time,"name":"Клятва стрелка","item_type":"Оружие","rank_type":"3","lang":"ru-ru"})
}
#[tokio::test]
async fn captured_wishes_are_idempotent_and_api_prefixes_reconcile_ten_identical_rewards() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("wishes.db")).await.unwrap();
    let mut p = Producer::new();
    let mut start = p.start(profile());
    start[0]["payload"]["configuration"]["wish_tracking"] = json!(true);
    s.ingest(&p.id, start).await.unwrap();
    let profile_id = id(&mut s, "123456789", "os_euro").await;
    s.import_wishes(&profile_id, wish_archive(vec![]))
        .await
        .unwrap();
    let context = s.wish_context().await.unwrap().unwrap();
    s.set_wish_sync(&context, "synced", Some(crate::ingest::now_ms() - 1000))
        .await
        .unwrap();
    let event=p.event("wish.observed",json!({"gacha_type":"200","gacha_id":"3983","results":vec![json!({"item_id":15302,"count":1});10]}));
    s.ingest(&p.id, vec![event.clone()]).await.unwrap();
    let ack = s.ingest(&p.id, vec![event.clone()]).await.unwrap();
    assert_eq!(ack["results"][0]["duplicate"], true);
    let view = s.wishes(&profile_id).await.unwrap();
    assert_eq!(view["live_enabled"], true);
    assert_eq!(view["live_records"].as_array().unwrap().len(), 10);
    let portable = dir.path().join("pending.visionbackup");
    s.backup(
        &portable,
        crate::backup::Settings::default(),
        json!({"schema_version":1}),
    )
    .await
    .unwrap();
    let validated = crate::backup::validate(&portable).await.unwrap();
    let mut restored = Store::open(&dir.path().join("restored.db")).await.unwrap();
    restored
        .restore(&validated, &dir.path().join("rollback.visionbackup"), &[])
        .await
        .unwrap();
    assert!(!restored.wish_live_enabled(&profile_id).await.unwrap());
    let preserved: i64 = sqlx::query_scalar("SELECT count(*) FROM wish_live")
        .fetch_one(&mut restored.connection)
        .await
        .unwrap();
    assert_eq!(preserved, 10);
    let date = OffsetDateTime::parse(event["observed_at"].as_str().unwrap(), &Rfc3339).unwrap()
        + time::Duration::hours(1);
    let wall = date
        .format(
            &time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day] [hour]:[minute]:[second]",
            )
            .unwrap(),
        )
        .unwrap();
    let catalog = json!({"names":{"15302":"Клятва стрелка"}});
    // An API lag is not a new result; observations survive an empty import.
    s.import_wishes_with_catalog(&profile_id, wish_archive(vec![]), Some(&catalog))
        .await
        .unwrap();
    assert_eq!(s.wish_live_rows(&profile_id).await.unwrap().len(), 10);
    s.import_wishes_with_catalog(
        &profile_id,
        wish_archive((1..=3).map(|n| wish_api(n, &wall)).collect()),
        Some(&catalog),
    )
    .await
    .unwrap();
    assert_eq!(s.wish_live_rows(&profile_id).await.unwrap().len(), 7);
    s.import_wishes_with_catalog(
        &profile_id,
        wish_archive((1..=10).map(|n| wish_api(n, &wall)).collect()),
        Some(&catalog),
    )
    .await
    .unwrap();
    let view = s.wishes(&profile_id).await.unwrap();
    assert_eq!(view["records"].as_array().unwrap().len(), 10);
    assert_eq!(view["live_records"], json!([]));
    assert_eq!(view["hidden_live_count"], 0);
    assert_eq!(view["status"], "synced");
    // Another account never receives these observations or their imported rows.
    let mut other = profile();
    other.uid = "123456788".into();
    let login = p.login("2", other);
    s.ingest(&p.id, login).await.unwrap();
    let other = id(&mut s, "123456788", "os_euro").await;
    assert_eq!(s.wishes(&other).await.unwrap()["live_records"], json!([]));
    assert_eq!(s.wishes(&other).await.unwrap()["records"], json!([]));
}

#[tokio::test]
async fn previous_session_pending_wish_blocks_new_live_until_api_catches_up() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("pending.db")).await.unwrap();
    let mut p = Producer::new();
    let mut start = p.start(profile());
    start[0]["payload"]["configuration"]["wish_tracking"] = json!(true);
    s.ingest(&p.id, start).await.unwrap();
    let profile_id = id(&mut s, "123456789", "os_euro").await;
    let c = s.wish_context().await.unwrap().unwrap();
    s.set_wish_sync(&c, "synced", Some(crate::ingest::now_ms() - 1000))
        .await
        .unwrap();
    let event = p.event(
        "wish.observed",
        json!({"gacha_type":"200","gacha_id":"3983","results":[{"item_id":15302,"count":1}]}),
    );
    s.ingest(&p.id, vec![event]).await.unwrap();
    assert_eq!(s.wish_live_rows(&profile_id).await.unwrap().len(), 1);
    let mut next = p.login("2", profile());
    next.push(p.snapshot(10));
    s.ingest(&p.id, next).await.unwrap();
    let c = s.wish_context().await.unwrap().unwrap();
    s.set_wish_sync(&c, "synced", Some(crate::ingest::now_ms()))
        .await
        .unwrap();
    let view = s.wishes(&profile_id).await.unwrap();
    assert_eq!(view["live_enabled"], false);
    assert_eq!(view["status"], "needs_sync");
    assert_eq!(view["hidden_live_count"], 1);
}

#[tokio::test]
async fn unsynchronized_wishes_and_delivery_loss_cannot_resume_as_live() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("w.db")).await.unwrap();
    let mut p = Producer::new();
    let mut start = p.start(profile());
    start[0]["payload"]["configuration"]["wish_tracking"] = json!(true);
    s.ingest(&p.id, start).await.unwrap();
    let id = id(&mut s, "123456789", "os_euro").await;
    let event = p.event(
        "wish.observed",
        json!({"gacha_type":"301","gacha_id":"1000","results":[{"item_id":15302,"count":1}]}),
    );
    s.ingest(&p.id, vec![event]).await.unwrap();
    let context = s.wish_context().await.unwrap().unwrap();
    s.set_wish_sync(&context, "synced", Some(crate::ingest::now_ms()))
        .await
        .unwrap();
    assert_eq!(s.wishes(&id).await.unwrap()["live_records"], json!([]));
    assert_eq!(s.wishes(&id).await.unwrap()["hidden_live_count"], 1);
    assert_eq!(
        s.wish_sync_state(&id).await.unwrap()["status"],
        "needs_sync"
    );
    s.fail_delivery(&p.id).await.unwrap();
    assert!(!s.wish_live_enabled(&id).await.unwrap());
    // Malformed batches roll back the whole native envelope, including sequence.
    let mut fresh = Producer::new();
    let start = fresh.start(profile());
    s.ingest(&fresh.id, start).await.unwrap();
    let bad = fresh.event(
        "wish.observed",
        json!({"gacha_type":"200","gacha_id":"1000","results":[]}),
    );
    assert!(s.ingest(&fresh.id, vec![bad]).await.is_err());
}

fn profile() -> ProfileKey {
    ProfileKey {
        game_id: "genshin".into(),
        account_namespace: "hk4e_global".into(),
        server_region: "os_euro".into(),
        uid: "123456789".into(),
    }
}
struct Producer {
    id: String,
    n: u64,
    epoch: Option<String>,
    profile: Option<ProfileKey>,
}
impl Producer {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            n: 0,
            epoch: None,
            profile: None,
        }
    }
    fn event(&mut self, kind: &str, payload: Value) -> Value {
        self.n += 1;
        serde_json::to_value(Envelope {
            contract_version: 1,
            capture_id: self.id.clone(),
            source_sequence: self.n.to_string(),
            epoch: self.epoch.clone(),
            profile: self.profile.clone(),
            source: Source {
                producer: "test".into(),
                producer_version: "1".into(),
                protocol_version: "7.2".into(),
                adapter_revision: "test-v1".into(),
                mode: "live".into(),
            },
            observed_at: Some(OffsetDateTime::now_utc().format(&Rfc3339).unwrap()),
            source_elapsed_ms: Some((self.n * 100).to_string()),
            kind: kind.into(),
            payload,
        })
        .unwrap()
    }
    fn start(&mut self, p: ProfileKey) -> Vec<Value> {
        let mut out = vec![self.event("capture.opened", json!({"configuration":{}}))];
        out.extend(self.login("1", p));
        out.push(self.snapshot(10));
        out
    }
    fn login(&mut self, epoch: &str, p: ProfileKey) -> Vec<Value> {
        self.epoch = Some(epoch.into());
        self.profile = None;
        let mut out = vec![self.event("session.opened", json!({"reason":"login"}))];
        self.profile = Some(p);
        out.push(self.event("identity.confirmed",json!({"evidence":{"uid_source":"login_packet","region_source":"user_import","login_verified":true}})));
        out
    }
    fn snapshot(&mut self, count: u32) -> Value {
        self.event("state.snapshot",json!({"snapshot_id":Uuid::new_v4().to_string(),"collections":[{"name":"material_stacks","scope":"full","verified_fields":["count","item_id"],"live_fields":["count"],"entities":[{"entity_key":"18446744073709551615","values":{"item_id":100002,"count":count}}]}],"evidence":{}}))
    }
    fn patch(&mut self, old: u32, new: u32) -> Value {
        self.event("state.batch",json!({"operations":[{"collection":"material_stacks","entity_key":"18446744073709551615","action":"set","values":{"count":new},"expected":{"count":old}}],"evidence":{}}))
    }
}
async fn id(s: &mut Store, uid: &str, region: &str) -> String {
    s.profiles()
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["uid"] == uid && p["server_region"] == region)
        .unwrap()["profile_id"]
        .as_str()
        .unwrap()
        .into()
}
async fn count(s: &mut Store, id: &str) -> Value {
    s.state(id).await.unwrap()["material_totals"][0]["count"].clone()
}

#[tokio::test]
async fn prefix_evidence_requires_matching_global_region_and_verified_login() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("prefix.db")).await.unwrap();
    for (uid, region, verified, accepted) in [
        ("600000001", "os_usa", true, true),
        ("1800000001", "os_asia", true, true),
        ("700000001", "os_usa", true, false),
        ("800000001", "os_asia", false, false),
        ("100000001", "os_asia", true, false),
    ] {
        let mut producer = Producer::new();
        let mut p = profile();
        p.uid = uid.into();
        p.server_region = region.into();
        let mut events = producer.start(p);
        events[2]["payload"]["evidence"]["region_source"] = json!("uid_prefix");
        events[2]["payload"]["evidence"]["login_verified"] = json!(verified);
        let result = store.ingest(&producer.id, events).await;
        assert_eq!(result.is_ok(), accepted, "{uid} {region}");
    }
    assert_eq!(store.profiles().await.unwrap().as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn native_delivery_is_atomic_idempotent_and_conflicting_keys_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    let ack = s.ingest(&p.id, initial.clone()).await.unwrap();
    assert_eq!(ack["acked_sequence"], "4");
    let id = id(&mut s, "123456789", "os_euro").await;
    assert_eq!(count(&mut s, &id).await, "10");
    assert_eq!(s.state(&id).await.unwrap()["mode"], "live");
    let repeated = s.ingest(&p.id, initial.clone()).await.unwrap();
    assert_eq!(repeated["revision"], ack["revision"]);
    assert!(
        repeated["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["duplicate"] == true)
    );
    let mut conflict = initial[3].clone();
    conflict["payload"]["collections"][0]["entities"][0]["values"]["count"] = json!(99);
    assert!(matches!(
        s.ingest(&p.id, vec![conflict]).await,
        Err(Error::Invalid {
            code: "event_conflict",
            ..
        })
    ));
    assert_eq!(count(&mut s, &id).await, "10");
    s.close().await.unwrap();
}

#[tokio::test]
async fn gaps_and_late_old_epochs_do_not_advance_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    s.ingest(&p.id, initial).await.unwrap();
    let e = p.patch(10, 11);
    let mut gap = e.clone();
    gap["source_sequence"] = json!("6");
    assert!(
        matches!(s.ingest(&p.id,vec![gap]).await,Err(Error::Invalid{code:"sequence_gap",expected:Some(ref v),..})if v=="5")
    );
    s.ingest(&p.id, vec![e]).await.unwrap();
    let login = p.login("2", profile());
    s.ingest(&p.id, login).await.unwrap();
    let mut old = p.patch(11, 12);
    old["epoch"] = json!("1");
    assert!(matches!(
        s.ingest(&p.id, vec![old]).await,
        Err(Error::Invalid {
            code: "epoch_closed",
            ..
        })
    ));
    s.close().await.unwrap();
}

#[tokio::test]
async fn rejected_batch_applies_no_partial_values_and_requires_a_fresh_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    s.ingest(&p.id, initial).await.unwrap();
    let id = id(&mut s, "123456789", "os_euro").await;
    let mut bad = p.patch(10, 11);
    bad["payload"]["operations"].as_array_mut().unwrap().push(json!({"collection":"material_stacks","entity_key":"42","action":"set","values":{"item_id":100002,"count":1},"expected":{"count":99}}));
    let ack = s.ingest(&p.id, vec![bad]).await.unwrap();
    assert_eq!(ack["results"][0]["resync_required"], true);
    assert_eq!(count(&mut s, &id).await, "10");
    let blocked = p.patch(10, 12);
    assert_eq!(
        s.ingest(&p.id, vec![blocked]).await.unwrap()["results"][0]["outcome"],
        "rejected"
    );
    let snap = p.snapshot(20);
    s.ingest(&p.id, vec![snap]).await.unwrap();
    let good = p.patch(20, 21);
    s.ingest(&p.id, vec![good]).await.unwrap();
    assert_eq!(count(&mut s, &id).await, "21");
    s.close().await.unwrap();
}

#[tokio::test]
async fn profile_forgery_and_competing_collector_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    s.ingest(&p.id, initial).await.unwrap();
    let mut e = p.patch(10, 11);
    e["profile"]["uid"] = json!("987654321");
    assert!(matches!(
        s.ingest(&p.id, vec![e]).await,
        Err(Error::Invalid {
            code: "identity_conflict",
            ..
        })
    ));
    let mut other = Producer::new();
    let initial = other.start(profile());
    assert!(matches!(
        s.ingest(&other.id, initial).await,
        Err(Error::Invalid {
            code: "lease_conflict",
            ..
        })
    ));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM captures WHERE capture_id=?")
        .bind(&other.id)
        .fetch_one(&mut s.connection)
        .await
        .unwrap();
    assert_eq!(n, 0);
    s.close().await.unwrap();
}

#[tokio::test]
async fn known_source_resumes_after_restart_but_heartbeat_does_not_repair_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut s = Store::open(&path).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    s.ingest(&p.id, initial.clone()).await.unwrap();
    let id = id(&mut s, "123456789", "os_euro").await;
    s.close().await.unwrap();
    let mut s = Store::open(&path).await.unwrap();
    s.recover_live().await.unwrap();
    assert_eq!(
        s.state(&id).await.unwrap()["domains"][0]["freshness"],
        "stale"
    );
    s.ingest(&p.id, initial).await.unwrap();
    let heartbeat = p.event(
        "diagnostic.notice",
        json!({"code":"collector_heartbeat","details":{}}),
    );
    s.ingest(&p.id, vec![heartbeat]).await.unwrap();
    assert_eq!(
        s.state(&id).await.unwrap()["domains"][0]["freshness"],
        "synchronized"
    );
    let patch = p.patch(10, 11);
    s.ingest(&p.id, vec![patch]).await.unwrap();
    let gap = p.event(
        "state.invalidated",
        json!({"collections":["material_stacks"],"reason":"capture_loss","requires_snapshot":true}),
    );
    s.ingest(&p.id, vec![gap]).await.unwrap();
    let heartbeat = p.event(
        "diagnostic.notice",
        json!({"code":"collector_heartbeat","details":{}}),
    );
    s.ingest(&p.id, vec![heartbeat]).await.unwrap();
    assert_eq!(
        s.state(&id).await.unwrap()["domains"][0]["freshness"],
        "stale"
    );
    s.close().await.unwrap();
}

#[tokio::test]
async fn switches_servers_and_returns_without_crossing_entities_then_closes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut s = Store::open(&path).await.unwrap();
    let mut p = Producer::new();
    let a = profile();
    let mut b = a.clone();
    b.server_region = "os_asia".into();
    let mut events = p.start(a.clone());
    events.extend(p.login("2", b));
    events.push(p.snapshot(99));
    events.extend(p.login("3", a));
    events.push(p.snapshot(7));
    s.ingest(&p.id, events).await.unwrap();
    let a = id(&mut s, "123456789", "os_euro").await;
    let b = id(&mut s, "123456789", "os_asia").await;
    assert_eq!(count(&mut s, &a).await, "7");
    assert_eq!(count(&mut s, &b).await, "99");
    let r = Reader::open(&path).await.unwrap();
    assert_eq!(r.state().await.unwrap()["active_game_profile_id"], a);
    let close = p.event("capture.closed", json!({"reason":"game_exited"}));
    s.ingest(&p.id, vec![close]).await.unwrap();
    assert!(r.state().await.unwrap()["active_game_profile_id"].is_null());
    assert_eq!(
        s.state(&a).await.unwrap()["domains"][0]["freshness"],
        "saved"
    );
    r.close().await;
    s.close().await.unwrap();
}

#[tokio::test]
async fn sql_failure_rolls_back_the_ack_and_retries_the_same_event_safely() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut p = Producer::new();
    let initial = p.start(profile());
    s.ingest(&p.id, initial).await.unwrap();
    let id = id(&mut s, "123456789", "os_euro").await;
    let before = s.state(&id).await.unwrap();
    let e = p.patch(10, 11);
    sqlx::raw_sql("CREATE TRIGGER fail_ack BEFORE INSERT ON delivery_results BEGIN SELECT RAISE(ABORT,'injected crash before ACK'); END;").execute(&mut s.connection).await.unwrap();
    assert!(s.ingest(&p.id, vec![e.clone()]).await.is_err());
    assert_eq!(before, s.state(&id).await.unwrap());
    sqlx::query("DROP TRIGGER fail_ack")
        .execute(&mut s.connection)
        .await
        .unwrap();
    assert_eq!(
        s.ingest(&p.id, vec![e]).await.unwrap()["acked_sequence"],
        "5"
    );
    assert_eq!(count(&mut s, &id).await, "11");
    s.close().await.unwrap();
}

#[tokio::test]
async fn expired_lease_allows_takeover_and_delivery_failure_quarantines_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut s = Store::open(&path).await.unwrap();
    let mut old = Producer::new();
    let initial = old.start(profile());
    s.ingest(&old.id, initial).await.unwrap();
    let profile_id = id(&mut s, "123456789", "os_euro").await;
    sqlx::query("UPDATE live_leases SET expires_ms=0")
        .execute(&mut s.connection)
        .await
        .unwrap();
    s.expire_leases().await.unwrap();
    let reader = Reader::open(&path).await.unwrap();
    assert!(reader.state().await.unwrap()["active_game_profile_id"].is_null());
    assert_eq!(
        s.state(&profile_id).await.unwrap()["domains"][0]["reason"],
        "lease_expired"
    );
    let mut new = Producer::new();
    let initial = new.start(profile());
    s.ingest(&new.id, initial).await.unwrap();
    let patch = old.patch(10, 11);
    assert!(s.ingest(&old.id, vec![patch]).await.is_err());
    assert_eq!(count(&mut s, &profile_id).await, "10");
    let ack = s.fail_delivery(&new.id).await.unwrap();
    assert_eq!(s.fail_delivery(&new.id).await.unwrap(), ack);
    assert_eq!(
        s.state(&profile_id).await.unwrap()["domains"][0]["freshness"],
        "error"
    );
    assert!(reader.state().await.unwrap()["active_game_profile_id"].is_null());
    let patch = new.patch(10, 12);
    assert!(s.ingest(&new.id, vec![patch]).await.is_err());
    reader.close().await;
    s.close().await.unwrap();
}
