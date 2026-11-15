use anyhow::Result;
use std::collections::BTreeMap;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 5,
        "Usage: extend-player-profile <base> <PlayerLevel> <ProfilePicture> <ProfileFrame> <output>"
    );
    let base = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let mut source = BTreeMap::new();
    for (name, path) in ["PlayerLevel", "ProfilePicture", "ProfileFrame"]
        .iter()
        .zip(&args[1..4])
    {
        source.insert((*name).into(), std::fs::read(path)?);
    }
    let result = app_server::catalog::build::extend_player_profile(base, source)?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])?;
    f.write_all(&serde_json::to_vec(&result)?)?;
    println!("Validated player profile metadata");
    Ok(())
}
