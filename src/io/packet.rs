//! A fixed-size big-endian byte buffer with a cursor.
//!
//! Rules (docs/behaviour.md B.5):
//! - Writers take `i64`. Every write behaves like `DataView.setUint8/16` or
//!   `setInt32`, i.e. JS `ToUint8`/`ToUint16`/`ToInt32` (wrap modulo 2^n,
//!   NaN → 0). A caller holding a script `f64` converts it with
//!   [`crate::util::numbers::wrap_f64`] first; the low 32 bits are all any writer keeps.
//! - Out-of-range `DataView` access throws; here it panics at the same
//!   point. Where `this.pos++` is evaluated as an argument before the call
//!   throws, `pos` is bumped before the panic too.
//! - Typed-array (`this.data[i]`) writes out of range are silently dropped and
//!   reads yield `undefined` (→ 0 under bit ops); `p_bit` mirrors that.
//! - Strings are written as the low byte of each UTF-16 code unit and read
//!   back one byte per char (Latin-1).
//! - The free-list pool of `alloc/release` is not reproduced: buffers come
//!   back zeroed. ponytail: if a site is found whose output depends on stale
//!   pooled bytes (only possible via `p_bit` partial bytes), reproduce it there.

use num_bigint::BigInt;

pub struct Packet {
    pub data: Vec<u8>,
    pub pos: usize,
    pub bit_pos: usize,
}

/// `Packet.bitmask`: `(1 << i) - 1`, with `bitmask[32] = 0xffffffff`.
const fn bitmask(i: usize) -> u32 {
    if i >= 32 { u32::MAX } else { (1u32 << i) - 1 }
}

impl Packet {
    pub fn new(data: Vec<u8>) -> Self {
        Packet { data, pos: 0, bit_pos: 0 }
    }

    pub fn with_len(len: usize) -> Self {
        Packet::new(vec![0; len])
    }

    /// `Packet.alloc(type)`: 0..=5 are the pooled sizes, anything else is a length.
    pub fn alloc(kind: usize) -> Self {
        Packet::with_len(match kind {
            0 => 100,
            1 => 5000,
            2 => 30000,
            3 => 100000,
            4 => 500000,
            5 => 2000000,
            n => n,
        })
    }

    /// `Packet.load(path, seekToEnd)`.
    pub fn load(path: impl AsRef<std::path::Path>, seek_to_end: bool) -> std::io::Result<Self> {
        let mut p = Packet::new(std::fs::read(path)?);
        if seek_to_end {
            p.pos = p.data.len();
        }
        Ok(p)
    }

    /// `save(filePath, length = pos, start = 0)`.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, &self.data[..self.pos])
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn available(&self) -> isize {
        self.data.len() as isize - self.pos as isize
    }

    /// Bytes written so far (`data.subarray(0, pos)`).
    pub fn written(&self) -> &[u8] {
        &self.data[..self.pos]
    }

    // ---------------------------------------------------------------- crc

    /// `Packet.getcrc(src, offset, length)`: note the loop runs `offset..length`,
    /// not `offset..offset + length` (deliberate, harmless at offset 0).
    pub fn getcrc(src: &[u8], offset: usize, length: usize) -> i32 {
        if offset >= length {
            return 0;
        }
        crc32fast::hash(&src[offset..length]) as i32
    }

    pub fn checkcrc(src: &[u8], offset: usize, length: usize, expected: i32) -> bool {
        Packet::getcrc(src, offset, length) == expected
    }

    // ---------------------------------------------------------------- reads

    fn take<const N: usize>(&mut self) -> [u8; N] {
        let b: [u8; N] = self.data[self.pos..self.pos + N].try_into().unwrap();
        self.pos += N;
        b
    }

    fn post_inc(&mut self) -> usize {
        let p = self.pos;
        self.pos += 1;
        p
    }

    pub fn g1(&mut self) -> i32 {
        let p = self.post_inc();
        self.data[p] as i32
    }

    pub fn g1b(&mut self) -> i32 {
        let p = self.post_inc();
        self.data[p] as i8 as i32
    }

    pub fn g2(&mut self) -> i32 {
        u16::from_be_bytes(self.take()) as i32
    }

    pub fn g2s(&mut self) -> i32 {
        i16::from_be_bytes(self.take()) as i32
    }

    pub fn ig2(&mut self) -> i32 {
        u16::from_le_bytes(self.take()) as i32
    }

    /// `g3` reads through the typed array, not the DataView: out of range it
    /// would yield NaN. Not reproduced; it panics.
    pub fn g3(&mut self) -> i32 {
        let b: [u8; 3] = self.take();
        ((b[0] as i32) << 16) + ((b[1] as i32) << 8) + b[2] as i32
    }

