//! Base-37 player names.

/// `toBase37(name)`: trimmed, at most 12 UTF-16 units; letters 1-26, digits
/// 27-36, anything else contributes 0; trailing zero digits are stripped.
pub fn to_base37(s: &str) -> u64 {
    let mut l: u64 = 0;
    for c in s.trim().encode_utf16().take(12) {
        l *= 37;
        l += match c {
            0x41..=0x5a => (c + 1 - 0x41) as u64,
            0x61..=0x7a => (c + 1 - 0x61) as u64,
            0x30..=0x39 => (c + 27 - 0x30) as u64,
            _ => 0,
        };
    }
    while l % 37 == 0 && l != 0 {
        l /= 37;
    }
    l
}

const LOOKUP: &[u8; 37] = b"_abcdefghijklmnopqrstuvwxyz0123456789";

/// `fromBase37(value)`: `invalid_name` past 37^12 or for a trailing zero digit.
pub fn from_base37(mut value: i64) -> String {
    if !(0..6582952005840035281).contains(&value) || value % 37 == 0 {
        return "invalid_name".into();
    }
    let mut chars = Vec::new();
    while value != 0 {
        chars.push(LOOKUP[(value % 37) as usize] as char);
        value /= 37;
    }
    chars.iter().rev().collect()
}

/// `toTitleCase(str)`: every `/\w\S*/` run gets an upper-case first character
/// and a lower-cased rest.
pub fn to_title_case(s: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if is_word(c) {
            out.extend(c.to_uppercase());
            while let Some(&n) = chars.peek() {
                if n.is_whitespace() {
                    break;
                }
                out.extend(n.to_lowercase());
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn to_safe_name(name: &str) -> String {
    from_base37(to_base37(name) as i64)
}

pub fn to_display_name(name: &str) -> String {
    to_title_case(&to_safe_name(name).replace('_', " "))
}
