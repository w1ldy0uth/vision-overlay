//! Convert plaintext IEEE 802.11 MSDUs to local Ethernet framing.
//! PktMon's WiFi payload type is distinct from Ethernet (including on Windows 10).
//! No decryption, fragment reassembly, or guessed offsets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Truncated,
    Unsupported,
}

fn snap(payload: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    if payload.len() < 8 {
        return Err(Error::Truncated);
    }
    // RFC 1042 and bridge-tunnel SNAP. Other LLC protocols are not IP traffic.
    if payload[..3] != [0xaa, 0xaa, 3] {
        return Ok(None);
    }
    if payload[3..6] != [0, 0, 0] && payload[3..6] != [0, 0, 0xf8] {
        return Ok(None);
    }
    let mut frame = vec![0; 12];
    frame.extend_from_slice(&payload[6..]);
    Ok(Some(frame))
}

pub(crate) fn ethernet(bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let fc = bytes.get(..2).ok_or(Error::Truncated)?;
    if fc[0] & 3 != 0 {
        return Err(Error::Unsupported);
    }
    // Management/control and null-data frames have no application MSDU.
    if (fc[0] >> 2) & 3 != 2 || fc[0] & 0x40 != 0 {
        return Ok(Vec::new());
    }
    if bytes.len() < 24 {
        return Err(Error::Truncated);
    }
    if fc[1] & 4 != 0 || bytes[22] & 15 != 0 {
        return Err(Error::Unsupported);
    }
    let mut offset = 24 + if fc[1] & 3 == 3 { 6 } else { 0 };
    let mut amsdu = false;
    if fc[0] & 0x80 != 0 {
        let qos = bytes.get(offset..offset + 2).ok_or(Error::Truncated)?;
        amsdu = qos[0] & 0x80 != 0;
        offset += 2;
        if fc[1] & 0x80 != 0 {
            offset += 4;
        }
    }
    let body = bytes.get(offset..).ok_or(Error::Truncated)?;
    // Protected radio frames cannot be decrypted here. Drivers can preserve the
    // bit on plaintext offloaded MSDUs, so recognize SNAP rather than guessing.
    if !amsdu {
        let frame = snap(body)?;
        if frame.is_none() && fc[1] & 0x40 != 0 {
            return Err(Error::Unsupported);
        }
        return Ok(frame.into_iter().collect());
    }
    let mut frames = Vec::new();
    let mut pos = 0;
    let mut count = 0;
    while pos < body.len() {
        count += 1;
        if count > 64 {
            return Err(Error::Unsupported);
        }
        let header = body.get(pos..pos + 14).ok_or(Error::Truncated)?;
        let size = u16::from_be_bytes([header[12], header[13]]) as usize;
        if !(8..=2304).contains(&size) {
            return Err(Error::Unsupported);
        }
        let end = pos + 14 + size;
        if let Some(frame) = snap(body.get(pos + 14..end).ok_or(Error::Truncated)?)? {
            frames.push(frame);
        }
        if end == body.len() {
            break;
        }
        pos = end + (4 - ((14 + size) % 4)) % 4;
        if pos > body.len() {
            return Err(Error::Truncated);
        }
    }
    if frames.is_empty() && fc[1] & 0x40 != 0 {
        return Err(Error::Unsupported);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn msdu(ipv6: bool, port: u16) -> Vec<u8> {
        let b = if ipv6 {
            etherparse::PacketBuilder::ipv6([1; 16], [2; 16], 64)
        } else {
            etherparse::PacketBuilder::ipv4([1; 4], [2; 4], 64)
        }
        .udp(40000, port);
        let mut bytes = vec![0xaa, 0xaa, 3, 0, 0, 0];
        bytes.extend_from_slice(if ipv6 { &[0x86, 0xdd] } else { &[8, 0] });
        b.write(&mut bytes, &[1, 2, 3]).unwrap();
        bytes
    }
    fn header(ds: u8, qos: bool, ht: bool) -> Vec<u8> {
        let mut b = vec![0; 24];
        b[0] = if qos { 0x88 } else { 8 };
        b[1] = ds | if ht { 0x80 } else { 0 };
        if ds == 3 {
            b.extend_from_slice(&[0; 6]);
        }
        if qos {
            b.extend_from_slice(&[0; 2]);
            if ht {
                b.extend_from_slice(&[0; 4]);
            }
        }
        b
    }
    #[test]
    fn data_qos_four_addresses_and_ht_preserve_complete_udp() {
        for ds in 0..4 {
            for qos in [false, true] {
                for ht in [false, true] {
                    for v6 in [false, true] {
                        let mut b = header(ds, qos, ht);
                        b.extend(msdu(v6, 22101));
                        let frames = ethernet(&b).unwrap();
                        assert_eq!(frames.len(), 1);
                        let p = etherparse::SlicedPacket::from_ethernet(&frames[0]).unwrap();
                        assert!(
                            matches!(p.transport,Some(etherparse::TransportSlice::Udp(u)) if u.destination_port()==22101 && u.payload()==[1,2,3])
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn aggregated_msdus_have_individual_framing_and_padding() {
        let mut b = header(0, true, false);
        b[24] = 0x80;
        for (i, port) in [22101, 53].into_iter().enumerate() {
            let data = msdu(false, port);
            b.extend_from_slice(&[0; 12]);
            b.extend_from_slice(&(data.len() as u16).to_be_bytes());
            b.extend_from_slice(&data);
            if i == 0 {
                b.extend(vec![0; (4 - ((14 + data.len()) % 4)) % 4]);
            }
        }
        assert_eq!(ethernet(&b).unwrap().len(), 2);
    }
    #[test]
    fn malformed_fragments_and_encrypted_payloads_are_not_guessed() {
        let mut b = header(0, true, true);
        b.extend(msdu(false, 22101));
        for n in 0..b.len() {
            let _ = ethernet(&b[..n]);
        }
        b[1] |= 4;
        assert_eq!(ethernet(&b), Err(Error::Unsupported));
        b[1] &= !4;
        b[22] = 1;
        assert_eq!(ethernet(&b), Err(Error::Unsupported));
        let mut encrypted = header(0, false, false);
        encrypted[1] |= 0x40;
        encrypted.extend([0; 64]);
        assert_eq!(ethernet(&encrypted), Err(Error::Unsupported));
        assert!(ethernet(&[0x80, 0]).unwrap().is_empty());
        assert!(ethernet(&[0x48, 0]).unwrap().is_empty());
    }
    #[test]
    fn offloaded_plaintext_is_recognized_by_snap_not_radio_protection_flag() {
        let mut b = header(0, true, false);
        b[1] |= 0x40;
        b[25] = 1;
        b.extend(msdu(false, 22101));
        assert_eq!(ethernet(&b).unwrap().len(), 1);
    }
}
