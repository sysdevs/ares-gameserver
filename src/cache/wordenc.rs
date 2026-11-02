//! The chat filter, loaded from the
//! `data/raw/wordenc` jag (not `data/pack`).
//!
//! After `format` every character is ASCII, `£` or `€`, so lowercasing keeps
//! the length and a string is a `Vec<char>`. Neighbour reads go through
//! [`at`], which is `None` outside the string: an out-of-range neighbour
//! counts as a symbol and equals no character.
//!
//! `WordEncFragments.filter` never masks anything (its `startIndex` only
//! reaches 2 and masking needs 4), so it is omitted; `isBadFragment` is used.

use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;

type Chars = Vec<char>;

/// The character at `i`, `None` outside the string.
fn at(chars: &[char], i: isize) -> Option<char> {
    usize::try_from(i).ok().and_then(|i| chars.get(i)).copied()
}

fn is(c: Option<char>, x: char) -> bool {
    c == Some(x)
}

fn is_lower(c: Option<char>) -> bool {
    matches!(c, Some('a'..='z'))
}

fn is_upper(c: Option<char>) -> bool {
    matches!(c, Some('A'..='Z'))
}

fn is_alpha(c: Option<char>) -> bool {
    is_lower(c) || is_upper(c)
}

fn is_num(c: Option<char>) -> bool {
    matches!(c, Some('0'..='9'))
}

fn is_symbol(c: Option<char>) -> bool {
    !is_alpha(c) && !is_num(c)
}

fn is_not_lowercase_alpha(c: Option<char>) -> bool {
    if is_lower(c) { matches!(c, Some('v' | 'x' | 'j' | 'q' | 'z')) } else { true }
}

fn mask_chars(offset: usize, length: usize, chars: &mut Chars) {
    for i in offset..length {
        chars[i] = '*';
    }
}

fn masked_count_backwards(chars: &[char], offset: isize) -> usize {
    let mut count = 0;
    let mut i = offset - 1;
    while i >= 0 && is_symbol(at(chars, i)) {
        if is(at(chars, i), '*') {
            count += 1;
        }
        i -= 1;
    }
    count
}

fn masked_count_forwards(chars: &[char], offset: isize) -> usize {
    let mut count = 0;
    let mut i = offset + 1;
    while (i as usize) < chars.len() && is_symbol(at(chars, i)) {
        if is(at(chars, i), '*') {
            count += 1;
        }
        i += 1;
    }
    count
}

fn masked_chars_status(chars: &[char], filtered: &[char], offset: isize, length: usize, prefix: bool) -> i32 {
    let count = if prefix { masked_count_backwards(filtered, offset) } else { masked_count_forwards(filtered, offset) };
    if count >= length {
        4
    } else if is_symbol(at(chars, if prefix { offset - 1 } else { offset + 1 })) {
        1
    } else {
        0
    }
}

fn prefix_symbol_status(offset: isize, chars: &[char], length: usize, symbol_chars: &[char], symbols: &[char]) -> i32 {
    if offset == 0 {
        return 2;
    }
    let mut i = offset - 1;
    while i >= 0 && is_symbol(at(chars, i)) {
        if at(chars, i).is_some_and(|c| symbols.contains(&c)) {
            return 3;
        }
        i -= 1;
    }
    masked_chars_status(chars, symbol_chars, offset, length, true)
}

fn suffix_symbol_status(offset: isize, chars: &[char], length: usize, symbol_chars: &[char], symbols: &[char]) -> i32 {
    if offset + 1 == chars.len() as isize {
        return 2;
    }
    let mut i = offset + 1;
    while (i as usize) < chars.len() && is_symbol(at(chars, i)) {
        if at(chars, i).is_some_and(|c| symbols.contains(&c)) {
            return 3;
        }
        i += 1;
    }
    masked_chars_status(chars, symbol_chars, offset, length, false)
}

/// `nextChar`: `'\0'` past the end.
fn next_of(chars: &[char], i: usize) -> Option<char> {
    Some(chars.get(i + 1).copied().unwrap_or('\0'))
}

