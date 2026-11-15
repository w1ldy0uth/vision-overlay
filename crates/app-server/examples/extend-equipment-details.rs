use anyhow::{Result, ensure};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4,
        "Usage: extend-equipment-details <base> <tables-dir> <text-dir> <output>"
    );
    let base = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let dir = PathBuf::from(&args[1]);
    let text = PathBuf::from(&args[2]);
    let mut sources = BTreeMap::new();
    for name in [
        "Weapon",
        "WeaponCurve",
        "WeaponPromote",
        "EquipAffix",
        "Reliquary",
        "ReliquarySet",
    ] {
        sources.insert(
            format!("ExcelBinOutput/{name}ExcelConfigData.json"),
            std::fs::read(dir.join(format!("{name}.json")))?,
        );
    }
    for prefix in ["TextMapRU", "TextMap_MediumRU"] {
        for part in 0..2 {
            let file = format!("{prefix}_{part}.json");
            sources.insert(format!("TextMap/{file}"), std::fs::read(text.join(file))?);
        }
    }
    let bundle = app_server::catalog::build::extend_equipment_details(base, sources)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?;
    file.write_all(&serde_json::to_vec(&bundle)?)?;
    println!("Validated equipment details");
    Ok(())
}
