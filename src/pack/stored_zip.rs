//! A stored (uncompressed) zip: the format of `data/pack/.cache/maps-server.zip`
//! (fflate `zipSync(entries, {level: 0})`), which the server reads at runtime
//! in entry order. Only what that needs: write entries in order, read every
//! entry in central-directory order. Timestamps are fixed (fflate stamps the
//! current time, so the gate compares entries, not bytes).

use indexmap::IndexMap;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// 1980-01-01 00:00:00 in DOS date/time.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

pub fn write(entries: &IndexMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let header = |sig: u32, extra: &mut Vec<u8>| {
            extra.extend_from_slice(&sig.to_le_bytes());
            if sig == CENTRAL {
                extra.extend_from_slice(&20u16.to_le_bytes()); // version made by
            }
            extra.extend_from_slice(&20u16.to_le_bytes()); // version needed
            extra.extend_from_slice(&0u16.to_le_bytes()); // flags
            extra.extend_from_slice(&0u16.to_le_bytes()); // stored
            extra.extend_from_slice(&DOS_TIME.to_le_bytes());
            extra.extend_from_slice(&DOS_DATE.to_le_bytes());
            extra.extend_from_slice(&crc.to_le_bytes());
            extra.extend_from_slice(&(data.len() as u32).to_le_bytes());
            extra.extend_from_slice(&(data.len() as u32).to_le_bytes());
            extra.extend_from_slice(&(name.len() as u16).to_le_bytes());
            extra.extend_from_slice(&0u16.to_le_bytes()); // extra length
            if sig == CENTRAL {
                extra.extend_from_slice(&[0; 8]); // comment len, disk, internal attrs, external attrs (4)
                extra.extend_from_slice(&[0; 2]);
                extra.extend_from_slice(&offset.to_le_bytes());
            }
            extra.extend_from_slice(name.as_bytes());
        };
        header(LOCAL, &mut out);
        out.extend_from_slice(data);
        header(CENTRAL, &mut central);
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&END.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // disk numbers
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

fn u16_at(b: &[u8], i: usize) -> Result<usize, String> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).ok_or_else(|| "truncated zip".to_string())
}

fn u32_at(b: &[u8], i: usize) -> Result<usize, String> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).ok_or_else(|| "truncated zip".to_string())
}

/// Every entry in central-directory order. Only stored entries are supported.
pub fn read(zip: &[u8]) -> Result<IndexMap<String, Vec<u8>>, String> {
    let eocd = (0..zip.len().saturating_sub(21)).rev().find(|&i| u32_at(zip, i).ok() == Some(END as usize)).ok_or("no end of central directory")?;
    let count = u16_at(zip, eocd + 10)?;
    let mut at = u32_at(zip, eocd + 16)?;
    let mut entries = IndexMap::new();
    for _ in 0..count {
        if u32_at(zip, at)? != CENTRAL as usize {
            return Err("bad central directory".into());
        }
        let method = u16_at(zip, at + 10)?;
        let size = u32_at(zip, at + 20)?;
        let (name_len, extra_len, comment_len) = (u16_at(zip, at + 28)?, u16_at(zip, at + 30)?, u16_at(zip, at + 32)?);
        let local = u32_at(zip, at + 42)?;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len).ok_or("truncated zip")?).into_owned();
        if method != 0 {
            return Err(format!("{name}: compressed zip entries are not supported"));
        }
        let data_at = local + 30 + u16_at(zip, local + 26)? + u16_at(zip, local + 28)?;
        entries.insert(name, zip.get(data_at..data_at + size).ok_or("truncated zip")?.to_vec());
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip_keeps_order() {
        let mut e = indexmap::IndexMap::new();
        e.insert("m50_50".to_string(), vec![1, 2, 3]);
        e.insert("a".to_string(), vec![]);
        e.insert("l50_50".to_string(), vec![9; 300]);
        let back = super::read(&super::write(&e)).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.keys().collect::<Vec<_>>(), ["m50_50", "a", "l50_50"]);
    }
}
