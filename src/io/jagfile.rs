//! The client's named archive container.
//!
//! Layout: `p3 unpackedSize, p3 packedSize` then either the raw directory
//! (`unpacked == packed`, each entry bzip2'd on its own) or the whole
//! directory bzip2'd once (`compressWhole`). Directory: `p2 count`, per entry
//! `p4 hash, p3 unpacked, p3 packed`, then the entry bytes in order.
//!
//! Only load / get / write / save exist; `delete`/`rename` and the
//! `KNOWN_NAMES` reverse lookup are unused by the tools and the engine.

use crate::io::bzip2;
use crate::io::packet::Packet;

/// `genHash(name)`: Unicode upper-case, then `(hash * 61 + c - 32) | 0` over UTF-16 units.
pub fn gen_hash(name: &str) -> i32 {
    name.to_uppercase()
        .encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(61).wrapping_add(c as i32 - 32))
}

struct Entry {
    hash: i32,
    unpacked: usize,
    packed: usize,
    /// Offset into `data` for loaded entries.
    pos: usize,
    /// Bytes queued by `write` (already compressed unless `compress_whole`).
    write: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Jagfile {
    data: Vec<u8>,
    entries: Vec<Entry>,
    pub compress_whole: bool,
}

impl Jagfile {
    pub fn new(compress_whole: bool) -> Self {
        Jagfile { compress_whole, ..Default::default() }
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        Ok(Jagfile::from_bytes(std::fs::read(path)?))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut src = Packet::new(bytes);
        let unpacked = src.g3() as usize;
        let packed = src.g3() as usize;
        let compress_whole = unpacked != packed;
        if compress_whole {
            src = Packet::new(bzip2::decompress(&src.data[6..], unpacked));
        }
        let count = src.g2() as usize;
        let mut pos = src.pos + count * 10;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let hash = src.g4s();
            let unpacked = src.g3() as usize;
            let packed = src.g3() as usize;
            entries.push(Entry { hash, unpacked, packed, pos, write: None });
            pos += packed;
        }
        // Not compressed whole: the entire file is kept (6-byte header included)
        // and the offsets above already account for it.
        Jagfile { data: src.data, entries, compress_whole }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hash(&self, index: usize) -> i32 {
        self.entries[index].hash
    }

    /// `get(index)`: the decompressed entry.
    pub fn get(&self, index: usize) -> Option<Vec<u8>> {
        let e = self.entries.get(index)?;
        assert!(e.pos < self.data.len(), "Attempted out of bounds data array access; this is indicative of an improperly constructed Jagfile");
        let end = (e.pos + e.packed).min(self.data.len());
        let src = &self.data[e.pos..end];
        Some(if self.compress_whole { src.to_vec() } else { bzip2::decompress(src, e.unpacked) })
    }

    pub fn has(&self, name: &str) -> bool {
        let h = gen_hash(name);
        self.entries.iter().any(|e| e.hash == h)
    }

    pub fn read(&self, name: &str) -> Option<Packet> {
        let h = gen_hash(name);
        let i = self.entries.iter().position(|e| e.hash == h)?;
        self.get(i).map(Packet::new)
    }

    /// `write(name, src)` with `src.data[0..src.pos]`.
    pub fn write(&mut self, name: &str, data: &[u8]) {
        self.write_hash(gen_hash(name), data);
    }

    /// `write` by hash: replaces an existing entry in place, appends otherwise
    /// (what the queue does at `save` time).
    pub fn write_hash(&mut self, hash: i32, data: &[u8]) {
        let (packed_data, unpacked) =
            if self.compress_whole { (data.to_vec(), data.len()) } else { (bzip2::compress(data), data.len()) };
        let packed = packed_data.len();
        let e = Entry { hash, unpacked, packed, pos: 0, write: Some(packed_data) };
        match self.entries.iter().position(|x| x.hash == hash) {
            Some(i) => self.entries[i] = e,
            None => self.entries.push(e),
        }
    }

    /// The bytes `save(path)` writes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Packet::alloc(5);
        buf.p2(self.entries.len() as i64);
        for e in &self.entries {
            buf.p4(e.hash as i64);
            buf.p3(e.unpacked as i64);
            buf.p3(e.packed as i64);
        }
        for e in &self.entries {
            let data = e.write.as_deref().unwrap_or_else(|| &self.data[e.pos..e.pos + e.packed]);
            buf.pdata(data, 0, data.len());
        }
        let body = if self.compress_whole { bzip2::compress(buf.written()) } else { buf.written().to_vec() };
        assert!(body.len() + 6 <= 2_000_000, "Jagfile.save: Packet.alloc(5) overflow");
        let mut jag = Packet::with_len(body.len() + 6);
        jag.p3(buf.pos as i64);
        jag.p3(body.len() as i64);
        jag.pdata(&body, 0, body.len());
        jag.data
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_bytes())
    }
}
