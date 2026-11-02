//! The World's player phases: `processClientsIn`, `processPlayers`,
//! `processLogouts`, `processInfo`, `processClientsOut` and the player half of
//! `processCleanup`, with the `Player`/`NetworkPlayer` methods they call.

use super::World;
use super::movement::Mover;
use super::zones::EntityKey;
use crate::cache::config::inv as inv_cfg;
use crate::engine::coord;
use crate::engine::entity::pathing::{move_speed, move_strategy};
use crate::engine::entity::player::{modal, QueueType, stat};
use crate::engine::entity::{NO_RECEIVER, lifecycle};
use crate::engine::linklist::NodeId;
use crate::engine::zone::zone_index;
use crate::io::packet::Packet;
use crate::link::friend::FriendOut;
use crate::link::login::LoginRequest;
use crate::net::client as cm;
use crate::net::server::{ServerMessage as M, pack_zone_coord};
use crate::routefinder::{collision_type, flag, line, naive::find_naive_path, reach};
use crate::rsbuf::{Chat, ExactMove, NpcUpdate, PlayerUpdate, player_prot};
use crate::script::PointerType;
use crate::script::runner;
use crate::script::state::{self, Arg, EntityRef};
use ares_shared::trigger;

/// `World.TIMEOUT_NO_CONNECTION` / `TIMEOUT_NO_RESPONSE` / `AFK_EVENTRATE`.
const TIMEOUT_NO_CONNECTION: i32 = 50;
const TIMEOUT_NO_RESPONSE: i32 = 100;
const AFK_EVENTRATE: i32 = 500;

impl World {
    /// `canAccess()`.
    pub fn can_access(&self, slot: i32) -> bool {
        let p = self.player(slot).expect("player");
        if self.shutdown_tick != -1 && self.current_tick >= self.shutdown_tick {
            return true;
        }
        !p.protect && !self.busy(slot)
    }

    /// `busy()`: delayed or a main/chat modal open.
    pub fn busy(&self, slot: i32) -> bool {
        let p = self.player(slot).expect("player");
        p.p.delayed || p.modal_state & (modal::MAIN | modal::CHAT) != modal::NONE
    }

    /// `unsetMapFlag()`.
    pub fn unset_map_flag(&mut self, slot: i32) {
        self.pl(slot).p.clear_waypoints();
        self.write(slot, M::UnsetMapFlag);
    }

    /// The logout write of `catch`: `logout()` then `client.close()`.
    fn drop_client(&mut self, slot: i32) {
        if let Some(cid) = self.player(slot).and_then(|p| p.client) {
            self.write_client(cid, &M::Logout);
            self.close(cid);
        }
    }

    // ---- processClientsIn ----

