//! Optional raw diagnostics: bounded single file or a ring of complete VORAW segments.
use crate::source::{Packet, RecordingReader, RecordingWriter};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

pub struct Config {
    pub path: PathBuf,
    pub budget: u64,
    pub segment: u64,
    pub ring: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Segment {
    file: String,
    first_ms: Option<u64>,
    last_ms: Option<u64>,
    packets: u64,
    bytes: u64,
    closed: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    schema_version: u32,
    format: String,
    budget_bytes: u64,
    segment_limit_bytes: u64,
    closed: bool,
    evicted_segments: u64,
    evicted_packets: u64,
    evicted_through_ms: Option<u64>,
    standalone_replay: bool,
    replay_note: String,
    segments: VecDeque<Segment>,
}
pub struct Ring {
    root: PathBuf,
    manifest: Manifest,
    writer: Option<RecordingWriter<BufWriter<File>>>,
    next: u64,
}
impl Ring {
    pub fn new(path: &Path, budget: u64, segment: u64) -> Result<Self> {
        ensure!(
            segment >= 21 && segment <= budget,
            "Ring segment must fit the budget and a recording header/footer"
        );
        // Never adopt or clean an existing directory. Only files created by this instance can be evicted.
        fs::create_dir(path).context("Cannot create raw ring directory (must not exist)")?;
        let mut ring = Self {
            root: fs::canonicalize(path)?,
            writer: None,
            next: 0,
            manifest: Manifest {
                schema_version: 1,
                format: "VORAW001-ring".into(),
                budget_bytes: budget,
                segment_limit_bytes: segment,
                closed: false,
                evicted_segments: 0,
                evicted_packets: 0,
                evicted_through_ms: None,
                standalone_replay: false,
                replay_note: "active_or_unclean_capture".into(),
                segments: VecDeque::new(),
            },
        };
        ring.open_segment()?;
        Ok(ring)
    }
    fn save(&self) -> Result<()> {
        let temp = self.root.join("manifest.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        serde_json::to_writer_pretty(&mut file, &self.manifest)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(temp, self.root.join("manifest.json"))?;
        Ok(())
    }
    fn open_segment(&mut self) -> Result<()> {
        let mut retained: u64 = self.manifest.segments.iter().map(|s| s.bytes).sum();
        // Reserve the entire next segment, so the on-disk raw sum never exceeds the budget.
        while retained + self.manifest.segment_limit_bytes > self.manifest.budget_bytes {
            let oldest = self
                .manifest
                .segments
                .front()
                .context("No completed segment to evict")?;
            ensure!(oldest.closed, "Cannot evict active segment");
            let path = self.root.join(&oldest.file);
            ensure!(
                path.parent() == Some(self.root.as_path()),
                "Eviction escaped ring directory"
            );
            fs::remove_file(path)?;
            let removed = self.manifest.segments.pop_front().unwrap();
            retained -= removed.bytes;
            self.manifest.evicted_segments += 1;
            self.manifest.evicted_packets += removed.packets;
            self.manifest.evicted_through_ms = removed.last_ms.or(self.manifest.evicted_through_ms);
        }
        let name = format!("segment-{:08}.voraw", self.next);
        self.next += 1;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join(&name))?;
        self.writer = Some(RecordingWriter::new(
            BufWriter::new(file),
            self.manifest.segment_limit_bytes,
        )?);
        self.manifest.segments.push_back(Segment {
            file: name,
            first_ms: None,
            last_ms: None,
            packets: 0,
            bytes: 8,
            closed: false,
        });
        self.save()
    }
    fn close_segment(&mut self, time: u64) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            let bytes = writer.finish(time)?;
            let segment = self
                .manifest
                .segments
                .back_mut()
                .context("Missing active segment")?;
            segment.bytes = bytes;
            segment.closed = true;
        }
        Ok(())
    }
    pub fn packet(&mut self, time: u64, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() as u64 + 32 <= self.manifest.segment_limit_bytes,
            "Packet cannot fit raw segment"
        );
        if !self
            .writer
            .as_mut()
            .context("Ring closed")?
            .packet(time, bytes)?
        {
            self.close_segment(time)?;
            self.open_segment()?;
            ensure!(
                self.writer.as_mut().unwrap().packet(time, bytes)?,
                "Packet cannot fit empty segment"
            );
        }
        let segment = self.manifest.segments.back_mut().unwrap();
        segment.first_ms.get_or_insert(time);
        segment.last_ms = Some(time);
        segment.packets += 1;
        segment.bytes = self.writer.as_ref().unwrap().bytes;
        Ok(())
    }
    pub fn finish(&mut self, time: u64) -> Result<()> {
        self.close_segment(time)?;
        self.manifest.closed = true;
        self.manifest.standalone_replay = self.manifest.evicted_segments == 0;
        self.manifest.replay_note = if self.manifest.standalone_replay {
            "full_recording_from_capture_start_decoder_still_requires_handshake"
        } else {
            "evicted_prefix_key_and_kcp_context_may_be_missing"
        }
        .into();
        self.save()
    }
    fn stats(&self) -> Value {
        json!({"retained_bytes":self.manifest.segments.iter().map(|s|s.bytes).sum::<u64>(),
            "segments":self.manifest.segments.len(),"rotations":self.next.saturating_sub(1),
            "evicted_segments":self.manifest.evicted_segments,"evicted_packets":self.manifest.evicted_packets,
            "evicted_through_ms":self.manifest.evicted_through_ms,"standalone_replay":self.manifest.standalone_replay})
    }
}

