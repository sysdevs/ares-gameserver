//! CRCs of the cache archive 0 jags, sent at login
//! and served by the web client.

use crate::io::filestream::FileStream;
use crate::io::packet::Packet;

pub struct CrcTable {
    /// `CrcTable[i]` for every archive 0 file (0 when missing).
    pub crcs: Vec<i32>,
    /// `CrcBuffer`: the first nine CRCs and the hash, big-endian (40 bytes).
    pub buffer: Vec<u8>,
    /// `CrcBuffer32`: CRC of the buffer without the hash.
    pub crc32: i32,
}

impl CrcTable {
    /// `makeCrcs()`. The 40-byte buffer only fits nine CRCs; more files
    /// would make `p4` overflow it, so this panics.
    pub fn new(cache: &mut FileStream) -> Self {
        let mut buf = Packet::with_len(40);
        let mut crcs = Vec::new();
        for i in 0..cache.file_count(0) {
            let crc = cache.read(0, i, false).map_or(0, |jag| Packet::getcrc(&jag, 0, jag.len()));
            crcs.push(crc);
            assert!(buf.pos + 4 <= 40, "RangeError: CrcBuffer overflow at archive 0 file {i}");
            buf.p4(crc as i64);
        }
        // `hash = (hash << 1) + crc`: the sum is unwrapped until the next `<<`;
        // with fewer than nine files it is NaN, which `setInt32` writes as 0
        let mut hash: Option<i64> = Some(1234);
        for i in 0..9 {
            hash = match (hash, crcs.get(i)) {
                (Some(h), Some(&c)) => Some((((h as i32) << 1) as i64) + c as i64),
                _ => None,
            };
        }
        buf.p4(hash.unwrap_or(0));
        let crc32 = Packet::getcrc(&buf.data, 0, buf.data.len() - 4);
        CrcTable { crcs, buffer: buf.data, crc32 }
    }
}
