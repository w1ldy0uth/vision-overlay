use app_server::{
    ApiState,
    catalog::{Bundle, Catalogs},
    router,
};
use app_store::{Store, reader::Reader};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, sync::Arc};
use tokio::sync::watch;
fn payload(rev: &str, name: &str) -> Value {
    json!({
        "revisions":{"items":rev,"avatars":rev,"achievements":rev},
        "items":{"100073":{"icon":"UI_ItemIcon_100073","rank":null,"rarity":1}},
        "names":{"100073":name},"rarities":{"100073":1},
        "avatarNames":{"10000005":"Путешественник"},"avatarIcons":{"10000005":"UI_AvatarIcon_PlayerBoy"},
        "skillNames":{"10018":"Навык"},"materials":{"100073":{"itemType":"ITEM_MATERIAL","materialType":"MATERIAL_EXCHANGE"}},
        "achievements":{"84516":{"name":"Достижение","goal_id":0,"chain_id":"84516","stage":1}},
        "mainProps":[{"id":1,"propType":"FIGHT_PROP_HP"}],"affixes":[{"id":1,"propType":"FIGHT_PROP_HP","propValue":1}],
        "relicLevels":[{"level":1,"rank":1,"addProps":[]}],"inventoryClassification":{"schemaVersion":1}
    })
}
fn bundle(rev: &str, name: &str) -> Bundle {
    Bundle::new(payload(rev, name), rev, "bundled", BTreeMap::new()).unwrap()
}
#[test]
fn legacy_catalog_is_readable_but_cannot_seed_a_portable_release() {
    let b = bundle(&"a".repeat(40), "Краб");
    b.validate().unwrap();
    let error = app_server::catalog::portable_payload(&b).unwrap_err();
    assert!(error.to_string().contains("characterDetails.skills"));
}

#[test]
fn bundled_catalog_is_a_valid_bootstrap_version() {
    let payload = app_server::bundled_catalog().expect("Build the web assets before this test");
    let revision = payload["revisions"]["items"].as_str().unwrap().to_owned();
    Bundle::new(payload, &revision, "bundled", BTreeMap::new()).unwrap();
}
#[test]
fn achievement_counts_distinguish_playable_excluded_and_unclassified_packages() {
    let dir = tempfile::tempdir().unwrap();
    let revision = "a".repeat(40);
    let mut data = payload(&revision, "Краб");
    let legacy = Catalogs::open(dir.path().join("legacy"), data.clone()).unwrap();
    assert!(legacy.status()["current"]["counts"]["achievements_playable"].is_null());
    data["achievements"]["84516"]["is_disused"] = json!(false);
    for (id, disused) in [("84517", false), ("84518", true)] {
        data["achievements"][id] = json!({"name":"Исключено","goal_id":0,"chain_id":id,"stage":1,"is_disused":disused,"excluded":true});
    }
    let classified = Catalogs::open(dir.path().join("classified"), data.clone()).unwrap();
    let counts = classified.status()["current"]["counts"].clone();
    assert_eq!(counts["achievements"], 3);
    assert_eq!(counts["achievements_playable"], 1);
    assert_eq!(counts["achievements_excluded"], 2);
    data["achievements"]["84518"]["is_disused"] = json!("true");
    assert!(Bundle::new(data, &revision, "bundled", BTreeMap::new()).is_err());
}
fn current(c: &Catalogs) -> String {
    c.status()["current"]["id"].as_str().unwrap().into()
}

