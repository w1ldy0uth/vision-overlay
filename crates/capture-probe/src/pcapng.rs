//! Streaming reader for pktmon's ETL-to-pcapng export.
use crate::source::Packet;
use anyhow::{Context, Result, bail};
use pcap_file::{
    DataLink,
    pcapng::{Block, PcapNgReader, blocks::interface_description::InterfaceDescriptionOption},
};
use std::{fs::File, io::BufReader, path::Path};

#[derive(Clone)]
struct Interface {
    link: DataLink,
    resolution: u8,
    offset: i64,
}
pub struct Reader {
    reader: PcapNgReader<BufReader<File>>,
    interfaces: Vec<Interface>,
    origin: Option<i128>,
    last_ms: u64,
    pub timestamp_regressions: u64,
    pub metadata_blocks_skipped: u64,
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).context("Cannot open pcapng export")?;
        Ok(Self {
            reader: PcapNgReader::new(BufReader::new(file)).context("Invalid pcapng header")?,
            interfaces: Vec::new(),
            origin: None,
            last_ms: 0,
            timestamp_regressions: 0,
            metadata_blocks_skipped: 0,
        })
    }
    pub fn next(&mut self) -> Result<Option<Packet>> {
        loop {
            let endian = self.reader.section().endianness;
            let Some(raw) = self.reader.next_raw_block() else {
                break;
            };
            let raw = raw.context("Cannot parse pcapng block framing")?;
            // Statistics/name-resolution/vendor metadata are not packet data.
            // Validate their framing, but don't require their options to be understood.
            if !matches!(raw.type_, 0x0a0d0d0a | 1 | 2 | 3 | 6) {
                self.metadata_blocks_skipped += 1;
                continue;
            }
            let block = match endian {
                pcap_file::Endianness::Little => raw.try_into_block::<byteorder::LittleEndian>(),
                pcap_file::Endianness::Big => raw.try_into_block::<byteorder::BigEndian>(),
            }
            .context("Cannot parse pcapng packet/interface block")?;
            match block {
                Block::SectionHeader(_) => self.interfaces.clear(),
                Block::InterfaceDescription(interface) => {
                    let mut info = Interface {
                        link: interface.linktype,
                        resolution: 6,
                        offset: 0,
                    };
                    for option in interface.options {
                        match option {
                            InterfaceDescriptionOption::IfTsResol(value) => info.resolution = value,
                            InterfaceDescriptionOption::IfTsOffset(value) => {
                                info.offset = value as i64
                            }
                            _ => {}
                        }
                    }
                    self.interfaces.push(info);
                }
                Block::EnhancedPacket(packet) => {
                    let info = self
                        .interfaces
                        .get(packet.interface_id as usize)
                        .context("Unknown pcapng interface")?;
                    if packet.data.len() != packet.original_len as usize {
                        bail!("Truncated pcapng packet: capture must use --pkt-size 0");
                    }
                    if packet.data.is_empty() || packet.data.len() > 131072 {
                        bail!("Invalid pcapng packet size");
                    }
                    let link: u32 = info.link.into();
                    if ![1, 101, 228, 229].contains(&link) {
                        bail!(
                            "Unsupported pcapng link type {link}; choose an Ethernet/IP component"
                        );
                    }
                    let absolute =
                        timestamp_ns(packet.timestamp.as_nanos(), info.resolution, info.offset)?;
                    let origin = *self.origin.get_or_insert(absolute);
                    let relative = absolute.saturating_sub(origin).max(0) / 1_000_000;
                    let ms = u64::try_from(relative).context("Pcapng duration out of range")?;
                    if ms < self.last_ms {
                        self.timestamp_regressions += 1;
                    }
                    self.last_ms = self.last_ms.max(ms);
                    return Ok(Some(Packet {
                        elapsed_ms: self.last_ms,
                        bytes: packet.data.into_owned(),
                    }));
                }
                Block::SimplePacket(_) | Block::Packet(_) => {
                    bail!("Unsupported packet block: expected timestamped Enhanced Packet Blocks")
                }
                _ => {}
            }
        }
        Ok(None)
    }
}
fn timestamp_ns(ticks: u128, resolution: u8, offset: i64) -> Result<i128> {
    // pcap-file exposes raw timestamp units in a Duration named timestamp.
    let units = if resolution & 128 != 0 {
        2u128.checked_pow(u32::from(resolution & 127))
    } else {
        10u128.checked_pow(u32::from(resolution))
    }
    .context("Unsupported timestamp resolution")?;
    let nanos = ticks
        .checked_mul(1_000_000_000)
        .context("Timestamp overflow")?
        / units;
    Ok(i128::try_from(nanos)? + i128::from(offset) * 1_000_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_units_are_not_mistaken_for_nanoseconds() {
        assert_eq!(timestamp_ns(1_500_000, 6, 0).unwrap(), 1_500_000_000);
        assert_eq!(timestamp_ns(1024, 0x8a, -1).unwrap(), 0);
        assert_eq!(timestamp_ns(123456789, 9, 2).unwrap(), 2_123_456_789);
        assert!(timestamp_ns(1, 127, 0).is_err());
    }
}
