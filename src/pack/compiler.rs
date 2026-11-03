//! The symbol tables (`CompilerTypeInfo` per kind)
//! the server script compiler needs, then `CompileServerScript`.
//!
//! Maps are keyed by `String(id)` in insertion order; ares-compiler iterates
//! them in JS property order itself. Kept quirks: `add()` raises `max`
//! to `id + 1` only when `max < id`; constants from `loadFileFull` lines keep
//! the last duplicate; a pack id with no config behind it is an error.
//!
//! Scripts are compiled from `<srcDir>/scripts`.

use crate::cache::config::{
    ConfigSet, component::Components, dbtable::DbTableType, inv::InvType, param::ParamType, script_var_type as svt,
    var::{VarBitType, VarNpcType, VarObjType, VarPlayerType, VarSharedType},
};
use crate::pack::parse::{int_at, load_dir_ext_full, read_text};
use ares_shared::tables::{NPC_MODES, NPC_STATS, PLAYER_STATS};
use ares_shared::opcode::{OPCODES, POINTERS};
use ares_shared::pointer::PointerType;
use crate::util::config::Build;
use indexmap::IndexMap;
use ares_compiler::runescript::type_info::CompilerTypeInfo;
use serde_json::{Value, json};
use std::fmt::Display;
use std::path::Path;

/// One `CompilerTypeInfo` under construction.
struct Info {
    max: i64,
    t: CompilerTypeInfo,
}

impl Info {
    fn new() -> Self {
        let m = IndexMap::new;
        let t = CompilerTypeInfo {
            max: -1, map: m(), vartype: m(), protect: IndexMap::new(), require: m(), require2: m(), conditional: IndexMap::new(), set: m(), set2: m(), corrupt: m(),
            corrupt2: m(),
        };
        Info { max: -1, t }
    }

    fn has(&self, id: i64) -> bool {
        self.t.map.contains_key(&id.to_string())
    }

    /// `add(id, name, updateMax = true)`.
    fn add(&mut self, id: i64, name: &str, update_max: bool) {
        self.t.map.insert(id.to_string(), name.to_string());
        if update_max && self.max < id {
            self.max = id + 1;
        }
    }

    /// `CompilerTypeInfo.load(file)`: `id=name` lines (`name` stops at a second `=`).
    fn load(file: &str) -> Result<Self, String> {
        let mut info = Info::new();
        let Ok(text) = read_text(file) else { return Ok(info) };
        for (i, line) in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).enumerate() {
            if line.is_empty() || !line.contains('=') {
                continue;
            }
            let mut parts = line.split('=');
            let id = int_at(parts.next().unwrap(), format_args!("{file}:{}", i + 1))?;
            info.add(id, parts.next().unwrap(), true);
        }
        Ok(info)
    }

    fn load_array(input: &[&str]) -> Self {
        let mut info = Info::new();
        for (i, s) in input.iter().enumerate() {
            info.add(i as i64, &s.to_lowercase(), true);
        }
        info
    }

    /// `loadMap(map, valueAsKey = true)`.
    fn load_map(input: &[(&str, i32)]) -> Self {
        let mut info = Info::new();
        for (name, value) in input {
            info.t.map.insert(value.to_string(), name.to_lowercase());
        }
        info
    }

    /// The integer ids `0..=max` that `map` defines.
    fn ids(&self) -> Vec<i64> {
        (0..=self.max).filter(|&id| self.has(id)).collect()
    }

    fn finish(mut self) -> CompilerTypeInfo {
        self.t.max = i32::try_from(self.max).expect("symbol ids fit i32");
        self.t
    }
}

/// `ParamType.getType()` (a smaller table than `ScriptVarType.getType`).
fn param_type_name(kind: i32) -> &'static str {
    match kind {
        svt::INT | svt::STRING | svt::ENUM | svt::OBJ | svt::LOC | svt::COMPONENT | svt::NAMEDOBJ | svt::STRUCT | svt::BOOLEAN | svt::COORD | svt::CATEGORY
        | svt::SPOTANIM | svt::NPC | svt::INV | svt::SYNTH | svt::SEQ | svt::STAT | svt::INTERFACE | svt::MIDI | svt::HITSPLAT => svt::get_type(kind),
        _ => "unknown",
    }
}

/// Pointer names as the compiler's `PointerType.forName` reads them (the key lower-cased).
fn names(pointers: &[PointerType]) -> String {
    pointers.iter().map(|p| p.name().to_lowercase()).collect::<Vec<_>>().join(",")
}

fn missing(what: &str, id: impl Display) -> String {
    format!("{what} {id} is not defined")
}

