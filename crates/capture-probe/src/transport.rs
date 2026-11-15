//! Observation-only transport telemetry; never acknowledges or changes game traffic.
use etherparse::{SlicedPacket, TransportSlice};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Count warning/error call sites without collecting their fields or payloads.
#[derive(Clone, Default)]
pub struct ErrorCounts(Arc<Mutex<BTreeMap<String, u64>>>);
impl ErrorCounts {
    pub fn decoder_failures(&self) -> u64 {
        self.0
            .lock()
            .map(|counts| {
                counts
                    .iter()
                    .filter(|(key, _)| {
                        key.starts_with("auto_artifactarium:") && key.ends_with(":ERROR")
                    })
                    .map(|(_, count)| count)
                    .sum()
            })
            .unwrap_or(0)
    }
    pub fn report(&self) -> Value {
        match self.0.lock() {
            Ok(counts) => json!(*counts),
            Err(_) => json!({"counter_unavailable": true}),
        }
    }
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ErrorCounts {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if *metadata.level() <= tracing::Level::WARN
            && let Ok(mut counts) = self.0.lock()
        {
            let key = format!(
                "{}:{}:{}",
                metadata.target(),
                metadata.line().unwrap_or(0),
                metadata.level()
            );
            *counts.entry(key).or_default() += 1;
        }
    }
}
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
}
impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Incoming => 0,
            Self::Outgoing => 1,
        }
    }
}

#[derive(Default)]
struct Kcp {
    conversation: Option<(u32, u32)>,
    next: Option<u32>,
    pending: BTreeSet<u32>,
    pub pushes: u64,
    pub repeats: u64,
    pub resets: u64,
    pub capacity_exceeded: bool,
}
impl Kcp {
    fn push(&mut self, conversation: (u32, u32), sequence: u32) {
        if self.conversation != Some(conversation) {
            self.conversation = Some(conversation);
            self.next = Some(sequence);
            self.pending.clear();
            self.resets += 1;
        }
        self.pushes += 1;
        let next = self.next.unwrap();
        let distance = sequence.wrapping_sub(next);
        if distance >= (1 << 31) {
            self.repeats += 1;
            return;
        }
        if distance == 0 {
            let mut next = next.wrapping_add(1);
            while self.pending.remove(&next) {
                next = next.wrapping_add(1);
            }
            self.next = Some(next);
        } else if self.pending.contains(&sequence) {
            self.repeats += 1;
        } else if self.pending.len() < 4096 {
            self.pending.insert(sequence);
        } else {
            self.capacity_exceeded = true;
        }
    }
}

#[derive(Default)]
struct Flow {
    packets: u64,
    bytes: u64,
    data_packets: u64,
    malformed: u64,
    commands: u64,
    empty_command_batches: u64,
    no_decoded_commands: u64,
    last_packet: Option<u64>,
    first_data: Option<u64>,
    last_data: Option<u64>,
    last_command: Option<u64>,
    pushes_at_last_command: u64,
    command_counts: BTreeMap<u16, u64>,
    kcp: Kcp,
}

#[derive(Default)]
pub struct Transport {
    pub raw_ip_frames_normalized: u64,
    flows: [Flow; 2],
    pub unclassified: u64,
    pub duplicate_handshakes: u64,
}