    /// `processClientsIn()`.
    pub fn process_clients_in(&mut self) {
        for slot in self.player_loop() {
            if self.player(slot).is_none() {
                continue;
            }
            self.pl(slot).playtime = self.pl(slot).playtime.wrapping_add(1);
            if self.current_tick % AFK_EVENTRATE == 0 {
                let chance = if self.pl(slot).last_afk_zone == 1000 { 1.0 / 12.0 } else { 1.0 / 24.0 };
                self.pl(slot).afk_event_ready = self.math_random() < chance;
            }
            let decoded = match self.is_client_connected(slot) {
                true => self.decode_in(slot),
                false => Ok(false),
            };
            let decoded = match decoded {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("{e}");
                    self.drop_client(slot);
                    continue;
                }
            };
            if decoded {
                let p = self.pl(slot);
                if !p.user_path.is_empty() || p.opcalled {
                    if p.p.delayed {
                        self.unset_map_flag(slot);
                        continue;
                    }
                    let opcalled = p.opcalled;
                    let busy = self.busy(slot);
                    self.pl(slot).move_click_request = !(!busy && opcalled);
                }
            }
            let p = self.pl(slot);
            if let Some(chat) = p.log_message.clone() {
                let (session, c) = (p.session.clone(), coord::pack(p.e.level, p.e.x, p.e.z));
                self.friend_out.push(FriendOut::PublicMessage { session_uuid: session, coord: c, chat });
            }
        }
    }

    /// `NetworkPlayer.decodeIn()`.
    fn decode_in(&mut self, slot: i32) -> Result<bool, String> {
        let tick = self.current_tick;
        let p = self.pl(slot);
        p.user_path.clear();
        p.opcalled = false;
        let Some(cid) = p.client else { return Ok(false) };
        p.last_connected = tick;
        p.user_limit = 0;
        p.client_limit = 0;
        p.restricted_limit = 0;
        let start = self.clients.get(&cid).map_or(0, |c| c.available());
        loop {
            let p = self.pl(slot);
            if p.user_limit >= cm::USER_EVENT.1 || p.client_limit >= cm::CLIENT_EVENT.1 || p.restricted_limit >= cm::RESTRICTED_EVENT.1 {
                break;
            }
            if !self.read_packet(slot, cid)? {
                break;
            }
        }
        let read = start as i64 - self.clients.get(&cid).map_or(0, |c| c.available()) as i64;
        if read > 0
            && let Some(p) = self.player_mut(slot)
        {
            p.last_response = tick;
        }
        Ok(true)
    }

    /// `NetworkPlayer.read()`: one game packet, decoded and handled.
    fn read_packet(&mut self, slot: i32, cid: super::client::ClientId) -> Result<bool, String> {
        let Some(c) = self.clients.get_mut(&cid) else { return Ok(false) };
        if c.available() < 1 {
            return Ok(false);
        }
        if c.opcode == -1 {
            let b = c.read(1).unwrap()[0] as i32;
            c.opcode = match c.decryptor.as_mut() {
                Some(d) => (b - d.next_int()) & 0xff,
                None => b,
            };
            match cm::prot_length(c.opcode as u8) {
                Some(len) => c.waiting = len,
                None => {
                    c.opcode = -1;
                    self.close(cid);
                    return Ok(false);
                }
            }
        }
        let c = self.clients.get_mut(&cid).unwrap();
        if c.waiting == -1 {
            c.waiting = c.read(1).map_or(0, |b| b[0] as i32);
        } else if c.waiting == -2 {
            c.waiting = c.read(2).map_or(0, |b| ((b[0] as i32) << 8) | b[1] as i32);
            if c.waiting > 1600 {
                self.close(cid);
                return Ok(false);
            }
        }
        let c = self.clients.get_mut(&cid).unwrap();
        if (c.available() as i32) < c.waiting {
            return Ok(false);
        }
        let body = c.read(c.waiting.max(0) as usize).unwrap_or_default();
        let (opcode, len) = (c.opcode, c.waiting.max(0) as usize);
        let mut buf = Packet::alloc(1);
        buf.data[..body.len()].copy_from_slice(&body);
        if let Some(msg) = cm::decode(opcode as u8, &mut buf, len) {
            let category = msg.category();
            let success = self.handle_client_message(slot, msg)?;
            let p = self.pl(slot);
            if success && category == cm::USER_EVENT {
                p.user_limit += 1;
            } else if category == cm::RESTRICTED_EVENT {
                p.restricted_limit += 1;
            } else {
                p.client_limit += 1;
            }
        }
        if let Some(c) = self.clients.get_mut(&cid) {
            c.opcode = -1;
        }
        Ok(true)
    }

    // ---- processPlayers ----

    /// `processPlayers()`.
    pub fn process_players(&mut self) {
        for slot in self.player_loop() {
            if self.player(slot).is_none() {
                continue;
            }
            if let Err(e) = self.player_turn(slot) {
                tracing::error!("{e}");
                self.drop_client(slot);
            }
        }
    }

    fn player_turn(&mut self, slot: i32) -> Result<(), String> {
        let tick = self.current_tick;
        let p = self.pl(slot);
        if p.p.delayed && tick >= p.p.delayed_until {
            p.p.delayed = false;
        }
        if !p.p.delayed && p.active_script.as_ref().is_some_and(|s| s.execution == state::Execution::Suspended) {
            let s = p.active_script.take().unwrap();
            self.execute_player_script(slot, s, true, true, true)?;
        }
        self.process_queues(slot)?;
        if !self.pl(slot).logging_out {
            self.process_timers(slot, 0)?;
            self.process_timers(slot, 1)?;
        }
        self.process_engine_queue(slot)?;
        self.player_set_face_entity(slot);
        self.player_reorient_entity(slot);
        self.process_interaction(slot)?;
        self.player_reorient(slot);
        self.update_energy(slot);
        if self.pl(slot).p.masks & player_prot::EXACT_MOVE == 0 {
            self.validate_distance_walked(Mover::Player(slot));
        }
        Ok(())
    }

    /// `setFaceEntity()`: a player target is `slot + 32768`, an NPC its nid.
    pub(super) fn face_entity_of(&self, target: Option<EntityRef>) -> i32 {
        match target {
            Some(EntityRef::Player(s)) => self.player(s).map_or(-1, |p| p.slot + 32768),
            Some(EntityRef::Npc(n)) => self.npc(n).map_or(-1, |n| n.nid),
            _ => -1,
        }
    }

    pub fn player_set_face_entity(&mut self, slot: i32) {
        let target = self.pl(slot).p.target;
        let face = self.face_entity_of(target);
        self.pl(slot).p.set_face_entity(face);
    }

    fn player_reorient_entity(&mut self, slot: i32) {
        let target = self.pl(slot).p.target;
        if let Some(t @ (EntityRef::Player(_) | EntityRef::Npc(_))) = target
            && let Some((_, (x, z, w, l))) = self.entity_area(t)
        {
            self.pl(slot).p.focus(coord::fine(x, w), coord::fine(z, l), false);
        }
    }

    fn player_reorient(&mut self, slot: i32) {
        let p = self.pl(slot);
        if matches!(p.p.target, Some(EntityRef::Player(_) | EntityRef::Npc(_))) {
            return;
        }
        if p.p.target_x != -1 && p.p.steps_taken == 0 {
            let (x, z) = (p.p.target_x, p.p.target_z);
            p.p.focus(x, z, true);
            p.p.target_x = -1;
            p.p.target_z = -1;
        }
    }

    fn free_player_queue_garbage(&mut self, slot: i32) {
        if let Some(p) = self.player_mut(slot) {
            for id in std::mem::take(&mut p.queue_garbage) {
                p.queue_links.free(id);
            }
        }
    }

    /// `processQueues()`.
    fn process_queues(&mut self, slot: i32) -> Result<(), String> {
        let strong = self.queue_nodes(slot, QueueType::Normal).into_iter().any(|id| self.pl(slot).queue_links.get(id).is_some_and(|r| r.kind == QueueType::Strong));
        if strong {
            self.pl(slot).request_modal_close = true;
        }
        if self.pl(slot).request_modal_close {
            self.pl(slot).request_modal_close = false;
            self.close_modal(slot, true);
        }
        self.process_queue(slot, QueueType::Normal)?;
        self.process_queue(slot, QueueType::Weak)?;
        self.free_player_queue_garbage(slot);
        Ok(())
    }

    /// `processQueue()` / `processWeakQueue()`: the cursor caches the next
    /// request, so one appended before the end runs this tick.
    fn process_queue(&mut self, slot: i32, kind: QueueType) -> Result<(), String> {
        let p = self.pl(slot);
        let list = if kind == QueueType::Weak { p.weak_queue } else { p.queue };
        let mut c = p.queue_links.iter(list, false);
        loop {
            let Some(p) = self.player_mut(slot) else { return Ok(()) };
            let Some(id) = p.queue_links.next(&mut c) else { break };
            let logging_out = p.logging_out;
            let r = p.queue_links.get_mut(id).expect("request");
            if kind != QueueType::Weak && logging_out && r.kind == QueueType::Long && r.args.first() == Some(&Arg::Int(0)) {
                r.delay = 0;
            }
            let delay = r.delay;
            r.delay -= 1;
            if self.can_access(slot) && delay <= 0 {
                let p = self.pl(slot);
                p.queue_links.unlink(id);
                p.queue_garbage.push(id);
                let r = p.queue_links.get_mut(id).expect("request");
                if r.kind == QueueType::Long && !r.args.is_empty() {
                    r.args.remove(0);
                }
                let (script, args) = (r.script.clone(), r.args.clone());
                let s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &args);
                self.execute_player_script(slot, s, true, false, false)?;
            }
        }
        Ok(())
    }

    /// `processEngineQueue()`.
    fn process_engine_queue(&mut self, slot: i32) -> Result<(), String> {
        let p = self.pl(slot);
        let mut c = p.queue_links.iter(p.engine_queue, false);
        loop {
            let Some(p) = self.player_mut(slot) else { return Ok(()) };
            let Some(id) = p.queue_links.next(&mut c) else { break };
            let r = p.queue_links.get_mut(id).expect("request");
            let delay = r.delay;
            r.delay -= 1;
            if self.can_access(slot) && delay <= 0 {
                let r = self.pl(slot).queue_links.get(id).expect("request");
                let (script, args) = (r.script.clone(), r.args.clone());
                let s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &args);
                self.execute_player_script(slot, s, true, false, false)?;
                if let Some(p) = self.player_mut(slot) {
                    p.queue_links.unlink(id);
                    p.queue_garbage.push(id);
                }
            }
        }
        self.free_player_queue_garbage(slot);
        Ok(())
    }

    /// `processTimers(type)`: `Map` order, including timers set while running.
    fn process_timers(&mut self, slot: i32, kind: i32) -> Result<(), String> {
        let tick = self.current_tick;
        let mut i = 0;
        while let Some(p) = self.player_mut(slot) {
            let Some((_, entry)) = p.timers.get_mut(i) else { break };
            i += 1;
            let Some(t) = entry else { continue };
            if t.kind != kind {
                continue;
            }
            let due = tick >= t.clock + t.interval;
            if due && (t.kind == 1 || self.can_access(slot)) {
                let Some((_, Some(t))) = self.pl(slot).timers.get_mut(i - 1) else { continue };
                t.clock = tick;
                let (script, args, normal) = (t.script.clone(), t.args.clone(), t.kind == 0);
                let s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &args);
                self.execute_player_script(slot, s, normal, false, false)?;
            }
        }
        if let Some(p) = self.player_mut(slot) {
            p.timers.retain(|(_, t)| t.is_some());
        }
        Ok(())
    }

    // ---- interaction and movement ----

    /// `processWalktrigger()`.
    pub(crate) fn process_walktrigger(&mut self, slot: i32) {
        let p = self.pl(slot);
        if p.p.walktrigger == -1 || p.protect || p.p.delayed {
            return;
        }
        let id = p.p.walktrigger;
        p.p.walktrigger = -1;
        if let Some(script) = self.scripts.get(id) {
            let mut s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &[]);
            self.pl(slot).protect = true;
            s.pointer_add(PointerType::PActivePlayer);
            runner::execute(self, &mut s);
            if let Some(p) = self.player_mut(slot) {
                p.protect = false;
            }
            if s.pointer_get(PointerType::PActivePlayer)
                && let Some(o) = s.active_player[0].and_then(|o| self.player_mut(o))
            {
                o.protect = false;
            }
        }
    }

    fn player_area(&self, slot: i32) -> coord::Area {
        let e = &self.player(slot).expect("player").e;
        (e.x, e.z, e.width, e.length)
    }

    /// `Player.validateTarget()`.
    fn player_validate_target(&self, slot: i32) -> bool {
        let p = self.player(slot).expect("player");
        let Some(target) = p.p.target else { return false };
        if self.entity_area(target).map(|a| a.0) != Some(p.e.level) {
            return false;
        }
        if matches!(target, EntityRef::Npc(_) | EntityRef::Loc(_)) && p.p.target_subject_type != self.entity_type(target) {
            return false;
        }
        match target {
            EntityRef::Obj(o) => self.arenas.objs.get(o).is_some_and(|o| o.is_valid(Some(p.hash64))),
            EntityRef::Player(s) => self.player_is_valid(s),
            _ => self.entity_valid(target),
        }
    }

    /// `Player.isValid()`.
    pub fn player_is_valid(&self, slot: i32) -> bool {
        self.player(slot).is_some_and(|p| !p.logging_out && p.visibility == 0 && p.e.is_active)
    }

    fn player_trigger(&self, slot: i32, op: bool) -> Option<std::sync::Arc<ares_shared::script::ScriptFile>> {
        let p = self.player(slot).expect("player");
        let target = p.p.target?;
        let (mut type_id, mut category) = (-1, -1);
        let kind = self.entity_type(target);
        match target {
            EntityRef::Npc(_) => {
                let t = self.configs.npcs.get(kind as usize)?;
                (type_id, category) = (t.id as i32, t.category);
            }
            EntityRef::Loc(_) => {
                let t = self.configs.locs.get(kind as usize)?;
                (type_id, category) = (t.id as i32, t.category);
            }
            EntityRef::Obj(_) => {
                let t = self.configs.objs.get(kind as usize)?;
                (type_id, category) = (t.id as i32, t.category);
            }
            EntityRef::Player(_) => {}
        }
        if p.p.target_subject_com != -1 {
            type_id = p.p.target_subject_com;
        }
        let trigger = if op { p.p.target_op + 7 } else { p.p.target_op };
        self.scripts.get_by_trigger(trigger, type_id, category)
    }

    /// `Player.inOperableDistance(target)`.
    pub fn player_in_operable(&self, slot: i32, target: EntityRef) -> bool {
        self.player_in_operable_distance(slot, target)
    }

    fn player_in_operable_distance(&self, slot: i32, target: EntityRef) -> bool {
        let p = self.player(slot).expect("player");
        let Some((tl, (tx, tz, tw, th))) = self.entity_area(target) else { return false };
        if tl != p.e.level {
            return false;
        }
        let (x, z, level, w) = (p.e.x, p.e.z, p.e.level, p.e.width);
        let c = &self.collision;
        match target {
            EntityRef::Player(_) | EntityRef::Npc(_) => reach::reached(c, level, x, z, tx, tz, tw, th, w, 0, -2, 0),
            EntityRef::Loc(id) => {
                let loc = self.loc(id);
                let force = self.configs.locs.get(loc.kind() as usize).map_or(0, |t| t.forceapproach);
                reach::reached(c, level, x, z, tx, tz, tw, th, w, loc.angle(), loc.shape(), force)
            }
            EntityRef::Obj(_) => reach::reached(c, level, x, z, tx, tz, tw, th, w, 0, -2, 0) || reach::reached(c, level, x, z, tx, tz, tw, th, w, 0, -1, 0),
        }
    }

    /// `PathingEntity.inApproachDistance(range, target)` for a player.
    fn player_in_approach_distance(&self, slot: i32, range: i32, target: EntityRef) -> bool {
        let p = self.player(slot).expect("player");
        let Some((tl, ta)) = self.entity_area(target) else { return false };
        if tl != p.e.level {
            return false;
        }
        let me = self.player_area(slot);
        if matches!(target, EntityRef::Player(_) | EntityRef::Npc(_)) && coord::intersects(me.0, me.1, me.2, me.3, ta.0, ta.1, ta.2, ta.3) {
            return false;
        }
        coord::distance_to(me, ta) <= range && line::has_line_of_sight(&self.collision, p.e.level, me.0, me.1, ta.0, ta.1, me.2, me.3, ta.2, ta.3, flag::BLOCK_NPC_AND_PLAYERS)
    }

    /// `hasInteraction()`: following does nothing.
    fn has_interaction(&self, slot: i32) -> bool {
        let p = self.player(slot).expect("player");
        p.p.target.is_some() && p.p.target_op != trigger::APPLAYER3 && p.p.target_op != trigger::OPPLAYER3
    }

    /// `defaultOp()`.
    fn default_op(&mut self, slot: i32) {
        let op = self.player_trigger(slot, true);
        let ap = self.player_trigger(slot, false);
        if !self.production && op.is_none() && ap.is_none() {
            let p = self.pl(slot);
            let (target, subject_type, subject_com, target_op) = (p.p.target, p.p.target_subject_type, p.p.target_subject_com, p.p.target_op);
            let kind = target.map_or(-1, |t| self.entity_type(t));
            let debug_name = match target {
                Some(EntityRef::Npc(_)) => self.configs.npcs.get(kind as usize).and_then(|t| t.debug_name.clone()).unwrap_or(kind.to_string()),
                Some(EntityRef::Loc(_)) => self.configs.locs.get(kind as usize).and_then(|t| t.debug_name.clone()).unwrap_or(kind.to_string()),
                Some(EntityRef::Obj(_)) => self.configs.objs.get(kind as usize).and_then(|t| t.debug_name.clone()).unwrap_or(kind.to_string()),
                _ if (subject_com != -1 && target_op == trigger::APNPCT) || target_op == trigger::APPLAYERT || target_op == trigger::APLOCT || target_op == trigger::APOBJT => {
                    usize::try_from(subject_com).ok().and_then(|i| self.configs.components.get(i)).and_then(|c| c.com_name.clone()).unwrap_or_else(|| "[object Object]".into())
                }
                _ if subject_type != -1 => self.configs.objs.get(subject_type as usize).and_then(|t| t.debug_name.clone()).unwrap_or_else(|| "[object Object]".into()),
                _ => "_".into(),
            };
            let name = trigger::name(target_op + 7).map_or((target_op + 7).to_string(), str::to_lowercase);
            self.message_game(slot, &format!("No trigger for [{name},{debug_name}]"));
        }
        self.message_game(slot, "Nothing interesting happens.");
        self.pl(slot).p.clear_waypoints();
    }

    /// `tryInteract(allowOpScenery)`.
    fn try_interact(&mut self, slot: i32, allow_op_scenery: bool) -> Result<bool, String> {
        if self.pl(slot).p.target.is_none() || !self.has_interaction(slot) || !self.can_access(slot) {
            return Ok(false);
        }
        let op = self.player_trigger(slot, true);
        let ap = self.player_trigger(slot, false);
        let target = self.pl(slot).p.target.unwrap();
        let pathing = matches!(target, EntityRef::Player(_) | EntityRef::Npc(_));
        if let Some(op) = op
            && (pathing || allow_op_scenery)
            && self.player_in_operable_distance(slot, target)
        {
            let p = self.pl(slot);
            p.p.target = None;
            p.p.clear_waypoints();
            let s = Self::init_script(op, Some(EntityRef::Player(slot)), Some(target), &[]);
            self.execute_player_script(slot, s, true, false, false)?;
            let p = self.pl(slot);
            p.next_target = p.p.target;
            p.p.target = Some(target);
            return Ok(true);
        }
        let ap_range = self.pl(slot).p.ap_range;
        if let Some(ap) = ap
            && self.player_in_approach_distance(slot, ap_range, target)
        {
            let p = self.pl(slot);
            p.p.ap_range_called = false;
            let (waypoints, index) = (p.p.waypoints, p.p.waypoint_index);
            p.p.target = None;
            p.p.clear_waypoints();
            let s = Self::init_script(ap, Some(EntityRef::Player(slot)), Some(target), &[]);
            self.execute_player_script(slot, s, true, false, false)?;
            let p = self.pl(slot);
            p.next_target = p.p.target;
            p.p.target = Some(target);
            if p.next_target.is_some() {
                p.p.clear_waypoints();
            } else if p.p.ap_range_called {
                p.p.waypoints = waypoints;
                p.p.waypoint_index = index;
                p.p.target = Some(target);
                return Ok(false);
            }
            return Ok(true);
        }
        if self.player_in_approach_distance(slot, ap_range, target) {
            self.pl(slot).p.ap_range = -1;
            return Ok(false);
        }
        if (pathing || allow_op_scenery) && self.player_in_operable_distance(slot, target) {
            self.default_op(slot);
            return Ok(true);
        }
        Ok(false)
    }

    /// `PathingEntity.randomWalk()` for a player.
    fn player_random_walk(&mut self, slot: i32) {
        let (mut x, mut z) = (self.pl(slot).e.x, self.pl(slot).e.z);
        if self.math_random() < 0.5 {
            x += if self.math_random() < 0.5 { -1 } else { 1 };
        } else {
            z += if self.math_random() < 0.5 { -1 } else { 1 };
        }
        self.pl(slot).p.queue_waypoint(x, z);
    }

    /// `pathToPathingTarget()`.
    fn path_to_pathing_target(&mut self, slot: i32) {
        let p = self.pl(slot);
        let Some(target @ (EntityRef::Player(_) | EntityRef::Npc(_))) = p.p.target else { return };
        let follow = p.p.target_op == trigger::APPLAYER3 || p.p.target_op == trigger::OPPLAYER3;
        if p.p.is_last_waypoint() && follow {
            let (fx, fz) = match target {
                EntityRef::Player(o) => self.player(o).map_or((-1, -1), |o| (o.p.follow_x, o.p.follow_z)),
                EntityRef::Npc(n) => self.npc(n).map_or((-1, -1), |n| (n.p.follow_x, n.p.follow_z)),
                _ => (-1, -1),
            };
            self.pl(slot).p.queue_waypoint(fx, fz);
            return;
        }
        if !self.can_access(slot) {
            return;
        }
        let Some((_, ta)) = self.entity_area(target) else { return };
        let me = self.player_area(slot);
        let p = self.pl(slot);
        if p.p.move_strategy == move_strategy::NAIVE {
            if coord::intersects(me.0, me.1, me.2, me.3, ta.0, ta.1, ta.2, ta.3) {
                self.player_random_walk(slot);
                return;
            }
            if p.p.is_last_waypoint() {
                let (_, wx, wz) = coord::unpack(p.p.waypoints[0]);
                if p.p.waypoint_index == -1 || (ta.0 - wx).abs() > 1 || (ta.1 - wz).abs() > 1 {
                    let level = p.e.level;
                    let mut st = self.math_state;
                    let waypoints = find_naive_path(&self.collision, level, me.0, me.1, ta.0, ta.1, me.2, me.3, ta.2, ta.3, 0, collision_type::NORMAL, &mut || super::mulberry32(&mut st));
                    self.math_state = st;
                    self.pl(slot).p.queue_waypoints(&waypoints);
                }
            }
        } else if p.p.is_last_waypoint() {
            let level = p.e.level;
            let waypoints = self.path_finder.find_path(&self.collision, level, me.0, me.1, ta.0, ta.1, me.2, ta.2, ta.3, 0, -2, true, 0, 25, collision_type::NORMAL);
            self.pl(slot).p.queue_waypoints(&waypoints);
        }
    }

    /// `Player.updateMovement()`.
    fn player_update_movement(&mut self, slot: i32) -> bool {
        let tick = self.current_tick;
        let p = self.pl(slot);
        if p.move_click_request && self.busy(slot) {
            let p = self.pl(slot);
            if !p.queue_links.is_empty(p.queue) || !p.queue_links.is_empty(p.engine_queue) {
                return false;
            }
        }
        let p = self.pl(slot);
        if p.p.move_speed != move_speed::INSTANT {
            p.p.move_speed = if p.run != 0 { move_speed::RUN } else { move_speed::WALK };
            if p.runanim == -1 {
                p.p.move_speed = move_speed::WALK;
            } else if p.temp_run != 0 {
                p.p.move_speed = move_speed::RUN;
            }
        }
        if !self.process_movement(Mover::Player(slot)) {
            self.pl(slot).temp_run = 0;
        }
        let p = self.pl(slot);
        if p.p.steps_taken > 0 {
            p.p.last_movement = tick + 1;
        }
        p.p.steps_taken > 0
    }

    /// `processInteraction()`.
    fn process_interaction(&mut self, slot: i32) -> Result<(), String> {
        let p = self.pl(slot);
        p.p.follow_x = p.p.last_step_x;
        p.p.follow_z = p.p.last_step_z;
        p.next_target = None;
        let follow = p.p.target_op == trigger::APPLAYER3 || p.p.target_op == trigger::OPPLAYER3;
        let mut interacted = false;
        if self.pl(slot).p.target.is_some() && self.can_access(slot) {
            if !self.player_validate_target(slot) {
                self.pl(slot).p.clear_interaction();
                self.unset_map_flag(slot);
                return Ok(());
            }
            if self.client_routefinder && !follow {
                self.process_walktrigger(slot);
            }
            interacted = self.try_interact(slot, false)?;
        }
        if !interacted {
            self.path_to_pathing_target(slot);
            if self.pl(slot).p.has_waypoints() && self.can_access(slot) {
                self.process_walktrigger(slot);
            }
            if !self.pl(slot).p.has_waypoints() && follow {
                self.pl(slot).p.clear_interaction();
            }
            self.player_update_movement(slot);
            if self.pl(slot).p.target.is_some() && self.can_access(slot) && !follow {
                let steps = self.pl(slot).p.steps_taken;
                interacted = self.try_interact(slot, steps == 0)?;
                let p = self.pl(slot);
                if !interacted && !p.p.ap_range_called && !p.p.has_waypoints() && p.p.steps_taken == 0 {
                    self.message_game(slot, "I can't reach that!");
                    self.pl(slot).p.clear_interaction();
                }
            }
        }
        let p = self.pl(slot);
        if p.next_target.is_some() {
            p.p.target = p.next_target;
        } else if interacted && !p.p.ap_range_called {
            p.p.clear_interaction();
        }
        let p = self.pl(slot);
        if !p.p.has_waypoints() && p.p.steps_taken > 0 {
            self.unset_map_flag(slot);
        }
        Ok(())
    }

    /// `updateEnergy()`.
    fn update_energy(&mut self, slot: i32) {
        let p = self.pl(slot);
        if p.p.delayed {
            return;
        }
        if p.p.steps_taken < 2 {
            let recovered = p.base_levels[stat::AGILITY] as i32 / 6 + 8;
            p.runenergy = (p.runenergy + recovered).min(10000);
        } else {
            let kg = (p.runweight as f64 / 1000.0).clamp(0.0, 64.0);
            let loss = (67.0 + (67.0 * kg) / 64.0) as i32;
            p.runenergy = (p.runenergy - loss).max(0);
        }
        if p.runenergy == 0 {
            p.run = 0;
            let run_varp = self.run_varp;
            self.player_set_var(slot, run_varp, 0);
        }
        let p = self.pl(slot);
        if p.runenergy < 100 {
            p.temp_run = 0;
        }
    }

    // ---- processLogouts ----

    /// `processLogouts()`.
    pub fn process_logouts(&mut self) {
        let tick = self.current_tick;
        let shutdown = self.shutdown_tick != -1 && tick >= self.shutdown_tick;
        let mut c = self.player_loop.iter();
        while let Some(node) = self.player_loop.next(&self.player_loop_links, &mut c) {
            let slot = *self.player_loop_links.get(node).expect("loop node");
            let p = self.pl(slot);
            let mut force = false;
            if shutdown || tick - p.last_response >= TIMEOUT_NO_RESPONSE {
                p.logging_out = true;
                force = true;
            } else if tick - p.last_connected >= TIMEOUT_NO_CONNECTION {
                p.request_idle_logout = true;
            }
            if p.request_logout || p.request_idle_logout {
                if tick >= p.prevent_logout_until {
                    p.logging_out = true;
                } else if p.request_logout && let Some(m) = p.prevent_logout_message.take() {
                    self.message_game(slot, &m);
                }
                let p = self.pl(slot);
                p.request_logout = false;
                p.request_idle_logout = false;
            }
            let p = self.pl(slot);
            if p.logging_out && (force || tick >= p.prevent_logout_until) {
                self.close_modal(slot, true);
                let p = self.pl(slot);
                let discardable = p.queue_links.collect(p.queue).into_iter().all(|id| {
                    let r = p.queue_links.get(id).expect("request");
                    r.kind == QueueType::Long && r.args.first() == Some(&Arg::Int(1))
                });
                let engine_empty = p.queue_links.is_empty(p.engine_queue);
                if self.can_access(slot) && engine_empty && discardable {
                    let Some(script) = self.scripts.get_by_trigger_specific(trigger::LOGOUT, -1, -1) else {
                        tracing::error!("LOGOUT TRIGGER IS BROKEN!");
                        continue;
                    };
                    let mut s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &[]);
                    s.pointer_add(PointerType::PActivePlayer);
                    runner::execute(self, &mut s);
                    self.remove_player(slot);
                }
            }
        }
        for node in std::mem::take(&mut self.player_loop_garbage) {
            self.player_loop_links.free(node);
        }
        let now = self.clock_ms();
        let names: Vec<String> = self.logout_requests.keys().cloned().collect();
        for username in names {
            let r = self.logout_requests.get_mut(&username).unwrap();
            if r.last_attempt < now - 15000 {
                r.last_attempt = now;
                let save = r.save.clone();
                self.login_out.push(LoginRequest::PlayerLogout { username, save });
            }
        }
    }

    // ---- processInfo ----

    /// `generateAppearance()`.
    fn generate_appearance(&mut self, slot: i32) -> Vec<u8> {
        let tick = self.current_tick;
        let inv = self.pl(slot).appearance_inv;
        let worn = self.player_inventory(slot, inv).and_then(|o| self.inv(o)).cloned().unwrap_or_else(|| crate::engine::inventory::Inventory::new(self.configs.invs.get_id("worn"), 0, 0));
        let p = self.player(slot).expect("player");
        let mut s = Packet::alloc(0);
        s.p1(p.gender as i64);
        s.p1(p.headicons as i64);
        let mut skipped: Vec<i32> = Vec::new();
        for i in 0..worn.capacity {
            let Some(item) = worn.get(i) else { continue };
            let t = self.configs.objs.get(item.id as usize).expect("obj");
            for w in [t.wearpos2, t.wearpos3] {
                if w != -1 && !skipped.contains(&w) {
                    skipped.push(w);
                }
            }
        }
        for slot_i in 0..12 {
            if p.npc_id != -1 {
                s.p2(-1);
                s.p2(p.npc_id as i64);
                break;
            }
            if skipped.contains(&slot_i) {
                s.p1(0);
                continue;
            }
            match worn.get(slot_i as usize) {
                None => {
                    let part = match slot_i {
                        8 => p.body[0],
                        11 => p.body[1],
                        4 => p.body[2],
                        6 => p.body[3],
                        9 => p.body[4],
                        7 => p.body[5],
                        10 => p.body[6],
                        _ => -1,
                    };
                    if part == -1 {
                        s.p1(0);
                    } else {
                        s.p2(0x100 + part as i64);
                    }
                }
                Some(item) => s.p2(0x200 + item.id as i64),
            }
        }
        for &c in &p.colors {
            s.p1(c as i64);
        }
        for a in [p.readyanim, p.turnanim, p.walkanim, p.walkanim_b, p.walkanim_l, p.walkanim_r, p.runanim] {
            s.p2(a as i64);
        }
        s.p8(p.username37);
        s.p1(p.combat_level as i64);
        s.p2(p.skill_level as i64);
        let out = s.data[..s.pos].to_vec();
        let p = self.pl(slot);
        p.last_appearance = tick;
        p.appearance_buf = Some(out.clone());
        out
    }

    /// `processInfo()`.
    pub fn process_info(&mut self) {
        if self.player_count() == 0 {
            return;
        }
        for slot in self.player_loop() {
            self.rebuild_normal(slot, false);
            let p = self.pl(slot);
            let appearance = if p.p.masks & player_prot::APPEARANCE != 0 || p.appearance_buf.is_none() {
                self.generate_appearance(slot)
            } else {
                p.appearance_buf.clone().unwrap()
            };
            let p = self.player(slot).expect("player");
            let q = &p.p;
            let u = PlayerUpdate {
                x: p.e.x,
                y: p.e.level,
                z: p.e.z,
                origin_x: p.origin_x,
                origin_z: p.origin_z,
                tele: q.tele,
                jump: q.jump,
                run_dir: q.run_dir,
                walk_dir: q.walk_dir,
                visibility: p.visibility,
                active: p.e.is_active,
                masks: q.masks,
                appearance,
                last_appearance: p.last_appearance,
                face_entity: q.face_entity,
                face_x: q.face_square_x,
                face_z: q.face_square_z,
                orientation_x: q.face_angle_x,
                orientation_z: q.face_angle_z,
                damage_taken: q.hitmark_damage,
                damage_type: q.hitmark_type,
                damage_taken2: q.hitmark2_damage,
                damage_type2: q.hitmark2_type,
                current_hitpoints: p.levels[stat::HITPOINTS] as i32,
                base_hitpoints: p.base_levels[stat::HITPOINTS] as i32,
                anim_id: q.anim_id,
                anim_delay: q.anim_delay,
                say: q.say_message.clone(),
                chat: p.chat_message.clone().map(|bytes| Chat { bytes, color: p.chat_colour.unwrap_or(-1), effect: p.chat_effect.unwrap_or(-1), ignored: p.chat_rights.unwrap_or(0) }),
                graphic_id: q.spotanim_id,
                graphic_height: q.spotanim_height,
                graphic_delay: q.spotanim_time,
                exact_move: (q.exact_start_x != -1).then_some(ExactMove { start_x: q.exact_start_x, start_z: q.exact_start_z, end_x: q.exact_end_x, end_z: q.exact_end_z, begin: q.exact_move_start, finish: q.exact_move_end, dir: q.exact_move_facing }),
            };
            self.rsbuf.compute_player(slot, u);
        }
        for nid in 0..self.npcs.capacity() as i32 {
            let Some(n) = self.npcs.get(nid as usize) else { continue };
            let q = &n.p;
            let u = NpcUpdate {
                x: n.e.x,
                y: n.e.level,
                z: n.e.z,
                ntype: n.kind,
                tele: q.tele,
                jump: q.jump,
                run_dir: q.run_dir,
                walk_dir: q.walk_dir,
                active: n.e.is_active,
                masks: q.masks,
                face_entity: q.face_entity,
                face_x: q.face_square_x,
                face_z: q.face_square_z,
                orientation_x: q.face_angle_x,
                orientation_z: q.face_angle_z,
                damage_taken: q.hitmark_damage,
                damage_type: q.hitmark_type,
                damage_taken2: q.hitmark2_damage,
                damage_type2: q.hitmark2_type,
                current_hitpoints: n.levels[crate::engine::entity::npc::HITPOINTS] as i32,
                base_hitpoints: n.base_levels[crate::engine::entity::npc::HITPOINTS] as i32,
                anim_id: q.anim_id,
                anim_delay: q.anim_delay,
                say: q.say_message.clone(),
                graphic_id: q.spotanim_id,
                graphic_height: q.spotanim_height,
                graphic_delay: q.spotanim_time,
            };
            let nid = n.nid;
            self.rsbuf.compute_npc(nid, u);
        }
    }

    // ---- processClientsOut ----

    /// `processClientsOut()`.
    pub fn process_clients_out(&mut self) {
        for slot in self.player_loop() {
            if !self.is_client_connected(slot) {
                continue;
            }
            self.update_map(slot);
            self.update_players(slot);
            self.update_npcs(slot);
            self.update_zones(slot);
            self.update_invs(slot);
            self.update_stats(slot);
            self.update_afk_zones(slot);
            self.encode_out(slot);
        }
    }

    fn enqueue_engine_by_name(&mut self, slot: i32, name: String) {
        if let Some(script) = self.scripts.get_by_name(&name) {
            self.enqueue_player_script(slot, script, QueueType::Engine, 0, Vec::new());
        }
    }

    /// `updateMap()`.
    fn update_map(&mut self, slot: i32) {
        let p = self.pl(slot);
        let cams = std::mem::take(&mut p.camera_packets);
        let (ox, oz) = ((coord::zone(p.origin_x) - 6) << 3, (coord::zone(p.origin_z) - 6) << 3);
        for c in cams {
            let (x, z) = (c.cam_x - ox, c.cam_z - oz);
            let msg = if c.kind == 0 {
                M::CamMoveTo { x, z, height: c.height, rate: c.rotation_speed, rate2: c.rotation_multiplier }
            } else {
                M::CamLookAt { x, z, height: c.height, rate: c.rotation_speed, rate2: c.rotation_multiplier }
            };
            if c.kind <= 1 {
                self.write(slot, msg);
            }
        }
        let p = self.pl(slot);
        let (x, z, level) = (p.e.x, p.e.z, p.e.level);
        let map_zone = coord::pack(0, (x >> 6) << 6, (z >> 6) << 6);
        if p.last_map_zone != map_zone {
            if p.last_map_zone != -1 {
                let (_, lx, lz) = coord::unpack(p.last_map_zone);
                self.enqueue_engine_by_name(slot, format!("[mapzoneexit,0_{}_{}]", lx >> 6, lz >> 6));
            }
            self.enqueue_engine_by_name(slot, format!("[mapzone,0_{}_{}]", x >> 6, z >> 6));
            self.pl(slot).last_map_zone = map_zone;
        }
        let zone = coord::pack(level, (x >> 3) << 3, (z >> 3) << 3);
        let last_zone = self.pl(slot).last_zone;
        if last_zone != zone {
            self.rebuild_zones(slot);
            let multi = |w: &World, c: i32| {
                let (l, x, z) = coord::unpack(c);
                w.map.is_multi(x, z, l)
            };
            let (was, now) = (multi(self, last_zone), multi(self, zone));
            if was != now {
                self.write(slot, M::SetMultiway { hidden: now });
            }
            let trigger_name = |kind: &str, l: i32, x: i32, z: i32| format!("[{kind},{l}_{}_{}_{}_{}]", x >> 6, z >> 6, ((x & 0x3f) >> 3) << 3, ((z & 0x3f) >> 3) << 3);
            if last_zone != -1 {
                let (l, lx, lz) = coord::unpack(last_zone);
                self.enqueue_engine_by_name(slot, trigger_name("zoneexit", l, lx, lz));
            }
            self.enqueue_engine_by_name(slot, trigger_name("zone", level, (x >> 3) << 3, (z >> 3) << 3));
            self.pl(slot).last_zone = zone;
        }
    }

    /// `BuildArea.rebuildZones()`.
    fn rebuild_zones(&mut self, slot: i32) {
        let p = self.pl(slot);
        p.build_area.active_zones.clear();
        let (cx, cz) = (coord::zone(p.e.x), coord::zone(p.e.z));
        let (ox, oz) = (coord::zone(p.origin_x), coord::zone(p.origin_z));
        for x in cx - 3..=cx + 3 {
            for z in cz - 3..=cz + 3 {
                if x < ox - 6 || x > ox + 6 || z > oz + 6 || z < oz - 6 {
                    continue;
                }
                let index = zone_index(x << 3, z << 3, p.e.level);
                p.build_area.active_zones.insert(index);
            }
        }
    }

    fn client_out_pos(&self, slot: i32) -> i32 {
        self.player(slot).and_then(|p| p.client).and_then(|c| self.clients.get(&c)).map_or(0, |c| c.out_pos as i32)
    }

    fn update_players(&mut self, slot: i32) {
        let pos = self.client_out_pos(slot);
        let p = self.pl(slot);
        let (dx, dz, rebuild) = ((p.p.last_tick_x - p.e.x).abs(), (p.p.last_tick_z - p.e.z).abs(), p.p.last_level != p.e.level);
        let bytes = self.rsbuf.player_info(pos, slot, dx, dz, rebuild);
        self.write(slot, M::PlayerInfo { bytes });
    }

    fn update_npcs(&mut self, slot: i32) {
        let pos = self.client_out_pos(slot);
        let p = self.pl(slot);
        let (dx, dz, rebuild) = ((p.p.last_tick_x - p.e.x).abs(), (p.p.last_tick_z - p.e.z).abs(), p.p.last_level != p.e.level);
        let bytes = self.rsbuf.npc_info(pos, slot, dx, dz, rebuild);
        self.write(slot, M::NpcInfo { bytes });
    }

    /// `Zone.updatedThisTick(obj, player)`.
    fn obj_updated_this_tick(&self, zone: i32, obj: NodeId, hash64: i64) -> bool {
        let Some(z) = self.zone_events.get(&zone) else { return false };
        let Some(ids) = z.entity_events_of(EntityKey::Obj(obj)) else { return false };
        ids.iter().filter_map(|id| z.events.get(id)).any(|e| match e.message {
            M::ObjDel { .. } => true,
            M::ObjAdd { .. } => e.receiver64 == NO_RECEIVER || e.receiver64 == hash64,
            M::ObjReveal { .. } => e.receiver64 != hash64,
            _ => false,
        })
    }

    /// `updateZones()`.
    fn update_zones(&mut self, slot: i32) {
        let p = self.pl(slot);
        let active = p.build_area.active_zones.clone();
        p.build_area.loaded_zones.retain(|z| active.contains(z));
        let tick = self.current_tick;
        for index in active {
            self.zones.zone_by_index(index, &mut self.arenas);
            let z = &self.zones.zones[&index];
            let (zx, zz, locs, objs) = (z.x, z.z, z.locs, z.objs);
            let p = self.pl(slot);
            let (ox, oz, hash64, loaded) = (p.origin_x, p.origin_z, p.hash64, p.build_area.loaded_zones.contains(&index));
            if !loaded {
                self.write(slot, M::UpdateZoneFullFollows { zone_x: zx, zone_z: zz, origin_x: ox, origin_z: oz });
                for id in self.arenas.objs.collect(objs) {
                    let o = self.obj(id);
                    if self.obj_updated_this_tick(index, id, hash64) || (o.receiver64 != NO_RECEIVER && o.receiver64 != hash64) {
                        continue;
                    }
                    let (coord, obj, count, life, active) = (pack_zone_coord(o.e.x, o.e.z), o.kind, o.count, o.e.lifecycle, o.e.is_active);
                    self.write(slot, M::UpdateZonePartialFollows { zone_x: zx, zone_z: zz, origin_x: ox, origin_z: oz });
                    if (life == lifecycle::DESPAWN || life == lifecycle::RESPAWN) && active {
                        self.write(slot, M::ObjAdd { coord, obj, count });
                    }
                }
                for id in self.arenas.locs.collect(locs) {
                    let l = self.loc(id);
                    if l.e.last_lifecycle_tick == tick {
                        continue;
                    }
                    let (coord, kind, shape, angle) = (pack_zone_coord(l.e.x, l.e.z), l.kind(), l.shape(), l.angle());
                    let msg = match (l.e.lifecycle, l.e.is_active) {
                        (lifecycle::DESPAWN, true) => Some(M::LocAddChange { coord, loc: kind, shape, angle }),
                        (lifecycle::RESPAWN, false) => Some(M::LocDel { coord, shape, angle }),
                        (lifecycle::RESPAWN, true) if l.is_changed() => Some(M::LocAddChange { coord, loc: kind, shape, angle }),
                        _ => None,
                    };
                    if let Some(msg) = msg {
                        self.write(slot, msg);
                    }
                }
            }
            if let Some(shared) = self.zone_events.get(&index).and_then(|e| e.shared.clone()) {
                self.write(slot, M::UpdateZonePartialEnclosed { zone_x: zx, zone_z: zz, origin_x: ox, origin_z: oz, data: shared });
            }
            let follows: Vec<M> = match self.zone_events.get(&index) {
                Some(e) if !e.events.is_empty() => {
                    let mut v = vec![M::UpdateZonePartialFollows { zone_x: zx, zone_z: zz, origin_x: ox, origin_z: oz }];
                    v.extend(e.events.values().filter(|e| !e.enclosed && (e.receiver64 == NO_RECEIVER || e.receiver64 == hash64)).map(|e| e.message.clone()));
                    v
                }
                _ => Vec::new(),
            };
            for m in follows {
                self.write(slot, m);
            }
            self.pl(slot).build_area.loaded_zones.insert(index);
        }
    }

    fn com_size(&self, com: i32) -> i32 {
        usize::try_from(com).ok().and_then(|i| self.configs.components.get(i)).map_or(0, |c| c.width * c.height)
    }

    /// `updateInvs()`.
    fn update_invs(&mut self, slot: i32) {
        let mut runweight_changed = false;
        let mut first_seen = false;
        let n = self.pl(slot).inv_listeners.len();
        for i in 0..n {
            let Some(l) = self.pl(slot).inv_listeners.get(i).cloned() else { continue };
            let full = l.first_seen;
            let owner = if l.source == -1 {
                self.world_inventory(l.kind).map(super::player::InvOwner::World)
            } else {
                match self.player_by_uid(l.source as i32) {
                    Some(o) => self.player_inventory(o, l.kind),
                    None => continue,
                }
            };
            let Some(owner) = owner else { continue };
            let Some(inv) = self.inv(owner) else { continue };
            let update = inv.update;
            let msg = if full {
                Some(M::inv_full(l.com, inv, self.com_size(l.com)))
            } else if update {
                Some(M::inv_partial(l.com, inv))
            } else {
                None
            };
            if let Some(m) = msg {
                self.write(slot, m);
            }
            if full {
                self.pl(slot).inv_listeners[i].first_seen = false;
            }
            if l.source != -1 {
                if full {
                    first_seen = true;
                }
                if (update || full) && self.configs.invs.get(l.kind as usize).is_some_and(|t| t.runweight) {
                    runweight_changed = true;
                }
            }
        }
        if runweight_changed {
            let current = self.pl(slot).runweight;
            self.calculate_run_weight(slot);
            runweight_changed = current != self.pl(slot).runweight;
        }
        if runweight_changed || first_seen {
            let kg = (self.pl(slot).runweight / 1000) as i32;
            self.write(slot, M::UpdateRunWeight { kg });
        }
    }

    /// `calculateRunWeight()`.
    pub fn calculate_run_weight(&mut self, slot: i32) {
        let mut weight: i64 = 0;
        let World { players, configs, .. } = self;
        for inv in players[slot as usize].as_ref().expect("player").invs.values() {
            if !configs.invs.get(inv.kind as usize).is_some_and(|t| t.runweight) {
                continue;
            }
            for item in inv.items.iter().flatten() {
                let Some(t) = configs.objs.get(item.id as usize) else { continue };
                if t.stackable {
                    continue;
                }
                weight += t.weight as i64 * item.count;
            }
        }
        self.pl(slot).runweight = weight;
    }

    /// `updateStats()`.
    fn update_stats(&mut self, slot: i32) {
        for i in 0..21 {
            let p = self.pl(slot);
            if p.stats[i] != p.last_stats[i] || p.levels[i] != p.last_levels[i] {
                let (exp, level) = (p.stats[i], p.levels[i]);
                p.last_stats[i] = exp;
                p.last_levels[i] = level;
                self.write(slot, M::UpdateStat { stat: i as i32, exp, level: level as i32 });
            }
        }
        let p = self.pl(slot);
        if p.runenergy != p.last_run_energy {
            let energy = p.runenergy;
            p.last_run_energy = energy;
            self.write(slot, M::UpdateRunEnergy { energy });
        }
    }

    /// `updateAfkZones()`.
    fn update_afk_zones(&mut self, slot: i32) {
        let p = self.pl(slot);
        p.last_afk_zone = (p.last_afk_zone + 1).min(1000);
        let within = p.afk_zones.iter().any(|&c| {
            let (_, x, z) = coord::unpack(c);
            coord::intersects(p.e.x, p.e.z, p.e.width, p.e.length, x, z, 21, 21)
        });
        if within {
            return;
        }
        let c = coord::pack(0, p.e.x - 10, p.e.z - 10);
        if p.p.move_speed == move_speed::INSTANT && p.p.jump {
            p.afk_zones[1] = c;
        } else {
            p.afk_zones[1] = p.afk_zones[0];
        }
        p.afk_zones[0] = c;
        p.last_afk_zone = 0;
    }

    /// `encodeOut()`.
    fn encode_out(&mut self, slot: i32) {
        let p = self.pl(slot);
        if p.modal_main != p.last_modal_main || p.modal_chat != p.last_modal_chat || p.modal_side != p.last_modal_side || p.refresh_modal_close {
            let close = p.refresh_modal_close;
            p.refresh_modal_close = false;
            p.last_modal_main = p.modal_main;
            p.last_modal_chat = p.modal_chat;
            p.last_modal_side = p.modal_side;
            if close {
                self.write(slot, M::IfClose);
            }
        }
        let p = self.pl(slot);
        if p.refresh_modal {
            let (st, main, chat, side) = (p.modal_state, p.modal_main, p.modal_chat, p.modal_side);
            p.refresh_modal = false;
            let msg = if st & modal::MAIN != 0 && st & modal::SIDE != 0 {
                Some(M::IfOpenMainSide { main, side })
            } else if st & modal::MAIN != 0 {
                Some(M::IfOpenMain { component: main })
            } else if st & modal::CHAT != 0 {
                Some(M::IfOpenChat { component: chat })
            } else if st & modal::SIDE != 0 {
                Some(M::IfOpenSide { component: side })
            } else {
                None
            };
            if let Some(m) = msg {
                self.write(slot, m);
            }
        }
        let p = self.pl(slot);
        if p.overlay != p.last_overlay {
            let component = p.overlay;
            p.last_overlay = component;
            self.write(slot, M::IfOpenOverlay { component });
        }
    }

    // ---- processCleanup ----

    /// The player and world-inventory half of `processCleanup()`.
    pub fn cleanup_players(&mut self) {
        for slot in self.player_loop() {
            let p = self.pl(slot);
            let (x, z, level) = (p.e.x, p.e.z, p.e.level);
            let speed = if p.run != 0 { move_speed::RUN } else { move_speed::WALK };
            p.p.reset(speed, x, z, level);
            let target = self.pl(slot).p.target;
            let face = self.face_entity_of(target);
            let p = self.pl(slot);
            p.p.set_face_entity(face);
            p.p.repathed = false;
            p.protect = false;
            p.chat_colour = None;
            p.chat_effect = None;
            p.chat_rights = None;
            p.chat_message = None;
            p.log_message = None;
            p.social_protect = false;
            p.report_abuse_protect = false;
            for inv in p.invs.values_mut() {
                inv.reset_tracking();
            }
        }
    }

    /// The world-inventory part of `processCleanup()`: tracking reset and
    /// shop restocking.
    pub fn cleanup_world_invs(&mut self) {
        let tick = self.current_tick;
        for i in 0..self.invs.len() {
            self.invs[i].reset_tracking();
            let kind = self.invs[i].kind;
            let Some(t) = self.configs.invs.get(kind as usize) else { continue };
            let (Some(stockcount), Some(stockrate)) = (t.stockcount.clone(), t.stockrate.clone()) else { continue };
            if !t.restock {
                continue;
            }
            let allstock = t.allstock;
            for index in 0..self.invs[i].items.len() {
                let Some(item) = self.invs[i].items[index].clone() else { continue };
                let min = stockcount.get(index).copied();
                let rate = stockrate.get(index).copied();
                let due = rate.is_some_and(|r| r != 0 && tick % r == 0);
                if let Some(min) = min {
                    if item.count < min as i64 && due {
                        self.invs[i].add(&self.configs.objs, item.id, 1, index as i32);
                        self.invs[i].update = true;
                        continue;
                    }
                    if item.count > min as i64 && due {
                        self.invs[i].remove(&self.configs.invs, item.id, 1, index as i32);
                        self.invs[i].update = true;
                        continue;
                    }
                }
                if allstock && min.is_none_or(|m| m == 0) && tick % 100 == 0 {
                    self.invs[i].remove(&self.configs.invs, item.id, 1, index as i32);
                    self.invs[i].update = true;
                }
            }
        }
        let _ = inv_cfg::SCOPE_SHARED;
    }
}
