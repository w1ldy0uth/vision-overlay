//! pktmon can deliver the same traffic as Ethernet or as a bare IP packet.
use etherparse::{NetSlice, SlicedPacket};
use std::borrow::Cow;

pub fn ethernet(bytes: &[u8]) -> (Cow<'_, [u8]>, bool) {
    if SlicedPacket::from_ethernet(bytes).is_ok_and(|packet| packet.net.is_some()) {
        return (Cow::Borrowed(bytes), false);
    }
    let Ok(packet) = SlicedPacket::from_ip(bytes) else {
        return (Cow::Borrowed(bytes), false);
    };
    let ethertype = match packet.net {
        Some(NetSlice::Ipv4(ip)) if usize::from(ip.header().total_len()) == bytes.len() => {
            [0x08, 0x00]
        }
        Some(NetSlice::Ipv6(ip))
            if usize::from(ip.header().payload_length()) + 40 == bytes.len() =>
        {
            [0x86, 0xdd]
        }
        _ => return (Cow::Borrowed(bytes), false),
    };
    // The pinned decoder expects Ethernet. This header is local framing only.
    let mut framed = Vec::with_capacity(bytes.len() + 14);
    framed.extend_from_slice(&[0; 12]);
    framed.extend_from_slice(&ethertype);
    framed.extend_from_slice(bytes);
    (Cow::Owned(framed), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use etherparse::TransportSlice;
    #[test]
    fn accepts_bare_ip_and_preserves_ethernet_without_copying() {
        for ipv6 in [false, true] {
            let builder = if ipv6 {
                etherparse::PacketBuilder::ipv6([1; 16], [2; 16], 64)
            } else {
                etherparse::PacketBuilder::ipv4([10, 0, 0, 1], [10, 0, 0, 2], 64)
            }
            .udp(22101, 50000);
            let mut ip = Vec::new();
            builder.write(&mut ip, &[1, 2, 3, 4]).unwrap();
            let (framed, normalized) = ethernet(&ip);
            assert!(normalized);
            let parsed = SlicedPacket::from_ethernet(&framed).unwrap();
            let Some(TransportSlice::Udp(udp)) = parsed.transport else {
                panic!("Missing UDP");
            };
            assert_eq!(udp.payload(), [1, 2, 3, 4]);
            let (same, changed) = ethernet(&framed);
            assert!(!changed);
            assert!(matches!(same, Cow::Borrowed(_)));
        }
        assert!(!ethernet(&[1, 2, 3]).1);
    }
}
