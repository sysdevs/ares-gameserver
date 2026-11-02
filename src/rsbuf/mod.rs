//! Player and NPC info (the per-tick entity update packets): one [`Rsbuf`]
//! value holding the state.
//!
//! Order matters for the bytes (B.13): a zone's players/NPCs are a JS `Set`
//! (insertion order; delete then add moves an id to the end), so they are
//! `IndexSet`s with `shift_remove`; a tile's player list in the grid is in
//! `computePlayer` call order; a build area's ids keep insertion order.

mod build;
mod info;
mod renderer;

pub use build::BuildArea;

use indexmap::IndexSet;
use info::{NpcInfoEncoder, PlayerInfoEncoder};
use renderer::{NpcRenderer, PlayerRenderer};
use std::collections::HashMap;

/// `Visibility`.
pub mod visibility {
    pub const DEFAULT: i32 = 0;
    pub const SOFT: i32 = 1;
    pub const HARD: i32 = 2;
}

/// `PlayerInfoProt` masks.
pub mod player_prot {
    pub const APPEARANCE: u32 = 0x1;
    pub const ANIM: u32 = 0x2;
    pub const FACE_ENTITY: u32 = 0x4;
    pub const SAY: u32 = 0x8;
    pub const DAMAGE: u32 = 0x10;
    pub const FACE_COORD: u32 = 0x20;
    pub const CHAT: u32 = 0x40;
    pub const BIG: u32 = 0x80;
    pub const SPOT_ANIM: u32 = 0x100;
    pub const EXACT_MOVE: u32 = 0x200;
    pub const DAMAGE2: u32 = 0x400;
}

/// `NpcInfoProt` masks.
pub mod npc_prot {
    pub const DAMAGE2: u32 = 0x1;
    pub const ANIM: u32 = 0x2;
    pub const FACE_ENTITY: u32 = 0x4;
    pub const SAY: u32 = 0x8;
    pub const DAMAGE: u32 = 0x10;
    pub const CHANGE_TYPE: u32 = 0x20;
    pub const SPOT_ANIM: u32 = 0x40;
    pub const FACE_COORD: u32 = 0x80;
}

/// `CoordGrid` (rsbuf's own): `z | x << 14 | y << 28`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coord(pub u32);

impl Coord {
    pub fn from(x: i32, y: i32, z: i32) -> Self {
        Coord(((z & 0x3fff) | ((x & 0x3fff) << 14) | ((y & 0x3) << 28)) as u32)
    }
    pub fn x(self) -> i32 {
        ((self.0 >> 14) & 0x3fff) as i32
    }
    pub fn y(self) -> i32 {
        ((self.0 >> 28) & 0x3) as i32
    }
    pub fn z(self) -> i32 {
        (self.0 & 0x3fff) as i32
    }
    fn within_distance_sw(self, other: Coord, distance: i32) -> bool {
        !((self.x() - other.x()).abs() > distance || (self.z() - other.z()).abs() > distance)
    }
}

fn fine(pos: i32, size: i32) -> i32 {
    pos * 2 + size
}

#[derive(Default)]
struct Zone {
    players: IndexSet<i32>,
    npcs: IndexSet<i32>,
}

#[derive(Default)]
pub(crate) struct ZoneMap {
    zones: HashMap<u32, Zone>,
}

impl ZoneMap {
    fn zone(&mut self, x: i32, y: i32, z: i32) -> &mut Zone {
        let index = (((x >> 3) & 0x7ff) | (((z >> 3) & 0x7ff) << 11) | ((y & 0x3) << 22)) as u32;
        self.zones.entry(index).or_default()
    }
}

/// Masks every write instead of throwing (except past the
/// 5000-byte buffer, where the DataView would throw and this panics).
pub(crate) struct Packet {
    pub data: Vec<u8>,
    pub pos: usize,
    pub bit_pos: usize,
}

