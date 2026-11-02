//! Per-zone (8x8 tile) flag arrays keyed by
//! `x>>3 | z>>3 << 11 | level << 22`, allocated on first write.

use super::{flag, loc_angle, loc_shape};
use foldhash::HashMap; // not SipHash: every pathfinder step looks tiles up

#[derive(Default)]
pub struct CollisionEngine {
    zones: HashMap<i32, [u32; 64]>,
}

/// `zoneIndex`: arithmetic shifts, so negative coordinates wrap into zone 0x7ff.
pub fn zone_index(x: i32, z: i32, y: i32) -> i32 {
    ((x >> 3) & 0x7ff) | (((z >> 3) & 0x7ff) << 11) | ((y & 0x3) << 22)
}

fn tile_index(x: i32, z: i32) -> usize {
    ((x & 0x7) | ((z & 0x7) << 3)) as usize
}

impl CollisionEngine {
    fn zone_mut(&mut self, x: i32, z: i32, y: i32) -> &mut [u32; 64] {
        self.zones.entry(zone_index(x, z, y)).or_insert([0; 64])
    }

    pub fn allocate_if_absent(&mut self, x: i32, z: i32, y: i32) {
        self.zone_mut(x, z, y);
    }

    pub fn deallocate_if_present(&mut self, x: i32, z: i32, y: i32) {
        self.zones.remove(&zone_index(x, z, y));
    }

    pub fn is_zone_allocated(&self, x: i32, z: i32, y: i32) -> bool {
        self.zones.contains_key(&zone_index(x, z, y))
    }

    pub fn get(&self, x: i32, z: i32, y: i32) -> u32 {
        self.zones.get(&zone_index(x, z, y)).map_or(flag::NULL, |zone| zone[tile_index(x, z)])
    }

    pub fn is_flagged(&self, x: i32, z: i32, y: i32, masks: u32) -> bool {
        self.zones.get(&zone_index(x, z, y)).is_some_and(|zone| zone[tile_index(x, z)] & masks != flag::OPEN)
    }

    pub fn set(&mut self, x: i32, z: i32, y: i32, mask: u32) {
        self.zone_mut(x, z, y)[tile_index(x, z)] = mask;
    }

    pub fn add(&mut self, x: i32, z: i32, y: i32, mask: u32) {
        self.zone_mut(x, z, y)[tile_index(x, z)] |= mask;
    }

    pub fn remove(&mut self, x: i32, z: i32, y: i32, mask: u32) {
        self.zone_mut(x, z, y)[tile_index(x, z)] &= !mask;
    }

    fn apply(&mut self, x: i32, z: i32, y: i32, mask: u32, add: bool) {
        if add { self.add(x, z, y, mask) } else { self.remove(x, z, y, mask) }
    }

    pub fn change_floor(&mut self, x: i32, z: i32, y: i32, add: bool) {
        self.apply(x, z, y, flag::FLOOR, add);
    }

    pub fn change_roof(&mut self, x: i32, z: i32, y: i32, add: bool) {
        self.apply(x, z, y, flag::ROOF, add);
    }

    pub fn change_npc(&mut self, x: i32, z: i32, y: i32, size: i32, add: bool) {
        self.change_area(x, z, y, size, size, flag::NPC_OCC, add);
    }

    pub fn change_block(&mut self, x: i32, z: i32, y: i32, size: i32, add: bool) {
        self.change_area(x, z, y, size, size, flag::BLOCK_NPC_AND_PLAYERS, add);
    }

    pub fn change_player_occ(&mut self, x: i32, z: i32, y: i32, size: i32, add: bool) {
        self.change_area(x, z, y, size, size, flag::PLAYER_OCC, add);
    }

    pub fn change_loc(&mut self, x: i32, z: i32, y: i32, width: i32, length: i32, blockrange: bool, add: bool) {
        let mask = if blockrange { flag::LOC | flag::LOC_PROJ_BLOCKER } else { flag::LOC };
        self.change_area(x, z, y, width, length, mask, add);
    }

    /// `index % width`, `index / width | 0` over `width * length` tiles.
    fn change_area(&mut self, x: i32, z: i32, y: i32, width: i32, length: i32, mask: u32, add: bool) {
        for index in 0..(width * length).max(0) {
            self.apply(x + index % width, z + index / width, y, mask, add);
        }
    }

