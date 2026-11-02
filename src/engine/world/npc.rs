//! NPC behaviour and the World's NPC phases
//! (`addNpc`, `removeNpc`, `processWorld`, `processNpcEventQueue`,
//! `processNpcs`, the NPC half of `processCleanup`).
//!
//! NPCs are addressed by nid. A despawned NPC whose `ai_despawn` script is
//! still queued lives in `dead_npcs` under a negative id (the event's reference
//! keeps the cleaned NPC alive).
//! ponytail: a nid freed and reused within one turn is seen as the same NPC
//! by the rest of that turn. Add a spawn serial if a
//! script ever does `npc_del` + `npc_add` on itself.

use super::World;
use super::movement::Mover;
use crate::cache::config::npc::NpcType;
use crate::engine::coord::{self, Area};
use crate::engine::entity::npc::{self as en, Npc, hunt_mode, npc_mode, trigger_for_mode, varn_reset};
use crate::engine::entity::pathing::{move_restrict, move_speed, move_strategy};
use crate::engine::entity::lifecycle;
use crate::engine::game_map::npc_spawn_collision;
use crate::engine::linklist::NodeId;
use crate::routefinder::{collision_type, flag, line, naive::find_naive_path, reach};
use crate::rsbuf::npc_prot;
use ares_shared::script::ScriptFile;
use crate::script::runner;
use crate::script::state::{self, Arg, EntityRef, ScriptState};
use ares_shared::trigger;
use crate::script::PointerType;
use std::sync::Arc;

/// `NpcEventRequest` (spawn or despawn trigger).
pub struct NpcEvent {
    pub script: Arc<ScriptFile>,
    pub nid: i32,
}

/// `ObjDelayedRequest`.
pub struct ObjDelayed {
    pub obj: Option<crate::engine::entity::Obj>,
    pub duration: i32,
    pub delay: i32,
    pub receiver64: i64,
}

/// `EntityQueueState` on the world queue.
pub struct WorldQueued {
    pub script: Option<Box<ScriptState>>,
    pub delay: i32,
}

/// `HuntType.checkHuntCondition(value, condition, checkValue)`.
fn hunt_condition(value: i32, condition: &str, check: i32) -> bool {
    match condition {
        ">" => value > check,
        "<" => value < check,
        "=" => value == check,
        "!" => value != check,
        "&" => value & check == 0,
        _ => false,
    }
}

/// One `Math.random()` draw from a mulberry32 state.
pub fn mulberry32(state: &mut i32) -> f64 {
    *state = state.wrapping_add(0x6d2b79f5u32 as i32);
    let m = *state as u32;
    let mut t = (m ^ (m >> 15)).wrapping_mul(1 | m);
    t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
    ((t ^ (t >> 14)) as f64) / 4294967296.0
}

impl World {
    /// `Math.random()`: mulberry32, seeded by [`World::math_seed`].
    pub fn math_random(&mut self) -> f64 {
        mulberry32(&mut self.math_state)
    }

    pub fn math_seed(&mut self, seed: i32) {
        self.math_state = seed;
    }

    pub fn npc(&self, id: i32) -> Option<&Npc> {
        if id >= 0 { self.npcs.get(id as usize) } else { self.dead_npcs.get(&id) }
    }

    pub fn npc_mut(&mut self, id: i32) -> Option<&mut Npc> {
        if id >= 0 { self.npcs.get_mut(id as usize) } else { self.dead_npcs.get_mut(&id) }
    }

    fn n(&mut self, id: i32) -> &mut Npc {
        self.npc_mut(id).expect("npc")
    }

    fn npc_type(&self, kind: i32) -> &NpcType {
        self.configs.npcs.get(kind as usize).expect("npc type")
    }

    /// `ScriptRunner.init(script, self, target, args)`.
    pub fn init_script(script: Arc<ScriptFile>, self_ref: Option<EntityRef>, target: Option<EntityRef>, args: &[Arg]) -> Box<ScriptState> {
        let mut s = Box::new(ScriptState::new(script, args));
        s.self_ref = self_ref;
        match self_ref {
            Some(EntityRef::Player(p)) => {
                s.active_player[0] = Some(p);
                s.pointer_add(PointerType::ActivePlayer);
            }
            Some(EntityRef::Npc(n)) => {
                s.active_npc[0] = Some(n);
                s.pointer_add(PointerType::ActiveNpc);
            }
            Some(EntityRef::Loc(l)) => {
                s.active_loc[0] = Some(l);
                s.pointer_add(PointerType::ActiveLoc);
            }
            Some(EntityRef::Obj(o)) => {
                s.active_obj[0] = Some(o);
                s.pointer_add(PointerType::ActiveObj);
            }
            None => {}
        }
        let same = |a: Option<EntityRef>, b: EntityRef| std::mem::discriminant(&a.unwrap_or(b)) == std::mem::discriminant(&b) && a.is_some();
        match target {
            Some(t @ EntityRef::Player(p)) => {
                let i = same(self_ref, t) as usize;
                s.active_player[i] = Some(p);
                s.pointer_add(if i == 1 { PointerType::ActivePlayer2 } else { PointerType::ActivePlayer });
            }
            Some(t @ EntityRef::Npc(n)) => {
                let i = same(self_ref, t) as usize;
                s.active_npc[i] = Some(n);
                s.pointer_add(if i == 1 { PointerType::ActiveNpc2 } else { PointerType::ActiveNpc });
            }
            Some(t @ EntityRef::Loc(l)) => {
                let i = same(self_ref, t) as usize;
                s.active_loc[i] = Some(l);
                s.pointer_add(if i == 1 { PointerType::ActiveLoc2 } else { PointerType::ActiveLoc });
            }
            Some(t @ EntityRef::Obj(o)) => {
                let i = same(self_ref, t) as usize;
                s.active_obj[i] = Some(o);
                s.pointer_add(if i == 1 { PointerType::ActiveObj2 } else { PointerType::ActiveObj });
            }
            None => {}
        }
        s
    }

    /// `World.enqueueScript(script, delay)`.
    pub fn enqueue_script(&mut self, script: Box<ScriptState>, delay: i32) {
        let node = self.queue_links.node(WorldQueued { script: Some(script), delay: delay + 1 });
        self.queue_links.add_tail(self.queue, node);
    }

