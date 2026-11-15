use anyhow::{Context, Result, ensure};
use app_server::{ApiState, router};
use app_store::{Store, reader::Reader};
use clap::Parser;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{Mutex, watch};

#[derive(Parser)]
#[command(version, about = "Vision Overlay loopback read API and durable SSE")]
struct Args {
    #[arg(long, default_value = "local-data/vision-overlay.sqlite3")]
    db: PathBuf,
    #[arg(long, default_value_t = 18788)]
    port: u16,
    #[arg(long)]
    token_file: Option<PathBuf>,
    #[arg(long)]
    regions: Option<PathBuf>,
    #[arg(long)]
    watch_root: Option<PathBuf>,
    #[arg(long)]
    no_relay: bool,
    /// Create an empty database on first portable launch.
    #[arg(long)]
    initialize: bool,
    /// Private launcher lifecycle marker; never exposed through the HTTP API.
    #[arg(long)]
    shutdown_file: Option<PathBuf>,
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.initialize {
        fs::create_dir_all(args.db.parent().context("Database directory missing")?)?;
    }
    ensure!(
        args.initialize || args.db.is_file(),
        "Database not found; initialize it with vision-store first"
    );
    // Reserve the endpoint before recovery can alter freshness in a running backend.
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, args.port)))
            .await
            .context("Cannot bind loopback API port")?;
    let port = listener.local_addr()?.port();
    // Reads use a read-only pool; authenticated ingestion has one serialized writer.
    let mut store = Store::open(&args.db).await?;
    store.recover_live().await?;
    let local = args.db.parent().context("Database directory missing")?;
    let token_file = args
        .token_file
        .unwrap_or_else(|| local.join("collector-token.txt"));
    if !token_file.exists() {
        let secret = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        private_file(&token_file, secret.as_bytes())?;
    }
    let token = fs::read_to_string(&token_file)?.trim().to_owned();
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid collector token file"
    );
    let regions = args
        .regions
        .unwrap_or_else(|| local.join("live-regions.json"));
    if !regions.exists() {
        let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for p in store.profiles().await?.as_array().unwrap() {
            candidates
                .entry(p["uid"].as_str().unwrap().into())
                .or_default()
                .insert(p["server_region"].as_str().unwrap().into());
        }
        let mapping: BTreeMap<String, String> = candidates
            .into_iter()
            .filter_map(|(uid, set)| {
                if set.len() == 1 {
                    Some((uid, set.into_iter().next().unwrap()))
                } else {
                    None
                }
            })
            .collect();
        private_file(&regions, &serde_json::to_vec_pretty(&mapping)?)?;
    }
    let writer = Arc::new(Mutex::new(store));
    let catalogs = app_server::bundled_catalog()
        .map(|payload| {
            app_server::catalog::Catalogs::open(local.join("catalogs"), payload).map(Arc::new)
        })
        .transpose()?;
    let reader = Reader::open(&args.db).await?;
    let (tx, rx) = watch::channel(false);
    if let Err(error) = app_server::diagnostics::init_logging(local) {
        eprintln!("Application log unavailable: {error}");
    }
    println!("Vision Overlay API: http://127.0.0.1:{port}/");
    println!("SQLite API and authenticated collector receiver. Ctrl+C stops the server.");
    println!(
        "Capture journals: only runs with delivery.json are watched. Global UID prefixes determine regions; live-regions.json overrides them. Unknown formats wait for an explicit mapping."
    );
    let app = router(ApiState {
        reader: reader.clone(),
        port,
        shutdown: rx.clone(),
        writer: Some(writer.clone()),
        token: Some(token.clone().into()),
        automation_dir: Some(local.join("auto-capture")),
        catalogs: catalogs.clone(),
    });
    let mut health_shutdown = rx.clone();
    let health_writer = writer.clone();
    let health = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            tokio::select! {c=health_shutdown.changed()=>{if c.is_err()||*health_shutdown.borrow(){break;}},_=tick.tick()=>{if let Err(e)=health_writer.lock().await.expire_leases().await{app_server::app_log!("Lease check failed: {e}");}}}
        }
    });
    let maintenance = if args.no_relay {
        None
    } else {
        Some(tokio::spawn(app_server::maintenance::run(
            writer.clone(),
            local.to_path_buf(),
            rx.clone(),
        )))
    };
    let wish_sync = if args.no_relay {
        None
    } else {
        Some(tokio::spawn(app_server::wish_sync::run(
            writer.clone(),
            catalogs.clone(),
            local.to_path_buf(),
            rx.clone(),
        )))
    };
    let relay = if args.no_relay {
        None
    } else {
        let root = args.watch_root.unwrap_or_else(|| local.to_path_buf());
        Some(tokio::spawn(app_server::relay::watch(
            root, regions, token, port, rx,
        )))
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let shutdown_file = args.shutdown_file;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = async {
                    loop {
                        if shutdown_file.as_ref().is_some_and(|p| p.is_file()) { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                } => {},
            }
            let _ = tx.send(true);
        })
        .await?;
    app_server::diagnostics::shutdown_cleanup().await;
    reader.close().await;
    let _ = health.await;
    if let Some(maintenance) = maintenance {
        let _ = maintenance.await;
    }
    if let Some(relay) = relay {
        let _ = relay.await;
    }
    if let Some(wish_sync) = wish_sync {
        let _ = wish_sync.await;
    }
    if let Ok(writer) = Arc::try_unwrap(writer) {
        writer.into_inner().close().await?;
    }
    Ok(())
}

fn private_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