    pub fn change_wall(&mut self, x: i32, z: i32, y: i32, angle: i32, shape: i32, blockrange: bool, add: bool) {
        match shape {
            loc_shape::WALL_STRAIGHT => self.wall_straight(x, z, y, angle, blockrange, add),
            loc_shape::WALL_DIAGONAL_CORNER | loc_shape::WALL_SQUARE_CORNER => self.wall_corner(x, z, y, angle, blockrange, add),
            loc_shape::WALL_L => self.wall_l(x, z, y, angle, blockrange, add),
            _ => {}
        }
    }

    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    /// Every allocated zone, for dumps and tests.
    pub fn zones(&self) -> impl Iterator<Item = (i32, &[u32; 64])> {
        self.zones.iter().map(|(&k, v)| (k, v))
    }

    fn wall_straight(&mut self, x: i32, z: i32, y: i32, angle: i32, blockrange: bool, add: bool) {
        let m = |normal, proj| if blockrange { proj } else { normal };
        let west = m(flag::WALL_WEST, flag::WALL_WEST_PROJ_BLOCKER);
        let east = m(flag::WALL_EAST, flag::WALL_EAST_PROJ_BLOCKER);
        let north = m(flag::WALL_NORTH, flag::WALL_NORTH_PROJ_BLOCKER);
        let south = m(flag::WALL_SOUTH, flag::WALL_SOUTH_PROJ_BLOCKER);
        match angle {
            loc_angle::WEST => self.pair(x, z, y, west, x - 1, z, east, add),
            loc_angle::NORTH => self.pair(x, z, y, north, x, z + 1, south, add),
            loc_angle::EAST => self.pair(x, z, y, east, x + 1, z, west, add),
            loc_angle::SOUTH => self.pair(x, z, y, south, x, z - 1, north, add),
            _ => {}
        }
        if blockrange {
            self.wall_straight(x, z, y, angle, false, add);
        }
    }

    fn wall_corner(&mut self, x: i32, z: i32, y: i32, angle: i32, blockrange: bool, add: bool) {
        let m = |normal, proj| if blockrange { proj } else { normal };
        let nw = m(flag::WALL_NORTH_WEST, flag::WALL_NORTH_WEST_PROJ_BLOCKER);
        let se = m(flag::WALL_SOUTH_EAST, flag::WALL_SOUTH_EAST_PROJ_BLOCKER);
        let ne = m(flag::WALL_NORTH_EAST, flag::WALL_NORTH_EAST_PROJ_BLOCKER);
        let sw = m(flag::WALL_SOUTH_WEST, flag::WALL_SOUTH_WEST_PROJ_BLOCKER);
        match angle {
            loc_angle::WEST => self.pair(x, z, y, nw, x - 1, z + 1, se, add),
            loc_angle::NORTH => self.pair(x, z, y, ne, x + 1, z + 1, sw, add),
            loc_angle::EAST => self.pair(x, z, y, se, x + 1, z - 1, nw, add),
            loc_angle::SOUTH => self.pair(x, z, y, sw, x - 1, z - 1, ne, add),
            _ => {}
        }
        if blockrange {
            self.wall_corner(x, z, y, angle, false, add);
        }
    }

    fn wall_l(&mut self, x: i32, z: i32, y: i32, angle: i32, blockrange: bool, add: bool) {
        let m = |normal, proj| if blockrange { proj } else { normal };
        let west = m(flag::WALL_WEST, flag::WALL_WEST_PROJ_BLOCKER);
        let east = m(flag::WALL_EAST, flag::WALL_EAST_PROJ_BLOCKER);
        let north = m(flag::WALL_NORTH, flag::WALL_NORTH_PROJ_BLOCKER);
        let south = m(flag::WALL_SOUTH, flag::WALL_SOUTH_PROJ_BLOCKER);
        match angle {
            loc_angle::WEST => {
                self.apply(x, z, y, north | west, add);
                self.apply(x - 1, z, y, east, add);
                self.apply(x, z + 1, y, south, add);
            }
            loc_angle::NORTH => {
                self.apply(x, z, y, north | east, add);
                self.apply(x, z + 1, y, south, add);
                self.apply(x + 1, z, y, west, add);
            }
            loc_angle::EAST => {
                self.apply(x, z, y, south | east, add);
                self.apply(x + 1, z, y, west, add);
                self.apply(x, z - 1, y, north, add);
            }
            loc_angle::SOUTH => {
                self.apply(x, z, y, south | west, add);
                self.apply(x, z - 1, y, north, add);
                self.apply(x - 1, z, y, east, add);
            }
            _ => {}
        }
        if blockrange {
            self.wall_l(x, z, y, angle, false, add);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pair(&mut self, x: i32, z: i32, y: i32, mask: u32, x2: i32, z2: i32, mask2: u32, add: bool) {
        self.apply(x, z, y, mask, add);
        self.apply(x2, z2, y, mask2, add);
    }
}
