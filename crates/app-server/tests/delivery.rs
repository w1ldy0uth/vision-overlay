use app_server::{
    ApiState,
    relay::{self, Normalizer},
    router,
};
use app_store::{Store, reader::Reader};
use futures::StreamExt;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};

fn regions() -> BTreeMap<String, String> {
    BTreeMap::from([("123456789".into(), "os_euro".into())])
}

#[tokio::test]
async fn wish_reply_is_a_separate_durable_notification_without_mutating_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("wish.db")).await.unwrap();
    let capture = uuid::Uuid::new_v4().to_string();
    let mut n = Normalizer::new(&capture).unwrap();
    let mut raw = rows()[..3].to_vec();
    raw[0]["data"]["wish_tracking"] = json!(true);
    for row in raw {
        let events = n.feed(&row, &regions()).unwrap();
        store.ingest(&capture, events).await.unwrap();
    }
    let profiles = store.profiles().await.unwrap();
    let profile = profiles[0]["profile_id"].as_str().unwrap();
    let before = store.state(profile).await.unwrap()["material_totals"].clone();
    let row = json!({"schema_version":1,"sequence":4,"session":"1","event":"wish_completed","observed_unix_ms":(time::OffsetDateTime::now_utc().unix_timestamp_nanos()/1_000_000).to_string(),"data":{"gacha_type":"200","gacha_id":"3983","results":[{"item_id":15302,"count":1}]}});
    let events = n.feed(&row, &regions()).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "wish.observed");
    store.ingest(&capture, events).await.unwrap();
    assert_eq!(store.wishes(profile).await.unwrap()["hidden_live_count"], 1);
    assert_eq!(
        store.state(profile).await.unwrap()["material_totals"],
        before
    );
    let reader = Reader::open(&dir.path().join("wish.db")).await.unwrap();
    assert!(
        serde_json::to_string(&reader.changes(0, 100).await.unwrap())
            .unwrap()
            .contains("wish_history")
    );
    reader.close().await;
}