impl Packet {
    fn new(len: usize) -> Self {
        Packet { data: vec![0; len], pos: 0, bit_pos: 0 }
    }
    pub fn p1(&mut self, v: i32) {
        self.data[self.pos] = v as u8;
        self.pos += 1;
    }
    pub fn p2(&mut self, v: i32) {
        self.data[self.pos..self.pos + 2].copy_from_slice(&(v as u16).to_be_bytes());
        self.pos += 2;
    }
    pub fn ip2(&mut self, v: i32) {
        self.data[self.pos..self.pos + 2].copy_from_slice(&(v as u16).to_le_bytes());
        self.pos += 2;
    }
    pub fn p4(&mut self, v: i32) {
        self.data[self.pos..self.pos + 4].copy_from_slice(&v.to_be_bytes());
        self.pos += 4;
    }
    /// `pjstr`: the low byte of each UTF-16 unit, then the terminator.
    pub fn pjstr(&mut self, s: &str, terminator: u8) {
        for u in s.encode_utf16() {
            self.data[self.pos] = u as u8;
            self.pos += 1;
        }
        self.data[self.pos] = terminator;
        self.pos += 1;
    }
    pub fn pdata(&mut self, src: &[u8]) {
        self.data[self.pos..self.pos + src.len()].copy_from_slice(src);
        self.pos += src.len();
    }
    fn bits(&mut self) {
        self.bit_pos = self.pos << 3;
    }
    fn bytes(&mut self) {
        self.pos = (self.bit_pos + 7) >> 3;
    }
    pub fn pbit(&mut self, mut n: u32, value: i32) {
        let pos = self.bit_pos;
        self.bit_pos += n as usize;
        let mut byte_pos = pos >> 3;
        let mut remaining = 8 - (pos & 7) as u32;
        while n > remaining {
            let mask = (1i32 << remaining) - 1;
            let b = self.data[byte_pos] as i32;
            self.data[byte_pos] = ((b & !mask) | ((value >> (n - remaining)) & mask)) as u8;
            byte_pos += 1;
            n -= remaining;
            remaining = 8;
        }
        let r = remaining - n;
        let mask = (1i32 << n) - 1;
        let b = self.data[byte_pos] as i32;
        self.data[byte_pos] = ((b & (!mask << r)) | ((value & mask) << r)) as u8;
    }
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub bytes: Vec<u8>,
    pub color: i32,
    pub effect: i32,
    pub ignored: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ExactMove {
    pub start_x: i32,
    pub start_z: i32,
    pub end_x: i32,
    pub end_z: i32,
    pub begin: i32,
    pub finish: i32,
    pub dir: i32,
}

/// The per-tick player inputs of `computePlayer`.
#[derive(Clone, Debug)]
pub struct PlayerUpdate {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub origin_x: i32,
    pub origin_z: i32,
    pub tele: bool,
    pub jump: bool,
    pub run_dir: i32,
    pub walk_dir: i32,
    pub visibility: i32,
    pub active: bool,
    pub masks: u32,
    pub appearance: Vec<u8>,
    pub last_appearance: i32,
    pub face_entity: i32,
    pub face_x: i32,
    pub face_z: i32,
    pub orientation_x: i32,
    pub orientation_z: i32,
    pub damage_taken: i32,
    pub damage_type: i32,
    pub damage_taken2: i32,
    pub damage_type2: i32,
    pub current_hitpoints: i32,
    pub base_hitpoints: i32,
    pub anim_id: i32,
    pub anim_delay: i32,
    pub say: Option<String>,
    pub chat: Option<Chat>,
    pub graphic_id: i32,
    pub graphic_height: i32,
    pub graphic_delay: i32,
    pub exact_move: Option<ExactMove>,
}

/// The per-tick NPC inputs of `computeNpc`.
#[derive(Clone, Debug)]
pub struct NpcUpdate {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub ntype: i32,
    pub tele: bool,
    pub jump: bool,
    pub run_dir: i32,
    pub walk_dir: i32,
    pub active: bool,
    pub masks: u32,
    pub face_entity: i32,
    pub face_x: i32,
    pub face_z: i32,
    pub orientation_x: i32,
    pub orientation_z: i32,
    pub damage_taken: i32,
    pub damage_type: i32,
    pub damage_taken2: i32,
    pub damage_type2: i32,
    pub current_hitpoints: i32,
    pub base_hitpoints: i32,
    pub anim_id: i32,
    pub anim_delay: i32,
    pub say: Option<String>,
    pub graphic_id: i32,
    pub graphic_height: i32,
    pub graphic_delay: i32,
}

pub(crate) struct Player {
    pid: i32,
    coord: Coord,
    origin: Coord,
    u: PlayerUpdate,
    build: BuildArea,
}

impl Player {
    fn new(pid: i32) -> Self {
        Player { pid, coord: Coord::default(), origin: Coord::default(), u: PlayerUpdate::cleared(), build: BuildArea::default() }
    }
}

pub(crate) struct Npc {
    nid: i32,
    coord: Coord,
    u: NpcUpdate,
    observers: i32,
}

impl PlayerUpdate {
    /// The `Player` field defaults, which `cleanup()` also restores (all but
    /// the position, visibility, active flag, appearance and orientation).
    fn cleared() -> Self {
        PlayerUpdate {
            x: 0,
            y: 0,
            z: 0,
            origin_x: 0,
            origin_z: 0,
            tele: false,
            jump: false,
            run_dir: -1,
            walk_dir: -1,
            visibility: visibility::DEFAULT,
            active: false,
            masks: 0,
            appearance: Vec::new(),
            last_appearance: -1,
            face_entity: -1,
            face_x: -1,
            face_z: -1,
            orientation_x: -1,
            orientation_z: -1,
            damage_taken: -1,
            damage_type: -1,
            damage_taken2: -1,
            damage_type2: -1,
            current_hitpoints: -1,
            base_hitpoints: -1,
            anim_id: -1,
            anim_delay: -1,
            say: None,
            chat: None,
            graphic_id: -1,
            graphic_height: -1,
            graphic_delay: -1,
            exact_move: None,
        }
    }

