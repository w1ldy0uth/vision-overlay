use app_server::{ApiState, router};
use app_store::{Store, reader::Reader};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Read},
};
use tokio::sync::watch;
#[tokio::test]
async fn diagnostics_exports_only_selected_capture_requires_origin_and_streams_one_use_zip() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("s.db");
    let mut store = Store::open(&path).await.unwrap();
    let capture = uuid::Uuid::new_v4().to_string();
    let profile = uuid::Uuid::new_v4().to_string();
    let foreign = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO profiles(profile_id,game_id,account_namespace,server_region,uid) VALUES(?,'genshin','global','os_euro','700000001'),(?,'genshin','global','os_asia','800000001')").bind(&profile).bind(&foreign).execute(store.maintenance_db()).await.unwrap();
    sqlx::query("INSERT INTO captures(capture_id,artifact_sha256,binding_json,source_json,adapter_revision,event_count,terminal_status) VALUES(?,'fixture','{}','{}','test',1,'stopped_user')").bind(&capture).execute(store.maintenance_db()).await.unwrap();
    sqlx::query("INSERT INTO epochs(capture_id,epoch,profile_id,evidence) VALUES(?,'1',?,'test')")
        .bind(&capture)
        .bind(&profile)
        .execute(store.maintenance_db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO events(capture_id,source_sequence,epoch,profile_id,kind,event_json,projection_result) VALUES(?,1,'1',?,'fixture',?,'applied')").bind(&capture).bind(&profile).bind(json!({"kind":"fixture","authkey":"DO_NOT_EXPORT_AUTH"}).to_string()).execute(store.maintenance_db()).await.unwrap();
    sqlx::query("INSERT INTO snapshots(snapshot_id,capture_id,source_sequence,profile_id,collection,scope,snapshot_json,value_format) VALUES(?,?,1,?,'material_stacks','full',?,'legacy_capture_v1')").bind(uuid::Uuid::new_v4().to_string()).bind(&capture).bind(&profile).bind(json!({"item":{"item_id":100011,"count":315}}).to_string()).execute(store.maintenance_db()).await.unwrap();
    fs::write(temp.path().join("gi.json"), "DO_NOT_EXPORT_KEYS").unwrap();
    let run = temp.path().join("realtime-test");
    fs::create_dir(&run).unwrap();
    fs::write(
        run.join("delivery.json"),
        json!({"capture_id":capture}).to_string(),
    )
    .unwrap();
    fs::write(
        run.join("capture.jsonl"),
        "{\"event\":\"status\",\"seed\":\"DO_NOT_EXPORT_SEED\"}\nunfinished",
    )
    .unwrap();
    let reader = Reader::open(&path).await.unwrap();
    let facts = reader.diagnostic_capture(&capture).await.unwrap();
    assert_eq!(facts["snapshots"][0]["entities"]["item"]["count"], 315);
    assert_eq!(facts["profiles"].as_array().unwrap().len(), 1);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = watch::channel(false);
    let mut shutdown = rx.clone();
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx,
        writer: None,
        token: None,
        automation_dir: Some(temp.path().join("auto-capture")),
        catalogs: None,
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await
            .unwrap();
    });
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let payload = json!({"capture_id":capture,"include_packets":false,"description":"test authkey=DO_NOT_EXPORT_DESC"});
    assert_eq!(
        client
            .post(format!("{base}/api/v1/diagnostics/export"))
            .json(&payload)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let bad = client
        .post(format!("{base}/api/v1/diagnostics/export"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .json(&json!({"capture_id":"../gi.json"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let exported = client
        .post(format!("{base}/api/v1/diagnostics/export"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(exported.status(), 200);
    let prepared: Value = exported.json().await.unwrap();
    let url = format!("{}{}", base, prepared["download_url"].as_str().unwrap());
    assert_eq!(
        client
            .get(&url)
            .header("Origin", "https://example.com")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/zip");
    let length = response.content_length().unwrap();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(length, bytes.len() as u64);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 404);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert!(zip.by_name("gi.json").is_err());
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert!(
            !text.contains("DO_NOT_EXPORT"),
            "{} leaked a secret",
            file.name()
        );
        assert!(!text.contains("800000001"));
    }
    assert_eq!(prepared["manifest"]["raw"]["included"], false);
    tx.send(true).unwrap();
    task.await.unwrap();
    reader.close().await;
    drop(store);
}
