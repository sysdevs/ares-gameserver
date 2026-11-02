//! `network/game/client/handler/*`: what each client packet does to its
//! player. Returns the `handle()` result (`true` counts toward the user
//! packet limit). `ClientCheatHandler` is in `cheats.rs`.

use super::World;
use super::player::InvOwner;
use crate::cache::config::component::{Component, NO_BUTTON};
use crate::cache::word_pack;
use crate::engine::coord;
use crate::io::packet::Packet;
use crate::link::friend::{FriendOut, Name37};
use crate::net::client::ClientMessage as C;
use crate::net::server::ServerMessage as M;
use crate::rsbuf::player_prot;
use crate::script::state::{self, EntityRef};
use ares_shared::trigger;
use crate::util::jstring::from_base37;

/// `ComActionTarget`.
mod action_target {
    pub const OBJ: i32 = 1;
    pub const NPC: i32 = 2;
    pub const LOC: i32 = 4;
    pub const PLAYER: i32 = 8;
    pub const HELD: i32 = 16;
}

/// `Player.DESIGN_BODY_COLORS[i].length`.
const DESIGN_COLOURS: [i32; 5] = [12, 16, 16, 6, 8];

/// An op string that is unset or `hidden` (`op[i] === null || op[i] === 'hidden'`);
/// an index past the array reads `undefined` and passes.
fn bad_op(ops: Option<&Vec<Option<String>>>, i: i32) -> bool {
    match ops {
        None => true,
        Some(ops) => usize::try_from(i).ok().and_then(|i| ops.get(i)).is_some_and(|o| o.as_deref().is_none_or(|o| o == "hidden")),
    }
}

impl World {
    fn component(&self, id: i32) -> Option<&Component> {
        self.configs.components.get(usize::try_from(id).ok()?)
    }

    /// `isComponentVisible(com)`.
    fn is_component_visible(&self, slot: i32, root: i32) -> bool {
        let p = self.player(slot).expect("player");
        p.modal_main == root || p.modal_chat == root || p.modal_side == root || p.tabs.contains(&root) || p.modal_tutorial == root
    }

    /// `getInventoryFromListener(invListeners.find(l => l.com === com))`.
    fn listener_inventory(&mut self, slot: i32, com: i32) -> Option<InvOwner> {
        let l = self.pl(slot).inv_listeners.iter().find(|l| l.com == com)?.clone();
        if l.source == -1 {
            self.world_inventory(l.kind).map(InvOwner::World)
        } else {
            let other = self.player_by_uid(l.source as i32)?;
            self.player_inventory(other, l.kind)
        }
    }

    /// `last_inv` for the held-obj pointer (D8): -1 when the listener shows
    /// another player's inventory (invother), so no `active_invobj` names it.
    fn held_inv_kind(&mut self, slot: i32, com: i32, kind: i32) -> i32 {
        match self.listener_inventory(slot, com) {
            Some(InvOwner::Player(p, _)) if p != slot => -1,
            _ => kind,
        }
    }

    /// The use-item checks shared by the `*U` ops: the inventory slot holds the obj.
    fn use_item_ok(&mut self, slot: i32, com: i32, use_slot: i32, use_obj: i32) -> bool {
        let usable = self.component(com).map(|c| (c.usable, c.root_layer));
        let Some((true, root)) = usable else { return false };
        if !self.is_component_visible(slot, root) {
            return false;
        }
        let Some(inv) = self.listener_inventory(slot, com).and_then(|o| self.inv(o)) else { return false };
        inv.valid_slot(use_slot) && inv.has_at(use_slot as usize, use_obj)
    }

    fn unset(&mut self, slot: i32) -> bool {
        self.write(slot, M::UnsetMapFlag);
        false
    }

    fn in_build_view(&self, slot: i32, x: i32, z: i32) -> bool {
        let p = self.player(slot).expect("player");
        !(x < p.origin_x - 52 || x > p.origin_x + 52 || z < p.origin_z - 52 || z > p.origin_z + 52)
    }

