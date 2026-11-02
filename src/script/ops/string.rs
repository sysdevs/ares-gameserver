//! String ops. Lengths and indices are UTF-16 units like JS.

use super::super::runner::OpResult;
use super::super::state::ScriptState;
use ares_shared::opcode::op;
use super::valid;
use crate::engine::world::World;

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_units(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// JS `String.prototype.substring(start, end)`.
fn substring(s: &str, start: i32, end: i32) -> String {
    let u = units(s);
    let len = u.len() as i32;
    let (a, b) = (start.clamp(0, len), end.clamp(0, len));
    let (a, b) = if a > b { (b, a) } else { (a, b) };
    from_units(&u[a as usize..b as usize])
}

/// JS `indexOf` on UTF-16 units (`""` is found at 0).
fn index_of(hay: &str, needle: &str) -> i32 {
    let (h, n) = (units(hay), units(needle));
    if n.is_empty() {
        return 0;
    }
    h.windows(n.len()).position(|w| w == n.as_slice()).map_or(-1, |i| i as i32)
}

/// Java `String.compareTo` on UTF-16 units.
fn java_compare(a: &str, b: &str) -> i32 {
    let (a, b) = (units(a), units(b));
    for (x, y) in a.iter().zip(&b) {
        if x != y {
            return *x as i32 - *y as i32;
        }
    }
    a.len() as i32 - b.len() as i32
}

/// `String.fromCharCode(n)`: the low 16 bits as one UTF-16 unit.
fn from_char_code(n: i32) -> String {
    from_units(&[n as u16])
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    Some(match opcode {
        op::APPEND_NUM => {
            let text = s.pop_string();
            let num = s.pop_int();
            s.push_string(format!("{text}{num}"));
            Ok(())
        }
        op::APPEND => {
            let [a, b] = s.pop_strings::<2>();
            s.push_string(a + &b);
            Ok(())
        }
        op::APPEND_SIGNNUM => {
            let text = s.pop_string();
            let num = s.pop_int();
            s.push_string(if num >= 0 { format!("{text}+{num}") } else { format!("{text}{num}") });
            Ok(())
        }
        op::LOWERCASE => {
            let v = s.pop_string().to_lowercase();
            s.push_string(v);
            Ok(())
        }
        op::TOSTRING => {
            let v = s.pop_int().to_string();
            s.push_string(v);
            Ok(())
        }
        op::COMPARE => {
            let [a, b] = s.pop_strings::<2>();
            s.push_int(java_compare(&a, &b));
            Ok(())
        }
        op::TEXT_SWITCH => {
            let value = s.pop_int();
            let [a, b] = s.pop_strings::<2>();
            s.push_string(if value == 1 { a } else { b });
            Ok(())
        }
        op::APPEND_CHAR => {
            let text = s.pop_string();
            let c = s.pop_int();
            s.push_string(text + &from_char_code(c));
            Ok(())
        }
        op::STRING_LENGTH => {
            let v = s.pop_string();
            s.push_int(v.encode_utf16().count() as i32);
            Ok(())
        }
        op::SUBSTRING => {
            let text = s.pop_string();
            let [start, end] = s.pop_ints::<2>();
            s.push_string(substring(&text, start, end));
            Ok(())
        }
        op::STRING_INDEXOF_CHAR => {
            let text = s.pop_string();
            let find = from_char_code(s.pop_int());
            s.push_int(index_of(&text, &find));
            Ok(())
        }
        op::STRING_INDEXOF_STRING => {
            let text = s.pop_string();
            let find = s.pop_string();
            s.push_int(index_of(&text, &find));
            Ok(())
        }
        op::SPLIT_INIT => (|| {
            let [max_width, lines_per_page, font_id] = s.pop_ints::<3>();
            let mut text = s.pop_string();
            if font_id < 0 || font_id as usize >= w.configs.fonts.len() {
                return Err(format!("An input for a Font type was not valid to use. Input was {font_id}."));
            }
            // `<p,name>` sets the chat mesanim
            let close = index_of(&text, ">");
            if text.starts_with("<p,") && close != -1 {
                let name = substring(&text, 3, close);
                s.split_mesanim = w.configs.mesanims.get_id(&name);
                text = substring(&text, close + 1, i32::MAX);
            } else {
                s.split_mesanim = -1;
            }
            let mut lines = w.configs.fonts[font_id as usize].split(&text, max_width as i64);
            s.split_pages.clear();
            let per = lines_per_page.max(0) as usize;
            while !lines.is_empty() {
                // `splice(0, n)` with n <= 0 takes nothing, so this would loop forever
                if per == 0 {
                    return Err("split_init: zero lines per page".into());
                }
                let rest = lines.split_off(per.min(lines.len()));
                s.split_pages.push(std::mem::replace(&mut lines, rest));
            }
            Ok(())
        })(),
        op::SPLIT_GET => {
            let [page, line] = s.pop_ints::<2>();
            match s.split_pages.get(page as usize) {
                Some(p) => {
                    s.push_string(p.get(line as usize).cloned().unwrap_or_default());
                    Ok(())
                }
                None => Err(format!("no split page {page}")),
            }
        }
        op::SPLIT_PAGECOUNT => {
            s.push_int(s.split_pages.len() as i32);
            Ok(())
        }
        op::SPLIT_LINECOUNT => {
            let page = s.pop_int();
            match s.split_pages.get(page as usize) {
                Some(p) => {
                    s.push_int(p.len() as i32);
                    Ok(())
                }
                None => Err(format!("no split page {page}")),
            }
        }
        op::SPLIT_GETANIM => (|| {
            let page = s.pop_int();
            if s.split_mesanim == -1 {
                s.push_int(-1);
                return Ok(());
            }
            let m = valid(&w.configs.mesanims, s.split_mesanim, "Mesanim")?;
            let lines = s.split_pages.get(page as usize).ok_or_else(|| format!("no split page {page}"))?.len();
            // `len[lines - 1]`: undefined (0 when pushed) outside the 4 slots
            let v = (lines as i32 - 1).try_into().ok().and_then(|i: usize| m.len.get(i)).copied().unwrap_or(0);
            s.push_int(v);
            Ok(())
        })(),
        _ => return None,
    })
}