#[derive(Debug, Default)]
pub struct WordEnc {
    fragments: Vec<i32>,
    bads: Vec<Vec<char>>,
    bad_combinations: Vec<Option<Vec<[i32; 2]>>>,
    domains: Vec<Vec<char>>,
    tlds: Vec<Vec<char>>,
    tld_types: Vec<i32>,
}

const WHITELIST: [&str; 5] = ["cook", "cook's", "cooks", "seeks", "sheet"];
const PERIOD: [char; 3] = ['d', 'o', 't'];
const AMPERSAT: [char; 3] = ['(', 'a', ')'];
const SLASH: [char; 5] = ['s', 'l', 'a', 's', 'h'];

fn chars_g1(p: &mut Packet) -> Vec<char> {
    let n = p.g1();
    (0..n).map(|_| char::from(p.g1() as u8)).collect()
}

impl WordEnc {
    /// `WordEnc.load()`: `data/raw/wordenc`; an entry missing leaves the filter empty.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let jag = Jagfile::load(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        Ok(Self::read_all(&jag))
    }

    pub fn read_all(jag: &Jagfile) -> Self {
        let mut w = WordEnc::default();
        let (Some(mut fragments), Some(mut bad), Some(mut domain), Some(mut tld)) =
            (jag.read("fragmentsenc.txt"), jag.read("badenc.txt"), jag.read("domainenc.txt"), jag.read("tldlist.txt"))
        else {
            return w;
        };
        for _ in 0..bad.g4s() {
            w.bads.push(chars_g1(&mut bad));
            let n = bad.g1();
            let combos: Vec<[i32; 2]> = (0..n).map(|_| [bad.g1b(), bad.g1b()]).collect();
            w.bad_combinations.push(if combos.is_empty() { None } else { Some(combos) });
        }
        for _ in 0..domain.g4s() {
            w.domains.push(chars_g1(&mut domain));
        }
        for _ in 0..fragments.g4s() {
            w.fragments.push(fragments.g2());
        }
        for _ in 0..tld.g4s() {
            w.tld_types.push(tld.g1());
            w.tlds.push(chars_g1(&mut tld));
        }
        w
    }

    /// `WordEnc.filter(input)`.
    pub fn filter(&self, input: &str) -> String {
        let mut characters: Vec<String> = input.chars().map(String::from).collect();
        format(&mut characters);
        let joined: String = characters.concat();
        let trimmed = joined.trim();
        let lowercase = trimmed.to_lowercase();
        let mut filtered: Chars = lowercase.chars().collect();
        self.filter_tlds(&mut filtered);
        self.filter_bads(&mut filtered);
        self.filter_domains(&mut filtered);
        let lower16: Vec<u16> = lowercase.encode_utf16().collect();
        for word in WHITELIST {
            let w16: Vec<u16> = word.encode_utf16().collect();
            let mut from = 0;
            while let Some(found) = lower16.get(from..).and_then(|s| s.windows(w16.len()).position(|x| x == w16)) {
                let offset = from + found;
                for (i, c) in word.chars().enumerate() {
                    filtered[offset + i] = c;
                }
                from = offset + 1;
            }
        }
        let trimmed_chars: Vec<char> = trimmed.chars().collect();
        for (i, &c) in trimmed_chars.iter().enumerate() {
            if !is(at(&filtered, i as isize), '*') && is_upper(Some(c)) {
                filtered[i] = c;
            }
        }
        format_uppercases(&mut filtered);
        let out: String = filtered.iter().collect();
        out.trim().to_string()
    }

    fn filter_bads(&self, chars: &mut Chars) {
        for _ in 0..2 {
            for i in (0..self.bads.len()).rev() {
                self.filter_bad_combinations(self.bad_combinations[i].as_deref(), chars, &self.bads[i]);
            }
        }
    }

    fn filter_bad_combinations(&self, combos: Option<&[[i32; 2]]>, chars: &mut Chars, bads: &[char]) {
        if bads.len() > chars.len() {
            return;
        }
        let mut start = 0usize;
        while start as isize <= chars.len() as isize - bads.len() as isize {
            let (current, bad_index, has_symbol, has_number, has_digit) = self.process_bad_characters(chars, bads, start);
            let current = current as isize;
            let s = start as isize;
            if !(bad_index >= bads.len() && (!has_number || !has_digit)) {
                start += 1;
                continue;
            }
            let mut should_filter = true;
            if has_symbol {
                let before = s - 1 < 0 || (is_symbol(at(chars, s - 1)) && !is(at(chars, s - 1), '\''));
                let after = current >= chars.len() as isize || (is_symbol(at(chars, current)) && !is(at(chars, current), '\''));
                if !before || !after {
                    let mut valid = false;
                    let mut local = if before { s } else { s - 2 };
                    let word_char = |c: Option<char>| !is_symbol(c) || is(c, '\'');
                    while !valid && local < current {
                        if local >= 0 && word_char(at(chars, local)) {
                            let mut sub: Vec<char> = Vec::new();
                            let mut n = 0;
                            while n < 3 && ((local + n) as usize) < chars.len() && word_char(at(chars, local + n)) {
                                sub.push(chars[(local + n) as usize]);
                                n += 1;
                            }
                            let mut ok = n != 0;
                            if n < 3 && local - 1 >= 0 && word_char(at(chars, local - 1)) {
                                ok = false;
                            }
                            if ok && !self.is_bad_fragment(&sub) {
                                valid = true;
                            }
                        }
                        local += 1;
                    }
                    if !valid {
                        should_filter = false;
                    }
                }
            } else {
                let cur = if s - 1 >= 0 { at(chars, s - 1) } else { Some(' ') };
                let next = if current < chars.len() as isize { at(chars, current) } else { Some(' ') };
                if let Some(combos) = combos
                    && combo_matches(get_index(cur), combos, get_index(next))
                {
                    should_filter = false;
                }
            }
            if should_filter {
                let (mut numerals, mut alphas) = (0, 0);
                for i in s..current {
                    if is_num(at(chars, i)) {
                        numerals += 1;
                    } else if is_alpha(at(chars, i)) {
                        alphas += 1;
                    }
                }
                if numerals <= alphas {
                    mask_chars(start, current as usize, chars);
                }
            }
            start += 1;
        }
    }

    /// `processBadCharacters`: `(currentIndex, badIndex, hasSymbol, hasNumber, hasDigit)`.
    fn process_bad_characters(&self, chars: &[char], bads: &[char], start: usize) -> (usize, usize, bool, bool, bool) {
        let mut index = start;
        let mut bad_index = 0;
        let mut count = 0;
        let (mut has_symbol, mut has_number, mut has_digit) = (false, false, false);
        while index < chars.len() && !(has_number && has_digit) {
            let cur = Some(chars[index]);
            let next = next_of(chars, index);
            let len = if bad_index < bads.len() { bad_char_len(next, bads[bad_index], cur) } else { 0 };
            if len > 0 {
                if len == 1 && is_num(cur) {
                    has_number = true;
                }
                if len == 2 && (is_num(cur) || is_num(next)) {
                    has_number = true;
                }
                index += len;
                bad_index += 1;
            } else {
                if bad_index == 0 {
                    break;
                }
                let prev = bad_char_len(next, bads[bad_index - 1], cur);
                if prev > 0 {
                    index += prev;
                } else {
                    if bad_index >= bads.len() || !is_not_lowercase_alpha(cur) {
                        break;
                    }
                    if is_symbol(cur) && !is(cur, '\'') {
                        has_symbol = true;
                    }
                    if is_num(cur) {
                        has_digit = true;
                    }
                    index += 1;
                    count += 1;
                    if (count * 100) / (index - start) > 90 {
                        break;
                    }
                }
            }
        }
        (index, bad_index, has_symbol, has_number, has_digit)
    }

    /// `WordEncFragments.isBadFragment`.
    pub fn is_bad_fragment(&self, chars: &[char]) -> bool {
        if chars.iter().all(|&c| c.is_ascii_digit() || c == '\0') {
            return true;
        }
        let value = get_integer(chars);
        let f = &self.fragments;
        if f.first() == Some(&value) || f.last() == Some(&value) {
            return true;
        }
        ts_bsearch(f, value)
    }

    fn filter_domains(&self, chars: &mut Chars) {
        let mut ampersat = chars.clone();
        let mut period = chars.clone();
        self.filter_bad_combinations(None, &mut ampersat, &AMPERSAT);
        self.filter_bad_combinations(None, &mut period, &PERIOD);
        for domain in self.domains.iter().rev() {
            let chars_len = chars.len() as isize;
            let mut index = 0isize;
            while index <= chars_len - domain.len() as isize {
                if let Some(current) = match_domain(index as usize, domain, chars) {
                    let a = prefix_symbol_status(index, chars, 3, &ampersat, &['@']);
                    let p = suffix_symbol_status(current as isize - 1, chars, 3, &period, &['.', ',']);
                    if a > 2 || p > 2 {
                        mask_chars(index as usize, current, chars);
                    }
                }
                index += 1;
            }
        }
    }

    fn filter_tlds(&self, chars: &mut Chars) {
        let mut period = chars.clone();
        let mut slash = chars.clone();
        self.filter_bad_combinations(None, &mut period, &PERIOD);
        self.filter_bad_combinations(None, &mut slash, &SLASH);
        for (tld, &kind) in self.tlds.iter().zip(&self.tld_types) {
            self.filter_tld(&slash, kind, chars, tld, &period);
        }
    }

    fn filter_tld(&self, slash: &[char], kind: i32, chars: &mut Chars, tld: &[char], period: &[char]) {
        if tld.len() > chars.len() {
            return;
        }
        let mut index = 0isize;
        while index <= chars.len() as isize - tld.len() as isize {
            let (current, tld_index) = process_tld(chars, tld, index as usize);
            if tld_index < tld.len() {
                index += 1;
                continue;
            }
            let p = prefix_symbol_status(index, chars, 3, period, &[',', '.']);
            let s = suffix_symbol_status(current as isize - 1, chars, 5, slash, &['\\', '/']);
            let should = match kind {
                1 => p > 0 && s > 0,
                2 => (p > 2 && s > 0) || (p > 0 && s > 2),
                3 => p > 0 && s > 2,
                _ => false,
            };
            if !should {
                index += 1;
                continue;
            }
            let mut start = index;
            let mut end = current as isize - 1;
            if p > 2 {
                if p == 4 {
                    let mut found = false;
                    let mut i = index - 1;
                    while i >= 0 {
                        if found {
                            if !is(at(period, i), '*') {
                                break;
                            }
                            start = i;
                        } else if is(at(period, i), '*') {
                            start = i;
                            found = true;
                        }
                        i -= 1;
                    }
                }
                let mut found = false;
                let mut i = start - 1;
                while i >= 0 {
                    if found {
                        if is_symbol(at(chars, i)) {
                            break;
                        }
                        start = i;
                    } else if !is_symbol(at(chars, i)) {
                        found = true;
                        start = i;
                    }
                    i -= 1;
                }
            }
            if s > 2 {
                if s == 4 {
                    let mut found = false;
                    let mut i = end + 1;
                    while i < chars.len() as isize {
                        if found {
                            if !is(at(slash, i), '*') {
                                break;
                            }
                            end = i;
                        } else if is(at(slash, i), '*') {
                            end = i;
                            found = true;
                        }
                        i += 1;
                    }
                }
                let mut found = false;
                let mut i = end + 1;
                while i < chars.len() as isize {
                    if found {
                        if is_symbol(at(chars, i)) {
                            break;
                        }
                        end = i;
                    } else if !is_symbol(at(chars, i)) {
                        found = true;
                        end = i;
                    }
                    i += 1;
                }
            }
            mask_chars(start as usize, (end + 1) as usize, chars);
            index += 1;
        }
    }
}