    fn cleanup(&mut self) {
        let keep = std::mem::replace(self, PlayerUpdate::cleared());
        (self.visibility, self.active, self.appearance, self.last_appearance, self.face_entity, self.orientation_x, self.orientation_z) =
            (keep.visibility, keep.active, keep.appearance, keep.last_appearance, keep.face_entity, keep.orientation_x, keep.orientation_z);
    }
}

impl NpcUpdate {
    fn cleared(ntype: i32) -> Self {
        NpcUpdate {
            x: 0,
            y: 0,
            z: 0,
            ntype,
            tele: false,
            jump: false,
            run_dir: -1,
            walk_dir: -1,
            active: false,
            masks: 0,
            face_entity: -1,
            face_x: -1,
            face_z: -1,
            orientation_x: -1,
            orientation_z: -1,
            damage_taken: -1,
            damage_type: -1,
            damage_taken2: -1,
            damage_type2: -1,
            current_hitpoints: -1,
            base_hitpoints: -1,
            anim_id: -1,
            anim_delay: -1,
            say: None,
            graphic_id: -1,
            graphic_height: -1,
            graphic_delay: -1,
        }
    }

    fn cleanup(&mut self) {
        let keep = std::mem::replace(self, NpcUpdate::cleared(self.ntype));
        (self.active, self.face_entity, self.orientation_x, self.orientation_z) = (keep.active, keep.face_entity, keep.orientation_x, keep.orientation_z);
    }
}

pub struct Rsbuf {
    players: Vec<Option<Player>>,
    player_grid: HashMap<u32, Vec<i32>>,
    player_renderer: PlayerRenderer,
    player_info: PlayerInfoEncoder,
    npcs: Vec<Option<Npc>>,
    npc_renderer: NpcRenderer,
    npc_info: NpcInfoEncoder,
    zone_map: ZoneMap,
}

impl Default for Rsbuf {
    fn default() -> Self {
        Rsbuf {
            players: (0..2048).map(|_| None).collect(),
            player_grid: HashMap::new(),
            player_renderer: PlayerRenderer::default(),
            player_info: PlayerInfoEncoder::default(),
            npcs: (0..16384).map(|_| None).collect(),
            npc_renderer: NpcRenderer::default(),
            npc_info: NpcInfoEncoder::default(),
            zone_map: ZoneMap::default(),
        }
    }
}

fn changed_zone(a: Coord, b: Coord) -> bool {
    a != b && (a.x() >> 3 != b.x() >> 3 || a.z() >> 3 != b.z() >> 3 || a.y() != b.y())
}

impl Rsbuf {
    pub fn add_player(&mut self, pid: i32) {
        if pid != -1 {
            self.players[pid as usize] = Some(Player::new(pid));
        }
    }

    pub fn remove_player(&mut self, pid: i32) {
        if pid == -1 {
            return;
        }
        if let Some(p) = &mut self.players[pid as usize] {
            self.zone_map.zone(p.coord.x(), p.coord.y(), p.coord.z()).players.shift_remove(&pid);
            for nid in p.build.npcs.iter() {
                if let Some(n) = &mut self.npcs[nid as usize] {
                    n.observers = (n.observers - 1).max(0);
                }
            }
            p.build.cleanup();
        }
        self.player_renderer.remove_permanent(pid);
        self.players[pid as usize] = None;
    }

