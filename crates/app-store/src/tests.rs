use super::*;
use legacy::Identity;

#[tokio::test]
async fn verified_material_baseline_removes_old_stocks_without_promoting_other_collections() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("full-materials.db"))
        .await
        .unwrap();
    let mut log = Log::new(1700);
    log.add("1","inventory_snapshot",json!({"completeness":"unverified","materials":[{"guid":"42","item_id":104003,"count":24}],"items":[{"guid":"43","item_id":11516,"kind":"weapon","count":1}]}));
    log.add("2","inventory_snapshot",json!({"synthetic":false,"command_id":20892,"completeness":"unverified","material_completeness":"full","material_coverage":"player_store_notify_7_2","recognized_item_count":1,"materials":[{"guid":"44","item_id":101241,"count":234}],"items":[]}));
    let input = log.finish();
    let binding = crate::legacy::Bindings::from([
        (
            "1".into(),
            Identity {
                uid: "700000001".into(),
                server_region: "os_euro".into(),
            },
        ),
        (
            "2".into(),
            Identity {
                uid: "700000001".into(),
                server_region: "os_euro".into(),
            },
        ),
    ]);
    let result = store.import(&input, &binding).await.unwrap();
    let id = result["profiles"]["2"].as_str().unwrap();
    let state = store.state(id).await.unwrap();
    assert_eq!(
        state["collections"]["material_stacks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        state["collections"]["material_stacks"][0]["values"]["item_id"],
        101241
    );
    assert_eq!(
        state["collections"]["weapons"][0]["confirmation"],
        "historical_unconfirmed"
    );
    assert!(
        state["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["collection"] == "material_stacks" && d["scope"] == "full")
    );
    let mut projection = legacy::Projection::default();
    for patch in [
        json!({"command_id":7608}),
        json!({"synthetic":true}),
        json!({"recognized_item_count":99}),
        json!({"material_coverage":"unknown"}),
    ] {
        let mut v = json!({"synthetic":false,"command_id":20892,"completeness":"unverified","material_completeness":"full","material_coverage":"player_store_notify_7_2","recognized_item_count":0,"materials":[],"items":[]});
        for (key, value) in patch.as_object().unwrap() {
            v[key] = value.clone();
        }
        assert!(projection.apply("inventory_snapshot", &v).is_err());
    }
}

#[test]
fn fight_property_deltas_preserve_unmentioned_stats_and_reject_wrong_history_atomically() {
    let mut p = legacy::Projection::default();
    p.apply("character_snapshot",&json!({"profiles":[{"guid":"42","avatar_id":10000137,"fight_props":{"2000":3700.0,"2001":464.0,"2002":211.0,"40":0.2}}]})).unwrap();
    p.apply("fight_properties_changed",&json!({"guid":"42","avatar_id":10000137,"previous":{"2001":464.0,"40":0.2,"28":null},"current":{"2001":467.25,"40":0.0,"28":91.92999267578125}})).unwrap();
    let saved = p.entity("characters", "42").unwrap().clone();
    assert_eq!(saved["fight_props"]["2000"], 3700.0);
    assert_eq!(saved["fight_props"]["2002"], 211.0);
    assert_eq!(saved["fight_props"]["40"], 0.0);
    assert_eq!(saved["fight_props"]["28"].to_string(), "91.92999267578125");
    assert_eq!(saved["fight_props_live"], true);
    assert!(p.apply("fight_properties_changed",&json!({"guid":"42","avatar_id":10000137,"previous":{"2001":467.25,"2002":999.0},"current":{"2001":787.0,"2002":216.0}})).is_err());
    assert_eq!(p.entity("characters", "42"), Some(&saved));
}

#[tokio::test]
async fn equipment_transfer_persists_both_characters_without_changing_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("equipment.db");
    let mut store = Store::open(&path).await.unwrap();
    let mut log = Log::new(1700);
    log.add("1","inventory_snapshot",json!({"materials":[],"items":[{"guid":"101","item_id":11517,"kind":"weapon","count":1},{"guid":"102","item_id":11401,"kind":"weapon","count":1},{"guid":"201","item_id":40524,"kind":"artifact","count":1},{"guid":"202","item_id":32524,"kind":"artifact","count":1}]}));
    log.add("1","character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000114,"equip_guids":["201","101"]},{"guid":"43","avatar_id":10000089,"equip_guids":["202","102"]}]}));
    log.add("1","equipment_changed",json!({"guid":"42","avatar_id":10000114,"item_id":32524,"item_guid":"202","equip_type":2,"updates":[{"guid":"42","avatar_id":10000114,"previous":["201","101"],"current":["101","202"]},{"guid":"43","avatar_id":10000089,"previous":["202","102"],"current":["102"]}]}));
    let archive = log.finish();
    let imported = store.import(&archive, &bindings()).await.unwrap();
    let id = imported["profiles"]["1"].as_str().unwrap().to_owned();
    let expected = store.state(&id).await.unwrap();
    assert_eq!(
        expected["collections"]["characters"][0]["values"]["equip_guids"],
        json!(["101", "202"])
    );
    assert_eq!(
        expected["collections"]["characters"][1]["values"]["equip_guids"],
        json!(["102"])
    );
    assert_eq!(
        expected["collections"]["artifacts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        expected["collections"]["weapons"].as_array().unwrap().len(),
        2
    );
    let reader = crate::reader::Reader::open(&path).await.unwrap();
    let api = reader
        .entities(&id, crate::reader::PageQuery::default(), true)
        .await
        .unwrap();
    let target = api["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entity_key"] == "42")
        .unwrap();
    assert_eq!(target["equipment"][1]["guid"], "202");
    assert_eq!(
        target["equipment"][1]["reference"]["values"]["item_id"],
        32524
    );
    reader.close().await;
    drop(store);
    let mut reopened = Store::open(&path).await.unwrap();
    assert_eq!(reopened.state(&id).await.unwrap(), expected);
    let duplicate = reopened.import(&archive, &bindings()).await.unwrap();
    assert_eq!(duplicate["inserted_events"].as_u64().unwrap_or(0), 0);
}

#[test]
fn equipment_projection_validates_both_sides_before_mutation() {
    let mut p = legacy::Projection::default();
    p.apply(
        "inventory_snapshot",
        &json!({"materials":[],"items":[{"guid":"101","item_id":11401,"kind":"weapon","count":1}]}),
    )
    .unwrap();
    p.apply("character_snapshot",&json!({"profiles":[{"guid":"42","avatar_id":10000114,"equip_guids":[]},{"guid":"43","avatar_id":10000089,"equip_guids":["101"]}]})).unwrap();
    let target = json!({"guid":"42","avatar_id":10000114,"previous":[],"current":["101"]});
    // Missing source removal must never give the same GUID to two owners.
    assert!(
        p.apply("equipment_changed", &json!({"updates":[target]}))
            .is_err()
    );
    let invalid_source = json!({"guid":"43","avatar_id":10000089,"previous":["999"],"current":[]});
    assert!(
        p.apply(
            "equipment_changed",
            &json!({"updates":[target,invalid_source]})
        )
        .is_err()
    );
    assert_eq!(
        p.entity("characters", "42").unwrap()["equip_guids"],
        json!([])
    );
    assert_eq!(
        p.entity("characters", "43").unwrap()["equip_guids"],
        json!(["101"])
    );
}

#[tokio::test]
async fn player_profile_and_teams_survive_restart_and_remain_account_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::open(&path).await.unwrap();
    let mut log = Log::new(1000);
    let mut b = bindings();
    b.insert(
        "2".into(),
        Identity {
            uid: "987654321".into(),
            server_region: "os_asia".into(),
        },
    );
    for (epoch, uid, name) in [("1", "123456789", "Europe"), ("2", "987654321", "Asia")] {
        log.add(
            epoch,
            "player_profile_snapshot",
            json!({"profile":{"uid":uid,"nickname":name}}),
        );
        let team = json!({"team_id":10,"name":name,"members":[{"position":1,"guid":"18446744073709551615"}],"is_active":null});
        log.add(epoch, "team_snapshot", json!({"teams":[team]}));
        let changed = json!({"team_id":10,"name":"Empty","members":[],"is_active":true});
        log.add(
            epoch,
            "team_changed",
            json!({"changes":[{"previous":team,"team":changed}]}),
        );
    }
    let result = store.import(&log.finish(), &b).await.unwrap();
    let eu = result["profiles"]["1"].as_str().unwrap().to_owned();
    let asia = result["profiles"]["2"].as_str().unwrap().to_owned();
    let state = store.state(&eu).await.unwrap();
    assert_eq!(
        state["collections"]["teams"][0]["values"]["members"],
        json!([])
    );
    assert_eq!(
        state["collections"]["player_profile"][0]["values"]["nickname"],
        "Europe"
    );
    assert_eq!(
        store.state(&asia).await.unwrap()["collections"]["player_profile"][0]["values"]["nickname"],
        "Asia"
    );
    store.close().await.unwrap();
    let mut store = Store::open(&path).await.unwrap();
    assert_eq!(store.state(&eu).await.unwrap(), state);
    store.close().await.unwrap();
}