    /// `Npc.executeScript(script)`: runs it and parks it where it suspended.
    /// `Err` on a player suspension with no active player.
    pub fn execute_npc_script(&mut self, mut s: Box<ScriptState>) -> Result<(), String> {
        let st = runner::execute(self, &mut s);
        if st == state::Execution::Finished || st == state::Execution::Aborted {
            return Ok(());
        }
        if st == state::Execution::WorldSuspended {
            let delay = s.pop_int();
            self.enqueue_script(s, delay);
        } else if st == state::Execution::NpcSuspended {
            let nid = s.active_npc()?;
            if let Some(n) = self.npc_mut(nid) {
                n.active_script = Some(s);
            }
        } else {
            let pid = s.active_player()?;
            if let Some(p) = self.player_mut(pid) {
                p.active_script = Some(s);
            }
        }
        Ok(())
    }

    // ---- spawn / despawn ----

    /// `World.addNpc(npc, duration)` for a new NPC.
    pub fn add_npc(&mut self, npc: Npc, duration: i32) {
        let nid = npc.nid;
        self.rsbuf.add_npc(nid, npc.kind);
        self.npcs.set(nid as usize, npc);
        self.respawn_npc(nid, duration);
    }

    /// `World.addNpc(npc, duration, false)`: an NPC already in the list.
    pub fn respawn_npc(&mut self, nid: i32, duration: i32) {
        let n = self.n(nid);
        n.e.x = n.start_x;
        n.e.z = n.start_z;
        n.e.is_active = true;
        let (x, z, level, width, block) = (n.e.x, n.e.z, n.e.level, n.e.width, n.block_walk());
        self.zone_enter(Mover::Npc(nid), x, z, level);
        npc_spawn_collision(self, width, x, z, level, block, true);
        self.npc_reset_respawn(nid);
        self.npc_play_animation(nid, -1, 0).ok();
        let kind = self.n(nid).kind;
        let t = self.npc_type(kind);
        if let Some(script) = self.scripts.get_by_trigger(trigger::AI_SPAWN, t.id as i32, t.category) {
            let node = self.npc_events.node(NpcEvent { script, nid });
            self.npc_events.add_tail(self.npc_event_queue, node);
        }
        if duration > -1 {
            self.npc_set_lifecycle(nid, duration);
        }
    }

    /// `World.removeNpc(npc, duration)`: the NPC object when it was despawned
    /// (taken out of the list), else `None`.
    pub fn remove_npc(&mut self, nid: i32, duration: i32) -> Option<Npc> {
        let n = self.npc(nid)?;
        if !n.e.is_active {
            return None;
        }
        let (x, z, level, width, block, life) = (n.e.x, n.e.z, n.e.level, n.e.width, n.block_walk(), n.e.lifecycle);
        let adjusted = self.scale_by_player_count(duration);
        self.zone_leave(Mover::Npc(nid), x, z, level);
        self.n(nid).e.is_active = false;
        npc_spawn_collision(self, width, x, z, level, block, false);
        if life == lifecycle::DESPAWN {
            let mut npc = if nid >= 0 {
                self.rsbuf.remove_npc(nid);
                let npc = self.npcs.take(nid as usize)?;
                if let Some(node) = self.npc_nodes.remove(&nid) {
                    self.arenas.members.free(node);
                }
                npc
            } else {
                self.dead_npcs.remove(&nid)?
            };
            npc.cleanup();
            return Some(npc);
        } else if life == lifecycle::RESPAWN && duration > -1 {
            self.npc_set_lifecycle(nid, adjusted);
        }
        None
    }

    /// `Entity.setLifeCycle(tick)`.
    fn npc_set_lifecycle(&mut self, nid: i32, tick: i32) {
        let now = self.current_tick;
        let n = self.n(nid);
        n.e.lifecycle_tick = if tick == -1 { -1 } else { tick.max(1) };
        n.e.last_lifecycle_tick = now;
    }

    /// `resetEntity(true)`.
    fn npc_reset_respawn(&mut self, nid: i32) {
        let varn_kinds: Vec<i32> = (0..self.configs.varns.count()).map(|i| self.configs.varns.get(i).map_or(-1, |v| v.kind)).collect();
        let base = self.n(nid).base_type;
        let stats = self.npc_type(base).stats;
        let n = self.n(nid);
        n.kind = n.base_type;
        n.uid = (n.kind << 16) | n.nid;
        n.unfocus();
        self.npc_play_animation(nid, -1, 0).ok();
        let n = self.n(nid);
        for (i, &s) in stats.iter().enumerate() {
            n.levels[i] = s as u16;
            n.base_levels[i] = s as u16;
        }
        n.hero_points.clear();
        n.clear_queue();
        n.active_script = None;
        n.p.delayed = false;
        n.p.delayed_until = -1;
        n.p.clear_waypoints();
        for (i, v) in n.vars.iter_mut().enumerate() {
            if let Some(r) = varn_reset(varn_kinds.get(i).copied().unwrap_or(-1)) {
                *v = r;
            }
        }
        n.vars_string.iter_mut().for_each(|s| *s = None);
        self.npc_reset_defaults(nid);
        let t = self.npc_type(base);
        let (huntrange, huntmode) = (t.huntrange, t.huntmode);
        let n = self.n(nid);
        n.huntrange = huntrange;
        n.hunt_mode = huntmode;
        n.hunt_clock = 0;
        n.hunt_target = None;
        n.p.tele = true;
    }

    /// `resetDefaults()`.
    pub fn npc_reset_defaults(&mut self, nid: i32) {
        let kind = self.n(nid).kind;
        let t = self.npc_type(kind);
        let (defaultmode, huntmode, huntrange, timer) = (t.defaultmode, t.huntmode, t.huntrange, t.timer);
        let n = self.n(nid);
        n.clear_interaction();
        n.p.target_op = defaultmode;
        n.hunt_mode = huntmode;
        n.huntrange = huntrange;
        n.hunt_clock = 0;
        n.hunt_target = None;
        n.timer_interval = timer;
    }

    /// `playAnimation(anim, delay)`.
    pub fn npc_play_animation(&mut self, nid: i32, anim: i32, delay: i32) -> Result<(), String> {
        if anim >= self.configs.seqs.count() as i32 {
            return Ok(());
        }
        let cur = self.n(nid).p.anim_id;
        let priority = |id: i32| self.configs.seqs.get(id as usize).map(|s| s.priority);
        let replace = anim == -1 || cur == -1 || {
            let new = priority(anim).ok_or_else(|| format!("no SeqType for anim {anim}"))?;
            new >= priority(cur).unwrap_or(i32::MAX)
        };
        if replace {
            let n = self.n(nid);
            n.p.anim_id = anim;
            n.p.anim_delay = delay;
            n.p.masks |= npc_prot::ANIM;
        }
        Ok(())
    }

