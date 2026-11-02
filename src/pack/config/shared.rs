//! Shared config packing (+ `lookup_param_value`).
//!
//! Config sources are read with Node `readline` semantics (lines split on
//! `\n`, `\r\n` and a lone `\r`; nothing trimmed; empty and `//` lines
//! skipped), `^CONST` references are substituted with the index quirk
//! (B.7), and each line goes through a per-type `parse` callback that yields a
//! [`Value`], an invalid value or an invalid key.

use crate::cache::config::{ConfigSet, dbtable::DbTableType, param::ParamType, script_var_type as svt};
use crate::io::packet::Packet;
use crate::pack::name_map;
use crate::pack::pack_file::Packs;
use crate::pack::parse::int;
use indexmap::IndexMap;
use std::collections::HashMap;

/// `PackedData`: a `dat` stream with one opcode block per config and an `idx`
/// of block sizes. Writers take `i64` and wrap to the field width (`Packet.p*`).
pub struct PackedData {
    pub dat: Packet,
    pub idx: Packet,
    marker: usize,
}

impl PackedData {
    pub fn new(size: i64) -> Self {
        let mut dat = Packet::alloc(5);
        let mut idx = Packet::alloc(3);
        dat.p2(size);
        idx.p2(size);
        PackedData { dat, idx, marker: 2 }
    }

    /// `next()`: terminate the current config with `0` and record its size.
    pub fn next(&mut self) {
        self.dat.p1(0);
        self.idx.p2((self.dat.pos - self.marker) as i64);
        self.marker = self.dat.pos;
    }

    pub fn p1(&mut self, v: i64) {
        self.dat.p1(v);
    }

    pub fn p2(&mut self, v: i64) {
        self.dat.p2(v);
    }

    pub fn p3(&mut self, v: i64) {
        self.dat.p3(v);
    }

    pub fn p4(&mut self, v: i64) {
        self.dat.p4(v);
    }

    pub fn pbool(&mut self, v: bool) {
        self.dat.pbool(v);
    }

    pub fn pjstr(&mut self, s: &str) {
        self.dat.pjstr(s);
    }
}

/// A `lookupParamValue` result.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Num(i64),
    Str(String),
}

impl Lookup {
    /// Strings are only looked up for `string` types, which write them as strings.
    pub fn num(&self) -> i64 {
        match self {
            Lookup::Num(n) => *n,
            Lookup::Str(_) => unreachable!("a string lookup written as a number"),
        }
    }
}

/// `ParamValue = {id, type, value}`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamValue {
    pub id: usize,
    pub kind: i32,
    pub value: Lookup,
}

/// A parsed config value (`ConfigValue`).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(i64),
    Bool(bool),
    Nums(Vec<i64>),
    Param(ParamValue),
    /// `HuntCheckInv`, `HuntCheckInvParam`, `HuntCheckInvCat`: `{inv, X, condition, val}`.
    HuntInv { inv: i64, other: i64, condition: String, val: i64 },
    /// `HuntCheckVar`.
    HuntVar { varp: i64, condition: String, val: i64 },
}

impl Value {
    /// The value as a number; other kinds are never written as one.
    pub fn num(&self) -> i64 {
        match self {
            Value::Num(n) => *n,
            Value::Bool(b) => *b as i64,
            _ => 0,
        }
    }

    pub fn str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }

    pub fn nums(&self) -> &[i64] {
        match self {
            Value::Nums(v) => v,
            _ => &[],
        }
    }

    pub fn is_true(&self) -> bool {
        *self == Value::Bool(true)
    }

    pub fn is_false(&self) -> bool {
        *self == Value::Bool(false)
    }
}

/// Why a `parse` callback rejected a line (JS `null` / `undefined`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bad {
    Value,
    Key,
}

pub type Parsed = Result<Value, Bad>;
pub type ConfigLines = Vec<(String, Value)>;

/// Everything the parse and pack callbacks read: the pack ids and the
/// decoders reloaded from the files this build already wrote.
pub struct Ctx<'a> {
    pub packs: &'a Packs,
    pub params: ConfigSet<ParamType>,
    pub dbtables: ConfigSet<DbTableType>,
    pub constants: HashMap<String, String>,
    pub model_flags: Vec<i32>,
}

