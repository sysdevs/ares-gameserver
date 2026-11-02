//! Nibble-packed chat text.

use crate::io::packet::Packet;

/// `CHAR_LOOKUP`; index 37 is a second space that `pack` never finds.
const CHAR_LOOKUP: [char; 61] = [
    ' ', 'e', 't', 'a', 'o', 'i', 'h', 'n', 's', 'r', 'd', 'l', 'u', 'm', 'w', 'c', 'y', 'f', 'g', 'p', 'b', 'v', 'k', 'x', 'j', 'q', 'z', '0', '1', '2', '3',
    '4', '5', '6', '7', '8', '9', ' ', '!', '?', '.', ',', ':', ';', '(', ')', '-', '&', '*', '\\', '\'', '@', '#', '+', '=', '\u{a3}', '$', '%', '"', '[', ']',
];

/// `unpack(packet, length)`: stops once 80 characters are out (a byte's
/// second nibble can make it 81), then sentence case.
/// Two-nibble codes only reach indices 13..=60, so every lookup is defined.
pub fn unpack(packet: &mut Packet, length: usize) -> String {
    let mut out = String::new();
    let mut pos = 0;
    let mut carry: i32 = -1;
    let mut i = 0;
    while i < length && pos < 80 {
        let data = packet.g1();
        for nibble in [(data >> 4) & 0xf, data & 0xf] {
            if carry != -1 {
                out.push(CHAR_LOOKUP[((carry << 4) + nibble - 195) as usize]);
                pos += 1;
                carry = -1;
            } else if nibble < 13 {
                out.push(CHAR_LOOKUP[nibble as usize]);
                pos += 1;
            } else {
                carry = nibble;
            }
        }
        i += 1;
    }
    to_sentence_case(&out)
}

/// `pack(packet, input)`: the first 80 UTF-16 units, lower-cased; unknown
/// characters become spaces (index 0).
pub fn pack(packet: &mut Packet, input: &str) {
    let units: Vec<u16> = input.encode_utf16().take(80).collect();
    let lower: Vec<u16> = String::from_utf16_lossy(&units).to_lowercase().encode_utf16().collect();
    let mut carry: i64 = -1;
    for u in lower {
        let mut index = CHAR_LOOKUP.iter().position(|&c| c as u32 == u as u32).unwrap_or(0) as i64;
        if index > 12 {
            index += 195;
        }
        if carry == -1 {
            if index < 13 {
                carry = index;
            } else {
                packet.p1(index);
            }
        } else if index < 13 {
            packet.p1((carry << 4) + index);
            carry = -1;
        } else {
            packet.p1((carry << 4) + (index >> 4));
            carry = index & 0xf;
        }
    }
    if carry != -1 {
        packet.p1(carry << 4);
    }
}

/// `toSentenceCase`: lower-case, then capitalise the first letter of the
/// text and after every `.` or `!`.
pub fn to_sentence_case(input: &str) -> String {
    let mut punctuation = true;
    input
        .to_lowercase()
        .chars()
        .map(|c| {
            let out = if punctuation && c.is_ascii_lowercase() {
                punctuation = false;
                c.to_ascii_uppercase()
            } else {
                c
            };
            if c == '.' || c == '!' {
                punctuation = true;
            }
            out
        })
        .collect()
}
