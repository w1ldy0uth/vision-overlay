//! Parse network packets transmitted between the game and the server
//!
//! Packets are built up in following layers depending on the purpose of the packet:
//!
//! - Packets for connection management ([`GamePacket::Connection`])
//!     - **Ethernet/IP/UDP**, handled using [`etherparse`]
//!     - **[`ConnectionPacket`]**, containing events for connection establishment/disconnection
//! - Packets for game commands ([`GamePacket::Commands`])
//!     - **Ethernet/IP/UDP**, handled using [`etherparse`]
//!     - **KCP**, handled using [`kcp`]
//!         - The KCP header contains an extra field that needs to be removed
//!           to be compatible with the regular KCP protocol
//!     - **[`GameCommand`]**, encrypted using XOR
//!     - **Protobuf**, payload, needs to be parsed into using the types generated in [`gen::proto`]
//!
//! [`GameCommand`]s are encrypted using an XOR-key.
//! One of the first packets sent is a request for a new key from a seed.
//! That key is used for the rest of the packets.
//! This means the recording for packets needs to start before the game starts (train hyperdrive).
//!
//! ## Example
//! ```
//! use auto_artifactarium::{GamePacket, GameSniffer, ConnectionPacket};
//!
//! let packets: Vec<Vec<u8>> = vec![/**/];
//!
//! let mut sniffer = GameSniffer::new();
//! for packet in packets {
//!     match sniffer.receive_packet(packet) {
//!         Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
//!             println!("Disconnected!");
//!             break;
//!         }
//!         Some(GamePacket::Commands(commands)) => {
//!             for command in commands {
//!                 println!("{:?}", command);
//!             }
//!         }
//!         _ => {}
//!     }
//! }
//! ```
//!

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write;

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey};
use tracing::{error, info, info_span, instrument, trace, warn};

use crate::connection::parse_connection_packet;
use crate::crypto::{bruteforce, decrypt_command, lookup_initial_key};
// use crate::gen::protos::GetPlayerTokenRsp;
use crate::Key::Dispatch;
use crate::r#gen::protos::PacketHead;
use crate::kcp::KcpSniffer;
pub use crate::unk_util::Achievement;
pub use crate::unk_util::{
    matches_achievement_all_data_notify, matches_avatars_all_data_notify,
    matches_get_player_token_rsp, matches_items_all_data_notify,
};

fn bytes_as_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, b| {
        let _ = write!(output, "{b:02x}");
        output
    })
}

// pub mod command_id;
pub mod r#gen;

mod connection;
mod crypto;
mod cs_rand;
mod kcp;
mod unk_util;

const PORTS: [u16; 2] = [22101, 22102];

/// Top-level packet sent by the game
pub enum GamePacket {
    Connection(ConnectionPacket),
    Commands(Vec<GameCommand>),
}

/// Packet for connection management
pub enum ConnectionPacket {
    HandshakeRequested,
    Disconnected,
    HandshakeEstablished,
    SegmentData(PacketDirection, Vec<u8>),
}

#[repr(u16)]
enum CommandId {
    AvatarDataNotify = 20143,
    PlayerStoreNotify = 20892,
}

/// Game command header.
///
/// Contains the type of the command in `command_id`
/// and the data encoded in protobuf in `proto_data`
///
/// ## Bit Layout
/// | Bit indices     |  Type |  Name |
/// | - | - | - |
/// |   0..2      |  `u16`  |  Header (magic constant) |
/// |   2..4      |  `u16`  |  command_id |
/// |   4..6      |  `u16`  |  header_len (unsure) |
/// |   6..10     |  `u32`  |  data_len |
/// |  10..10+data_len |  variable  |  proto_data |
/// | data_len..data_len+2  |  `u16`  |  Tail (magic constant) |
#[derive(Clone)]
pub struct GameCommand {
    pub command_id: u16,
    #[allow(unused)]
    pub header_len: u16,
    #[allow(unused)]
    pub data_len: u32,
    pub proto_data: Vec<u8>,
}

impl GameCommand {
    const HEADER_LEN: usize = 10;
    const TAIL_LEN: usize = 2;