    fn spell_ok(&self, slot: i32, com: i32, target: i32) -> bool {
        match self.component(com) {
            Some(c) if c.action_target & target != 0 => self.is_component_visible(slot, c.root_layer),
            _ => false,
        }
    }

    fn members_item_refused(&mut self, slot: i32, obj: i32) -> bool {
        if self.configs.objs.get(obj as usize).is_some_and(|t| t.members) && !self.map.members {
            self.message_game(slot, "To use this item please login to a members' server.");
            return true;
        }
        false
    }

    /// An ENGINE interaction from an op packet.
    fn op_interaction(&mut self, slot: i32, target: EntityRef, op: i32, com: Option<i32>) -> bool {
        self.clear_pending_action(slot);
        self.player_set_interaction(slot, target, op, com);
        self.pl(slot).opcalled = true;
        true
    }

    fn npc_target_ok(&mut self, slot: i32, npc_slot: i32) -> Option<i32> {
        let n = self.npc(npc_slot)?;
        if n.p.delayed || !self.rsbuf.has_npc(slot, n.nid) {
            return None;
        }
        Some(npc_slot)
    }

    fn player_target_ok(&self, slot: i32, other: i32) -> Option<i32> {
        self.player(other)?;
        self.rsbuf.has_player(slot, other).then_some(other)
    }