#[tokio::test]
async fn foreign_profile_uid_rolls_back_entire_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut log = Log::new(1000);
    log.add(
        "1",
        "player_profile_snapshot",
        json!({"profile":{"uid":"987654321","nickname":"Foreign"}}),
    );
    assert!(store.import(&log.finish(), &bindings()).await.is_err());
    assert_eq!(store.profiles().await.unwrap(), json!([]));
    store.close().await.unwrap();
}

#[tokio::test]
async fn loss_in_source_summary_remains_stale_after_an_otherwise_clean_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    l.snapshot("1", json!([row("42", 3)]), "unverified");
    l.add("1", "source_summary", json!({"dropped_queue_full":2}));
    s.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut s).await;
    let state = s.state(&id).await.unwrap();
    assert!(
        state["domains"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["freshness"] == "stale" && d["reason"] == "capture_loss")
    );
    s.close().await.unwrap();
}

#[tokio::test]
async fn newer_database_schema_is_rejected_without_downgrading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    let mut s = Store::open(&path).await.unwrap();
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES(999,'future',1,x'00',0)").execute(&mut s.connection).await.unwrap();
    s.close().await.unwrap();
    assert!(Store::open(&path).await.is_err());
}

#[tokio::test]
async fn observed_decimal_stats_keep_their_exact_source_representation() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    let avatar:Value=serde_json::from_str(r#"{"guid":"42","avatar_id":10000035,"fight_props":{"28":91.92999267578125,"29":0.12345678901234567890123456789}}"#).unwrap();
    l.add("1", "character_snapshot", json!({"profiles":[avatar]}));
    s.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut s).await;
    let state = s.state(&id).await.unwrap();
    assert_eq!(
        state["collections"]["characters"][0]["values"]["fight_props"]["28"].to_string(),
        "91.92999267578125"
    );
    assert_eq!(
        state["collections"]["characters"][0]["values"]["fight_props"]["29"].to_string(),
        "0.12345678901234567890123456789"
    );
    s.close().await.unwrap();
}