    #[instrument(skip(bytes), fields(len = bytes.len()))]
    pub fn try_new(bytes: Vec<u8>) -> Option<Self> {
        let header_overhead = Self::HEADER_LEN + Self::TAIL_LEN;
        if bytes.len() < header_overhead {
            warn!(len = bytes.len(), "game command header incomplete");
            return None;
        }

        if bytes[0] != 0x45
            || bytes[1] != 0x67
            || bytes[bytes.len() - 2] != 0x89
            || bytes[bytes.len() - 1] != 0xAB
        {
            error!("Didn't get magic in try_new!");
            return None;
        }

        // skip header magic const
        let command_id = u16::from_be_bytes(bytes[2..4].try_into().unwrap());
        let header_len = u16::from_be_bytes(bytes[4..6].try_into().unwrap());
        let data_len = u32::from_be_bytes(bytes[6..10].try_into().unwrap());

        let end = 10usize.checked_add(data_len as usize)?.checked_add(header_len as usize)?;
        if end.checked_add(Self::TAIL_LEN)? != bytes.len() {
            warn!("game command length mismatch");
            return None;
        }
        let data = bytes[10..end].to_vec();
        Some(GameCommand {
            command_id,
            header_len,
            data_len,
            proto_data: data,
        })
    }

    pub fn parse_proto<T: protobuf::Message>(&self) -> protobuf::Result<T> {
        T::parse_from_bytes(&self.proto_data)
    }

    pub fn is_avatar_data_notify(&self) -> bool {
        self.command_id == CommandId::AvatarDataNotify as u16
    }

    pub fn is_player_store_notify(&self) -> bool {
        self.command_id == CommandId::PlayerStoreNotify as u16
    }
}

impl fmt::Debug for GameCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameCommand")
            .field("command_id", &self.command_id)
            .field("header_len", &self.header_len)
            .field("data_len", &self.data_len)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum PacketDirection {
    Sent,
    Received,
}

pub enum Key {
    Dispatch(Vec<u8>),
    Session(Vec<u8>),
}

#[derive(Default)]
pub struct GameSniffer {
    decoded_bodies: u64,
    body_decoder: Option<Box<dyn FnMut(&[u8], usize) -> Result<Vec<u8>, String> + Send>>,
    sent_kcp: Option<KcpSniffer>,
    recv_kcp: Option<KcpSniffer>,
    client_seed: Option<u64>,
    client_seed_next_position: usize,
    retained_origin_recoveries: u64,
    max_client_rng_position: usize,
    key: Option<Key>,
    initial_keys: HashMap<u16, Vec<u8>>,
    rsa_keys: Vec<RsaPrivateKey>,
    sent_time: Option<u64>,
    possible_seeds: Vec<u64>,
}

impl GameSniffer {
    pub fn decoded_body_count(&self) -> u64 { self.decoded_bodies }
    /// Optional bounded envelope decoder. No packet or key data is logged here.
    pub fn set_body_decoder(mut self, decoder: impl FnMut(&[u8], usize) -> Result<Vec<u8>, String> + Send + 'static) -> Self {
        self.body_decoder = Some(Box::new(decoder));
        self
    }
    pub fn new() -> Self {
        let pem_data_4 = include_str!("../keys/private_key_4.pem");
        let pem_data_5 = include_str!("../keys/private_key_5.pem");

        let rsa_4 = RsaPrivateKey::from_pkcs1_pem(pem_data_4);
        let rsa_5 = RsaPrivateKey::from_pkcs1_pem(pem_data_5);

        GameSniffer {
            rsa_keys: vec![rsa_4, rsa_5]
                .iter()
                .filter_map(|rsa_key| rsa_key.clone().ok())
                .collect(),
            ..Default::default()
        }
    }

    /// Counts only; no key material or origin timestamps are exposed.
    pub fn recovery_stats(&self) -> (u64, usize) {
        (
            self.retained_origin_recoveries,
            self.max_client_rng_position,
        )
    }

    /// Diagnostic counts only; no tokens, keys or seed values.
    pub fn recovery_context(&self) -> (usize, bool) {
        (self.possible_seeds.len(), self.sent_time.is_some())
    }

    pub fn set_initial_keys(mut self, initial_keys: HashMap<u16, Vec<u8>>) -> Self {
        self.initial_keys = initial_keys;
        self
    }

    #[instrument(skip_all, fields(len = bytes.len()))]
    pub fn receive_packet(&mut self, bytes: Vec<u8>) -> Option<GamePacket> {
        let packet = parse_connection_packet(&PORTS, bytes)?;
        match packet {
            ConnectionPacket::HandshakeRequested => {
                info!("handshake requested, resetting state");
                self.recv_kcp = None;
                self.sent_kcp = None;
                self.key = None;
                Some(GamePacket::Connection(packet))
            }
            ConnectionPacket::HandshakeEstablished | ConnectionPacket::Disconnected => {
                Some(GamePacket::Connection(packet))
            }

            ConnectionPacket::SegmentData(direction, kcp_seg) => {
                let commands = self.receive_kcp_segment(direction, &kcp_seg);
                match commands {
                    Some(commands) => Some(GamePacket::Commands(commands)),
                    None => Some(GamePacket::Connection(ConnectionPacket::SegmentData(
                        direction, kcp_seg,
                    ))),
                }
            }
        }
    }

