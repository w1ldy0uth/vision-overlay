//! Prepare a pinned catalog package without touching the running app or account data.
use anyhow::{Context, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let output = args
        .next()
        .context("Usage: build-catalog <output.visioncatalog> [revision]")?;
    let bundle = app_server::catalog::build::download(args.next()).await?;
    bundle.validate()?;
    std::fs::write(&output, serde_json::to_vec(&bundle)?)?;
    println!("Validated catalog written to {output}");
    Ok(())
}
