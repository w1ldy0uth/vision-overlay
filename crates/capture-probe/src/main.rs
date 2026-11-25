use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use auto_artifactarium::{ConnectionPacket, GamePacket, GameSniffer, matches_item_packet};
use base64::{Engine, prelude::BASE64_STANDARD};
use clap::Parser;
use serde::Serialize;
use serde_json::{Value, json};
use tracing_subscriber::prelude::*;

mod abyss;
mod account_state;
mod achievement_research;
mod achievement_state;
mod avatar_research;
#[cfg(windows)]
mod body_codec;
#[cfg(windows)]
mod capture_lock;
mod console;
mod control;
mod diagnostics;
mod equipment_state;
mod event_format;
mod frame;
#[cfg(any(windows, test))]
mod game_process;
mod handshake;
mod inventory;
mod item_state;
mod material_research;
mod material_state;
mod onslaught;
mod pcapng;
mod player_research;
mod profile_protocol;
mod profile_state;
mod recording;
mod source;
mod theater;
mod tracking_health;
mod transport;
mod traveler;
mod wish_state;

#[derive(Parser)]
#[command(
    version,
    about = "Experimental Windows inventory capture and material tracking prototype"
)]
struct Args {
    /// Backend selected by capability; force ETW for compatibility testing.
    #[arg(long, default_value="auto", value_parser=["auto","realtime","etw"])]
    capture_backend: String,
    /// Dispatch-key JSON from a compatible Irminsul checkout. Never printed.
    #[arg(long, required_unless_present = "demo")]
    keys: Option<PathBuf>,
    /// Emit synthetic JSONL and exit without capture or administrator rights.
    #[arg(long)]
    demo: bool,
    /// Create a new JSONL file directly (keeps stdin available for markers).
    #[arg(long)]
    output: Option<PathBuf>,
    /// Local item names used in console messages; missing catalogs keep numeric fallbacks.
    #[arg(long, default_value = "local-data/materials.ru.json")]
    catalog: PathBuf,
    #[arg(long, default_value = "local-data/avatars.ru.json")]
    avatar_catalog: PathBuf,
    #[arg(long, default_value = "local-data/achievements.ru.json")]
    achievement_catalog: PathBuf,
    /// Inspect numeric protobuf fields matching selected materials; not live tracking.
    #[arg(long, requires = "watch_item", conflicts_with = "demo")]
    diagnose: bool,
    /// Begin offline numeric diagnostics at this recorded time; decoding still starts at login.
    #[arg(long, default_value_t = 0, requires_all = ["replay", "diagnose"])]
    diagnostic_start_ms: u64,
    /// Material IDs, comma-separated or repeated. Only used with --diagnose.
    #[arg(long, value_delimiter = ',', requires = "diagnose", value_parser = clap::value_parser!(u32).range(1..))]
    watch_item: Vec<u32>,
    /// Optional diagnostic deadline in seconds. Default: run until Ctrl+C.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400))]
    duration: Option<u64>,
    /// Keep live capture running after the game exits (Ctrl+C still stops it).
    #[arg(long, conflicts_with_all = ["replay", "pcapng", "demo"])]
    no_stop_on_game_exit: bool,
    /// Record per-direction UDP/KCP/decoder counters from startup; no markers needed.
    #[arg(long, conflicts_with = "demo")]
    transport: bool,
    /// Save bounded raw Ethernet observations before decoding (local session data).
    #[arg(long, conflicts_with_all = ["replay", "demo", "pcapng"])]
    record: Option<PathBuf>,
    /// Create a new bounded ring directory containing VORAW segments and manifest.
    #[arg(long, conflicts_with_all = ["record", "replay", "pcapng", "demo"])]
    record_ring: Option<PathBuf>,
    /// Maximum size of one raw ring segment in MiB (must fit total budget).
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u64).range(1..=2048))]
    record_segment_mib: u64,
    /// Replay a recording offline, without pktmon or administrator rights.
    #[arg(long, conflicts_with_all = ["record", "demo", "pcapng"])]
    replay: Option<PathBuf>,
    /// Read pktmon ETL export produced by pktmon etl2pcap (offline).
    #[arg(long, conflicts_with_all = ["record", "replay", "demo"])]
    pcapng: Option<PathBuf>,
    /// Experimental: learn reconnect state from an earlier VORAW of the same game process.
    #[arg(long, requires = "pcapng")]
    warmup_replay: Option<PathBuf>,
    /// Find known material Item structures after a snapshot; observations do not change stock.
    #[arg(long, requires = "pcapng")]
    research_inventory: bool,
    /// Export candidate character profiles from a complete offline recording.
    #[arg(long, conflicts_with = "demo")]
    research_avatars: bool,
    /// Export observed achievement baselines from an offline recording.
    #[arg(long, requires = "replay", conflicts_with = "demo")]
    research_achievements: bool,
    /// Inspect UID, player display fields and Adventure Rank properties offline.
    #[arg(long, requires = "replay", conflicts_with = "demo")]
    research_player: bool,
    /// Apply verified protocol 7.2 counts from snapshots and incoming updates.
    #[arg(long, conflicts_with = "demo")]
    track_materials: bool,
    /// Exercise live health decisions offline using recorded timestamps (no capture).
    #[arg(long, requires_all = ["replay", "track_materials"])]
    simulate_realtime: bool,
    /// Print named, timestamped inventory events during fast offline replay.
    #[arg(long, requires = "replay")]
    log_replay_events: bool,
    /// Restrict live capture to one NIC component ID; default: all NIC sources.
    #[arg(long, conflicts_with_all = ["pcapng", "replay", "demo"], value_parser = clap::value_parser!(u32).range(1..))]
    component: Option<u32>,
    /// Maximum recording size in MiB.
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u64).range(1..=2048))]
    record_max_mib: u64,
    /// Bounded queue capacity in packets.
    #[arg(long, default_value_t = 8192, value_parser = clap::value_parser!(u32).range(1..=16384))]
    queue_capacity: u32,
    /// Per-run control marker. Presence requests a graceful stop; offline replay ignores UI control.
    #[arg(long, conflicts_with_all=["replay","pcapng","demo"])]
    stop_file: Option<PathBuf>,
    /// Create after the realtime NIC stream is ready (used by the elevated helper).
    #[arg(long, conflicts_with_all=["replay","pcapng","demo"])]
    ready_file: Option<PathBuf>,
}

