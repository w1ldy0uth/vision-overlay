use app_server::{ApiState, router};
use app_store::{
    Store,
    legacy::{Bindings, Identity},
    reader::Reader,
};
use serde_json::{Value, json};
use sqlx::Connection;
use std::{path::Path, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

struct Server {
    url: String,
    reader: Reader,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Server {
    async fn start(path: &Path) -> Self {
        let reader = Reader::open(path).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (stop, rx) = watch::channel(false);
        let mut shutdown = rx.clone();
        let app = router(ApiState {
            reader: reader.clone(),
            port,
            shutdown: rx,
            writer: None,
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
        Self {
            url: format!("http://127.0.0.1:{port}"),
            reader,
            stop,
            task,
        }
    }
    async fn close(self) {
        let _ = self.stop.send(true);
        tokio::time::timeout(Duration::from_secs(3), self.task)
            .await
            .unwrap()
            .unwrap();
        self.reader.close().await;
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

#[tokio::test]
async fn capture_control_requires_same_origin_json_and_closed_request_shape() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("s.db");
    let store = Store::open(&path).await.unwrap();
    let server = Server::start(&path).await;
    let c = client();
    let url = format!("{}/api/v1/capture/control", server.url);
    let body = json!({"request_id":"5a7d217e-64c7-42e4-96f8-cac5c9f11916","action":"start","target_run_id":null});
    assert_eq!(c.post(&url).json(&body).send().await.unwrap().status(), 403);
    assert_eq!(
        c.post(&url)
            .header("origin", "https://example.org")
            .header("x-vision-control", "1")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let invalid = json!({"request_id":"5a7d217e-64c7-42e4-96f8-cac5c9f11916","action":"start","arguments":"powershell"});
    assert_eq!(
        c.post(&url)
            .header("origin", &server.url)
            .header("x-vision-control", "1")
            .json(&invalid)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        c.post(&url)
            .header("origin", &server.url)
            .header("x-vision-control", "1")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    server.close().await;
    store.close().await.unwrap();
}

#[tokio::test]
async fn newest_history_pages_are_ordered_and_assets_do_not_expose_local_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::open(&path).await.unwrap();
    store.import(&archive(1000, 10), &bindings()).await.unwrap();
    let profile = store.profiles().await.unwrap()[0]["profile_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let server = Server::start(&path).await;
    let c = client();
    let url = format!(
        "{}/api/v1/profiles/{profile}/events?order=desc&limit=1",
        server.url
    );
    let first = json_get(&c, url.clone()).await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = json_get(&c, format!("{url}&cursor={cursor}")).await;
    assert!(
        first["items"][0]["revision"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > next["items"][0]["revision"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
    );
    assert_eq!(
        c.get(format!("{}/api/v1/captures?order=desc", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        c.get(format!(
            "{}/api/v1/profiles/{profile}/events?order=invalid",
            server.url
        ))
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    for path in [
        "/collector-token.txt",
        "/local-data/vision-overlay.sqlite3",
        "/assets/missing.js",
        "/api/unknown",
    ] {
        assert_eq!(
            c.get(format!("{}{path}", server.url))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let root = c.get(format!("{}/", server.url)).send().await.unwrap();
    // Rust-only clean builds deliberately expose a clear 503 until web is built.
    if root.status() == 200 {
        assert!(
            root.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(
            root.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("script-src 'self'")
        );
        let html = root.text().await.unwrap();
        assert!(html.contains("Vision Overlay"));
    } else {
        assert_eq!(root.status(), 503);
    }
    server.close().await;
    store.close().await.unwrap();
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

#[tokio::test]
async fn unread_sse_clients_do_not_pin_read_pool_or_block_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let result = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let before = result["revision"].as_str().unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let mut streams = Vec::new();
    // More open streams than the read pool size. None of their bodies is consumed.
    for _ in 0..10 {
        let r = tokio::time::timeout(
            Duration::from_secs(3),
            c.get(format!("{}/api/v1/events?after={before}", s.url))
                .send(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(r.status(), 200);
        streams.push(r);
    }
    tokio::time::timeout(
        Duration::from_secs(3),
        writer.import(&archive(2000, 11), &bindings()),
    )
    .await
    .unwrap()
    .unwrap();
    let state = tokio::time::timeout(
        Duration::from_secs(3),
        json_get(&c, format!("{}/api/v1/state", s.url)),
    )
    .await
    .unwrap();
    assert!(
        state["revision"].as_str().unwrap().parse::<i64>().unwrap()
            > before.parse::<i64>().unwrap()
    );
    drop(streams);
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn rolled_back_write_produces_no_sse_notification_or_revision_gap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let result = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let before = result["revision"].as_str().unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let mut response = c
        .get(format!("{}/api/v1/events?after={before}", s.url))
        .send()
        .await
        .unwrap();
    let mut raw = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(&path),
    )
    .await
    .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_import BEFORE INSERT ON events WHEN NEW.source_sequence=2 BEGIN SELECT RAISE(ABORT,'injected failure'); END;").execute(&mut raw).await.unwrap();
    assert!(
        writer
            .import(&archive(2000, 11), &bindings())
            .await
            .is_err()
    );
    assert!(
        s.reader
            .changes(before.parse().unwrap(), 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        json_get(&c, format!("{}/api/v1/state", s.url)).await["revision"],
        before
    );
    sqlx::query("DROP TRIGGER fail_import")
        .execute(&mut raw)
        .await
        .unwrap();
    raw.close().await.unwrap();
    writer
        .import(&archive(2000, 12), &bindings())
        .await
        .unwrap();
    let f = frames(&mut response, 1).await;
    assert_eq!(
        f[0].1["revision"],
        (before.parse::<i64>().unwrap() + 1).to_string()
    );
    drop(response);
    s.close().await;
    writer.close().await.unwrap();
}
fn archive(time: u64, count: u32) -> Vec<u8> {
    let weapon = json!({"guid":"18446744073709551615","kind":"weapon","item_id":11101,"level":1});
    let avatar = json!({"guid":"42","avatar_id":10000035,"equip_guids":["18446744073709551615","99"],"trial_info_present":false});
    let data = [
        ("source_config", json!({"mode":"live"})),
        (
            "inventory_snapshot",
            json!({"materials":[{"guid":"41","item_id":100002,"count":count},{"guid":"43","item_id":100002,"count":2}],"items":[weapon],"completeness":"unverified"}),
        ),
        ("character_snapshot", json!({"profiles":[avatar]})),
        ("status", json!({"state":"stopped_game_exited"})),
    ];
    let mut bytes = Vec::new();
    for (i, (kind, d)) in data.into_iter().enumerate() {
        let e = json!({"schema_version":1,"sequence":i+1,"session":if i==0{"0"}else{"1"},"observed_unix_ms":(time+i as u64).to_string(),"event":kind,"data":d});
        serde_json::to_writer(&mut bytes, &e).unwrap();
        bytes.push(b'\n');
    }
    bytes
}
async fn json_get(c: &reqwest::Client, url: String) -> Value {
    let r = c.get(url).send().await.unwrap();
    assert!(r.status().is_success(), "HTTP {}", r.status());
    r.json().await.unwrap()
}
async fn frames(response: &mut reqwest::Response, n: usize) -> Vec<(String, Value)> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let mut buffer = String::new();
        let mut found = Vec::new();
        while found.len() < n {
            let chunk = response.chunk().await.unwrap().expect("SSE ended early");
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            buffer = buffer.replace("\r\n", "\n");
            while let Some(end) = buffer.find("\n\n") {
                let block = buffer[..end].to_owned();
                buffer.drain(..end + 2);
                let event = block.lines().find_map(|l| l.strip_prefix("event: "));
                let data = block.lines().find_map(|l| l.strip_prefix("data: "));
                if let (Some(e), Some(d)) = (event, data) {
                    found.push((e.into(), serde_json::from_str(d).unwrap()));
                }
            }
        }
        found
    })
    .await
    .expect("Timed out waiting for SSE data")
}

#[tokio::test]
async fn api_reads_profiles_inventory_totals_links_snapshots_and_filtered_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let result = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let profile = result["profiles"]["1"].as_str().unwrap();
    let server = Server::start(&path).await;
    let c = client();
    let state = json_get(&c, format!("{}/api/v1/state", server.url)).await;
    assert_eq!(state["active_game_profile_id"], Value::Null);
    assert_eq!(state["profiles"][0]["uid"], "123456789");
    let full = json_get(&c, format!("{}/api/v1/profiles/{profile}", server.url)).await;
    assert_eq!(full["material_totals"][0]["count"], "12");
    let page = json_get(
        &c,
        format!("{}/api/v1/profiles/{profile}/inventory?limit=1", server.url),
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    let cursor = page["next_cursor"].as_str().unwrap();
    let page2 = json_get(
        &c,
        format!(
            "{}/api/v1/profiles/{profile}/inventory?limit=1&cursor={cursor}",
            server.url
        ),
    )
    .await;
    assert_ne!(
        page["items"][0]["entity_key"],
        page2["items"][0]["entity_key"]
    );
    let totals = json_get(
        &c,
        format!(
            "{}/api/v1/profiles/{profile}/inventory/totals?q=100002",
            server.url
        ),
    )
    .await;
    assert_eq!(totals["items"][0]["count"], "12");
    let chars = json_get(
        &c,
        format!("{}/api/v1/profiles/{profile}/characters", server.url),
    )
    .await;
    assert_eq!(
        chars["items"][0]["equipment"][0]["reference"]["resolved"],
        true
    );
    assert_eq!(
        chars["items"][0]["equipment"][1]["reference"]["resolved"],
        false
    );
    let captures = json_get(&c, format!("{}/api/v1/captures", server.url)).await;
    let capture = captures["items"][0]["capture_id"].as_str().unwrap();
    let detail = json_get(&c, format!("{}/api/v1/captures/{capture}", server.url)).await;
    assert_eq!(detail["terminal_status"], "stopped_game_exited");
    let snaps = json_get(
        &c,
        format!(
            "{}/api/v1/captures/{capture}/snapshots?collection=material_stacks",
            server.url
        ),
    )
    .await;
    let id = snaps["items"][0]["snapshot_id"].as_str().unwrap();
    let snap = json_get(&c, format!("{}/api/v1/snapshots/{id}", server.url)).await;
    assert_eq!(snap["entities"]["41"]["count"], 10);
    let history = json_get(
        &c,
        format!(
            "{}/api/v1/profiles/{profile}/events?kind=inventory_snapshot",
            server.url
        ),
    )
    .await;
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    assert_eq!(history["items"][0]["snapshot_payload_omitted"], true);
    drop(c);
    server.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn current_pagination_detects_revision_change_and_cursor_filter_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let result = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let id = result["profiles"]["1"].as_str().unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let first = json_get(
        &c,
        format!("{}/api/v1/profiles/{id}/inventory?limit=1", s.url),
    )
    .await;
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        c.get(format!(
            "{}/api/v1/profiles/{id}/inventory?limit=1&q=41&cursor={cursor}",
            s.url
        ))
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    writer
        .import(&archive(2000, 20), &bindings())
        .await
        .unwrap();
    let r = c
        .get(format!(
            "{}/api/v1/profiles/{id}/inventory?limit=1&cursor={cursor}",
            s.url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "view_changed"
    );
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn sse_covers_get_subscribe_race_two_clients_and_last_event_id_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let state = json_get(&c, format!("{}/api/v1/state", s.url)).await;
    let before = state["revision"].as_str().unwrap().parse::<i64>().unwrap();
    // Commit deliberately happens between GET state and the two subscriptions.
    writer
        .import(&archive(2000, 20), &bindings())
        .await
        .unwrap();
    let url = format!("{}/api/v1/events?after={before}", s.url);
    let mut a = c.get(&url).send().await.unwrap();
    let mut b = c.get(&url).send().await.unwrap();
    assert_eq!(a.status(), 200);
    assert_eq!(a.headers()["content-type"], "text/event-stream");
    let ea = frames(&mut a, 4).await;
    let eb = frames(&mut b, 4).await;
    assert_eq!(ea, eb);
    assert_eq!(ea[0].1["revision"], (before + 1).to_string());
    drop(a);
    drop(b);
    let mut resumed = c
        .get(format!("{}/api/v1/events?after=0", s.url))
        .header("last-event-id", (before + 1).to_string())
        .send()
        .await
        .unwrap();
    let e = frames(&mut resumed, 1).await;
    assert_eq!(e[0].1["revision"], (before + 2).to_string());
    drop(resumed);
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn active_stream_receives_later_commits_and_duplicate_import_adds_no_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let bytes = archive(1000, 10);
    let result = writer.import(&bytes, &bindings()).await.unwrap();
    let before = result["revision"].as_str().unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let mut response = c
        .get(format!("{}/api/v1/events?after={before}", s.url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        writer.import(&bytes, &bindings()).await.unwrap()["duplicate"],
        true
    );
    let same = json_get(&c, format!("{}/api/v1/state", s.url)).await;
    assert_eq!(same["revision"], before);
    writer
        .import(&archive(2000, 11), &bindings())
        .await
        .unwrap();
    let e = frames(&mut response, 1).await;
    assert_eq!(
        e[0].1["revision"],
        (before.parse::<i64>().unwrap() + 1).to_string()
    );
    drop(response);
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn durable_replay_survives_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let result = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let before = result["revision"].as_str().unwrap();
    let s = Server::start(&path).await;
    s.close().await;
    writer
        .import(&archive(2000, 12), &bindings())
        .await
        .unwrap();
    let s = Server::start(&path).await;
    let mut response = client()
        .get(format!("{}/api/v1/events?after={before}", s.url))
        .send()
        .await
        .unwrap();
    assert_eq!(
        frames(&mut response, 1).await[0].1["revision"],
        (before.parse::<i64>().unwrap() + 1).to_string()
    );
    drop(response);
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn future_and_pruned_cursors_require_resync_before_or_during_stream() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut writer = Store::open(&path).await.unwrap();
    let r = writer
        .import(&archive(1000, 10), &bindings())
        .await
        .unwrap();
    let before = r["revision"].as_str().unwrap().parse::<i64>().unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let r = c
        .get(format!("{}/api/v1/events?after=999", s.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "resync_required"
    );
    let mut response = c
        .get(format!("{}/api/v1/events?after={before}", s.url))
        .send()
        .await
        .unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut raw = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    // Retention overtakes the open stream atomically, without a timing race.
    sqlx::raw_sql(
        "BEGIN; UPDATE store_meta SET revision=revision+1; DELETE FROM change_log; COMMIT;",
    )
    .execute(&mut raw)
    .await
    .unwrap();
    raw.close().await.unwrap();
    let f = frames(&mut response, 1).await;
    assert_eq!(f[0].0, "resync_required");
    drop(response);
    assert_eq!(
        c.get(format!("{}/api/v1/events?after=0", s.url))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    s.close().await;
    writer.close().await.unwrap();
}

#[tokio::test]
async fn host_origin_methods_and_errors_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let store = Store::open(&path).await.unwrap();
    let s = Server::start(&path).await;
    let c = client();
    let url = format!("{}/api/v1/state", s.url);
    assert_eq!(
        c.get(&url)
            .header("host", "evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        c.get(&url)
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        c.get(&url)
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(c.post(&url).send().await.unwrap().status(), 405);
    let r = c.get(&url).send().await.unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none());
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(
        c.get(format!("{}/api/v1/profiles/invalid/inventory", s.url))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        c.get(format!(
            "{}/api/v1/profiles/00000000-0000-0000-0000-000000000000/inventory",
            s.url
        ))
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    assert_eq!(
        c.get(format!("{}/api/v1/captures?limit=0", s.url))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        c.get(format!("{}/api/v1/captures?unknown=value", s.url))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        c.get(format!("{}/api/v1/events?after=01", s.url))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    s.close().await;
    store.close().await.unwrap();
}