/// The `symbols` object `runServerCompiler` passes to the compiler
/// (`out` is `data/pack`, already holding this build's config files).
pub fn tables(b: &Build, out: &Path) -> Result<IndexMap<String, CompilerTypeInfo>, String> {
    let pack = |name: &str| Info::load(&format!("{}/pack/{name}.pack", b.src_dir));

    // commands: ScriptOpcodeMap sorted (stably) by opcode
    let mut commands: Vec<(&str, i32)> = OPCODES.to_vec();
    commands.sort_by_key(|c| c.1);
    let mut command = Info::new();
    for (name, opcode) in commands {
        command.add(opcode as i64, &name.to_lowercase(), true);
        let Some((_, p)) = POINTERS.iter().find(|(o, _)| *o == opcode) else { continue };
        let (k, t) = (opcode.to_string(), &mut command.t);
        if let Some(r) = p.require {
            t.require.insert(k.clone(), names(r));
            if let Some(r2) = p.require2 {
                t.require2.insert(k.clone(), names(r2));
            }
        }
        if let Some(s) = p.set {
            if p.conditional {
                t.conditional.insert(k.clone(), true);
            }
            t.set.insert(k.clone(), names(s));
            if let Some(s2) = p.set2 {
                t.set2.insert(k.clone(), names(s2));
            }
        }
        if let Some(c) = p.corrupt {
            t.corrupt.insert(k.clone(), names(c));
            if let Some(c2) = p.corrupt2 {
                t.corrupt2.insert(k, names(c2));
            }
        }
    }

    // constants: the last duplicate wins (unlike PackShared, which throws)
    let mut constant = Info::new();
    for entry in load_dir_ext_full(&format!("{}/scripts", b.src_dir), ".constant") {
        for line in &entry?.0 {
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let mut parts = line.split('=');
            let mut name = parts.next().unwrap().trim();
            if let Some(n) = name.strip_prefix('^') {
                name = n;
            }
            let raw = parts.next().ok_or_else(|| format!("constant line without '=': {line}"))?;
            let mut value = raw.trim();
            if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                value = v;
            }
            constant.t.map.insert(name.to_string(), value.to_string());
        }
    }

    let mut writeinv = pack("inv")?;
    let invs = ConfigSet::<InvType>::load(out, "inv.dat")?;
    for id in writeinv.ids() {
        let inv = invs.get(id as usize).ok_or_else(|| missing("InvType", id))?;
        writeinv.t.protect.insert(id.to_string(), inv.protect);
    }

    let component = pack("interface")?;
    let (mut interface, mut overlay) = (Info::new(), Info::new());
    let coms = Components::load(out)?;
    for id in component.ids() {
        let com = coms.get(id as usize).ok_or_else(|| missing("Component", id))?;
        let name = com.com_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| component.t.map[&id.to_string()].clone());
        interface.add(id, &name, true);
        if com.overlay {
            overlay.add(id, &name, true);
        }
    }

    let mut varp = pack("varp")?;
    let varps = ConfigSet::<VarPlayerType>::load_with_client(out, "varp.dat")?;
    for id in varp.ids() {
        let v = varps.get(id as usize).ok_or_else(|| missing("VarPlayerType", id))?;
        varp.t.vartype.insert(id.to_string(), svt::get_type(v.kind).into());
        varp.t.protect.insert(id.to_string(), v.protect);
    }

    let mut varbit = pack("varbit")?;
    let varbits = ConfigSet::<VarBitType>::load_with_client(out, "varbit.dat")?;
    for id in varbit.ids() {
        let v = varbits.get(id as usize).ok_or_else(|| missing("VarBitType", id))?;
        let base = varps.get(v.basevar as usize).filter(|_| v.basevar >= 0).ok_or_else(|| missing("VarPlayerType", v.basevar))?;
        varbit.t.vartype.insert(id.to_string(), svt::get_type(base.kind).into());
        varbit.t.protect.insert(id.to_string(), base.protect);
    }

    let mut varn = pack("varn")?;
    let varns = ConfigSet::<VarNpcType>::load(out, "varn.dat")?;
    for id in varn.ids() {
        let v = varns.get(id as usize).ok_or_else(|| missing("VarNpcType", id))?;
        varn.t.vartype.insert(id.to_string(), svt::get_type(v.kind).into());
    }

    let mut vars = pack("vars")?;
    let varss = ConfigSet::<VarSharedType>::load(out, "vars.dat")?;
    for id in vars.ids() {
        let v = varss.get(id as usize).ok_or_else(|| missing("VarSharedType", id))?;
        vars.t.vartype.insert(id.to_string(), svt::get_type(v.kind).into());
    }

    let mut varo = pack("varo")?;
    let varos = ConfigSet::<VarObjType>::load(out, "varo.dat")?;
    for id in varo.ids() {
        let v = varos.get(id as usize).ok_or_else(|| missing("VarObjType", id))?;
        varo.t.vartype.insert(id.to_string(), svt::get_type(v.kind).into());
    }

    let mut param = pack("param")?;
    let params = ConfigSet::<ParamType>::load(out, "param.dat")?;
    for id in param.ids() {
        let p = params.get(id as usize).ok_or_else(|| missing("ParamType", id))?;
        param.t.vartype.insert(id.to_string(), param_type_name(p.kind).into());
    }

    let dbtable = pack("dbtable")?;
    let mut dbcolumn = Info::new();
    let tables = ConfigSet::<DbTableType>::load(out, "dbtable.dat")?;
    for id in dbtable.ids() {
        let t = tables.get(id as usize).ok_or_else(|| missing("DbTableType", id))?;
        let tname = t.name.as_deref().unwrap_or("null");
        for (column, cname) in t.column_names.iter().enumerate() {
            let types: Vec<&str> = t.types.get(column).and_then(Option::as_ref).ok_or_else(|| missing("dbtable column", column))?.iter().map(|&k| svt::get_type(k)).collect();
            let column_index = (((t.id as i32) & 0xffff) << 12) | (((column as i32) & 0x7f) << 4);
            dbcolumn.add(column_index as i64, &format!("{tname}:{cname}"), false);
            dbcolumn.t.vartype.insert(column_index.to_string(), types.join(","));
            if types.len() > 1 {
                for (tuple, ty) in types.iter().enumerate() {
                    let tuple_index = column_index | (((tuple as i32) + 1) & 0xf);
                    dbcolumn.add(tuple_index as i64, &format!("{tname}:{cname}:{tuple}"), false);
                    dbcolumn.t.vartype.insert(tuple_index.to_string(), ty.to_string());
                }
            }
        }
    }

    let locshapes = [
        "wall_straight", "wall_diagonalcorner", "wall_l", "wall_squarecorner", "walldecor_straight_nooffset", "walldecor_straight_offset", "walldecor_diagonal_offset",
        "walldecor_diagonal_nooffset", "walldecor_diagonal_both", "wall_diagonal", "centrepiece_straight", "centrepiece_diagonal", "roof_straight",
        "roof_diagonal_with_roofedge", "roof_diagonal", "roof_l_concave", "roof_l_convex", "roof_flat", "roofedge_straight", "roofedge_diagonalcorner", "roofedge_l",
        "roofedge_squarecorner", "grounddecor",
    ];

    let tables: Vec<(&str, Info)> = vec![
        ("command", command),
        ("constant", constant),
        ("npc", pack("npc")?),
        ("obj", pack("obj")?),
        ("inv", pack("inv")?),
        ("writeinv", writeinv),
        ("seq", pack("seq")?),
        ("idk", pack("idk")?),
        ("spotanim", pack("spotanim")?),
        ("hitsplat", pack("hitsplat")?),
        ("loc", pack("loc")?),
        ("component", component),
        ("interface", interface),
        ("overlayinterface", overlay),
        ("varp", varp),
        ("varbit", varbit),
        ("varn", varn),
        ("vars", vars),
        ("varo", varo),
        ("param", param),
        ("struct", pack("struct")?),
        ("enum", pack("enum")?),
        ("hunt", pack("hunt")?),
        ("mesanim", pack("mesanim")?),
        ("synth", pack("synth")?),
        ("category", pack("category")?),
        ("runescript", pack("script")?),
        ("dbtable", dbtable),
        ("dbcolumn", dbcolumn),
        ("dbrow", pack("dbrow")?),
        ("midi", pack("midi")?),
        ("stat", Info::load_map(PLAYER_STATS)),
        ("npc_stat", Info::load_map(NPC_STATS)),
        ("npc_mode", Info::load_map(NPC_MODES)),
        ("fontmetrics", Info::load_array(&["p11_full", "p12_full", "b12_full", "q8_full"])),
        ("locshape", Info::load_array(&locshapes)),
    ];
    Ok(tables.into_iter().map(|(k, v)| (k.to_string(), v.finish())).collect())
}

