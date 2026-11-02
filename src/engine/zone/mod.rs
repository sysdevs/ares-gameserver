//! `src/engine/zone/*`: zones (8x8 tiles of one level), the zone map, and
//! `ZoneGrid`. A zone's entity lists are intrusive lists in the World's
//! arenas (players/NPCs on the `next2/prev2` links, locs/objs on `next/prev`).

use crate::engine::linklist::{Links, List};
use indexmap::IndexMap;

/// `ZoneGrid`: one bit per zone of a level, 2048x2048 zones, for "is any
/// player within `radius` zones" checks. Out-of-range writes are dropped
/// like `Int32Array` stores.
pub struct ZoneGrid {
    grid: Vec<i32>,
}

const GRID_SIZE: i32 = 2048;
const INT_BITS: i32 = 5;
const BITS: i32 = (1 << INT_BITS) - 1;

impl Default for ZoneGrid {
    fn default() -> Self {
        ZoneGrid { grid: vec![0; (GRID_SIZE * (GRID_SIZE >> INT_BITS)) as usize] }
    }
}

fn grid_index(zone_x: i32, zone_y: i32) -> i32 {
    (zone_x << INT_BITS) | ((zone_y as u32) >> INT_BITS) as i32
}

impl ZoneGrid {
    pub fn flag(&mut self, zone_x: i32, zone_y: i32) {
        if let Some(v) = self.grid.get_mut(grid_index(zone_x, zone_y) as usize) {
            *v |= 1i32.wrapping_shl((zone_y & BITS) as u32);
        }
    }

    pub fn unflag(&mut self, zone_x: i32, zone_y: i32) {
        if let Some(v) = self.grid.get_mut(grid_index(zone_x, zone_y) as usize) {
            *v &= !1i32.wrapping_shl((zone_y & BITS) as u32);
        }
    }

    pub fn is_flagged(&self, zone_x: i32, zone_y: i32, radius: i32) -> bool {
        let (min_x, max_x) = ((zone_x - radius).max(0), (zone_x + radius).min(GRID_SIZE - 1));
        let (min_y, max_y) = ((zone_y - radius).max(0), (zone_y + radius).min(GRID_SIZE - 1));
        let start_y = min_y & !BITS;
        let end_y = ((max_y as u32 >> INT_BITS) << INT_BITS) as i32;
        for x in min_x..=max_x {
            let mut y = start_y;
            while y <= end_y {
                let line = self.grid.get(grid_index(x, y) as usize).copied().unwrap_or(0) as u32;
                let mut trimmed = line;
                if y + BITS > max_y {
                    trimmed = line & (1u32.wrapping_shl((max_y - y + 1) as u32).wrapping_sub(1));
                }
                if y < min_y {
                    trimmed >>= (min_y - y) as u32 & 31;
                }
                if trimmed != 0 {
                    return true;
                }
                y += 32;
            }
        }
        false
    }
}

/// `ZoneMap.zoneIndex`.
pub fn zone_index(x: i32, z: i32, level: i32) -> i32 {
    ((x >> 3) & 0x7ff) | (((z >> 3) & 0x7ff) << 11) | ((level & 0x3) << 22)
}

/// What a zone's player/NPC list nodes point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    Player(i32),
    Npc(i32),
}

pub struct Zone {
    pub index: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub players: List,
    pub npcs: List,
    pub locs: List,
    pub objs: List,
    pub players_count: i32,
    pub npcs_count: i32,
    pub locs_count: i32,
    pub objs_count: i32,
}

/// The arenas a zone's lists live in (owned by the World).
pub struct ZoneArenas<L, O> {
    pub members: Links<Member>,
    pub locs: Links<L>,
    pub objs: Links<O>,
}

impl<L, O> Default for ZoneArenas<L, O> {
    fn default() -> Self {
        ZoneArenas { members: Links::default(), locs: Links::default(), objs: Links::default() }
    }
}

impl Zone {
    fn new<L, O>(index: i32, a: &mut ZoneArenas<L, O>) -> Self {
        // `unpackIndex`: x and z are zone coordinates, level the top bits
        Zone {
            index,
            x: index & 0x7ff,
            z: (index >> 11) & 0x7ff,
            level: index >> 22,
            players: a.members.list(),
            npcs: a.members.list(),
            locs: a.locs.list(),
            objs: a.objs.list(),
            players_count: 0,
            npcs_count: 0,
            locs_count: 0,
            objs_count: 0,
        }
    }
}

/// `ZoneMap`: zones are created on first access and kept in creation order.
#[derive(Default)]
pub struct ZoneMap {
    pub zones: IndexMap<i32, Zone>,
    grids: std::collections::HashMap<i32, ZoneGrid>,
}

impl ZoneMap {
    pub fn zone_by_index<L, O>(&mut self, index: i32, a: &mut ZoneArenas<L, O>) -> &mut Zone {
        self.zones.entry(index).or_insert_with(|| Zone::new(index, a))
    }

    pub fn zone<L, O>(&mut self, x: i32, z: i32, level: i32, a: &mut ZoneArenas<L, O>) -> &mut Zone {
        self.zone_by_index(zone_index(x, z, level), a)
    }

    pub fn grid(&mut self, level: i32) -> &mut ZoneGrid {
        self.grids.entry(level).or_default()
    }

    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    pub fn loc_count(&self) -> i32 {
        self.zones.values().map(|z| z.locs_count).sum()
    }

    pub fn obj_count(&self) -> i32 {
        self.zones.values().map(|z| z.objs_count).sum()
    }
}