#[tokio::test]
async fn traveler_sets_are_atomic_persist_across_capture_and_are_profile_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traveler.db");
    let mut store = Store::open(&path).await.unwrap();
    let cryo = json!({"skill_depot_id":505,"skill_levels":{"10127":6},"talent_ids":[81,82,83,84,85,86],"extra_skill_levels":{"832":3,"839":3}});
    let wind = json!({"skill_depot_id":504,"skill_levels":{"10067":4},"talent_ids":[71,72,73,74,75,76],"extra_skill_levels":{"732":3,"739":3}});
    let start = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    for run in 0..3 {
        let capture = uuid::Uuid::new_v4().to_string();
        let mut normalizer = Normalizer::new(&capture).unwrap();
        let uid = if run == 2 { "722222222" } else { "711111111" };
        let guid = if run == 2 { "43" } else { "42" };
        let mut character = cryo.clone();
        character["guid"] = json!(guid);
        character["avatar_id"] = json!(10000005);
        character["props"] = json!({"4001":{"ival":"90"},"1002":{"ival":"6"},"1001":{"ival":"0"}});
        character["equip_guids"] = json!(["100"]);
        let mut events = vec![
            ("source_config", json!({"mode":"live"})),
            ("login_identity", json!({"uid":uid})),
            (
                "character_snapshot",
                json!({"profiles":[character,{"guid":"99","avatar_id":10000025,"skill_depot_id":2501}]}),
            ),
            (
                "traveler_element_baseline",
                json!({"guid":guid,"avatar_id":10000005,"current":cryo}),
            ),
        ];
        if run == 0 {
            events.push(("traveler_element_changed",json!({"guid":guid,"avatar_id":10000005,"previous":cryo,"current":wind,"command_id":25666})));
            events.push(("traveler_element_changed",json!({"guid":guid,"avatar_id":10000005,"previous":wind,"current":cryo,"command_id":25666})));
        }
        events.push(("status", json!({"state":"stopped_game_exited"})));
        for (i, (kind, data)) in events.into_iter().enumerate() {
            let raw = json!({"schema_version":1,"sequence":i+1,"session":if i==0{"0"}else{"1"},"event":kind,"data":data,"observed_unix_ms":(start+i as i128+run*100).to_string()});
            let normalized = normalizer.feed(&raw, &BTreeMap::new()).unwrap();
            if kind == "traveler_element_changed" {
                assert_eq!(normalized.len(), 1);
                assert_eq!(normalized[0]["kind"], "state.snapshot");
                assert_eq!(
                    normalized[0]["payload"]["collections"]
                        .as_array()
                        .unwrap()
                        .len(),
                    2
                );
            }
            if !normalized.is_empty() {
                store.ingest(&capture, normalized.clone()).await.unwrap();
            }
            if run == 0 && i == 5 {
                let profiles = store.profiles().await.unwrap();
                let id = profiles[0]["profile_id"].as_str().unwrap();
                let before = store.state(id).await.unwrap();
                store.ingest(&capture, normalized).await.unwrap();
                assert_eq!(store.state(id).await.unwrap(), before);
            }
        }
    }
    let profiles = store.profiles().await.unwrap();
    let id = profiles
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["uid"] == "711111111")
        .unwrap()["profile_id"]
        .as_str()
        .unwrap();
    let state = store.state(id).await.unwrap();
    let sets = state["collections"]["traveler_depots"].as_array().unwrap();
    assert_eq!(sets.len(), 2);
    assert_eq!(
        sets.iter()
            .find(|r| r["values"]["skill_depot_id"] == 504)
            .unwrap()["values"]["skill_levels"]["10067"],
        4
    );
    assert_eq!(
        state["collections"]["characters"].as_array().unwrap().len(),
        2
    );
    let hero = &state["collections"]["characters"][0]["values"];
    assert_eq!(hero["skill_depot_id"], 505);
    assert_eq!(hero["props"]["4001"]["ival"], "90");
    assert_eq!(hero["equip_guids"], json!(["100"]));
    let other = profiles
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["uid"] == "722222222")
        .unwrap()["profile_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        store.state(other).await.unwrap()["collections"]["traveler_depots"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(store);
    let reader = Reader::open(&path).await.unwrap();
    let restored = reader.profile(id).await.unwrap();
    assert_eq!(
        restored["collections"]["traveler_depots"],
        state["collections"]["traveler_depots"]
    );
    reader.close().await;
}
#[tokio::test]
async fn fight_properties_are_delivered_persisted_and_resolved_without_replacing_other_stats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fight.db");
    let mut store = Store::open(&path).await.unwrap();
    let capture = uuid::Uuid::new_v4().to_string();
    let mut normalizer = Normalizer::new(&capture).unwrap();
    let mut input = rows()[..3].to_vec();
    input.push(json!({"schema_version":1,"session":"1","sequence":4,"event":"character_snapshot","data":{"profiles":[{"guid":"42","avatar_id":10000137,"fight_props":{"2000":3700.0,"2001":464.0,"2002":211.0,"40":0.2}}]}}));
    input.push(json!({"schema_version":1,"session":"1","sequence":5,"event":"fight_properties_changed","data":{"guid":"42","avatar_id":10000137,"previous":{"2001":464.0,"40":0.2},"current":{"2001":787.0,"40":0.0}}}));
    let mut applied = vec![];
    for mut raw in input {
        raw["observed_unix_ms"] = json!(
            time::OffsetDateTime::now_utc()
                .unix_timestamp_nanos()
                .checked_div(1_000_000)
                .unwrap()
                .to_string()
        );
        applied = normalizer.feed(&raw, &regions()).unwrap();
        if !applied.is_empty() {
            store.ingest(&capture, applied.clone()).await.unwrap();
        }
    }
    let profiles = store.profiles().await.unwrap();
    let id = profiles[0]["profile_id"].as_str().unwrap().to_owned();
    let expected = store.state(&id).await.unwrap();
    store.ingest(&capture, applied).await.unwrap();
    assert_eq!(store.state(&id).await.unwrap(), expected);
    drop(store);
    let reader = Reader::open(&path).await.unwrap();
    let api = reader
        .entities(&id, Default::default(), true)
        .await
        .unwrap();
    let v = &api["items"][0]["values"];
    assert_eq!(v["fight_props"]["2001"], 787.0);
    assert_eq!(v["fight_props"]["2000"], 3700.0);
    assert_eq!(v["fight_props"]["2002"], 211.0);
    assert_eq!(v["fight_props"]["40"], 0.0);
    assert_eq!(v["fight_props_live"], true);
    reader.close().await;
}
fn rows() -> Vec<Value> {
    let data = [
        ("source_config", json!({"mode":"live"})),
        ("login_identity", json!({"uid":"123456789"})),
        (
            "inventory_snapshot",
            json!({"completeness":"full","materials":[{"guid":"123","item_id":100002,"count":10}],"items":[]}),
        ),
        (
            "material_changed",
            json!({"quantity_semantics":"absolute","change":{"guid":"123","item_id":100002,"previous_count":10,"count":11,"delta":1,"new_entry":false}}),
        ),
        (
            "player_profile_snapshot",
            json!({"profile":{"uid":"123456789","nickname":"Owner"}}),
        ),
        (
            "team_snapshot",
            json!({"teams":[{"team_id":10,"name":"Empty","members":[],"is_active":null}]}),
        ),
        (
            "team_changed",
            json!({"changes":[{"previous":{"team_id":10,"name":"Empty","members":[],"is_active":null},"team":{"team_id":10,"name":"Renamed","members":[],"is_active":true}}]}),
        ),
        (
            "player_profile_changed",
            json!({"previous":{"uid":"123456789","nickname":"Owner"},"profile":{"uid":"123456789","nickname":"Updated"}}),
        ),
        ("status", json!({"state":"stopped_game_exited"})),
    ];
    data.into_iter().enumerate().map(|(i,(kind,data))|json!({"schema_version":1,"sequence":i+1,"session":if i==0{"0"}else{"1"},"event":kind,"data":data,"observed_unix_ms":time::OffsetDateTime::now_utc().unix_timestamp_nanos().checked_div(1_000_000).unwrap().to_string(),"source_elapsed_ms":i*100})).collect()
}
#[test]
fn region_prefixes_respect_uid_length_and_explicit_mappings() {
    use app_store::legacy::global_region_from_uid;
    for (uid, expected) in [
        ("600000001", Some("os_usa")),
        ("700000001", Some("os_euro")),
        ("800000001", Some("os_asia")),
        ("900000001", Some("os_cht")),
        ("1800000001", Some("os_asia")),
        ("180000001", None),
        ("6000000000", None),
        ("60000000", None),
        ("0600000000", None),
        ("100000001", None),
        ("200000001", None),
        ("300000001", None),
        ("500000001", None),
        ("000000001", None),
        ("80000000x", None),
        ("８00000001", None),
        ("", None),
    ] {
        assert_eq!(global_region_from_uid(uid), expected, "{uid}");
    }
    let mut n = Normalizer::new(&uuid::Uuid::new_v4().to_string()).unwrap();
    let mut raw = rows();
    raw[1]["data"]["uid"] = json!("600000001");
    n.feed(&raw[0], &BTreeMap::new()).unwrap();
    let override_map = BTreeMap::from([("600000001".into(), "os_euro".into())]);
    let events = n.feed(&raw[1], &override_map).unwrap();
    let identity = events
        .iter()
        .find(|e| e["kind"] == "identity.confirmed")
        .unwrap();
    assert_eq!(identity["profile"]["server_region"], "os_euro");
    assert_eq!(
        identity["payload"]["evidence"]["region_source"],
        "user_import"
    );
}