impl Ctx<'_> {
    /// `modelFlags[id] |= flag` (a JS array: writes past the end extend it).
    pub fn flag_model(&mut self, id: i64, flag: i32) {
        let Ok(i) = usize::try_from(id) else { return };
        if self.model_flags.len() <= i {
            self.model_flags.resize(i + 1, 0);
        }
        self.model_flags[i] |= flag;
    }
}

pub fn is_config_boolean(s: &str) -> bool {
    matches!(s, "yes" | "no" | "true" | "false" | "1" | "0")
}

pub fn get_config_boolean(s: &str) -> bool {
    matches!(s, "yes" | "true" | "1")
}

/// The shared `numberKeys` block: a strict [`int`].
pub fn parse_number(value: &str) -> Parsed {
    int(value).map(Value::Num).ok_or(Bad::Value)
}

/// The shared `stringKeys` block.
pub fn parse_string(value: &str) -> Parsed {
    if value.encode_utf16().count() > 1000 { Err(Bad::Value) } else { Ok(Value::Str(value.to_string())) }
}

/// The shared `booleanKeys` block.
pub fn parse_bool(value: &str) -> Parsed {
    if is_config_boolean(value) { Ok(Value::Bool(get_config_boolean(value))) } else { Err(Bad::Value) }
}

/// A pack-name lookup as a parse result (missing → invalid value).
pub fn pack_index(id: Option<i64>) -> Parsed {
    id.map(Value::Num).ok_or(Bad::Value)
}

/// `ScriptVarType.getTypeChar(value)` as a parse result (`null` → invalid value).
pub fn type_char(value: &str) -> Parsed {
    svt::get_type_char(value).map(|c| Value::Num(c as i64)).ok_or(Bad::Value)
}

/// The text after the first comma; the whole string without one (autoint enum `val=`s rely on it, B.7).
pub fn after_comma(s: &str) -> &str {
    s.split_once(',').map_or(s, |(_, v)| v)
}

/// The shared `param=` parse: `name,value` against a loaded ParamType.
pub fn parse_param(ctx: &Ctx, value: &str) -> Parsed {
    let (name, v) = value.split_once(',').ok_or(Bad::Value)?;
    let param = ctx.params.get_by_name(name).ok_or(Bad::Value)?;
    let v = lookup_param_value(ctx, param.kind, v).ok_or(Bad::Value)?;
    Ok(Value::Param(ParamValue { id: param.id, kind: param.kind, value: v }))
}

/// Writes a params block (`249`, count, then `p3 id, pbool isString, value`).
pub fn write_params(out: &mut PackedData, params: &[ParamValue]) {
    out.p1(249);
    out.p1(params.len() as i64);
    for p in params {
        out.p3(p.id as i64);
        out.pbool(p.kind == svt::STRING);
        match (&p.value, p.kind == svt::STRING) {
            (Lookup::Str(s), true) => out.pjstr(s),
            (v, _) => out.p4(v.num()),
        }
    }
}

const STATS: &[&str] = &[
    "attack", "defence", "strength", "hitpoints", "ranged", "prayer", "magic", "cooking", "woodcutting", "fletching", "fishing", "firemaking", "crafting",
    "smithing", "mining", "herblore", "agility", "thieving", "slayer", "farming", "runecraft",
];

const NPC_STATS: &[&str] = &["hitpoints", "attack", "strength", "defence", "magic", "ranged"];

/// `lookupParamValue(type, value)`: `None` is `null`.
pub fn lookup_param_value(ctx: &Ctx, kind: i32, value: &str) -> Option<Lookup> {
    if value == "null" {
        return Some(if kind != svt::STRING { Lookup::Num(-1) } else { Lookup::Str(String::new()) });
    }
    let p = ctx.packs;
    let index = match kind {
        svt::INT => return int(value).map(Lookup::Num),
        svt::STRING => return (value.encode_utf16().count() <= 1000).then(|| Lookup::Str(value.to_string())),
        svt::BOOLEAN => return is_config_boolean(value).then(|| Lookup::Num(get_config_boolean(value) as i64)),
        svt::COORD => return coord(value).map(|(level, x, z)| Lookup::Num(z | (x << 14) | (level << 28))),
        svt::ENUM => p.enums.get_by_name(value),
        svt::NAMEDOBJ | svt::OBJ => p.obj.get_by_name(value),
        svt::LOC => p.loc.get_by_name(value),
        svt::COMPONENT => p.interface.get_by_name(value),
        svt::STRUCT => p.structs.get_by_name(value),
        svt::CATEGORY => p.category.get_by_name(value),
        svt::SPOTANIM => p.spot_anim.get_by_name(value),
        svt::HITSPLAT => p.hitsplat.get_by_name(value),
        svt::NPC => p.npc.get_by_name(value),
        svt::INV => p.inv.get_by_name(value),
        svt::SYNTH => p.synth.get_by_name(value),
        svt::SEQ => p.seq.get_by_name(value),
        svt::STAT => STATS.iter().position(|s| *s == value).map(|i| i as i64),
        svt::NPC_STAT => NPC_STATS.iter().position(|s| *s == value).map(|i| i as i64),
        svt::VARP => p.varp.get_by_name(value),
        svt::INTERFACE if !value.contains(':') => p.interface.get_by_name(value),
        svt::DBROW => p.db_row.get_by_name(value),
        svt::MIDI => p.midi.get_by_name(value),
        _ => None,
    };
    index.map(Lookup::Num)
}

