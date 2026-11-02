//! Player behaviour that needs the World: output,
//! login and logout, scripts and their queues, timers, modals, inventories,
//! vars and stats. The per-tick phases are in `player_tick.rs`.

use super::World;
use super::client::ClientId;
use super::movement::Mover;
use crate::cache::config::inv as inv_cfg;
use crate::cache::config::script_var_type as svt;
use crate::engine::entity::player::{InvListener, Player, PlayerTimer, modal, QueueType};
use crate::engine::inventory::Inventory;
use crate::engine::linklist::NodeId;
use crate::engine::xp::{get_exp_by_level, get_level_by_exp};
use crate::link::friend::FriendOut;
use crate::link::login::LoginRequest;
use crate::net::server::{ServerMessage as M, frame};
use crate::rsbuf::player_prot;
use crate::script::PointerType;
use ares_shared::script::ScriptFile;
use crate::script::runner;
use crate::script::state::{self, Arg, EntityRef, ScriptState};
use ares_shared::trigger;
use std::sync::Arc;

/// Where an inventory lives (`getInventory` resolves to one of these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvOwner {
    World(usize),
    Player(i32, i32),
}

/// `World.logoutRequests` entry.
#[derive(Clone, Debug)]
pub struct LogoutRequest {
    pub save: Vec<u8>,
    pub last_attempt: i64,
}

/// `PlayerStatEnabled` / `PlayerStatFree`.
const STAT_ENABLED: [bool; 21] = [true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, false, false, true];

impl World {
    pub fn player(&self, slot: i32) -> Option<&Player> {
        self.players.get(usize::try_from(slot).ok()?)?.as_deref()
    }

    pub fn player_mut(&mut self, slot: i32) -> Option<&mut Player> {
        self.players.get_mut(usize::try_from(slot).ok()?)?.as_deref_mut()
    }

    pub(crate) fn pl(&mut self, slot: i32) -> &mut Player {
        self.player_mut(slot).expect("player")
    }

    /// `getTotalPlayers()`: occupied slots 1..2046.
    pub fn player_count(&self) -> usize {
        self.players.iter().take(2047).skip(1).filter(|p| p.is_some()).count()
    }

    /// `getNextPlayerSlot()`: `None` when the world is full.
    pub fn next_player_slot(&self) -> Option<i32> {
        (1..2047).find(|&i| self.players[i].is_none()).map(|i| i as i32)
    }

    /// The players in `playerLoop` order.
    pub fn player_loop(&self) -> Vec<i32> {
        let mut c = self.player_loop.iter();
        let mut out = Vec::new();
        while let Some(n) = self.player_loop.next(&self.player_loop_links, &mut c) {
            out.push(*self.player_loop_links.get(n).expect("loop node"));
        }
        out
    }

    /// `getPlayerByUsername(username)` (online or joining).
    pub fn player_by_username(&self, username: &str) -> Option<&Player> {
        let name37 = crate::util::jstring::to_base37(username) as i64;
        self.player_loop().into_iter().filter_map(|s| self.player(s)).find(|p| p.username37 == name37).or_else(|| self.new_players.iter().find(|p| p.username37 == name37))
    }

