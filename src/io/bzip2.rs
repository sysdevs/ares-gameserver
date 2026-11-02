//! bzip2 via the vendored bzip2-wasm (stock libbz2 1.0.8, B.2).
//!
//! `compress`: `BZ2_bzBuffToBuffCompress(blockSize100k = 1, verbosity = 0,
//! workFactor = 30)` with the 4-byte `BZh1` header removed. The output
//! buffer is `len + 1024` bytes (at least 128) and overflowing it throws.
//! `decompress`: re-adds the header and inflates.

use bzip2::Compression;
use std::io::{Read, Write};

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut enc = bzip2::write::BzEncoder::new(Vec::with_capacity(data.len() + 1024), Compression::new(1));
    enc.write_all(data).expect("in-memory bzip2");
    let out = enc.finish().expect("in-memory bzip2");
    let cap = (data.len() + 1024).max(128);
    assert!(out.len() <= cap, "bzip2: BZ_OUTBUFF_FULL ({} > {cap})", out.len());
    out[4..].to_vec()
}

/// `decompress(data, decompressedLength, prependHeader = true)`.
pub fn decompress(data: &[u8], decompressed_len: usize) -> Vec<u8> {
    let mut src = Vec::with_capacity(data.len() + 4);
    src.extend_from_slice(b"BZh1");
    src.extend_from_slice(data);
    let mut out = Vec::with_capacity(decompressed_len);
    bzip2::read::BzDecoder::new(&src[..]).read_to_end(&mut out).expect("bzip2 decompress");
    assert!(out.len() <= decompressed_len, "bzip2: BZ_OUTBUFF_FULL");
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let c = super::compress(&data);
        assert_ne!(&c[..2], b"BZ");
        assert_eq!(super::decompress(&c, data.len()), data);
    }
}