    /// `changeType(type, duration, reset)`.
    pub fn npc_change_type(&mut self, nid: i32, kind: i32, duration: i32, reset: bool) {
        if !self.n(nid).e.is_active || duration < 1 {
            return;
        }
        let stats = self.npc_type(kind).stats;
        let n = self.n(nid);
        n.kind = kind;
        n.p.masks |= npc_prot::CHANGE_TYPE;
        n.uid = (kind << 16) | n.nid;
        n.reset_on_revert = reset;
        if reset {
            for (i, &level) in stats.iter().enumerate() {
                n.levels[i] = (level - (n.base_levels[i] as i32 - n.levels[i] as i32)).max(0) as u16;
                n.base_levels[i] = level as u16;
            }
        }
        if kind == n.base_type && n.e.lifecycle == lifecycle::RESPAWN {
            self.npc_set_lifecycle(nid, -1);
        } else {
            self.npc_set_lifecycle(nid, duration);
        }
    }

    fn npc_revert_type(&mut self, nid: i32) {
        if self.n(nid).reset_on_revert {
            self.remove_npc(nid, -1);
            self.respawn_npc(nid, -1);
        } else {
            let n = self.n(nid);
            n.kind = n.base_type;
            n.p.masks |= npc_prot::CHANGE_TYPE;
            n.uid = (n.kind << 16) | n.nid;
        }
    }

    // ---- entity lookups for targets ----

    /// `(level, x, z, width, length)` of any entity.
    pub fn entity_area(&self, r: EntityRef) -> Option<(i32, Area)> {
        let e = match r {
            EntityRef::Player(p) => &self.player(p)?.e,
            EntityRef::Npc(n) => &self.npc(n)?.e,
            EntityRef::Loc(l) => &self.arenas.locs.get(l)?.e,
            EntityRef::Obj(o) => &self.arenas.objs.get(o)?.e,
        };
        Some((e.level, (e.x, e.z, e.width, e.length)))
    }

    /// `target.type` of an NPC, loc or obj.
    pub fn entity_type(&self, r: EntityRef) -> i32 {
        match r {
            EntityRef::Player(_) => -1,
            EntityRef::Npc(n) => self.npc(n).map_or(-1, |n| n.kind),
            EntityRef::Loc(l) => self.arenas.locs.get(l).map_or(-1, |l| l.kind()),
            EntityRef::Obj(o) => self.arenas.objs.get(o).map_or(-1, |o| o.kind),
        }
    }

    /// `isValid()` with no observer.
    pub fn entity_valid(&self, r: EntityRef) -> bool {
        match r {
            EntityRef::Player(s) => self.player_is_valid(s),
            EntityRef::Npc(n) => self.npc(n).is_some_and(|n| n.is_valid()),
            EntityRef::Loc(l) => self.arenas.locs.get(l).is_some_and(|l| l.e.is_active),
            EntityRef::Obj(o) => self.arenas.objs.get(o).is_some_and(|o| o.is_valid(None)),
        }
    }

    // ---- PathingEntity (for NPCs) ----

    /// `teleport(x, z, level)` for an NPC.
    pub fn npc_teleport(&mut self, nid: i32, x: i32, z: i32, level: i32) {
        self.teleport(Mover::Npc(nid), x, z, level);
    }

    fn npc_area(&self, nid: i32) -> Area {
        let e = &self.npc(nid).expect("npc").e;
        (e.x, e.z, e.width, e.length)
    }

    /// `inOperableDistance(target)`.
    fn npc_in_operable_distance(&self, nid: i32, target: EntityRef) -> bool {
        let n = self.npc(nid).expect("npc");
        let Some((tlevel, (tx, tz, tw, tl))) = self.entity_area(target) else { return false };
        if tlevel != n.e.level {
            return false;
        }
        let (x, z, level, w) = (n.e.x, n.e.z, n.e.level, n.e.width);
        match target {
            EntityRef::Player(_) | EntityRef::Npc(_) => reach::reached(&self.collision, level, x, z, tx, tz, tw, tl, w, 0, -2, 0),
            EntityRef::Loc(id) => {
                let loc = self.loc(id);
                let force = self.configs.locs.get(loc.kind() as usize).map_or(0, |t| t.forceapproach);
                reach::reached(&self.collision, level, x, z, tx, tz, tw, tl, w, loc.angle(), loc.shape(), force)
            }
            EntityRef::Obj(_) => reach::reached(&self.collision, level, x, z, tx, tz, tw, tl, w, 0, -1, 0),
        }
    }

    /// `inApproachDistance(range, target)`: an NPC traces line of sight from
    /// the target back to itself.
    fn npc_in_approach_distance(&self, nid: i32, range: i32, target: EntityRef) -> bool {
        let n = self.npc(nid).expect("npc");
        let Some((tlevel, ta)) = self.entity_area(target) else { return false };
        if tlevel != n.e.level {
            return false;
        }
        let me = self.npc_area(nid);
        if matches!(target, EntityRef::Player(_) | EntityRef::Npc(_)) && coord::intersects(me.0, me.1, me.2, me.3, ta.0, ta.1, ta.2, ta.3) {
            return false;
        }
        coord::distance_to(me, ta) <= range && line::has_line_of_sight(&self.collision, n.e.level, ta.0, ta.1, me.0, me.1, ta.2, ta.3, me.2, me.3, flag::BLOCK_NPC_AND_PLAYERS)
    }

    /// `randomWalk()`.
    fn npc_random_walk(&mut self, nid: i32) {
        let (mut x, mut z) = (self.n(nid).e.x, self.n(nid).e.z);
        if self.math_random() < 0.5 {
            x += if self.math_random() < 0.5 { -1 } else { 1 };
        } else {
            z += if self.math_random() < 0.5 { -1 } else { 1 };
        }
        self.n(nid).p.queue_waypoint(x, z);
    }