#[tokio::test]
async fn new_global_profiles_are_created_and_updated_without_region_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("auto-regions.db"))
        .await
        .unwrap();
    for (uid, region) in [
        ("600000001", "os_usa"),
        ("700000001", "os_euro"),
        ("800000001", "os_asia"),
        ("900000001", "os_cht"),
        ("1800000001", "os_asia"),
    ] {
        let capture = uuid::Uuid::new_v4().to_string();
        let mut n = Normalizer::new(&capture).unwrap();
        let mut raw = rows();
        raw[1]["data"]["uid"] = json!(uid);
        for row in &raw[..4] {
            let events = n.feed(row, &BTreeMap::new()).unwrap();
            for event in &events {
                if event["kind"] == "identity.confirmed" {
                    assert_eq!(event["payload"]["evidence"]["region_source"], "uid_prefix");
                }
            }
            if !events.is_empty() {
                store.ingest(&capture, events).await.unwrap();
            }
        }
        let saved = store.profiles().await.unwrap();
        let p = saved
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["uid"] == uid)
            .unwrap();
        assert_eq!(p["server_region"], region);
        let state = store
            .state(p["profile_id"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(state["material_totals"][0]["count"], "11");
    }
    assert_eq!(store.profiles().await.unwrap().as_array().unwrap().len(), 5);
}

