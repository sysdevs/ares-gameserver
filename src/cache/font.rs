//! Glyph metrics from the `title` jag, used by
//! scripts for `stringWidth` and line splitting.
//!
//! Strings are handled as UTF-16 units. A unit above 255 has no glyph, so the
//! string has no width (`None`) and fits no limit and exceeds none.

use crate::io::jagfile::Jagfile;
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct FontType {
    /// `None` for the glyphs never set (a missing `title` entry leaves all 256 holes).
    pub char_mask: Vec<Option<Vec<u8>>>,
    pub char_mask_width: Vec<u8>,
    pub char_mask_height: Vec<u8>,
    pub char_offset_x: Vec<u8>,
    pub char_offset_y: Vec<u8>,
    pub char_advance: Vec<u8>,
    pub height: i32,
}

/// `FontType.load(dir)`: p11, p12, b12, q8 (quill) from `client/title`.
pub fn load_fonts(dir: impl AsRef<Path>) -> Result<Vec<FontType>, String> {
    let title = Jagfile::load(dir.as_ref().join("client/title")).map_err(|e| format!("client/title: {e}"))?;
    Ok([("p11_full", false), ("p12_full", false), ("b12_full", false), ("q8_full", true)]
        .iter()
        .map(|&(name, quill)| FontType::new(&title, name, quill))
        .collect())
}

const AT: u16 = b'@' as u16;
const BAR: u16 = b'|' as u16;
const SPACE: u16 = b' ' as u16;

fn index_of(s: &[u16], pat: &[u16]) -> Option<usize> {
    s.windows(pat.len()).position(|w| w == pat)
}

/// JS `substring(a, b)` with `b` clamped to the length.
fn substring(s: &[u16], a: usize, b: usize) -> &[u16] {
    let b = b.min(s.len());
    let a = a.min(b);
    &s[a..b]
}

impl FontType {
    pub fn new(title: &Jagfile, font: &str, quill: bool) -> Self {
        let mut f = FontType {
            char_mask: vec![None; 256],
            char_mask_width: vec![0; 256],
            char_mask_height: vec![0; 256],
            char_offset_x: vec![0; 256],
            char_offset_y: vec![0; 256],
            char_advance: vec![0; 256],
            height: 0,
        };
        let (Some(mut data), Some(mut index)) = (title.read(&format!("{font}.dat")), title.read("index.dat")) else {
            return f;
        };
        index.pos = data.g2() as usize + 4;
        let pal_count = index.g1();
        if pal_count > 0 {
            index.pos += (pal_count as usize - 1) * 3;
        }
        for c in 0..256 {
            f.char_offset_x[c] = index.g1() as u8;
            f.char_offset_y[c] = index.g1() as u8;
            // the stores truncate to u8; `wi`/`hi` keep the full g2 value
            let wi = index.g2() as usize;
            let hi = index.g2() as usize;
            f.char_mask_width[c] = wi as u8;
            f.char_mask_height[c] = hi as u8;
            let pixel_order = index.g1();
            let mut mask = vec![0u8; wi * hi];
            if pixel_order == 0 {
                for px in mask.iter_mut() {
                    *px = data.g1() as u8;
                }
            } else if pixel_order == 1 {
                for x in 0..wi {
                    for y in 0..hi {
                        mask[x + y * wi] = data.g1() as u8;
                    }
                }
            }
            if hi as i32 > f.height && c < 128 {
                f.height = hi as i32;
            }
            f.char_offset_x[c] = 1;
            f.char_advance[c] = (wi + 2) as u8;
            // with wi == 0 and hi > 0 there is no column to read: not an edge
            let edge = |col: &dyn Fn(usize) -> usize| -> bool {
                if hi == 0 {
                    return true;
                }
                if wi == 0 {
                    return false;
                }
                let space: usize = (hi / 7..hi).map(|y| mask[col(y)] as usize).sum();
                space <= hi / 7
            };
            if edge(&|y| y * wi) {
                f.char_advance[c] = f.char_advance[c].wrapping_sub(1);
                f.char_offset_x[c] = 0;
            }
            if edge(&|y| wi + y * wi - 1) {
                f.char_advance[c] = f.char_advance[c].wrapping_sub(1);
            }
            f.char_mask[c] = Some(mask);
        }
        f.char_advance[32] = f.char_advance[if quill { 73 } else { 105 }];
        f
    }

    /// `stringWidth(str)`: `@xxx@` colour tags are skipped; `None` when a
    /// unit has no glyph.
    pub fn string_width(&self, s: &str) -> Option<i64> {
        self.width16(&s.encode_utf16().collect::<Vec<_>>())
    }

    fn width16(&self, s: &[u16]) -> Option<i64> {
        let mut size = 0;
        let mut c = 0;
        while c < s.len() {
            if s[c] == AT && c + 4 < s.len() && s[c + 4] == AT {
                c += 4;
            } else {
                size += *self.char_advance.get(s[c] as usize)? as i64;
            }
            c += 1;
        }
        Some(size)
    }

    /// `split(str, maxWidth)`: word-wraps on spaces and `|`, carrying the
    /// last colour tag onto the next line.
    pub fn split(&self, s: &str, max_width: i64) -> Vec<String> {
        let mut s: Vec<u16> = s.encode_utf16().collect();
        if s.is_empty() {
            return vec![String::new()];
        }
        let str16: Vec<u16> = "str".encode_utf16().collect();
        let tag_str: Vec<u16> = "@str@".encode_utf16().collect();
        let tag_bla: Vec<u16> = "@bla@".encode_utf16().collect();
        let mut lines = Vec::new();
        let mut saved_col: Option<Vec<u16>> = None;
        while !s.is_empty() {
            if self.width16(&s).is_some_and(|w| w <= max_width) && !s.contains(&BAR) {
                lines.push(String::from_utf16_lossy(&s));
                break;
            }
            let mut split_index = s.len();
            for i in 0..s.len() {
                if s[i] == SPACE {
                    if self.width16(&s[..i]).is_some_and(|w| w > max_width) {
                        break;
                    }
                    split_index = i;
                } else if s[i] == BAR {
                    split_index = i;
                    break;
                }
            }
            let line = s[..split_index].to_vec();
            lines.push(String::from_utf16_lossy(&line));

            if line.contains(&AT) {
                let mut i = 0;
                while i + 4 < line.len() {
                    if line[i] == AT && line[i + 4] == AT {
                        if line[i + 1..i + 4] == str16[..] {
                            saved_col = None;
                            if substring(&line, i + 5, i + 10) == &tag_bla[..] {
                                i += 10;
                                continue;
                            }
                        } else {
                            saved_col = Some(line[i..i + 5].to_vec());
                        }
                        i += 4;
                    }
                    i += 1;
                }
            }

            s = substring(&s, split_index + 1, s.len()).to_vec();

            if let Some(col) = &saved_col
                && !s.is_empty()
                && s[0] != BAR
            {
                match index_of(&s, &tag_str) {
                    Some(at) => {
                        if substring(&s, at + 5, at + 10) != &tag_bla[..] {
                            s.splice(at + 5..at + 5, tag_bla.iter().copied());
                        }
                        saved_col = None;
                    }
                    None => s = [col.as_slice(), &s].concat(),
                }
            }
        }
        lines
    }
}
