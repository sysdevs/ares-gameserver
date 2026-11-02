//! `::` commands by staff level (4 dev, 3 admin, 2 super-moderator).

use super::World;
use crate::cache::config::script_var_type as svt;
use crate::engine::entity::pathing::move_strategy;
use crate::engine::entity::player::stat;
use crate::engine::entity::{Loc, Npc, lifecycle};
use crate::engine::xp::get_exp_by_level;
use crate::script::state::{Arg, EntityRef};

/// `PlayerStatMap`: upper-case stat names in id order.
const STAT_NAMES: [&str; 21] = [
    "ATTACK", "DEFENCE", "STRENGTH", "HITPOINTS", "RANGED", "PRAYER", "MAGIC", "COOKING", "WOODCUTTING", "FLETCHING", "FISHING", "FIREMAKING", "CRAFTING", "SMITHING", "MINING", "HERBLORE", "AGILITY",
    "THIEVING", "STAT18", "STAT19", "RUNECRAFT",
];

fn stat_id(name: &str) -> Option<usize> {
    STAT_NAMES.iter().position(|s| *s == name.to_uppercase())
}

fn int(v: Option<&str>) -> Option<i32> {
    v?.parse().ok()
}

/// `tryParseInt(value, default)`: a malformed argument gives the default.
fn int_or(v: Option<&String>, default: i32) -> i32 {
    int(v.map(String::as_str)).unwrap_or(default)
}

impl World {
    /// Closes the modal and refuses when busy; clears the interaction otherwise.
    fn cheat_ready(&mut self, slot: i32, busy_message: &str) -> bool {
        self.close_modal(slot, true);
        if !self.can_access(slot) {
            self.message_game(slot, busy_message);
            return false;
        }
        self.pl(slot).p.clear_interaction();
        self.unset_map_flag(slot);
        true
    }

    fn player_slot_by_name(&self, name: &str) -> Option<i32> {
        let n37 = crate::util::jstring::to_base37(name) as i64;
        self.player_loop().into_iter().find(|&s| self.player(s).is_some_and(|p| p.username37 == n37))
    }

    fn inv_add_cheat(&mut self, slot: i32, obj: i32, count: i64) -> Result<(), String> {
        let inv = self.configs.invs.get_id("inv");
        let owner = self.player_inventory(slot, inv).ok_or_else(|| format!("invAdd: Invalid inventory type: {inv}"))?;
        let (c, configs) = self.inv_with_configs(owner).expect("inventory");
        c.add(&configs.objs, obj, count, -1);
        Ok(())
    }

