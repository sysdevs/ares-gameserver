//! Resumable walks over zone entity lists.
//!
//! An iterator is created by one op and advanced by later ones
//! (`npc_findnext`), possibly after the lists changed. [`ZoneIter`] keeps its
//! position (zone and list cursor) and re-reads the lists on every step, so
//! changes between steps are seen.
//! Zones are walked from the highest x, then highest z, down; each list is
//! walked tail first, skipping invalid entities.

use crate::engine::coord;
use crate::engine::entity::npc::{hunt_mode, hunt_vis};
use crate::engine::linklist::Cursor;
use crate::engine::world::World;
use crate::engine::zone::Member;
use crate::script::state::EntityRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    Players,
    Npcs,
    Locs,
    Objs,
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Every valid entity of one zone (NPC zone and loc/obj iterators).
    Zone,
    /// NPCs within a distance, optionally of one NPC type.
    Distance { npc_type: Option<i32> },
    /// NPCs with a second op.
    HuntAll,
    /// NPCs, objs or locs of a type/category.
    Hunt { check_type: i32, check_category: i32 },
    /// Player hunts: line of sight is
    /// traced from the player back to the centre.
    Players,
}

#[derive(Clone, Debug)]
pub struct ZoneIter {
    tick: i32,
    level: i32,
    x: i32,
    z: i32,
    distance: i32,
    vis: i32,
    walk: Walk,
    mode: Mode,
    label: &'static str,
    min_x: i32,
    min_z: i32,
    max_z: i32,
    cx: i32,
    cz: i32,
    cursor: Option<Cursor>,
    done: bool,
}

impl ZoneIter {
    #[allow(clippy::too_many_arguments)]
    fn around(tick: i32, level: i32, x: i32, z: i32, distance: i32, vis: i32, walk: Walk, mode: Mode, label: &'static str) -> Self {
        let radius = (1.0 + distance as f64 / 8.0) as i32;
        let (zx, zz) = (coord::zone(x), coord::zone(z));
        ZoneIter { tick, level, x, z, distance, vis, walk, mode, label, min_x: zx - radius, min_z: zz - radius, max_z: zz + radius, cx: zx + radius, cz: zz + radius, cursor: None, done: false }
    }

    /// One zone, given a tile in it.
    fn zone(tick: i32, level: i32, x: i32, z: i32, walk: Walk, label: &'static str) -> Self {
        let mut it = Self::around(tick, level, x, z, 0, hunt_vis::OFF, walk, Mode::Zone, label);
        (it.min_x, it.min_z, it.max_z, it.cx, it.cz) = (0, 0, 0, 0, 0);
        it
    }

    pub fn npc_zone(tick: i32, level: i32, x: i32, z: i32) -> Self {
        Self::zone(tick, level, x, z, Walk::Npcs, "NpcIterator")
    }

    pub fn npc_distance(tick: i32, level: i32, x: i32, z: i32, distance: i32, vis: i32, npc_type: Option<i32>) -> Self {
        Self::around(tick, level, x, z, distance, vis, Walk::Npcs, Mode::Distance { npc_type }, "NpcIterator")
    }

    pub fn npc_hunt_all(tick: i32, level: i32, x: i32, z: i32, distance: i32, vis: i32) -> Self {
        Self::around(tick, level, x, z, distance, vis, Walk::Npcs, Mode::HuntAll, "HuntIterator")
    }

    pub fn player_hunt_all(tick: i32, level: i32, x: i32, z: i32, distance: i32, vis: i32) -> Self {
        Self::around(tick, level, x, z, distance, vis, Walk::Players, Mode::Players, "HuntIterator")
    }

    pub fn locs(tick: i32, level: i32, x: i32, z: i32) -> Self {
        Self::zone(tick, level, x, z, Walk::Locs, "LocIterator")
    }

    pub fn objs(tick: i32, level: i32, x: i32, z: i32) -> Self {
        Self::zone(tick, level, x, z, Walk::Objs, "ObjIterator")
    }

    /// The next valid entity of the walk, before the mode's filters.
    fn next_valid(&mut self, w: &mut World) -> Option<EntityRef> {
        loop {
            if self.cursor.is_none() {
                if self.done {
                    return None;
                }
                let (tx, tz) = if matches!(self.mode, Mode::Zone) { (self.x, self.z) } else { (self.cx << 3, self.cz << 3) };
                let zone = w.zones.zone(tx, tz, self.level, &mut w.arenas);
                self.cursor = Some(match self.walk {
                    Walk::Players => w.arenas.members.iter(zone.players, true),
                    Walk::Npcs => w.arenas.members.iter(zone.npcs, true),
                    Walk::Locs => w.arenas.locs.iter(zone.locs, true),
                    Walk::Objs => w.arenas.objs.iter(zone.objs, true),
                });
            }
            let c = self.cursor.as_mut().unwrap();
            let node = match self.walk {
                Walk::Players | Walk::Npcs => w.arenas.members.next(c),
                Walk::Locs => w.arenas.locs.next(c),
                Walk::Objs => w.arenas.objs.next(c),
            };
            let Some(node) = node else {
                self.cursor = None;
                self.advance();
                continue;
            };
            let e = match self.walk {
                Walk::Players => match w.arenas.members.get(node) {
                    Some(Member::Player(p)) => EntityRef::Player(*p),
                    _ => continue,
                },
                Walk::Npcs => match w.arenas.members.get(node) {
                    Some(Member::Npc(nid)) => EntityRef::Npc(*nid),
                    _ => continue,
                },
                Walk::Locs => EntityRef::Loc(node),
                Walk::Objs => EntityRef::Obj(node),
            };
            let valid = match e {
                EntityRef::Player(p) => w.player_is_valid(p),
                _ => w.entity_valid(e),
            };
            if valid {
                return Some(e);
            }
        }
    }

