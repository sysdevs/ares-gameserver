//! Loads `maps-server.zip` (or `server/maps/`) into
//! zones, collision and NPC spawns, plus the multiway and free-to-play CSVs.

use super::entity::{Loc, Npc, Obj, block_walk, lifecycle};
use super::world::World;
use super::zone::zone_index;
use crate::io::packet::Packet;
use crate::routefinder::{loc_angle, loc_layer, loc_shape_layer};
use std::collections::HashSet;
use std::path::Path;

const BLOCK_MAP_SQUARE: i32 = 0x1;
const LINK_BELOW: i32 = 0x2;
const REMOVE_ROOFS: i32 = 0x4;
const MAPSQUARE: usize = 4 * 64 * 64;

#[derive(Default)]
pub struct GameMap {
    pub members: bool,
    pub multimap: HashSet<i32>,
    pub freemap: HashSet<i32>,
}

impl GameMap {
    pub fn is_free_to_play(&self, x: i32, z: i32) -> bool {
        self.freemap.contains(&zone_index(x, z, 0))
    }

    pub fn is_multi(&self, x: i32, z: i32, level: i32) -> bool {
        self.multimap.contains(&zone_index(x, z, level))
    }

    fn borders_free_to_play(&self, x: i32, z: i32) -> bool {
        self.is_free_to_play(x + 1, z) || self.is_free_to_play(x - 1, z) || self.is_free_to_play(x, z + 1) || self.is_free_to_play(x, z - 1)
    }

    /// Skipped on a free world outside (and not next to) free-to-play zones.
    fn skip(&self, x: i32, z: i32) -> bool {
        !self.members && !self.is_free_to_play(x, z) && !self.borders_free_to_play(x, z)
    }
}

/// `loadCsvMap`: `y_mx_mz_lx_lz` per line; a part that is not an integer
/// (`"8,0"` from a range line) reads as 0, and a bad `lx`/`lz` zeroes the whole
/// `x`/`z` (the TS sum goes NaN).
fn load_csv_map(map: &mut HashSet<i32>, text: &str) {
    for line in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        let parts: Vec<Option<i32>> = line.split('_').map(|p| p.parse().ok()).collect();
        let part = |i: usize| parts.get(i).copied().flatten();
        let (y, mx, mz, lx, lz) = (part(0), part(1), part(2), part(3), part(4));
        if lx.is_none_or(|v| v % 8 != 0) || lz.is_none_or(|v| v % 8 != 0) {
            tracing::warn!("CSV map line is not aligned to a zone: {line}");
        }
        let abs = |m: Option<i32>, l: Option<i32>| l.map_or(0, |l| (m.unwrap_or(0) << 6).wrapping_add(l));
        map.insert(zone_index(abs(mx, lx), abs(mz, lz), y.unwrap_or(0)));
    }
}

/// `(x, z, level)` from `z | x << 6 | level << 12`.
fn unpack_coord(packed: i32) -> (i32, i32, i32) {
    ((packed >> 6) & 0x3f, packed & 0x3f, (packed >> 12) & 0x3)
}

fn pack_coord(x: i32, z: i32, level: i32) -> usize {
    ((z & 0x3f) | ((x & 0x3f) << 6) | ((level & 0x3) << 12)) as usize
}

/// `changeLocCollision`.
#[allow(clippy::too_many_arguments)]
pub fn change_loc_collision(w: &mut World, shape: i32, angle: i32, blockrange: bool, length: i32, width: i32, active: i32, x: i32, z: i32, level: i32, add: bool) {
    match loc_shape_layer(shape) {
        loc_layer::WALL => w.collision.change_wall(x, z, level, angle, shape, blockrange, add),
        loc_layer::GROUND => {
            if angle == loc_angle::NORTH || angle == loc_angle::SOUTH {
                w.collision.change_loc(x, z, level, length, width, blockrange, add);
            } else {
                w.collision.change_loc(x, z, level, width, length, blockrange, add);
            }
        }
        loc_layer::GROUND_DECOR if active == 1 => w.collision.change_floor(x, z, level, add),
        _ => {}
    }
}