/// `level_mx_mz_lx_lz` (strict ints within the map bounds) → absolute `(level, x, z)`.
pub fn coord(value: &str) -> Option<(i64, i64, i64)> {
    let mut parts = value.split('_');
    let mut next = |max: i64| parts.next().and_then(int).filter(|v| (0..=max).contains(v));
    let (level, mx, mz, lx, lz) = (next(3)?, next(255)?, next(255)?, next(63)?, next(63)?);
    parts.next().is_none().then_some((level, (mx << 6) + lx, (mz << 6) + lz))
}

/// `v[i] = value`, growing with holes (`None`, written as 0: B.8).
pub fn set_at<T: Clone>(v: &mut Vec<Option<T>>, i: usize, value: T) {
    if v.len() <= i {
        v.resize(i + 1, None);
    }
    v[i] = Some(value);
}

/// The 1-based number after `prefix` in a key as a 0-based index
/// (`frame3` → 2); `None` unless it is a strict positive [`int`].
pub fn key_index(key: &str, prefix: &str) -> Option<usize> {
    key.strip_prefix(prefix).and_then(int).filter(|&n| n >= 1).and_then(|n| usize::try_from(n - 1).ok())
}

/// `loadConstants()`: `^name = value` lines from every `.constant` file.
pub fn load_constants(src_dir: &str) -> Result<HashMap<String, String>, String> {
    let mut constants = HashMap::new();
    for entry in name_map::load_dir(&format!("{src_dir}/scripts"), ".constant") {
        for (_, line) in entry?.0 {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() != 2 {
                return Err(format!("Bad constant declaration on line: {line}"));
            }
            let mut name = parts[0].trim();
            let value = parts[1].trim();
            if let Some(n) = name.strip_prefix('^') {
                name = n;
            }
            if constants.contains_key(name) {
                return Err(format!("Duplicate constant found: {name}"));
            }
            constants.insert(name.to_string(), value.to_string());
        }
    }
    Ok(constants)
}

/// The `^CONST` substitution loop, in UTF-16 code units: after a replacement
/// the scan resumes at the *old* end index (B.7).
pub fn substitute_constants(value: &str, constants: &HashMap<String, String>) -> String {
    if !value.contains('^') {
        return value.to_string();
    }
    let mut v: Vec<u16> = value.encode_utf16().collect();
    let mut i = 0;
    while i < v.len() {
        if v[i] == b'^' as u16 {
            let start = i;
            let mut end = i + 1;
            while end < v.len() && ![b'\r', b'\n', b',', b' '].map(u16::from).contains(&v[end]) {
                end += 1;
            }
            let name = String::from_utf16_lossy(&v[start + 1..end]);
            if let Some(c) = constants.get(&name) {
                let mut next: Vec<u16> = v[..start].to_vec();
                next.extend(c.encode_utf16());
                next.extend_from_slice(&v[end..]);
                v = next;
            }
            i = end;
        }
        i += 1;
    }
    String::from_utf16_lossy(&v)
}

/// Node `readline` line splitting over a file decoded as UTF-8.
fn readline_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if b.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < b.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// `at` is `file:line`, or the file alone.
pub fn parse_step_error(at: impl std::fmt::Display, message: &str) -> String {
    format!("\nError during parsing - see {at}\n{message}")
}

pub fn pack_step_error(debug_name: &str, message: &str) -> String {
    format!("\nError during packing - [{debug_name}]\n{message}")
}