    /// `g3s`: the sign test `v > 0xFFFFFF` can never be true, so this is `g3`.
    pub fn g3s(&mut self) -> i32 {
        self.g3()
    }

    pub fn g4(&mut self) -> u32 {
        u32::from_be_bytes(self.take())
    }

    pub fn g4s(&mut self) -> i32 {
        i32::from_be_bytes(self.take())
    }

    pub fn g8(&mut self) -> i64 {
        i64::from_be_bytes(self.take())
    }

    pub fn gbool(&mut self) -> bool {
        self.g1() == 1
    }

    /// `gjstr(terminator = 10)`: stops at the terminator *or* when the cursor
    /// reaches the end, and in the latter case the last byte is dropped.
    pub fn gjstr(&mut self, terminator: u8) -> String {
        let length = self.data.len();
        let mut s = String::new();
        loop {
            let p = self.post_inc();
            let b = self.data[p];
            if b == terminator || self.pos >= length {
                break;
            }
            s.push(b as char);
        }
        s
    }

    /// `gsmart`: signed (`g1 - 0x40` / `g2 - 0xc000`), the counterpart of `psmarts`.
    pub fn gsmart(&mut self) -> i32 {
        if self.data[self.pos] < 0x80 { self.g1() - 0x40 } else { self.g2() - 0xc000 }
    }

    /// `gsmarts`: unsigned (`g1` / `g2 - 0x8000`), the counterpart of `psmart`.
    pub fn gsmarts(&mut self) -> i32 {
        if self.data[self.pos] < 0x80 { self.g1() } else { self.g2() - 0x8000 }
    }

    pub fn gdata(&mut self, dest: &mut [u8], offset: usize, length: usize) {
        dest[offset..offset + length].copy_from_slice(&self.data[self.pos..self.pos + length]);
        self.pos += length;
    }

    pub fn g_var_int(&mut self) -> u32 {
        let p = self.post_inc();
        let mut byte = self.data[p] as i32;
        let mut result: i32 = 0;
        while byte & 0x80 != 0 {
            result = (result | (byte & 0x7f)).wrapping_shl(7);
            let p = self.post_inc();
            byte = self.data[p] as i32;
        }
        (result | byte) as u32
    }

    // ---------------------------------------------------------------- writes

    pub fn p1(&mut self, value: i64) {
        let p = self.post_inc();
        self.data[p] = value as u8;
    }

    pub fn p2(&mut self, value: i64) {
        self.data[self.pos..self.pos + 2].copy_from_slice(&(value as u16).to_be_bytes());
        self.pos += 2;
    }

    pub fn ip2(&mut self, value: i64) {
        self.data[self.pos..self.pos + 2].copy_from_slice(&(value as u16).to_le_bytes());
        self.pos += 2;
    }

    /// `p3`: `setUint8(pos++, value >> 16)` then `setUint16(pos, value)`.
    pub fn p3(&mut self, value: i64) {
        self.p1(((value as i32) >> 16) as i64);
        self.p2(value);
    }

    pub fn p4(&mut self, value: i64) {
        self.data[self.pos..self.pos + 4].copy_from_slice(&(value as i32).to_be_bytes());
        self.pos += 4;
    }

    pub fn ip4(&mut self, value: i64) {
        self.data[self.pos..self.pos + 4].copy_from_slice(&(value as i32).to_le_bytes());
        self.pos += 4;
    }

    pub fn p8(&mut self, value: i64) {
        self.data[self.pos..self.pos + 8].copy_from_slice(&value.to_be_bytes());
        self.pos += 8;
    }

    pub fn pbool(&mut self, value: bool) {
        self.p1(value as i64);
    }