#[test]
fn rejected_updates_invalidate_but_do_not_abort_delivery_or_hide_pause_and_stop() {
    for kind in [
        "profile_update_rejected",
        "account_update_rejected",
        "achievement_update_rejected",
        "material_update_rejected",
    ] {
        let mut n = Normalizer::new(&uuid::Uuid::new_v4().to_string()).unwrap();
        let raw = rows();
        for row in &raw[..3] {
            n.feed(row, &regions()).unwrap();
        }
        let mut row = raw[3].clone();
        row["event"] = json!(kind);
        row["data"] = json!({"command_id":3996,"reason":"invalid_profile_envelope","private_field":"must not forward"});
        let events = n.feed(&row, &regions()).unwrap();
        assert!(events.iter().any(|e| e["kind"] == "state.invalidated"));
        assert!(events.iter().any(|e| e["kind"] == "diagnostic.notice"
            && e["payload"]["code"] == "capture_update_rejected"));
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("private_field")
        );
        assert!(!n.closed);
        row["sequence"] = json!(5);
        row["event"] = json!("status");
        row["data"] = json!({"state":"realtime_tracking_paused_invalid_update"});
        assert!(
            n.feed(&row, &regions())
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "state.invalidated")
        );
        row["sequence"] = json!(6);
        row["event"] = json!("source_stats");
        row["data"] = json!({"acquired_packets":100});
        assert!(n.feed(&row, &regions()).is_ok());
        row["sequence"] = json!(7);
        row["event"] = json!("status");
        row["data"] = json!({"state":"stopped_game_exited"});
        assert!(
            n.feed(&row, &regions())
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "capture.closed")
        );
        assert!(n.closed);
    }
    let mut n = Normalizer::new(&uuid::Uuid::new_v4().to_string()).unwrap();
    let raw = rows();
    n.feed(&raw[0], &regions()).unwrap();
    let mut unknown = raw[1].clone();
    unknown["event"] = json!("unknown_update_rejected");
    assert!(n.feed(&unknown, &regions()).is_err());
}

#[test]
fn traveler_partial_constellations_do_not_replace_captured_element() {
    let mut n = Normalizer::new(&uuid::Uuid::new_v4().to_string()).unwrap();
    let raw = rows();
    for row in &raw[..3] {
        n.feed(row, &regions()).unwrap();
    }
    let mut row = raw[3].clone();
    row["event"] = json!("character_snapshot");
    let original = json!({"guid":"42","avatar_id":10000005,"skill_depot_id":505,"talent_ids":[81,82,83,84,85,86],"skill_levels":{"100544":6}});
    row["data"] = json!({"profiles":[original.clone()]});
    n.feed(&row, &regions()).unwrap();
    row["sequence"] = json!(5);
    row["event"] = json!("constellation_depot_changed");
    row["data"] = json!({"guid":"42","avatar_id":10000005,"skill_depot_id":504,"talent_id":31,"previous":null,"current":{"observed_talent_ids":[31],"completeness":"partial"}});
    let out = n.feed(&row, &regions()).unwrap();
    let value = &out.iter().find(|e| e["kind"] == "state.batch").unwrap()["payload"]["operations"]
        [0]["values"];
    for field in ["skill_depot_id", "talent_ids", "skill_levels"] {
        assert_eq!(value[field], original[field]);
    }
    assert_eq!(
        value["constellations_by_depot"]["504"]["observed_talent_ids"],
        json!([31])
    );
    row["sequence"] = json!(6);
    // A duplicate/fabricated update must not silently overwrite the partial baseline.
    assert!(n.feed(&row, &regions()).is_err());
}

