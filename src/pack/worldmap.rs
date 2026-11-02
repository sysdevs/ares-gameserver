//! `data/pack/mapview/worldmap.jag` from the
//! server map store, the loc/npc/flo decoders and the csv zone lists.
//!
//! Kept quirk: `floorcol` indexes a fixed 103-row colour table and throws past
//! it. Zone csvs, labels and mapsquare names are strict ints.

#[path = "worldmap_colors.rs"]
mod colors;

use crate::cache::config::{ConfigSet, flo::FloType, loc::LocType, npc::NpcType};
use crate::engine::coord::pack_coord;
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use crate::pack::name_map::read_ascii;
use crate::pack::parse::{int, int_at};
use crate::pack::pack_file::Packs;
use crate::pack::pix::convert_image;
use crate::util::config::Build;
use indexmap::IndexMap;
use foldhash::HashSet; // ~32M tile lookups per build; never iterated
use std::path::Path;

/// `LocShape` values the wall classification uses.
const WALL_STRAIGHT: i32 = 0;
const WALL_L: i32 = 2;
const WALLDECOR_STRAIGHT_NOOFFSET: i32 = 4;
const WALL_DIAGONAL: i32 = 9;

fn csv_lines(path: &str) -> Result<Vec<String>, String> {
    let text = read_ascii(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect())
}

/// `processCsv`: the set of packed coords the zones in `path` cover. A line
/// is `level_mx_mz_lx_lz` (one 8x8 zone) or `from,to` (an inclusive range).
fn process_csv(path: &str, name: &str) -> Result<HashSet<i32>, String> {
    let mut result = HashSet::default();
    for (n, line) in csv_lines(path)?.iter().enumerate() {
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        let at = format!("{path}:{}", n + 1);
        let coord = |s: &str| -> Result<[i64; 5], String> {
            let v: Vec<i64> = s.split('_').map(|p| int_at(p, &at)).collect::<Result<_, _>>()?;
            v.try_into().map_err(|_| format!("{at}: bad zone {s:?}"))
        };
        match line.split_once(',') {
            Some((from, to)) => {
                let ([fl, fmx, fmz, flx, flz], [_, tmx, tmz, tlx, tlz]) = (coord(from)?, coord(to)?);
                if flx % 8 != 0 || flz % 8 != 0 || tlx % 8 != 7 || tlz % 8 != 7 || fmx > tmx || fmz > tmz || (flx > tlx || flz > tlz) {
                    tracing::warn!("{name} map not aligned to a zone {line}");
                }
                for x in (fmx << 6) + flx..=(tmx << 6) + tlx {
                    for z in (fmz << 6) + flz..=(tmz << 6) + tlz {
                        if !result.insert(pack_coord(fl, x, z)) {
                            tracing::warn!("Overlapping {name} map {line}");
                        }
                    }
                }
            }
            None => {
                let [level, mx, mz, lx, lz] = coord(line)?;
                for i in 0..8 {
                    for j in 0..8 {
                        result.insert(pack_coord(level, (mx << 6) + lx + i, (mz << 6) + lz + j));
                    }
                }
            }
        }
    }
    Ok(result)
}

type Grid<T> = [[[T; 64]; 64]; 4];

fn grid<T: Copy>(v: T) -> Box<Grid<T>> {
    Box::new([[[v; 64]; 64]; 4])
}

