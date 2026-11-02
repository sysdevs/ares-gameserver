//! NPC state, hero points, NPC queue requests and the NPC enums.
//! The per-tick behaviour (`turn` and the AI modes) is in `world/npc.rs`.

use super::EntityBase;
use super::pathing::{Pathing, move_strategy};
use crate::cache::config::npc::NpcType;
use crate::engine::linklist::{Links, List};
use crate::rsbuf::npc_prot;
use crate::script::state::ScriptState;
use crate::util::numbers::quicksort;

/// `NpcMode`.
pub mod npc_mode {
    pub const NULL: i32 = -1;
    pub const NONE: i32 = 0;
    pub const WANDER: i32 = 1;
    pub const PATROL: i32 = 2;
    pub const PLAYERESCAPE: i32 = 3;
    pub const PLAYERFOLLOW: i32 = 4;
    pub const PLAYERFACE: i32 = 5;
    pub const PLAYERFACECLOSE: i32 = 6;
    pub const OPPLAYER1: i32 = 7;
    pub const OPPLAYER5: i32 = 11;
    pub const APPLAYER1: i32 = 12;
    pub const APPLAYER5: i32 = 16;
    pub const OPLOC1: i32 = 17;
    pub const OPLOC5: i32 = 21;
    pub const APLOC1: i32 = 22;
    pub const APLOC5: i32 = 26;
    pub const OPOBJ1: i32 = 27;
    pub const OPOBJ5: i32 = 31;
    pub const APOBJ1: i32 = 32;
    pub const APOBJ5: i32 = 36;
    pub const OPNPC1: i32 = 37;
    pub const OPNPC5: i32 = 41;
    pub const APNPC1: i32 = 42;
    pub const APNPC5: i32 = 46;
    pub const QUEUE1: i32 = 47;
    pub const QUEUE20: i32 = 66;
}

/// `HuntModeType`.
pub mod hunt_mode {
    pub const OFF: i32 = 0;
    pub const PLAYER: i32 = 1;
    pub const NPC: i32 = 2;
    pub const OBJ: i32 = 3;
    pub const SCENERY: i32 = 4;
}

/// `HuntVis`.
pub mod hunt_vis {
    pub const OFF: i32 = 0;
    pub const LINEOFSIGHT: i32 = 1;
    pub const LINEOFWALK: i32 = 2;
}

/// `HuntNobodyNear.PAUSEHUNT`.
pub const PAUSEHUNT: i32 = 1;

/// `NpcStat.HITPOINTS`.
pub const HITPOINTS: usize = 3;

/// `Npc.vars` reset on respawn: an int var is 0, any other non-string -1.
pub fn varn_reset(kind: i32) -> Option<i32> {
    use crate::cache::config::script_var_type as svt;
    match kind {
        svt::STRING => None,
        svt::INT => Some(0),
        _ => Some(-1),
    }
}

/// `HeroPoints`: 16 `(hash64, points)` slots, -1 when empty.
#[derive(Clone, Debug)]
pub struct HeroPoints(pub [(i64, i64); 16]);

impl Default for HeroPoints {
    fn default() -> Self {
        HeroPoints([(-1, 0); 16])
    }
}

impl HeroPoints {
    pub fn clear(&mut self) {
        self.0 = [(-1, 0); 16];
    }

    pub fn add_hero(&mut self, hash64: i64, points: i32) {
        if points < 1 {
            return;
        }
        if let Some(h) = self.0.iter_mut().find(|h| h.0 == hash64) {
            h.1 += points as i64;
        } else if let Some(h) = self.0.iter_mut().find(|h| h.0 == -1) {
            *h = (hash64, points as i64);
        }
    }

    /// `findHero()`: the top slot after the (unstable) OSRS quicksort.
    pub fn find_hero(&self) -> i64 {
        let mut clone = self.0;
        let high = clone.len() as isize - 1;
        quicksort(0, high, &mut clone, &mut |a: &(i64, i64), b: &(i64, i64)| b.1 - a.1);
        clone[0].0
    }
}

/// `NpcQueueRequest`.
#[derive(Clone, Debug)]
pub struct NpcQueueRequest {
    pub queue_id: i32,
    pub delay: i32,
    pub last_int: i32,
}

