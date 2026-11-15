use etherparse::{SlicedPacket, TransportSlice};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// pktmon may observe the same handshake at multiple network components.
/// This is a bounded deduplication heuristic, not a server session identifier.
#[derive(Default)]
pub struct HandshakeFilter {
    recent: HashMap<Vec<u8>, Instant>,
}

impl HandshakeFilter {
    pub fn is_duplicate(&mut self, ethernet: &[u8], now: Instant) -> bool {
        let Ok(packet) = SlicedPacket::from_ethernet(ethernet) else {
            return false;
        };
        let Some(TransportSlice::Udp(udp)) = packet.transport else {
            return false;
        };
        let payload = udp.payload();
        if !(4..=20).contains(&payload.len())
            || !matches!(&payload[..4], [0, 0, 0, 255] | [0, 0, 1, 148])
        {
            return false;
        }
        // IP headers can vary by network component; use endpoint addresses, not headers.
        let mut fingerprint = Vec::new();
        match packet.net {
            Some(etherparse::NetSlice::Ipv4(ip)) => {
                fingerprint.extend(ip.header().source());
                fingerprint.extend(ip.header().destination());
            }
            Some(etherparse::NetSlice::Ipv6(ip)) => {
                fingerprint.extend(ip.header().source());
                fingerprint.extend(ip.header().destination());
            }
            _ => return false,
        }
        fingerprint.extend(udp.source_port().to_be_bytes());
        fingerprint.extend(udp.destination_port().to_be_bytes());
        fingerprint.extend(payload);
        self.seen(fingerprint, now)
    }

    fn seen(&mut self, key: Vec<u8>, now: Instant) -> bool {
        self.recent
            .retain(|_, time| now.duration_since(*time) < Duration::from_secs(2));
        if self.recent.contains_key(&key) {
            return true;
        }
        self.recent.insert(key, now);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(payload: &[u8], source: [u8; 4]) -> Vec<u8> {
        let builder = etherparse::PacketBuilder::ethernet2([1; 6], [2; 6])
            .ipv4(source, [127, 0, 0, 1], 64)
            .udp(12345, 22101);
        let mut bytes = Vec::new();
        builder.write(&mut bytes, payload).unwrap();
        bytes
    }
    #[test]
    fn filters_handshake_frames_without_dropping_regular_game_data() {
        let now = Instant::now();
        let mut filter = HandshakeFilter::default();
        let handshake = packet(&[0, 0, 0, 255, 1, 2, 3, 4], [10, 0, 0, 1]);
        assert!(!filter.is_duplicate(&handshake, now));
        assert!(filter.is_duplicate(&handshake, now + Duration::from_millis(1)));
        assert!(!filter.is_duplicate(&packet(&[0, 0, 0, 255, 1, 2, 3, 4], [10, 0, 0, 2]), now));
        let data = packet(&[7; 32], [10, 0, 0, 1]);
        assert!(!filter.is_duplicate(&data, now));
        assert!(!filter.is_duplicate(&data, now));
    }
    #[test]
    fn suppresses_burst_but_allows_new_connection_or_later_retry() {
        let mut filter = HandshakeFilter::default();
        let now = Instant::now();
        assert!(!filter.seen(vec![1], now));
        for millis in 1..16 {
            assert!(filter.seen(vec![1], now + Duration::from_millis(millis)));
        }
        assert!(!filter.seen(vec![2], now + Duration::from_millis(20)));
        assert!(!filter.seen(vec![1], now + Duration::from_secs(3)));
    }
}