#[tokio::test]
async fn achievement_progress_and_constellation_are_saved_with_raw_character_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    let before =
        json!({"achievement_id":80001,"status_code":1,"current_progress":1,"total_progress":5});
    let after =
        json!({"achievement_id":80001,"status_code":1,"current_progress":2,"total_progress":5});
    l.add("1", "achievement_snapshot", json!({"records":[before]}));
    l.add(
        "1",
        "achievement_changed",
        json!({"achievement_id":80001,"previous":before,"current":after}),
    );
    l.add("1","character_snapshot",json!({"profiles":[{"guid":"42","avatar_id":10000035,"trial_info_present":false,"skill_depot_id":3501,"talent_ids":[],"equip_guids":["18446744073709551615"]}]}));
    l.add("1","constellation_changed",json!({"guid":"42","avatar_id":10000035,"skill_depot_id":3501,"previous":{"talent_ids":[]},"current":{"talent_ids":[351]}}));
    store.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let s = store.state(&id).await.unwrap();
    assert_eq!(s["collections"]["achievements"][0]["values"], after);
    assert_eq!(
        s["collections"]["characters"][0]["values"]["talent_ids"],
        json!([351])
    );
    assert_eq!(
        s["collections"]["characters"][0]["values"]["equip_guids"][0],
        "18446744073709551615"
    );
    store.close().await.unwrap();
}