    fn advance(&mut self) {
        if matches!(self.mode, Mode::Zone) {
            self.done = true;
            return;
        }
        self.cz -= 1;
        if self.cz < self.min_z {
            self.cz = self.max_z;
            self.cx -= 1;
            if self.cx < self.min_x {
                self.done = true;
            }
        }
    }

    fn vis_ok(&self, w: &World, tx: i32, tz: i32) -> bool {
        match self.vis {
            hunt_vis::LINEOFSIGHT => w.is_line_of_sight(self.level, self.x, self.z, tx, tz),
            hunt_vis::LINEOFWALK => w.is_line_of_walk(self.level, self.x, self.z, tx, tz),
            _ => true,
        }
    }

    /// `next()`: `Err` when used on a later tick than it was made on.
    pub fn next(&mut self, w: &mut World) -> Result<Option<EntityRef>, String> {
        while let Some(e) = self.next_valid(w) {
            if w.current_tick > self.tick {
                return Err(format!("[{}] tried to use an old iterator. Create a new iterator instead.", self.label));
            }
            if matches!(self.mode, Mode::Zone) {
                return Ok(Some(e));
            }
            let kind = w.entity_type(e);
            if let Mode::Hunt { check_type, check_category } = self.mode {
                if check_type != -1 && kind != check_type {
                    continue;
                }
                let category = match e {
                    EntityRef::Npc(_) => w.configs.npcs.get(kind as usize).map(|t| t.category),
                    EntityRef::Obj(_) => w.configs.objs.get(kind as usize).map(|t| t.category),
                    EntityRef::Loc(_) => w.configs.locs.get(kind as usize).map(|t| t.category),
                    EntityRef::Player(_) => None,
                };
                if check_category != -1 && category != Some(check_category) {
                    continue;
                }
            }
            if let Mode::HuntAll = self.mode {
                let has_op2 = w.configs.npcs.get(kind as usize).and_then(|t| t.op.as_ref()).and_then(|ops| ops.get(1).cloned().flatten()).is_some_and(|op| !op.is_empty());
                if !has_op2 {
                    continue;
                }
            }
            let Some((_, (tx, tz, _, _))) = w.entity_area(e) else { continue };
            if coord::distance_to_sw(self.x, self.z, tx, tz) > self.distance {
                continue;
            }
            let visible = match self.mode {
                Mode::Players => match self.vis {
                    hunt_vis::LINEOFSIGHT => w.is_line_of_sight(self.level, tx, tz, self.x, self.z),
                    hunt_vis::LINEOFWALK => w.is_line_of_walk(self.level, tx, tz, self.x, self.z),
                    _ => true,
                },
                _ => self.vis_ok(w, tx, tz),
            };
            if !visible {
                continue;
            }
            if let Mode::Distance { npc_type: Some(t) } = self.mode
                && kind != t
            {
                continue;
            }
            return Ok(Some(e));
        }
        Ok(None)
    }
}

/// `Array.from(new HuntIterator(...))` for NPC, obj and loc hunts.
#[allow(clippy::too_many_arguments)]
pub fn hunt(w: &mut World, level: i32, x: i32, z: i32, distance: i32, vis: i32, check_type: i32, check_category: i32, kind: i32) -> Result<Vec<EntityRef>, String> {
    let walk = match kind {
        hunt_mode::PLAYER => {
            let mut it = ZoneIter::around(w.current_tick, level, x, z, distance, vis, Walk::Players, Mode::Players, "HuntIterator");
            let mut out = Vec::new();
            while let Some(e) = it.next(w)? {
                out.push(e);
            }
            return Ok(out);
        }
        hunt_mode::NPC => Walk::Npcs,
        hunt_mode::OBJ => Walk::Objs,
        hunt_mode::SCENERY => Walk::Locs,
        _ => return Ok(Vec::new()),
    };
    let mut it = ZoneIter::around(w.current_tick, level, x, z, distance, vis, walk, Mode::Hunt { check_type, check_category }, "HuntIterator");
    let mut out = Vec::new();
    while let Some(e) = it.next(w)? {
        out.push(e);
    }
    Ok(out)
}
