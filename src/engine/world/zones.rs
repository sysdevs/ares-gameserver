//! The World's loc/obj and zone-event operations, the loc/obj lifecycle
//! trackers (`LocObjEvent`, `Loc.turn`, `Obj.turn`) and the zone parts of `processZones`/`processCleanup`.

use super::World;
use crate::cache::wordenc::WordEnc;
use crate::engine::entity::{Loc, NO_RECEIVER, Obj, lifecycle};
use crate::engine::game_map::change_loc_collision;
use crate::engine::linklist::NodeId;
use crate::engine::zone::zone_index;
use crate::io::packet::Packet;
use crate::net::server::{ServerMessage as M, pack_zone_coord};
use std::sync::LazyLock;

/// Zone messages never need the chat filter.
static NO_WORDENC: LazyLock<WordEnc> = LazyLock::new(WordEnc::default);

/// `ZoneEvent`: `enclosed` events go into the shared per-zone block,
/// `follows` ones are written to each observer separately.
#[derive(Clone, Debug)]
pub struct ZoneEvent {
    pub enclosed: bool,
    pub receiver64: i64,
    pub message: M,
}

/// The key of `Zone.entityEvents` (a `Map` keyed by entity object).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityKey {
    Loc(NodeId),
    Obj(NodeId),
}


/// `Zone.OBJS`: a zone holds at most 129 objs before dropping a dynamic one.
const ZONE_OBJS: i32 = (64 << 1) + 1;

impl World {
    pub(crate) fn zone_of(&mut self, x: i32, z: i32, level: i32) -> i32 {
        let index = zone_index(x, z, level);
        self.zones.zone_by_index(index, &mut self.arenas);
        index
    }

    /// `trackZone`: zones to compute and reset this tick, in first-touch order.
    pub fn track_zone(&mut self, index: i32) {
        self.zones_tracking.insert(index);
    }

    fn queue_event(&mut self, zone: i32, key: Option<EntityKey>, event: ZoneEvent) {
        let id = self.next_event;
        self.next_event += 1;
        let z = self.zone_events.entry(zone).or_default();
        z.events.insert(id, event);
        if let Some(key) = key {
            z.entity_events.entry(key).or_default().push(id);
        }
    }

    fn clear_queued_events(&mut self, zone: i32, key: EntityKey) {
        if let Some(z) = self.zone_events.get_mut(&zone)
            && let Some(ids) = z.entity_events.shift_remove(&key)
        {
            for id in ids {
                z.events.shift_remove(&id);
            }
        }
    }

    pub fn loc(&self, id: NodeId) -> &Loc {
        self.arenas.locs.get(id).expect("loc")
    }

    pub fn obj(&self, id: NodeId) -> &Obj {
        self.arenas.objs.get(id).expect("obj")
    }

    pub(crate) fn loc_mut(&mut self, id: NodeId) -> &mut Loc {
        self.arenas.locs.get_mut(id).expect("loc")
    }

    pub(crate) fn obj_mut(&mut self, id: NodeId) -> &mut Obj {
        self.arenas.objs.get_mut(id).expect("obj")
    }

    /// `changeLocCollision` for a loc's current type (`add` or remove).
    fn loc_collision(&mut self, id: NodeId, add: bool) {
        let l = self.loc(id);
        let (kind, shape, angle, x, z, level) = (l.kind(), l.shape(), l.angle(), l.e.x, l.e.z, l.e.level);
        let Some(t) = self.configs.locs.get(kind as usize) else { return };
        if t.blockwalk {
            let (blockrange, length, width, active) = (t.blockrange, t.length, t.width, t.active);
            change_loc_collision(self, shape, angle, blockrange, length, width, active, x, z, level, add);
        }
    }

