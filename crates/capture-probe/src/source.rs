//! Dedicated acquisition thread and bounded pre-decoder recordings.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const MAGIC: &[u8; 8] = b"VORAW001";
const MAX_PACKET: usize = 131072;
const FOOTER_BYTES: u64 = 12;

pub struct Packet {
    pub elapsed_ms: u64,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Metrics {
    #[cfg(windows)]
    backend: Arc<pktmon::RealtimeStats>,
    acquired: AtomicU64,
    queued: AtomicU64,
    dropped: AtomicU64,
    high_water: AtomicU64,
    pub elapsed_ms: AtomicU64,
    recording: Mutex<Value>,
    timestamp_regressions: AtomicU64,
    metadata_blocks_skipped: AtomicU64,
}
impl Metrics {
    pub fn report(&self, queue_depth: usize, capacity: usize) -> Value {
        let recording = self.recording.lock().unwrap().clone();
        let mut report = json!({"acquired_packets": self.acquired.load(Ordering::Relaxed),
            "enqueued_packets": self.queued.load(Ordering::Relaxed), "dropped_queue_full": self.dropped.load(Ordering::Relaxed),
            "queue_high_water": self.high_water.load(Ordering::Relaxed), "queue_depth": queue_depth, "queue_capacity": capacity,
            "recorded_packets": recording["recorded_packets"].as_u64().unwrap_or(0),
            "packet_record_bytes_written":recording["packet_record_bytes_written"].as_u64().unwrap_or(0),
            "recording": recording,
            "pcapng_timestamp_regressions": self.timestamp_regressions.load(Ordering::Relaxed),
            "pcapng_metadata_blocks_skipped": self.metadata_blocks_skipped.load(Ordering::Relaxed),
            "interpretation": "counts_start_at_pktmon_delivery_not_network_or_driver"});
        #[cfg(windows)]
        {
            report["realtime_backend"] = json!({"missed_write":self.backend.missed_write.load(Ordering::Relaxed),
                "missed_read":self.backend.missed_read.load(Ordering::Relaxed),"callback_queue_drops":self.backend.callback_queue_drops.load(Ordering::Relaxed),
                "invalid_descriptors":self.backend.invalid_descriptors.load(Ordering::Relaxed),"selected_nic_sources":self.backend.selected_sources.load(Ordering::Relaxed)});
        }
        report
    }
}

pub struct Source {
    pub receiver: mpsc::Receiver<Packet>,
    pub metrics: Arc<Metrics>,
    pub capacity: usize,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<()>>>,
}
impl Source {
    pub fn has_capture_loss(&self) -> bool {
        let dropped = self.metrics.dropped.load(Ordering::Relaxed) > 0;
        #[cfg(windows)]
        {
            let stats = &self.metrics.backend;
            dropped
                || stats.missed_write.load(Ordering::Relaxed) > 0
                || stats.missed_read.load(Ordering::Relaxed) > 0
                || stats.callback_queue_drops.load(Ordering::Relaxed) > 0
                || stats.invalid_descriptors.load(Ordering::Relaxed) > 0
        }
        #[cfg(not(windows))]
        {
            dropped
        }
    }
    pub fn pcapng(path: &Path, capacity: usize) -> Result<Self> {
        let mut reader = crate::pcapng::Reader::open(path)?;
        let (sender, receiver) = mpsc::channel(capacity);
        let metrics = Arc::new(Metrics::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (stats, cancel) = (metrics.clone(), stop.clone());
        let worker = std::thread::Builder::new()
            .name("pcapng-import".into())
            .spawn(move || {
                while !cancel.load(Ordering::Relaxed) {
                    let Some(packet) = reader.next()? else {
                        stats
                            .metadata_blocks_skipped
                            .store(reader.metadata_blocks_skipped, Ordering::Relaxed);
                        return Ok(());
                    };
                    stats.elapsed_ms.store(packet.elapsed_ms, Ordering::Relaxed);
                    stats
                        .timestamp_regressions
                        .store(reader.timestamp_regressions, Ordering::Relaxed);
                    stats.acquired.fetch_add(1, Ordering::Relaxed);
                    if sender.blocking_send(packet).is_err() {
                        return Ok(());
                    }
                    stats.queued.fetch_add(1, Ordering::Relaxed);
                    stats
                        .high_water
                        .fetch_max((capacity - sender.capacity()) as u64, Ordering::Relaxed);
                }
                Ok(())
            })?;
        Ok(Self {
            receiver,
            metrics,
            capacity,
            stop,
            worker: Some(worker),
        })
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn report(&self) -> Value {
        self.metrics.report(self.receiver.len(), self.capacity)
    }
    pub fn finish(&mut self) -> Result<()> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("Acquisition worker panicked"))??;
        }
        Ok(())
    }
    pub fn replay(path: &Path, capacity: usize) -> Result<Self> {
        let mut reader = crate::recording::Reader::open(path)?;
        let (sender, receiver) = mpsc::channel(capacity);
        let metrics = Arc::new(Metrics::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (stats, cancel) = (metrics.clone(), stop.clone());
        let worker = std::thread::Builder::new()
            .name("packet-replay".into())
            .spawn(move || {
                while !cancel.load(Ordering::Relaxed) {
                    let Some(packet) = reader.next()? else {
                        stats
                            .elapsed_ms
                            .store(reader.last_time(), Ordering::Relaxed);
                        return Ok(());
                    };
                    stats.elapsed_ms.store(packet.elapsed_ms, Ordering::Relaxed);
                    stats.acquired.fetch_add(1, Ordering::Relaxed);
                    if sender.blocking_send(packet).is_err() {
                        return Ok(());
                    }
                    stats.queued.fetch_add(1, Ordering::Relaxed);
                    stats
                        .high_water
                        .fetch_max((capacity - sender.capacity()) as u64, Ordering::Relaxed);
                }
                Ok(())
            })?;
        Ok(Self {
            receiver,
            metrics,
            capacity,
            stop,
            worker: Some(worker),
        })
    }

    #[cfg(windows)]
    pub fn live(
        started: Instant,
        recording: Option<crate::recording::Config>,
        capacity: usize,
        component: Option<u32>,
        ready_file: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        if recording
            .as_ref()
            .is_some_and(|config| config.path.exists())
        {
            bail!(
                "Cannot create recording (existing files are never overwritten; ring directory must be new)"
            );
        }
        let mut recording = crate::recording::Recorder::new(recording);
        let (sender, receiver) = mpsc::channel(capacity);
        let metrics = Arc::new(Metrics::default());
        *metrics.recording.lock().unwrap() = recording.report();
        let stop = Arc::new(AtomicBool::new(false));
        let (stats, cancel) = (metrics.clone(), stop.clone());
        let worker = std::thread::Builder::new().name("pktmon-acquisition".into()).spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async move {
                use futures::StreamExt;
                use pktmon::{Capture, filter::{PktMonFilter, TransportProtocol}};
                let mut capture = Capture::new_realtime_nics(stats.backend.clone(),component).context("Cannot start realtime NIC capture; Windows 11 realtime API and administrator terminal required")?;
                for port in [22101u16,22102] {
                    capture.add_filter(PktMonFilter { name:format!("VisionOverlay-{port}"),transport_protocol:Some(TransportProtocol::UDP),port:port.into(),..Default::default() })?;
                }
                let mut stream = capture.stream().context("Cannot open pktmon stream")?.boxed();
                crate::control::ready(ready_file.as_ref()).context("Cannot publish capture readiness")?;
                eprintln!("Realtime NIC capture active ({} sources). Start the game now. Ctrl+C stops acquisition and drains the queue.",stats.backend.selected_sources.load(Ordering::Relaxed));
                let mut poll = tokio::time::interval(Duration::from_millis(20));
                let mut failure = None;
                loop {
                    if cancel.load(Ordering::Relaxed) { break; }
                    tokio::select! {
                        _ = poll.tick() => { *stats.recording.lock().unwrap()=recording.report(); },
                        packet = stream.next() => {
                            let Some(packet) = packet else { failure=Some(anyhow::anyhow!("pktmon stream closed")); break; };
                            let bytes = packet.payload.to_vec().clone();
                            let now = started.elapsed().as_millis() as u64;
                            stats.elapsed_ms.store(now, Ordering::Relaxed);
                            stats.acquired.fetch_add(1, Ordering::Relaxed);
                            recording.packet(now,&bytes);
                            enqueue(&sender, Packet {elapsed_ms:now,bytes}, &stats);
                        }
                    }
                }
                let now=started.elapsed().as_millis() as u64;
                stats.elapsed_ms.store(now,Ordering::Relaxed);
                recording.finish(now);
                *stats.recording.lock().unwrap()=recording.report();
                if let Some(error)=failure { return Err(error); }
                Ok(())
            })
        })?;
        Ok(Self {
            receiver,
            metrics,
            capacity,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.stop();
        self.receiver.close();
        let _ = self.finish();
    }
}