#[test]
fn normalization_is_deterministic_and_unknown_identity_does_not_advance() {
    let id = uuid::Uuid::new_v4().to_string();
    let raw = rows();
    let mut a = Normalizer::new(&id).unwrap();
    let mut b = a.clone();
    assert_eq!(
        a.feed(&raw[0], &regions()).unwrap(),
        b.feed(&raw[0], &regions()).unwrap()
    );
    assert!(
        a.feed(&raw[1], &BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("Identity pending")
    );
    for row in &raw[1..] {
        assert_eq!(
            a.feed(row, &regions()).unwrap(),
            b.feed(row, &regions()).unwrap()
        );
    }
    let mut no_id = Normalizer::new(&id).unwrap();
    no_id.feed(&raw[0], &regions()).unwrap();
    let mut fact = raw[2].clone();
    fact["sequence"] = json!(2);
    assert!(
        no_id
            .feed(&fact, &regions())
            .unwrap_err()
            .to_string()
            .contains("Identity pending")
    );
    assert!(no_id.feed(&raw[1], &regions()).is_ok());
}
#[test]
fn packet_loss_invalidates_and_prevents_following_patches() {
    let mut n = Normalizer::new(&uuid::Uuid::new_v4().to_string()).unwrap();
    let raw = rows();
    for row in &raw[..3] {
        n.feed(row, &regions()).unwrap();
    }
    let mut stats = raw[3].clone();
    stats["event"] = json!("source_stats");
    stats["data"] = json!({"dropped_queue_full":1});
    let out = n.feed(&stats, &regions()).unwrap();
    assert!(
        out.iter()
            .any(|e| e["kind"] == "state.invalidated" && e["payload"]["reason"] == "capture_loss")
    );
    let mut patch = raw[3].clone();
    patch["sequence"] = json!(5);
    assert!(n.feed(&patch, &regions()).is_err());
}

#[tokio::test]
async fn journal_http_sqlite_ack_loss_restart_and_source_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.db");
    let writer = Arc::new(Mutex::new(Store::open(&path).await.unwrap()));
    let reader = Reader::open(&path).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, rx) = watch::channel(false);
    let mut server_rx = rx.clone();
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx.clone(),
        writer: Some(writer.clone()),
        token: Some("secret".into()),
        automation_dir: None,
        catalogs: None,
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = server_rx.changed().await;
            })
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let url = format!("http://127.0.0.1:{port}/internal/v1/captures/{id}/events");
    assert_eq!(
        client
            .post(&url)
            .body("invalid")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("secret")
            .json(&json!({"events":[]}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("secret")
            .header("Origin", "https://example.org")
            .json(&json!({"events":[]}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let dir = temp.path().join("realtime-test");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("delivery.json"),
        serde_json::to_vec(&json!({"schema_version":1,"capture_id":id,"log_file":"capture.jsonl"}))
            .unwrap(),
    )
    .unwrap();
    let config = temp.path().join("regions.json");
    fs::write(&config, serde_json::to_vec(&regions()).unwrap()).unwrap();
    let mut bytes = Vec::new();
    for row in rows() {
        serde_json::to_writer(&mut bytes, &row).unwrap();
        bytes.push(b'\n');
    }
    fs::write(dir.join("capture.jsonl"), &bytes).unwrap();
    let mut stream = client
        .get(format!("http://127.0.0.1:{port}/api/v1/events?after=0"))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    tokio::time::timeout(
        Duration::from_secs(5),
        relay::run(temp.path(), &dir, &config, "secret", port, rx.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    let state = writer.lock().await.profiles().await.unwrap();
    let profile = state[0]["profile_id"].as_str().unwrap();
    let before = writer.lock().await.state(profile).await.unwrap();
    assert_eq!(before["material_totals"][0]["count"], "11");
    assert_eq!(
        before["collections"]["teams"][0]["values"]["name"],
        "Renamed"
    );
    assert_eq!(
        before["collections"]["teams"][0]["values"]["members"],
        json!([])
    );
    assert_eq!(
        before["collections"]["player_profile"][0]["values"]["nickname"],
        "Updated"
    );
    let served: Value = client
        .get(format!("http://127.0.0.1:{port}/api/v1/profiles/{profile}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        served["collections"]["teams"],
        before["collections"]["teams"]
    );
    let changes = reader.changes(0, 100).await.unwrap();
    for collection in ["teams", "player_profile"] {
        assert!(changes.iter().any(|change| {
            change["profile_id"] == profile
                && change["topics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|topic| topic == collection)
        }));
    }

    let ack: Value =
        serde_json::from_slice(&fs::read(dir.join("delivery/ack.json")).unwrap()).unwrap();
    assert_eq!(ack["acked_sequence"], 10);
    let notification = tokio::time::timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&notification).contains("id:"));
    drop(stream);
    // Model an ACK lost after commit: regenerate every event and resend identical keys.
    fs::remove_file(dir.join("delivery/ack.json")).unwrap();
    relay::run(temp.path(), &dir, &config, "secret", port, rx.clone())
        .await
        .unwrap();
    assert_eq!(writer.lock().await.state(profile).await.unwrap(), before);
    // A normal restart verifies the saved prefix and sends nothing new.
    relay::run(temp.path(), &dir, &config, "secret", port, rx.clone())
        .await
        .unwrap();
    assert_eq!(writer.lock().await.state(profile).await.unwrap(), before);
    let text = String::from_utf8(bytes)
        .unwrap()
        .replace("\"count\":10", "\"count\":12");
    fs::write(dir.join("capture.jsonl"), text).unwrap();
    assert!(
        relay::run(temp.path(), &dir, &config, "secret", port, rx)
            .await
            .is_err()
    );
    let failure = format!("http://127.0.0.1:{port}/internal/v1/captures/{id}/delivery-failed");
    assert_eq!(client.post(&failure).send().await.unwrap().status(), 401);
    stop.send(true).unwrap();
    server.await.unwrap();
    reader.close().await;
}

#[tokio::test]
async fn equipment_transfer_is_one_durable_batch_and_resolves_both_api_cards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("equipment.db");
    let mut store = Store::open(&path).await.unwrap();
    let capture = uuid::Uuid::new_v4().to_string();
    let mut n = Normalizer::new(&capture).unwrap();
    let mut raw = rows()[..3].to_vec();
    raw[2]["data"]["items"] = json!([{"guid":"101","item_id":11401,"kind":"weapon","count":1}]);
    raw.push(json!({"schema_version":1,"session":"1","sequence":4,"event":"character_snapshot","data":{"profiles":[{"guid":"42","avatar_id":10000114,"equip_guids":[]},{"guid":"43","avatar_id":10000089,"equip_guids":["101"]}]}}));
    raw.push(json!({"schema_version":1,"session":"1","sequence":5,"event":"equipment_changed","data":{"guid":"42","avatar_id":10000114,"equip_type":6,"item_id":11401,"item_guid":"101","updates":[{"guid":"42","avatar_id":10000114,"previous":[],"current":["101"]},{"guid":"43","avatar_id":10000089,"previous":["101"],"current":[]}]}}));
    let mut applied = Vec::new();
    for mut row in raw {
        row["observed_unix_ms"] = json!(
            time::OffsetDateTime::now_utc()
                .unix_timestamp_nanos()
                .checked_div(1_000_000)
                .unwrap()
                .to_string()
        );
        applied = n.feed(&row, &regions()).unwrap();
        if !applied.is_empty() {
            store.ingest(&capture, applied.clone()).await.unwrap();
        }
    }
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0]["kind"], "state.batch");
    assert_eq!(
        applied[0]["payload"]["operations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let profiles = store.profiles().await.unwrap();
    let profile = profiles[0]["profile_id"].as_str().unwrap();
    let reader = Reader::open(&path).await.unwrap();
    let api = reader
        .entities(profile, Default::default(), true)
        .await
        .unwrap();
    assert_eq!(
        api["items"][0]["equipment"][0]["reference"]["values"]["item_id"],
        11401
    );
    assert_eq!(api["items"][1]["equipment"], json!([]));
    reader.close().await;
    // A replayed collector batch is acknowledged without a second state mutation.
    let before = store.state(profile).await.unwrap();
    store.ingest(&capture, applied).await.unwrap();
    assert_eq!(store.state(profile).await.unwrap(), before);
}
