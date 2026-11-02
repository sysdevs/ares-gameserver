//! Standard base64 (`Buffer.toString('base64')`) for saves on the login link.

const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Lenient like `Buffer.from(s, 'base64')`: unknown characters are skipped.
pub fn decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for b in s.bytes() {
        let Some(v) = A.iter().position(|&a| a == b) else { continue };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"\x00\xff\x10\x20"] {
            assert_eq!(super::decode(&super::encode(data)), data);
        }
        assert_eq!(super::encode(b"hello"), "aGVsbG8=");
    }
}
