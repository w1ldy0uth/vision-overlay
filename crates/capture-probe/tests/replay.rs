use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
static FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vision-overlay-test-{}-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keys.json"), r#"{"1":"AQ=="}"#).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_capture-probe"));
        cmd.arg("--keys").arg(self.0.join("keys.json"));
        cmd
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let ring = self.0.join("ring");
        if ring.is_dir() {
            for file in fs::read_dir(&ring).unwrap().flatten() {
                let _ = fs::remove_file(file.path());
            }
            let _ = fs::remove_dir(ring);
        }
        for name in ["keys.json", "test.voraw", "result.jsonl", "test.pcapng"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn pcapng_fixture(truncated: bool, endianness: pcap_file::Endianness) -> Vec<u8> {
    use pcap_file::{
        DataLink,
        pcapng::{
            Block, PcapNgWriter,
            blocks::{
                enhanced_packet::EnhancedPacketBlock,
                interface_description::{InterfaceDescriptionBlock, InterfaceDescriptionOption},
            },
        },
    };
    let mut result = Vec::new();
    let mut writer = PcapNgWriter::with_endianness(&mut result, endianness).unwrap();
    writer
        .write_block(&Block::InterfaceDescription(InterfaceDescriptionBlock {
            linktype: DataLink::ETHERNET,
            snaplen: 65535,
            options: vec![InterfaceDescriptionOption::IfTsResol(6)],
        }))
        .unwrap();
    let raw = recording(true);
    let mut cursor = 8;
    while cursor + 12 <= raw.len() {
        let mut time = u64::from_le_bytes(raw[cursor..cursor + 8].try_into().unwrap());
        let length = u32::from_le_bytes(raw[cursor + 8..cursor + 12].try_into().unwrap()) as usize;
        cursor += 12;
        if length == 0 {
            break;
        }
        if time == 20 {
            time = 2520;
        }
        writer
            .write_block(&Block::EnhancedPacket(EnhancedPacketBlock {
                interface_id: 0,
                timestamp: std::time::Duration::from_nanos(1_700_000_000_000_000 + time * 1000),
                original_len: length as u32 + u32::from(truncated),
                data: std::borrow::Cow::Borrowed(&raw[cursor..cursor + length]),
                options: vec![],
            }))
            .unwrap();
        cursor += length;
    }
    result
}

#[test]
fn pcapng_export_uses_interface_timestamp_resolution_and_common_decoder() {
    for endian in [pcap_file::Endianness::Little, pcap_file::Endianness::Big] {
        let f = Fixture::new();
        fs::write(f.0.join("test.pcapng"), pcapng_fixture(false, endian)).unwrap();
        let result = f
            .command()
            .arg("--pcapng")
            .arg(f.0.join("test.pcapng"))
            .args(["--queue-capacity", "1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let events: Vec<serde_json::Value> = String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let summary = &events
            .iter()
            .find(|e| e["event"] == "transport_summary")
            .unwrap()["data"];
        assert_eq!(summary["elapsed_ms"], 2510);
        assert_eq!(summary["directions"]["incoming"]["udp_observations"], 1);
        assert_eq!(summary["duplicate_handshakes_filtered"], 1);
        assert_eq!(events[0]["data"]["mode"], "pcapng");
    }
}

#[test]
fn rejects_truncated_pcapng_packets_and_damaged_blocks() {
    for packet_truncated in [false, true] {
        let f = Fixture::new();
        let mut bytes = pcapng_fixture(packet_truncated, pcap_file::Endianness::Little);
        if !packet_truncated {
            bytes.pop();
        }
        fs::write(f.0.join("test.pcapng"), bytes).unwrap();
        let result = f
            .command()
            .arg("--pcapng")
            .arg(f.0.join("test.pcapng"))
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("source_error"));
    }
}

#[test]
fn ignores_statistics_options_but_still_checks_block_framing() {
    let f = Fixture::new();
    let mut bytes = pcapng_fixture(false, pcap_file::Endianness::Little);
    // Statistics with one option ending exactly at the block end, as in pktmon export.
    let mut body = vec![0; 12];
    body.extend(4u16.to_le_bytes());
    body.extend(8u16.to_le_bytes());
    body.extend(0u64.to_le_bytes());
    let length = (body.len() + 12) as u32;
    bytes.extend(5u32.to_le_bytes());
    bytes.extend(length.to_le_bytes());
    bytes.extend(body);
    bytes.extend(length.to_le_bytes());
    fs::write(f.0.join("test.pcapng"), &bytes).unwrap();
    let result = f
        .command()
        .arg("--pcapng")
        .arg(f.0.join("test.pcapng"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .find(|e| e["event"] == "source_summary")
            .unwrap()["data"]["pcapng_metadata_blocks_skipped"],
        1
    );
    bytes.pop();
    fs::write(f.0.join("test.pcapng"), bytes).unwrap();
    assert!(
        !f.command()
            .arg("--pcapng")
            .arg(f.0.join("test.pcapng"))
            .output()
            .unwrap()
            .status
            .success()
    );
}
fn recording(footer: bool) -> Vec<u8> {
    let mut raw = b"VORAW001".to_vec();
    for (time, source, destination, code) in [
        (10u64, 50000u16, 22101u16, 255u32),
        (11, 50000, 22101, 255),
        (20, 22101, 50000, 325),
    ] {
        let builder = etherparse::PacketBuilder::ethernet2([1; 6], [2; 6])
            .ipv4([10, 0, 0, 1], [10, 0, 0, 2], 64)
            .udp(source, destination);
        let mut bytes = Vec::new();
        let mut payload = vec![0; 20];
        payload[..4].copy_from_slice(&code.to_be_bytes());
        builder.write(&mut bytes, &payload).unwrap();
        raw.extend(time.to_le_bytes());
        raw.extend((bytes.len() as u32).to_le_bytes());
        raw.extend(bytes);
    }
    if footer {
        raw.extend(30u64.to_le_bytes());
        raw.extend(0u32.to_le_bytes());
    }
    raw
}
#[test]
fn cli_replays_complete_ring_and_rejects_evicted_prefix() {
    let f = Fixture::new();
    let root = f.0.join("ring");
    fs::create_dir(&root).unwrap();
    let bytes = recording(true);
    fs::write(root.join("segment-00000000.voraw"), &bytes).unwrap();
    let mut manifest = serde_json::json!({"schema_version":1,"format":"VORAW001-ring","budget_bytes":4096,
        "segment_limit_bytes":4096,"closed":true,"evicted_segments":0,"evicted_packets":0,"evicted_through_ms":null,
        "standalone_replay":true,"replay_note":"complete","segments":[{"file":"segment-00000000.voraw",
        "first_ms":10,"last_ms":20,"packets":3,"bytes":bytes.len(),"closed":true}]});
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let result = f.command().arg("--replay").arg(&root).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .find(|e| e["event"] == "source_summary")
            .unwrap()["data"]["acquired_packets"],
        3
    );
    manifest["evicted_segments"] = serde_json::json!(1);
    manifest["standalone_replay"] = serde_json::json!(false);
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let result = f.command().arg("--replay").arg(&root).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("evicted prefix"));
}
#[test]
fn offline_cli_uses_same_pipeline_preserves_time_and_drains_bounded_queue() {
    let f = Fixture::new();
    fs::write(f.0.join("test.voraw"), recording(true)).unwrap();
    let result = f
        .command()
        .arg("--replay")
        .arg(f.0.join("test.voraw"))
        .args(["--queue-capacity", "1", "--output"])
        .arg(f.0.join("result.jsonl"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let events: Vec<serde_json::Value> = fs::read_to_string(f.0.join("result.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let source = &events
        .iter()
        .find(|e| e["event"] == "source_summary")
        .unwrap()["data"];
    assert_eq!(source["acquired_packets"], 3);
    assert_eq!(source["enqueued_packets"], 3);
    assert_eq!(source["dropped_queue_full"], 0);
    let transport = &events
        .iter()
        .find(|e| e["event"] == "transport_summary")
        .unwrap()["data"];
    assert_eq!(transport["elapsed_ms"], 30);
    assert_eq!(transport["duplicate_handshakes_filtered"], 1);
    assert_eq!(transport["directions"]["incoming"]["udp_observations"], 1);
    assert_eq!(transport["directions"]["outgoing"]["udp_observations"], 2);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["data"]["state"] == "waiting_for_snapshot")
            .count(),
        1
    );
    assert!(!String::from_utf8_lossy(&result.stderr).contains("Capture active"));
}
#[test]
fn truncated_recording_fails_and_existing_recordings_are_not_overwritten() {
    let f = Fixture::new();
    let bytes = recording(false);
    fs::write(f.0.join("test.voraw"), &bytes).unwrap();
    let result = f
        .command()
        .arg("--replay")
        .arg(f.0.join("test.voraw"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Truncated recording"));
    let result = f
        .command()
        .arg("--record")
        .arg(f.0.join("test.voraw"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("existing files are never overwritten")
    );
    assert_eq!(fs::read(f.0.join("test.voraw")).unwrap(), bytes);
}