    /// `NonPathingEntity.setLifeCycle(duration)`: a positive duration starts
    /// a tracker on `locObjTracker`; anything else just stamps the tick.
    fn set_lifecycle(&mut self, key: EntityKey, duration: i32) {
        let tick = self.current_tick;
        let old = match key {
            EntityKey::Loc(id) => self.loc_mut(id).event_tracker.take(),
            EntityKey::Obj(id) => self.obj_mut(id).event_tracker.take(),
        };
        if let Some(old) = old {
            // unlinked but not recycled: a `processZones` pass may still hold
            // it as its cached next node (freed at the end of the pass)
            self.tracker_links.unlink(old);
            self.tracker_garbage.push(old);
        }
        let (tracker, lifecycle_tick) = if duration > 0 {
            let node = self.tracker_links.node(key);
            self.tracker_links.add_tail(self.loc_obj_tracker, node);
            (Some(node), duration.max(1))
        } else {
            (None, -1)
        };
        let e = match key {
            EntityKey::Loc(id) => {
                let l = self.loc_mut(id);
                l.event_tracker = tracker;
                &mut l.e
            }
            EntityKey::Obj(id) => {
                let o = self.obj_mut(id);
                o.event_tracker = tracker;
                &mut o.e
            }
        };
        e.lifecycle_tick = lifecycle_tick;
        e.last_lifecycle_tick = tick;
    }

    // ---- locs ----

    /// A new script loc (call [`World::add_loc`] to place it).
    pub fn new_loc(&mut self, loc: Loc) -> NodeId {
        self.arenas.locs.node(loc)
    }

    /// `World.getLoc(x, z, level, type)`: the first active loc of that type on the tile.
    pub fn get_loc(&mut self, x: i32, z: i32, level: i32, kind: i32) -> Option<NodeId> {
        let zone = self.zone_of(x, z, level);
        let list = self.zones.zones[&zone].locs;
        let coord = pack_zone_coord(x, z);
        self.arenas.locs.collect(list).into_iter().find(|&id| {
            let l = self.loc(id);
            l.e.is_active && pack_zone_coord(l.e.x, l.e.z) == coord && l.kind() == kind
        })
    }