/// `GameMap.init()`: `src_dir` holds `maps/*.csv`, `pack` is `data/pack`.
pub fn init(w: &mut World, src_dir: &Path, pack: &Path) -> Result<(), String> {
    if !src_dir.join("maps").exists() {
        return Ok(());
    }
    for (file, multi) in [("multiway.csv", true), ("free2play.csv", false)] {
        let path = src_dir.join("maps").join(file);
        if path.exists() {
            let text = std::fs::read(&path).map_err(|e| e.to_string())?;
            // read as 'ascii': high bits stripped
            let text: String = text.iter().map(|&b| (b & 0x7f) as char).collect();
            load_csv_map(if multi { &mut w.map.multimap } else { &mut w.map.freemap }, &text);
        }
    }

    let zip = pack.join(".cache/maps-server.zip");
    let entries = if zip.exists() {
        crate::pack::stored_zip::read(&std::fs::read(&zip).map_err(|e| e.to_string())?)?
    } else {
        let dir = pack.join("server/maps");
        let mut entries = indexmap::IndexMap::new();
        // `readdirSync` order (pinned bytewise, like the tools)
        for name in crate::pack::parse::list_dir(&dir.to_string_lossy()) {
            entries.insert(name.clone(), std::fs::read(dir.join(&name)).map_err(|e| e.to_string())?);
        }
        entries
    };
    let maps: Vec<String> = entries.keys().filter(|k| k.starts_with('m')).cloned().collect();
    for name in maps {
        let mut it = name[1..].split('_').map(|p| p.parse::<i32>().unwrap_or(0));
        let (mx, mz) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        let (bx, bz) = (mx << 6, mz << 6);
        let get = |prefix: &str| entries.get(&format!("{prefix}{mx}_{mz}")).cloned().unwrap_or_default();
        load_npcs(w, &mut Packet::new(get("n")), bx, bz);
        load_objs(w, &mut Packet::new(get("o")), bx, bz);
        let mut lands = vec![0i8; MAPSQUARE];
        load_ground(w, &mut lands, &mut Packet::new(get("m")), bx, bz);
        load_locations(w, &lands, &mut Packet::new(get("l")), bx, bz);
    }
    Ok(())
}

fn load_npcs(w: &mut World, p: &mut Packet, bx: i32, bz: i32) {
    while p.available() > 0 {
        let (x, z, level) = unpack_coord(p.g2());
        let (ax, az) = (bx + x, bz + z);
        for _ in 0..p.g1() {
            let id = p.g2();
            if !w.map.members && !w.map.is_free_to_play(ax, az) {
                continue;
            }
            let Some(t) = w.configs.npcs.get(id as usize) else {
                tracing::error!("Invalid npc type {id} in map m{}_{}.jm2", bx >> 6, bz >> 6);
                continue;
            };
            if !t.members || w.map.members {
                let (size, blockwalk) = (t.size, t.blockwalk);
                let nid = w.npcs.next_id() as i32;
                let t = w.configs.npcs.get(id as usize).expect("npc type");
                let npc = Npc::new(level, ax, az, size, size, lifecycle::RESPAWN, nid, t, blockwalk, w.configs.varns.count());
                w.add_npc(npc, -1);
            }
        }
    }
}

fn load_objs(w: &mut World, p: &mut Packet, bx: i32, bz: i32) {
    while p.available() > 0 {
        let (x, z, level) = unpack_coord(p.g2());
        let (ax, az) = (bx + x, bz + z);
        for _ in 0..p.g1() {
            let id = p.g2();
            let count = p.g1();
            if !w.map.members && !w.map.is_free_to_play(ax, az) {
                continue;
            }
            let members = w.configs.objs.get(id as usize).is_some_and(|t| t.members);
            if !members || w.map.members {
                w.add_static_obj(Obj::new(level, ax, az, lifecycle::RESPAWN, id, count as i64));
            }
        }
    }
}