fn enqueue(sender: &mpsc::Sender<Packet>, packet: Packet, metrics: &Metrics) {
    match sender.try_send(packet) {
        Ok(()) => {
            metrics.queued.fetch_add(1, Ordering::Relaxed);
            metrics.high_water.fetch_max(
                (sender.max_capacity() - sender.capacity()) as u64,
                Ordering::Relaxed,
            );
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            metrics.dropped.fetch_add(1, Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

pub(crate) struct RecordingWriter<W: Write> {
    writer: W,
    pub(crate) bytes: u64,
    max_bytes: u64,
    last_time: u64,
}
impl<W: Write> RecordingWriter<W> {
    pub(crate) fn new(mut writer: W, max_bytes: u64) -> Result<Self> {
        if max_bytes < 8 + FOOTER_BYTES {
            bail!("Recording limit is too small");
        }
        writer.write_all(MAGIC)?;
        Ok(Self {
            writer,
            bytes: 8,
            max_bytes,
            last_time: 0,
        })
    }
    pub(crate) fn packet(&mut self, time: u64, bytes: &[u8]) -> Result<bool> {
        if bytes.is_empty() || bytes.len() > MAX_PACKET || time < self.last_time {
            bail!("Invalid recording packet");
        }
        if self.bytes + 12 + bytes.len() as u64 + FOOTER_BYTES > self.max_bytes {
            return Ok(false);
        }
        self.writer.write_all(&time.to_le_bytes())?;
        self.writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.writer.write_all(bytes)?;
        self.bytes += 12 + bytes.len() as u64;
        self.last_time = time;
        Ok(true)
    }
    pub(crate) fn finish(mut self, time: u64) -> Result<u64> {
        self.writer
            .write_all(&time.max(self.last_time).to_le_bytes())?;
        self.writer.write_all(&0u32.to_le_bytes())?;
        self.writer.flush()?;
        Ok(self.bytes + FOOTER_BYTES)
    }
}
pub(crate) struct RecordingReader<R: Read> {
    reader: R,
    pub(crate) last_time: u64,
    ended: bool,
}
impl<R: Read> RecordingReader<R> {
    pub(crate) fn new(mut reader: R) -> Result<Self> {
        let mut magic = [0; 8];
        reader
            .read_exact(&mut magic)
            .context("Missing recording header")?;
        if &magic != MAGIC {
            bail!("Unsupported recording format");
        }
        Ok(Self {
            reader,
            last_time: 0,
            ended: false,
        })
    }
    pub(crate) fn next(&mut self) -> Result<Option<Packet>> {
        if self.ended {
            return Ok(None);
        }
        let mut header = [0; 12];
        self.reader
            .read_exact(&mut header)
            .context("Truncated recording: missing packet or clean-end footer")?;
        let time = u64::from_le_bytes(header[..8].try_into()?);
        let length = u32::from_le_bytes(header[8..].try_into()?) as usize;
        if time < self.last_time || length > MAX_PACKET {
            bail!("Invalid recording timestamp or packet length");
        }
        self.last_time = time;
        if length == 0 {
            let mut extra = [0];
            if self.reader.read(&mut extra)? != 0 {
                bail!("Trailing data after recording footer");
            }
            self.ended = true;
            return Ok(None);
        }
        let mut bytes = vec![0; length];
        self.reader
            .read_exact(&mut bytes)
            .context("Truncated packet in recording")?;
        Ok(Some(Packet {
            elapsed_ms: time,
            bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn manual_stop_drains_queue_and_closes_raw_ring() {
        let root = std::env::temp_dir().join(format!(
            "vision-manual-stop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("ring");
        let worker_path = path.clone();
        let (sender, receiver) = mpsc::channel(64);
        let metrics = Arc::new(Metrics::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (stats, cancel) = (metrics.clone(), stop.clone());
        let worker = std::thread::spawn(move || -> Result<()> {
            let mut recorder = crate::recording::Recorder::new(Some(crate::recording::Config {
                path: worker_path,
                budget: 1024,
                segment: 128,
                ring: true,
            }));
            let mut time = 0;
            while !cancel.load(Ordering::Relaxed) {
                time += 1;
                recorder.packet(time, &[1; 8]);
                enqueue(
                    &sender,
                    Packet {
                        elapsed_ms: time,
                        bytes: vec![1; 8],
                    },
                    &stats,
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            recorder.finish(time);
            *stats.recording.lock().unwrap() = recorder.report();
            Ok(())
        });
        let mut source = Source {
            receiver,
            metrics,
            capacity: 64,
            stop,
            worker: Some(worker),
        };
        let mut received = 0;
        for _ in 0..2 {
            assert!(source.receiver.recv().await.is_some());
            received += 1;
        }
        source.stop();
        while source.receiver.recv().await.is_some() {
            received += 1;
        }
        source.finish().unwrap();
        assert_eq!(source.report()["recording"]["state"], "closed");
        assert_eq!(source.report()["recording"]["recorded_packets"], received);
        assert_eq!(source.report()["dropped_queue_full"], 0);
        let mut reader = crate::recording::Reader::open(&path).unwrap();
        let mut replayed = 0;
        while reader.next().unwrap().is_some() {
            replayed += 1;
        }
        assert_eq!(replayed, received);
        drop(reader);
        for entry in std::fs::read_dir(&path).unwrap().flatten() {
            std::fs::remove_file(entry.path()).unwrap();
        }
        std::fs::remove_dir(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn recording_round_trip_size_limit_and_truncation() {
        let mut bytes = Vec::new();
        let mut writer = RecordingWriter::new(&mut bytes, 35).unwrap();
        assert!(writer.packet(42, &[1, 2, 3]).unwrap());
        assert!(!writer.packet(43, &[4]).unwrap());
        assert_eq!(writer.finish(50).unwrap(), 35);
        let mut reader = RecordingReader::new(bytes.as_slice()).unwrap();
        let p = reader.next().unwrap().unwrap();
        assert_eq!(p.elapsed_ms, 42);
        assert_eq!(p.bytes, [1, 2, 3]);
        assert!(reader.next().unwrap().is_none());
        for length in [8, 19, 22, 34] {
            let mut reader = RecordingReader::new(&bytes[..length]).unwrap();
            let failed = loop {
                match reader.next() {
                    Err(_) => break true,
                    Ok(None) => break false,
                    _ => {}
                }
            };
            assert!(failed);
        }
    }
    #[tokio::test]
    async fn overflow_is_counted_and_recording_can_precede_queue() {
        let (sender, mut receiver) = mpsc::channel(1);
        let metrics = Metrics::default();
        let mut raw = Vec::new();
        let mut writer = RecordingWriter::new(&mut raw, 100).unwrap();
        for n in 0..3 {
            writer.packet(n, &[n as u8]).unwrap();
            enqueue(
                &sender,
                Packet {
                    elapsed_ms: n,
                    bytes: vec![n as u8],
                },
                &metrics,
            );
        }
        writer.finish(3).unwrap();
        assert_eq!(metrics.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(receiver.recv().await.unwrap().bytes, [0]);
        let mut reader = RecordingReader::new(raw.as_slice()).unwrap();
        let mut count = 0;
        while reader.next().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn startup_burst_fits_bounded_queue_and_drains_in_order() {
        // Real failing login acquired 5332 packets by the first loss report.
        // Hold the consumer for the entire burst, stricter than the live overlap.
        let (sender, mut receiver) = mpsc::channel(8192);
        let metrics = Metrics::default();
        for n in 0..5332 {
            enqueue(
                &sender,
                Packet {
                    elapsed_ms: n,
                    bytes: vec![0; 1500],
                },
                &metrics,
            );
        }
        assert_eq!(metrics.dropped.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.high_water.load(Ordering::Relaxed), 5332);
        drop(sender);
        for n in 0..5332 {
            assert_eq!(receiver.recv().await.unwrap().elapsed_ms, n);
        }
        assert!(receiver.recv().await.is_none());
    }

    #[test]
    #[cfg(windows)]
    fn loss_detection_includes_both_acquisition_queues_and_backend_counters() {
        let (_sender, receiver) = mpsc::channel(1);
        let source = Source {
            receiver,
            metrics: Arc::new(Metrics::default()),
            capacity: 1,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        assert!(!source.has_capture_loss());
        for counter in [
            &source.metrics.dropped,
            &source.metrics.backend.missed_write,
            &source.metrics.backend.missed_read,
            &source.metrics.backend.callback_queue_drops,
            &source.metrics.backend.invalid_descriptors,
        ] {
            counter.store(1, Ordering::Relaxed);
            assert!(source.has_capture_loss());
            counter.store(0, Ordering::Relaxed);
        }
        assert!(!source.has_capture_loss());
    }
}