impl Transport {
    /// Validate the layout used by the pinned upstream decoder before it slices it.
    pub fn packet(&mut self, bytes: &[u8], now: u64) -> Option<(Direction, bool)> {
        let Ok(packet) = SlicedPacket::from_ethernet(bytes) else {
            self.unclassified += 1;
            return None;
        };
        let Some(TransportSlice::Udp(udp)) = packet.transport else {
            self.unclassified += 1;
            return None;
        };
        let source = [22101, 22102].contains(&udp.source_port());
        let destination = [22101, 22102].contains(&udp.destination_port());
        let direction = match (source, destination) {
            (true, false) => Direction::Incoming,
            (false, true) => Direction::Outgoing,
            _ => {
                self.unclassified += 1;
                return None;
            }
        };
        let flow = &mut self.flows[direction.index()];
        let payload = udp.payload();
        flow.packets += 1;
        flow.bytes += payload.len() as u64;
        flow.last_packet = Some(now);
        if payload.len() <= 20 {
            let valid = payload.len() >= 4;
            if !valid {
                flow.malformed += 1;
            }
            return Some((direction, valid));
        }
        flow.data_packets += 1;
        flow.first_data.get_or_insert(now);
        flow.last_data = Some(now);
        // Game-specific KCP header: 32 bytes, content length at offset 24.
        let mut cursor = 0;
        let mut segments = Vec::new();
        while cursor < payload.len() {
            let Some(header) = payload.get(cursor..cursor + 32) else {
                flow.malformed += 1;
                return Some((direction, false));
            };
            let number =
                |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
            let Some(end) = (cursor + 32).checked_add(number(24) as usize) else {
                flow.malformed += 1;
                return Some((direction, false));
            };
            if end > payload.len() {
                flow.malformed += 1;
                return Some((direction, false));
            }
            if header[8] == 81 {
                segments.push(((number(0), number(4)), number(16)));
            }
            cursor = end;
        }
        for (conversation, sequence) in segments {
            flow.kcp.push(conversation, sequence);
        }
        Some((direction, true))
    }

    pub fn decoded(
        &mut self,
        direction: Direction,
        commands: Option<&[auto_artifactarium::GameCommand]>,
        now: u64,
    ) {
        let flow = &mut self.flows[direction.index()];
        match commands {
            Some([]) => flow.empty_command_batches += 1,
            Some(commands) => {
                flow.commands += commands.len() as u64;
                flow.last_command = Some(now);
                flow.pushes_at_last_command = flow.kcp.pushes.saturating_sub(flow.kcp.repeats);
                for command in commands {
                    *flow.command_counts.entry(command.command_id).or_default() += 1;
                }
            }
            None => flow.no_decoded_commands += 1,
        }
    }

    pub fn reset_kcp(&mut self) {
        for flow in &mut self.flows {
            flow.kcp = Kcp::default();
            flow.pushes_at_last_command = 0;
            flow.first_data = None;
            flow.last_data = None;
            flow.last_command = None;
        }
    }