impl Args {
    fn raw_requested(&self) -> bool {
        self.record.is_some() || self.record_ring.is_some()
    }
}

/// Field 8 plus an invalid body signals an unsupported envelope, not a bad RNG guess.
/// Keep the name numeric until the compression algorithm has been verified.
fn unsupported_body_encoding(command: &auto_artifactarium::GameCommand) -> Option<Value> {
    use protobuf::{Message, UnknownValueRef};
    // Only the observed protocol 7.2 token response is verified here.
    if command.command_id != 27808 {
        return None;
    }
    let split = usize::from(command.header_len);
    let header =
        auto_artifactarium::r#gen::protos::Unk::parse_from_bytes(command.proto_data.get(..split)?)
            .ok()?;
    let expected_size = match header.unknown_fields().get(8)? {
        UnknownValueRef::Varint(size) if size > 0 => size,
        _ => return None,
    };
    let body = diagnostics::command_body(command)?;
    if auto_artifactarium::r#gen::protos::Unk::parse_from_bytes(body).is_ok() {
        return None;
    }
    Some(
        json!({"command_id":command.command_id,"body_bytes":body.len(),"header_field_8":expected_size,
        "interpretation":"unsupported_body_encoding_compression_suspected","compression_algorithm_verified":false}),
    )
}

async fn deadline(seconds: Option<u64>) {
    match seconds {
        Some(seconds) => tokio::time::sleep(std::time::Duration::from_secs(seconds)).await,
        None => std::future::pending::<()>().await,
    }
}

#[derive(Serialize)]
struct Material {
    item_id: u32,
    guid: String,
    count: u32,
}

fn report_tracking_pause(source: &source::Source, decoder_stalled: bool) {
    let report = source.report();
    eprintln!(
        "Realtime tracking PAUSED: capture_loss={}, incoming_decoder_stall={decoder_stalled}; queue={}/{}, high_water={}, dropped_queue_full={}, backend={}. Restart capture and game for a fresh snapshot.",
        source.has_capture_loss(),
        report["queue_depth"],
        report["queue_capacity"],
        report["queue_high_water"],
        report["dropped_queue_full"],
        report["realtime_backend"]
    );
}

struct Output<W: Write> {
    writer: W,
    sequence: u64,
    session: u64,
    source_elapsed_ms: Option<u64>,
}

impl<W: Write> Output<W> {
    fn emit(&mut self, event: &str, data: Value) -> Result<()> {
        self.emit_event(event, data).map(|_| ())
    }
    fn emit_event(&mut self, event: &str, data: Value) -> Result<String> {
        self.sequence += 1;
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let timestamp =
            event_format::event_timestamp(&json!({"observed_unix_ms":time.to_string()}))
                .context("Cannot format event timestamp")?;
        serde_json::to_writer(
            &mut self.writer,
            &json!({
                "schema_version": 1, "sequence": self.sequence,
                "session": self.session.to_string(), "observed_unix_ms": time.to_string(),"timestamp":timestamp,
                "source_elapsed_ms": self.source_elapsed_ms,
                "event": event, "data": data,
            }),
        )?;
        writeln!(self.writer)?;
        self.writer.flush()?;
        Ok(timestamp)
    }

    fn change(
        &mut self,
        event: &str,
        data: Value,
        labels: &console::Labels,
        show: bool,
    ) -> Result<()> {
        let line = labels.format(event, &data);
        let stamp = self.emit_event(event, data)?;
        if show {
            eprintln!("[{stamp}] {line}");
        }
        Ok(())
    }

    fn status(&mut self, state: &str) -> Result<()> {
        self.emit(
            "status",
            json!({"state": state, "live_updates_supported": false}),
        )
    }
}

