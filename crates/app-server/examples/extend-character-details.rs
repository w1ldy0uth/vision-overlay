use anyhow::{Result, ensure};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 3,
        "Usage: extend-character-details <base> <source-dir> <output>"
    );
    let base = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let dir = PathBuf::from(&args[1]);
    let mut sources = BTreeMap::new();
    for name in [
        "AvatarSkill",
        "AvatarSkillDepot",
        "AvatarTalent",
        "ProudSkill",
    ] {
        let file = format!("{name}ExcelConfigData.json");
        sources.insert(
            format!("ExcelBinOutput/{file}"),
            std::fs::read(dir.join(file))?,
        );
    }
    for prefix in ["TextMapRU", "TextMap_MediumRU"] {
        for part in 0..2 {
            let file = format!("{prefix}_{part}.json");
            sources.insert(format!("TextMap/{file}"), std::fs::read(dir.join(file))?);
        }
    }
    let bundle = app_server::catalog::build::extend_character_details(base, sources)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    file.write_all(&serde_json::to_vec(&bundle)?)?;
    println!("Validated character descriptions");
    Ok(())
}