enum Sink {
    Single(RecordingWriter<BufWriter<File>>),
    Ring(Ring),
}
pub struct Recorder {
    sink: Option<Sink>,
    state: &'static str,
    mode: &'static str,
    packets: u64,
    written: u64,
    stats: Value,
    error: Option<String>,
}
impl Recorder {
    pub fn new(config: Option<Config>) -> Self {
        let mut recorder = Self {
            sink: None,
            state: "disabled",
            mode: "none",
            packets: 0,
            written: 0,
            stats: json!({}),
            error: None,
        };
        if let Some(config) = config {
            recorder.mode = if config.ring { "ring" } else { "single" };
            let result = (|| -> Result<Sink> {
                if config.ring {
                    Ok(Sink::Ring(Ring::new(
                        &config.path,
                        config.budget,
                        config.segment,
                    )?))
                } else {
                    Ok(Sink::Single(RecordingWriter::new(
                        BufWriter::new(
                            OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&config.path)?,
                        ),
                        config.budget,
                    )?))
                }
            })();
            match result {
                Ok(sink) => {
                    recorder.sink = Some(sink);
                    recorder.state = "recording";
                }
                Err(error) => recorder.fail(error),
            }
        }
        recorder
    }
    fn fail(&mut self, error: anyhow::Error) {
        if let Some(Sink::Ring(ring)) = &self.sink {
            self.stats = ring.stats();
        }
        self.sink = None;
        self.state = "failed";
        self.error = Some(format!("{error:#}"));
        eprintln!(
            "Raw recording FAILED: {error:#}. Capture and inventory tracking continue; raw diagnostics are incomplete."
        );
    }
    pub fn packet(&mut self, time: u64, bytes: &[u8]) {
        let result = (|| -> Result<bool> {
            match &mut self.sink {
                Some(Sink::Ring(ring)) => {
                    ring.packet(time, bytes)?;
                    Ok(true)
                }
                Some(Sink::Single(writer)) => writer.packet(time, bytes),
                None => Ok(false),
            }
        })();
        match result {
            Ok(true) => {
                self.packets += 1;
                self.written += bytes.len() as u64 + 12;
            }
            Ok(false) if self.sink.is_some() => {
                self.finish(time);
                if self.state != "failed" {
                    self.state = "limit_reached";
                }
                eprintln!(
                    "Raw single-file limit reached. Recording stopped; capture and tracking continue."
                );
            }
            Ok(false) => {}
            Err(error) => self.fail(error),
        }
    }
    pub fn finish(&mut self, time: u64) {
        let result = match self.sink.take() {
            Some(Sink::Ring(mut ring)) => {
                let result = ring.finish(time);
                self.stats = ring.stats();
                result
            }
            Some(Sink::Single(writer)) => writer.finish(time).map(|_| ()),
            None => return,
        };
        if let Err(error) = result {
            self.fail(error);
        } else {
            self.state = "closed";
        }
    }
    pub fn report(&self) -> Value {
        let ring = if let Some(Sink::Ring(ring)) = &self.sink {
            ring.stats()
        } else {
            self.stats.clone()
        };
        json!({"mode":self.mode,"state":self.state,"recorded_packets":self.packets,"packet_record_bytes_written":self.written,
            "ring":ring,"error":self.error})
    }
}

