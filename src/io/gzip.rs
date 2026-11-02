//! Node `zlib.gzipSync` defaults (level 6) with header byte 9
//! (OS) forced to 0. Node links Chromium's zlib fork, so the deflate stream can
//! differ from stock zlib (B.3); gates compare these members decompressed.

use flate2::Compression;
use std::io::{Read, Write};

pub fn compress_gz(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), Compression::new(6));
    enc.write_all(data).expect("in-memory gzip");
    let mut out = enc.finish().expect("in-memory gzip");
    out[9] = 0;
    out
}

/// `decompressGz`: `None` on a malformed stream.
pub fn decompress_gz(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn header_matches_node() {
        let c = super::compress_gz(b"hello hello hello");
        // magic, deflate, no flags, mtime 0, xfl 0 (level 6), os forced to 0
        assert_eq!(&c[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(super::decompress_gz(&c).unwrap(), b"hello hello hello");
    }
}