    pub fn player_by_username_mut(&mut self, username: &str) -> Option<&mut Player> {
        let name37 = crate::util::jstring::to_base37(username) as i64;
        let slot = self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|p| p.username37 == name37));
        match slot {
            Some(s) => Some(self.pl(s)),
            None => self.new_players.iter_mut().find(|p| p.username37 == name37),
        }
    }

    /// `getPlayerByHash64(hash64)?.slot`.
    pub fn player_slot_by_hash64(&self, hash64: i64) -> Option<i32> {
        self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|p| p.hash64 == hash64))
    }

    /// `getPlayerByUid(uid)`.
    pub fn player_by_uid(&self, uid: i32) -> Option<i32> {
        let slot = uid & 0x7ff;
        let hash = (uid >> 11) & 0x1fffff;
        self.player(slot).filter(|p| (p.username37 & 0x1fffff) as i32 == hash).map(|_| slot)
    }

    pub fn is_client_connected(&self, slot: i32) -> bool {
        self.player(slot).is_some_and(|p| p.client.is_some())
    }

    /// `player.write(message)`: framed (and ISAAC-encrypted) when connected.
    pub fn write(&mut self, slot: i32, msg: M) {
        let Some(cid) = self.player(slot).and_then(|p| p.client) else { return };
        self.write_client(cid, &msg);
    }

    /// `writeInner(message)`.
    pub(crate) fn write_client(&mut self, cid: ClientId, msg: &M) {
        let World { clients, wordenc, out_buf, outbox, .. } = self;
        let isaac = clients.get_mut(&cid).and_then(|c| c.encryptor.as_mut());
        let bytes = frame(msg, isaac, wordenc, out_buf);
        if let Some(c) = clients.get_mut(&cid) {
            c.out_pos = bytes.len();
        }
        outbox.push((cid, super::client::Outgoing::Data(bytes)));
    }

    pub fn message_game(&mut self, slot: i32, msg: &str) {
        self.write(slot, M::MessageGame { msg: msg.into() });
    }

    /// `wrappedMessageGame(mes)`: split to 456px in font 1.
    pub fn wrapped_message_game(&mut self, slot: i32, mes: &str) {
        let lines = match self.configs.fonts.get(1) {
            Some(f) => f.split(mes, 456),
            None => vec![mes.to_string()],
        };
        for l in lines {
            self.message_game(slot, &l);
        }
    }

    // ---- vars ----

    fn write_varp(&mut self, slot: i32, id: i32, value: i32) {
        if (-128..=127).contains(&value) {
            self.write(slot, M::VarpSmall { varp: id, value });
        } else {
            self.write(slot, M::VarpLarge { varp: id, value });
        }
    }

    /// `setVar(id, value)` for an int value.
    pub fn player_set_var(&mut self, slot: i32, id: i32, value: i32) {
        let Some(t) = self.configs.varps.get(id as usize) else { return };
        if t.kind == svt::STRING {
            return;
        }
        let (vid, transmit) = (t.id, t.transmit);
        if let Some(v) = self.pl(slot).vars.get_mut(vid) {
            *v = value;
        }
        if transmit {
            self.write_varp(slot, id, value);
        }
    }

    // ---- inventories ----

    /// `World.getInventory(inv)`: the shared inventory, created on first use.
    pub fn world_inventory(&mut self, inv: i32) -> Option<usize> {
        if inv == -1 {
            return None;
        }
        if let Some(i) = self.invs.iter().position(|i| i.kind == inv) {
            return Some(i);
        }
        self.invs.push(Inventory::from_type(&self.configs.invs, inv));
        Some(self.invs.len() - 1)
    }

    /// `player.getInventory(inv)`.
    pub fn player_inventory(&mut self, slot: i32, inv: i32) -> Option<InvOwner> {
        if inv == -1 {
            return None;
        }
        let t = self.configs.invs.get(usize::try_from(inv).ok()?)?;
        if t.scope == inv_cfg::SCOPE_SHARED {
            return self.world_inventory(inv).map(InvOwner::World);
        }
        let invs = &self.configs.invs;
        self.players.get_mut(slot as usize)?.as_mut()?.own_inventory(invs, inv);
        Some(InvOwner::Player(slot, inv))
    }

    pub fn inv(&self, owner: InvOwner) -> Option<&Inventory> {
        match owner {
            InvOwner::World(i) => self.invs.get(i),
            InvOwner::Player(s, inv) => self.player(s)?.invs.get(&inv),
        }
    }

    /// An inventory and the configs its `add`/`remove` need.
    pub fn inv_with_configs(&mut self, owner: InvOwner) -> Option<(&mut Inventory, &crate::engine::configs::Configs)> {
        let World { invs, players, configs, .. } = self;
        let inv = match owner {
            InvOwner::World(i) => invs.get_mut(i)?,
            InvOwner::Player(s, inv) => players.get_mut(s as usize)?.as_mut()?.invs.get_mut(&inv)?,
        };
        Some((inv, configs))
    }

    pub fn inv_mut(&mut self, owner: InvOwner) -> Option<&mut Inventory> {
        match owner {
            InvOwner::World(i) => self.invs.get_mut(i),
            InvOwner::Player(s, inv) => self.player_mut(s)?.invs.get_mut(&inv),
        }
    }

    /// `invListenOnCom(inv, com, source)`.
    pub fn inv_listen_on_com(&mut self, slot: i32, inv: i32, com: i32, mut source: i64) {
        if inv == -1 {
            return;
        }
        let shared = self.configs.invs.get(inv as usize).is_some_and(|t| t.scope == inv_cfg::SCOPE_SHARED);
        let p = self.pl(slot);
        if p.inv_listeners.iter().any(|l| l.kind == inv && l.com == com) {
            return;
        }
        if let Some(i) = p.inv_listeners.iter().position(|l| l.com == com) {
            p.inv_listeners.remove(i);
        }
        if shared {
            source = -1;
        }
        p.inv_listeners.push(InvListener { kind: inv, com, source, first_seen: true });
    }

    /// `invStopListenOnCom(com)`.
    pub fn inv_stop_listen_on_com(&mut self, slot: i32, com: i32) {
        let p = self.pl(slot);
        let Some(i) = p.inv_listeners.iter().position(|l| l.com == com) else { return };
        p.inv_listeners.remove(i);
        self.write(slot, M::UpdateInvStopTransmit { component: com });
    }

    // ---- stats ----

    /// `getCombatLevel()` and `buildAppearance` when it changed.
    fn refresh_combat_level(&mut self, slot: i32) {
        let p = self.pl(slot);
        let level = p.compute_combat_level();
        if p.combat_level != level {
            p.combat_level = level;
            let inv = p.appearance_inv;
            self.build_appearance(slot, inv);
        }
    }

    pub fn build_appearance(&mut self, slot: i32, inv: i32) {
        let p = self.pl(slot);
        p.appearance_inv = inv;
        p.p.masks |= player_prot::APPEARANCE;
    }

    /// `setLevel(stat, level)`.
    pub fn player_set_level(&mut self, slot: i32, stat: usize, level: i32) {
        let level = level.clamp(1, 99);
        let p = self.pl(slot);
        p.base_levels[stat] = level as u8;
        p.levels[stat] = level as u8;
        p.stats[stat] = get_exp_by_level(level);
        self.refresh_combat_level(slot);
    }

    /// `addXp(stat, xp, allowMulti)`.
    pub fn player_add_xp(&mut self, slot: i32, stat: usize, xp: i32, allow_multi: bool) -> Result<(), String> {
        if xp < 0 {
            return Err(format!("Invalid xp parameter for addXp call: Stat was: {stat}, Exp was: {xp}"));
        }
        if xp == 0 {
            return Ok(());
        }
        let multi = if allow_multi { self.xp_rate } else { 1.0 };
        let p = self.pl(slot);
        let sum = p.stats[stat] as f64 + xp as f64 * multi;
        p.stats[stat] = crate::util::numbers::wrap_f64(sum.min(2_000_000_000.0));
        let before = p.base_levels[stat];
        if p.levels[stat] == p.base_levels[stat] {
            p.levels[stat] = get_level_by_exp(p.stats[stat]) as u8;
        }
        p.base_levels[stat] = get_level_by_exp(p.stats[stat]) as u8;
        if p.base_levels[stat] > before {
            if p.levels[stat] < before {
                p.levels[stat] = p.levels[stat].wrapping_add(p.base_levels[stat] - before);
            }
            self.change_stat(slot, stat as i32);
            let _total: i32 = (0..21).filter(|&i| STAT_ENABLED[i]).map(|i| self.pl(slot).base_levels[i] as i32).sum();
            if let Some(script) = self.scripts.get_by_trigger_specific(trigger::ADVANCESTAT, stat as i32, -1) {
                self.enqueue_player_script(slot, script, QueueType::Engine, 0, Vec::new());
            }
        }
        self.refresh_combat_level(slot);
        Ok(())
    }

    /// `changeStat(stat)`.
    pub fn change_stat(&mut self, slot: i32, stat: i32) {
        if let Some(script) = self.scripts.get_by_trigger(trigger::CHANGESTAT, stat, -1) {
            self.enqueue_player_script(slot, script, QueueType::Engine, 0, Vec::new());
        }
    }

    // ---- queues and timers ----

    /// `enqueueScript(script, type, delay, args)`.
    pub fn enqueue_player_script(&mut self, slot: i32, script: Arc<ScriptFile>, kind: QueueType, delay: i32, args: Vec<Arg>) {
        self.pl(slot).enqueue_script(script, kind, delay, args);
    }

    /// `setTimer(type, script, args, interval)`: a `Map.set` keeps the
    /// position of an existing timer.
    pub fn set_player_timer(&mut self, slot: i32, kind: i32, script: Arc<ScriptFile>, args: Vec<Arg>, interval: i32) {
        let clock = self.current_tick;
        let id = script.id as i32;
        let timer = PlayerTimer { kind, script, args, interval, clock };
        let p = self.pl(slot);
        match p.timers.iter_mut().find(|(k, t)| *k == id && t.is_some()) {
            Some((_, t)) => *t = Some(timer),
            None => p.timers.push((id, Some(timer))),
        }
    }

    pub fn clear_player_timer(&mut self, slot: i32, id: i32) {
        if let Some((_, t)) = self.pl(slot).timers.iter_mut().find(|(k, t)| *k == id && t.is_some()) {
            *t = None;
        }
    }

    // ---- scripts ----

    /// `runScript(script, protect, force)`: `None` when protected access
    /// was refused.
    fn run_player_script(&mut self, slot: i32, s: &mut ScriptState, protect: bool, force: bool) -> Option<state::Execution> {
        let p = self.pl(slot);
        if !force && protect && (p.protect || p.p.delayed) {
            return None;
        }
        if protect {
            s.pointer_add(PointerType::PActivePlayer);
            p.protect = true;
        }
        let st = runner::execute(self, s);
        if protect && let Some(p) = self.player_mut(slot) {
            p.protect = false;
        }
        for (ptr, i) in [(PointerType::PActivePlayer, 0), (PointerType::PActivePlayer2, 1)] {
            if s.pointer_get(ptr)
                && let Some(other) = s.active_player[i]
            {
                s.pointer_remove(ptr);
                if let Some(o) = self.player_mut(other) {
                    o.protect = false;
                }
            }
        }
        Some(st)
    }

    /// `executeScript(script, protect, force)` on player `slot`. `was_active`
    /// says whether `script` is the player's `activeScript` being resumed.
    pub fn execute_player_script(&mut self, slot: i32, mut s: Box<ScriptState>, protect: bool, force: bool, was_active: bool) -> Result<(), String> {
        let Some(st) = self.run_player_script(slot, &mut s, protect, force) else { return Ok(()) };
        if st != state::Execution::Finished && st != state::Execution::Aborted {
            if st == state::Execution::WorldSuspended {
                let delay = s.pop_int();
                self.enqueue_script(s, delay);
            } else if st == state::Execution::NpcSuspended {
                let nid = s.active_npc()?;
                if let Some(n) = self.npc_mut(nid) {
                    n.active_script = Some(s);
                }
            } else {
                let other = s.active_player()?;
                if let Some(o) = self.player_mut(other) {
                    o.active_script = Some(s);
                    o.protect = protect;
                }
            }
        } else if was_active && let Some(p) = self.player_mut(slot) {
            p.resume_buttons.clear();
            if p.modal_state & modal::MAIN == modal::NONE {
                self.close_modal(slot, false);
            }
        }
        Ok(())
    }

    /// `ScriptRunner.init(script, player)` + `executeScript` (no target).
    pub fn run_player_trigger(&mut self, slot: i32, script: Arc<ScriptFile>, protect: bool) -> Result<(), String> {
        let mut s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &[]);
        self.set_held_invobj(slot, &mut s);
        self.execute_player_script(slot, s, protect, false, false)
    }

    /// opheld1-5/u/t and inv_button1-5 start on the clicked slot (D8).
    pub(crate) fn set_held_invobj(&self, slot: i32, s: &mut ScriptState) {
        let held = (trigger::OPHELD1..=trigger::OPHELDT).contains(&s.trigger) || (trigger::INV_BUTTON1..=trigger::INV_BUTTON5).contains(&s.trigger);
        let Some(p) = self.player(slot).filter(|p| held && p.last_inv != -1 && p.last_slot >= 0) else { return };
        s.set_active_invobj(state::InvObjRef { pid: slot, inv: p.last_inv, slot: p.last_slot as usize });
    }

    // ---- modals ----

    fn if_close_trigger(&mut self, slot: i32, com: i32) {
        if let Some(script) = self.scripts.get_by_trigger(trigger::IF_CLOSE, com, -1)
            && let Err(e) = self.run_player_trigger(slot, script, false)
        {
            tracing::error!("{e}");
        }
    }

    /// `clearComListeners(root)`.
    fn clear_com_listeners(&mut self, slot: i32, root: i32) {
        if root == -1 {
            return;
        }
        let coms: Vec<i32> = self.pl(slot).inv_listeners.iter().map(|l| l.com).collect();
        for com in coms {
            if usize::try_from(com).ok().and_then(|i| self.configs.components.get(i)).is_some_and(|c| c.root_layer == root) {
                self.inv_stop_listen_on_com(slot, com);
            }
        }
    }

    /// `closeModal(clearWeakQueue)`.
    pub fn close_modal(&mut self, slot: i32, clear_weak_queue: bool) {
        let p = self.pl(slot);
        if clear_weak_queue {
            for id in p.queue_links.collect(p.weak_queue) {
                p.queue_links.unlink(id);
                p.queue_garbage.push(id);
            }
        }
        if !p.p.delayed {
            p.protect = false;
        }
        if p.modal_state == modal::NONE {
            return;
        }
        p.modal_state = modal::NONE;
        if p.active_script.as_ref().is_some_and(|s| s.execution == state::Execution::CountDialog || s.execution == state::Execution::PauseButton) {
            p.active_script = None;
            p.resume_buttons.clear();
        }
        for which in 0..3 {
            let p = self.pl(slot);
            let com = [p.modal_main, p.modal_chat, p.modal_side][which];
            if com == -1 {
                continue;
            }
            self.if_close_trigger(slot, com);
            self.clear_com_listeners(slot, com);
            let p = self.pl(slot);
            match which {
                0 => p.modal_main = -1,
                1 => p.modal_chat = -1,
                _ => p.modal_side = -1,
            }
        }
        self.pl(slot).refresh_modal_close = true;
    }

    // ---- login / logout ----

    /// `BuildArea.rebuildNormal(reconnect)`.
    pub fn rebuild_normal(&mut self, slot: i32, reconnect: bool) {
        let tick = self.current_tick;
        let p = self.pl(slot);
        let (ox, oz) = (crate::engine::coord::zone(p.origin_x), crate::engine::coord::zone(p.origin_z));
        let (left, right, top, bottom) = ((ox - 4) << 3, (ox + 5) << 3, (oz + 5) << 3, (oz - 4) << 3);
        let (x, z) = (p.e.x, p.e.z);
        if x < left || z < bottom || x > right - 1 || z > top - 1 || reconnect {
            let (zx, zz) = (crate::engine::coord::zone(x), crate::engine::coord::zone(z));
            p.build_area.mapsquares.clear();
            for bx in zx - 6..=zx + 6 {
                let mx = (bx << 3) >> 6;
                for bz in zz - 6..=zz + 6 {
                    let mz = (bz << 3) >> 6;
                    p.build_area.mapsquares.insert((mx << 8) | mz);
                }
            }
            p.origin_x = x;
            p.origin_z = z;
            p.build_area.loaded_zones.clear();
            p.build_area.last_build = tick;
            self.write(slot, M::RebuildNormal { zone_x: zx, zone_z: zz });
        }
    }

    /// `onLogin()`.
    fn on_login(&mut self, slot: i32) -> Result<(), String> {
        self.rebuild_normal(slot, false);
        let p = self.pl(slot);
        let (public_chat, private_chat, trade_duel, members) = (p.public_chat, p.private_chat, p.trade_duel, p.members);
        self.write(slot, M::ChatFilterSettings { public_chat, private_chat, trade_duel });
        if self.friend_enabled {
            self.write(slot, M::FriendlistLoaded { status: 1 });
        } else {
            self.write(slot, M::FriendlistLoaded { status: 2 });
            self.write(slot, M::UpdateIgnoreList { names: Vec::new() });
        }
        self.write(slot, M::IfClose);
        self.write(slot, M::UpdatePid { uid: slot, members });
        self.write(slot, M::ResetClientVarCache);
        for id in 0..self.pl(slot).vars.len() {
            let transmit = self.configs.varps.get(id).is_some_and(|t| t.transmit);
            if transmit {
                let v = self.pl(slot).vars[id];
                self.write_varp(slot, id as i32, v);
            }
        }
        self.write(slot, M::ResetAnims);
        if let Some(script) = self.scripts.get_by_trigger_specific(trigger::LOGIN, -1, -1) {
            self.run_player_trigger(slot, script, true)?;
        }
        let p = self.pl(slot);
        p.p.last_step_x = p.e.x - 1;
        p.p.last_step_z = p.e.z;
        p.e.is_active = true;
        Ok(())
    }

    /// The `playerLoop` bucket key of a remote address.
    fn loop_key(remote: &str) -> Option<i128> {
        if remote.contains('.') {
            let o: Vec<i32> = remote.split('.').map(|s| s.parse::<u8>().map_or(0, i32::from)).collect();
            let g = |i: usize| o.get(i).copied().unwrap_or(0);
            Some(((g(0).wrapping_shl(24) | g(1).wrapping_shl(16) | g(2).wrapping_shl(8) | g(3)) as u32) as i128)
        } else if remote.contains(':') {
            let hextets: Vec<&str> = remote.split('%').next().unwrap_or("").split(':').collect();
            let mut omitted = 8 - hextets.iter().filter(|h| !h.is_empty()).count() as i128;
            let mut key: i128 = 0;
            for h in hextets {
                if !h.is_empty() {
                    key = (key << 16) | i64::from_str_radix(h, 16).unwrap_or(0) as i128;
                } else if omitted != 0 {
                    key <<= omitted * 16;
                    omitted = 0;
                }
            }
            Some(key)
        } else {
            None
        }
    }

    fn force_logout(&mut self, cid: Option<ClientId>, username: String, response: Option<u8>) {
        self.login_out.push(LoginRequest::PlayerForceLogout { username });
        if let Some(cid) = cid {
            if let Some(r) = response {
                self.send(cid, vec![r]);
            }
            self.close(cid);
        }
    }

    /// `processLogins()`.
    pub fn process_logins(&mut self) {
        for player in std::mem::take(&mut self.new_players) {
            let cid = player.client;
            if self.logout_requests.contains_key(&player.username) {
                if let Some(cid) = cid {
                    self.send(cid, vec![5]);
                    self.close(cid);
                }
                continue;
            }
            let online = self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|o| o.username == player.username));
            if player.reconnecting
                && let Some(other) = online
            {
                if let Some(old) = self.pl(other).client {
                    self.close(old);
                }
                if let Some(cid) = cid {
                    let o = self.pl(other);
                    o.client = Some(cid);
                    o.session = player.session.clone();
                    self.send(cid, vec![15]);
                }
                self.rsbuf.cleanup_player_build_area(other);
                self.on_reconnect(other);
                let o = self.pl(other);
                let msg = FriendOut::PlayerLogin { username: o.username.clone(), chat_mode_private: o.private_chat, staff_lvl: o.staff_mod_level };
                self.friend_out.push(msg);
                continue;
            }
            if online.is_some() {
                if let Some(cid) = cid {
                    self.send(cid, vec![5]);
                    self.close(cid);
                }
                continue;
            }
            if self.shutdown_tick != -1 && self.current_tick >= self.shutdown_tick - 50 {
                if cid.is_some() {
                    self.force_logout(cid, player.username.clone(), Some(14));
                }
                continue;
            }
            let Some(slot) = self.next_player_slot() else {
                if cid.is_some() {
                    self.force_logout(cid, player.username.clone(), Some(7));
                }
                continue;
            };
            let key = match cid {
                Some(cid) => {
                    if let Some(c) = self.clients.get_mut(&cid) {
                        c.state = super::ClientState::Game;
                    }
                    self.send(cid, vec![2, player.staff_mod_level.min(2) as u8, 1]);
                    self.clients.get(&cid).and_then(|c| Self::loop_key(&c.remote_address))
                }
                None => Some(2130706433),
            };
            let node = self.player_loop_links.node(slot);
            if let Some(key) = key {
                self.player_loop.add(&mut self.player_loop_links, key, node);
            }
            self.player_loop_nodes.insert(slot, node);
            self.players[slot as usize] = Some(Box::new(player));
            self.rsbuf.add_player(slot);
            let p = self.pl(slot);
            p.slot = slot;
            p.uid = ((((p.username37 & 0x1fffff) as i64) << 11) | slot as i64) as u32;
            p.p.tele = true;
            p.move_click_request = false;
            let (x, z, level) = (p.e.x, p.e.z, p.e.level);
            self.zone_enter(Mover::Player(slot), x, z, level);
            if let Err(e) = self.on_login(slot) {
                tracing::error!("{e}");
            }
            if self.shutdown_tick != -1 {
                let ticks = self.shutdown_tick - self.current_tick;
                self.write(slot, M::UpdateRebootTimer { ticks });
            }
            let p = self.pl(slot);
            let msg = FriendOut::PlayerLogin { username: p.username.clone(), chat_mode_private: p.private_chat, staff_lvl: p.staff_mod_level };
            self.friend_out.push(msg);
        }
    }

    /// `onReconnect()`.
    fn on_reconnect(&mut self, slot: i32) {
        self.write(slot, M::ResetClientVarCache);
        for id in 0..self.pl(slot).vars.len() {
            if self.configs.varps.get(id).is_some_and(|t| t.transmit) {
                let v = self.pl(slot).vars[id];
                self.write_varp(slot, id as i32, v);
            }
        }
        self.rebuild_normal(slot, true);
        if self.shutdown_tick != -1 && self.current_tick >= self.shutdown_tick - 50 {
            let ticks = self.shutdown_tick - self.current_tick;
            self.write(slot, M::UpdateRebootTimer { ticks });
        }
        self.close_modal(slot, true);
        for i in 0..14 {
            let component = self.pl(slot).tabs[i];
            self.write(slot, M::IfSetTab { component, tab: i as i32 });
        }
        for l in self.pl(slot).inv_listeners.iter_mut() {
            l.first_seen = true;
        }
        for i in 0..21 {
            let p = self.pl(slot);
            let (exp, level) = (p.stats[i], p.levels[i] as i32);
            self.write(slot, M::UpdateStat { stat: i as i32, exp, level });
        }
        let energy = self.pl(slot).runenergy;
        self.write(slot, M::UpdateRunEnergy { energy });
        self.write(slot, M::ResetAnims);
        let p = self.pl(slot);
        p.p.masks |= player_prot::FACE_ENTITY | player_prot::APPEARANCE;
        p.p.move_speed = crate::engine::entity::pathing::move_speed::INSTANT;
        p.p.tele = true;
        p.p.jump = true;
    }

    /// `removePlayer(player)`.
    pub fn remove_player(&mut self, slot: i32) {
        if self.player(slot).is_none() {
            return;
        }
        if let Some(cid) = self.pl(slot).client {
            self.write_client(cid, &M::Logout);
            self.close(cid);
        }
        self.rsbuf.remove_player(slot);
        let p = self.pl(slot);
        let (x, z, level, width) = (p.e.x, p.e.z, p.e.level, p.e.width);
        self.zone_leave(Mover::Player(slot), x, z, level);
        let mut player = *self.players[slot as usize].take().expect("player");
        if let Some(node) = self.player_loop_nodes.remove(&slot) {
            self.player_loop_links.unlink(node);
            self.player_loop_garbage.push(node);
        }
        if let Some(node) = self.player_nodes.remove(&slot) {
            self.arenas.members.free(node);
        }
        self.collision.change_npc(x, z, level, width, false);
        self.collision.change_player_occ(x, z, level, width, false);
        player.cleanup_removed();
        let save = player.save(&self.configs.varps, &self.configs.invs, &self.configs.varos);
        self.logout_requests.insert(player.username.clone(), LogoutRequest { save, last_attempt: -1 });
        self.friend_out.push(FriendOut::PlayerLogout { username: player.username.clone() });
    }

    /// The scripts a slot's player holds, by queue (`all()` order).
    pub(crate) fn queue_nodes(&self, slot: i32, kind: QueueType) -> Vec<NodeId> {
        let p = self.player(slot).expect("player");
        let list = match kind {
            QueueType::Engine => p.engine_queue,
            QueueType::Weak => p.weak_queue,
            _ => p.queue,
        };
        p.queue_links.collect(list)
    }
}

impl Player {
    /// `cleanup()` after `removePlayer`.
    pub fn cleanup_removed(&mut self) {
        self.slot = -1;
        self.uid = u32::MAX;
        self.active_script = None;
        self.resume_buttons.clear();
        self.inv_listeners.clear();
        for list in [self.queue, self.weak_queue, self.engine_queue] {
            self.queue_links.clear(list);
        }
        self.camera_packets.clear();
        self.timers.clear();
        self.hero_points.clear();
        self.build_area.active_zones.clear();
        self.build_area.loaded_zones.clear();
        self.build_area.mapsquares.clear();
        self.appearance_inv = -1;
        self.last_appearance = 0;
        self.appearance_buf = None;
        self.e.is_active = false;
    }
}
