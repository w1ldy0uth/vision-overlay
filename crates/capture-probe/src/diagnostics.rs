//! Bounded, heuristic protobuf inspection. Candidates are never inventory updates.
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Serialize)]
pub struct Scalar {
    path: String,
    value: String,
    wire_type: u8,
    matched: bool,
}

#[derive(Default)]
pub struct Diagnostics {
    pub targets: HashSet<u64>,
    counts: BTreeMap<u16, u64>,
    pub emitted: usize,
    pub suppressed: usize,
    pub oversized: usize,
    structure_counts: BTreeMap<u16, usize>,
    pub structures: usize,
}

/// Upstream proto_data concatenates PacketHead and the message body.
/// Inspect the body independently, using the lengths from the frame.
pub fn command_body(command: &auto_artifactarium::GameCommand) -> Option<&[u8]> {
    let start = usize::from(command.header_len);
    let end = start.checked_add(usize::try_from(command.data_len).ok()?)?;
    command.proto_data.get(start..end)
}

impl Diagnostics {
    pub fn structure(&mut self, command: u16, bytes: &[u8]) -> Option<Vec<Scalar>> {
        if bytes.len() > 65536 || self.structures >= 512 {
            return None;
        }
        let count = self.structure_counts.entry(command).or_default();
        if *count >= 16 {
            return None;
        }
        *count += 1;
        self.structures += 1;
        let mut budget = 4096;
        let mut fields = Vec::new();
        inspect(bytes, "", 0, true, &self.targets, &mut budget, &mut fields);
        Some(fields)
    }
    pub fn observe(&mut self, command: u16, bytes: &[u8]) -> Option<Vec<Scalar>> {
        *self.counts.entry(command).or_default() += 1;
        if bytes.len() > 65536 {
            self.oversized += 1;
            return None;
        }
        let mut budget = 4096;
        let mut result = Vec::new();
        inspect(bytes, "", 0, false, &self.targets, &mut budget, &mut result);
        if result.is_empty() {
            return None;
        }
        if self.emitted >= 200 {
            self.suppressed += 1;
            return None;
        }
        self.emitted += 1;
        Some(result)
    }

    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"command_counts": self.counts, "candidates_emitted": self.emitted,
            "candidates_suppressed": self.suppressed, "oversized_commands_skipped": self.oversized,
            "structures_emitted": self.structures, "structure_counts": self.structure_counts})
    }
}

fn varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Some(value);
        }
    }
    None
}

