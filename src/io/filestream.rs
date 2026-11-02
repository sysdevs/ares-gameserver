//! The client cache
//! (`main_file_cache.dat` + `.idx0..4`), 520-byte sectors (B.4).
//!
//! Sector = `p2 file, p2 part, p3 next, p1 archive + 1` + up to 512 bytes.
//! Index entry (`file * 6`) = `p3 size, p3 first sector`. Writes always append
//! new sectors computed from the current `.dat` length; the last sector is not
//! padded. Reads past end of file yield zeros (`Buffer.alloc` + short `readSync`).
//! Comparisons against `length / 6` and `length / 520` are float comparisons.

use crate::io::gzip;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub struct FileStream {
    dat: File,
    idx: Vec<File>,
    pub discard_packed: bool,
    packed: Vec<Vec<Option<Vec<u8>>>>,
}

fn len(f: &File) -> u64 {
    f.metadata().expect("fstat").len()
}

/// `RandomAccessFile.gdata`: zero-filled past end of file.
fn read_at(f: &mut File, pos: u64, n: usize) -> Vec<u8> {
    let mut buf = vec![0; n];
    f.seek(SeekFrom::Start(pos)).expect("seek");
    let mut filled = 0;
    while filled < n {
        match f.read(&mut buf[filled..]).expect("read") {
            0 => break,
            k => filled += k,
        }
    }
    buf
}

fn write_at(f: &mut File, pos: u64, data: &[u8]) {
    f.seek(SeekFrom::Start(pos)).expect("seek");
    f.write_all(data).expect("write");
}

fn g3(b: &[u8]) -> u64 {
    ((b[0] as u64) << 16) | ((b[1] as u64) << 8) | b[2] as u64
}

impl FileStream {
    pub fn open(dir: impl AsRef<Path>, create_new: bool) -> std::io::Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let dat_path = dir.join("main_file_cache.dat");
        if create_new || !dat_path.exists() {
            std::fs::write(&dat_path, [])?;
            for i in 0..=4 {
                std::fs::write(dir.join(format!("main_file_cache.idx{i}")), [])?;
            }
        }
        let open = |p: &Path| -> std::io::Result<File> {
            if !p.exists() {
                std::fs::write(p, [])?;
            }
            OpenOptions::new().read(true).write(true).open(p)
        };
        Ok(FileStream {
            dat: open(&dat_path)?,
            idx: (0..=4).map(|i| open(&dir.join(format!("main_file_cache.idx{i}")))).collect::<Result<_, _>>()?,
            discard_packed: false,
            packed: vec![Vec::new(); 5],
        })
    }

    /// Whole entries in the index (what callers iterate over).
    pub fn file_count(&self, archive: usize) -> usize {
        self.idx.get(archive).map_or(0, |f| len(f) / 6) as usize
    }

    /// `file >= idx.length / 6` (a float compare): `file * 6 >= idx.length`.
    fn out_of_range(&self, archive: usize, file: usize) -> bool {
        archive >= self.idx.len() || file as u64 * 6 >= len(&self.idx[archive])
    }

    fn cached(&self, archive: usize, file: usize) -> Option<&Vec<u8>> {
        self.packed[archive].get(file).and_then(Option::as_ref)
    }

    fn set_cached(&mut self, archive: usize, file: usize, data: Vec<u8>) {
        let p = &mut self.packed[archive];
        if p.len() <= file {
            p.resize(file + 1, None);
        }
        p[file] = Some(data);
    }

    /// `read(archive, file, decompress = false)`.
    pub fn read(&mut self, archive: usize, file: usize, decompress: bool) -> Option<Vec<u8>> {
        if self.out_of_range(archive, file) {
            return None;
        }
        if let Some(d) = self.cached(archive, file) {
            return Some(d.clone());
        }

        let header = read_at(&mut self.idx[archive], file as u64 * 6, 6);
        let size = g3(&header[0..3]) as usize;
        let mut sector = g3(&header[3..6]);
        if size > 2_000_000 || sector == 0 || sector as u64 * 520 > len(&self.dat) {
            return None;
        }

        let mut data = Vec::with_capacity(size);
        let mut part = 0;
        while data.len() < size {
            if sector == 0 {
                break;
            }
            let available = (size - data.len()).min(512);
            let block = read_at(&mut self.dat, sector * 520, available + 8);
            let sector_file = ((block[0] as usize) << 8) | block[1] as usize;
            let sector_part = ((block[2] as usize) << 8) | block[3] as usize;
            let next = g3(&block[4..7]);
            let sector_archive = block[7] as usize;
            if file != sector_file || part != sector_part || archive + 1 != sector_archive {
                return None;
            }
            if next as u64 * 520 > len(&self.dat) {
                return None;
            }
            data.extend_from_slice(&block[8..]);
            sector = next;
            part += 1;
        }
        // A broken chain leaves the tail zeroed (`size` is allocated up front).
        data.resize(size, 0);

        if !decompress {
            if !self.discard_packed {
                self.set_cached(archive, file, data.clone());
            }
            return Some(data);
        }
        if archive == 0 { Some(data) } else { gzip::decompress_gz(&data) }
    }

    /// `write(archive, file, data, version = 0)`: a non-zero version is appended
    /// as two big-endian bytes.
    pub fn write(&mut self, archive: usize, file: usize, data: &[u8], version: u16) -> bool {
        if archive >= self.idx.len() {
            return false;
        }
        let mut data = data.to_vec();
        if version != 0 {
            data.extend_from_slice(&version.to_be_bytes());
        }

        if self.read(archive, file, false).is_some_and(|existing| existing == data) {
            self.set_cached(archive, file, data);
            return true;
        }

        let next_free = |dat: &File| ((len(dat) + 519) / 520).max(1);
        let mut sector = next_free(&self.dat);

        let mut idx_header = [0u8; 6];
        idx_header[..3].copy_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        idx_header[3..].copy_from_slice(&(sector as u32).to_be_bytes()[1..]);
        write_at(&mut self.idx[archive], file as u64 * 6, &idx_header);

        let mut written = 0;
        let mut part: u16 = 0;
        while written < data.len() {
            let mut next = next_free(&self.dat);
            if next == sector {
                next += 1;
            }
            if data.len() - written <= 512 {
                next = 0;
            }
            let mut header = [0u8; 8];
            header[0..2].copy_from_slice(&(file as u16).to_be_bytes());
            header[2..4].copy_from_slice(&part.to_be_bytes());
            header[4..7].copy_from_slice(&(next as u32).to_be_bytes()[1..]);
            header[7] = (archive + 1) as u8;
            write_at(&mut self.dat, sector * 520, &header);

            let available = (data.len() - written).min(512);
            self.dat.write_all(&data[written..written + available]).expect("write");
            written += available;
            sector = next;
            part = part.wrapping_add(1);
        }
        true
    }

    pub fn has(&mut self, archive: usize, file: usize) -> bool {
        if self.out_of_range(archive, file) {
            return false;
        }
        if self.cached(archive, file).is_some() {
            return true;
        }
        let header = read_at(&mut self.idx[archive], file as u64 * 6, 6);
        let size = g3(&header[0..3]);
        let sector = g3(&header[3..6]);
        size <= 2_000_000 && sector != 0 && sector as u64 * 520 <= len(&self.dat)
    }

    pub fn clear_archive(&mut self, archive: usize) {
        if let Some(f) = self.idx.get_mut(archive) {
            f.set_len(0).expect("truncate");
            self.packed[archive].clear();
        }
    }
}
