//! Player methods scripts reach through PlayerOps: visuals, modals,
//! interactions, teleports, hints, varbits and queue/timer bookkeeping.

use super::World;
use super::movement::Mover;
use crate::cache::config::script_var_type as svt;
use crate::engine::coord;
use crate::engine::entity::pathing::move_speed;
use crate::engine::entity::player::{modal, QueueType};
use crate::net::server::ServerMessage as M;
use crate::rsbuf::player_prot;
use crate::script::state::{self, EntityRef};

/// `Packet.bitmask[n]`.
fn bitmask(n: i32) -> i32 {
    match n {
        0..=31 => ((1u64 << n) - 1) as i32,
        32 => -1,
        _ => 0,
    }
}

impl World {
    /// `playAnimation(anim, delay)` for a player (`animProtect` blocks it).
    pub fn player_play_animation(&mut self, slot: i32, anim: i32, delay: i32) -> Result<(), String> {
        let count = self.configs.seqs.count() as i32;
        let p = self.pl(slot);
        if anim >= count || p.anim_protect != 0 {
            return Ok(());
        }
        let cur = p.p.anim_id;
        let priority = |id: i32| self.configs.seqs.get(id as usize).map(|s| s.priority);
        let replace = anim == -1 || cur == -1 || {
            let new = priority(anim).ok_or_else(|| format!("no SeqType for anim {anim}"))?;
            new >= priority(cur).unwrap_or(i32::MAX)
        };
        if replace {
            let p = self.pl(slot);
            p.p.anim_id = anim;
            p.p.anim_delay = delay;
            p.p.masks |= player_prot::ANIM;
        }
        Ok(())
    }

    pub fn player_spotanim(&mut self, slot: i32, spotanim: i32, height: i32, delay: i32) {
        let p = &mut self.pl(slot).p;
        p.spotanim_id = spotanim;
        p.spotanim_height = height;
        p.spotanim_time = delay;
        p.masks |= player_prot::SPOT_ANIM;
    }

    pub fn player_say(&mut self, slot: i32, text: String) {
        let p = &mut self.pl(slot).p;
        p.say_message = Some(text);
        p.masks |= player_prot::SAY;
    }

    pub fn player_face_square(&mut self, slot: i32, x: i32, z: i32) {
        self.pl(slot).p.focus(coord::fine(x, 1), coord::fine(z, 1), true);
    }

    /// `applyDamage(damage, type)` for a player.
    pub fn player_apply_damage(&mut self, slot: i32, mut damage: i32, kind: i32) {
        let p = self.pl(slot);
        let hp = crate::engine::entity::player::stat::HITPOINTS;
        let current = p.levels[hp] as i32;
        if current - damage <= 0 {
            p.levels[hp] = 0;
            damage = current;
        } else {
            p.levels[hp] = (current - damage) as u8;
        }
        if p.p.hitmark_slot % 2 == 1 {
            p.p.hitmark2_damage = damage;
            p.p.hitmark2_type = kind;
            p.p.masks |= player_prot::DAMAGE2;
        } else {
            p.p.hitmark_damage = damage;
            p.p.hitmark_type = kind;
            p.p.masks |= player_prot::DAMAGE;
        }
        p.p.hitmark_slot += 1;
    }

    /// `teleJump(x, z, level)`.
    pub fn player_tele_jump(&mut self, slot: i32, x: i32, z: i32, level: i32) {
        self.teleport(Mover::Player(slot), x, z, level);
        let p = &mut self.pl(slot).p;
        p.move_speed = move_speed::INSTANT;
        p.jump = true;
    }

