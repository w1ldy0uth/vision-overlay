use serde_json::json;
use std::{fs, process::Command};

#[test]
fn live_prefix_export_omits_partial_tail_without_changing_source_and_rejects_overwrite() {
    let dir = std::env::temp_dir().join(format!("vision-prefix-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let manifest = json!({"schema_version":1,"evicted_segments":0,"closed":false,"segments":[{"file":"segment-00000000.voraw","closed":false}]});
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    let mut raw = b"VORAW001".to_vec();
    raw.extend(12u64.to_le_bytes());
    raw.extend(3u32.to_le_bytes());
    raw.extend([1, 2, 3]);
    let complete = raw.clone();
    raw.extend(15u64.to_le_bytes());
    raw.extend(8u32.to_le_bytes());
    raw.extend([1, 2]);
    let source = dir.join("segment-00000000.voraw");
    fs::write(&source, &raw).unwrap();
    let output = dir.join("cut.voraw");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_raw-ring-export"))
            .arg("--ring")
            .arg(&dir)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    assert_eq!(fs::read(&source).unwrap(), raw);
    let mut expected = complete;
    expected.extend(12u64.to_le_bytes());
    expected.extend(0u32.to_le_bytes());
    assert_eq!(fs::read(&output).unwrap(), expected);
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("cut.export.json")).unwrap()).unwrap();
    assert_eq!(report["sources"][0]["incomplete_tail_bytes"], 14);
    assert_eq!(report["original_capture_closed"], false);
    assert!(!run().status.success());
    fs::remove_file(&output).unwrap();
    fs::remove_file(dir.join("cut.export.json")).unwrap();
    let mut evicted = manifest;
    evicted["evicted_segments"] = json!(1);
    fs::write(dir.join("manifest.json"), evicted.to_string()).unwrap();
    assert!(!run().status.success());
    assert!(!output.exists());
    for name in ["manifest.json", "segment-00000000.voraw"] {
        fs::remove_file(dir.join(name)).unwrap();
    }
    fs::remove_dir(dir).unwrap();
}