/// [`tables`] as `symbols.json` (`max` an integer), for the golden gate.
pub fn symbols(b: &Build, out: &Path) -> Result<Value, String> {
    let json = |t: CompilerTypeInfo| {
        json!({
            "max": t.max, "map": t.map, "vartype": t.vartype, "protect": t.protect, "require": t.require, "require2": t.require2,
            "conditional": t.conditional, "set": t.set, "set2": t.set2, "corrupt": t.corrupt, "corrupt2": t.corrupt2,
        })
    };
    Ok(Value::Object(tables(b, out)?.into_iter().map(|(k, t)| (k, json(t))).collect()))
}

/// `runServerCompiler()`: compile `<srcDir>/scripts` into `<out>/server/script.{dat,idx}`.
pub fn run_server_compiler(b: &Build, out: &Path, source_names: ares_compiler::compiler::SourceNames) -> Result<ares_compiler::compiler::Output, String> {
    let symbols = tables(b, out)?;
    let config = ares_compiler::Config {
        source_paths: vec![format!("{}/scripts", b.src_dir).into()],
        symbols,
        writer: ares_compiler::WriterConfig::Jag { output: out.join("server") },
        source_names,
        ..Default::default()
    };
    ares_compiler::compile_server_script(&config).map_err(|e| format!("{e:?}"))
}
