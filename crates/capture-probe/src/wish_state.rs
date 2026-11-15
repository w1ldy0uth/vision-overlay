//! Protocol 7.2 GachaRsp (6373). Only the actual reward is exported, not bonus items.
use anyhow::{Context, Result, ensure};
use protobuf::CodedInputStream;
use serde_json::{Value, json};
fn item(bytes: &[u8]) -> Result<Value> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let (mut id, mut count) = (None, None);
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            8 => {
                ensure!(id.is_none(), "Duplicate wish item");
                id = Some(input.read_uint32()?);
            }
            16 => {
                ensure!(count.is_none(), "Duplicate wish count");
                count = Some(input.read_uint32()?);
            }
            _ => protobuf::rt::skip_field_for_tag(tag, &mut input)?,
        }
    }
    ensure!(
        id.is_some_and(|n| n > 0) && count == Some(1),
        "Invalid wish reward"
    );
    Ok(json!({"item_id":id.unwrap(),"count":1}))
}
pub fn parse(command: u16, body: &[u8]) -> Result<Option<Value>> {
    if command != 6373 {
        return Ok(None);
    }
    ensure!(body.len() <= 262144, "Oversized wish response");
    let mut input = CodedInputStream::from_bytes(body);
    let (mut count, mut banner, mut kind) = (None, None, None);
    let mut results = Vec::new();
    let mut fields = 0;
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        fields += 1;
        ensure!(fields <= 1024, "Too many wish fields");
        match tag {
            40 => {
                ensure!(count.is_none(), "Duplicate wish size");
                count = Some(input.read_uint32()?);
            }
            96 => {
                ensure!(banner.is_none(), "Duplicate wish banner");
                banner = Some(input.read_uint32()?);
            }
            104 => {
                ensure!(kind.is_none(), "Duplicate wish type");
                kind = Some(input.read_uint32()?);
            }
            114 => {
                ensure!(results.len() < 10, "Oversized wish batch");
                let bytes = input.read_bytes()?;
                let mut reward = CodedInputStream::from_bytes(&bytes);
                let mut actual = None;
                let mut n = 0;
                while let Some(t) = reward.read_raw_tag_or_eof()? {
                    n += 1;
                    ensure!(n <= 128, "Too many reward fields");
                    if t == 74 {
                        ensure!(actual.is_none(), "Duplicate primary reward");
                        actual = Some(item(&reward.read_bytes()?)?);
                    } else {
                        protobuf::rt::skip_field_for_tag(t, &mut reward)?;
                    }
                }
                results.push(actual.context("Missing primary wish reward")?);
            }
            _ => protobuf::rt::skip_field_for_tag(tag, &mut input)?,
        }
    }
    if kind.is_none() && results.is_empty() {
        return Ok(None);
    } // failed request without a result
    ensure!(
        matches!(kind, Some(100 | 200 | 301 | 400 | 302 | 500)) && banner.is_some_and(|n| n > 0),
        "Unknown wish identity"
    );
    ensure!(
        matches!(count, Some(1 | 10)) && count == Some(results.len() as u32),
        "Wish result count mismatch"
    );
    Ok(Some(
        json!({"gacha_type":kind.unwrap().to_string(),"gacha_id":banner.unwrap().to_string(),"results":results}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::CodedOutputStream;
    #[test]
    fn single_and_ten_preserve_primary_order_and_ignore_bonus_rewards() {
        for count in [1, 10] {
            let mut bytes = Vec::new();
            {
                let mut out = CodedOutputStream::vec(&mut bytes);
                out.write_uint32(5, count).unwrap();
                out.write_uint32(12, 1000).unwrap();
                out.write_uint32(13, 200).unwrap();
                for n in 0..count {
                    let mut entry = Vec::new();
                    {
                        let mut reward = CodedOutputStream::vec(&mut entry);
                        let mut primary = Vec::new();
                        {
                            let mut p = CodedOutputStream::vec(&mut primary);
                            p.write_uint32(1, 15302 + n).unwrap();
                            p.write_uint32(2, 1).unwrap();
                            p.flush().unwrap();
                        }
                        reward.write_bytes(9, &primary).unwrap();
                        reward.write_bytes(7, &[8, 222, 1, 16, 15]).unwrap();
                        reward.flush().unwrap();
                    }
                    out.write_bytes(14, &entry).unwrap();
                }
                out.flush().unwrap();
            }
            let v = parse(6373, &bytes).unwrap().unwrap();
            assert_eq!(v["results"].as_array().unwrap().len(), count as usize);
            assert_eq!(v["results"][0]["item_id"], 15302);
            assert_eq!(v["gacha_type"], "200");
            let last = bytes.len() - 1;
            assert!(parse(6373, &bytes[..last]).is_err());
        }
        assert!(parse(6373, &[]).unwrap().is_none());
        assert!(parse(7608, &[]).unwrap().is_none());
    }
}