/// Binary search over `fragments`, kept as written in case the list
/// is ever unsorted.
fn ts_bsearch(f: &[i32], value: i32) -> bool {
    let (mut start, mut end) = (0isize, f.len() as isize - 1);
    while start <= end {
        let mid = (start + end) / 2;
        let m = f[mid as usize];
        if value == m {
            return true;
        } else if value < m {
            end = mid - 1;
        } else {
            start = mid + 1;
        }
    }
    false
}

/// `WordEncFragments.getInteger`: base 38, read right to left.
fn get_integer(chars: &[char]) -> i32 {
    if chars.len() > 6 {
        return 0;
    }
    let mut value = 0i32;
    for &c in chars.iter().rev() {
        match c {
            ch @ 'a'..='z' => value = value * 38 + ch as i32 + 1 - 'a' as i32,
            '\'' => value = value * 38 + 27,
            ch @ '0'..='9' => value = value * 38 + ch as i32 + 28 - '0' as i32,
            '\0' => {}
            _ => return 0,
        }
    }
    value
}

fn get_index(c: Option<char>) -> i32 {
    match c {
        Some(ch @ 'a'..='z') => ch as i32 + 1 - 'a' as i32,
        Some('\'') => 28,
        Some(ch @ '0'..='9') => ch as i32 + 29 - '0' as i32,
        _ => 27,
    }
}