    /// `pjstr`: low byte of each UTF-16 code unit, then `\n`.
    pub fn pjstr(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.p1(u as i64);
        }
        self.p1(10);
    }

    /// `pdata(src, offset, length)`: copies `src[offset..offset+length]` but
    /// advances `pos` by `length - offset` (deliberate, kept).
    pub fn pdata(&mut self, src: &[u8], offset: usize, length: usize) {
        self.data[self.pos..self.pos + length].copy_from_slice(&src[offset..offset + length]);
        self.pos = (self.pos as isize + length as isize - offset as isize) as usize;
    }

    pub fn psize4(&mut self, size: usize) {
        let at = self.pos - size - 4;
        self.data[at..at + 4].copy_from_slice(&(size as u32).to_be_bytes());
    }

    pub fn psize2(&mut self, size: usize) {
        let at = self.pos - size - 2;
        self.data[at..at + 2].copy_from_slice(&(size as u16).to_be_bytes());
    }

    pub fn psize1(&mut self, size: usize) {
        let at = self.pos - size - 1;
        self.data[at] = size as u8;
    }

    /// `psmarts` (signed). Panics when out of range.
    pub fn psmarts(&mut self, value: i32) {
        if (-64..64).contains(&value) {
            self.p1(value as i64 + 64);
        } else if (-16384..16384).contains(&value) {
            self.p2(value as i64 + 0xc000);
        } else {
            panic!("Error psmarts out of range: {value}");
        }
    }

    /// `psmart` (unsigned). Panics when out of range.
    pub fn psmart(&mut self, value: i32) {
        if (0..128).contains(&value) {
            self.p1(value as i64);
        } else if (0..32768).contains(&value) {
            self.p2(value as i64 + 0x8000);
        } else {
            panic!("Error psmart out of range: {value}");
        }
    }

    pub fn p_var_int(&mut self, value: i64) {
        let v = value as u32;
        if v & 0xffff_ff80 != 0 {
            if v & 0xffff_c000 != 0 {
                if v & 0xffe0_0000 != 0 {
                    if v & 0xf000_0000 != 0 {
                        self.p1(((v >> 28) | 0x80) as i64);
                    }
                    self.p1(((v >> 21) | 0x80) as i64);
                }
                self.p1(((v >> 14) | 0x80) as i64);
            }
            self.p1(((v >> 7) | 0x80) as i64);
        }
        self.p1((v & 0x7f) as i64);
    }

    // ---------------------------------------------------------------- bits

    pub fn bit_start(&mut self) {
        self.bit_pos = self.pos << 3;
    }

    pub fn bit_end(&mut self) {
        self.pos = (self.bit_pos + 7) >> 3;
    }

    pub fn g_bit(&mut self, mut n: usize) -> i32 {
        let mut byte_pos = self.bit_pos >> 3;
        let mut remaining = 8 - (self.bit_pos & 7);
        let mut value: i32 = 0;
        self.bit_pos += n;

        while n > remaining {
            let b = self.data[byte_pos] as u32;
            byte_pos += 1;
            value = value.wrapping_add(((b & bitmask(remaining)) as i32).wrapping_shl((n - remaining) as u32));
            n -= remaining;
            remaining = 8;
        }

        let b = self.data[byte_pos] as u32;
        if n == remaining {
            value.wrapping_add((b & bitmask(remaining)) as i32)
        } else {
            value.wrapping_add(((b >> (remaining - n)) & bitmask(n)) as i32)
        }
    }

    pub fn p_bit(&mut self, mut n: usize, value: i64) {
        let value = value as u32;
        let pos = self.bit_pos;
        self.bit_pos += n;
        let mut byte_pos = pos >> 3;
        let mut remaining = 8 - (pos & 7);

        while n > remaining {
            let shift = bitmask(remaining);
            let byte = self.data.get(byte_pos).copied().unwrap_or(0) as u32;
            let v = (byte & !shift) | ((value >> (n - remaining)) & shift);
            if let Some(slot) = self.data.get_mut(byte_pos) {
                *slot = v as u8;
            }
            byte_pos += 1;
            n -= remaining;
            remaining = 8;
        }

        let r = remaining - n;
        let shift = bitmask(n);
        let byte = self.data.get(byte_pos).copied().unwrap_or(0) as u32;
        let v = (byte & (!shift << r)) | ((value & shift) << r);
        if let Some(slot) = self.data.get_mut(byte_pos) {
            *slot = v as u8;
        }
    }

    // ---------------------------------------------------------------- rsa

    /// `rsaenc(pem)`: encrypts `data[0..pos]` with the public exponent and
    /// replaces the buffer contents with `p1(len) + ciphertext`.
    pub fn rsaenc(&mut self, key: &RsaKey) {
        let length = self.pos;
        let raw = BigInt::from_signed_bytes_be(&self.data[..length]);
        let enc = mod_floor(&raw, &key.n).modpow(&key.e, &key.n).to_signed_bytes_be();
        self.pos = 0;
        self.p1(enc.len() as i64);
        self.pdata(&enc, 0, enc.len());
    }

    /// `rsadec(pem)`: reads `g1` length + ciphertext and decrypts it with the
    /// CRT parameters into the start of the buffer, leaving `pos = 0`. The
    /// ciphertext is a *signed* two's complement integer (jsbn semantics, B.19).
    pub fn rsadec(&mut self, key: &RsaKey) {
        let len = self.g1() as usize;
        let mut enc = vec![0; len];
        self.gdata(&mut enc, 0, len);
        let cipher = BigInt::from_signed_bytes_be(&enc);

        let m_p = mod_floor(&cipher, &key.p).modpow(&key.dp, &key.p);
        let m_q = mod_floor(&cipher, &key.q).modpow(&key.dq, &key.q);
        let h = mod_floor(&(&key.qinv * (&m_p - &m_q)), &key.p);
        let plain = (m_q + h * &key.q).to_signed_bytes_be();

        self.pos = 0;
        self.pdata(&plain, 0, plain.len());
        self.pos = 0;
    }
}

