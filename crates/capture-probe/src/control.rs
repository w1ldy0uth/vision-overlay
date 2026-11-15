//! A per-run stop marker requests the same acquisition stop/drain as Ctrl+C.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub async fn wait_for_stop(path: Option<&Path>) {
    let Some(path) = path else {
        std::future::pending::<()>().await;
        return;
    };
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if path.is_file() {
            return;
        }
    }
}
pub fn ready(path: Option<&PathBuf>) -> anyhow::Result<()> {
    if let Some(path) = path {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(b"ready\n")?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_the_current_run_marker_stops_the_waiter() {
        let base = std::env::temp_dir().join(format!(
            "vision-control-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        let old = base.join("old");
        let current = base.join("new");
        std::fs::write(&old, "").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), wait_for_stop(Some(&current)))
                .await
                .is_err()
        );
        std::fs::write(&current, "").unwrap();
        tokio::time::timeout(Duration::from_secs(1), wait_for_stop(Some(&current)))
            .await
            .unwrap();
        std::fs::remove_file(old).unwrap();
        std::fs::remove_file(current).unwrap();
        std::fs::remove_dir(base).unwrap();
    }
    #[test]
    fn readiness_is_explicit_and_does_not_overwrite_a_stale_marker() {
        let path = std::env::temp_dir().join(format!(
            "vision-ready-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ready(Some(&path)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ready\n");
        assert!(ready(Some(&path)).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