fn combo_matches(current: i32, combos: &[[i32; 2]], next: i32) -> bool {
    let (mut start, mut end) = (0isize, combos.len() as isize - 1);
    while start <= end {
        let mid = (start + end) / 2;
        let [a, b] = combos[mid as usize];
        if a == current && b == next {
            return true;
        } else if current < a || (current == a && next < b) {
            end = mid - 1;
        } else {
            start = mid + 1;
        }
    }
    false
}

/// `getEmulatedBadCharLen(nextChar, badChar, currentChar)`.
fn bad_char_len(next: Option<char>, bad: char, cur: Option<char>) -> usize {
    if Some(bad) == cur {
        return 1;
    }
    let c = |x: char| cur == Some(x);
    let n = |x: char| next == Some(x);
    let pair = |a: char, b: char| c(a) && n(b);
    match bad {
        'a' => {
            if c('4') || c('@') || c('^') {
                1
            } else if pair('/', '\\') {
                2
            } else {
                0
            }
        }
        'b' => {
            if c('6') || c('8') {
                1
            } else if pair('1', '3') {
                2
            } else {
                0
            }
        }
        'c' => (c('(') || c('<') || c('{') || c('[')) as usize,
        'd' => 2 * pair('[', ')') as usize,
        'e' => (c('3') || c('\u{20ac}')) as usize,
        'f' => {
            if pair('p', 'h') {
                2
            } else {
                c('\u{a3}') as usize
            }
        }
        'g' => (c('9') || c('6')) as usize,
        'h' => c('#') as usize,
        'i' => (c('y') || c('l') || c('j') || c('1') || c('!') || c(':') || c(';') || c('|')) as usize,
        'l' => (c('1') || c('|') || c('i')) as usize,
        'o' => {
            if c('0') || c('*') {
                1
            } else if pair('(', ')') || pair('[', ']') || pair('{', '}') || pair('<', '>') {
                2
            } else {
                0
            }
        }
        's' => (c('5') || c('z') || c('$') || c('2')) as usize,
        't' => (c('7') || c('+')) as usize,
        'u' => {
            if c('v') {
                1
            } else if pair('\\', '/') || pair('\\', '|') || pair('|', '/') {
                2
            } else {
                0
            }
        }
        'v' => 2 * (pair('\\', '/') || pair('\\', '|') || pair('|', '/')) as usize,
        'w' => 2 * pair('v', 'v') as usize,
        'x' => 2 * (pair(')', '(') || pair('}', '{') || pair(']', '[') || pair('>', '<')) as usize,
        '0' => {
            if c('o') || c('O') {
                1
            } else if pair('(', ')') || pair('{', '}') || pair('[', ']') {
                2
            } else {
                0
            }
        }
        '1' => c('l') as usize,
        ',' => c('.') as usize,
        '.' => c(',') as usize,
        '!' => c('i') as usize,
        _ => 0,
    }
}

