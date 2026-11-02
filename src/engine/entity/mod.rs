//! `src/engine/entity/*` (so far: the map entities and NPCs; `Player`
//! lands with the World phase).

pub mod npc;
pub mod pathing;
pub mod player;

pub use npc::Npc;
pub use player::Player;

use crate::routefinder::loc_shape_layer;

/// `EntityLifeCycle`.
pub mod lifecycle {
    /// Never respawns or despawns.
    pub const FOREVER: i32 = 0;
    /// Added by the engine (map data); respawns later.
    pub const RESPAWN: i32 = 1;
    /// Added by a script; despawns later.
    pub const DESPAWN: i32 = 2;
}

/// `BlockWalk`.
pub mod block_walk {
    pub const NONE: i32 = 0;
    pub const NPC: i32 = 1;
    pub const ALL: i32 = 2;
    pub const PLAYER: i32 = 3;
}

/// `Entity` fields shared by every entity.
#[derive(Clone, Debug)]
pub struct EntityBase {
    pub level: i32,
    pub x: i32,
    pub z: i32,
    pub width: i32,
    pub length: i32,
    pub lifecycle: i32,
    pub is_active: bool,
    pub lifecycle_tick: i32,
    pub last_lifecycle_tick: i32,
}

impl EntityBase {
    pub fn new(level: i32, x: i32, z: i32, width: i32, length: i32, lifecycle: i32) -> Self {
        EntityBase { level, x, z, width, length, lifecycle, is_active: false, lifecycle_tick: -1, last_lifecycle_tick: -1 }
    }
}

/// `Loc`: type, shape and angle packed with the layer; `current` differs
/// from `base` while a script has changed the loc.
#[derive(Clone, Debug)]
pub struct Loc {
    pub e: EntityBase,
    base_info: i32,
    current_info: i32,
    /// The `LocObjEvent` tracking this loc's lifecycle, if any.
    pub event_tracker: Option<crate::engine::linklist::NodeId>,
}

fn pack_info(kind: i32, shape: i32, angle: i32) -> i32 {
    (kind & 0x3fff) | ((shape & 0x1f) << 14) | ((angle & 0x3) << 19) | ((loc_shape_layer(shape) & 0x3) << 21)
}

impl Loc {
    #[allow(clippy::too_many_arguments)]
    pub fn new(level: i32, x: i32, z: i32, width: i32, length: i32, lifecycle: i32, kind: i32, shape: i32, angle: i32) -> Self {
        let info = pack_info(kind, shape, angle);
        Loc { e: EntityBase::new(level, x, z, width, length, lifecycle), base_info: info, current_info: info, event_tracker: None }
    }
    pub fn kind(&self) -> i32 {
        self.current_info & 0x3fff
    }
    pub fn shape(&self) -> i32 {
        (self.current_info >> 14) & 0x1f
    }
    pub fn angle(&self) -> i32 {
        (self.current_info >> 19) & 0x3
    }
    /// `layer`: from the *base* info.
    pub fn layer(&self) -> i32 {
        (self.base_info >> 21) & 0x3
    }
    pub fn is_changed(&self) -> bool {
        self.current_info != self.base_info
    }
    pub fn change(&mut self, kind: i32, shape: i32, angle: i32) {
        self.current_info = pack_info(kind, shape, angle);
    }
    pub fn revert(&mut self) {
        self.current_info = self.base_info;
    }
}

/// `Obj.NO_RECEIVER`.
pub const NO_RECEIVER: i64 = -1;

/// `Obj`: a ground item.
#[derive(Clone, Debug)]
pub struct Obj {
    pub e: EntityBase,
    pub kind: i32,
    pub count: i64,
    pub receiver64: i64,
    pub reveal: i32,
    pub last_change: i32,
    /// The `LocObjEvent` tracking this obj's lifecycle, if any.
    pub event_tracker: Option<crate::engine::linklist::NodeId>,
}

impl Obj {
    pub const REVEAL: i32 = 100;

    pub fn new(level: i32, x: i32, z: i32, lifecycle: i32, kind: i32, count: i64) -> Self {
        Obj { e: EntityBase::new(level, x, z, 1, 1, lifecycle), kind, count, receiver64: NO_RECEIVER, reveal: -1, last_change: -1, event_tracker: None }
    }

    /// `isValid(hash64)`: hidden from everyone but its receiver until revealed.
    pub fn is_valid(&self, hash64: Option<i64>) -> bool {
        if self.reveal > -1 && hash64.is_some_and(|h| h != 0 && h != self.receiver64) {
            return false;
        }
        self.count >= 1 && self.e.is_active
    }
}

/// `EntityList` for NPCs: ids map to storage slots; iteration is by id.
/// `next()` scans from the last used id + 1 and wraps to `padding`, so the
/// first NPC gets id 1. Boxed so the per-tick id scans walk pointers, not
/// 16k inline NPCs.
pub struct NpcList {
    slots: Vec<Option<Box<Npc>>>,
    last_used: usize,
    padding: usize,
}

impl NpcList {
    pub fn new(size: usize) -> Self {
        NpcList { slots: (0..size).map(|_| None).collect(), last_used: 0, padding: 0 }
    }

    /// `next()`: the next free id; panics when full.
    pub fn next_id(&self) -> usize {
        let start = self.last_used + 1;
        (start..self.slots.len()).chain(self.padding..start.min(self.slots.len())).find(|&i| self.slots[i].is_none()).expect("[EntityList] no space for new entities")
    }

    pub fn set(&mut self, id: usize, npc: Npc) {
        assert!(id < self.slots.len() && self.slots[id].is_none(), "[EntityList] invalid or occupied id {id}");
        self.slots[id] = Some(Box::new(npc));
        self.last_used = id;
    }

    pub fn get(&self, id: usize) -> Option<&Npc> {
        self.slots.get(id)?.as_deref()
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Npc> {
        self.slots.get_mut(id)?.as_deref_mut()
    }

    /// Removes and returns the NPC (`remove` plus ownership).
    pub fn take(&mut self, id: usize) -> Option<Npc> {
        self.slots.get_mut(id)?.take().map(|n| *n)
    }

    /// The id range (`ids.length`).
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn remove(&mut self, id: usize) {
        if let Some(s) = self.slots.get_mut(id) {
            *s = None;
        }
    }

    pub fn count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// NPCs in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Npc> {
        self.slots.iter().filter_map(Option::as_deref)
    }
}
