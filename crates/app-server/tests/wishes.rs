use app_server::{ApiState, router};
use app_store::{Store, reader::Reader};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

#[tokio::test]
async fn wish_import_requires_controls_is_profile_scoped_and_notifies_sse_log() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("test.db");
    let mut store = Store::open(&path).await.unwrap();
    let input = [
        json!({"schema_version":1,"session":"0","sequence":1,"event":"source_config","data":{"mode":"live"}}),
        json!({"schema_version":1,"session":"1","sequence":2,"observed_unix_ms":"1000","event":"inventory_snapshot","data":{"completeness":"full","materials":[],"items":[]}}),
        json!({"schema_version":1,"session":"1","sequence":3,"observed_unix_ms":"1001","event":"status","data":{"state":"stopped_game_exited"}}),
    ];
    let bytes = input
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let bindings = std::collections::BTreeMap::from([(
        "1".into(),
        app_store::legacy::Identity {
            uid: "700000001".into(),
            server_region: "os_euro".into(),
        },
    )]);
    store.import(bytes.as_bytes(), &bindings).await.unwrap();
    let profile = store.profiles().await.unwrap()[0]["profile_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reader = Reader::open(&path).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, rx) = watch::channel(false);
    let mut shutdown = rx.clone();
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx,
        writer: Some(Arc::new(Mutex::new(store))),
        token: None,
        automation_dir: None,
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
    let url = format!("{base}/api/v1/profiles/{profile}/wishes");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let archive = json!({"schema_version":1,"source":"hoyoverse_wish_api","game_id":"genshin","account_namespace":"hk4e_global","uid":"700000001","server_region":"os_euro","fetched_at":"2026-11-12T15:50:00Z","coverage":{"all_available_pages_fetched":true},"streams":[{"gacha_type":"200","pages":2}],"records":[{"id":"12345678901234567890","uid":"700000001","gacha_type":"200","count":"1","time":"2026-11-12 16:00:00","name":"Меч","rank_type":"3"}]});
    assert_eq!(
        client
            .post(&url)
            .json(&archive)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let result: Value = client
        .post(&url)
        .header("Origin", &base)
        .header("X-Vision-Control", "1")
        .json(&archive)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["added"], 1);
    let data: Value = client.get(&url).send().await.unwrap().json().await.unwrap();
    assert_eq!(data["records"][0]["id"], "12345678901234567890");
    assert_eq!(data["live_enabled"], false);
    let changes = reader.changes(0, 100).await.unwrap();
    assert!(changes.iter().any(|c| {
        c["topics"]
            .as_array()
            .is_some_and(|a| a.contains(&json!("wish_history")))
    }));
    stop.send(true).unwrap();
    task.await.unwrap();
    reader.close().await;
}