/// `getEmulatedDomainCharLen(nextChar, domainChar, currentChar)`.
fn domain_char_len(next: Option<char>, d: char, cur: Option<char>) -> usize {
    let c = |x: char| cur == Some(x);
    if Some(d) == cur {
        1
    } else if d == 'o' && c('0') {
        1
    } else if d == 'o' && c('(') && next == Some(')') {
        2
    } else if d == 'c' && (c('(') || c('<') || c('[')) {
        1
    } else if d == 'e' && c('\u{20ac}') {
        1
    } else if d == 's' && c('$') {
        1
    } else if d == 'l' && c('i') {
        1
    } else {
        0
    }
}

/// `findMatchingDomain`: `Some(currentIndex)` when the whole domain matched.
fn match_domain(start: usize, domain: &[char], chars: &[char]) -> Option<usize> {
    let mut index = start;
    let mut di = 0;
    while index < chars.len() && di < domain.len() {
        let cur = Some(chars[index]);
        let next = next_of(chars, index);
        let len = domain_char_len(next, domain[di], cur);
        if len > 0 {
            index += len;
            di += 1;
        } else {
            if di == 0 {
                break;
            }
            let prev = domain_char_len(next, domain[di - 1], cur);
            if prev > 0 {
                index += prev;
            } else {
                if di >= domain.len() || !is_symbol(cur) {
                    break;
                }
                index += 1;
            }
        }
    }
    (di >= domain.len()).then_some(index)
}