    /// `Npc.pathToTarget()` (NPCs move NAIVE, or FLY via a script).
    fn npc_path_to_target(&mut self, nid: i32) {
        let Some(target) = self.n(nid).p.target else { return };
        let Some((_, ta)) = self.entity_area(target) else { return };
        let pathing = matches!(target, EntityRef::Player(_) | EntityRef::Npc(_));
        let me = self.npc_area(nid);
        if pathing && coord::intersects(me.0, me.1, me.2, me.3, ta.0, ta.1, ta.2, ta.3) {
            self.npc_random_walk(nid);
            return;
        }
        // PathingEntity.pathToTarget, NAIVE and FLY branches
        // ponytail: SMART (findPath) is a player strategy; it lands with Player.
        if self.collision_strategy(Mover::Npc(nid)).is_none() || self.block_walk_flag(Mover::Npc(nid)) == flag::NULL {
            return;
        }
        if self.n(nid).p.move_strategy == move_strategy::NAIVE && pathing {
            let angle = match target {
                EntityRef::Loc(l) => self.loc(l).angle(),
                _ => 0,
            };
            let level = self.n(nid).e.level;
            let mut st = self.math_state;
            let waypoints = find_naive_path(&self.collision, level, me.0, me.1, ta.0, ta.1, me.2, me.3, ta.2, ta.3, angle as u32, collision_type::NORMAL, &mut || mulberry32(&mut st));
            self.math_state = st;
            let (_, x, z) = coord::unpack(waypoints.first().copied().unwrap_or(0) as i32);
            if x != me.0 || z != me.1 {
                self.n(nid).p.queue_waypoints(&waypoints);
            }
        } else {
            self.n(nid).p.queue_waypoint(ta.0, ta.1);
        }
    }

    /// `Npc.updateMovement()`: true when the NPC moved this tick.
    fn npc_update_movement(&mut self, nid: i32) -> bool {
        let kind = self.n(nid).kind;
        if self.npc_type(kind).moverestrict == move_restrict::NOMOVE {
            return false;
        }
        let n = self.n(nid);
        if n.p.move_speed != move_speed::INSTANT {
            n.p.move_speed = move_speed::WALK;
        }
        if n.p.waypoint_index != -1 {
            if n.p.walktrigger != -1 {
                let (wt, arg) = (n.p.walktrigger, n.p.walktrigger_arg);
                n.p.walktrigger = -1;
                let t = self.npc_type(kind);
                if let Some(script) = self.scripts.get_by_trigger(trigger::AI_QUEUE1 + wt, t.id as i32, t.category) {
                    let mut s = Self::init_script(script, Some(EntityRef::Npc(nid)), None, &[Arg::Int(arg)]);
                    runner::execute(self, &mut s);
                }
            }
            if self.npc(nid).is_none() {
                return false;
            }
            self.process_movement(Mover::Npc(nid));
        }
        let tick = self.current_tick;
        let n = self.n(nid);
        let moved = n.p.last_tick_x != n.e.x || n.p.last_tick_z != n.e.z;
        if moved {
            n.p.last_movement = tick + 1;
            n.stuck_counter = 0;
        }
        moved
    }

    /// `setInteraction(SCRIPT, target, op)`.
    pub fn npc_set_interaction(&mut self, nid: i32, target: EntityRef, op: i32) -> bool {
        if !self.entity_valid(target) {
            return false;
        }
        let kind = self.entity_type(target);
        let area = self.entity_area(target);
        let n = self.n(nid);
        n.p.target = Some(target);
        n.p.target_op = op;
        n.p.ap_range = 10;
        n.p.ap_range_called = false;
        n.p.target_subject_com = -1;
        n.p.target_subject_type = if matches!(target, EntityRef::Player(_)) { -1 } else { kind };
        if let (EntityRef::Loc(_) | EntityRef::Obj(_), Some((_, (x, z, w, l)))) = (target, area) {
            n.p.target_x = coord::fine(x, w);
            n.p.target_z = coord::fine(z, l);
        }
        true
    }

    /// `targetWithinMaxRange()`.
    pub fn npc_target_within_max_range(&self, nid: i32) -> bool {
        let n = self.npc(nid).expect("npc");
        let Some(target) = n.p.target else { return true };
        if n.p.target_op == npc_mode::PLAYERFOLLOW {
            return true;
        }
        let t = self.npc_type(n.kind);
        let Some((_, (tx, tz, tw, tl))) = self.entity_area(target) else { return true };
        if n.op_trigger() {
            let (dx, dz) = ((tx - n.start_x).abs(), (tz - n.start_z).abs());
            if dx.max(dz) > t.maxrange + 1 {
                return false;
            }
            if dx == t.maxrange + 1 && dz == t.maxrange + 1 {
                return false;
            }
        } else if n.ap_trigger() {
            if coord::distance_to_sw(tx, tz, n.start_x, n.start_z) > t.maxrange + t.attackrange {
                return false;
            }
        } else if n.p.target_op == npc_mode::PLAYERESCAPE {
            let escape = coord::distance_to((n.e.x, n.e.z, n.e.width, n.e.length), (n.start_x, n.start_z, n.e.width, n.e.length));
            let from_start = coord::distance_to((tx, tz, tw, tl), (n.start_x, n.start_z, tw, tl));
            if from_start > t.maxrange && escape > t.maxrange {
                return false;
            }
        } else if coord::distance_to_sw(tx, tz, n.start_x, n.start_z) > t.maxrange + 1 {
            return false;
        }
        true
    }

    // ---- Npc.turn ----