struct Log {
    events: Vec<Value>,
    time: i64,
}
impl Log {
    fn new(time: i64) -> Self {
        let mut s = Self {
            events: vec![],
            time,
        };
        s.add(
            "0",
            "source_config",
            json!({"mode":"live","decoder_protocol":"7.2"}),
        );
        s
    }
    fn add(&mut self, epoch: &str, kind: &str, data: Value) {
        let n = self.events.len() + 1;
        self.events.push(json!({"schema_version":1,"sequence":n,"session":epoch,"observed_unix_ms":(self.time+n as i64).to_string(),"event":kind,"data":data}));
    }
    fn snapshot(&mut self, epoch: &str, rows: Value, scope: &str) {
        self.add(
            epoch,
            "inventory_snapshot",
            json!({"materials":rows,"items":[],"completeness":scope}),
        );
    }
    fn finish(mut self) -> Vec<u8> {
        let epoch = self.events.last().unwrap()["session"]
            .as_str()
            .unwrap()
            .to_owned();
        self.add(&epoch, "status", json!({"state":"stopped_game_exited"}));
        self.bytes()
    }
    fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for e in &self.events {
            serde_json::to_writer(&mut b, e).unwrap();
            b.push(b'\n');
        }
        b
    }
}
fn row(key: &str, count: u32) -> Value {
    json!({"guid":key,"item_id":100002,"count":count})
}
fn bindings() -> Bindings {
    Bindings::from([(
        "1".into(),
        Identity {
            uid: "123456789".into(),
            server_region: "os_euro".into(),
        },
    )])
}
fn pickup(before: u32, after: u32) -> Value {
    json!({"quantity_semantics":"absolute","change":{"guid":"18446744073709551615","item_id":100002,"new_entry":false,"removed":after==0,"previous_count":before,"count":after,"delta":i64::from(after)-i64::from(before)}})
}
async fn profile(store: &mut Store) -> String {
    store.profiles().await.unwrap()[0]["profile_id"]
        .as_str()
        .unwrap()
        .into()
}
fn sample(time: i64, count: u32) -> Vec<u8> {
    let mut l = Log::new(time);
    l.snapshot(
        "1",
        json!([row("18446744073709551615", count)]),
        "unverified",
    );
    l.finish()
}

#[tokio::test]
async fn theater_arcana_survive_a_lower_snapshot_until_the_next_season() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state.sqlite3"))
        .await
        .unwrap();
    let mut log = Log::new(1000);
    log.snapshot("1", json!([]), "unverified");
    let season = |id, starts, arcana| {
        json!({"schedule_id":id, "starts_unix_s":starts,
            "ends_unix_s":starts + 30 * 86400, "acts":10, "arcana":arcana, "echo_keys":2})
    };
    log.add("1", "theater_snapshot", season(31, 1793502000_i64, 2));
    log.add("1", "theater_snapshot", season(31, 1793502000_i64, 0));
    store.import(&log.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let best = sqlx::query_scalar::<_, i64>(
        "SELECT json_extract(value_json,'$.arcana') FROM current_entities WHERE profile_id=? AND collection='theater'",
    )
    .bind(&id)
    .fetch_one(&mut store.connection)
    .await
    .unwrap();
    assert_eq!(best, 2);

    // An existing database may contain the older projector's lowered value.
    sqlx::query("UPDATE current_entities SET value_json=json_set(value_json,'$.arcana',0) WHERE collection='theater'")
        .execute(&mut store.connection)
        .await
        .unwrap();
    assert_eq!(
        store.state(&id).await.unwrap()["collections"]["theater"][0]["values"]["arcana"],
        2
    );

    let mut log = Log::new(2000);
    log.snapshot("1", json!([]), "unverified");
    log.add("1", "theater_snapshot", season(32, 1796094000_i64, 0));
    store.import(&log.finish(), &bindings()).await.unwrap();
    assert_eq!(
        store.state(&id).await.unwrap()["collections"]["theater"][0]["values"]["arcana"],
        Value::Null
    );
}

#[tokio::test]
async fn onslaught_exalted_roster_is_saved_without_a_personal_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("state.sqlite3"))
        .await
        .unwrap();
    let mut log = Log::new(1000);
    log.snapshot("1", json!([]), "unverified");
    let roster = json!({"schedule_id":5269013, "starts_unix_s":1794387600,
        "ends_unix_s":1797303599, "avatar_ids":[10000089,10000130]});
    log.add("1", "onslaught_exalted_snapshot", roster.clone());
    store.import(&log.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let state = store.state(&id).await.unwrap();
    assert_eq!(
        state["collections"]["onslaught_exalted"][0]["values"],
        roster
    );
    assert!(state["collections"]["onslaught"].is_null());
}