/// `readConfigs(...)` up to the `pack` call: every `[name]` block of every
/// file with this extension, in walk order, as parsed lines.
pub fn read_configs(
    ctx: &Ctx,
    files: &[String],
    ext: &str,
    required: &[&str],
    parse: impl Fn(&Ctx, &str, &str) -> Parsed,
) -> Result<IndexMap<String, ConfigLines>, String> {
    let mut configs: IndexMap<String, ConfigLines> = IndexMap::new();
    let check_required = |file: &str, config: &ConfigLines| -> Result<(), String> {
        for r in required {
            if !config.iter().any(|(k, _)| k == r) {
                return Err(parse_step_error(file, &format!("Missing required property: {r}")));
            }
        }
        Ok(())
    };
    for file in files.iter().filter(|f| f.ends_with(ext)) {
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut debug_name: Option<String> = None;
        let mut config: ConfigLines = Vec::new();
        for (n, line) in readline_lines(&text).into_iter().enumerate() {
            let line_number = n + 1;
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if line.starts_with('[') {
                if !line.ends_with(']') {
                    return Err(parse_step_error(format_args!("{file}:{line_number}"), &format!("Missing closing bracket: {line}")));
                }
                if let Some(name) = debug_name.take() {
                    check_required(file, &config)?;
                    configs.insert(name, std::mem::take(&mut config));
                }
                let name = &line[1..line.len() - 1];
                if name.is_empty() {
                    return Err(parse_step_error(format_args!("{file}:{line_number}"), "No config name"));
                }
                if configs.contains_key(name) {
                    return Err(parse_step_error(format_args!("{file}:{line_number}"), &format!("Duplicate config found: {name}")));
                }
                debug_name = Some(name.to_string());
                config = Vec::new();
                continue;
            }
            let Some(sep) = line.find('=') else {
                return Err(parse_step_error(format_args!("{file}:{line_number}"), &format!("Missing property separator: {line}")));
            };
            let key = &line[..sep];
            let value = substitute_constants(&line[sep + 1..], &ctx.constants);
            match parse(ctx, key, &value) {
                Ok(v) => config.push((key.to_string(), v)),
                Err(Bad::Value) => return Err(parse_step_error(format_args!("{file}:{line_number}"), &format!("Invalid property value: {line}"))),
                Err(Bad::Key) => return Err(parse_step_error(format_args!("{file}:{line_number}"), &format!("Invalid property key: {line}"))),
            }
        }
        if let Some(name) = debug_name {
            check_required(file, &config)?;
            configs.insert(name, config);
        }
    }
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_resume_at_old_index() {
        let c: HashMap<String, String> = [("a".into(), "LONGVALUE".into()), ("b".into(), "2".into())].into();
        assert_eq!(substitute_constants("^a,^b", &c), "LONGVALUE,2");
        // after `^a` → 9 chars the scan resumes at old index 2 (inside LONGVALUE), so `^b` is still reached later
        assert_eq!(substitute_constants("x^a ^b", &c), "xLONGVALUE 2");
        // a replacement that shortens the string skips the following reference
        let c2: HashMap<String, String> = [("long_name".into(), "1".into()), ("b".into(), "2".into())].into();
        assert_eq!(substitute_constants("^long_name,^b", &c2), "1,^b");
        assert_eq!(substitute_constants("^missing,^b", &c2), "^missing,2");
    }

    #[test]
    fn malformed_number_names_file_and_line() {
        let packs = Packs::default();
        let ctx = Ctx { packs: &packs, params: ConfigSet::default(), dbtables: ConfigSet::default(), constants: HashMap::new(), model_flags: Vec::new() };
        let dir = std::env::temp_dir().join(format!("ares-strict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.seq").to_string_lossy().replace('\\', "/");
        std::fs::write(&file, "[a]\nloops=2\ndelay3=5x\n").unwrap();
        let e = read_configs(&ctx, std::slice::from_ref(&file), ".seq", &[], super::super::client::parse_seq_config).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(e.contains(&format!("{file}:3\n")) && e.contains("delay3=5x"), "{e}");
    }

    #[test]
    fn readline_splits_like_node() {
        assert_eq!(readline_lines("a\r\nb\rc\n\nd"), ["a", "b", "c", "", "d"]);
        assert_eq!(readline_lines("a\n"), ["a"]);
    }
}