pub struct Npc {
    pub e: EntityBase,
    pub p: Pathing,
    pub nid: i32,
    pub uid: i32,
    pub base_type: i32,
    pub kind: i32,
    pub start_x: i32,
    pub start_z: i32,
    pub start_level: i32,
    /// `Uint16Array`s: writes wrap.
    pub levels: [u16; 6],
    pub base_levels: [u16; 6],
    pub vars: Vec<i32>,
    pub vars_string: Vec<Option<String>>,
    pub active_script: Option<Box<ScriptState>>,
    pub queue_links: Links<NpcQueueRequest>,
    pub queue: List,
    /// Unlinked queue nodes, freed after the queue pass (a pass may still
    /// hold one as its cached next node).
    pub queue_garbage: Vec<crate::engine::linklist::NodeId>,
    pub timer_interval: i32,
    pub timer_clock: i32,
    pub regen_interval: i32,
    pub regen_clock: i32,
    pub hunt_clock: i32,
    pub hunt_mode: i32,
    pub hunt_target: Option<crate::script::state::EntityRef>,
    pub huntrange: i32,
    pub next_patrol_point: i32,
    pub patrol_delay_ticks_remaining: i32,
    pub reset_on_revert: bool,
    pub stuck_counter: i32,
    pub hero_points: HeroPoints,
}

