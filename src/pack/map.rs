//! A clean build of the maps: every `m<x>_<z>` map in
//! `map.pack` id order is parsed from `maps/m<x>_<z>.jm2` and encoded into
//! the land and loc streams (gzipped into cache archive 4 with version 1) and
//! the server's npc/obj spawn blobs. The raw land/loc/npc/obj streams are
//! stored (in that order per map) in `data/pack/.cache/maps-server.zip`.
//!
//! Lines are `level x z: data` under `==== SECTION ====` headers; every
//! number is a strict int and tile values are stored as `i16` (wrapping).

use crate::cache::config::{ConfigSet, npc::NpcType};
use crate::io::filestream::FileStream;
use crate::io::gzip::compress_gz;
use crate::io::packet::Packet;
use crate::pack::pack_file::Packs;
use crate::pack::parse::int_at;
use crate::pack::stored_zip;
use crate::util::config::Build;
use indexmap::IndexMap;
use std::path::Path;

#[derive(Clone, Copy)]
struct Tile {
    h: i64,
    overlay_id: i64,
    overlay_shape: i64,
    overlay_rot: i64,
    flags: i64,
    underlay: i64,
}

struct ParsedMap {
    land: IndexMap<i32, Tile>,
    loc: IndexMap<i32, Vec<(i64, i64, i64)>>,
    npc: IndexMap<i32, Vec<i64>>,
    obj: IndexMap<i32, Vec<(i64, i64)>>,
}

/// Reads a `.jm2`: `\r` dropped, empty lines skipped.
fn read_map(file: &str) -> Result<ParsedMap, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).replace('\r', "");
    let mut m = ParsedMap { land: IndexMap::new(), loc: IndexMap::new(), npc: IndexMap::new(), obj: IndexMap::new() };
    let mut section = "";
    for (n, line) in text.split('\n').enumerate().filter(|(_, l)| !l.is_empty()) {
        let at = format!("{file}:{}", n + 1);
        let bad = || format!("{at}: bad map line {line:?}");
        if line.starts_with('=') {
            section = line.trim_matches('=').trim();
            continue;
        }
        let int = |s: &str| int_at(s, &at);
        let (pos, data) = line.split_once(": ").ok_or_else(bad)?;
        let pos: Vec<i64> = pos.split(' ').map(int).collect::<Result<_, _>>()?;
        let &[level, x, z] = pos.as_slice() else { return Err(bad()) };
        let key = ((level << 12) | (x << 6) | z) as i32;
        let ints = |s: &str, sep: char, n: std::ops::RangeInclusive<usize>| -> Result<Vec<i64>, String> {
            let v: Vec<i64> = s.split(sep).map(int).collect::<Result<_, _>>()?;
            if n.contains(&v.len()) { Ok(v) } else { Err(bad()) }
        };

        match section {
            "MAP" => {
                let mut t = Tile { h: 0, overlay_id: -1, overlay_shape: -1, overlay_rot: -1, flags: -1, underlay: -1 };
                for token in data.split(' ') {
                    let (kind, info) = token.split_at_checked(1).ok_or_else(bad)?;
                    match kind {
                        "h" => t.h = int(info)?,
                        // `o<id>[;<shape>[;<rotation>]]`
                        "o" => {
                            let o = ints(info, ';', 1..=3)?;
                            (t.overlay_id, t.overlay_shape, t.overlay_rot) = (o[0], o.get(1).copied().unwrap_or(-1), o.get(2).copied().unwrap_or(-1));
                        }
                        "f" => t.flags = int(info)?,
                        "u" => t.underlay = int(info)?,
                        _ => return Err(bad()),
                    }
                }
                m.land.insert(key, t);
            }
            // `<id>[ <shape>[ <angle>]]`
            "LOC" => {
                let v = ints(data, ' ', 1..=3)?;
                m.loc.entry(key).or_default().push((v[0], v.get(1).copied().unwrap_or(10), v.get(2).copied().unwrap_or(0)));
            }
            "NPC" => m.npc.entry(key).or_default().push(int(data)?),
            "OBJ" => {
                let v = ints(data, ' ', 2..=2)?;
                m.obj.entry(key).or_default().push((v[0], v[1]));
            }
            _ => {}
        }
    }
    Ok(m)
}

/// `updateModelFlags`: npc models and heads get `0x4`.
fn update_model_flags(npc: &IndexMap<i32, Vec<i64>>, model_flags: &mut Vec<i32>, npcs: &ConfigSet<NpcType>) -> Result<(), String> {
    for &id in npc.values().flatten() {
        let t = usize::try_from(id).ok().and_then(|i| npcs.get(i)).ok_or_else(|| format!("map npc {id} has no NpcType"))?;
        for &m in t.models.iter().flatten().chain(t.heads.iter().flatten()) {
            let i = m as usize;
            if model_flags.len() <= i {
                model_flags.resize(i + 1, 0);
            }
            model_flags[i] |= 0x4;
        }
    }
    Ok(())
}