    fn run_button_script(&mut self, slot: i32, trigger: i32, com: i32, root: i32, name: &str) {
        match self.scripts.get_by_trigger(trigger, com, -1) {
            Some(script) => {
                let protect = !self.component(root).is_some_and(|r| r.overlay);
                let mut s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &[]);
                self.set_held_invobj(slot, &mut s);
                if let Err(e) = self.execute_player_script(slot, s, protect, false, false) {
                    tracing::error!("{e}");
                }
            }
            None if self.debug => {
                let com_name = self.component(com).and_then(|c| c.com_name.clone()).unwrap_or_default();
                self.message_game(slot, &format!("No trigger for [{name},{com_name}]"));
            }
            None => {}
        }
    }

    fn resume_active(&mut self, slot: i32) {
        if let Some(s) = self.pl(slot).active_script.take()
            && let Err(e) = self.execute_player_script(slot, s, true, true, true)
        {
            tracing::error!("{e}");
        }
    }


    /// `notifyPlayerBan(staff, username, until)`.
    fn notify_player_ban(&mut self, staff: &str, username: &str, until: i64) {
        if let Some(other) = self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|p| p.username37 == crate::util::jstring::to_base37(username) as i64)) {
            self.pl(other).logging_out = true;
            if let Some(cid) = self.pl(other).client {
                self.write_client(cid, &M::Logout);
                self.close(cid);
            }
        }
        self.login_out.push(crate::link::login::LoginRequest::PlayerBan { staff: staff.into(), username: username.into(), until });
    }

    /// `ClientGameProtRepository.getHandler(prot)?.handle(message, player)`:
    /// `Err` where a handler throws (the player is then dropped).
    pub(crate) fn handle_client_message(&mut self, slot: i32, msg: C) -> Result<bool, String> {
        if let C::ClientCheat { input } = msg {
            return self.client_cheat(slot, input);
        }
        Ok(self.handle_message(slot, msg))
    }

    fn handle_message(&mut self, slot: i32, msg: C) -> bool {
        match msg {
            C::MoveClick { path, ctrl_held, op_click } => {
                if self.pl(slot).p.delayed {
                    return self.unset(slot);
                }
                let Some(start) = path.first() else { return false };
                let p = self.pl(slot);
                if !(0..=1).contains(&ctrl_held) || coord::distance_to_sw(p.e.x, p.e.z, start.x, start.z) > 104 {
                    self.unset_map_flag(slot);
                    self.pl(slot).user_path.clear();
                    return false;
                }
                if !op_click {
                    self.clear_pending_action(slot);
                }
                let client_routefinder = self.client_routefinder;
                let p = self.pl(slot);
                p.temp_run = if p.runenergy < 100 && ctrl_held == 1 { 0 } else { ctrl_held };
                if client_routefinder {
                    let level = p.e.level;
                    p.user_path = path.iter().map(|w| coord::pack(level, w.x, w.z)).collect();
                    let waypoints: Vec<u32> = p.user_path.iter().map(|&c| c as u32).collect();
                    p.p.queue_waypoints(&waypoints);
                    self.process_walktrigger(slot);
                } else {
                    let dest = path[path.len() - 1];
                    let (level, x, z) = (p.e.level, p.e.x, p.e.z);
                    let waypoints = self.path_finder.find_path(&self.collision, level, x, z, dest.x, dest.z, 1, 1, 1, 0, -1, true, 0, 25, crate::routefinder::collision_type::NORMAL);
                    self.pl(slot).p.queue_waypoints(&waypoints);
                }
                true
            }
            C::IfButton { com } => {
                let Some(c) = self.component(com) else { return false };
                if c.button_type == NO_BUTTON {
                    return false;
                }
                let root = c.root_layer;
                if !self.is_component_visible(slot, root) {
                    return false;
                }
                self.pl(slot).last_com = com;
                if self.pl(slot).resume_buttons.contains(&com) {
                    if self.pl(slot).active_script.as_ref().is_some_and(|s| s.execution == state::Execution::PauseButton) {
                        self.resume_active(slot);
                    }
                } else {
                    match self.scripts.get_by_trigger_specific(trigger::IF_BUTTON, com, -1) {
                        Some(script) => {
                            let protect = !self.component(root).is_some_and(|r| r.overlay);
                            let s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &[]);
                            if let Err(e) = self.execute_player_script(slot, s, protect, false, false) {
                                tracing::error!("{e}");
                            }
                        }
                        None if self.debug => {
                            let name = self.component(com).and_then(|c| c.com_name.clone()).unwrap_or_default();
                            self.message_game(slot, &format!("No trigger for [if_button,{name}]"));
                        }
                        None => {}
                    }
                }
                true
            }
            C::OpNpc { op, npc_slot } => {
                if self.pl(slot).p.delayed {
                    return self.unset(slot);
                }
                let Some(nid) = self.npc_target_ok(slot, npc_slot) else { return self.unset(slot) };
                let kind = self.npc(nid).unwrap().kind;
                if bad_op(self.configs.npcs.get(kind as usize).and_then(|t| t.op.as_ref()), op - 1) {
                    return self.unset(slot);
                }
                self.op_interaction(slot, EntityRef::Npc(nid), trigger::APNPC1 + (op - 1), None)
            }
            C::OpLoc { op, x, z, loc } => {
                if self.pl(slot).p.delayed || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let level = self.pl(slot).e.level;
                let Some(id) = self.get_loc(x, z, level, loc) else { return self.unset(slot) };
                if bad_op(self.configs.locs.get(loc as usize).and_then(|t| t.op.as_ref()), op - 1) {
                    return self.unset(slot);
                }
                self.op_interaction(slot, EntityRef::Loc(id), trigger::APLOC1 + (op - 1), None)
            }
            C::OpObj { op, x, z, obj } => {
                if self.pl(slot).p.delayed || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let (level, hash) = (self.pl(slot).e.level, self.pl(slot).hash64);
                let Some(id) = self.get_obj(x, z, level, obj, hash) else { return self.unset(slot) };
                let kind = self.obj(id).kind;
                let ops = self.configs.objs.get(kind as usize).map(|t| t.op.clone());
                if ops.as_ref().is_some_and(|o| usize::try_from(op - 1).ok().and_then(|i| o.get(i)).is_some_and(|e| e.as_deref().is_none_or(|e| e == "hidden"))) {
                    return self.unset(slot);
                }
                self.op_interaction(slot, EntityRef::Obj(id), trigger::APOBJ1 + (op - 1), None)
            }
            C::OpPlayer { op, player_slot } => {
                if self.pl(slot).p.delayed {
                    return self.unset(slot);
                }
                let Some(other) = self.player_target_ok(slot, player_slot) else { return self.unset(slot) };
                self.op_interaction(slot, EntityRef::Player(other), trigger::APPLAYER1 + (op - 1), None)
            }
            C::OpNpcT { npc_slot, spell_com } => {
                if self.pl(slot).p.delayed || !self.spell_ok(slot, spell_com, action_target::NPC) {
                    return self.unset(slot);
                }
                let Some(nid) = self.npc_target_ok(slot, npc_slot) else { return self.unset(slot) };
                self.op_interaction(slot, EntityRef::Npc(nid), trigger::APNPCT, Some(spell_com))
            }
            C::OpLocT { x, z, loc, spell_com } => {
                if self.pl(slot).p.delayed || !self.spell_ok(slot, spell_com, action_target::LOC) || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let level = self.pl(slot).e.level;
                let Some(id) = self.get_loc(x, z, level, loc) else { return self.unset(slot) };
                self.op_interaction(slot, EntityRef::Loc(id), trigger::APLOCT, Some(spell_com))
            }
            C::OpObjT { x, z, obj, spell_com } => {
                if self.pl(slot).p.delayed || !self.spell_ok(slot, spell_com, action_target::OBJ) || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let (level, hash) = (self.pl(slot).e.level, self.pl(slot).hash64);
                let Some(id) = self.get_obj(x, z, level, obj, hash) else { return self.unset(slot) };
                self.op_interaction(slot, EntityRef::Obj(id), trigger::APOBJT, Some(spell_com))
            }
            C::OpPlayerT { player_slot, spell_com } => {
                if self.pl(slot).p.delayed || !self.spell_ok(slot, spell_com, action_target::PLAYER) {
                    return self.unset(slot);
                }
                let Some(other) = self.player_target_ok(slot, player_slot) else { return self.unset(slot) };
                self.op_interaction(slot, EntityRef::Player(other), trigger::APPLAYERT, Some(spell_com))
            }
            C::OpNpcU { npc_slot, use_obj, use_slot, use_com } => {
                if self.pl(slot).p.delayed || !self.use_item_ok(slot, use_com, use_slot, use_obj) {
                    return self.unset(slot);
                }
                let Some(nid) = self.npc_target_ok(slot, npc_slot) else { return self.unset(slot) };
                self.use_on(slot, EntityRef::Npc(nid), trigger::APNPCU, use_obj, use_slot, None)
            }
            C::OpLocU { x, z, loc, use_obj, use_slot, use_com } => {
                if self.pl(slot).p.delayed || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let level = self.pl(slot).e.level;
                let Some(id) = self.get_loc(x, z, level, loc) else { return self.unset(slot) };
                if !self.use_item_ok(slot, use_com, use_slot, use_obj) {
                    return self.unset(slot);
                }
                self.use_on(slot, EntityRef::Loc(id), trigger::APLOCU, use_obj, use_slot, None)
            }
            C::OpObjU { x, z, obj, use_obj, use_slot, use_com } => {
                if self.pl(slot).p.delayed || !self.in_build_view(slot, x, z) {
                    return self.unset(slot);
                }
                let (level, hash) = (self.pl(slot).e.level, self.pl(slot).hash64);
                let Some(id) = self.get_obj(x, z, level, obj, hash) else { return self.unset(slot) };
                if !self.use_item_ok(slot, use_com, use_slot, use_obj) {
                    return self.unset(slot);
                }
                self.use_on(slot, EntityRef::Obj(id), trigger::APOBJU, use_obj, use_slot, None)
            }
            C::OpPlayerU { player_slot, use_obj, use_slot, use_com } => {
                if self.pl(slot).p.delayed || !self.use_item_ok(slot, use_com, use_slot, use_obj) {
                    return self.unset(slot);
                }
                let Some(other) = self.player_target_ok(slot, player_slot) else { return self.unset(slot) };
                self.use_on(slot, EntityRef::Player(other), trigger::APPLAYERU, use_obj, use_slot, Some(use_obj))
            }
            C::OpHeld { op, obj, slot: at, com } => self.op_held(slot, op, obj, at, com),
            C::OpHeldT { obj, slot: at, com, spell_com } => self.op_held_t(slot, obj, at, com, spell_com),
            C::OpHeldU { obj, slot: at, com, use_obj, use_slot, use_com } => self.op_held_u(slot, obj, at, com, use_obj, use_slot, use_com),
            C::InvButton { op, obj, slot: at, com } => {
                if self.pl(slot).p.delayed {
                    return false;
                }
                let Some(c) = self.component(com) else { return false };
                let (root, bad) = (c.root_layer, bad_op(c.iop.as_ref(), op - 1));
                if !self.is_component_visible(slot, root) || bad {
                    return false;
                }
                let Some(inv) = self.listener_inventory(slot, com).and_then(|o| self.inv(o)) else { return false };
                if !inv.valid_slot(at) || !inv.has_at(at as usize, obj) {
                    return false;
                }
                let kind = inv.kind;
                let kind = self.held_inv_kind(slot, com, kind);
                let p = self.pl(slot);
                p.last_item = obj;
                p.last_slot = at;
                p.last_inv = kind;
                let t = trigger::INV_BUTTON1 + (op - 1);
                let name = trigger::name(t).map_or(t.to_string(), str::to_lowercase);
                self.run_button_script(slot, t, com, root, &name);
                true
            }
            C::InvButtonD { com, slot: at, target_slot, .. } => {
                let Some(c) = self.component(com) else { return false };
                if !c.draggable && !c.swappable {
                    return false;
                }
                let root = c.root_layer;
                if !self.is_component_visible(slot, root) {
                    return false;
                }
                let Some(owner) = self.listener_inventory(slot, com) else { return false };
                let inv = self.inv(owner).unwrap();
                if !inv.valid_slot(at) || !inv.valid_slot(target_slot) || inv.get(at as usize).is_none() {
                    return false;
                }
                if self.pl(slot).p.delayed {
                    let inv = self.inv(owner).unwrap();
                    let msg = M::UpdateInvPartial { component: com, slots: vec![(at, inv.get(at as usize)), (target_slot, inv.get(target_slot as usize))] };
                    self.write(slot, msg);
                    return false;
                }
                let p = self.pl(slot);
                p.last_slot = at;
                p.last_target_slot = target_slot;
                self.run_button_script(slot, trigger::INV_BUTTOND, com, root, "inv_buttond");
                true
            }
            C::ChatSetMode { public_chat, private_chat, trade_duel } => {
                let p = self.pl(slot);
                p.public_chat = public_chat;
                p.private_chat = private_chat;
                p.trade_duel = trade_duel;
                let username = p.username.clone();
                self.friend_out.push(FriendOut::PlayerChatSetmode { username, chat_mode_private: private_chat });
                true
            }
            C::CloseModal {} => {
                self.pl(slot).request_modal_close = true;
                true
            }
            // input tracking only feeds the logger
            C::EventAppletFocus { .. } | C::EventCameraPosition { .. } | C::EventMouseClick { .. } | C::EventMouseMove { .. } => true,
            C::FriendListAdd { username } | C::FriendListDel { username } | C::IgnoreListAdd { username } | C::IgnoreListDel { username } => {
                if self.pl(slot).social_protect || from_base37(username) == "invalid_name" {
                    return false;
                }
                let (username, target) = (self.pl(slot).username.clone(), Name37(username));
                self.friend_out.push(match msg {
                    C::FriendListAdd { .. } => FriendOut::PlayerFriendslistAdd { username, target },
                    C::FriendListDel { .. } => FriendOut::PlayerFriendslistRemove { username, target },
                    C::IgnoreListAdd { .. } => FriendOut::PlayerIgnorelistAdd { username, target },
                    _ => FriendOut::PlayerIgnorelistRemove { username, target },
                });
                self.pl(slot).social_protect = true;
                true
            }
            C::IdleTimer {} => {
                if !self.debug {
                    self.pl(slot).request_idle_logout = true;
                }
                true
            }
            C::ResumePauseButton {} => {
                if !self.pl(slot).active_script.as_ref().is_some_and(|s| s.execution == state::Execution::PauseButton) {
                    return false;
                }
                self.resume_active(slot);
                true
            }
            C::ResumePCountDialog { input } => {
                let p = self.pl(slot);
                match p.active_script.as_mut() {
                    Some(s) if s.execution == state::Execution::CountDialog => s.last_int = input,
                    _ => return false,
                }
                self.resume_active(slot);
                true
            }
            C::TutClickSide { tab } => {
                if !(0..=13).contains(&tab) {
                    return false;
                }
                if let Some(script) = self.scripts.get_by_trigger_specific(trigger::TUTORIAL, -1, -1)
                    && let Err(e) = self.run_player_trigger(slot, script, true)
                {
                    tracing::error!("{e}");
                }
                true
            }
            C::ReportAbuse { offender, reason, moderator_mute } => {
                if self.pl(slot).report_abuse_protect {
                    return false;
                }
                let username = self.pl(slot).username.clone();
                let now = self.clock_ms();
                if !(0..=11).contains(&reason) {
                    self.notify_player_ban("automated", &username, now + 172800000);
                    return false;
                }
                if moderator_mute && self.pl(slot).staff_mod_level > 0 && self.production {
                    let target = from_base37(offender);
                    if let Some(o) = self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|p| p.username37 == offender)) {
                        self.pl(o).muted_until = Some(now + 172800000);
                    }
                    self.login_out.push(crate::link::login::LoginRequest::PlayerMute { staff: username.clone(), username: target, until: now + 172800000 });
                }
                // the report itself goes to the logger
                self.message_game(slot, "Thank-you, your abuse report has been received");
                self.pl(slot).report_abuse_protect = true;
                true
            }
            C::MessagePrivate { username, input } => {
                let now = self.clock_ms();
                let p = self.pl(slot);
                if p.social_protect || input.len() > 100 {
                    return false;
                }
                if p.muted_until.is_some_and(|m| m > now) {
                    return false;
                }
                if from_base37(username) == "invalid_name" {
                    let me = p.username.clone();
                    self.notify_player_ban("automated", &me, now + 172800000);
                    return false;
                }
                let mut buf = Packet::new(input.clone());
                let text = word_pack::unpack(&mut buf, input.len());
                let pm_id = (10i64 << 24) + (((self.math_random() * 255.0) as i64) << 16) + self.pm_count as i64;
                self.pm_count += 1;
                let p = self.pl(slot);
                let (me, staff, c) = (p.username.clone(), p.staff_mod_level, coord::pack(p.e.level, p.e.x, p.e.z));
                self.friend_out.push(FriendOut::PrivateMessage { username: me, staff_lvl: staff, pm_id: pm_id as i32, target: Name37(username), message: text, coord: c });
                self.pl(slot).social_protect = true;
                true
            }
            C::MessagePublic { input, colour, effect } => {
                let now = self.clock_ms();
                let p = self.pl(slot);
                if p.social_protect || !(0..=11).contains(&colour) || !(0..=2).contains(&effect) || input.len() > 100 {
                    return false;
                }
                if p.muted_until.is_some_and(|m| m > now) {
                    return false;
                }
                let mut buf = Packet::new(input.clone());
                let text = word_pack::unpack(&mut buf, input.len());
                let filtered = self.wordenc.filter(&text);
                let mut out = Packet::alloc(0);
                word_pack::pack(&mut out, &filtered);
                let p = self.pl(slot);
                p.chat_colour = Some(colour);
                p.chat_effect = Some(effect);
                p.chat_rights = Some(p.staff_mod_level.min(2));
                p.log_message = Some(text);
                p.chat_message = Some(out.data[..out.pos].to_vec());
                p.p.masks |= player_prot::CHAT;
                p.social_protect = true;
                true
            }
            C::IdkSaveDesign { gender, idkit, color } => {
                let p = self.pl(slot);
                if !p.allow_design || gender > 1 {
                    return false;
                }
                for i in 0..7 {
                    let ty = i as i32 + if gender == 1 { 7 } else { 0 };
                    let kit = idkit.get(i).copied().unwrap_or(-1);
                    if ty == 8 && kit == -1 {
                        continue;
                    }
                    match usize::try_from(kit).ok().and_then(|k| self.configs.idks.get(k)) {
                        Some(t) if !t.disable && t.kind == ty => {}
                        _ => return false,
                    }
                }
                for i in 0..5 {
                    if color.get(i).copied().unwrap_or(0) >= DESIGN_COLOURS[i] {
                        return false;
                    }
                }
                let p = self.pl(slot);
                p.gender = gender;
                for (b, &k) in p.body.iter_mut().zip(idkit.iter()) {
                    *b = k;
                }
                for (c, &v) in p.colors.iter_mut().zip(color.iter()) {
                    *c = v;
                }
                let inv = p.appearance_inv;
                self.build_appearance(slot, inv);
                true
            }
            C::ClientCheat { .. } => unreachable!("handled above"),
        }
    }

    /// The tail of the `*U` op handlers.
    fn use_on(&mut self, slot: i32, target: EntityRef, op: i32, use_obj: i32, use_slot: i32, com: Option<i32>) -> bool {
        self.clear_pending_action(slot);
        if self.members_item_refused(slot, use_obj) {
            return self.unset(slot);
        }
        let p = self.pl(slot);
        p.last_use_item = use_obj;
        p.last_use_slot = use_slot;
        self.player_set_interaction(slot, target, op, com);
        self.pl(slot).opcalled = true;
        true
    }

    fn op_held(&mut self, slot: i32, op: i32, obj: i32, at: i32, com: i32) -> bool {
        if self.pl(slot).p.delayed {
            return false;
        }
        let Some(c) = self.component(com) else { return false };
        let (operable, root) = (c.operable, c.root_layer);
        if !operable || !self.is_component_visible(slot, root) {
            return false;
        }
        let Some(inv) = self.listener_inventory(slot, com).and_then(|o| self.inv(o)) else { return false };
        if !inv.valid_slot(at) || !inv.has_at(at as usize, obj) {
            return false;
        }
        let kind = inv.kind;
        let kind = self.held_inv_kind(slot, com, kind);
        let Some(t) = self.configs.objs.get(obj as usize) else { return false };
        if usize::try_from(op - 1).ok().and_then(|i| t.iop.get(i)).is_some_and(|o| o.is_none()) {
            return false;
        }
        let (id, category, debug_name) = (t.id as i32, t.category, t.debug_name.clone().unwrap_or_default());
        let p = self.pl(slot);
        p.last_item = obj;
        p.last_slot = at;
        p.last_inv = kind;
        if root != p.modal_main {
            self.clear_pending_action(slot);
        }
        self.pl(slot).move_click_request = false;
        let trig = trigger::OPHELD1 + (op - 1);
        match self.scripts.get_by_trigger(trig, id, category) {
            Some(script) => {
                if let Err(e) = self.run_player_trigger(slot, script, true) {
                    tracing::error!("{e}");
                }
            }
            None if self.debug => {
                let name = trigger::name(trig).map_or(trig.to_string(), str::to_lowercase);
                self.message_game(slot, &format!("No trigger for [{name},{debug_name}]"));
            }
            None => {}
        }
        true
    }

    fn op_held_t(&mut self, slot: i32, obj: i32, at: i32, com: i32, spell_com: i32) -> bool {
        if self.pl(slot).p.delayed || !self.spell_ok(slot, spell_com, action_target::HELD) {
            return false;
        }
        let Some(c) = self.component(com) else { return false };
        let (usable, root) = (c.usable, c.root_layer);
        if !usable || !self.is_component_visible(slot, root) {
            return false;
        }
        let Some(inv) = self.listener_inventory(slot, com).and_then(|o| self.inv(o)) else { return false };
        if !inv.valid_slot(at) || !inv.has_at(at as usize, obj) {
            return false;
        }
        let kind = inv.kind;
        let kind = self.held_inv_kind(slot, com, kind);
        let p = self.pl(slot);
        p.last_item = obj;
        p.last_slot = at;
        p.last_inv = kind;
        self.clear_pending_action(slot);
        match self.scripts.get_by_trigger(trigger::OPHELDT, spell_com, -1) {
            Some(script) => {
                if let Err(e) = self.run_player_trigger(slot, script, true) {
                    tracing::error!("{e}");
                }
            }
            None => {
                if self.debug {
                    let name = self.component(spell_com).and_then(|c| c.com_name.clone()).unwrap_or_default();
                    self.message_game(slot, &format!("No trigger for [opheldt,{name}]"));
                }
                self.message_game(slot, "Nothing interesting happens.");
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn op_held_u(&mut self, slot: i32, obj: i32, at: i32, com: i32, use_obj: i32, use_slot: i32, use_com: i32) -> bool {
        if self.pl(slot).p.delayed || com != use_com {
            return false;
        }
        for c in [com, use_com] {
            let Some(cc) = self.component(c) else { return false };
            let (usable, root) = (cc.usable, cc.root_layer);
            if !usable || !self.is_component_visible(slot, root) {
                return false;
            }
        }
        for (c, s, o) in [(com, at, obj), (use_com, use_slot, use_obj)] {
            let Some(inv) = self.listener_inventory(slot, c).and_then(|o| self.inv(o)) else { return false };
            if !inv.valid_slot(s) {
                return false;
            }
            if !inv.has_at(s as usize, o) {
                self.pl(slot).move_click_request = false;
                self.clear_pending_action(slot);
                return false;
            }
        }
        // com == use_com: both slots are in this inv
        let kind = self.listener_inventory(slot, com).and_then(|o| self.inv(o)).map_or(-1, |i| i.kind);
        let kind = self.held_inv_kind(slot, com, kind);
        let p = self.pl(slot);
        p.last_item = obj;
        p.last_slot = at;
        p.last_inv = kind;
        p.last_use_item = use_obj;
        p.last_use_slot = use_slot;
        self.clear_pending_action(slot);
        let t = |w: &World, id: i32| w.configs.objs.get(id as usize).map(|t| (t.id as i32, t.category, t.members, t.debug_name.clone().unwrap_or_default()));
        let (Some(a), Some(b)) = (t(self, obj), t(self, use_obj)) else { return false };
        if (a.2 || b.2) && !self.map.members {
            self.message_game(slot, "To use this item please login to a members' server.");
            return false;
        }
        let swap = |w: &mut World| {
            let p = w.pl(slot);
            std::mem::swap(&mut p.last_item, &mut p.last_use_item);
            std::mem::swap(&mut p.last_slot, &mut p.last_use_slot);
        };
        let mut script = self.scripts.get_by_trigger_specific(trigger::OPHELDU, a.0, -1);
        if script.is_none() {
            script = self.scripts.get_by_trigger_specific(trigger::OPHELDU, b.0, -1);
            swap(self);
        }
        if script.is_none() && a.1 != -1 {
            script = self.scripts.get_by_trigger_specific(trigger::OPHELDU, -1, a.1);
        }
        if script.is_none() && b.1 != -1 {
            script = self.scripts.get_by_trigger_specific(trigger::OPHELDU, -1, b.1);
            swap(self);
        }
        match script {
            Some(script) => {
                if let Err(e) = self.run_player_trigger(slot, script, true) {
                    tracing::error!("{e}");
                }
            }
            None => {
                if self.debug {
                    self.message_game(slot, &format!("No trigger for [opheldu,{}]", a.3));
                }
                self.message_game(slot, "Nothing interesting happens.");
            }
        }
        true
    }
}
