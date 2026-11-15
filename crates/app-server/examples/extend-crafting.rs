use anyhow::{Context, Result};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "Usage: extend-crafting <base.visioncatalog> <CombineExcelConfigData.json> <output.visioncatalog>"
    );
    let base = serde_json::from_slice(&std::fs::read(&args[0])?).context("Invalid base package")?;
    let result = app_server::catalog::build::extend_crafting(base, std::fs::read(&args[1])?)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(&result)?)?;
    println!(
        "Validated crafting recipes: {}",
        result.payload["crafting"]["recipes"]
            .as_object()
            .unwrap()
            .len()
    );
    Ok(())
}
