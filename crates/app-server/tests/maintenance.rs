use app_server::{maintenance, relay::Normalizer};
use app_store::Store;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

async fn fixture(
    writer: &Arc<Mutex<Store>>,
    root: &Path,
    index: u32,
    count: u32,
) -> (String, String) {
    fixture_with_coverage(writer, root, index, count, true).await
}

async fn fixture_with_coverage(
    writer: &Arc<Mutex<Store>>,
    root: &Path,
    index: u32,
    count: u32,
    complete: bool,
) -> (String, String) {
    let capture = Uuid::new_v4().to_string();
    let relative = format!("realtime-test-{index}");
    let dir = root.join(&relative);
    fs::create_dir_all(dir.join("delivery")).unwrap();
    let mut data = vec![
        ("source_config", json!({"mode":"live"})),
        ("login_identity", json!({"uid":"700000001"})),
        (
            "player_profile_snapshot",
            json!({"profile":{"uid":"700000001","nickname":"Test"}}),
        ),
        ("currency_snapshot", json!({"item_id":201,"count":0})),
        ("currency_snapshot", json!({"item_id":202,"count":10})),
        (
            "adventure_snapshot",
            json!({"current":{"rank":12,"exp":430}}),
        ),
        (
            "inventory_snapshot",
            json!({"completeness":"full","materials":[{"guid":"42","item_id":100011,"count":count}],"items":[]}),
        ),
        (
            "team_snapshot",
            json!({"teams":[{"team_id":1,"name":"Team","members":[],"is_active":null}]}),
        ),
        (
            "character_snapshot",
            json!({"profiles":[{"guid":"77","avatar_id":10000035}]}),
        ),
        (
            "achievement_snapshot",
            json!({"records":[{"achievement_id":80001,"status_code":1,"total_progress":10,"current_progress":2}]}),
        ),
        (
            "material_changed",
            json!({"quantity_semantics":"absolute","change":{"guid":"42","item_id":100011,"previous_count":count,"count":count+1,"delta":1,"new_entry":false}}),
        ),
        ("status", json!({"state":"stopped_game_exited"})),
    ];
    if !complete {
        data.retain(|(kind, _)| *kind != "achievement_snapshot");
    }
    let mut normalizer = Normalizer::new(&capture).unwrap();
    let regions = std::collections::BTreeMap::from([("700000001".into(), "os_euro".into())]);
    let mut native = vec![];
    let mut bytes = vec![];
    let source_length = data.len();
    for (i, (kind, data)) in data.into_iter().enumerate() {
        let row = json!({"schema_version":1,"sequence":i+1,"session":if i==0{"0"}else{"1"},"observed_unix_ms":(app_store::ingest::now_ms()+i as i64).to_string(),"source_elapsed_ms":i*100,"event":kind,"data":data});
        native.extend(normalizer.feed(&row, &regions).unwrap());
        serde_json::to_writer(&mut bytes, &row).unwrap();
        bytes.push(b'\n');
    }
    let trailer = json!({"event":"decoder_warning_counts","data":{},"schema_version":1,"session":"1","sequence":source_length+1,"observed_unix_ms":app_store::ingest::now_ms().to_string(),"source_elapsed_ms":source_length*100});
    serde_json::to_writer(&mut bytes, &trailer).unwrap();
    bytes.push(b'\n');
    let sequence = native.len();
    writer.lock().await.ingest(&capture, native).await.unwrap();
    fs::write(dir.join("capture.jsonl"), &bytes).unwrap();
    fs::write(
        dir.join("delivery.json"),
        json!({"schema_version":1,"capture_id":capture,"log_file":"capture.jsonl"}).to_string(),
    )
    .unwrap();
    fs::write(dir.join("delivery/ack.json"),json!({"capture_id":capture,"acked_sequence":sequence,"source_offset":bytes.len(),"source_sha256":format!("{:x}",Sha256::digest(&bytes))}).to_string()).unwrap();
    fs::write(dir.join("delivery/outbox.jsonl"), []).unwrap();
    (capture, relative)
}
async fn setup(root: &Path) -> Arc<Mutex<Store>> {
    let mut store = Store::open(&root.join("s.db")).await.unwrap();
    sqlx::query("INSERT INTO retention_meta VALUES(1,1)")
        .execute(store.maintenance_db())
        .await
        .unwrap();
    Arc::new(Mutex::new(store))
}
#[tokio::test]
async fn points_include_live_updates_auto_backups_rotate_and_cleanup_rechecks_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let writer = setup(root).await;
    let mut first = None;
    for i in 1..=4 {
        let capture = fixture(&writer, root, i, 100 + i).await;
        if first.is_none() {
            first = Some(capture);
        }
        maintenance::cycle(writer.clone(), root.into())
            .await
            .unwrap();
    }
    {
        let mut store = writer.lock().await;
        let points = store.recovery_points().await.unwrap();
        assert_eq!(points.as_array().unwrap().len(), 3);
        let state: String = sqlx::query_scalar(
            "SELECT state_json FROM recovery_points ORDER BY observed_ms DESC LIMIT 1",
        )
        .fetch_one(store.maintenance_db())
        .await
        .unwrap();
        let state: Value = serde_json::from_str(&state).unwrap();
        assert_eq!(state["collections"]["material_stacks"]["42"]["count"], 105);
        let copies: i64 =
            sqlx::query_scalar("SELECT count(*) FROM automatic_backups WHERE state='verified'")
                .fetch_one(store.maintenance_db())
                .await
                .unwrap();
        assert_eq!(copies, 3);
    }
    let (first_id, first_dir) = first.unwrap();
    let plan = maintenance::preview(writer.clone(), root.into())
        .await
        .unwrap();
    assert!(
        plan["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["capture_id"] == first_id && a["eligible"] == true)
    );
    let ack = root.join(&first_dir).join("delivery/ack.json");
    let original = fs::read(&ack).unwrap();
    let mut altered: Value = serde_json::from_slice(&original).unwrap();
    altered["source_offset"] = json!(0);
    fs::write(&ack, altered.to_string()).unwrap();
    assert!(
        maintenance::apply(
            writer.clone(),
            root.into(),
            plan["plan_id"].as_str().unwrap().into()
        )
        .await
        .is_err()
    );
    assert!(root.join(&first_dir).is_dir());
    fs::write(ack, original).unwrap();
    // A manually saved archive inside a managed folder protects the whole folder.
    let manual = root.join(&first_dir).join("manual.visionbackup");
    fs::write(&manual, b"manual").unwrap();
    let protected = maintenance::preview(writer.clone(), root.into())
        .await
        .unwrap();
    assert!(
        !protected["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["capture_id"] == first_id && a["eligible"] == true)
    );
    fs::remove_file(manual).unwrap();
    let fresh = maintenance::preview(writer.clone(), root.into())
        .await
        .unwrap();
    let result = maintenance::apply(
        writer.clone(),
        root.into(),
        fresh["plan_id"].as_str().unwrap().into(),
    )
    .await
    .unwrap();
    assert_eq!(result["removed"], 1);
    assert!(!root.join(&first_dir).exists());
    {
        let mut store = writer.lock().await;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE capture_id=?")
            .bind(first_id)
            .fetch_one(store.maintenance_db())
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(store.maintenance_db())
                .await
                .unwrap()
                .is_empty()
        );
    }
    // Identical final user facts must not create another archive after a new capture.
    fixture(&writer, root, 5, 104).await;
    maintenance::cycle(writer.clone(), root.into())
        .await
        .unwrap();
    {
        let mut store = writer.lock().await;
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM automatic_backups WHERE state='verified'")
                .fetch_one(store.maintenance_db())
                .await
                .unwrap();
        assert_eq!(n, 3);
        let result: String = sqlx::query_scalar(
            "SELECT result FROM automatic_backup_captures WHERE result='unchanged' LIMIT 1",
        )
        .fetch_one(store.maintenance_db())
        .await
        .unwrap();
        assert_eq!(result, "unchanged");
    }
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}
#[tokio::test]
async fn bootstrap_pins_history_and_unsafe_paths_are_never_retired() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("managed");
    fs::create_dir(&root).unwrap();
    let writer = Arc::new(Mutex::new(Store::open(&root.join("s.db")).await.unwrap()));
    fs::create_dir(root.join("realtime-legacy")).unwrap();
    fs::write(root.join("realtime-legacy/notes.txt"), b"research").unwrap();
    fs::write(temp.path().join("outside.txt"), b"keep").unwrap();
    writer
        .lock()
        .await
        .register_artifact(
            &Uuid::new_v4().to_string(),
            "diagnostics",
            "../outside.txt",
            None,
            false,
        )
        .await
        .unwrap();
    let plan = maintenance::preview(writer.clone(), root.clone())
        .await
        .unwrap();
    assert_eq!(plan["eligible_count"], 0);
    let historical = plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["path"] == "realtime-legacy")
        .unwrap();
    assert_eq!(historical["pinned"], true);
    maintenance::apply(
        writer.clone(),
        root.clone(),
        plan["plan_id"].as_str().unwrap().into(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(temp.path().join("outside.txt")).unwrap(), b"keep");
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}
#[tokio::test]
async fn failed_new_backup_does_not_rotate_verified_copies() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let writer = setup(root).await;
    for i in 1..=3 {
        fixture(&writer, root, i, 200 + i).await;
        maintenance::cycle(writer.clone(), root.into())
            .await
            .unwrap();
    }
    let files = fs::read_dir(root.join("automatic-backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 3);
    fixture(&writer, root, 4, 204).await;
    sqlx::query(
        "CREATE TRIGGER unexpected AFTER INSERT ON profiles BEGIN DELETE FROM profiles; END",
    )
    .execute(writer.lock().await.maintenance_db())
    .await
    .unwrap();
    assert!(
        maintenance::cycle(writer.clone(), root.into())
            .await
            .is_err()
    );
    assert!(files.iter().all(|path| path.is_file()));
    sqlx::query("DROP TRIGGER unexpected")
        .execute(writer.lock().await.maintenance_db())
        .await
        .unwrap();
    maintenance::cycle(writer.clone(), root.into())
        .await
        .unwrap();
    assert_eq!(
        fs::read_dir(root.join("automatic-backups"))
            .unwrap()
            .count(),
        3
    );
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn incomplete_login_does_not_evict_points_or_rotate_backups() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let writer = setup(root).await;
    for i in 1..=3 {
        fixture(&writer, root, i, 300 + i).await;
        maintenance::cycle(writer.clone(), root.into())
            .await
            .unwrap();
    }
    let before = writer.lock().await.recovery_points().await.unwrap();
    fixture_with_coverage(&writer, root, 4, 999, false).await;
    maintenance::cycle(writer.clone(), root.into())
        .await
        .unwrap();
    let mut store = writer.lock().await;
    assert_eq!(store.recovery_points().await.unwrap(), before);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM automatic_backups WHERE state='verified'")
            .fetch_one(store.maintenance_db())
            .await
            .unwrap();
    assert_eq!(count, 3);
    let skipped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM automatic_backup_captures WHERE result='incomplete_capture'",
    )
    .fetch_one(store.maintenance_db())
    .await
    .unwrap();
    assert_eq!(skipped, 1);
    drop(store);
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}
#[tokio::test]
async fn interrupted_cleanup_is_repaired_without_overwriting_user_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let writer = setup(root).await;
    let id = Uuid::new_v4().to_string();
    let trash = format!(".retention-trash/{id}");
    fs::create_dir_all(root.join(&trash)).unwrap();
    fs::write(root.join(&trash).join("keep.txt"), b"keep").unwrap();
    sqlx::query("INSERT INTO retention_jobs VALUES(?,'prepared',?)")
        .bind(&id)
        .bind(json!({"path":"realtime-repaired","trash":trash}).to_string())
        .execute(writer.lock().await.maintenance_db())
        .await
        .unwrap();
    maintenance::cycle(writer.clone(), root.into())
        .await
        .unwrap();
    assert_eq!(
        fs::read(root.join("realtime-repaired/keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!root.join(&trash).exists());
    let id = Uuid::new_v4().to_string();
    let trash = format!(".retention-trash/{id}");
    fs::create_dir_all(root.join(&trash)).unwrap();
    fs::write(root.join(&trash).join("retired.txt"), b"retired").unwrap();
    sqlx::query("INSERT INTO retention_jobs VALUES(?,'committed',?)")
        .bind(&id)
        .bind(json!({"path":"realtime-retired","trash":trash}).to_string())
        .execute(writer.lock().await.maintenance_db())
        .await
        .unwrap();
    maintenance::cycle(writer.clone(), root.into())
        .await
        .unwrap();
    assert!(!root.join(&trash).exists());
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}