/// `processTlds`: `(currentIndex, tldIndex)`.
fn process_tld(chars: &[char], tld: &[char], mut index: usize) -> (usize, usize) {
    let mut ti = 0;
    while index < chars.len() && ti < tld.len() {
        let cur = Some(chars[index]);
        let next = next_of(chars, index);
        let len = domain_char_len(next, tld[ti], cur);
        if len > 0 {
            index += len;
            ti += 1;
        } else {
            if ti == 0 {
                break;
            }
            let prev = domain_char_len(next, tld[ti - 1], cur);
            if prev > 0 {
                index += prev;
            } else {
                if !is_symbol(cur) {
                    break;
                }
                index += 1;
            }
        }
    }
    (index, ti)
}

/// `WordEnc.format`: disallowed characters become spaces, runs of spaces
/// collapse, and the freed tail is filled with spaces.
fn format(chars: &mut [String]) {
    let mut pos = 0;
    for i in 0..chars.len() {
        let c = if is_character_allowed(&chars[i]) { chars[i].clone() } else { " ".to_string() };
        chars[pos] = c;
        if pos == 0 || chars[pos] != " " || chars[pos - 1] != " " {
            pos += 1;
        }
    }
    for c in chars.iter_mut().skip(pos) {
        *c = " ".to_string();
    }
}

/// `isCharacterAllowed`: a string comparison, so an astral character (whose
/// first UTF-16 unit is a surrogate) is never allowed.
fn is_character_allowed(s: &str) -> bool {
    let first = s.encode_utf16().next().unwrap_or(0);
    (0x20..=0x7f).contains(&first) && s.encode_utf16().count() == 1 || s == "\n" || s == "\t" || s == "\u{a3}" || s == "\u{20ac}"
}

fn format_uppercases(chars: &mut Chars) {
    let mut flagged = true;
    for i in 0..chars.len() {
        let c = Some(chars[i]);
        if !is_alpha(c) {
            flagged = true;
        } else if flagged {
            if is_lower(c) {
                flagged = false;
            }
        } else if let Some(u @ 'A'..='Z') = c {
            chars[i] = u.to_ascii_lowercase();
        }
    }
}