    /// `exactMove(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn player_exact_move(&mut self, slot: i32, sx: i32, sz: i32, ex: i32, ez: i32, start: i32, end: i32, dir: i32) {
        let level = self.pl(slot).e.level;
        self.teleport(Mover::Player(slot), ex, ez, level);
        let p = &mut self.pl(slot).p;
        p.exact_start_x = sx;
        p.exact_start_z = sz;
        p.exact_end_x = ex;
        p.exact_end_z = ez;
        p.exact_move_start = start;
        p.exact_move_end = end;
        p.exact_move_facing = dir;
        p.masks |= player_prot::EXACT_MOVE;
    }

    /// `clearPendingAction()`.
    pub fn clear_pending_action(&mut self, slot: i32) {
        self.pl(slot).p.clear_interaction();
        self.close_modal(slot, true);
    }

    /// `stopAction()`.
    pub fn stop_action(&mut self, slot: i32) {
        self.clear_pending_action(slot);
        self.unset_map_flag(slot);
    }

    /// `setInteraction(SCRIPT, target, op, com)` for a player.
    pub fn player_set_interaction(&mut self, slot: i32, target: EntityRef, op: i32, com: Option<i32>) -> bool {
        let hash64 = self.pl(slot).hash64;
        let valid = match target {
            EntityRef::Obj(o) => self.arenas.objs.get(o).is_some_and(|o| o.is_valid(Some(hash64))),
            EntityRef::Player(s) => self.player_is_valid(s),
            _ => self.entity_valid(target),
        };
        if !valid {
            return false;
        }
        let kind = self.entity_type(target);
        let area = self.entity_area(target);
        let p = &mut self.pl(slot).p;
        p.target = Some(target);
        p.target_op = op;
        p.ap_range = 10;
        p.ap_range_called = false;
        p.target_subject_com = com.filter(|&c| c != 0).unwrap_or(-1);
        p.target_subject_type = if matches!(target, EntityRef::Player(_)) { -1 } else { kind };
        if let (EntityRef::Loc(_) | EntityRef::Obj(_), Some((_, (x, z, w, l)))) = (target, area) {
            p.target_x = coord::fine(x, w);
            p.target_z = coord::fine(z, l);
        }
        true
    }

    fn clear_dialog_script(&mut self, slot: i32) {
        let p = self.pl(slot);
        if p.active_script.as_ref().is_some_and(|s| s.execution == state::Execution::CountDialog || s.execution == state::Execution::PauseButton) {
            p.active_script = None;
            p.resume_buttons.clear();
        }
    }

    /// `openMainModal(com)`.
    pub fn open_main_modal(&mut self, slot: i32, com: i32) {
        if self.pl(slot).modal_state & modal::CHAT != 0 {
            self.write(slot, M::IfClose);
            let p = self.pl(slot);
            p.modal_state &= !modal::CHAT;
            p.modal_chat = -1;
        }
        if self.pl(slot).modal_state & modal::SIDE != 0 {
            self.write(slot, M::IfClose);
            let p = self.pl(slot);
            p.modal_state &= !modal::SIDE;
            p.modal_side = -1;
        }
        let p = self.pl(slot);
        p.modal_state |= modal::MAIN;
        p.modal_main = com;
        p.refresh_modal = true;
        self.clear_dialog_script(slot);
    }

    /// `openChatModal(com)` (closing main or side resets `modalChat`).
    pub fn open_chat_modal(&mut self, slot: i32, com: i32) {
        for flag in [modal::MAIN, modal::SIDE] {
            if self.pl(slot).modal_state & flag != 0 {
                self.write(slot, M::IfClose);
                let p = self.pl(slot);
                p.modal_state &= !flag;
                p.modal_chat = -1;
            }
        }
        let p = self.pl(slot);
        p.modal_state |= modal::CHAT;
        p.modal_chat = com;
        p.refresh_modal = true;
        self.clear_dialog_script(slot);
    }

    /// `openSideModal(com)` (closing main resets `modalChat`, closing chat `modalSide`).
    pub fn open_side_modal(&mut self, slot: i32, com: i32) {
        if self.pl(slot).modal_state & modal::MAIN != 0 {
            self.write(slot, M::IfClose);
            let p = self.pl(slot);
            p.modal_state &= !modal::MAIN;
            p.modal_chat = -1;
        }
        if self.pl(slot).modal_state & modal::CHAT != 0 {
            self.write(slot, M::IfClose);
            let p = self.pl(slot);
            p.modal_state &= !modal::CHAT;
            p.modal_side = -1;
        }
        let p = self.pl(slot);
        p.modal_state |= modal::SIDE;
        p.modal_side = com;
        p.refresh_modal = true;
        self.clear_dialog_script(slot);
    }

    /// `openMainSideModal(top, side)`.
    pub fn open_main_side_modal(&mut self, slot: i32, top: i32, side: i32) {
        if self.pl(slot).modal_state & modal::CHAT != 0 {
            self.write(slot, M::IfClose);
            let p = self.pl(slot);
            p.modal_state &= !modal::CHAT;
            p.modal_chat = -1;
        }
        let p = self.pl(slot);
        p.modal_state |= modal::MAIN | modal::SIDE;
        p.modal_main = top;
        p.modal_side = side;
        p.refresh_modal = true;
        self.clear_dialog_script(slot);
    }

    /// `openMainOverlay(com)`.
    pub fn open_main_overlay(&mut self, slot: i32, com: i32) {
        let overlay = self.pl(slot).overlay;
        if overlay == com {
            return;
        }
        if com == -1 {
            self.clear_listeners_for_root(slot, overlay);
        }
        self.pl(slot).overlay = com;
    }

    fn clear_listeners_for_root(&mut self, slot: i32, root: i32) {
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

    /// `openTutorial(com)`.
    pub fn open_tutorial(&mut self, slot: i32, com: i32) {
        self.write(slot, M::TutOpen { component: com });
        let p = self.pl(slot);
        p.modal_state |= modal::TUT;
        p.modal_tutorial = com;
    }

    /// `closeTutorial()`.
    pub fn close_tutorial(&mut self, slot: i32) -> Result<(), String> {
        let com = self.pl(slot).modal_tutorial;
        if com != -1 {
            if let Some(script) = self.scripts.get_by_trigger(ares_shared::trigger::IF_CLOSE, com, -1) {
                self.run_player_trigger(slot, script, false)?;
            }
            self.pl(slot).modal_tutorial = -1;
            self.write(slot, M::TutOpen { component: -1 });
        }
        Ok(())
    }

    /// `setTab(com, tab)`.
    pub fn set_tab(&mut self, slot: i32, com: i32, tab: i32) {
        if let Some(t) = self.pl(slot).tabs.get_mut(tab as usize) {
            *t = com;
        }
        self.write(slot, M::IfSetTab { component: com, tab });
    }

    // ---- hints, sounds, login info ----

    pub fn hint(&mut self, slot: i32, kind: i32, nid: i32, player_slot: i32, x: i32, z: i32, y: i32) {
        self.write(slot, M::HintArrow { kind, nid, player_slot, x, z, y });
    }

    /// `playJingle(id)`: the jingle's length in ms rides along.
    pub fn play_jingle(&mut self, slot: i32, id: i32) {
        let len = usize::try_from(id).ok().and_then(|i| self.midi_lengths.get(i)).copied().unwrap_or(0.0);
        self.write(slot, M::MidiJingle { id, delay: crate::util::numbers::wrap_f64(len) });
    }

    /// `lastLoginInfo()`.
    pub fn last_login_info(&mut self, slot: i32) {
        let now = self.clock_ms();
        let members = self.map.members;
        let p = self.pl(slot);
        let last = if p.last_login_time == 0 { now } else { p.last_login_time };
        let days = ((now - last) / (1000 * 60 * 60 * 24)) as i32;
        let warn = !members && p.members;
        let count = p.message_count;
        p.last_login_time = now;
        self.write(slot, M::LastLoginInfo { last_login_ip: 2130706433, days_since_login: days, days_since_recovery_change: 201, unread_message_count: count, warn_members_in_non_members: warn });
    }

    // ---- vars ----

    /// `getVar(id)` as a script value: `Ok(int)` or `Err(string)`.
    pub fn player_get_var(&self, slot: i32, id: i32) -> Result<i32, Option<String>> {
        let p = self.player(slot).expect("player");
        let Some(t) = self.configs.varps.get(id as usize) else { return Ok(0) };
        if t.kind == svt::STRING { Err(p.vars_string.get(t.id).cloned().flatten()) } else { Ok(p.vars.get(t.id).copied().unwrap_or(0)) }
    }

    pub fn player_set_var_string(&mut self, slot: i32, id: i32, value: String) {
        let Some(t) = self.configs.varps.get(id as usize) else { return };
        if t.kind == svt::STRING {
            let vid = t.id;
            if let Some(v) = self.pl(slot).vars_string.get_mut(vid) {
                *v = Some(value);
            }
        }
    }

    /// `getVarBit(id)`.
    pub fn player_get_varbit(&self, slot: i32, id: i32) -> i32 {
        let Some(b) = self.configs.varbits.get(id as usize) else { return 0 };
        let p = self.player(slot).expect("player");
        let base = p.vars.get(b.basevar as usize).copied().unwrap_or(0);
        base.wrapping_shr(b.startbit as u32) & bitmask(b.endbit - b.startbit + 1)
    }

    /// `setVarBit(id, value)`.
    pub fn player_set_varbit(&mut self, slot: i32, id: i32, mut value: i32) {
        let Some(b) = self.configs.varbits.get(id as usize) else { return };
        let (basevar, start, end) = (b.basevar, b.startbit, b.endbit);
        let mut mask = bitmask(end - start + 1);
        if value < 0 || value > mask {
            value = 0;
        }
        mask = mask.wrapping_shl(start as u32);
        let base = self.pl(slot).vars.get(basevar as usize).copied().unwrap_or(0);
        self.player_set_var(slot, basevar, (mask & value.wrapping_shl(start as u32)) | (base & !mask));
    }

    // ---- queues ----

    /// `unlinkQueuedScript(scriptId, type)`.
    pub fn unlink_queued_script(&mut self, slot: i32, script_id: i32, kind: QueueType) {
        let p = self.pl(slot);
        let lists = if kind == QueueType::Engine { vec![p.engine_queue] } else { vec![p.queue, p.weak_queue] };
        for list in lists {
            for id in p.queue_links.collect(list) {
                if p.queue_links.get(id).is_some_and(|r| r.script.id as i32 == script_id) {
                    p.queue_links.unlink(id);
                    p.queue_garbage.push(id);
                }
            }
        }
    }
}