    /// `Npc.turn()`; on `Err` `processNpcs` removes the NPC.
    pub fn npc_turn(&mut self, nid: i32) -> Result<(), String> {
        let tick = self.current_tick;
        let n = self.n(nid);
        if n.e.is_active {
            if n.p.delayed && tick >= n.p.delayed_until {
                n.p.delayed = false;
            }
            if !n.p.delayed && n.active_script.as_ref().is_some_and(|s| s.execution == state::Execution::NpcSuspended) {
                let s = n.active_script.take().unwrap();
                self.execute_npc_script(s)?;
            }
        }
        let Some(n) = self.npc_mut(nid) else { return Ok(()) };
        if !n.p.delayed && {
            n.e.lifecycle_tick -= 1;
            n.e.lifecycle_tick == 0
        } {
            match n.e.lifecycle {
                lifecycle::RESPAWN if !n.e.is_active => self.respawn_npc(nid, -1),
                lifecycle::RESPAWN => self.npc_revert_type(nid),
                lifecycle::DESPAWN => {
                    let kind = n.kind;
                    if let Some(npc) = self.remove_npc(nid, -1) {
                        let t = self.npc_type(kind);
                        if let Some(script) = self.scripts.get_by_trigger(trigger::AI_DESPAWN, t.id as i32, t.category) {
                            self.next_dead_npc -= 1;
                            let id = self.next_dead_npc;
                            self.dead_npcs.insert(id, npc);
                            let node = self.npc_events.node(NpcEvent { script, nid: id });
                            self.npc_events.add_tail(self.npc_event_queue, node);
                        }
                    }
                }
                _ => {}
            }
        }
        if !self.npc(nid).is_some_and(|n| n.is_valid()) {
            return Ok(());
        }

        // partial hunt
        let hunt_mode = self.n(nid).hunt_mode;
        if hunt_mode != -1 {
            let hunt = self.configs.hunts.get(hunt_mode as usize).ok_or_else(|| format!("no HuntType for hunt mode {hunt_mode}"))?;
            if hunt.nobody_near != en::PAUSEHUNT || self.rsbuf.npc_observers(nid) > 0 || hunt.kind == hunt_mode::PLAYER {
                if hunt.kind != hunt_mode::PLAYER {
                    self.npc_hunt_all(nid, hunt_mode)?;
                }
                self.n(nid).hunt_clock += 1;
            }
        }
        self.npc_consume_hunt_target(nid);
        if self.npc(nid).is_none() {
            return Ok(());
        }
        self.npc_process_regen(nid);
        self.npc_process_timers(nid)?;
        if self.npc(nid).is_none() {
            return Ok(());
        }
        self.npc_process_queue(nid)?;
        if self.npc(nid).is_none() {
            return Ok(());
        }
        self.npc_process_movement_interaction(nid)?;
        if self.npc(nid).is_none() {
            return Ok(());
        }
        // reorientEntity, reorient, setFaceEntity, validateDistanceWalked
        let target = self.n(nid).p.target;
        let target_area = target.and_then(|t| self.entity_area(t));
        let pathing = matches!(target, Some(EntityRef::Player(_) | EntityRef::Npc(_)));
        let face = self.face_entity_of(target);
        let n = self.n(nid);
        if let (true, Some((_, (tx, tz, tw, tl)))) = (pathing, target_area) {
            n.p.focus(coord::fine(tx, tw), coord::fine(tz, tl), false);
        }
        if !pathing && n.p.target_x != -1 && n.p.steps_taken == 0 {
            let (fx, fz) = (n.p.target_x, n.p.target_z);
            n.p.focus(fx, fz, true);
            n.p.target_x = -1;
            n.p.target_z = -1;
        }
        n.p.set_face_entity(face);
        let (x, z, w, l) = (n.e.x, n.e.z, n.e.width, n.e.length);
        if coord::distance_to((x, z, w, l), (n.p.last_tick_x, n.p.last_tick_z, w, l)) > 2 {
            n.p.jump = true;
        }
        Ok(())
    }

    /// `huntAll(hunt)` for NPC, obj and loc hunts.
    fn npc_hunt_all(&mut self, nid: i32, hunt_id: i32) -> Result<(), String> {
        self.n(nid).hunt_target = None;
        let hunt = self.configs.hunts.get(hunt_id as usize).expect("hunt");
        let (kind, rate, vis, cat) = (hunt.kind, hunt.rate, hunt.check_vis, hunt.check_category);
        let check = match kind {
            hunt_mode::NPC => hunt.check_npc,
            hunt_mode::OBJ => hunt.check_obj,
            _ => hunt.check_loc,
        };
        let n = self.n(nid);
        if n.hunt_clock < rate - 1 {
            return Ok(());
        }
        if kind == hunt_mode::OFF || n.huntrange < 1 {
            return Ok(());
        }
        let (level, x, z, range) = (n.e.level, n.e.x, n.e.z, n.huntrange);
        let mut hunted = crate::script::iterators::hunt(self, level, x, z, range, vis, check, cat, kind)?;
        if kind == hunt_mode::PLAYER {
            hunted = self.hunt_players_filter(nid, hunt_id, hunted);
        }
        if !hunted.is_empty() {
            let i = (self.math_random() * hunted.len() as f64) as usize;
            self.n(nid).hunt_target = Some(hunted[i]);
        }
        Ok(())
    }

    /// The per-player checks of `huntPlayers(hunt)`.
    fn hunt_players_filter(&mut self, nid: i32, hunt_id: i32, candidates: Vec<EntityRef>) -> Vec<EntityRef> {
        let tick = self.current_tick;
        let hunt = self.configs.hunts.get(hunt_id as usize).expect("hunt").clone();
        let n = self.npc(nid).expect("npc");
        let (vislevel, target) = (self.npc_type(n.kind).vislevel, n.p.target);
        let npc_var = |w: &World, id: i32| w.npc(nid).and_then(|n| n.vars.get(id as usize).copied()).unwrap_or(0);
        let mut out = Vec::new();
        for e in candidates {
            let EntityRef::Player(slot) = e else { continue };
            if hunt.check_not_busy && self.busy(slot) {
                continue;
            }
            let p = self.player(slot).expect("player");
            if hunt.check_afk && p.last_afk_zone == 1000 {
                continue;
            }
            let wilderness = (p.e.x >= 2944 && p.e.x < 3392) && ((p.e.z >= 3520 && p.e.z < 6400) || (p.e.z >= 9920 && p.e.z < 12800));
            if hunt.check_not_too_strong == 1 && !wilderness && p.combat_level > vislevel * 2 {
                continue;
            }
            if target != Some(e) && !self.map.is_multi(p.e.x, p.e.z, p.e.level) {
                let var = |id: i32| self.player_get_var(slot, id).unwrap_or(0);
                if hunt.check_not_combat != -1 && var(hunt.check_not_combat) as i64 + 8 > tick as i64 {
                    continue;
                }
                if hunt.check_not_combat_self != -1 && npc_var(self, hunt.check_not_combat_self) as i64 + 8 > tick as i64 {
                    continue;
                }
            }
            if !hunt.check_vars.iter().all(|v| v.var_id == -1 || hunt_condition(self.player_get_var(slot, v.var_id).unwrap_or(0), &v.condition, v.val)) {
                continue;
            }
            if hunt.check_inv != -1 {
                let quantity = self.hunt_inv_quantity(slot, &hunt);
                if !hunt_condition(quantity, &hunt.check_inv_condition, hunt.check_inv_val) {
                    continue;
                }
            }
            out.push(e);
        }
        out
    }

    fn hunt_inv_quantity(&mut self, slot: i32, hunt: &crate::cache::config::hunt::HuntType) -> i32 {
        let Some(owner) = self.player_inventory(slot, hunt.check_inv) else { return 0 };
        let inv = self.inv(owner).expect("inventory");
        let total: i64 = if hunt.check_obj != -1 {
            inv.item_count(hunt.check_obj)
        } else if hunt.check_obj_param != -1 {
            let Some(p) = self.configs.params.get(hunt.check_obj_param as usize) else { return 0 };
            inv.items.iter().take(inv.capacity).flatten().filter(|it| it.id >= 0 && (it.id as usize) < self.configs.objs.count()).map(|it| crate::script::ops::int_param(Some(&self.configs.objs.get(it.id as usize).unwrap().params), p.id as i32, p.default_int) as i64).sum()
        } else if hunt.check_obj_cat != -1 {
            inv.items_filtered().iter().filter(|it| self.configs.objs.get(it.id as usize).is_some_and(|t| t.category == hunt.check_obj_cat)).map(|it| it.count).sum()
        } else {
            0
        };
        total as i32
    }

