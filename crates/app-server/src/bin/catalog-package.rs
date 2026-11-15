//! Build-time only: validate a managed bundle before embedding its complete payload.
use anyhow::{Result, ensure};
use app_server::catalog::{Bundle, LIMIT, portable_payload};
use clap::Parser;
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    references_dir: Option<PathBuf>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        fs::metadata(&args.input)?.len() <= LIMIT as u64,
        "Catalog bundle too large"
    );
    let bundle: Bundle = serde_json::from_slice(&fs::read(args.input)?)?;
    let bytes = portable_payload(&bundle)?;
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(args.output, bytes)?;
    if let Some(dir) = args.references_dir {
        fs::create_dir_all(&dir)?;
        for (name, data) in [
            (
                "materials.ru.json",
                serde_json::json!({"revision":bundle.manifest.revision,"names":bundle.payload["names"]}),
            ),
            (
                "avatars.ru.json",
                serde_json::json!({"revision":bundle.manifest.revision,"names":bundle.payload["avatarNames"],"skill_names":bundle.payload["skillNames"]}),
            ),
            (
                "achievements.ru.json",
                serde_json::json!({"revision":bundle.manifest.revision,"records":bundle.payload["achievements"]}),
            ),
        ] {
            fs::write(dir.join(name), serde_json::to_vec(&data)?)?;
        }
    }
    println!(
        "Complete portable catalog validated: {}",
        bundle.manifest.revision
    );
    Ok(())
}