fn load_ground(w: &mut World, lands: &mut [i8], p: &mut Packet, bx: i32, bz: i32) {
    for level in 0..4 {
        for x in 0..64 {
            for z in 0..64 {
                loop {
                    let opcode = p.g1();
                    if opcode == 0 {
                        break;
                    } else if opcode == 1 {
                        p.pos += 1;
                        break;
                    }
                    if opcode <= 49 {
                        p.pos += 1;
                    } else if opcode <= 81 {
                        lands[pack_coord(x, z, level)] = (opcode - 49) as i8;
                    }
                }
            }
        }
    }
    for level in 0..4 {
        for x in 0..64 {
            let ax = x + bx;
            for z in 0..64 {
                let az = z + bz;
                if w.map.skip(ax, az) {
                    continue;
                }
                if x % 7 == 0 && z % 7 == 0 {
                    w.collision.allocate_if_absent(ax, az, level);
                }
                let land = lands[pack_coord(x, z, level)] as i32;
                if land & REMOVE_ROOFS != 0 {
                    w.collision.change_roof(ax, az, level, true);
                }
                if land & BLOCK_MAP_SQUARE != BLOCK_MAP_SQUARE {
                    continue;
                }
                let link = if level == 1 { land } else { lands[pack_coord(x, z, 1)] as i32 };
                let actual = if link & LINK_BELOW == LINK_BELOW { level - 1 } else { level };
                if actual < 0 {
                    continue;
                }
                w.collision.change_floor(ax, az, actual, true);
            }
        }
    }
}

fn load_locations(w: &mut World, lands: &[i8], p: &mut Packet, bx: i32, bz: i32) {
    let mut loc_id = -1;
    let mut loc_offset = p.gsmarts();
    while loc_offset != 0 {
        loc_id += loc_offset;
        let mut coord = 0;
        let mut coord_offset = p.gsmarts();
        while coord_offset != 0 {
            coord += coord_offset - 1;
            let (x, z, level) = unpack_coord(coord);
            let info = p.g1();
            coord_offset = p.gsmarts();
            let (ax, az) = (x + bx, z + bz);
            if w.map.skip(ax, az) {
                continue;
            }
            // `lands[coord]` indexes with the unmasked running coord
            let link = if level == 1 { lands.get(coord as usize).copied().unwrap_or(0) as i32 } else { lands[pack_coord(x, z, 1)] as i32 };
            let actual = if link & LINK_BELOW == LINK_BELOW { level - 1 } else { level };
            if actual < 0 {
                continue;
            }
            let Some(t) = w.configs.locs.get(loc_id as usize) else {
                tracing::error!("Invalid loc type {loc_id} in map m{}_{}.jm2", bx >> 6, bz >> 6);
                continue;
            };
            let (width, length, blockwalk, blockrange, active) = (t.width, t.length, t.blockwalk, t.blockrange, t.active);
            let (shape, angle) = (info >> 2, info & 0x3);
            if blockwalk {
                change_loc_collision(w, shape, angle, blockrange, length, width, active, ax, az, actual, true);
            }
            // `if (type.active)`: -1 is truthy
            if active != 0 {
                w.add_static_loc(Loc::new(actual, ax, az, width, length, lifecycle::RESPAWN, loc_id, shape, angle));
            }
        }
        loc_offset = p.gsmarts();
    }
}

/// `BlockWalk` collision an NPC adds on spawn (`World.addNpc`).
pub fn npc_spawn_collision(w: &mut World, npc_width: i32, x: i32, z: i32, level: i32, block: i32, add: bool) {
    match block {
        block_walk::NPC => w.collision.change_npc(x, z, level, npc_width, add),
        block_walk::ALL => {
            w.collision.change_npc(x, z, level, npc_width, add);
            w.collision.change_block(x, z, level, npc_width, add);
        }
        _ => {}
    }
}