    pub fn report(&self, now: u64) -> Value {
        let mut result = serde_json::Map::new();
        for direction in [Direction::Incoming, Direction::Outgoing] {
            let flow = &self.flows[direction.index()];
            let baseline = flow.last_command.or(flow.first_data);
            let stalled = baseline.is_some_and(|time| now.saturating_sub(time) >= 5000)
                && flow
                    .last_data
                    .is_some_and(|time| now.saturating_sub(time) < 5000)
                && flow
                    .kcp
                    .pushes
                    .saturating_sub(flow.kcp.repeats)
                    .saturating_sub(flow.pushes_at_last_command)
                    >= 10;
            result.insert(direction.name().to_string(), json!({
                "udp_observations": flow.packets, "udp_payload_bytes": flow.bytes,
                "data_datagrams": flow.data_packets, "malformed_datagrams": flow.malformed,
                "decoded_commands": flow.commands, "command_counts": flow.command_counts,
                "empty_command_batches": flow.empty_command_batches,
                "observations_without_commands": flow.no_decoded_commands,
                "last_packet_age_ms": flow.last_packet.map(|t| now.saturating_sub(t)),
                "last_command_age_ms": flow.last_command.map(|t| now.saturating_sub(t)),
                "possible_decoder_stall": stalled,
                "kcp": {"push_observations": flow.kcp.pushes, "duplicate_or_old_sequences": flow.kcp.repeats,
                    "next_observed_contiguous_sequence": flow.kcp.next, "sequences_ahead_of_gap": flow.kcp.pending.len(),
                    "conversation_observations": flow.kcp.resets, "tracking_capacity_exceeded": flow.kcp.capacity_exceeded}
            }));
        }
        json!({"elapsed_ms": now, "directions": result, "unclassified_frames": self.unclassified,
            "raw_ip_frames_normalized": self.raw_ip_frames_normalized,
            "duplicate_handshakes_filtered": self.duplicate_handshakes,
            "interpretation": "observation_counters_not_proof_of_packet_loss_or_decoder_error"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_decoder_errors_for_bounded_recovery_without_collecting_seeds() {
        use tracing_subscriber::prelude::*;
        let counts = ErrorCounts::default();
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(counts.clone()),
            || {
                for _ in 0..3 {
                    tracing::error!(target:"auto_artifactarium","Couldn't bruteforce from deduced keys");
                }
                tracing::warn!(target:"auto_artifactarium::crypto","Unable to find the encryption key seed.");
            },
        );
        assert_eq!(counts.decoder_failures(), 3);
    }
    #[test]
    fn warning_counter_keeps_call_sites_without_recording_event_fields() {
        use tracing_subscriber::prelude::*;
        let counts = ErrorCounts::default();
        let subscriber = tracing_subscriber::registry().with(counts.clone());
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(private_field = "not-for-report", "test-warning");
            tracing::info!("not-counted");
        });
        let report = counts.report();
        assert_eq!(report.as_object().unwrap().values().next().unwrap(), 1);
        assert_eq!(report.as_object().unwrap().len(), 1);
        assert!(!report.to_string().contains("not-for-report"));
    }
    #[test]
    fn acknowledgements_alone_do_not_trigger_stall_hint() {
        let mut t = Transport::default();
        let mut ack = segment(0);
        ack[8] = 82;
        for time in 0..20 {
            t.packet(&frame(22101, 50000, &ack), time * 1000);
        }
        assert_eq!(
            t.report(19000)["directions"]["incoming"]["possible_decoder_stall"],
            false
        );
    }
    fn frame(source: u16, destination: u16, payload: &[u8]) -> Vec<u8> {
        let builder = etherparse::PacketBuilder::ethernet2([1; 6], [2; 6])
            .ipv4([10, 0, 0, 1], [10, 0, 0, 2], 64)
            .udp(source, destination);
        let mut bytes = Vec::new();
        builder.write(&mut bytes, payload).unwrap();
        bytes
    }
    fn segment(sequence: u32) -> Vec<u8> {
        let mut bytes = vec![0; 32];
        bytes[0..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[8] = 81;
        bytes[16..20].copy_from_slice(&sequence.to_le_bytes());
        bytes
    }
    #[test]
    fn tracks_directions_gaps_and_recovery_without_confusing_duplicates() {
        let mut t = Transport::default();
        for sequence in [10, 12, 12] {
            t.packet(&frame(22101, 50000, &segment(sequence)), 0);
        }
        t.packet(&frame(50000, 22101, &segment(0)), 0);
        assert_eq!(
            t.report(0)["directions"]["incoming"]["kcp"]["sequences_ahead_of_gap"],
            1
        );
        t.packet(&frame(22101, 50000, &segment(11)), 1);
        let r = t.report(1);
        assert_eq!(
            r["directions"]["incoming"]["kcp"]["next_observed_contiguous_sequence"],
            13
        );
        assert_eq!(r["directions"]["outgoing"]["udp_observations"], 1);
        assert_eq!(
            r["directions"]["incoming"]["kcp"]["duplicate_or_old_sequences"],
            1
        );
    }
    #[test]
    fn rejects_truncated_headers_and_payloads_before_upstream_slicing() {
        let mut t = Transport::default();
        for payload in [vec![0; 2], vec![0; 25], {
            let mut b = segment(0);
            b[24] = 1;
            b
        }] {
            assert!(!t.packet(&frame(22101, 50000, &payload), 0).unwrap().1);
        }
        assert_eq!(
            t.report(0)["directions"]["incoming"]["malformed_datagrams"],
            3
        );
    }
    #[test]
    fn stall_hint_requires_recent_traffic_and_clears_after_command() {
        let mut t = Transport::default();
        for sequence in 0..20 {
            t.packet(
                &frame(22101, 50000, &segment(sequence)),
                u64::from(sequence) * 1000,
            );
        }
        assert_eq!(
            t.report(19000)["directions"]["incoming"]["possible_decoder_stall"],
            true
        );
        let c = auto_artifactarium::GameCommand {
            command_id: 1,
            header_len: 0,
            data_len: 0,
            proto_data: vec![],
        };
        t.decoded(Direction::Incoming, Some(&[c]), 19000);
        assert_eq!(
            t.report(19000)["directions"]["incoming"]["possible_decoder_stall"],
            false
        );
        assert_eq!(
            t.report(30000)["directions"]["incoming"]["possible_decoder_stall"],
            false
        );
    }
}