impl Npc {
    /// `new Npc(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(level: i32, x: i32, z: i32, width: i32, length: i32, lifecycle: i32, nid: i32, t: &NpcType, block_walk: i32, varn_count: usize) -> Self {
        let kind = t.id as i32;
        let mut queue_links = Links::default();
        let queue = queue_links.list();
        let mut levels = [0u16; 6];
        for (l, &s) in levels.iter_mut().zip(t.stats.iter()) {
            *l = s as u16;
        }
        let mut p = Pathing::new(x, z, block_walk, move_strategy::NAIVE, npc_prot::FACE_COORD, npc_prot::FACE_ENTITY);
        p.target_op = t.defaultmode;
        Npc {
            e: EntityBase::new(level, x, z, width, length, lifecycle),
            p,
            nid,
            uid: (kind << 16) | nid,
            base_type: kind,
            kind,
            start_x: x,
            start_z: z,
            start_level: level,
            levels,
            base_levels: levels,
            vars: vec![0; varn_count],
            vars_string: vec![None; varn_count],
            active_script: None,
            queue_links,
            queue,
            queue_garbage: Vec::new(),
            timer_interval: if t.timer != -1 { t.timer } else { 0 },
            timer_clock: 0,
            regen_interval: 0,
            regen_clock: 0,
            hunt_clock: 0,
            hunt_mode: t.huntmode,
            hunt_target: None,
            huntrange: t.huntrange,
            next_patrol_point: 0,
            patrol_delay_ticks_remaining: -1,
            reset_on_revert: true,
            stuck_counter: 0,
            hero_points: HeroPoints::default(),
        }
    }

    pub fn block_walk(&self) -> i32 {
        self.p.block_walk
    }

    /// `isValid()`: active and not delayed.
    pub fn is_valid(&self) -> bool {
        !self.p.delayed && self.e.is_active
    }

    pub fn set_timer(&mut self, interval: i32) {
        if interval != -1 {
            self.timer_interval = interval;
        }
    }

    pub fn enqueue_script(&mut self, queue_id: i32, delay: i32, arg: i32) {
        let node = self.queue_links.node(NpcQueueRequest { queue_id, delay, last_int: arg });
        self.queue_links.add_tail(self.queue, node);
    }

    pub fn clear_queue(&mut self) {
        for id in self.queue_links.collect(self.queue) {
            self.queue_links.unlink(id);
            self.queue_garbage.push(id);
        }
    }

    pub fn free_queue_garbage(&mut self) {
        for id in std::mem::take(&mut self.queue_garbage) {
            self.queue_links.free(id);
        }
    }

    pub fn unfocus(&mut self) {
        let (x, z, w, l) = (self.e.x, self.e.z, self.e.width, self.e.length);
        self.p.face_angle_x = crate::engine::coord::fine(x, w);
        self.p.face_angle_z = crate::engine::coord::fine(z - 1, l);
    }

    pub fn say(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.p.say_message = Some(text);
        self.p.masks |= npc_prot::SAY;
    }

    pub fn spotanim(&mut self, spotanim: i32, height: i32, delay: i32) {
        self.p.spotanim_id = spotanim;
        self.p.spotanim_height = height;
        self.p.spotanim_time = delay;
        self.p.masks |= npc_prot::SPOT_ANIM;
    }

    pub fn apply_damage(&mut self, mut damage: i32, kind: i32) {
        let current = self.levels[HITPOINTS] as i32;
        if current - damage <= 0 {
            self.levels[HITPOINTS] = 0;
            damage = current;
        } else {
            self.levels[HITPOINTS] = (current - damage) as u16;
        }
        if self.p.hitmark_slot % 2 == 1 {
            self.p.hitmark2_damage = damage;
            self.p.hitmark2_type = kind;
            self.p.masks |= npc_prot::DAMAGE2;
        } else {
            self.p.hitmark_damage = damage;
            self.p.hitmark_type = kind;
            self.p.masks |= npc_prot::DAMAGE;
        }
        self.p.hitmark_slot += 1;
    }

    pub fn face_square(&mut self, x: i32, z: i32) {
        self.p.focus(crate::engine::coord::fine(x, 1), crate::engine::coord::fine(z, 1), true);
    }

    pub fn clear_patrol(&mut self) {
        self.next_patrol_point = 0;
        self.stuck_counter = 0;
        self.patrol_delay_ticks_remaining = -1;
    }

    /// `Npc.clearInteraction()`.
    pub fn clear_interaction(&mut self) {
        self.p.clear_interaction();
        self.p.target_op = npc_mode::NONE;
    }

    /// `checkApTrigger()`.
    pub fn ap_trigger(&self) -> bool {
        use npc_mode::*;
        let m = self.p.target_op;
        (APNPC1..=APNPC5).contains(&m) || (APPLAYER1..=APPLAYER5).contains(&m) || (APLOC1..=APLOC5).contains(&m) || (APOBJ1..=APOBJ5).contains(&m)
    }

    /// `checkOpTrigger()`.
    pub fn op_trigger(&self) -> bool {
        use npc_mode::*;
        let m = self.p.target_op;
        (OPNPC1..=OPNPC5).contains(&m) || (OPPLAYER1..=OPPLAYER5).contains(&m) || (OPLOC1..=OPLOC5).contains(&m) || (OPOBJ1..=OPOBJ5).contains(&m)
    }

    /// `cleanup()` after a despawn.
    pub fn cleanup(&mut self) {
        self.nid = -1;
        self.uid = -1;
        self.active_script = None;
        self.p.delayed = false;
        self.p.delayed_until = -1;
        self.hunt_target = None;
        self.clear_queue();
    }
}

/// `getTriggerForMode`: the `ai_*` trigger an interaction mode runs.
pub fn trigger_for_mode(mode: i32) -> Option<i32> {
    use ares_shared::trigger as t;
    use npc_mode::*;
    let base = |first_mode: i32, first_trigger: i32| first_trigger + (mode - first_mode);
    Some(match mode {
        OPPLAYER1..=OPPLAYER5 => base(OPPLAYER1, t::AI_OPPLAYER1),
        APPLAYER1..=APPLAYER5 => base(APPLAYER1, t::AI_APPLAYER1),
        OPLOC1..=OPLOC5 => base(OPLOC1, t::AI_OPLOC1),
        APLOC1..=APLOC5 => base(APLOC1, t::AI_APLOC1),
        OPOBJ1..=OPOBJ5 => base(OPOBJ1, t::AI_OPOBJ1),
        APOBJ1..=APOBJ5 => base(APOBJ1, t::AI_APOBJ1),
        OPNPC1..=OPNPC5 => base(OPNPC1, t::AI_OPNPC1),
        APNPC1..=APNPC5 => base(APNPC1, t::AI_APNPC1),
        QUEUE1..=QUEUE20 => base(QUEUE1, t::AI_QUEUE1),
        _ => return None,
    })
}