    /// `World.addLoc(loc, duration)`.
    pub fn add_loc(&mut self, id: NodeId, duration: i32) {
        self.loc_collision(id, true);
        let l = self.loc(id);
        let (x, z, level) = (l.e.x, l.e.z, l.e.level);
        let zone = self.zone_of(x, z, level);
        // Zone.addLoc
        if self.loc(id).e.lifecycle == lifecycle::DESPAWN {
            let zz = self.zones.zones.get_mut(&zone).unwrap();
            zz.locs_count += 1;
            let list = zz.locs;
            self.arenas.locs.add_tail(list, id);
        }
        let l = self.loc_mut(id);
        l.revert();
        l.e.is_active = true;
        let msg = M::LocAddChange { coord: pack_zone_coord(x, z), loc: l.kind(), shape: l.shape(), angle: l.angle() };
        self.queue_event(zone, Some(EntityKey::Loc(id)), ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
        self.set_lifecycle(EntityKey::Loc(id), duration);
    }

    /// `World.changeLoc(loc, type, shape, angle, duration)`.
    pub fn change_loc(&mut self, id: NodeId, kind: i32, shape: i32, angle: i32, duration: i32) {
        let l = self.loc(id);
        if l.e.lifecycle == lifecycle::DESPAWN && !l.e.is_active {
            return;
        }
        if l.e.is_active {
            self.loc_collision(id, false);
        }
        self.loc_mut(id).change(kind, shape, angle);
        self.loc_collision(id, true);
        self.zone_change_loc(id);
        let l = self.loc(id);
        let keep = l.is_changed() || l.e.lifecycle == lifecycle::DESPAWN;
        self.set_lifecycle(EntityKey::Loc(id), if keep { duration } else { -1 });
    }

    /// `Zone.changeLoc`: reactivate, move to the end of the zone's list, notify.
    fn zone_change_loc(&mut self, id: NodeId) {
        let l = self.loc_mut(id);
        l.e.is_active = true;
        let (x, z, level) = (l.e.x, l.e.z, l.e.level);
        let zone = self.zone_of(x, z, level);
        let list = self.zones.zones[&zone].locs;
        self.arenas.locs.unlink(id);
        self.arenas.locs.add_tail(list, id);
        let l = self.loc(id);
        let msg = M::LocAddChange { coord: pack_zone_coord(x, z), loc: l.kind(), shape: l.shape(), angle: l.angle() };
        self.queue_event(zone, Some(EntityKey::Loc(id)), ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
    }

    /// `World.mergeLoc` (`player_slot` is the merging player's slot).
    #[allow(clippy::too_many_arguments)]
    pub fn merge_loc(&mut self, id: NodeId, player_slot: i32, start_cycle: i32, end_cycle: i32, south: i32, east: i32, north: i32, west: i32) {
        let l = self.loc(id);
        let msg = M::LocMerge {
            src_x: l.e.x,
            src_z: l.e.z,
            shape: l.shape(),
            angle: l.angle(),
            loc_id: l.kind(),
            start_cycle,
            end_cycle,
            player_slot,
            east,
            south,
            west,
            north,
        };
        let zone = self.zone_of(l.e.x, l.e.z, l.e.level);
        self.queue_event(zone, Some(EntityKey::Loc(id)), ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
    }

    /// `World.animLoc`.
    pub fn anim_loc(&mut self, id: NodeId, seq: i32) {
        let l = self.loc(id);
        let msg = M::LocAnim { coord: pack_zone_coord(l.e.x, l.e.z), shape: l.shape(), angle: l.angle(), seq };
        let zone = self.zone_of(l.e.x, l.e.z, l.e.level);
        self.queue_event(zone, Some(EntityKey::Loc(id)), ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
    }

    /// `World.removeLoc(loc, duration)`: static locs come back after `duration`.
    pub fn remove_loc(&mut self, id: NodeId, duration: i32) {
        if !self.loc(id).e.is_active {
            return;
        }
        self.loc_collision(id, false);
        let l = self.loc(id);
        let (x, z, level, life) = (l.e.x, l.e.z, l.e.level, l.e.lifecycle);
        let zone = self.zone_of(x, z, level);
        // Zone.removeLoc
        self.arenas.locs.unlink(id);
        let zz = self.zones.zones.get_mut(&zone).unwrap();
        if life == lifecycle::RESPAWN {
            let list = zz.locs;
            self.arenas.locs.add_tail(list, id);
        } else {
            zz.locs_count -= 1;
        }
        self.clear_queued_events(zone, EntityKey::Loc(id));
        let l = self.loc_mut(id);
        l.e.is_active = false;
        let msg = M::LocDel { coord: pack_zone_coord(x, z), shape: l.shape(), angle: l.angle() };
        self.queue_event(zone, Some(EntityKey::Loc(id)), ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
        self.set_lifecycle(EntityKey::Loc(id), if life == lifecycle::RESPAWN { duration } else { -1 });
    }

    /// `World.revertLoc`: back to the base type.
    pub fn revert_loc(&mut self, id: NodeId) {
        self.loc_collision(id, false);
        self.loc_mut(id).revert();
        self.loc_collision(id, true);
        self.zone_change_loc(id);
        self.set_lifecycle(EntityKey::Loc(id), -1);
    }

    // ---- objs ----

    pub fn new_obj(&mut self, obj: Obj) -> NodeId {
        self.arenas.objs.node(obj)
    }

    fn find_obj(&mut self, x: i32, z: i32, level: i32, kind: i32, matches: impl Fn(i64) -> bool) -> Option<NodeId> {
        let zone = self.zone_of(x, z, level);
        let list = self.zones.zones[&zone].objs;
        let coord = pack_zone_coord(x, z);
        self.arenas.objs.collect(list).into_iter().find(|&id| {
            let o = self.obj(id);
            o.is_valid(None) && pack_zone_coord(o.e.x, o.e.z) == coord && matches(o.receiver64) && o.kind == kind
        })
    }

    /// `World.getObj`: visible to everyone or to `receiver64`.
    pub fn get_obj(&mut self, x: i32, z: i32, level: i32, kind: i32, receiver64: i64) -> Option<NodeId> {
        self.find_obj(x, z, level, kind, |r| r == NO_RECEIVER || r == receiver64)
    }

    /// `World.getObjOfReceiver`: owned by exactly `receiver64`.
    pub fn get_obj_of_receiver(&mut self, x: i32, z: i32, level: i32, kind: i32, receiver64: i64) -> Option<NodeId> {
        self.find_obj(x, z, level, kind, |r| r == receiver64)
    }

    /// `World.addObj(obj, receiver64, duration)`: a stackable dropped obj
    /// merges into an existing one of the same owner when the sum fits.
    pub fn add_obj(&mut self, id: NodeId, receiver64: i64, duration: i32) {
        let o = self.obj(id);
        let (x, z, level, kind, life, count) = (o.e.x, o.e.z, o.e.level, o.kind, o.e.lifecycle, o.count);
        let stackable = self.configs.objs.get(kind as usize).is_some_and(|t| t.stackable);
        if stackable && life == lifecycle::DESPAWN
            && let Some(existing) = self.get_obj_of_receiver(x, z, level, kind, receiver64)
            && self.obj(existing).e.lifecycle == lifecycle::DESPAWN
        {
            let next = count + self.obj(existing).count;
            if next <= crate::engine::inventory::STACK_LIMIT {
                self.change_obj(existing, next);
                // the lifecycle is set directly, without a tracker
                self.obj_mut(existing).e.lifecycle_tick = duration;
                return;
            }
        }

        let zone = self.zone_of(x, z, level);
        // Zone.addObj
        if life == lifecycle::DESPAWN {
            if self.zones.zones[&zone].objs_count >= ZONE_OBJS {
                // make room: drop the first dynamic obj of the zone
                let list = self.zones.zones[&zone].objs;
                if let Some(old) = self.arenas.objs.collect(list).into_iter().find(|&o| self.obj(o).e.lifecycle == lifecycle::DESPAWN) {
                    self.remove_obj(old, 0);
                }
            }
            let zz = self.zones.zones.get_mut(&zone).unwrap();
            zz.objs_count += 1;
            let list = zz.objs;
            self.arenas.objs.add_tail(list, id);
        }
        self.obj_mut(id).e.is_active = true;
        let msg = M::ObjAdd { coord: pack_zone_coord(x, z), obj: kind, count };
        if life == lifecycle::RESPAWN || receiver64 == NO_RECEIVER {
            self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: true, receiver64, message: msg });
        } else if life == lifecycle::DESPAWN {
            self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: false, receiver64, message: msg });
        }
        self.track_zone(zone);
        self.set_lifecycle(EntityKey::Obj(id), duration);
        let o = self.obj_mut(id);
        if receiver64 != NO_RECEIVER {
            o.receiver64 = receiver64;
            o.reveal = Obj::REVEAL;
        } else {
            o.reveal = -1;
        }
    }

    /// `World.revealObj`: a private drop becomes public (tradeable objs only).
    pub fn reveal_obj(&mut self, id: NodeId) {
        let o = self.obj(id);
        let (x, z, level, kind) = (o.e.x, o.e.z, o.e.level, o.kind);
        let zone = self.zone_of(x, z, level);
        let (tradeable, members) = self.configs.objs.get(kind as usize).map_or((false, false), |t| (t.tradeable, t.members));
        let members_world = self.map.members;
        let o = self.obj_mut(id);
        o.last_change = -1;
        if !tradeable || (members && !members_world) || o.reveal == -1 {
            o.reveal = -1;
        } else {
            let initial = o.receiver64;
            o.receiver64 = NO_RECEIVER;
            o.reveal = -1;
            let msg = M::ObjReveal { coord: pack_zone_coord(x, z), obj: kind, count: o.count, receiver_id: self.player_slot_by_hash64(initial).unwrap_or(0) };
            self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: true, receiver64: initial, message: msg });
        }
        self.track_zone(zone);
    }

    /// `World.changeObj(obj, newCount)`.
    pub fn change_obj(&mut self, id: NodeId, new_count: i64) {
        let tick = self.current_tick;
        let o = self.obj_mut(id);
        let (x, z, level, kind, old) = (o.e.x, o.e.z, o.e.level, o.kind, o.count);
        o.count = new_count;
        o.last_change = tick;
        let receiver = o.receiver64;
        let zone = self.zone_of(x, z, level);
        let msg = M::ObjCount { coord: pack_zone_coord(x, z), obj: kind, old_count: old, new_count };
        self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: false, receiver64: receiver, message: msg });
        self.track_zone(zone);
    }

    /// `World.removeObj(obj, duration)`: static objs respawn after the
    /// player-count-scaled duration.
    pub fn remove_obj(&mut self, id: NodeId, duration: i32) {
        if !self.obj(id).e.is_active {
            return;
        }
        let o = self.obj(id);
        let (x, z, level, kind, life, receiver, last) = (o.e.x, o.e.z, o.e.level, o.kind, o.e.lifecycle, o.receiver64, o.e.last_lifecycle_tick);
        let zone = self.zone_of(x, z, level);
        let adjusted = self.scale_by_player_count(duration);
        // Zone.removeObj
        if life == lifecycle::DESPAWN {
            self.arenas.objs.unlink(id);
            self.zones.zones.get_mut(&zone).unwrap().objs_count -= 1;
        }
        self.clear_queued_events(zone, EntityKey::Obj(id));
        self.obj_mut(id).e.is_active = false;
        if last != self.current_tick {
            let msg = M::ObjDel { coord: pack_zone_coord(x, z), obj: kind };
            if life == lifecycle::RESPAWN || receiver == NO_RECEIVER {
                self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: true, receiver64: NO_RECEIVER, message: msg });
            } else if life == lifecycle::DESPAWN {
                self.queue_event(zone, Some(EntityKey::Obj(id)), ZoneEvent { enclosed: false, receiver64: receiver, message: msg });
            }
        }
        self.track_zone(zone);
        self.set_lifecycle(EntityKey::Obj(id), if duration > 0 && life == lifecycle::RESPAWN { adjusted } else { -1 });
    }

    // ---- not tied to entities ----

    /// `World.animMap`.
    pub fn anim_map(&mut self, level: i32, x: i32, z: i32, spotanim: i32, height: i32, delay: i32) {
        let zone = self.zone_of(x, z, level);
        let msg = M::MapAnim { coord: pack_zone_coord(x, z), spotanim, height, delay };
        self.queue_event(zone, None, ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
    }

    /// `World.mapProjAnim`.
    #[allow(clippy::too_many_arguments)]
    pub fn map_proj_anim(&mut self, level: i32, x: i32, z: i32, dst_x: i32, dst_z: i32, target: i32, spotanim: i32, src_height: i32, dst_height: i32, start_delay: i32, end_delay: i32, peak: i32, arc: i32) {
        let zone = self.zone_of(x, z, level);
        let msg = M::MapProjAnim { src_x: x, src_z: z, dst_x, dst_z, target, spotanim, src_height, dst_height, start_delay, end_delay, peak, arc };
        self.queue_event(zone, None, ZoneEvent { enclosed: true, receiver64: -1, message: msg });
        self.track_zone(zone);
    }

    // ---- per tick ----

    /// `Loc.turn()`: one tick of a tracked loc's lifecycle.
    fn turn_loc(&mut self, id: NodeId) {
        let l = self.loc_mut(id);
        l.e.lifecycle_tick -= 1;
        let (tick, life, active, changed) = (l.e.lifecycle_tick, l.e.lifecycle, l.e.is_active, l.is_changed());
        if tick == 0 {
            if life == lifecycle::DESPAWN && active {
                self.remove_loc(id, 0);
            } else if life == lifecycle::RESPAWN && changed && active {
                self.revert_loc(id);
            } else if life == lifecycle::RESPAWN && !active {
                self.add_loc(id, 0);
            } else {
                tracing::error!("Loc is tracked but there is no event. Type: {}", self.loc(id).kind());
                self.set_lifecycle(EntityKey::Loc(id), -1);
            }
        } else if tick < 0 {
            tracing::error!("Loc is tracked but has a negative lifecycle tick");
            self.set_lifecycle(EntityKey::Loc(id), -1);
        }
    }

    /// `Obj.turn()`: reveal countdown, then lifecycle.
    fn turn_obj(&mut self, id: NodeId) {
        let o = self.obj_mut(id);
        if o.reveal > -1 {
            o.reveal -= 1;
            if o.reveal == 0 {
                self.reveal_obj(id);
            }
        }
        let o = self.obj_mut(id);
        o.e.lifecycle_tick -= 1;
        let (tick, life, active) = (o.e.lifecycle_tick, o.e.lifecycle, o.e.is_active);
        if tick == 0 {
            if life == lifecycle::DESPAWN && active {
                self.remove_obj(id, 0);
            } else if life == lifecycle::RESPAWN && !active {
                self.add_obj(id, NO_RECEIVER, 0);
            } else {
                self.set_lifecycle(EntityKey::Obj(id), -1);
                tracing::error!("Obj is tracked but has no event");
            }
        } else if tick < 0 {
            self.set_lifecycle(EntityKey::Obj(id), -1);
            tracing::error!("Obj is tracked but has a negative lifecycle tick");
        }
    }

    /// `processZones`: tick every tracked loc/obj, then build each tracked
    /// zone's shared enclosed block.
    pub fn process_zones(&mut self) {
        let mut c = self.tracker_links.iter(self.loc_obj_tracker, false);
        while let Some(event) = self.tracker_links.next(&mut c) {
            let key = *self.tracker_links.get(event).expect("tracker");
            // `LocObjEvent.check()`: still the entity's current tracker
            let current = match key {
                EntityKey::Loc(id) => self.loc(id).event_tracker,
                EntityKey::Obj(id) => self.obj(id).event_tracker,
            };
            if current != Some(event) {
                tracing::error!("Loc Obj event is invalid");
                continue;
            }
            match key {
                EntityKey::Loc(id) => self.turn_loc(id),
                EntityKey::Obj(id) => self.turn_obj(id),
            }
        }
        for node in std::mem::take(&mut self.tracker_garbage) {
            self.tracker_links.free(node);
        }
        let mut i = 0;
        while i < self.zones_tracking.len() {
            let zone = self.zones_tracking[i];
            self.compute_shared(zone);
            i += 1;
        }
    }

    /// `Zone.computeShared`: the enclosed events, each prot id + payload.
    fn compute_shared(&mut self, zone: i32) {
        let Some(z) = self.zone_events.get_mut(&zone) else { return };
        let mut buf = Packet::with_len(5000);
        for e in z.events.values().filter(|e| e.enclosed) {
            e.message.enclose(&mut buf, &NO_WORDENC);
        }
        z.shared = (buf.pos > 0).then(|| buf.data[..buf.pos].to_vec());
    }

    /// The zone part of `processCleanup`: reset every tracked zone.
    pub fn cleanup_zones(&mut self) {
        for zone in std::mem::take(&mut self.zones_tracking) {
            if let Some(z) = self.zone_events.get_mut(&zone) {
                z.shared = None;
                z.events.clear();
                z.entity_events.clear();
            }
        }
    }

    /// `scaleByPlayerCount(rate)`.
    pub fn scale_by_player_count(&self, rate: i32) -> i32 {
        let players = (self.player_count() as i64).min(2000);
        (((4000 - players) * rate as i64) / 4000) as i32
    }
}