/// `packWorldmap()`.
pub fn pack_worldmap(b: &Build, packs: &Packs, out: &Path, server: &IndexMap<String, Vec<u8>>) -> Result<(), String> {
    let flos = ConfigSet::<FloType>::load_with_client(out, "flo.dat")?;
    let locs = ConfigSet::<LocType>::load_with_client(out, "loc.dat")?;
    let npcs = ConfigSet::<NpcType>::load_with_client(out, "npc.dat")?;

    let mut underlay = Packet::alloc(20_000_000);
    let mut overlay = Packet::alloc(20_000_000);
    let mut loc = Packet::alloc(20_000_000);
    let mut obj = Packet::alloc(5);
    let mut npc = Packet::alloc(5);
    let mut multi = Packet::alloc(5);
    let mut free = Packet::alloc(5);

    let maps = format!("{}/maps", b.src_dir);
    let multimap = process_csv(&format!("{maps}/multiway.csv"), "multiway")?;
    let freemap = process_csv(&format!("{maps}/free2play.csv"), "free")?;
    let ignoremap = process_csv(&format!("{maps}/ignore.csv"), "ignore")?;

    for map_id in 0..packs.map.max {
        let name = packs.map.get_by_id(map_id);
        if !name.starts_with('m') || !server.contains_key(name) {
            continue;
        }
        // `m<mx>_<mz>`
        let Some((mx, mz)) = name[1..].split_once('_').and_then(|(x, z)| Some((int(x)?, int(z)?))) else {
            return Err(format!("bad mapsquare name {name}"));
        };
        if ignoremap.contains(&pack_coord(0, mx << 6, mz << 6)) {
            continue;
        }
        // underground pass exception
        let level: usize = if mx == 33 && (71..=73).contains(&mz) { 1 } else { 0 };

        let mut flags = grid(0i32);
        let mut overlay_ids = grid(-1i32);
        let mut overlay_shape = grid(0i32);
        let mut overlay_rot = grid(0i32);
        let mut underlay_ids = grid(-1i32);

        let mut land = Packet::new(server[name].clone());
        for l in 0..4 {
            for x in 0..64 {
                for z in 0..64 {
                    loop {
                        let opcode = land.g1();
                        if opcode == 0 {
                            break;
                        } else if opcode == 1 {
                            land.g1();
                            break;
                        }
                        if opcode <= 49 {
                            overlay_ids[l][x][z] = land.g1();
                            overlay_shape[l][x][z] = (opcode - 2) >> 2;
                            overlay_rot[l][x][z] = (opcode - 2) & 0x3;
                        } else if opcode <= 81 {
                            flags[l][x][z] = opcode - 49;
                        } else {
                            underlay_ids[l][x][z] = opcode - 81;
                        }
                    }
                }
            }
        }

        overlay.p1(mx);
        overlay.p1(mz);
        underlay.p1(mx);
        underlay.p1(mz);
        for x in 0..64 {
            for z in 0..64 {
                let bridged = flags[1][x][z] & 0x2 == 2;
                let actual = bridged as usize + level;
                if overlay_ids[actual][x][z] != -1 {
                    overlay.p1(overlay_ids[actual][x][z] as i64);
                    overlay.p1((overlay_rot[actual][x][z] + (overlay_shape[actual][x][z] << 2)) as i64);
                } else {
                    overlay.p1(0);
                }
                underlay.p1(if underlay_ids[actual][x][z] != -1 { underlay_ids[actual][x][z] as i64 } else { 0 });
            }
        }

        let mut walls = grid(-1i32);
        let mut mapscenes = grid(-1i32);
        let mut mapfunctions = grid(-1i32);
        let Some(loc_data) = server.get(&format!("l{mx}_{mz}")) else { continue };
        let mut lb = Packet::new(loc_data.clone());
        let mut loc_id = -1;
        let mut id_offset = lb.gsmarts();
        while id_offset != 0 {
            loc_id += id_offset;
            let mut coord = 0;
            let mut coord_offset = lb.gsmarts();
            while coord_offset != 0 {
                coord += coord_offset - 1;
                let (z, x, l) = ((coord & 0x3f) as usize, ((coord >> 6) & 0x3f) as usize, ((coord >> 12) & 0x3) as usize);
                let info = lb.g1();
                coord_offset = lb.gsmarts();
                let bridged = flags[1][x][z] & 0x2 == 2;
                let actual = if bridged { l as isize - 1 } else { l as isize };
                if actual < 0 {
                    continue;
                }
                let actual = actual as usize;
                let t = locs.get(loc_id as usize).ok_or_else(|| format!("worldmap loc {loc_id} has no LocType"))?;
                let (shape, angle) = (info >> 2, info & 0x3);
                if t.mapscene == 22 {
                    continue; // hiding a dumb sprite
                }
                if walls[actual][x][z] == -1 {
                    let active = t.active != 0;
                    let w = match shape {
                        WALL_STRAIGHT => Some(1 + angle + if active { 4 } else { 0 }),
                        WALL_L => Some(9 + angle + if active { 4 } else { 0 }),
                        WALLDECOR_STRAIGHT_NOOFFSET => Some(17 + angle + if active { 4 } else { 0 }),
                        WALL_DIAGONAL => Some(25 + angle % 2 + if active { 2 } else { 0 }),
                        _ => None,
                    };
                    if let Some(w) = w {
                        walls[actual][x][z] = w;
                    }
                }
                if t.mapscene != -1 {
                    mapscenes[actual][x][z] = t.mapscene;
                }
                if t.mapfunction != -1 {
                    mapfunctions[actual][x][z] = t.mapfunction;
                }
            }
            id_offset = lb.gsmarts();
        }
        loc.p1(mx);
        loc.p1(mz);
        for x in 0..64 {
            for z in 0..64 {
                if walls[level][x][z] != -1 {
                    loc.p1(walls[level][x][z] as i64);
                }
                if mapscenes[level][x][z] != -1 {
                    loc.p1((29 + mapscenes[level][x][z]) as i64);
                }
                if mapfunctions[level][x][z] != -1 {
                    loc.p1((160 + mapfunctions[level][x][z]) as i64);
                }
                loc.p1(0);
            }
        }

        let spawns = |data: Option<&Vec<u8>>, out: &mut Packet, has_npc: bool| -> Result<(), String> {
            let data = data.cloned().unwrap_or_default();
            if data.is_empty() {
                return Ok(());
            }
            let mut tiles = grid(-1i32);
            let mut p = Packet::new(data);
            while p.available() > 0 {
                let pos = p.g2();
                let (l, x, z) = (((pos >> 12) & 0x3) as usize, ((pos >> 6) & 0x3f) as usize, (pos & 0x3f) as usize);
                let count = p.g1();
                for _ in 0..count {
                    let id = p.g2();
                    if has_npc {
                        let t = npcs.get(id as usize).ok_or_else(|| format!("worldmap npc {id} has no NpcType"))?;
                        if t.minimap {
                            tiles[l][x][z] = id;
                        }
                    } else {
                        p.g1();
                        tiles[l][x][z] = id;
                    }
                }
            }
            out.p1(mx);
            out.p1(mz);
            for x in 0..64 {
                for z in 0..64 {
                    out.pbool(tiles[level][x][z] != -1);
                }
            }
            Ok(())
        };
        spawns(server.get(&format!("o{mx}_{mz}")), &mut obj, false)?;
        spawns(server.get(&format!("n{mx}_{mz}")), &mut npc, true)?;

        for (set, out) in [(&multimap, &mut multi), (&freemap, &mut free)] {
            let mut tiles = grid(false);
            let mut any = false;
            for l in 0..4 {
                for x in 0..64 {
                    for z in 0..64 {
                        if set.contains(&pack_coord(l as i64, (mx << 6) + x as i64, (mz << 6) + z as i64)) {
                            tiles[l][x][z] = true;
                            any = true;
                        }
                    }
                }
            }
            if any {
                out.p1(mx);
                out.p1(mz);
                for x in 0..64 {
                    for z in 0..64 {
                        out.pbool(tiles[0][x][z]);
                    }
                }
            }
        }
    }

    let mut floorcol = Packet::alloc(1);
    floorcol.p2(flos.count() as i64);
    for i in 0..flos.count() {
        let (a, b) = colors::REF_COLORS.get(i).ok_or_else(|| format!("no worldmap colour for flo {i}"))?;
        floorcol.p4(*a as i64);
        floorcol.p4(*b as i64);
    }

    let mut index = Packet::alloc(2);
    let sprites = format!("{}/sprites", b.src_dir);
    let fonts = format!("{}/fonts", b.src_dir);
    let mapscene = convert_image(&mut index, &sprites, "mapscene")?;
    let mapfunction = convert_image(&mut index, &sprites, "mapfunction")?;
    let b12 = convert_image(&mut index, &fonts, "b12_full")?;
    let mapdots = convert_image(&mut index, &sprites, "mapdots")?;
    let fm = |n: &str| std::fs::read(format!("{fonts}/{n}.fm")).map_err(|e| format!("{n}.fm: {e}"));

    // `=<text>,<x>,<z>,<size>`; other lines are ignored
    let mut labels = Packet::alloc(1);
    let path = format!("{maps}/labels.txt");
    let label_lines: Vec<(usize, String)> = csv_lines(&path)?.into_iter().enumerate().filter(|(_, l)| l.starts_with('=')).collect();
    labels.p2(label_lines.len() as i64);
    for (n, line) in &label_lines {
        let at = format!("{path}:{}", n + 1);
        let parts: Vec<&str> = line[1..].split(',').collect();
        let &[text, x, z, size] = parts.as_slice() else { return Err(format!("{at}: bad label {line:?}")) };
        labels.pjstr(text);
        labels.p2(int_at(x, &at)?);
        labels.p2(int_at(z, &at)?);
        labels.p1(int_at(size, &at)?);
    }

    let mut jag = Jagfile::new(false);
    for (name, p) in [
        ("underlay", &underlay),
        ("overlay", &overlay),
        ("loc", &loc),
        ("obj", &obj),
        ("npc", &npc),
        ("multi", &multi),
        ("free", &free),
        ("floorcol", &floorcol),
        ("mapscene", &mapscene),
        ("mapfunction", &mapfunction),
        ("b12_full", &b12),
    ] {
        jag.write(&format!("{name}.dat"), p.written());
    }
    for n in ["f11", "f12", "f14", "f17", "f19", "f22", "f26", "f30"] {
        jag.write(&format!("{n}.dat"), &fm(n)?);
    }
    jag.write("mapdots.dat", mapdots.written());
    jag.write("index.dat", index.written());
    jag.write("labels.dat", labels.written());
    jag.save(out.join("mapview/worldmap.jag")).map_err(|e| format!("worldmap.jag: {e}"))
}