/// jsbn `mod`: always non-negative for a positive modulus.
fn mod_floor(x: &BigInt, m: &BigInt) -> BigInt {
    let r = x % m;
    if r.sign() == num_bigint::Sign::Minus { r + m } else { r }
}

/// The node-forge private-key fields `rsadec`/`rsaenc` use.
#[derive(Clone)]
pub struct RsaKey {
    pub n: BigInt,
    pub e: BigInt,
    pub p: BigInt,
    pub q: BigInt,
    pub dp: BigInt,
    pub dq: BigInt,
    pub qinv: BigInt,
}

impl RsaKey {
    /// Parses a PKCS#8 (`BEGIN PRIVATE KEY`) or PKCS#1 (`BEGIN RSA PRIVATE KEY`)
    /// PEM and takes the stored fields as-is, like `forge.pki.privateKeyFromPem`.
    /// No consistency validation: the stock `data/config/private.pem` is a key
    /// that `rsa::RsaPrivateKey` rejects as malformed but node-forge accepts.
    pub fn from_pem(pem: &str) -> Result<Self, String> {
        use rsa::pkcs1::der::Decode;
        let (label, doc) = rsa::pkcs8::SecretDocument::from_pem(pem.trim()).map_err(|e| e.to_string())?;
        let pkcs1 = match label {
            "PRIVATE KEY" => rsa::pkcs8::PrivateKeyInfo::from_der(doc.as_bytes()).map_err(|e| e.to_string())?.private_key,
            "RSA PRIVATE KEY" => doc.as_bytes(),
            other => return Err(format!("unexpected PEM label {other}")),
        };
        let k = rsa::pkcs1::RsaPrivateKey::from_der(pkcs1).map_err(|e| e.to_string())?;
        let big = |u: rsa::pkcs1::UintRef<'_>| BigInt::from_bytes_be(num_bigint::Sign::Plus, u.as_bytes());
        Ok(RsaKey {
            n: big(k.modulus),
            e: big(k.public_exponent),
            p: big(k.prime1),
            q: big(k.prime2),
            dp: big(k.exponent1),
            dq: big(k.exponent2),
            qinv: big(k.coefficient),
        })
    }
}

impl RsaKey {
    /// The modulus and exponent of a `BEGIN PUBLIC KEY` (SPKI) PEM, for
    /// `rsaenc` only (the CRT fields are zero). Parsed field by field like
    /// `from_pem`, since `rsa::RsaPublicKey` rejects the dev key's exponent.
    pub fn from_public_pem(pem: &str) -> Result<Self, String> {
        use rsa::pkcs1::der::Decode;
        let (label, doc) = rsa::pkcs8::Document::from_pem(pem.trim()).map_err(|e| e.to_string())?;
        if label != "PUBLIC KEY" {
            return Err(format!("unexpected PEM label {label}"));
        }
        let spki = rsa::pkcs8::spki::SubjectPublicKeyInfoRef::from_der(doc.as_bytes()).map_err(|e| e.to_string())?;
        let k = rsa::pkcs1::RsaPublicKey::from_der(spki.subject_public_key.raw_bytes()).map_err(|e| e.to_string())?;
        let big = |u: rsa::pkcs1::UintRef<'_>| BigInt::from_bytes_be(num_bigint::Sign::Plus, u.as_bytes());
        let zero = BigInt::default();
        Ok(RsaKey { n: big(k.modulus), e: big(k.public_exponent), p: zero.clone(), q: zero.clone(), dp: zero.clone(), dq: zero.clone(), qinv: zero })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_standard_crc32() {
        // `~crc` of the IEEE CRC-32 as a signed int.
        assert_eq!(Packet::getcrc(b"123456789", 0, 9), 0xcbf43926u32 as i32);
        // loop bound is `length`, not `offset + length`
        assert_eq!(Packet::getcrc(b"xx123456789", 2, 11), 0xcbf43926u32 as i32);
        assert_eq!(Packet::getcrc(b"abc", 2, 1), 0);
    }

    #[test]
    fn pos_bumps_before_out_of_range_byte_write() {
        let mut p = Packet::with_len(2);
        p.pos = 2;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.p1(1)));
        assert!(r.is_err());
        assert_eq!(p.pos, 3);
    }
}
