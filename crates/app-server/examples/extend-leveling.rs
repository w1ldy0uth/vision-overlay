//! Source directory must come from the exact revision of the base package.
use anyhow::{Context, Result};
use app_server::catalog::{Bundle, build};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let base=args.next().context("Usage: extend-leveling <base.visioncatalog> <same-revision-source-dir> <output.visioncatalog>")?;
    let root = PathBuf::from(args.next().context("Missing source directory")?);
    let output = args.next().context("Missing output path")?;
    let bundle: Bundle = serde_json::from_slice(&std::fs::read(base)?)?;
    let mut files = BTreeMap::new();
    for name in [
        "AvatarLevel",
        "WeaponLevel",
        "Material",
        "ConstValue",
        "Avatar",
        "AvatarSkill",
    ] {
        files.insert(
            name.into(),
            std::fs::read(root.join(format!("{name}.json")))?,
        );
    }
    let next = build::extend_leveling(bundle, files)?;
    std::fs::write(output, serde_json::to_vec(&next)?)?;
    println!("Verified catalog extended with leveling tables");
    Ok(())
}