pub enum Reader {
    Single(RecordingReader<BufReader<File>>),
    Ring(RingReader),
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        if path.is_dir() {
            Ok(Self::Ring(RingReader::open(path)?))
        } else {
            Ok(Self::Single(RecordingReader::new(BufReader::new(
                File::open(path).context("Cannot open recording")?,
            ))?))
        }
    }
    pub fn next(&mut self) -> Result<Option<Packet>> {
        match self {
            Self::Single(reader) => reader.next(),
            Self::Ring(reader) => reader.next(),
        }
    }
    pub fn last_time(&self) -> u64 {
        match self {
            Self::Single(reader) => reader.last_time,
            Self::Ring(reader) => reader.last_time,
        }
    }
}

/// Full, closed, never-evicted rings may be replayed sequentially. A retained tail
/// is transport evidence, not a new independent KCP session.
pub struct RingReader {
    paths: VecDeque<PathBuf>,
    reader: Option<RecordingReader<BufReader<File>>>,
    pub last_time: u64,
}
impl RingReader {
    pub fn open(root: &Path) -> Result<Self> {
        let manifest: Manifest = serde_json::from_reader(File::open(root.join("manifest.json"))?)?;
        ensure!(
            manifest.schema_version == 1 && manifest.format == "VORAW001-ring",
            "Unsupported raw ring manifest"
        );
        ensure!(
            manifest.closed && manifest.standalone_replay && manifest.evicted_segments == 0,
            "Raw ring is active, unclean, or has an evicted prefix: standalone decoder replay is unavailable (key/KCP context may be missing)"
        );
        let mut paths = VecDeque::new();
        for (index, segment) in manifest.segments.into_iter().enumerate() {
            ensure!(
                segment.closed
                    && segment.file == format!("segment-{index:08}.voraw")
                    && Path::new(&segment.file).components().count() == 1
                    && segment.file.starts_with("segment-")
                    && segment.file.ends_with(".voraw"),
                "Invalid raw segment path or state"
            );
            let path = root.join(segment.file);
            ensure!(
                !fs::symlink_metadata(&path)?.file_type().is_symlink(),
                "Raw segment must not be a symlink"
            );
            ensure!(
                fs::metadata(&path)?.len() == segment.bytes,
                "Raw segment size differs from manifest"
            );
            paths.push_back(path);
        }
        ensure!(!paths.is_empty(), "Empty raw ring");
        Ok(Self {
            paths,
            reader: None,
            last_time: 0,
        })
    }
    pub fn next(&mut self) -> Result<Option<Packet>> {
        loop {
            if self.reader.is_none() {
                let Some(path) = self.paths.pop_front() else {
                    return Ok(None);
                };
                self.reader = Some(RecordingReader::new(BufReader::new(File::open(path)?))?);
            }
            let reader = self.reader.as_mut().unwrap();
            if let Some(packet) = reader.next()? {
                ensure!(
                    packet.elapsed_ms >= self.last_time,
                    "Raw ring timestamps regress"
                );
                self.last_time = packet.elapsed_ms;
                return Ok(Some(packet));
            }
            ensure!(
                reader.last_time >= self.last_time,
                "Raw segment end time regresses"
            );
            self.last_time = reader.last_time;
            self.reader = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "vision-ring-test-{}-{}-{}",
                std::process::id(),
                FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Non-recursive cleanup of this test's own files only.
            for entry in fs::read_dir(&self.0).unwrap().flatten() {
                if entry.file_type().unwrap().is_dir() {
                    for file in fs::read_dir(entry.path()).unwrap().flatten() {
                        let _ = fs::remove_file(file.path());
                    }
                    let _ = fs::remove_dir(entry.path());
                } else {
                    let _ = fs::remove_file(entry.path());
                }
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
    #[test]
    fn multiple_rotations_keep_bounded_raw_window_and_reject_incomplete_replay() {
        let fixture = Fixture::new();
        let path = fixture.0.join("ring");
        fs::write(fixture.0.join("events.jsonl"), b"keep events").unwrap();
        let mut ring = Ring::new(&path, 200, 80).unwrap();
        for n in 1..=40 {
            ring.packet(n, &[n as u8; 8]).unwrap();
            let raw_bytes: u64 = fs::read_dir(&path)
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|s| s == "voraw"))
                .map(|e| e.metadata().unwrap().len())
                .sum();
            assert!(raw_bytes <= 200);
        }
        ring.finish(50).unwrap();
        assert!(ring.manifest.evicted_segments > 2);
        assert!(ring.manifest.evicted_packets > 0);
        assert_eq!(ring.manifest.evicted_through_ms, Some(36));
        assert!(!ring.manifest.standalone_replay);
        assert!(RingReader::open(&path).is_err());
        assert_eq!(
            fs::read(fixture.0.join("events.jsonl")).unwrap(),
            b"keep events"
        );
        let mut retained = Vec::new();
        for segment in &ring.manifest.segments {
            assert!(segment.closed);
            let mut reader = RecordingReader::new(BufReader::new(
                File::open(path.join(&segment.file)).unwrap(),
            ))
            .unwrap();
            while let Some(packet) = reader.next().unwrap() {
                retained.push(packet.elapsed_ms);
            }
        }
        assert_eq!(retained, vec![37, 38, 39, 40]);
        let manifest: Manifest =
            serde_json::from_reader(File::open(path.join("manifest.json")).unwrap()).unwrap();
        assert!(manifest.closed);
        assert_eq!(manifest.evicted_packets, 36);
        assert!(manifest.segments.iter().map(|s| s.bytes).sum::<u64>() <= 200);
    }
    #[test]
    fn complete_ring_round_trips_all_segments_and_preserves_existing_directory() {
        let fixture = Fixture::new();
        let path = fixture.0.join("ring");
        let mut ring = Ring::new(&path, 1000, 80).unwrap();
        assert!(RingReader::open(&path).is_err());
        for time in 1..=8 {
            ring.packet(time, &[time as u8; 8]).unwrap();
        }
        ring.finish(10).unwrap();
        assert!(Ring::new(&path, 1000, 80).is_err());
        let mut reader = Reader::open(&path).unwrap();
        for time in 1..=8 {
            let packet = reader.next().unwrap().unwrap();
            assert_eq!(packet.elapsed_ms, time);
            assert_eq!(packet.bytes, vec![time as u8; 8]);
        }
        assert!(reader.next().unwrap().is_none());
        assert_eq!(reader.last_time(), 10);
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
        manifest["segments"][0]["file"] = json!("../outside.voraw");
        fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(RingReader::open(&path).is_err());
    }
    #[test]
    fn recording_errors_disable_only_raw_and_single_limit_finishes_cleanly() {
        let fixture = Fixture::new();
        let path = fixture.0.join("ring");
        let mut recorder = Recorder::new(Some(Config {
            path: path.clone(),
            budget: 160,
            segment: 80,
            ring: true,
        }));
        for time in 1..=3 {
            recorder.packet(time, &[1; 8]);
        }
        fs::write(path.join("manifest.tmp"), b"do not overwrite").unwrap();
        for time in 4..=10 {
            recorder.packet(time, &[1; 8]);
        }
        assert_eq!(recorder.report()["state"], "failed");
        assert_eq!(recorder.report()["recorded_packets"], 3);
        assert_eq!(
            fs::read(path.join("manifest.tmp")).unwrap(),
            b"do not overwrite"
        );
        recorder.finish(11);
        assert_eq!(recorder.report()["state"], "failed");
        let single = fixture.0.join("single.voraw");
        let mut recorder = Recorder::new(Some(Config {
            path: single.clone(),
            budget: 40,
            segment: 0,
            ring: false,
        }));
        for time in 1..=10 {
            recorder.packet(time, &[1; 8]);
        }
        assert_eq!(recorder.report()["state"], "limit_reached");
        let mut reader = Reader::open(&single).unwrap();
        assert!(reader.next().unwrap().is_some());
        assert!(reader.next().unwrap().is_none());
    }
}