fn inspect(
    bytes: &[u8],
    prefix: &str,
    depth: usize,
    matched_parent: bool,
    targets: &HashSet<u64>,
    budget: &mut usize,
    result: &mut Vec<Scalar>,
) {
    if depth > 6 || *budget == 0 || result.len() >= 128 {
        return;
    }
    let mut cursor = 0;
    let mut scalars = Vec::new();
    let mut nested = Vec::new();
    while cursor < bytes.len() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Some(tag) = varint(bytes, &mut cursor) else {
            return;
        };
        let field = tag >> 3;
        let wire = (tag & 7) as u8;
        if field == 0 || field > 536870911 {
            return;
        }
        let path = if prefix.is_empty() {
            field.to_string()
        } else {
            format!("{prefix}.{field}")
        };
        let value = match wire {
            0 => varint(bytes, &mut cursor),
            1 | 5 => {
                let size = if wire == 1 { 8 } else { 4 };
                let Some(data) = bytes.get(cursor..cursor + size) else {
                    return;
                };
                cursor += size;
                let mut number = [0u8; 8];
                number[..size].copy_from_slice(data);
                Some(u64::from_le_bytes(number))
            }
            2 => {
                let Some(size) = varint(bytes, &mut cursor).and_then(|s| usize::try_from(s).ok())
                else {
                    return;
                };
                let Some(end) = cursor.checked_add(size) else {
                    return;
                };
                let Some(data) = bytes.get(cursor..end) else {
                    return;
                };
                cursor = end;
                nested.push((path, data));
                continue;
            }
            _ => return,
        };
        let Some(value) = value else {
            return;
        };
        scalars.push(Scalar {
            path,
            value: value.to_string(),
            wire_type: wire,
            matched: targets.contains(&value),
        });
    }
    // Keep numeric siblings and descendants of a match. Never emit strings or raw bytes.
    let matched = matched_parent || scalars.iter().any(|s| s.matched);
    if matched {
        result.extend(scalars.into_iter().take(128 - result.len()));
    }
    for (path, data) in nested {
        // Only consider packed GUID-sized values, avoiding ordinary nested
        // messages, whose tags/IDs/text bytes would be small scalar values.
        let mut cursor = 0;
        let mut packed = Vec::new();
        let mut complete = true;
        while cursor < data.len() && packed.len() < 128 && *budget > 0 {
            *budget -= 1;
            let Some(value) = varint(data, &mut cursor) else {
                complete = false;
                break;
            };
            if value <= u64::from(u32::MAX) {
                complete = false;
                break;
            }
            packed.push(Scalar {
                path: format!("{path}.packed[{}]", packed.len()),
                value: value.to_string(),
                wire_type: 2,
                matched: targets.contains(&value),
            });
        }
        if complete && cursor == data.len() && packed.iter().any(|scalar| scalar.matched) {
            result.extend(packed.into_iter().take(128 - result.len()));
        }
        inspect(data, &path, depth + 1, matched, targets, budget, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_watched_guid_in_complete_packed_list_but_not_truncated_list() {
        let mut diagnostics = Diagnostics {
            targets: HashSet::from([1u64 << 40]),
            ..Default::default()
        };
        let rows = diagnostics
            .observe(24980, &[122, 6, 128, 128, 128, 128, 128, 32])
            .unwrap();
        assert!(
            rows.iter().any(|row| row.path == "15.packed[0]"
                && row.value == "1099511627776"
                && row.matched)
        );
        assert!(
            diagnostics
                .observe(24980, &[122, 7, 128, 128, 128, 128, 128, 32, 128])
                .is_none()
        );
    }
    #[test]
    fn separates_frame_header_from_body_and_checks_bounds() {
        let command = auto_artifactarium::GameCommand {
            command_id: 1,
            header_len: 3,
            data_len: 2,
            proto_data: vec![0, 255, 0, 8, 1],
        };
        assert_eq!(command_body(&command), Some(&[8, 1][..]));
        let invalid = auto_artifactarium::GameCommand {
            data_len: 10,
            ..command
        };
        assert!(command_body(&invalid).is_none());
    }
    #[test]
    fn captures_unmatched_numeric_structure_with_per_command_limit() {
        let mut d = Diagnostics::default();
        let fields = d.structure(10, &[8, 7, 18, 2, 8, 9]).unwrap();
        assert_eq!(fields.len(), 2);
        assert!(fields.iter().all(|f| !f.matched));
        for _ in 0..15 {
            assert!(d.structure(10, &[8, 7]).is_some());
        }
        assert!(d.structure(10, &[8, 7]).is_none());
        assert!(d.structure(11, &[8, 7]).is_some());
    }
    #[test]
    fn retains_nested_material_count_next_to_item_id() {
        let mut d = Diagnostics {
            targets: HashSet::from([100051]),
            ..Default::default()
        };
        // Item field 1 = id; material submessage field 5 contains count in field 1.
        let values = d
            .observe(1, &[8, 0xd3, 0x8d, 6, 42, 3, 8, 0xed, 4])
            .unwrap();
        assert!(values.iter().any(|s| s.path == "5.1" && s.value == "621"));
    }
    #[test]
    fn finds_id_and_sibling_count_without_exporting_text() {
        // Nested message: field 1 = 100051, field 2 = 621, field 3 = "secret".
        let bytes = [
            0x32, 15, 8, 0xd3, 0x8d, 6, 16, 0xed, 4, 26, 6, b's', b'e', b'c', b'r', b'e', b't',
        ];
        let mut diagnostics = Diagnostics {
            targets: HashSet::from([100051]),
            ..Default::default()
        };
        let candidates = diagnostics.observe(123, &bytes).unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].path, "6.1");
        assert!(candidates[0].matched);
        assert_eq!(candidates[1].value, "621");
        assert!(
            !serde_json::to_string(&candidates)
                .unwrap()
                .contains("secret")
        );
        assert!(diagnostics.observe(123, &[8, 1]).is_none());
    }
    #[test]
    fn rejects_truncation_overflow_and_limits_recording() {
        let mut d = Diagnostics {
            targets: HashSet::from([1]),
            ..Default::default()
        };
        for bytes in [
            &[8, 128][..],
            &[18, 255, 255][..],
            &[0][..],
            &[8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2][..],
        ] {
            assert!(d.observe(1, bytes).is_none());
        }
        for _ in 0..205 {
            d.observe(2, &[8, 1]);
        }
        assert_eq!(d.emitted, 200);
        assert_eq!(d.suppressed, 5);
        assert!(d.observe(1, &vec![1; 65537]).is_none());
        assert_eq!(d.oversized, 1);
    }
}