    /// `ClientCheatHandler.handle`: `Err` on a script-level error.
    pub(crate) fn client_cheat(&mut self, slot: i32, cheat: String) -> Result<bool, String> {
        if cheat.encode_utf16().count() > 80 {
            return Ok(false);
        }
        let lower = cheat.to_lowercase();
        let mut args: Vec<String> = lower.split(' ').map(str::to_string).collect();
        let cmd = args.remove(0);
        if cmd.is_empty() {
            return Ok(false);
        }
        let staff = self.pl(slot).staff_mod_level;

        if !self.production && staff >= 4 {
            if cmd.starts_with('~') {
                let Some(script) = self.scripts.get_by_name(&format!("[debugproc,{}]", &cmd[1..])) else { return Ok(false) };
                let mut it = args.iter();
                let mut params = Vec::new();
                for &t in &script.info.parameter_types {
                    let mut next = || it.next().cloned();
                    let name_id = |id: i32| Arg::Int(id);
                    params.push(match t {
                        svt::STRING => Arg::Str(next().unwrap_or_default()),
                        svt::INT => Arg::Int(int(next().as_deref()).unwrap_or(0)),
                        svt::OBJ | svt::NAMEDOBJ => name_id(self.configs.objs.get_id(&next().unwrap_or_default())),
                        svt::NPC => name_id(self.configs.npcs.get_id(&next().unwrap_or_default())),
                        svt::LOC => name_id(self.configs.locs.get_id(&next().unwrap_or_default())),
                        svt::SEQ => name_id(self.configs.seqs.get_id(&next().unwrap_or_default())),
                        svt::STAT => match stat_id(&next().unwrap_or_default()) {
                            Some(s) => Arg::Int(s as i32),
                            None => Arg::Str(String::new()),
                        },
                        svt::INV => name_id(self.configs.invs.get_id(&next().unwrap_or_default())),
                        svt::COORD => {
                            let parts: Vec<&str> = cheat.split('_').collect();
                            let p = |i: usize| int(parts.get(i).copied()).unwrap_or(0);
                            let first: String = parts.first().map(|s| s.encode_utf16().skip(6).collect::<Vec<u16>>()).map(|u| String::from_utf16_lossy(&u)).unwrap_or_default();
                            let level = int(Some(&first)).unwrap_or(0);
                            let x = (p(1) << 6).wrapping_add(p(3));
                            let z = (p(2) << 6).wrapping_add(p(4));
                            Arg::Int(crate::engine::coord::pack(level, x, z))
                        }
                        svt::INTERFACE => name_id(self.configs.components.get_id(&next().unwrap_or_default())),
                        svt::SPOTANIM => name_id(self.configs.spotanims.get_id(&next().unwrap_or_default())),
                        svt::IDKIT => name_id(self.configs.idks.get_id(&next().unwrap_or_default())),
                        _ => Arg::Int(-1),
                    });
                }
                let s = Self::init_script(script, Some(EntityRef::Player(slot)), None, &params);
                self.execute_player_script(slot, s, false, false, false)?;
            } else if cmd == "reload" {
                self.reload(true);
            } else if cmd == "rebuild" {
                self.message_game(slot, "Rebuilding scripts...");
                self.rebuild_requested = true;
            } else if cmd == "speed" {
                if args.is_empty() {
                    self.message_game(slot, "Usage: ::speed <ms>");
                    return Ok(false);
                }
                let speed = int_or(args.first(), 20);
                if speed < 20 {
                    self.message_game(slot, "::speed input was too low.");
                    return Ok(false);
                }
                self.message_game(slot, &format!("World speed was changed to {speed}ms"));
                self.tick_rate = speed as f64;
            } else if cmd == "fly" || cmd == "naive" {
                let target = if cmd == "fly" { move_strategy::FLY } else { move_strategy::NAIVE };
                let p = self.pl(slot);
                p.p.move_strategy = if p.p.move_strategy == target { move_strategy::SMART } else { target };
                let m = p.p.move_strategy;
                let msg = if cmd == "fly" {
                    format!("Changed move strategy: {}", if m == move_strategy::FLY { "fly" } else { "smart" })
                } else {
                    format!("Naive move strategy: {}", if m == move_strategy::NAIVE { "naive" } else { "smart" })
                };
                self.message_game(slot, &msg);
            } else if cmd == "random" {
                self.pl(slot).afk_event_ready = true;
            }
        }

        if staff >= 3 {
            match cmd.as_str() {
                "setvar" | "getvar" => {
                    if args.len() < if cmd == "setvar" { 2 } else { 1 } {
                        return Ok(false);
                    }
                    let name = args[0].clone();
                    let value = int_or(args.get(1), 0);
                    let varbit = self.configs.varbits.get_by_name(&name).map(|b| (b.id as i32, b.basevar, b.name.clone()));
                    let varp = match varbit {
                        Some((_, base, _)) => self.configs.varps.get(base as usize).map(|v| (v.id as i32, v.protect, v.name.clone())),
                        None => self.configs.varps.get_by_name(&name).map(|v| (v.id as i32, v.protect, v.name.clone())),
                    };
                    if varbit.is_some() && varp.as_ref().is_some_and(|v| v.1) && !self.cheat_ready(slot, "Please finish what you are doing first.") {
                        return Ok(false);
                    }
                    let Some((vid, protect, vname)) = varp else { return Ok(false) };
                    if cmd == "setvar" {
                        if protect && !self.cheat_ready(slot, "Please finish what you are doing first.") {
                            return Ok(false);
                        }
                        match varbit {
                            Some((bid, _, bname)) => {
                                self.player_set_varbit(slot, bid, value);
                                self.message_game(slot, &format!("set {}: to {value}", bname.unwrap_or_default()));
                            }
                            None => {
                                self.player_set_var(slot, vid, value);
                                self.message_game(slot, &format!("set {}: to {value}", vname.unwrap_or_default()));
                            }
                        }
                    } else {
                        match varbit {
                            Some((bid, _, bname)) => {
                                let v = self.player_get_varbit(slot, bid);
                                self.message_game(slot, &format!("get {}: {v}", bname.unwrap_or_default()));
                            }
                            None => {
                                let v = match self.player_get_var(slot, vid) {
                                    Ok(v) => v.to_string(),
                                    Err(s) => s.unwrap_or_default(),
                                };
                                self.message_game(slot, &format!("get {}: {v}", vname.unwrap_or_default()));
                            }
                        }
                    }
                }
                "give" | "givemany" => {
                    if args.is_empty() {
                        return Ok(false);
                    }
                    let obj = self.configs.objs.get_id(&args[0]);
                    if obj == -1 {
                        return Ok(false);
                    }
                    let count = if cmd == "give" { int_or(args.get(1), 1).max(1) as i64 } else { 1000 };
                    self.inv_add_cheat(slot, obj, count)?;
                }
                "givecrap" => {
                    for _ in 0..28 {
                        let mut random = -1;
                        while random == -1 {
                            random = (self.math_random() * self.configs.objs.count() as f64).trunc() as i32;
                            let t = self.configs.objs.get(random as usize).expect("obj");
                            if (!self.map.members && t.members) || t.dummyitem != 0 || t.certtemplate != -1 {
                                random = -1;
                            }
                        }
                        self.inv_add_cheat(slot, random, 1)?;
                    }
                }
                "serverdrop" => {
                    if let Some(cid) = self.pl(slot).client {
                        self.terminate(cid);
                    }
                }
                "setstat" | "advancestat" => {
                    if args.len() < if cmd == "setstat" { 2 } else { 1 } {
                        return Ok(false);
                    }
                    let Some(st) = stat_id(&args[0]) else { return Ok(false) };
                    let level = int(args.get(1).map(String::as_str));
                    if cmd == "setstat" {
                        if let Some(level) = level {
                            self.player_set_level(slot, st, level);
                        } else {
                            let p = self.pl(slot);
                            p.base_levels[st] = 0;
                            p.levels[st] = 0;
                            p.stats[st] = 0;
                        }
                    } else {
                        let p = self.pl(slot);
                        p.stats[st] = 0;
                        p.base_levels[st] = 1;
                        p.levels[st] = 1;
                        if let Some(l) = level.filter(|l| (2..=99).contains(l)) {
                            self.player_add_xp(slot, st, get_exp_by_level(l), false)?;
                        }
                    }
                }
                "minme" => {
                    for i in 0..21 {
                        self.player_set_level(slot, i, if i == stat::HITPOINTS { 10 } else { 1 });
                    }
                }
                "locadd" => {
                    let Some(name) = args.first() else { return Ok(false) };
                    let Some(t) = self.configs.locs.get_by_name(name) else { return Ok(false) };
                    let (w, l, id) = (t.width, t.length, t.id as i32);
                    let p = self.pl(slot);
                    let loc = Loc::new(p.e.level, p.e.x, p.e.z, w, l, lifecycle::DESPAWN, id, 10, 0);
                    let lid = self.new_loc(loc);
                    self.add_loc(lid, 500);
                    let name = name.clone();
                    self.message_game(slot, &format!("Loc Added: {name} (ID: {id})"));
                }
                "npcadd" => {
                    let Some(name) = args.first() else { return Ok(false) };
                    let Some(t) = self.configs.npcs.get_by_name(name) else { return Ok(false) };
                    let p = self.player(slot).expect("player");
                    let nid = self.npcs.next_id() as i32;
                    let npc = Npc::new(p.e.level, p.e.x, p.e.z, t.size, t.size, lifecycle::DESPAWN, nid, t, t.blockwalk, self.configs.varns.count());
                    self.add_npc(npc, 500);
                }
                "openmain" | "openoverlay" => {
                    let Some(name) = args.first() else { return Ok(false) };
                    let id = self.configs.components.get_id(name);
                    let ok = usize::try_from(id).ok().and_then(|i| self.configs.components.get(i)).is_some_and(|c| c.root_layer == c.id);
                    if !ok {
                        return Ok(false);
                    }
                    if cmd == "openmain" {
                        self.open_main_modal(slot, id);
                    } else {
                        self.open_main_overlay(slot, id);
                    }
                }
                "closeoverlay" => self.open_main_overlay(slot, -1),
                "teleother" if self.production => {
                    let Some(name) = args.first().cloned() else { return Ok(false) };
                    let Some(other) = self.player_slot_by_name(&name) else {
                        self.message_game(slot, &format!("{name} is not logged in."));
                        return Ok(false);
                    };
                    self.close_modal(other, true);
                    if !self.can_access(other) {
                        self.message_game(slot, &format!("{name} is busy right now."));
                        return Ok(false);
                    }
                    self.pl(other).p.clear_interaction();
                    self.unset_map_flag(other);
                    let p = self.pl(slot);
                    let (x, z, level) = (p.e.x, p.e.z, p.e.level);
                    self.player_tele_jump(other, x, z, level);
                }
                _ => {}
            }
        }

        // ares only
        if staff >= 3 && cmd == "serverstats" {
            self.toggle_server_stats(slot);
        }

        if staff >= 2 {
            match cmd.as_str() {
                "getcoord" => {
                    let p = self.pl(slot);
                    let (l, x, z) = (p.e.level, p.e.x, p.e.z);
                    self.message_game(slot, &format!("{l},{},{},{},{}", x >> 6, z >> 6, x & 0x3f, z & 0x3f));
                }
                "tele" => {
                    let Some(arg) = args.first() else { return Ok(false) };
                    let parts: Vec<String> = arg.split(',').map(str::to_string).collect();
                    if parts.len() < 3 {
                        return Ok(false);
                    }
                    if !self.cheat_ready(slot, "Please finish what you are doing first.") {
                        return Ok(false);
                    }
                    let level = int_or(parts.first(), 0);
                    let mx = int_or(parts.get(1), 50);
                    let mz = int_or(parts.get(2), 50);
                    let lx = int_or(parts.get(3), 32);
                    let lz = int_or(parts.get(4), 32);
                    if !(0..=3).contains(&level) || !(0..=255).contains(&mx) || !(0..=255).contains(&mz) || !(0..=63).contains(&lx) || !(0..=63).contains(&lz) {
                        return Ok(false);
                    }
                    self.player_tele_jump(slot, (mx << 6) + lx, (mz << 6) + lz, level);
                }
                "teleto" if self.production => {
                    let Some(name) = args.first().cloned() else { return Ok(false) };
                    let Some(other) = self.player_slot_by_name(&name) else {
                        self.message_game(slot, &format!("{name} is not logged in."));
                        return Ok(false);
                    };
                    if !self.cheat_ready(slot, "Please finish what you are doing first.") {
                        return Ok(false);
                    }
                    let o = self.pl(other);
                    let (x, z, level) = (o.e.x, o.e.z, o.e.level);
                    self.player_tele_jump(slot, x, z, level);
                }
                "kick" if self.production => {
                    let Some(name) = args.first().cloned() else {
                        self.message_game(slot, "Usage: ::kick <username>");
                        return Ok(false);
                    };
                    match self.player_slot_by_name(&name) {
                        Some(other) => {
                            self.pl(other).logging_out = true;
                            if let Some(cid) = self.pl(other).client {
                                self.write_client(cid, &crate::net::server::ServerMessage::Logout);
                                self.close(cid);
                            }
                            self.message_game(slot, &format!("Player '{name}' has been kicked from the game."));
                        }
                        None => self.message_game(slot, &format!("Player '{name}' does not exist or is not logged in.")),
                    }
                }
                _ => {}
            }
        }
        Ok(true)
    }
}