#[tokio::test]
async fn restart_and_duplicate_preserve_exact_state_and_large_guid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite3");
    let mut store = Store::open(&path).await.unwrap();
    let mut l = Log::new(1000);
    l.snapshot("1", json!([row("18446744073709551615", 10)]), "unverified");
    l.add("1", "material_changed", pickup(10, 11));
    let bytes = l.finish();
    store.import(&bytes, &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let state = store.state(&id).await.unwrap();
    assert_eq!(state["material_totals"][0]["count"], "11");
    assert_eq!(
        state["collections"]["material_stacks"][0]["entity_key"],
        "18446744073709551615"
    );
    assert_eq!(
        store.import(&bytes, &bindings()).await.unwrap()["duplicate"],
        true
    );
    assert_eq!(state, store.state(&id).await.unwrap());
    store.close().await.unwrap();
    let mut reopened = Store::open(&path).await.unwrap();
    assert_eq!(state, reopened.state(&id).await.unwrap());
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn multi_epoch_same_uid_different_server_and_return_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    for (epoch, count) in [("1", 5), ("2", 99), ("3", 7)] {
        l.snapshot(epoch, json!([row("42", count)]), "unverified");
    }
    let mut b = bindings();
    b.insert(
        "2".into(),
        Identity {
            uid: "123456789".into(),
            server_region: "os_asia".into(),
        },
    );
    b.insert("3".into(), b["1"].clone());
    let result = store.import(&l.finish(), &b).await.unwrap();
    let eu = result["profiles"]["1"].as_str().unwrap();
    let asia = result["profiles"]["2"].as_str().unwrap();
    assert_ne!(eu, asia);
    assert_eq!(result["profiles"]["1"], result["profiles"]["3"]);
    assert_eq!(
        store.state(eu).await.unwrap()["material_totals"][0]["count"],
        "7"
    );
    assert_eq!(
        store.state(asia).await.unwrap()["material_totals"][0]["count"],
        "99"
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn unverified_omissions_survive_unconfirmed_full_and_explicit_delete_remove() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    l.snapshot("1", json!([row("41", 10), row("42", 20)]), "unverified");
    store.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let mut l = Log::new(2000);
    l.snapshot("1", json!([row("42", 3)]), "unverified");
    store.import(&l.finish(), &bindings()).await.unwrap();
    let s = store.state(&id).await.unwrap();
    assert_eq!(
        s["collections"]["material_stacks"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        s["collections"]["material_stacks"][0]["confirmation"],
        "historical_unconfirmed"
    );
    assert_eq!(s["material_totals"][0]["count"], "3");
    let mut l = Log::new(3000);
    l.snapshot("1", json!([]), "full");
    store.import(&l.finish(), &bindings()).await.unwrap();
    assert!(
        store.state(&id).await.unwrap()["material_totals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut l = Log::new(4000);
    l.snapshot("1", json!([row("18446744073709551615", 3)]), "unverified");
    l.add("1", "material_changed", pickup(3, 0));
    store.import(&l.finish(), &bindings()).await.unwrap();
    assert!(
        store.state(&id).await.unwrap()["material_totals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn older_archives_are_stored_without_replacing_newer_saved_values() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    store.import(&sample(2000, 20), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    store.import(&sample(1000, 1), &bindings()).await.unwrap();
    assert_eq!(
        store.state(&id).await.unwrap()["material_totals"][0]["count"],
        "20"
    );
    let captures: i64 = sqlx::query_scalar("SELECT count(*) FROM captures")
        .fetch_one(&mut store.connection)
        .await
        .unwrap();
    assert_eq!(captures, 2);
    store.close().await.unwrap();
}

#[tokio::test]
async fn conflicting_binding_and_missing_epoch_cannot_reassign_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let bytes = sample(1000, 10);
    store.import(&bytes, &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let before = store.state(&id).await.unwrap();
    let mut b = bindings();
    b.get_mut("1").unwrap().uid = "987654321".into();
    assert!(store.import(&bytes, &b).await.is_err());
    assert!(store.import(&bytes, &Bindings::new()).await.is_err());
    assert_eq!(before, store.state(&id).await.unwrap());
    assert_eq!(store.profiles().await.unwrap().as_array().unwrap().len(), 1);
    store.close().await.unwrap();
}

#[tokio::test]
async fn database_failure_mid_import_rolls_back_events_profiles_snapshots_and_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_entity BEFORE INSERT ON current_entities BEGIN SELECT RAISE(ABORT,'injected failure'); END;").execute(&mut store.connection).await.unwrap();
    assert!(store.import(&sample(1000, 10), &bindings()).await.is_err());
    for (table, query) in [
        ("captures", "SELECT count(*) FROM captures"),
        ("profiles", "SELECT count(*) FROM profiles"),
        ("events", "SELECT count(*) FROM events"),
        ("snapshots", "SELECT count(*) FROM snapshots"),
        ("epochs", "SELECT count(*) FROM epochs"),
        ("current_entities", "SELECT count(*) FROM current_entities"),
        ("change_log", "SELECT count(*) FROM change_log"),
    ] {
        let n: i64 = sqlx::query_scalar(query)
            .fetch_one(&mut store.connection)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let n: i64 = sqlx::query_scalar("SELECT revision FROM store_meta")
        .fetch_one(&mut store.connection)
        .await
        .unwrap();
    assert_eq!(n, 0);
    store.close().await.unwrap();
}

#[tokio::test]
async fn malformed_history_and_incomplete_archives_leave_database_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let bytes = sample(1000, 10);
    store.import(&bytes, &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let state = store.state(&id).await.unwrap();
    let mut l = Log::new(2000);
    l.snapshot("1", json!([row("18446744073709551615", 10)]), "unverified");
    l.add("1", "material_changed", pickup(9, 11));
    assert!(store.import(&l.finish(), &bindings()).await.is_err());
    assert!(
        store
            .import(&bytes[..bytes.len() - 1], &bindings())
            .await
            .is_err()
    );
    let mut l = Log::new(3000);
    l.snapshot("1", json!([]), "unverified");
    assert!(store.import(&l.bytes(), &bindings()).await.is_err());
    let mut l = Log::new(4000);
    l.snapshot("1", json!([]), "unverified");
    l.events[1]["sequence"] = json!(3);
    assert!(store.import(&l.finish(), &bindings()).await.is_err());
    assert_eq!(state, store.state(&id).await.unwrap());
    store.close().await.unwrap();
}

#[tokio::test]
async fn partial_wallet_baselines_merge_and_errors_do_not_become_saved_on_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    for (id, count) in [(201, 50), (202, 1000)] {
        l.add(
            "1",
            "currency_snapshot",
            json!({"item_id":id,"count":count}),
        );
    }
    l.add(
        "1",
        "status",
        json!({"state":"decoder_failed_key_recovery"}),
    );
    store.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let s = store.state(&id).await.unwrap();
    assert_eq!(s["collections"]["wallet"].as_array().unwrap().len(), 2);
    assert!(
        s["collections"]["wallet"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["confirmation"] == "observed")
    );
    assert_eq!(s["domains"][0]["freshness"], "error");
    store.close().await.unwrap();
}

#[tokio::test]
async fn character_talents_friendship_progress_and_instance_consumption_persist() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("s.db")).await.unwrap();
    let mut l = Log::new(1000);
    l.snapshot("1", json!([]), "unverified");
    let weapon = json!({"guid":"99","item_id":11101,"kind":"weapon","level":1});
    l.add("1","item_changed",json!({"change":{"guid":"99","item_id":11101,"new_entry":true,"removed":false,"previous":null,"item":weapon}}));
    l.add("1","item_changed",json!({"change":{"guid":"99","item_id":11101,"new_entry":false,"removed":true,"previous":weapon,"item":null}}));
    let avatar = json!({"guid":"42","avatar_id":10000035,"trial_info_present":false,"skill_levels":{"101":1},"friendship":{"level":1,"exp":3},"props":{"4001":{"ival":"20"},"1001":{"ival":"0"},"1002":{"ival":"0"}}});
    l.add("1", "character_snapshot", json!({"profiles":[avatar]}));
    l.add(
        "1",
        "talent_changed",
        json!({"guid":"42","avatar_id":10000035,"skill_id":101,"previous_level":1,"level":2}),
    );
    l.add("1","friendship_changed",json!({"guid":"42","avatar_id":10000035,"previous":{"guid":"42","avatar_id":10000035,"level":1,"exp":3},"current":{"guid":"42","avatar_id":10000035,"level":1,"exp":8}}));
    l.add("1","character_progress_changed",json!({"guid":"42","avatar_id":10000035,"previous":{"level":20,"exp":0,"ascension":0},"current":{"level":21,"exp":50,"ascension":0}}));
    store.import(&l.finish(), &bindings()).await.unwrap();
    let id = profile(&mut store).await;
    let s = store.state(&id).await.unwrap();
    assert!(s["collections"]["weapons"].is_null());
    let v = &s["collections"]["characters"][0]["values"];
    assert_eq!(v["skill_levels"]["101"], 2);
    assert_eq!(v["friendship"]["exp"], 8);
    assert_eq!(v["props"]["4001"]["ival"], "21");
    store.close().await.unwrap();
}
