use app_server::{ApiState, router};
use app_store::{Store, reader::Reader};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

#[tokio::test]
async fn export_inspect_restore_rejects_cross_origin_and_retains_read_api() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("s.db");
    let store = Store::open(&path).await.unwrap();
    let writer = Arc::new(Mutex::new(store));
    let reader = Reader::open(&path).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, rx) = watch::channel(false);
    let mut shutdown = rx.clone();
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx,
        writer: Some(writer.clone()),
        token: Some("test".into()),
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
    let payload = json!({"settings":{"schema_version":1,"locale":"ru","selected_profile_id":null},"catalog":{"schema_version":1,"sources":{}}});
    assert_eq!(
        client
            .post(format!("{base}/api/v1/backups/export"))
            .json(&payload)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let exported = client
        .post(format!("{base}/api/v1/backups/export"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .json(&payload)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let inspected: Value = client
        .post(format!("{base}/api/v1/backups/inspect"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .body(exported.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inspected["manifest"]["format_version"], 1);
    let unconfirmed = client
        .post(format!("{base}/api/v1/backups/restore"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .body(exported.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(unconfirmed.status(), 403);
    let invalid = client
        .post(format!("{base}/api/v1/backups/restore"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .header("X-Vision-Restore", "replace-all")
        .header("X-Backup-Sha256", "bad")
        .body(exported.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    let restored: Value = client
        .post(format!("{base}/api/v1/backups/restore"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .header("X-Vision-Restore", "replace-all")
        .header("X-Backup-Sha256", inspected["sha256"].as_str().unwrap())
        .body(exported)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["restored"], true);
    assert!(std::path::Path::new(restored["rollback_path"].as_str().unwrap()).is_file());
    let state: Value = client
        .get(format!("{base}/api/v1/state"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(state["active_game_profile_id"].is_null());
    assert!(state["revision"].as_str().unwrap().parse::<i64>().unwrap() > 0);
    let capture = uuid::Uuid::new_v4().to_string();
    let mut normalizer = app_server::relay::Normalizer::new(&capture).unwrap();
    let old_events=normalizer.feed(&json!({"schema_version":1,"sequence":1,"session":"0","observed_unix_ms":"1000","source_elapsed_ms":0,"event":"source_config","data":{"mode":"live"}}),&std::collections::BTreeMap::new()).unwrap();
    assert!(
        writer
            .lock()
            .await
            .ingest(&capture, old_events)
            .await
            .unwrap_err()
            .to_string()
            .contains("predates")
    );
    let plan: Value = client
        .get(format!("{base}/api/v1/maintenance"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(plan["eligible_count"], 0);
    assert_eq!(
        client
            .post(format!("{base}/api/v1/maintenance/apply"))
            .json(&json!({"plan_id":plan["plan_id"]}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let cleared: Value = client
        .post(format!("{base}/api/v1/maintenance/apply"))
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .json(&json!({"plan_id":plan["plan_id"]}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared["removed"], 0);
    stop.send(true).unwrap();
    task.await.unwrap();
    reader.close().await;
    Arc::try_unwrap(writer)
        .ok()
        .unwrap()
        .into_inner()
        .close()
        .await
        .unwrap();
}