fn load_keys(path: &PathBuf) -> Result<HashMap<u16, Vec<u8>>> {
    let text = fs::read(path).context("Cannot read dispatch-key file")?;
    let encoded: HashMap<u16, String> = serde_json::from_slice(&text).map_err(|_| {
        anyhow::anyhow!("Invalid dispatch-key JSON; expected numeric IDs and base64 strings")
    })?;
    if encoded.is_empty() {
        bail!("Dispatch-key file is empty");
    }
    encoded
        .into_iter()
        .map(|(id, value)| {
            let bytes = BASE64_STANDARD
                .decode(value)
                .map_err(|_| anyhow::anyhow!("Invalid base64 for dispatch-key ID {id}"))?;
            if bytes.is_empty() {
                bail!("Empty dispatch-key ID {id}");
            }
            Ok((id, bytes))
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(windows)]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|v| v == "--body-codec-worker")
    {
        let path = std::env::args_os()
            .nth(2)
            .context("Codec config required")?;
        return body_codec::worker_main(std::path::Path::new(&path));
    }
    let args = Args::parse();
    let error_counts = transport::ErrorCounts::default();
    // Raw recording is opt-in. Library diagnostics stay on stderr at warning level.
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("warn"))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(io::stderr)
                .with_ansi(false),
        )
        .with(error_counts.clone())
        .init();
    let writer: Box<dyn Write> = match &args.output {
        Some(path) => Box::new(io::BufWriter::new(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .context("Cannot create output file (existing files are never overwritten)")?,
        )),
        None => Box::new(io::stdout()),
    };
    let mut output = Output {
        writer,
        sequence: 0,
        session: 0,
        source_elapsed_ms: None,
    };
    if args.demo {
        output.status("demo")?;
        output.emit("inventory_snapshot", json!({"synthetic": true, "completeness": "unverified", "materials": [Material { item_id: 100001, guid: u64::MAX.to_string(), count: 12 }]}))?;
        return output.status("stopped");
    }
    let keys = load_keys(args.keys.as_ref().context("--keys is required")?)?;
    if let Err(error) = capture(keys, &mut output, &args, &error_counts).await {
        if args.transport
            || args.diagnose
            || args.raw_requested()
            || args.replay.is_some()
            || args.pcapng.is_some()
        {
            output.emit("decoder_warning_counts", error_counts.report())?;
        }
        output.status("source_error")?;
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
async fn capture(
    keys: HashMap<u16, Vec<u8>>,
    output: &mut Output<Box<dyn Write>>,
    args: &Args,
    error_counts: &transport::ErrorCounts,
) -> Result<()> {
    let started = std::time::Instant::now();
    let replay = args.replay.is_some() || args.pcapng.is_some();
    if !replay
        && args
            .record_ring
            .as_ref()
            .or(args.record.as_ref())
            .is_some_and(|p| p.exists())
    {
        bail!(
            "Cannot create recording (existing files are never overwritten; ring directory must be new)"
        );
    }
    let _live_instance = if replay {
        None
    } else {
        Some(capture_lock::CaptureLock::acquire()?)
    };
    let labels = console::Labels::load(
        &args.catalog,
        &args.avatar_catalog,
        &args.achievement_catalog,
    )?;
    if args.research_avatars && !replay {
        bail!("--research-avatars requires --replay or --pcapng; live research is not enabled");
    }
    let telemetry =
        args.transport || args.diagnose || args.raw_requested() || args.track_materials || replay;
    if args.record_ring.is_some() && args.record_segment_mib > args.record_max_mib {
        bail!("--record-segment-mib must not exceed --record-max-mib");
    }
    let mut sniffer = GameSniffer::new().set_initial_keys(keys.clone());
    let codec_config = args
        .keys
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or(std::path::Path::new("."))
        .join("body-codec.json");
    if codec_config.is_file() {
        match body_codec::Worker::start(&codec_config) {
            Ok(mut worker) => {
                sniffer = sniffer.set_body_decoder(move |body, size| worker.decode(body, size));
                eprintln!("Version-verified isolated network body decoder ready.");
            }
            Err(_) => eprintln!(
                "Network body decoder unavailable for this client; unsupported envelopes remain explicit errors."
            ),
        }
    }
    let mut source = if let Some(path) = &args.pcapng {
        source::Source::pcapng(path, args.queue_capacity as usize)?
    } else {
        match &args.replay {
            Some(path) => source::Source::replay(path, args.queue_capacity as usize)?,
            None => source::Source::live(
                started,
                args.record_ring
                    .as_ref()
                    .or(args.record.as_ref())
                    .map(|path| recording::Config {
                        path: path.clone(),
                        budget: args.record_max_mib * 1024 * 1024,
                        segment: args.record_segment_mib * 1024 * 1024,
                        ring: args.record_ring.is_some(),
                    }),
                args.queue_capacity as usize,
                args.component,
                args.ready_file.clone(),
                args.capture_backend.clone(),
            )?,
        }
    };
    let mut stopping = false;
    let mut stop_reason = "stopped";
    let mut replay_report_at = 0;
    output.emit(
        "source_config",
        json!({"mode": if args.pcapng.is_some() {"pcapng"} else if replay {"replay"} else {"live"},
        "decoder_revision": "7dafbf0af0bfece23e46382dd3f991d39a5436ac", "decoder_protocol": "7.2",
        "raw_payloads_saved": args.raw_requested(), "queue_capacity": args.queue_capacity,
        "recording_limit_bytes": args.record_max_mib * 1024 * 1024,
        "recording_mode":if args.record_ring.is_some(){"ring"}else if args.record.is_some(){"single"}else{"none"},
        "recording_segment_bytes":args.record_segment_mib*1024*1024,"duration_seconds":args.duration,
        "stop_on_game_exit":!replay && !args.no_stop_on_game_exit,
        "live_backend":"pktmon_capability_selection","link_layer_normalization":"ethernet-ip-wifi-msdu-v1","requested_backend":args.capture_backend,"component":args.component,"track_materials":args.track_materials,"wish_tracking":args.track_materials,
        "replay_timing": "as_fast_as_possible_using_recorded_timestamps"}),
    )?;
    if let Some(path) = &args.warmup_replay {
        let mut warmup = source::Source::replay(path, args.queue_capacity as usize)?;
        let mut filter = handshake::HandshakeFilter::default();
        let mut validation = transport::Transport::default();
        let clock = std::time::Instant::now();
        let mut snapshots = 0;
        let failures_before = error_counts.decoder_failures();
        eprintln!("Learning reconnect state from earlier local recording...");
        while let Some(packet) = warmup.receiver.recv().await {
            let (bytes, _) = frame::ethernet(&packet.bytes);
            if !validation
                .packet(&bytes, packet.elapsed_ms)
                .is_some_and(|(_, safe)| safe)
            {
                continue;
            }
            let time = clock
                .checked_add(std::time::Duration::from_millis(packet.elapsed_ms))
                .context("Warmup timestamp out of range")?;
            if filter.is_duplicate(&bytes, time) {
                continue;
            }
            if let Some(GamePacket::Commands(commands)) = sniffer.receive_packet(bytes.into_owned())
            {
                snapshots += commands
                    .iter()
                    .filter(|command| matches_item_packet(command).is_some())
                    .count();
            }
            if error_counts
                .decoder_failures()
                .saturating_sub(failures_before)
                >= 3
            {
                bail!("Warmup decoder failed; earlier recording cannot establish reconnect state");
            }
        }
        warmup.finish()?;
        if snapshots == 0 {
            bail!("Warmup did not decode an inventory snapshot; reconnect state is unverified");
        }
        output.emit(
            "decoder_warmup",
            json!({"snapshots_decoded":snapshots,"state_scope":"same_game_process_only"}),
        )?;
    }
    let mut handshakes = handshake::HandshakeFilter::default();
    let mut diagnostics = diagnostics::Diagnostics::default();
    let mut known_materials = material_research::KnownMaterials::new();
    let mut material_state = material_state::State::default();
    let mut item_state = item_state::State::default();
    let mut profile_state = profile_state::State::default();
    if args.track_materials {
        match equipment_state::load_types(
            args.catalog
                .parent()
                .unwrap_or(std::path::Path::new("local-data")),
        ) {
            Ok(types) => profile_state.equipment.types = types,
            Err(error) => eprintln!("Equipment slot catalog unavailable: {error:#}"),
        }
    }
    let mut account_state = account_state::State::default();
    let mut achievement_state = achievement_state::State::default();
    let mut avatar_guids = std::collections::HashSet::new();
    let mut tracking_health = tracking_health::Health::default();
    let mut material_observations = 0usize;
    let mut material_commands_scanned = 0usize;
    let mut changed_material_observations = 0usize;
    let mut diagnostic_active = replay;
    let mut transport = transport::Transport::default();
    let mut telemetry_interval = tokio::time::interval(std::time::Duration::from_secs(2));
    telemetry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    diagnostics
        .targets
        .extend(args.watch_item.iter().map(|id| u64::from(*id)));
    let (marker_tx, mut marker_rx) = tokio::sync::mpsc::channel::<String>(16);
    if args.diagnose {
        use std::io::BufRead;
        std::thread::spawn(move || {
            // Bound each line as well as the channel; do not retain arbitrary terminal input.
            let mut input = io::stdin().lock();
            loop {
                let mut line = Vec::new();
                let mut valid = true;
                loop {
                    let Ok(buffer) = input.fill_buf() else {
                        return;
                    };
                    if buffer.is_empty() {
                        return;
                    }
                    let end = buffer.iter().position(|b| *b == b'\n');
                    let count = end.map_or(buffer.len(), |n| n + 1);
                    if line.len() + count <= 256 {
                        line.extend_from_slice(&buffer[..count]);
                    } else {
                        valid = false;
                    }
                    input.consume(count);
                    if end.is_some() {
                        break;
                    }
                }
                if valid {
                    let text = String::from_utf8_lossy(&line).trim().to_owned();
                    if !text.is_empty()
                        && !text.chars().any(char::is_control)
                        && marker_tx.blocking_send(text).is_err()
                    {
                        return;
                    }
                }
            }
        });
        output.emit("diagnostic_config", json!({"diagnostic_version": 3, "watch_item_ids": args.watch_item, "duration_seconds": args.duration, "candidate_limit": 200, "structure_limit": 512, "per_command_structure_limit": 16, "raw_payloads_saved": args.raw_requested()}))?;
        eprintln!(
            "Diagnostic mode: type a marker and Enter before/after an action. {} Candidates are unconfirmed.",
            args.duration
                .map(|n| format!("Auto-stop in {n} seconds."))
                .unwrap_or("Run until Ctrl+C.".into())
        );
    } else {
        drop(marker_tx);
    }
    output.status("waiting_for_session")?;
    if telemetry {
        output.emit("transport_config", json!({"telemetry_version": 1, "interval_ms": 2000, "duration_seconds": args.duration,
            "scope": "capture_cumulative_counters_kcp_state_resets_on_handshake", "raw_payloads_saved": args.raw_requested()}))?;
    }
    if replay {
        eprintln!("Offline replay: no network capture. Diagnostics start immediately.");
    }
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    let control_stop = control::wait_for_stop(args.stop_file.as_deref());
    tokio::pin!(control_stop);
    let deadline = deadline(args.duration);
    tokio::pin!(deadline);
    let watch_game = !replay && !args.no_stop_on_game_exit;
    let mut game_watcher = game_process::Watcher::default();
    let mut game_poll = tokio::time::interval(std::time::Duration::from_secs(1));
    game_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut game_poll_failed = false;
    if watch_game {
        eprintln!(
            "Auto-stop: waiting for {}. Capture stops after the game exits (3-second confirmation).",
            game_process::GAME_EXE
        );
    }
    loop {
        tokio::select! {
            _ = &mut control_stop, if !stopping => {
                stopping=true;
                stop_reason="stopped";
                eprintln!("UI stop requested. Stopping capture and draining the queue.");
                source.stop();
            }
            _ = game_poll.tick(), if watch_game && !stopping => {
                let present = match game_process::is_running() {
                    Ok(present) => {
                        if game_poll_failed {
                            output.emit("game_process_watch", json!({"state":"recovered"}))?;
                        }
                        game_poll_failed = false;
                        Some(present)
                    }
                    Err(error) => {
                        if !game_poll_failed {
                            eprintln!("Game process check failed: {error}. Capture continues; Ctrl+C remains available.");
                            output.emit("game_process_watch", json!({"state":"check_failed","os_error":error.raw_os_error()}))?;
                        }
                        game_poll_failed = true;
                        None
                    }
                };
                match game_watcher.observe(present, started.elapsed()) {
                    Some(game_process::Transition::Started) => {
                        output.emit("game_process_watch", json!({"state":"running","executable":game_process::GAME_EXE}))?;
                        eprintln!("Game process detected; auto-stop armed.");
                    }
                    Some(game_process::Transition::Exited) => {
                        output.emit("game_process_watch", json!({"state":"exited","executable":game_process::GAME_EXE}))?;
                        eprintln!("Game exited. Stopping capture and draining the queue.");
                        stopping = true;
                        stop_reason = "stopped_game_exited";
                        source.stop();
                    }
                    None => {}
                }
            }
            _ = telemetry_interval.tick(), if telemetry && !replay => {
                let report=transport.report(started.elapsed().as_millis() as u64);
                if args.track_materials && tracking_health.observe(source.has_capture_loss(),report["directions"]["incoming"]["possible_decoder_stall"]==true) {
                    material_state.reset(); item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear();
                    output.status("realtime_tracking_paused_capture_unreliable")?;
                    report_tracking_pause(&source, report["directions"]["incoming"]["possible_decoder_stall"]==true);
                }
                output.emit("transport_stats", report)?;
                output.emit("source_stats", source.report())?;
                output.emit("decoder_warning_counts", error_counts.report())?;
            }
            _ = &mut deadline, if args.duration.is_some() && !replay && !stopping => {
                stopping = true;
                stop_reason = "stopped_duration_limit";
                source.stop();
            }
            Some(marker) = marker_rx.recv(), if args.diagnose => {
                diagnostic_active = true;
                output.emit("marker", json!({"label": marker}))?;
                eprintln!("Marker saved.");
            }
            result = &mut interrupt, if !stopping => {
                result.context("Cannot listen for Ctrl+C")?;
                stopping = true;
                stop_reason = if replay { "stopped_replay_interrupted" } else { "stopped" };
                source.stop();
            }
            packet = source.receiver.recv() => {
                let Some(packet) = packet else {
                    let result = source.finish();
                    let (retained_origin_recoveries,max_client_rng_position)=sniffer.recovery_stats();
                    output.emit("decoder_recovery_stats",json!({"retained_origin_recoveries":retained_origin_recoveries,"max_client_rng_position":max_client_rng_position,"compressed_bodies_decoded":sniffer.decoded_body_count()}))?;
                    output.emit("source_summary", source.report())?;
                    let now = source.metrics.elapsed_ms.load(std::sync::atomic::Ordering::Relaxed);
                    output.source_elapsed_ms = Some(now);
                    if args.diagnose { output.emit("diagnostic_summary", diagnostics.summary())?; }
                    if args.research_inventory {
                        output.emit("material_research_summary",json!({"known_materials":known_materials.len(),"incoming_commands_scanned":material_commands_scanned,
                            "observations":material_observations,"differing_from_snapshot":changed_material_observations,
                            "observation_limit":1000,"limit_reached":material_observations>=1000,"inventory_modified":false}))?;
                    }
                    if args.track_materials { output.emit("material_state", material_state.report())?; }
                    if telemetry { output.emit("transport_summary", transport.report(now))?; }
                    result?;
                    if source.report()["dropped_queue_full"].as_u64().unwrap_or(0) > 0 {
                        output.status("capture_incomplete_queue_overflow")?;
                    }
                    output.status(stop_reason)?;
                    break;
                };
                let (framed, normalized) = frame::ethernet(&packet.bytes);
                let simulated_stall = args.simulate_realtime && transport.report(packet.elapsed_ms)["directions"]["incoming"]["possible_decoder_stall"]==true;
                if args.track_materials && (!replay || args.simulate_realtime) && tracking_health.observe(source.has_capture_loss(),simulated_stall) {
                    material_state.reset(); item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear();
                    output.status("realtime_tracking_paused_capture_unreliable")?;
                    report_tracking_pause(&source, simulated_stall);
                }
                if normalized { transport.raw_ip_frames_normalized += 1; }
                let bytes = framed.as_ref();
                let now = packet.elapsed_ms;
                output.source_elapsed_ms = Some(now);
                let Some((direction, safe_to_decode)) = transport.packet(bytes, now) else { continue; };
                if !safe_to_decode { continue; }
                // Filter repeated observations before the upstream parser resets its state.
                let observed = started.checked_add(std::time::Duration::from_millis(now)).context("Recording timestamp out of range")?;
                if handshakes.is_duplicate(bytes, observed) {
                    transport.duplicate_handshakes += 1;
                    continue;
                }
                let decoded = sniffer.receive_packet(bytes.to_vec());
                transport.decoded(direction, match &decoded { Some(GamePacket::Commands(commands)) => Some(commands.as_slice()), _ => None }, now);
                if let Some(GamePacket::Commands(commands)) = &decoded
                    && let Some(detail) = commands.iter().find_map(unsupported_body_encoding)
                {
                    source.stop();
                    output.emit("unsupported_body_encoding", detail)?;
                    output.emit("source_summary",source.report())?;
                    output.status("decoder_failed_unsupported_body_encoding")?;
                    bail!("A game message has an unsupported body encoding (compression suspected). Session key recovery cannot proceed; raw recording retained for research.");
                }
                if error_counts.decoder_failures()>=3 {
                    let (server_candidates,has_sent_time)=sniffer.recovery_context();
                    output.emit("decoder_recovery_context",json!({"server_candidates":server_candidates,"has_sent_time":has_sent_time}))?;
                    source.stop();
                    output.emit("source_summary",source.report())?;
                    output.emit("transport_summary",transport.report(now))?;
                    output.status("decoder_failed_key_recovery")?;
                    bail!("Decoder repeatedly failed to recover a session key; stopping instead of retrying every packet");
                }
                if replay && now >= replay_report_at {
                    output.emit("transport_stats", transport.report(now))?;
                    replay_report_at = now.saturating_add(2000);
                }
                match decoded {
                    Some(GamePacket::Connection(ConnectionPacket::HandshakeRequested)) => {
                        // Upstream resets KCP and the session key itself; preserve its reconnect seed.
                        output.session += 1;
                        tracking_health.session_reset();
                        material_state.reset(); item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear();
                        known_materials.clear();
                        transport.reset_kcp();
                        output.status("waiting_for_snapshot")?;
                    }
                    Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
                        tracking_health.session_reset();
                        material_state.reset(); item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear();
                        known_materials.clear();
                        // The next handshake resets upstream session transport state.
                        transport.reset_kcp();
                        output.status("stale")?;
                    }
                    Some(GamePacket::Commands(commands)) => {
                        for command in commands {
                            if args.track_materials && !replay && direction.name()=="incoming" && command.command_id==27808
                                && let Some(body)=diagnostics::command_body(&command)
                                && let Some(identity)=player_research::parse(command.command_id,body)?
                                && identity["uid"].is_string() {
                                    output.emit("login_identity",json!({"uid":identity["uid"],"source":"verified_login_packet"}))?;
                                }
                            if args.track_materials && !tracking_health.paused() && direction.name()=="incoming" && let Some(body)=diagnostics::command_body(&command) {
                                match account_state.apply(command.command_id,body) {
                                    Ok(events)=>for (kind,data) in events {output.change(kind,data,&labels,kind.ends_with("_changed") && (!replay || args.log_replay_events))?;},
                                    Err(_)=>{
                                        account_state.reset(); profile_state.reset(); achievement_state.reset(); material_state.reset(); item_state.reset(); tracking_health.invalid_update();
                                        output.emit("account_update_rejected",json!({"command_id":command.command_id,"reason":"invalid_account_envelope"}))?;
                                        output.status("realtime_tracking_paused_invalid_update")?;
                                    }
                                }
                            }
                            if args.track_materials && !tracking_health.paused() && direction.name()=="incoming" && command.command_id==6373
                                && let Some(body)=diagnostics::command_body(&command) {
                                match wish_state::parse(command.command_id,body) {
                                    Ok(Some(data))=>output.change("wish_completed",data,&labels,!replay || args.log_replay_events)?,
                                    Ok(None)=>{},
                                    Err(_)=>output.emit("wish_tracking_rejected",json!({"command_id":6373,"reason":"unsupported_wish_response"}))?,
                                }
                            }
                            if args.research_player && direction.name()=="incoming" && let Some(body)=diagnostics::command_body(&command)
                                && let Some(candidate)=player_research::parse(command.command_id,body)? {output.emit("player_profile_candidate",candidate)?;}
                            if args.research_achievements && direction.name()=="incoming" && command.command_id==4164
                                && let Some(body)=diagnostics::command_body(&command) {output.emit("achievement_snapshot_candidate",achievement_research::parse(body)?)?;}
                            if (args.research_avatars || args.track_materials) && direction.name()=="incoming" && command.command_id==20143
                                && let Some(body)=diagnostics::command_body(&command) {
                                    let profiles=avatar_research::parse(body)?;
                                    avatar_guids=profiles.iter().filter_map(|p|p["guid"].as_str()?.parse().ok()).collect();
                                    if args.research_avatars && args.diagnose { diagnostics.targets.extend(avatar_guids.iter().copied()); }
                                    if args.diagnose {diagnostics.targets.extend(profiles.iter().filter(|p|p["avatar_id"].as_u64().is_some_and(|id|args.watch_item.contains(&(id as u32)))).filter_map(|p|p["guid"].as_str()?.parse::<u64>().ok()));}
                                    eprintln!("Character research: {} candidate profiles (command {}).",profiles.len(),command.command_id);
                                    if args.track_materials && !tracking_health.paused() {
                                        let account_profiles:Vec<_>=profiles.iter().filter(|p|p["trial_info_present"]!=true).cloned().collect();
                                        let excluded=profiles.len()-account_profiles.len();
                                        profile_state.snapshot(&account_profiles);
                                        output.emit("character_snapshot",json!({"profiles":account_profiles,"excluded_trial_count":excluded,"raw_profile_count":profiles.len(),"command_id":command.command_id}))?;
                                        for profile in &account_profiles {
                                            if traveler::owned(&profile["avatar_id"]) {
                                                output.emit("traveler_element_baseline",json!({"guid":profile["guid"],"avatar_id":profile["avatar_id"],"current":traveler::subset(profile),"command_id":20143}))?;
                                            }
                                        }
                                    }
                                    if args.research_avatars { output.emit("team_snapshot_candidate",avatar_research::parse_teams(body,&profiles)?)?; }
                                    if args.research_avatars { output.emit("avatar_snapshot_candidate",json!({"command_id":command.command_id,"profiles":profiles,
                                        "interpretation":"unverified_character_fields_not_applied_to_account_state"}))?; }
                            }
                            if args.research_avatars && direction.name()=="incoming" && let Some(body)=diagnostics::command_body(&command) {
                                if let Some(mut candidate)=avatar_research::team_command(command.command_id,body,&avatar_guids)? {candidate["direction"]=json!(direction.name());output.emit("team_protocol_candidate",candidate)?;}
                                let observations=profile_protocol::discover(body,&avatar_guids);
                                if !observations.is_empty() { output.emit("profile_protocol_candidate",json!({"command_id":command.command_id,"observations":observations,"interpretation":"offline_candidate_not_applied"}))?; }
                            }
                            if args.research_avatars && direction.name()=="outgoing" && let Some(body)=diagnostics::command_body(&command)
                                && let Some(mut candidate)=avatar_research::team_command(command.command_id,body,&avatar_guids)? {candidate["direction"]=json!(direction.name());output.emit("team_protocol_candidate",candidate)?;}
                            if args.track_materials && !tracking_health.paused() && let Some(body)=diagnostics::command_body(&command) {
                                match achievement_state.apply(direction.name()=="incoming",command.command_id,body) {
                                    Ok(events)=>for(kind,data)in events {output.change(kind,data,&labels,(!replay || args.simulate_realtime || args.log_replay_events) && kind!="achievement_snapshot")?;},
                                    Err(_)=>{
                                        achievement_state.reset(); profile_state.reset(); account_state.reset(); material_state.reset(); item_state.reset(); avatar_guids.clear(); tracking_health.invalid_update();
                                        output.emit("achievement_update_rejected",json!({"command_id":command.command_id,"reason":"invalid_achievement_envelope"}))?;
                                        output.status("realtime_tracking_paused_invalid_update")?;
                                    }
                                }
                            }
                            if args.track_materials && !tracking_health.paused() && let Some(body)=diagnostics::command_body(&command) {
                                if direction.name()=="incoming" && command.command_id==abyss::COMMAND
                                    && let Ok(snapshot)=abyss::snapshot(body) {
                                    output.emit("abyss_snapshot",snapshot)?;
                                }
                                if direction.name()=="incoming" && matches!(command.command_id,theater::LOGIN_RESPONSE | theater::SCREEN_RESPONSE)
                                    && let Ok(snapshot)=theater::snapshot(command.command_id,body) {
                                    output.emit("theater_snapshot",snapshot)?;
                                }
                                match profile_state.apply(direction.name()=="incoming",command.command_id,body) {
                                    Ok(events)=>for (kind,data) in events {
                                        if kind == "character_added" && let Some(guid)=data["guid"].as_str().and_then(|g|g.parse::<u64>().ok()) {avatar_guids.insert(guid);}
                                        let traveler_baseline=if kind=="character_added" && traveler::owned(&data["avatar_id"]) {
                                            Some(json!({"guid":data["guid"],"avatar_id":data["avatar_id"],"current":traveler::subset(&data["profile"]),"command_id":3231}))
                                        } else {None};
                                        output.change(kind,data,&labels,!replay || args.simulate_realtime || args.log_replay_events)?;
                                        if let Some(baseline)=traveler_baseline {output.emit("traveler_element_baseline",baseline)?;}
                                    },
                                    Err(error)=>{
                                        profile_state.reset(); account_state.reset(); achievement_state.reset(); material_state.reset(); item_state.reset(); tracking_health.invalid_update();
                                        let validation = match error.to_string().as_str() {
                                            "Constellation skill depot mismatch" => "constellation_depot_mismatch",
                                            "Constellation limit exceeded" => "constellation_limit_exceeded",
                                            "Invalid constellation baseline" => "invalid_constellation_baseline",
                                            _ => "invalid_profile_envelope",
                                        };
                                        output.emit("profile_update_rejected",json!({"command_id":command.command_id,"reason":"invalid_profile_envelope","validation":validation}))?;
                                        output.status("realtime_tracking_paused_invalid_update")?;
                                    }
                                }
                            }
                            if let Some(items) = matches_item_packet(&command).filter(|_| direction.name()=="incoming") {
                                let item_count = items.len();
                                if args.research_avatars && args.diagnose { diagnostics.targets.extend(items.iter().map(|item|item.guid)); }
                                let reliable_material_baseline = args.track_materials && tracking_health.snapshot();
                                if reliable_material_baseline { material_state.snapshot(&items); item_state.snapshot(&items); profile_state.equipment.snapshot_items(&inventory::non_materials(&items)); }
                                let materials: Vec<Material> = items.iter().filter(|item| item.has_material())
                                    .map(|item| Material { item_id: item.item_id, guid: item.guid.to_string(), count: item.material().count }).collect();
                                if args.research_inventory {
                                    known_materials=items.iter().filter(|item|item.has_material()).map(|item|(item.guid,(item.item_id,item.material().count))).collect();
                                }
                                if args.diagnose {
                                    for material in &materials {
                                        if args.watch_item.contains(&material.item_id) && let Ok(guid) = material.guid.parse() {
                                            diagnostics.targets.insert(guid);
                                        }
                                    }
                                }
                                output.emit("inventory_snapshot", json!({
                                    "synthetic": false, "command_id": command.command_id,
                                    "completeness": "unverified", "recognized_item_count": item_count,
                                    "material_completeness": if reliable_material_baseline && command.command_id == 20892 {"full"} else {"unverified"},
                                    "material_coverage": if reliable_material_baseline && command.command_id == 20892 {Some("player_store_notify_7_2")} else {None},
                                    "materials": materials,
                                    "items": inventory::non_materials(&items),
                                }))?;
                                output.status(if tracking_health.paused() { "realtime_tracking_paused_capture_unreliable" } else if args.track_materials { if replay { "snapshot_received_offline_material_tracking" } else { "snapshot_received_experimental_realtime_tracking" } } else { "snapshot_received_live_tracking_unavailable" })?;
                                eprintln!("Snapshot received: {item_count} items.");
                            } else {
                                if args.track_materials && !tracking_health.paused() && let Some(body)=diagnostics::command_body(&command) {
                                    let mut handled_item_deletions=0usize;
                                    match item_state.apply(direction.name()=="incoming",command.command_id,body) {
                                        Ok(changes) => for change in changes {
                                            if change["removed"]==true {handled_item_deletions+=1;}
                                            if !change["item"].is_null() { profile_state.equipment.item(&change["item"]); }
                                            output.change("item_changed",json!({"command_id":command.command_id,"change":change}),&labels,!replay || args.simulate_realtime || args.log_replay_events)?;
                                        },
                                        Err(_) => {
                                            item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear(); material_state.reset(); tracking_health.invalid_update();
                                            output.emit("material_update_rejected",json!({"command_id":command.command_id,"reason":"invalid_item_update_envelope"}))?;
                                            output.status("realtime_tracking_paused_invalid_update")?;
                                        }
                                    }
                                    match material_state.apply(direction.name()=="incoming",command.command_id,body) {
                                        Ok((changes, ignored)) => {
                                            let ignored=if command.command_id==24980 {ignored.saturating_sub(handled_item_deletions)} else {ignored};
                                            for change in changes {
                                                output.change("material_changed",json!({"command_id":command.command_id,"change":change,"quantity_semantics":"absolute"}),&labels,!replay || args.simulate_realtime || args.log_replay_events)?;
                                            }
                                            if ignored>0 { output.emit("material_update_ignored",json!({"command_id":command.command_id,"entries":ignored,"reason":"invalid_identity_or_unmatched_material"}))?; }
                                        }
                                        Err(_) => {
                                            output.emit("material_update_rejected",json!({"command_id":command.command_id,"reason":"invalid_update_envelope","inventory_modified":false}))?;
                                            if !replay {
                                                tracking_health.invalid_update();
                                                material_state.reset(); item_state.reset(); profile_state.reset(); account_state.reset(); achievement_state.reset(); avatar_guids.clear();
                                                output.status("realtime_tracking_paused_invalid_update")?;
                                                eprintln!("Realtime tracking PAUSED: invalid material update envelope. Capture retained for investigation.");
                                            }
                                        }
                                    }
                                }
                                if args.research_inventory && direction.name()=="incoming" && material_observations<1000
                                    && let Some(body)=diagnostics::command_body(&command) {
                                    material_commands_scanned += 1;
                                    for observation in material_research::scan(body,&known_materials) {
                                        if material_observations>=1000 {break;}
                                        material_observations+=1;
                                        changed_material_observations += usize::from(observation.differs_from_snapshot);
                                        output.emit("material_observation",json!({"command_id":command.command_id,"direction":direction.name(),"material":observation,
                                            "command_numeric_fields":diagnostics.structure(command.command_id,body),
                                            "interpretation":"unconfirmed_item_structure_not_applied"}))?;
                                    }
                                }
                                if args.diagnose && diagnostic_active && now >= args.diagnostic_start_ms {
                                let Some(body) = diagnostics::command_body(&command) else {
                                    output.emit("diagnostic_invalid_frame", json!({"command_id": command.command_id, "header_len": command.header_len, "data_len": command.data_len, "buffer_len": command.proto_data.len()}))?;
                                    continue;
                                };
                                if let Some(fields) = diagnostics.structure(command.command_id, body) {
                                    output.emit("diagnostic_structure", json!({"command_id": command.command_id,
                                        "direction": direction.name(),
                                        "header_bytes": command.header_len, "body_bytes": body.len(), "fields": fields,
                                        "interpretation": "unconfirmed_structure_not_inventory_update"}))?;
                                }
                                if let Some(fields) = diagnostics.observe(command.command_id, body) {
                                    output.emit("diagnostic_candidate", json!({"command_id": command.command_id,
                                        "direction": direction.name(),
                                        "payload_bytes": command.proto_data.len(), "fields": fields,
                                        "interpretation": "unconfirmed_numeric_match_not_inventory_update"}))?;
                                    eprintln!("Candidate command {} ({} bytes)", command.command_id, command.proto_data.len());
                                }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if telemetry {
        output.emit("decoder_warning_counts", error_counts.report())?;
    }
    Ok(())
}

#[cfg(not(windows))]
async fn capture(
    _: HashMap<u16, Vec<u8>>,
    _: &mut Output<Box<dyn Write>>,
    _: &Args,
    _: &transport::ErrorCounts,
) -> Result<()> {
    bail!("Capture is only supported on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_frame_rejects_mismatched_body_lengths_without_panicking() {
        let mut frame = vec![0x45, 0x67, 0, 1, 0, 0, 0, 0, 0, 1, 8, 0x89, 0xab];
        assert!(auto_artifactarium::GameCommand::try_new(frame.clone()).is_some());
        frame[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(auto_artifactarium::GameCommand::try_new(frame.clone()).is_none());
        frame[6..10].copy_from_slice(&0u32.to_be_bytes());
        assert!(auto_artifactarium::GameCommand::try_new(frame).is_none());
    }
    #[test]
    fn token_encoding_diagnostic_requires_marker_and_invalid_body() {
        let mut command = auto_artifactarium::GameCommand {
            command_id: 27808,
            header_len: 4,
            data_len: 1,
            proto_data: vec![64, 229, 200, 1, 0],
        };
        let report = unsupported_body_encoding(&command).unwrap();
        assert_eq!(report["header_field_8"], 25701);
        assert_eq!(report["body_bytes"], 1);
        command.data_len = 2;
        command.proto_data = vec![64, 229, 200, 1, 8, 1];
        assert!(unsupported_body_encoding(&command).is_none());
        command.header_len = 0;
        assert!(unsupported_body_encoding(&command).is_none());
        command.header_len = 100;
        assert!(unsupported_body_encoding(&command).is_none());
    }
    #[tokio::test(start_paused = true)]
    async fn default_has_no_deadline_even_after_old_capture_limit() {
        let args = Args::try_parse_from(["probe", "--keys", "local.json", "--transport"]).unwrap();
        assert_eq!(args.duration, None);
        assert!(!args.no_stop_on_game_exit);
        let unlimited = tokio::spawn(deadline(args.duration));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(1000)).await;
        assert!(!unlimited.is_finished());
        unlimited.abort();
        let limited = tokio::spawn(deadline(Some(180)));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(181)).await;
        limited.await.unwrap();
    }
    #[test]
    fn game_exit_opt_out_is_live_only() {
        assert!(
            Args::try_parse_from(["probe", "--keys", "local.json", "--no-stop-on-game-exit"])
                .unwrap()
                .no_stop_on_game_exit
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--keys",
                "local.json",
                "--replay",
                "saved.voraw",
                "--no-stop-on-game-exit"
            ])
            .is_err()
        );
    }
    #[test]
    fn realtime_tracking_accepts_live_mode_but_rejects_offline_component_selection() {
        assert!(
            Args::try_parse_from([
                "probe",
                "--keys",
                "local.json",
                "--track-materials",
                "--component",
                "12"
            ])
            .is_ok()
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--keys",
                "local.json",
                "--pcapng",
                "x.pcapng",
                "--component",
                "12"
            ])
            .is_err()
        );
    }
    #[test]
    fn small_and_empty_known_store_snapshots_initialize_tracking() {
        use auto_artifactarium::r#gen::protos::{Item, PacketWithItems};
        use protobuf::Message;
        for count in [0, 1, 2, 9, 10] {
            let mut packet = PacketWithItems::new();
            for n in 0..count {
                let mut item = Item::new();
                item.item_id = 100073 + n;
                item.guid = 1000 + u64::from(n);
                item.mut_material().count = 1;
                packet.items.push(item);
            }
            let bytes = packet.write_to_bytes().unwrap();
            let mut command = auto_artifactarium::GameCommand {
                command_id: 20892,
                header_len: 0,
                data_len: bytes.len() as u32,
                proto_data: bytes,
            };
            let items = matches_item_packet(&command).expect("Known small baseline");
            assert_eq!(items.len(), count as usize);
            let mut health = tracking_health::Health::default();
            let mut state = material_state::State::default();
            assert!(health.snapshot());
            state.snapshot(&items);
            let mut update = Vec::new();
            {
                let mut out = protobuf::CodedOutputStream::vec(&mut update);
                let mut crab = Item::new();
                crab.item_id = 100073;
                crab.guid = 1000;
                crab.mut_material().count = if count == 0 { 1 } else { 2 };
                out.write_bytes(1, &crab.write_to_bytes().unwrap()).unwrap();
                out.flush().unwrap();
            }
            let changes = state.apply(true, 7608, &update).unwrap().0;
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].delta, 1);
            assert_eq!(changes[0].new_entry, count == 0);
            command.command_id = 7608;
            assert!(
                matches_item_packet(&command).is_none(),
                "Other messages must not become empty snapshots"
            );
        }
    }
    #[test]
    fn known_store_snapshot_rejects_invalid_identity_duplicates_and_truncated_body() {
        use auto_artifactarium::r#gen::protos::{Item, PacketWithItems};
        use protobuf::Message;
        for (id, guid, duplicate) in [(0, 1, false), (100073, 0, false), (100073, 1, true)] {
            let mut packet = PacketWithItems::new();
            let mut item = Item::new();
            item.item_id = id;
            item.guid = guid;
            item.mut_material().count = 1;
            packet.items.push(item.clone());
            if duplicate {
                packet.items.push(item);
            }
            let bytes = packet.write_to_bytes().unwrap();
            let command = auto_artifactarium::GameCommand {
                command_id: 20892,
                header_len: 0,
                data_len: bytes.len() as u32,
                proto_data: bytes,
            };
            assert!(matches_item_packet(&command).is_none());
        }
        let command = auto_artifactarium::GameCommand {
            command_id: 20892,
            header_len: 0,
            data_len: 3,
            proto_data: vec![18, 5, 8],
        };
        assert!(matches_item_packet(&command).is_none());
    }
    #[test]
    fn recognizes_protocol_72_inventory_message() {
        fn integer(mut value: u64, bytes: &mut Vec<u8>) {
            while value >= 128 {
                bytes.push((value as u8 & 127) | 128);
                value >>= 7;
            }
            bytes.push(value as u8);
        }
        let mut payload = Vec::new();
        for n in 0..10 {
            let mut item = vec![8];
            integer(100051 + n, &mut item);
            item.push(16);
            integer(1000 + n, &mut item);
            item.extend([42, 2, 8, (n + 1) as u8]);
            payload.push(18);
            integer(item.len() as u64, &mut payload);
            payload.extend(item);
        }
        let command = auto_artifactarium::GameCommand {
            command_id: 20892,
            header_len: 0,
            data_len: payload.len() as u32,
            proto_data: payload,
        };
        let items = matches_item_packet(&command).expect("Protocol 7.2 inventory not recognized");
        assert_eq!(items.len(), 10);
        assert_eq!(items[0].item_id, 100051);
        assert_eq!(items[0].material().count, 1);
    }

    #[test]
    fn jsonl_preserves_guid_precision_and_session_boundary() {
        let mut output = Output {
            writer: Vec::new(),
            sequence: 0,
            session: 1,
            source_elapsed_ms: None,
        };
        output.emit("inventory_snapshot", json!({"materials": [Material { item_id: 1, guid: u64::MAX.to_string(), count: 5 }]})).unwrap();
        output.session = 2;
        output.status("stale").unwrap();
        let text = String::from_utf8(output.writer).unwrap();
        let events: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0]["data"]["materials"][0]["guid"],
            u64::MAX.to_string()
        );
        assert_eq!(events[1]["sequence"], 2);
        assert_eq!(events[1]["session"], "2");
        assert_eq!(events[1]["data"]["live_updates_supported"], false);
    }
}
