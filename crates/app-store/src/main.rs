use anyhow::{Context, Result, ensure};
use app_store::{
    Store,
    legacy::{self, Bindings, Identity},
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Offline SQLite inventory archive; does not start capture or a web server")]
struct Args {
    #[arg(long, default_value = "local-data/vision-overlay.sqlite3")]
    db: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Import a finalized UTF-8 live JSONL. Never infer the server from UID.
    Import {
        input: PathBuf,
        #[arg(long, requires = "region", conflicts_with = "bindings")]
        uid: Option<String>,
        #[arg(long, requires = "uid", conflicts_with = "bindings")]
        region: Option<String>,
        /// JSON map from legacy session string to {uid, server_region}.
        #[arg(long,conflicts_with_all=["uid","region"])]
        bindings: Option<PathBuf>,
    },
    Backup {
        output: PathBuf,
    },
    InspectBackup {
        input: PathBuf,
    },
    RestoreBackup {
        input: PathBuf,
    },
    Profiles,
    Show {
        profile_id: String,
    },
    History {
        profile_id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Command::InspectBackup { input } = &args.command {
        let archive = app_store::backup::validate(input).await?;
        println!("{}", serde_json::to_string_pretty(&archive.manifest)?);
        return Ok(());
    }
    if matches!(args.command, Command::RestoreBackup { .. }) {
        ensure!(
            !args.db.exists(),
            "CLI restore requires a new database path; use the UI to replace an installation"
        );
    }
    let mut store = Store::open(&args.db).await?;
    let result = match args.command {
        Command::Import {
            input,
            uid,
            region,
            bindings,
        } => {
            ensure!(
                std::fs::metadata(&input)?.len() <= 512 * 1024 * 1024,
                "Archive exceeds size limit"
            );
            let bytes = std::fs::read(&input).context("Cannot read archive")?;
            let binding: Bindings = if let Some(path) = bindings {
                serde_json::from_slice(&std::fs::read(path)?)?
            } else {
                let archive = legacy::parse(&bytes)?;
                if archive.data_epochs.is_empty() {
                    Bindings::new()
                } else {
                    ensure!(
                        archive.data_epochs.len() == 1,
                        "Multiple game-data epochs: use --bindings with an explicit profile for each epoch"
                    );
                    Bindings::from([(
                        archive.data_epochs.into_iter().next().unwrap(),
                        Identity {
                            uid: uid.context("--uid and --region are required")?,
                            server_region: region.context("--region is required")?,
                        },
                    )])
                }
            };
            store.import(&bytes, &binding).await?
        }
        Command::Backup { output } => serde_json::to_value(
            store
                .backup(
                    &output,
                    app_store::backup::Settings::default(),
                    serde_json::json!({"schema_version":1,"sources":{}}),
                )
                .await?,
        )?,
        Command::InspectBackup { .. } => unreachable!(),
        Command::RestoreBackup { input } => {
            let archive = app_store::backup::validate(&input).await?;
            let rollback = args.db.with_extension(format!(
                "before-restore-{}.visionbackup",
                uuid::Uuid::new_v4()
            ));
            store.restore(&archive, &rollback, &[]).await?
        }
        Command::Profiles => store.profiles().await?,
        Command::Show { profile_id } => store.state(&profile_id).await?,
        Command::History {
            profile_id,
            after,
            limit,
        } => store.history(&profile_id, after, limit).await?,
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    store.close().await?;
    Ok(())
}