    fn receive_kcp_segment(
        &mut self,
        direction: PacketDirection,
        kcp_seg: &[u8],
    ) -> Option<Vec<GameCommand>> {
        let kcp = match direction {
            PacketDirection::Sent => &mut self.sent_kcp,
            PacketDirection::Received => &mut self.recv_kcp,
        };

        if kcp.is_none() {
            let new_kcp = KcpSniffer::try_new(kcp_seg)?;
            *kcp = Some(new_kcp);
        }

        if let Some(kcp) = kcp {
            let commands = kcp
                .receive_segments(kcp_seg)
                .into_iter()
                .filter_map(|data| self.receive_command(data))
                .collect();

            return Some(commands);
        }

        None
    }

    #[instrument(skip_all, fields(len = data.len()))]
    fn receive_command(&mut self, mut data: Vec<u8>) -> Option<GameCommand> {
        let key_r = match &self.key {
            None => {
                let key = lookup_initial_key(&self.initial_keys, &data);
                match key {
                    Some(key) => {
                        self.key = Some(Dispatch(key));
                        self.key.as_ref().unwrap()
                    }
                    None => {
                        error!("No dispatch key found");
                        return None;
                    }
                }
            }
            Some(Dispatch(k)) => {
                let mut test = data.clone();
                decrypt_command(k, &mut test);

                if test[0] == 0x45
                    && test[1] == 0x67
                    && test[test.len() - 2] == 0x89
                    && test[test.len() - 1] == 0xAB
                {
                    self.key.as_ref().unwrap()
                } else {
                    let mut discovered_key: Option<&Key> = None;
                    // Try every server candidate against the known origin before doing any
                    // expensive timestamp search. One wrong RSA candidate must not block a
                    // valid retained origin behind thousands of unrelated timestamps.
                    if let Some(origin) = self.client_seed {
                        let start = self.client_seed_next_position.saturating_sub(8);
                        for &seed in &self.possible_seeds {
                            if let Some((position, key)) =
                                crate::crypto::guess_range(origin as i64, seed, start, 64, &data)
                            {
                                self.client_seed_next_position = position + 1;
                                self.max_client_rng_position =
                                    self.max_client_rng_position.max(position + 1);
                                self.retained_origin_recoveries += 1;
                                self.key = Some(Key::Session(key));
                                discovered_key = self.key.as_ref();
                                break;
                            }
                        }
                    }
                    if discovered_key.is_none() {
                        for &seed in &self.possible_seeds {
                            // If that fails, try with a client seed generated from the packet's
                            // `sent_time`
                            if let Some((client_seed, key)) =
                                bruteforce(self.sent_time.unwrap(), seed, data.clone())
                            {
                                self.client_seed = Some(client_seed);
                                self.client_seed_next_position = 0;
                                self.key = Some(Key::Session(key));
                                discovered_key = self.key.as_ref();
                                break;
                            }
                        }
                    }

                    match discovered_key {
                        Some(key) => key,
                        None => {
                            error!("Couldn't bruteforce from deduced keys");
                            return None;
                        }
                    }
                }
            }
            Some(Key::Session(k)) => {
                let mut test = data.clone();
                decrypt_command(k, &mut test);

                if test[0] == 0x45 && test[1] == 0x67 {
                    //|| test[test.len() - 2] == 0x89 && test[test.len() - 1] == 0xAB
                    self.key.as_ref().unwrap()
                } else {
                    warn!("Invalidated session key");
                    self.key = None;
                    error!("Session key dead, relaunch game");
                    return None;
                }
            }
        };

        let key = match key_r {
            Dispatch(k) | Key::Session(k) => k,
        };

        decrypt_command(key, &mut data);

        let mut command = GameCommand::try_new(data)?;
        if let Some(decoder) = &mut self.body_decoder {
            use protobuf::{Message, UnknownValueRef};
            let split = usize::from(command.header_len);
            if let Ok(header) = crate::r#gen::protos::Unk::parse_from_bytes(&command.proto_data[..split])
                && let Some(UnknownValueRef::Varint(size)) = header.unknown_fields().get(8)
                && size > 0
            {
                // Bound before invoking native code or allocating its output.
                if size <= 16 * 1024 * 1024 {
                    match decoder(&command.proto_data[split..], size as usize) {
                        Ok(body) if body.len() == size as usize => {
                            command.proto_data.truncate(split);
                            command.proto_data.extend_from_slice(&body);
                            command.data_len = size as u32;
                            self.decoded_bodies += 1;
                        }
                        _ => warn!(command_id=command.command_id, "Body decompression unavailable or rejected"),
                    }
                }
            }
        }
        let span = info_span!("command", ?command);
        let _enter = span.enter();

        info!("received");
        trace!(data = BASE64_STANDARD.encode(&command.proto_data), "data");

        // if !matches!(
        //     command.command_id,
        //     command_id::GET_PLAYER_TOKEN_RSP | command_id::ACHIEVEMENT_ALL_DATA_NOTIFY
        // ) {
        //     return None;
        // }

        if let Some(Dispatch(_)) = self.key {
            if let Some(possible_seeds) =
                matches_get_player_token_rsp(command.proto_data.clone(), self.rsa_keys.clone())
            {
                self.possible_seeds = possible_seeds;
                info!(?self.possible_seeds, "setting new possible session seeds");
                let header_command = command.parse_proto::<PacketHead>().unwrap();
                self.sent_time = Some(header_command.sent_ms);
                info!(?self.sent_time, "setting new send time");
            }
        }

        Some(command)
    }
}