    /// `computePlayer`.
    pub fn compute_player(&mut self, pid: i32, u: PlayerUpdate) {
        if pid == -1 {
            return;
        }
        let Some(p) = &mut self.players[pid as usize] else { return };
        let coord = Coord::from(u.x, u.y, u.z);
        if changed_zone(coord, p.coord) {
            self.zone_map.zone(p.coord.x(), p.coord.y(), p.coord.z()).players.shift_remove(&pid);
            self.zone_map.zone(coord.x(), coord.y(), coord.z()).players.insert(pid);
        }
        p.coord = coord;
        p.origin = Coord::from(u.origin_x, u.y, u.origin_z);
        p.u = u;
        self.player_renderer.compute_info(p);
        self.player_grid.entry(coord.0).or_default().push(pid);
    }

    /// `playerInfo(pos, pid, dx, dz, rebuild)`.
    pub fn player_info(&mut self, pos: i32, pid: i32, dx: i32, dz: i32, rebuild: bool) -> Vec<u8> {
        if pid == -1 || self.players[pid as usize].is_none() {
            return Vec::new();
        }
        self.player_info.encode(pos, &mut self.player_renderer, &mut self.players, &mut self.zone_map, &self.player_grid, pid, dx, dz, rebuild)
    }

    pub fn has_player(&self, pid: i32, other: i32) -> bool {
        pid != -1 && other != -1 && self.players[pid as usize].as_ref().is_some_and(|p| p.build.players.contains(other))
    }

    pub fn add_npc(&mut self, nid: i32, ntype: i32) {
        if nid != -1 && ntype != -1 {
            self.npcs[nid as usize] = Some(Npc { nid, coord: Coord::default(), u: NpcUpdate::cleared(ntype), observers: 0 });
        }
    }

    pub fn remove_npc(&mut self, nid: i32) {
        if nid == -1 {
            return;
        }
        if let Some(n) = &self.npcs[nid as usize] {
            self.zone_map.zone(n.coord.x(), n.coord.y(), n.coord.z()).npcs.shift_remove(&nid);
        }
        self.npc_renderer.remove_permanent(nid);
        self.npcs[nid as usize] = None;
    }

    /// `computeNpc`.
    pub fn compute_npc(&mut self, nid: i32, u: NpcUpdate) {
        if nid == -1 || u.ntype == -1 {
            return;
        }
        let Some(n) = &mut self.npcs[nid as usize] else { return };
        let coord = Coord::from(u.x, u.y, u.z);
        if changed_zone(coord, n.coord) {
            self.zone_map.zone(n.coord.x(), n.coord.y(), n.coord.z()).npcs.shift_remove(&nid);
            self.zone_map.zone(coord.x(), coord.y(), coord.z()).npcs.insert(nid);
        }
        n.coord = coord;
        n.u = u;
        self.npc_renderer.compute_info(n);
    }

    /// `npcInfo(pos, pid, dx, dz, rebuild)`.
    pub fn npc_info(&mut self, pos: i32, pid: i32, dx: i32, dz: i32, rebuild: bool) -> Vec<u8> {
        if pid == -1 {
            return Vec::new();
        }
        let Some(player) = &mut self.players[pid as usize] else { return Vec::new() };
        self.npc_info.encode(pos, &mut self.npc_renderer, &mut self.npcs, &mut self.zone_map, player, dx, dz, rebuild)
    }

    pub fn has_npc(&self, pid: i32, nid: i32) -> bool {
        pid != -1 && nid != -1 && self.players[pid as usize].as_ref().is_some_and(|p| p.build.npcs.contains(nid))
    }

    pub fn npc_observers(&self, nid: i32) -> i32 {
        if nid == -1 { 0 } else { self.npcs[nid as usize].as_ref().map_or(0, |n| n.observers) }
    }

    /// `cleanup()`: end of tick.
    pub fn cleanup(&mut self) {
        self.player_grid.clear();
        self.player_renderer.remove_temporary();
        self.npc_renderer.remove_temporary();
        for p in self.players.iter_mut().flatten() {
            p.u.cleanup();
        }
        for n in self.npcs.iter_mut().flatten() {
            n.u.cleanup();
        }
    }

    pub fn cleanup_player_build_area(&mut self, pid: i32) {
        if pid != -1
            && let Some(p) = &mut self.players[pid as usize]
        {
            p.build.cleanup();
        }
    }
}