fn encode_land(m: &ParsedMap) -> Vec<u8> {
    const STRIDE: usize = 4 * 64 * 64;
    let mut height = vec![0i16; STRIDE];
    let mut overlay = vec![-1i16; STRIDE];
    let mut shape = vec![-1i16; STRIDE];
    let mut rotation = vec![-1i16; STRIDE];
    let mut flags = vec![-1i16; STRIDE];
    let mut underlay = vec![-1i16; STRIDE];
    for (&key, t) in &m.land {
        // writes outside the typed arrays are dropped
        if key < 0 || key as usize >= STRIDE {
            continue;
        }
        let i = key as usize;
        height[i] = t.h as i16;
        overlay[i] = t.overlay_id as i16;
        shape[i] = t.overlay_shape as i16;
        rotation[i] = t.overlay_rot as i16;
        flags[i] = t.flags as i16;
        underlay[i] = t.underlay as i16;
    }
    let mut out = Packet::alloc(3);
    for i in 0..STRIDE {
        let (h, o, s, r, f, u) = (height[i] as i64, overlay[i] as i64, shape[i] as i64, rotation[i] as i64, flags[i] as i64, underlay[i] as i64);
        if h == 0 && o == -1 && f == -1 && u == -1 {
            out.p1(0);
            continue;
        }
        if o != -1 {
            let mut opcode = 2;
            if s != -1 {
                opcode += s << 2;
            }
            if r != -1 {
                opcode += r;
            }
            out.p1(opcode);
            out.p1(o);
        }
        if f != -1 {
            out.p1(f + 49);
        }
        if u != -1 {
            out.p1(u + 81);
        }
        if h != 0 {
            out.p1(1);
            out.p1(h);
        } else {
            out.p1(0);
        }
    }
    out.written().to_vec()
}

fn encode_locs(m: &ParsedMap) -> Vec<u8> {
    let mut list: Vec<(i64, i32, i64, i64)> = Vec::new();
    for (&key, entries) in &m.loc {
        let packed = (((key >> 12) & 0x3) << 12) | (((key >> 6) & 0x3f) << 6) | (key & 0x3f);
        for &(id, shape, angle) in entries {
            list.push((id, packed, shape, angle));
        }
    }
    // stable sort by id, then packed coord
    list.sort_by_key(|l| (l.0, l.1));
    let mut out = Packet::alloc(3);
    let mut last_id = -1;
    let mut i = 0;
    while i < list.len() {
        let id = list[i].0;
        out.psmart((id - last_id) as i32);
        last_id = id;
        let mut last_data = 0;
        while i < list.len() && list[i].0 == id {
            let (_, data, shape, angle) = list[i];
            i += 1;
            out.psmart(data - last_data + 1);
            last_data = data;
            out.p1((shape << 2) | angle);
        }
        out.psmart(0);
    }
    out.psmart(0);
    out.written().to_vec()
}

fn encode_npcs(m: &ParsedMap) -> Vec<u8> {
    let mut out = Packet::alloc(1);
    for (&key, ids) in &m.npc {
        out.p2(key as i64);
        out.p1(ids.len() as i64);
        for &id in ids {
            out.p2(id);
        }
    }
    out.written().to_vec()
}

fn encode_objs(m: &ParsedMap) -> Vec<u8> {
    let mut out = Packet::alloc(1);
    for (&key, objs) in &m.obj {
        out.p2(key as i64);
        out.p1(objs.len() as i64);
        for &(id, count) in objs {
            out.p2(id);
            out.p1(count);
        }
    }
    out.written().to_vec()
}

/// `packMaps(cache, modelFlags)`: returns the server map store (also written
/// to `<out>/.cache/maps-server.zip`).
/// One map's streams, ready for the serial cache writes.
struct Encoded {
    xz: String,
    land: Vec<u8>,
    land_gz: Vec<u8>,
    locs: Vec<u8>,
    locs_gz: Vec<u8>,
    npcs: Vec<u8>,
    objs: Vec<u8>,
    npc: IndexMap<i32, Vec<i64>>,
}

fn encode(xz: &str, file: &str) -> Result<Encoded, String> {
    let m = read_map(file)?;
    let (land, locs) = (encode_land(&m), encode_locs(&m));
    let (npcs, objs) = (encode_npcs(&m), encode_objs(&m));
    Ok(Encoded { xz: xz.to_string(), land_gz: compress_gz(&land), locs_gz: compress_gz(&locs), land, locs, npcs, objs, npc: m.npc })
}

pub fn pack_maps(b: &Build, packs: &Packs, out: &Path, cache: &mut FileStream, model_flags: &mut Vec<i32>) -> Result<IndexMap<String, Vec<u8>>, String> {
    let mut server: IndexMap<String, Vec<u8>> = IndexMap::new();
    if !Path::new(&format!("{}/maps", b.src_dir)).exists() {
        return Ok(server);
    }
    let npcs = ConfigSet::<NpcType>::load_with_client(out, "npc.dat")?;
    let mut todo = Vec::new();
    for id in 0..packs.map.max {
        let name = packs.map.get_by_id(id);
        let Some(xz) = name.strip_prefix('m') else { continue };
        let file = format!("{}/maps/m{xz}.jm2", b.src_dir);
        if !Path::new(&file).exists() {
            tracing::warn!("missing map m{xz}");
            continue;
        }
        todo.push((xz.to_string(), file));
    }
    // parse, encode and gzip in parallel; the cache writes below stay serial
    // and in map id order, which fixes the cache layout
    for e in crate::pack::par_map(&todo, |(xz, file)| encode(xz, file)) {
        let e = e?;
        let xz = &e.xz;
        if let Some(map_id) = packs.map.get_by_name(&format!("m{xz}")) {
            cache.write(4, map_id as usize, &e.land_gz, 1);
        }
        if let Some(loc_id) = packs.map.get_by_name(&format!("l{xz}")) {
            cache.write(4, loc_id as usize, &e.locs_gz, 1);
        }
        server.insert(format!("m{xz}"), e.land);
        server.insert(format!("l{xz}"), e.locs);
        server.insert(format!("n{xz}"), e.npcs);
        server.insert(format!("o{xz}"), e.objs);
        update_model_flags(&e.npc, model_flags, &npcs)?;
    }
    let zip = out.join(".cache/maps-server.zip");
    std::fs::create_dir_all(zip.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&zip, stored_zip::write(&server)).map_err(|e| format!("{}: {e}", zip.display()))?;
    Ok(server)
}
