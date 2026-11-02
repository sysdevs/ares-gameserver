//! A player's view of other entities.

use super::{Coord, Npc, Player, ZoneMap};
use std::collections::HashMap;

pub const INTERVAL: i32 = 10;
pub const PREFERRED_PLAYERS: usize = 250;
pub const PREFERRED_NPCS: usize = 255;
pub const PREFERRED_VIEW_DISTANCE: i32 = 15;

/// `IdBitSet`: membership bits plus the ids in insertion order.
pub struct IdBitSet {
    bits: Vec<i32>,
    ids: Vec<i32>,
}

impl IdBitSet {
    fn new(length: usize) -> Self {
        IdBitSet { bits: vec![0; length / 32], ids: Vec::new() }
    }

    pub fn contains(&self, id: i32) -> bool {
        self.bits[(id >> 5) as usize] & (1 << (id & 0x1f)) != 0
    }

    pub fn insert(&mut self, id: i32) {
        if !self.contains(id) {
            self.bits[(id >> 5) as usize] |= 1 << (id & 0x1f);
            self.ids.push(id);
        }
    }

    pub fn remove(&mut self, id: i32) {
        if self.contains(id) {
            self.bits[(id >> 5) as usize] &= !(1 << (id & 0x1f));
            if let Some(i) = self.ids.iter().position(|&x| x == id) {
                self.ids.remove(i);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// `iter()`: a snapshot, so removing while iterating is safe.
    pub fn iter(&self) -> Vec<i32> {
        self.ids.clone()
    }

    fn clear(&mut self) {
        self.bits.fill(0);
        self.ids.clear();
    }
}

pub struct BuildArea {
    pub players: IdBitSet,
    pub npcs: IdBitSet,
    appearances: Vec<u32>,
    pub force_view_distance: bool,
    pub view_distance: i32,
    pub last_resize: i32,
}

impl Default for BuildArea {
    fn default() -> Self {
        BuildArea {
            players: IdBitSet::new(2048),
            npcs: IdBitSet::new(16384),
            appearances: vec![0; 2048],
            force_view_distance: false,
            view_distance: PREFERRED_VIEW_DISTANCE,
            last_resize: 0,
        }
    }
}

impl BuildArea {
    pub fn cleanup(&mut self) {
        self.players.clear();
        self.npcs.clear();
        self.appearances.fill(0);
    }

    pub(crate) fn resize(&mut self) {
        if self.force_view_distance {
            return;
        }
        if self.players.len() >= PREFERRED_PLAYERS {
            if self.view_distance > 1 {
                self.view_distance -= 1;
            }
            self.last_resize = 0;
            return;
        }
        self.last_resize += 1;
        if self.last_resize >= INTERVAL {
            if self.view_distance < PREFERRED_VIEW_DISTANCE {
                self.view_distance += 1;
            } else {
                self.last_resize = 0;
            }
        }
    }

    pub(crate) fn rebuild_npcs(&mut self) {
        self.npcs.clear();
    }

    pub(crate) fn rebuild_players(&mut self, players: &[Option<Player>], grid: &HashMap<u32, Vec<i32>>, pid: i32, x: i32, y: i32, z: i32) {
        self.players.clear();
        self.last_resize = 0;
        self.view_distance = PREFERRED_VIEW_DISTANCE;
        // the nearest-first scan stops at PREFERRED_PLAYERS, so reaching the
        // cap means the area is crowded
        if self.nearby_players_nearest(players, grid, pid, x, y, z).len() >= PREFERRED_PLAYERS {
            self.view_distance -= 1;
        }
    }

    pub(crate) fn has_appearance(&self, pid: i32, tick: u32) -> bool {
        self.appearances[pid as usize] == tick
    }

    pub(crate) fn save_appearance(&mut self, pid: i32, tick: u32) {
        self.appearances[pid as usize] = tick;
    }

    pub(crate) fn nearby_players(&self, players: &[Option<Player>], grid: &HashMap<u32, Vec<i32>>, map: &mut ZoneMap, pid: i32, x: i32, y: i32, z: i32) -> Vec<i32> {
        if self.view_distance < PREFERRED_VIEW_DISTANCE {
            self.nearby_players_nearest(players, grid, pid, x, y, z)
        } else {
            self.nearby_players_zones(players, map, pid, x, y, z)
        }
    }

    fn nearby_players_zones(&self, players: &[Option<Player>], map: &mut ZoneMap, pid: i32, x: i32, y: i32, z: i32) -> Vec<i32> {
        let d = self.view_distance;
        let (start_x, start_z) = ((x - d).max(0) >> 3, (z - d).max(0) >> 3);
        let (end_x, end_z) = ((x + d).min(0xffff) >> 3, (z + d).min(0xffff) >> 3);
        let count = self.players.len();
        let mut nearby = Vec::new();
        for zx in start_x..=end_x {
            for zz in start_z..=end_z {
                if nearby.len() + count >= PREFERRED_PLAYERS {
                    return nearby;
                }
                for &other in &map.zone(zx << 3, y, zz << 3).players {
                    if nearby.len() >= PREFERRED_PLAYERS - count {
                        return nearby;
                    }
                    if self.filter_player(players, other, pid, x, y, z) {
                        nearby.push(other);
                    }
                }
            }
        }
        nearby
    }

    /// `getNearbyPlayersNearest`: an outward square spiral over the tile grid.
    fn nearby_players_nearest(&self, players: &[Option<Player>], grid: &HashMap<u32, Vec<i32>>, pid: i32, x: i32, y: i32, z: i32) -> Vec<i32> {
        let radius = self.view_distance * 2;
        let (min, max) = (-(radius >> 1), radius >> 1);
        let (mut dx, mut dz, mut ldx, mut ldz) = (0, 0, 0, -1);
        let count = self.players.len();
        let mut nearby = Vec::new();
        for _ in 1..=radius * radius {
            if nearby.len() + count >= PREFERRED_PLAYERS {
                return nearby;
            }
            if min < dx && dx <= max && min < dz && dz <= max
                && let Some(set) = grid.get(&Coord::from(x + dx, y, z + dz).0)
            {
                for &other in set {
                    if nearby.len() >= PREFERRED_PLAYERS - count {
                        return nearby;
                    }
                    if self.filter_player(players, other, pid, x, y, z) {
                        nearby.push(other);
                    }
                }
            }
            if dx == dz || (dx < 0 && dx == -dz) || (dx > 0 && dx == 1 - dz) {
                let next = -ldz;
                ldz = ldx;
                ldx = next;
            }
            dx += ldx;
            dz += ldz;
        }
        nearby
    }

    pub(crate) fn nearby_npcs(&self, npcs: &[Option<Npc>], map: &mut ZoneMap, x: i32, y: i32, z: i32) -> Vec<i32> {
        let d = PREFERRED_VIEW_DISTANCE;
        let (start_x, start_z) = ((x - d).max(0) >> 3, (z - d).max(0) >> 3);
        let (end_x, end_z) = ((x + d).min(0xffff) >> 3, (z + d).min(0xffff) >> 3);
        let count = self.npcs.len();
        let mut nearby = Vec::new();
        for zx in start_x..=end_x {
            for zz in start_z..=end_z {
                if nearby.len() + count >= PREFERRED_NPCS {
                    return nearby;
                }
                for &other in &map.zone(zx << 3, y, zz << 3).npcs {
                    if nearby.len() >= PREFERRED_NPCS - count {
                        return nearby;
                    }
                    if self.filter_npc(npcs, other, x, y, z) {
                        nearby.push(other);
                    }
                }
            }
        }
        nearby
    }

    fn filter_player(&self, players: &[Option<Player>], id: i32, pid: i32, x: i32, y: i32, z: i32) -> bool {
        let Some(other) = &players[id as usize] else { return false };
        !(self.players.contains(id) || !other.coord.within_distance_sw(Coord::from(x, y, z), self.view_distance) || other.pid == -1 || other.pid == pid || other.coord.y() != y)
    }

    fn filter_npc(&self, npcs: &[Option<Npc>], id: i32, x: i32, y: i32, z: i32) -> bool {
        let Some(other) = &npcs[id as usize] else { return false };
        !(self.npcs.contains(id) || !other.coord.within_distance_sw(Coord::from(x, y, z), PREFERRED_VIEW_DISTANCE) || other.nid == -1 || other.coord.y() != y || !other.u.active)
    }
}