#[test]
fn atomic_activation_rollback_and_restart_use_whole_versions() {
    let dir = tempfile::tempdir().unwrap();
    let rev = "a".repeat(40);
    let seed = payload(&rev, "Краб");
    let c = Catalogs::open(dir.path().into(), seed.clone()).unwrap();
    let original = current(&c);
    let next = c
        .stage(bundle(&"b".repeat(40), "Обновлённый краб"))
        .unwrap();
    assert_eq!(current(&c), original);
    c.activate(next["id"].as_str().unwrap(), &original).unwrap();
    let activated = current(&c);
    assert_ne!(activated, original);
    drop(c);
    let c = Catalogs::open(dir.path().into(), seed).unwrap();
    assert_eq!(current(&c), activated);
    assert_eq!(c.status()["previous"]["id"], original);
    assert!(c.activate(&activated, &original).is_err());
    c.rollback(&activated).unwrap();
    assert_eq!(current(&c), original);
    assert_eq!(
        serde_json::from_slice::<Value>(&c.payload().1).unwrap()["names"]["100073"],
        "Краб"
    );
}
#[test]
fn invalid_bundles_tampered_stage_and_activation_io_failure_preserve_working_data() {
    let dir = tempfile::tempdir().unwrap();
    let rev = "a".repeat(40);
    let c = Catalogs::open(dir.path().into(), payload(&rev, "Краб")).unwrap();
    let original = current(&c);
    let mut bad = bundle(&rev, "Краб");
    bad.payload["names"]["100073"] = json!("tampered");
    assert!(c.stage(bad).is_err());
    let mut bad = bundle(&rev, "Краб");
    bad.manifest.format_version = 2;
    assert!(c.stage(bad).is_err());
    for mutation in ["link", "revision", "icon"] {
        let mut p = payload(&rev, "Краб");
        match mutation {
            "link" => p["achievements"]["84516"]["chain_id"] = json!("999"),
            "revision" => p["revisions"]["avatars"] = json!("b".repeat(40)),
            _ => p["items"]["100073"]["icon"] = json!("https://evil/image.svg"),
        }
        assert!(Bundle::new(p, &rev, "bundled", BTreeMap::new()).is_err());
    }
    let staged = c.stage(bundle(&"b".repeat(40), "Новый")).unwrap();
    let path = dir.path().join("staged.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["bundle"]["payload"]["names"]["100073"] = json!("tampered");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        c.activate(staged["id"].as_str().unwrap(), &original)
            .is_err()
    );
    assert_eq!(current(&c), original);
    let next = c.stage(bundle(&"b".repeat(40), "Новый")).unwrap();
    let active = dir.path().join("active.json");
    fs::remove_file(&active).unwrap();
    fs::create_dir(&active).unwrap();
    assert!(c.activate(next["id"].as_str().unwrap(), &original).is_err());
    assert_eq!(current(&c), original);
}
#[test]
fn missing_active_pointer_and_unavailable_storage_keep_a_working_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let seed = payload(&"a".repeat(40), "Краб");
    let c = Catalogs::open(dir.path().join("catalogs"), seed.clone()).unwrap();
    let original = current(&c);
    drop(c);
    fs::remove_file(dir.path().join("catalogs/active.json")).unwrap();
    let c = Catalogs::open(dir.path().join("catalogs"), seed.clone()).unwrap();
    assert_eq!(current(&c), original);
    assert!(c.status()["last_error"].is_string());
    let blocked = dir.path().join("blocked");
    fs::write(&blocked, b"not a directory").unwrap();
    let c = Catalogs::open(blocked, seed).unwrap();
    assert!(c.status()["last_error"].is_string());
    let original_id = current(&c);
    assert!(c.stage(bundle(&"b".repeat(40), "Новый")).is_err());
    assert_eq!(original_id, current(&c));
}
#[test]
fn restart_recovers_previous_catalog_when_active_bytes_are_damaged() {
    let dir = tempfile::tempdir().unwrap();
    let seed = payload(&"a".repeat(40), "Краб");
    let c = Catalogs::open(dir.path().into(), seed.clone()).unwrap();
    let original = current(&c);
    let next = c.stage(bundle(&"b".repeat(40), "Новый")).unwrap();
    let id = next["id"].as_str().unwrap();
    c.activate(id, &original).unwrap();
    drop(c);
    fs::write(
        dir.path().join("versions").join(format!("{id}.json")),
        b"broken",
    )
    .unwrap();
    let c = Catalogs::open(dir.path().into(), seed).unwrap();
    assert_eq!(current(&c), original);
    assert!(c.status()["last_error"].is_string());
}

#[tokio::test]
async fn catalog_api_requires_control_origin_and_updates_without_rebuild_or_account_revision_change()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open(&path).await.unwrap();
    let reader = Reader::open(&path).await.unwrap();
    let c = Arc::new(
        Catalogs::open(
            dir.path().join("catalogs"),
            payload(&"a".repeat(40), "Краб"),
        )
        .unwrap(),
    );
    let original = current(&c);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    let (stop, rx) = watch::channel(false);
    let mut shutdown = rx.clone();
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx,
        writer: None,
        token: None,
        automation_dir: None,
        catalogs: Some(c.clone()),
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await
            .unwrap()
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let before: Value = client
        .get(format!("{url}/api/v1/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .post(format!("{url}/api/v1/catalog/prepare"))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let next = bundle(&"b".repeat(40), "Новый краб");
    let response: Value = client
        .post(format!("{url}/api/v1/catalog/import"))
        .header("origin", &url)
        .header("x-vision-control", "1")
        .json(&next)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(current(&c), original);
    let id = response["candidate"]["version"]["id"].as_str().unwrap();
    let r = client
        .post(format!("{url}/api/v1/catalog/activate"))
        .header("origin", &url)
        .header("x-vision-control", "1")
        .json(&json!({"id":id,"expected_active":original}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let r = client
        .get(format!("{url}/catalog.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-catalog-version"], id);
    let data: Value = r.json().await.unwrap();
    assert_eq!(data["names"]["100073"], "Новый краб");
    let after: Value = client
        .get(format!("{url}/api/v1/state"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["revision"], after["revision"]);
    stop.send(true).unwrap();
    server.await.unwrap();
    reader.close().await;
    store.close().await.unwrap();
}