    fn npc_consume_hunt_target(&mut self, nid: i32) {
        let n = self.npc(nid).expect("npc");
        let Some(target) = n.hunt_target else { return };
        if n.hunt_mode < 0 {
            return;
        }
        let Some(hunt) = self.configs.hunts.get(n.hunt_mode as usize) else { return };
        if hunt.kind == hunt_mode::OFF {
            return;
        }
        let (find_new_mode, keep) = (hunt.find_new_mode, hunt.find_keep_hunting);
        if (npc_mode::QUEUE1..=npc_mode::QUEUE20).contains(&find_new_mode) {
            let t = self.npc_type(n.kind);
            if let Some(script) = self.scripts.get_by_trigger(trigger::AI_QUEUE1 + (find_new_mode - npc_mode::QUEUE1), t.id as i32, t.category) {
                let mut s = Self::init_script(script, Some(EntityRef::Npc(nid)), None, &[]);
                runner::execute(self, &mut s);
            }
        } else {
            self.npc_set_interaction(nid, target, find_new_mode);
        }
        let Some(n) = self.npc_mut(nid) else { return };
        n.hunt_target = None;
        n.hunt_clock = 0;
        if !keep {
            n.hunt_mode = -1;
        }
    }

    fn npc_process_regen(&mut self, nid: i32) {
        let kind = self.n(nid).kind;
        let rate = self.npc_type(kind).regenrate;
        let n = self.n(nid);
        n.regen_clock += 1;
        if n.regen_clock >= n.regen_interval {
            n.regen_interval = rate;
            n.regen_clock = 0;
            for i in 0..6 {
                if n.levels[i] < n.base_levels[i] {
                    n.levels[i] = n.levels[i].wrapping_add(1);
                } else if n.levels[i] > n.base_levels[i] {
                    n.levels[i] -= 1;
                }
            }
        }
    }

    fn npc_process_timers(&mut self, nid: i32) -> Result<(), String> {
        let n = self.n(nid);
        if n.timer_interval > 0 && {
            n.timer_clock += 1;
            n.timer_clock >= n.timer_interval
        } {
            let kind = n.kind;
            let t = self.npc_type(kind);
            if let Some(script) = self.scripts.get_by_trigger(trigger::AI_TIMER, t.id as i32, t.category) {
                self.execute_npc_script(Self::init_script(script, Some(EntityRef::Npc(nid)), None, &[]))?;
                if let Some(n) = self.npc_mut(nid) {
                    n.timer_clock = 0;
                }
            }
        }
        Ok(())
    }

    fn npc_process_queue(&mut self, nid: i32) -> Result<(), String> {
        if !self.n(nid).e.is_active {
            return Ok(());
        }
        let n = self.n(nid);
        let mut c = n.queue_links.iter(n.queue, false);
        loop {
            let Some(n) = self.npc_mut(nid) else { return Ok(()) };
            let Some(id) = n.queue_links.next(&mut c) else { break };
            let delayed = n.p.delayed;
            let r = n.queue_links.get_mut(id).expect("queue request");
            if !delayed {
                r.delay -= 1;
            }
            if !delayed && r.delay <= 0 {
                let (queue_id, last_int) = (r.queue_id, r.last_int);
                n.queue_links.unlink(id);
                n.queue_garbage.push(id);
                let kind = n.kind;
                let t = self.npc_type(kind);
                if let Some(script) = self.scripts.get_by_trigger(queue_id, t.id as i32, t.category) {
                    let mut s = Self::init_script(script, Some(EntityRef::Npc(nid)), None, &[]);
                    s.last_int = last_int;
                    self.execute_npc_script(s)?;
                }
            }
        }
        if let Some(n) = self.npc_mut(nid) {
            n.free_queue_garbage();
        }
        Ok(())
    }