pub fn matches_achievement_packet(game_command: &GameCommand) -> Option<Vec<Achievement>> {
    return matches_achievement_all_data_notify(game_command.proto_data.clone());
}

pub fn matches_item_packet(game_command: &GameCommand) -> Option<Vec<r#gen::protos::Item>> {
    if !game_command.is_player_store_notify() {
        return None;
    }

    // The command ID already identifies PlayerStoreNotify. The generic heuristic
    // requires ten items to avoid false positives across unrelated commands, but
    // a new account can legitimately have fewer items (or none).
    let packet = game_command.parse_proto::<r#gen::protos::PacketWithItems>().ok()?;
    let mut guids = std::collections::HashSet::new();
    if packet.items.iter().any(|item| item.item_id == 0 || item.guid == 0 || !guids.insert(item.guid)) {
        return None;
    }
    Some(packet.items)
}

pub fn matches_avatar_packet(game_command: &GameCommand) -> Option<Vec<r#gen::protos::AvatarInfo>> {
    if !game_command.is_avatar_data_notify() {
        return None;
    }

    return matches_avatars_all_data_notify(&game_command.proto_data);
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    fn encrypted_frame(origin: i64, server: u64, position: usize) -> Vec<u8> {
        let mut rng = crate::cs_rand::Random::seeded(origin as i32);
        for _ in 0..position {
            rng.next_safe_uint64();
        }
        let key = crate::crypto::new_key_from_seed(rng.next_safe_uint64() ^ server);
        let mut frame = vec![0x45, 0x67, 0x27, 0x0f, 0, 0, 0, 0, 0, 0, 0x89, 0xab];
        crate::crypto::decrypt_command(&key, &mut frame);
        frame
    }
    #[test]
    fn sixth_and_later_logins_recover_before_any_timestamp_search() {
        let origin = 123456;
        let server = 0x12345678;
        let sixth = encrypted_frame(origin, server, 5);
        assert!(crate::crypto::guess(origin, server, 5, sixth.clone()).is_none());
        let mut sniffer = GameSniffer {
            client_seed: Some(origin as u64),
            possible_seeds: vec![server + 1, server],
            ..GameSniffer::new()
        };
        // sent_time intentionally stays None: a timestamp fallback would panic.
        for position in 5..=135 {
            sniffer.key = Some(Key::Dispatch(vec![0; 4096]));
            let command = sniffer
                .receive_command(encrypted_frame(origin, server, position))
                .unwrap();
            assert_eq!(command.command_id, 9999);
            assert_eq!(sniffer.client_seed_next_position, position + 1);
        }
        assert_eq!(sniffer.recovery_stats(), (131, 136));
    }
    #[test]
    fn recovery_window_is_bounded_and_short_or_corrupt_frames_do_not_match() {
        let origin = 123456;
        let server = 0x12345678;
        let frame = encrypted_frame(origin, server, 100);
        assert!(crate::crypto::guess_range(origin, server, 0, 64, &frame).is_none());
        assert_eq!(
            crate::crypto::guess_range(origin, server, 92, 64, &frame)
                .unwrap()
                .0,
            100
        );
        assert!(crate::crypto::guess_range(origin, server, 0, 64, &[]).is_none());
        assert!(crate::crypto::guess_range(origin, server, 0, 64, &[1, 2, 3]).is_none());
        assert!(crate::crypto::guess_range(origin, server, usize::MAX, 2, &frame).is_none());
        let mut corrupt = encrypted_frame(origin, server, 5);
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(crate::crypto::guess_range(origin, server, 0, 64, &corrupt).is_none());
    }
}