    fn npc_process_movement_interaction(&mut self, nid: i32) -> Result<(), String> {
        let n = self.n(nid);
        if n.p.delayed || !n.e.is_active {
            return Ok(());
        }
        if n.p.target_op == npc_mode::NULL {
            let kind = n.kind;
            let mode = self.npc_type(kind).defaultmode;
            self.n(nid).p.target_op = mode;
        }
        match self.n(nid).p.target_op {
            npc_mode::NONE => {
                self.npc_update_movement(nid);
                return Ok(());
            }
            npc_mode::WANDER => {
                self.npc_wander_mode(nid);
                return Ok(());
            }
            npc_mode::PATROL => {
                self.npc_patrol_mode(nid);
                return Ok(());
            }
            _ => {}
        }
        if self.n(nid).p.target.is_none() || !self.npc_validate_target(nid) {
            self.npc_reset_defaults(nid);
            return Ok(());
        }
        let mode = self.n(nid).p.target_op;
        let name = match mode {
            npc_mode::PLAYERESCAPE => "playerescape",
            npc_mode::PLAYERFOLLOW => "playerfollow",
            npc_mode::PLAYERFACE => "playerface",
            npc_mode::PLAYERFACECLOSE => "playerfaceclose",
            _ => return self.npc_ai_mode(nid),
        };
        let Some(EntityRef::Player(ps)) = self.n(nid).p.target else {
            return Err(format!("[Npc] Target must be a Player for {name} mode."));
        };
        match mode {
            npc_mode::PLAYERESCAPE => self.npc_player_escape_mode(nid, ps),
            npc_mode::PLAYERFOLLOW => {
                self.npc_path_to_target(nid);
                self.npc_update_movement(nid);
            }
            npc_mode::PLAYERFACECLOSE => {
                let me = self.npc_area(nid);
                if self.entity_area(EntityRef::Player(ps)).is_some_and(|(_, a)| coord::distance_to(me, a) > 1) {
                    self.npc_reset_defaults(nid);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `playerEscapeMode()`: step away from the player, inside `maxrange` of spawn.
    fn npc_player_escape_mode(&mut self, nid: i32, ps: i32) {
        use coord::direction::*;
        let p = self.player(ps).expect("player");
        let (px, pz) = (p.e.x, p.e.z);
        let n = self.n(nid);
        let (x, z, level, width, sx, sz, kind) = (n.e.x, n.e.z, n.e.level, n.e.width, n.start_x, n.start_z, n.kind);
        if coord::distance_to_sw(x, z, px, pz) > 25 {
            self.npc_reset_defaults(nid);
            return;
        }
        let dir = if px >= x && pz >= z {
            SOUTH_WEST
        } else if px >= x {
            NORTH_WEST
        } else if pz >= z {
            SOUTH_EAST
        } else {
            NORTH_EAST
        };
        let (dx, dz) = (coord::delta_x(dir), coord::delta_z(dir));
        let max_range = self.npc_type(kind).maxrange;
        let strategy = self.collision_strategy(Mover::Npc(nid)).unwrap_or(collision_type::NORMAL);
        let extra = self.block_walk_flag(Mover::Npc(nid));
        let ok = |w: &World, sdx: i32, sdz: i32| w.can_travel(level, x, z, sdx, sdz, width, extra, strategy) && coord::distance_to_sw(x + sdx, z + sdz, sx, sz) <= max_range;
        // the diagonal, else the x step, else the z step (every direction prefers x)
        let step = [(dx, dz), (dx, 0), (0, dz)].into_iter().find(|&(a, b)| ok(self, a, b));
        if let Some((a, b)) = step {
            self.n(nid).p.queue_waypoint(x + a, z + b);
        }
        if !self.npc_update_movement(nid) {
            let Some(n) = self.npc_mut(nid) else { return };
            n.stuck_counter += 1;
        }
        let Some(n) = self.npc_mut(nid) else { return };
        let at_max_both = coord::distance_to_sw(n.e.x, sz, sx, sz) >= max_range && coord::distance_to_sw(sx, n.e.z, sx, sz) >= max_range;
        if n.stuck_counter >= 5 && !at_max_both {
            self.npc_reset_defaults(nid);
            self.n(nid).stuck_counter = 0;
        }
    }

    fn npc_validate_target(&self, nid: i32) -> bool {
        let n = self.npc(nid).expect("npc");
        let Some(target) = n.p.target else { return false };
        if self.entity_area(target).map(|a| a.0) != Some(n.e.level) {
            return false;
        }
        if !self.npc_target_within_max_range(nid) {
            return false;
        }
        if matches!(target, EntityRef::Npc(_) | EntityRef::Loc(_)) && n.p.target_subject_type != self.entity_type(target) {
            return false;
        }
        match target {
            EntityRef::Npc(t) => self.npc(t).is_some_and(|t| t.e.is_active),
            _ => self.entity_valid(target),
        }
    }

    fn npc_wander(&mut self, nid: i32, range: i32) {
        let r = range as f64;
        let dx = crate::util::numbers::js_round(self.math_random() * (r * 2.0) - r) as i32;
        let dz = crate::util::numbers::js_round(self.math_random() * (r * 2.0) - r) as i32;
        let n = self.n(nid);
        let (x, z) = (n.start_x + dx, n.start_z + dz);
        if x != n.e.x || z != n.e.z {
            n.p.queue_waypoint(x, z);
        }
    }

    fn npc_wander_mode(&mut self, nid: i32) {
        let kind = self.n(nid).kind;
        let t = self.npc_type(kind);
        let (restrict, range) = (t.moverestrict, t.wanderrange);
        if restrict != move_restrict::NOMOVE && self.math_random() < 0.125 {
            self.npc_wander(nid, range);
        }
        self.npc_update_movement(nid);
        let Some(n) = self.npc_mut(nid) else { return };
        let on_spawn = n.e.x == n.start_x && n.e.z == n.start_z && n.e.level == n.start_level;
        let stuck = n.stuck_counter;
        n.stuck_counter += 1;
        if stuck > 500 {
            if !on_spawn {
                let (x, z, level) = (n.start_x, n.start_z, n.start_level);
                self.teleport(Mover::Npc(nid), x, z, level);
            }
            self.n(nid).stuck_counter = 0;
        }
    }

    fn npc_patrol_mode(&mut self, nid: i32) {
        let kind = self.n(nid).kind;
        let t = self.npc_type(kind);
        let (points, delays) = (t.patrol_coord.clone(), t.patrol_delay.clone());
        if points.is_empty() {
            self.npc_update_movement(nid);
            return;
        }
        let n = self.n(nid);
        let (mut dl, mut dx, mut dz) = coord::unpack(points.get(n.next_patrol_point as usize).copied().unwrap_or(0));
        if !n.p.has_waypoints() && n.p.target.is_none() {
            n.p.queue_waypoint(dx, dz);
        }
        n.stuck_counter += 1;
        if n.stuck_counter >= 32 || n.e.level != dl {
            self.teleport(Mover::Npc(nid), dx, dz, dl);
            self.n(nid).stuck_counter = 0;
        }
        let n = self.n(nid);
        if n.e.x == dx && n.e.z == dz {
            if n.patrol_delay_ticks_remaining < 0 {
                n.patrol_delay_ticks_remaining = delays.get(n.next_patrol_point as usize).copied().unwrap_or(0);
            }
            let remaining = n.patrol_delay_ticks_remaining;
            n.patrol_delay_ticks_remaining -= 1;
            if remaining <= 0 {
                n.next_patrol_point = (n.next_patrol_point + 1) % points.len() as i32;
                n.patrol_delay_ticks_remaining = -1;
                (dl, dx, dz) = coord::unpack(points[n.next_patrol_point as usize]);
                let _ = dl;
                n.p.queue_waypoint(dx, dz);
            }
        }
        self.npc_update_movement(nid);
    }

    fn npc_ai_mode(&mut self, nid: i32) -> Result<(), String> {
        let kind = self.n(nid).kind;
        let givechase = self.npc_type(kind).givechase;
        self.n(nid).stuck_counter = 0;
        if self.npc_try_interact(nid, true)? {
            return Ok(());
        }
        if self.npc(nid).is_none() {
            return Ok(());
        }
        self.npc_path_to_target(nid);
        let moved = self.npc_update_movement(nid);
        if self.npc(nid).is_none() {
            return Ok(());
        }
        if moved && !givechase {
            self.npc_reset_defaults(nid);
            return Ok(());
        }
        if self.n(nid).p.target.is_some() {
            self.npc_try_interact(nid, false)?;
        }
        Ok(())
    }

    fn npc_try_interact(&mut self, nid: i32, allow_op_scenery: bool) -> Result<bool, String> {
        let n = self.n(nid);
        let Some(target) = n.p.target else { return Ok(false) };
        let (kind, op, ap, mode) = (n.kind, n.op_trigger(), n.ap_trigger(), n.p.target_op);
        let t = self.npc_type(kind);
        let (category, attackrange) = (t.category, t.attackrange);
        let script = trigger_for_mode(mode).and_then(|tr| self.scripts.get_by_trigger(tr, kind, category));
        let pathing = matches!(target, EntityRef::Player(_) | EntityRef::Npc(_));
        let hit = if op && self.npc_in_operable_distance(nid, target) && (pathing || allow_op_scenery) {
            true
        } else {
            ap && self.npc_in_approach_distance(nid, attackrange, target)
        };
        if hit {
            if let Some(script) = script {
                self.execute_npc_script(Self::init_script(script, Some(EntityRef::Npc(nid)), Some(target), &[]))?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    // ---- World phases ----

    /// `objDelayedQueue.addTail(new ObjDelayedRequest(obj, duration, delay, receiver64))`.
    pub fn queue_delayed_obj(&mut self, obj: crate::engine::entity::Obj, receiver64: i64, duration: i32, delay: i32) {
        let node = self.obj_delayed_links.node(ObjDelayed { obj: Some(obj), duration, delay, receiver64 });
        self.obj_delayed_links.add_tail(self.obj_delayed, node);
    }

    /// `processWorld()`: the world queue, delayed objs and player hunts.
    pub fn process_world(&mut self) {
        let mut c = self.queue_links.iter(self.queue, false);
        let mut done: Vec<NodeId> = Vec::new();
        while let Some(id) = self.queue_links.next(&mut c) {
            let r = self.queue_links.get_mut(id).expect("queued");
            let delay = r.delay;
            r.delay -= 1;
            if delay > 0 {
                continue;
            }
            let Some(mut s) = r.script.take() else { continue };
            let st = runner::execute(self, &mut s);
            self.queue_links.unlink(id);
            done.push(id);
            if st == state::Execution::NpcSuspended {
                if let Ok(nid) = s.active_npc()
                    && let Some(n) = self.npc_mut(nid)
                {
                    n.active_script = Some(s);
                }
            } else if st == state::Execution::WorldSuspended {
                let delay = s.pop_int();
                self.enqueue_script(s, delay);
            } else if st == state::Execution::Suspended {
                // `script.activePlayer` errors without one; logged
                tracing::error!("Attempt to access null active_player");
            }
        }
        for id in done {
            self.queue_links.free(id);
        }
        let mut c = self.obj_delayed_links.iter(self.obj_delayed, false);
        let mut done = Vec::new();
        while let Some(id) = self.obj_delayed_links.next(&mut c) {
            let r = self.obj_delayed_links.get_mut(id).expect("delayed obj");
            let delay = r.delay;
            r.delay -= 1;
            if delay > 0 {
                continue;
            }
            let (obj, receiver, duration) = (r.obj.take(), r.receiver64, r.duration);
            self.obj_delayed_links.unlink(id);
            done.push(id);
            if let Some(obj) = obj {
                let oid = self.new_obj(obj);
                self.add_obj(oid, receiver, duration);
            }
        }
        for id in done {
            self.obj_delayed_links.free(id);
        }
        if self.player_count() > 0 {
            for nid in 0..self.npcs.capacity() as i32 {
                let Some(n) = self.npcs.get(nid as usize) else { continue };
                if !n.e.is_active || n.hunt_mode == -1 || self.rsbuf.npc_observers(nid) <= 0 {
                    continue;
                }
                let mode = n.hunt_mode;
                if self.configs.hunts.get(mode as usize).is_some_and(|h| h.kind == hunt_mode::PLAYER)
                    && let Err(e) = self.npc_hunt_all(nid, mode)
                {
                    tracing::error!("{e}");
                }
            }
        }
    }

    /// `processNpcEventQueue()`: spawn and despawn triggers.
    pub fn process_npc_event_queue(&mut self) {
        let mut c = self.npc_events.iter(self.npc_event_queue, false);
        let mut done = Vec::new();
        while let Some(id) = self.npc_events.next(&mut c) {
            let ev = self.npc_events.get(id).expect("event");
            let (nid, script) = (ev.nid, ev.script.clone());
            if self.npc(nid).is_some_and(|n| n.p.delayed) {
                continue;
            }
            self.npc_events.unlink(id);
            done.push(id);
            if self.npc(nid).is_none() {
                continue;
            }
            let s = Self::init_script(script, Some(EntityRef::Npc(nid)), None, &[]);
            if let Err(e) = self.execute_npc_script(s) {
                tracing::error!("{e}");
            }
            if nid < 0 {
                self.dead_npcs.remove(&nid);
            }
        }
        for id in done {
            self.npc_events.free(id);
        }
    }

    /// `processNpcs()`: every NPC's turn, in nid order (NPCs added during
    /// the pass with a higher nid get a turn too).
    pub fn process_npcs(&mut self) {
        for nid in 0..self.npcs.capacity() as i32 {
            if self.npcs.get(nid as usize).is_none() {
                continue;
            }
            if let Err(e) = self.npc_turn(nid) {
                tracing::error!("{e}");
                self.remove_npc(nid, 0);
            }
        }
    }

    /// Keeps an NPC despawned by `npc_del` reachable under a negative id for
    /// the rest of the tick: scripts keep using the removed NPC (the
    /// default death script reads `npc_param` and `npc_findhero` after it).
    /// ponytail: freed at cleanup, so a script that suspends across ticks
    /// after deleting its NPC loses it; hold it per script state if one does.
    pub(crate) fn keep_deleted_npc(&mut self, npc: Npc) -> i32 {
        self.next_dead_npc -= 1;
        let id = self.next_dead_npc;
        self.dead_npcs.insert(id, npc);
        self.deleted_npcs.push(id);
        id
    }

    /// The NPC half of `processCleanup()`: `resetEntity(false)`.
    pub fn cleanup_npcs(&mut self) {
        for id in std::mem::take(&mut self.deleted_npcs) {
            self.dead_npcs.remove(&id);
        }
        for nid in 0..self.npcs.capacity() as i32 {
            let Some(n) = self.npcs.get(nid as usize) else { continue };
            let face = self.face_entity_of(n.p.target);
            let n = self.n(nid);
            let (x, z, level) = (n.e.x, n.e.z, n.e.level);
            n.p.reset(move_speed::WALK, x, z, level);
            n.p.set_face_entity(face);
        }
    }
}
