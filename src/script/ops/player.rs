//! Player ops, including the player var ops.

use super::super::runner::OpResult;
use super::super::state::{self, Arg, EntityRef, ScriptState};
use ares_shared::{opcode::op, trigger};
use super::super::PointerType;
use super::{coord, in_range, not_null, valid};
use crate::cache::config::script_var_type as svt;
use crate::engine::coord as cg;
use crate::engine::entity::player::{CameraInfo, Player, QueueType, stat};
use crate::engine::world::World;
use crate::net::server::ServerMessage as M;

/// The types string, then the values in reverse.
fn pop_script_args(s: &mut ScriptState) -> Vec<Arg> {
    let types: Vec<char> = s.pop_string().chars().collect();
    let mut args = vec![Arg::Int(0); types.len()];
    for i in (0..types.len()).rev() {
        args[i] = if types[i] == 's' { Arg::Str(s.pop_string()) } else { Arg::Int(s.pop_int()) };
    }
    args
}

fn stat_valid(v: i32) -> Result<usize, String> {
    in_range(v, 0, stat::RUNECRAFT as i32, "PlayerStat").map(|v| v as usize)
}

fn queue_script(w: &World, id: i32) -> Result<std::sync::Arc<ares_shared::script::ScriptFile>, String> {
    w.scripts.get(id).ok_or_else(|| format!("Unable to find queue script: {id}"))
}

fn timer_script(w: &World, id: i32) -> Result<std::sync::Arc<ares_shared::script::ScriptFile>, String> {
    w.scripts.get(id).ok_or_else(|| format!("Unable to find timer script: {id}"))
}

/// The active player, which must still be in the world.
pub(super) fn ap(w: &World, s: &ScriptState) -> Result<i32, String> {
    let slot = s.active_player()?;
    w.player(slot).map(|_| slot).ok_or_else(|| "Attempt to access null active_player".into())
}

fn pl<'a>(w: &'a mut World, s: &ScriptState) -> Result<&'a mut Player, String> {
    let slot = ap(w, s)?;
    Ok(w.pl(slot))
}

/// `LAST_*` ops refuse triggers that do not set them.
fn trigger_allowed(s: &ScriptState, allowed: &[i32]) -> Result<(), String> {
    if allowed.contains(&s.trigger) { Ok(()) } else { Err("is not safe to use in this trigger".into()) }
}

/// `IF_SETTEXT`: carry an `@col@` across `\n` line breaks (except `@str@`).
fn carry_colours(text: String) -> String {
    if !text.contains("\\n") || !text.contains('@') {
        return text;
    }
    let mut lines: Vec<Vec<u16>> = text.split("\\n").map(|l| l.encode_utf16().collect()).collect();
    let at = '@' as u16;
    let s = |l: &[u16], a: usize, b: usize| String::from_utf16_lossy(&l[a.min(l.len())..b.min(l.len())]);
    let mut saved: Option<Vec<u16>> = None;
    for i in 0..lines.len() {
        let mut line = lines[i].clone();
        if i > 0 && !line.is_empty() && let Some(col) = saved.clone() {
            let idx = (0..line.len().saturating_sub(4)).find(|&k| s(&line, k, k + 5) == "@str@").or_else(|| if line.len() >= 5 { None } else { None });
            match idx {
                Some(k) => {
                    if s(&line, k + 5, k + 10) != "@bla@" {
                        let mut n = line[..k + 5].to_vec();
                        n.extend("@bla@".encode_utf16());
                        n.extend_from_slice(&line[k + 5..]);
                        line = n;
                    }
                    saved = None;
                }
                None => {
                    let mut n = col;
                    n.extend_from_slice(&line);
                    line = n;
                }
            }
            lines[i] = line.clone();
        }
        let mut j = 0;
        while j + 4 < line.len() {
            if line[j] == at && line[j + 4] == at {
                let col = s(&line, j + 1, j + 4);
                if col == "str" {
                    saved = None;
                    if s(&line, j + 5, j + 10) == "@bla@" {
                        j += 10;
                        continue;
                    }
                } else {
                    saved = Some(line[j..j + 5].to_vec());
                }
                j += 4;
            }
            j += 1;
        }
    }
    lines.iter().map(|l| String::from_utf16_lossy(l)).collect::<Vec<_>>().join("\\n")
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let tick = w.current_tick;
    super::handled((|| {
        match opcode {
            op::FINDUID => {
                let uid = s.pop_int();
                match w.player_by_uid(uid) {
                    Some(slot) => {
                        s.set_active_player(slot);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::P_FINDUID => {
                let uid = s.pop_int() as u32;
                let i = (s.int_operand() != 0) as usize;
                let protected = if i == 0 { PointerType::PActivePlayer } else { PointerType::PActivePlayer2 };
                if s.pointer_get(protected) && w.player(s.active_player()?).is_some_and(|p| p.uid == uid) {
                    s.push_int(1);
                    return Ok(());
                }
                match w.player_by_uid(uid as i32).filter(|&p| w.can_access(p)) {
                    Some(slot) => {
                        s.set_active_player(slot);
                        s.pointer_add(protected);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::STRONGQUEUE | op::WEAKQUEUE | op::QUEUE => {
                let [id, delay, arg] = s.pop_ints::<3>();
                let script = queue_script(w, id)?;
                let kind = match opcode {
                    op::STRONGQUEUE => QueueType::Strong,
                    op::WEAKQUEUE => QueueType::Weak,
                    _ => QueueType::Normal,
                };
                let slot = ap(w, s)?;
                w.enqueue_player_script(slot, script, kind, delay, vec![Arg::Int(arg)]);
            }
            op::STRONGQUEUE_STAR | op::WEAKQUEUE_STAR | op::QUEUE_STAR => {
                let args = pop_script_args(s);
                let [id, delay] = s.pop_ints::<2>();
                let script = queue_script(w, id)?;
                let kind = match opcode {
                    op::STRONGQUEUE_STAR => QueueType::Strong,
                    op::WEAKQUEUE_STAR => QueueType::Weak,
                    _ => QueueType::Normal,
                };
                let slot = ap(w, s)?;
                w.enqueue_player_script(slot, script, kind, delay, args);
            }
            op::LONGQUEUE => {
                let [id, delay, arg, logout] = s.pop_ints::<4>();
                let script = queue_script(w, id)?;
                let slot = ap(w, s)?;
                w.enqueue_player_script(slot, script, QueueType::Long, delay, vec![Arg::Int(logout), Arg::Int(arg)]);
            }
            op::LONGQUEUE_STAR => {
                let args = pop_script_args(s);
                let [id, delay, logout] = s.pop_ints::<3>();
                let script = queue_script(w, id)?;
                let slot = ap(w, s)?;
                let mut all = vec![Arg::Int(logout)];
                all.extend(args);
                w.enqueue_player_script(slot, script, QueueType::Long, delay, all);
            }
            op::ANIM => {
                let delay = s.pop_int();
                let seq = s.pop_int();
                let slot = ap(w, s)?;
                w.player_play_animation(slot, seq, delay)?;
            }
            op::BUILDAPPEARANCE => {
                let inv = valid(&w.configs.invs, s.pop_int(), "Inv")?.id as i32;
                let slot = ap(w, s)?;
                w.build_appearance(slot, inv);
            }
            op::CAM_LOOKAT | op::CAM_MOVETO => {
                let [c, height, speed, mult] = s.pop_ints::<4>();
                let (_, x, z) = coord(c)?;
                let kind = if opcode == op::CAM_LOOKAT { 1 } else { 0 };
                pl(w, s)?.camera_packets.push(CameraInfo { kind, cam_x: x, cam_z: z, height, rotation_speed: speed, rotation_multiplier: mult });
            }
            op::CAM_SHAKE => {
                let [axis, random, amplitude, rate] = s.pop_ints::<4>();
                let slot = ap(w, s)?;
                w.write(slot, M::CamShake { axis, random, amplitude, rate });
            }
            op::CAM_RESET => {
                let slot = ap(w, s)?;
                w.write(slot, M::CamReset);
            }
            op::COORD => {
                let p = pl(w, s)?;
                let v = cg::pack(p.e.level, p.e.x, p.e.z);
                s.push_int(v);
            }
            op::DISPLAYNAME => {
                let v = pl(w, s)?.display_name.clone();
                s.push_string(v);
            }
            op::FACESQUARE => {
                let (_, x, z) = coord(s.pop_int())?;
                let slot = ap(w, s)?;
                w.player_face_square(slot, x, z);
            }
            op::IF_CLOSE => {
                let slot = ap(w, s)?;
                w.close_modal(slot, true);
            }
            op::LAST_COM => {
                let v = pl(w, s)?.last_com;
                s.push_int(v);
            }
            op::LAST_INT => s.push_int(s.last_int),
            op::LAST_ITEM | op::LAST_SLOT | op::LAST_USEITEM | op::LAST_USESLOT | op::LAST_TARGETSLOT => {
                use trigger::*;
                let held = [OPHELD1, OPHELD2, OPHELD3, OPHELD4, OPHELD5, OPHELDU, OPHELDT, INV_BUTTON1, INV_BUTTON2, INV_BUTTON3, INV_BUTTON4, INV_BUTTON5];
                let used = [OPHELDU, APOBJU, APLOCU, APNPCU, APPLAYERU, OPOBJU, OPLOCU, OPNPCU, OPPLAYERU];
                match opcode {
                    op::LAST_ITEM => trigger_allowed(s, &held)?,
                    op::LAST_SLOT => trigger_allowed(s, &[&held[..], &[INV_BUTTOND]].concat())?,
                    op::LAST_TARGETSLOT => trigger_allowed(s, &[INV_BUTTOND])?,
                    _ => trigger_allowed(s, &used)?,
                }
                let p = pl(w, s)?;
                let v = match opcode {
                    op::LAST_ITEM => p.last_item,
                    op::LAST_SLOT => p.last_slot,
                    op::LAST_USEITEM => p.last_use_item,
                    op::LAST_USESLOT => p.last_use_slot,
                    _ => p.last_target_slot,
                };
                s.push_int(v);
            }
            op::MES => {
                let m = s.pop_string();
                let slot = ap(w, s)?;
                w.message_game(slot, &m);
            }
            op::NAME => {
                let v = pl(w, s)?.username.clone();
                s.push_string(v);
            }
            op::P_APRANGE => {
                let r = not_null(s.pop_int())?;
                let p = pl(w, s)?;
                p.p.ap_range = r;
                p.p.ap_range_called = true;
            }
            op::P_ARRIVEDELAY => {
                let p = pl(w, s)?;
                if p.p.last_movement < tick {
                    return Ok(());
                }
                p.p.delayed = true;
                p.p.delayed_until = tick + 1;
                s.execution = state::Execution::Suspended;
            }
            op::P_COUNTDIALOG => {
                let slot = ap(w, s)?;
                w.write(slot, M::PCountDialog);
                s.execution = state::Execution::CountDialog;
            }
            op::P_DELAY => {
                ap(w, s)?;
                let d = not_null(s.pop_int())?;
                let p = pl(w, s)?;
                p.p.delayed = true;
                p.p.delayed_until = tick + 1 + d;
                s.execution = state::Execution::Suspended;
            }
            op::P_OPHELD => return Err("unimplemented".into()),
            op::P_OPLOC | op::P_OPNPC | op::P_OPOBJ | op::P_OPPLAYER => {
                let n = not_null(s.pop_int())? - 1;
                let name = match opcode {
                    op::P_OPLOC => "oploc",
                    op::P_OPNPC => "opnpc",
                    op::P_OPOBJ => "opobj",
                    _ => "opplayer",
                };
                if !(0..5).contains(&n) {
                    return Err(format!("Invalid {name}: {}", n + 1));
                }
                let slot = ap(w, s)?;
                let has_op = |ops: &Option<Vec<Option<String>>>| ops.as_ref().and_then(|o| o.get(n as usize).cloned().flatten()).is_some_and(|o| !o.is_empty());
                match opcode {
                    op::P_OPLOC => {
                        let loc = s.active_loc()?;
                        let kind = w.loc(loc).kind();
                        if !w.configs.locs.get(kind as usize).is_some_and(|t| has_op(&t.op)) {
                            return Ok(());
                        }
                        w.stop_action(slot);
                        if !w.player_in_operable(slot, EntityRef::Loc(loc)) {
                            let (x, z) = (w.loc(loc).e.x, w.loc(loc).e.z);
                            w.pl(slot).p.queue_waypoint(x, z);
                        }
                        w.player_set_interaction(slot, EntityRef::Loc(loc), trigger::APLOC1 + n, None);
                    }
                    op::P_OPNPC => {
                        let nid = s.active_npc()?;
                        let kind = w.npc(nid).ok_or("Attempt to access null active_npc")?.kind;
                        if !w.configs.npcs.get(kind as usize).is_some_and(|t| has_op(&t.op)) {
                            return Ok(());
                        }
                        w.stop_action(slot);
                        w.player_set_interaction(slot, EntityRef::Npc(nid), trigger::APNPC1 + n, None);
                    }
                    op::P_OPOBJ => {
                        let obj = s.active_obj()?;
                        let kind = w.obj(obj).kind;
                        // `type.op[op] === null`: an absent entry (undefined) still proceeds
                        let null = w.configs.objs.get(kind as usize).is_some_and(|t| t.op.get(n as usize).is_some_and(|e| e.is_none()));
                        if null {
                            return Ok(());
                        }
                        w.stop_action(slot);
                        let (x, z) = (w.obj(obj).e.x, w.obj(obj).e.z);
                        w.pl(slot).p.queue_waypoint(x, z);
                        w.player_set_interaction(slot, EntityRef::Obj(obj), trigger::APOBJ1 + n, None);
                    }
                    _ => {
                        let Some(target) = s.active_player[1] else { return Ok(()) };
                        w.stop_action(slot);
                        w.player_set_interaction(slot, EntityRef::Player(target), trigger::APPLAYER1 + n, None);
                    }
                }
            }
            op::P_OPNPCT => {
                let spell = not_null(s.pop_int())?;
                let slot = ap(w, s)?;
                let nid = s.active_npc()?;
                w.stop_action(slot);
                w.player_set_interaction(slot, EntityRef::Npc(nid), trigger::APNPCT, Some(spell));
            }
            op::P_OPPLAYERT => {
                let spell = not_null(s.pop_int())?;
                let Some(target) = s.active_player[1] else { return Ok(()) };
                let slot = ap(w, s)?;
                w.stop_action(slot);
                w.player_set_interaction(slot, EntityRef::Player(target), trigger::APPLAYERT, Some(spell));
            }
            op::P_PAUSEBUTTON => s.execution = state::Execution::PauseButton,
            op::P_STOPACTION => {
                let slot = ap(w, s)?;
                w.stop_action(slot);
            }
            op::P_CLEARPENDINGACTION => {
                let slot = ap(w, s)?;
                w.clear_pending_action(slot);
            }
            op::P_TELEJUMP | op::P_TELEPORT => {
                let (level, x, z) = coord(s.pop_int())?;
                let slot = ap(w, s)?;
                if opcode == op::P_TELEJUMP {
                    w.player_tele_jump(slot, x, z, level);
                } else {
                    w.teleport(crate::engine::world::Mover::Player(slot), x, z, level);
                }
            }
            op::P_TRANSMOGRIFY => {
                let id = s.pop_int();
                if id < -1 || id >= w.configs.npcs.count() as i32 {
                    return Err("Invalid npc.".into());
                }
                pl(w, s)?.npc_id = id;
            }
            op::P_WALK => {
                let (_, x, z) = coord(s.pop_int())?;
                let slot = ap(w, s)?;
                let p = w.pl(slot);
                let (level, px, pz) = (p.e.level, p.e.x, p.e.z);
                let waypoints = w.path_finder.find_path(&w.collision, level, px, pz, x, z, 1, 1, 1, 0, -1, true, 0, 25, crate::routefinder::collision_type::NORMAL);
                w.pl(slot).p.queue_waypoints(&waypoints);
            }
            op::SAY => {
                let text = s.pop_string();
                let slot = ap(w, s)?;
                w.player_say(slot, text);
            }
            op::SOUND_SYNTH => {
                let [synth, loops, delay] = s.pop_ints::<3>();
                not_null(synth)?;
                let slot = ap(w, s)?;
                if !w.pl(slot).low_memory {
                    w.write(slot, M::SynthSound { synth, loops, delay });
                }
            }
            op::STAFFMODLEVEL => {
                let v = pl(w, s)?.staff_mod_level;
                s.push_int(v);
            }
            op::STAT | op::STAT_BASE => {
                let st = stat_valid(s.pop_int())?;
                let p = pl(w, s)?;
                let v = if opcode == op::STAT { p.levels[st] } else { p.base_levels[st] };
                s.push_int(v as i32);
            }
            op::STAT_TOTAL => {
                let total: i32 = pl(w, s)?.base_levels.iter().map(|&l| l as i32).sum();
                s.push_int(total);
            }
            op::STAT_ADD | op::STAT_SUB | op::STAT_BOOST | op::STAT_DRAIN | op::STAT_HEAL => {
                let [st, constant, percent] = s.pop_ints::<3>();
                let st = stat_valid(st)?;
                not_null(constant)?;
                not_null(percent)?;
                let slot = ap(w, s)?;
                let p = w.pl(slot);
                let (base, current) = (p.base_levels[st] as i32, p.levels[st] as i32);
                let of = |x: i32| crate::util::numbers::wrap_f64(constant as f64 + (x as f64 * percent as f64) / 100.0);
                let (next, stored) = match opcode {
                    op::STAT_ADD => {
                        let v = current + of(base);
                        (v, v.min(255))
                    }
                    op::STAT_SUB => {
                        let v = current - of(base);
                        (v, v.max(0))
                    }
                    op::STAT_BOOST => {
                        let boost = of(base);
                        let v = (current + boost).min(base + boost).max(current);
                        (v, v.min(255))
                    }
                    op::STAT_DRAIN => {
                        let v = current - of(current);
                        (v, v.max(0))
                    }
                    _ => {
                        let v = current + of(base);
                        (v, v.min(base).max(current))
                    }
                };
                p.levels[st] = stored as u8;
                let clears = matches!(opcode, op::STAT_ADD | op::STAT_BOOST | op::STAT_HEAL);
                if clears && st == stat::HITPOINTS && p.levels[st] >= p.base_levels[st] {
                    p.hero_points.clear();
                }
                if next != current {
                    w.change_stat(slot, st as i32);
                }
            }
            op::STAT_RANDOM => {
                let [st, low, high] = s.pop_ints::<3>();
                let level = pl(w, s)?.levels.get(st as usize).copied().unwrap_or(0) as i32;
                let clamped = level.min(99) as f64;
                let value = ((low as f64 * (99.0 - clamped)) / 98.0).floor() + ((high as f64 * (clamped - 1.0)) / 98.0).floor() + 1.0;
                let chance = (w.random.next_double() * 256.0).floor();
                s.push_int((value > chance) as i32);
            }
            op::SPOTANIM_PL => {
                let delay = not_null(s.pop_int())?;
                let height = s.pop_int();
                let spotanim = s.pop_int();
                let slot = ap(w, s)?;
                w.player_spotanim(slot, spotanim, height, delay);
            }
            op::UID => {
                let v = pl(w, s)?.uid as i32;
                s.push_int(v);
            }
            op::P_LOGOUT => pl(w, s)?.request_logout = true,
            op::P_PREVENTLOGOUT => {
                let m = s.pop_string();
                if m.is_empty() {
                    return Err("An input string was null(-1).".into());
                }
                let until = not_null(s.pop_int())?;
                let p = pl(w, s)?;
                p.prevent_logout_message = Some(m);
                p.prevent_logout_until = tick + until;
            }
            op::IF_SETCOLOUR => {
                let [com, colour] = s.pop_ints::<2>();
                not_null(com)?;
                not_null(colour)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetColour { component: com, colour: crate::util::color::rgb24to15(colour) });
            }
            op::IF_OPENCHAT | op::IF_OPENMAIN | op::IF_OPENSIDE | op::TUT_OPEN | op::TUT_FLASH | op::IF_SETTABACTIVE | op::IF_SETPLAYERHEAD | op::MINIMAP_TOGGLE => {
                let v = not_null(s.pop_int())?;
                let slot = ap(w, s)?;
                match opcode {
                    op::IF_OPENCHAT => w.open_chat_modal(slot, v),
                    op::IF_OPENMAIN => w.open_main_modal(slot, v),
                    op::IF_OPENSIDE => w.open_side_modal(slot, v),
                    op::TUT_OPEN => w.open_tutorial(slot, v),
                    op::TUT_FLASH => w.write(slot, M::TutFlash { tab: v }),
                    op::IF_SETTABACTIVE => w.write(slot, M::IfSetTabActive { tab: v }),
                    op::IF_SETPLAYERHEAD => w.write(slot, M::IfSetPlayerHead { component: v }),
                    _ => w.write(slot, M::MinimapToggle { minimap_type: v }),
                }
            }
            op::IF_OPENMAIN_SIDE => {
                let [main, side] = s.pop_ints::<2>();
                not_null(main)?;
                not_null(side)?;
                let slot = ap(w, s)?;
                w.open_main_side_modal(slot, main, side);
            }
            op::IF_SETHIDE => {
                let [com, hide] = s.pop_ints::<2>();
                not_null(com)?;
                not_null(hide)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetHide { component: com, hidden: hide == 1 });
            }
            op::IF_SETOBJECT => {
                let [com, obj, scale] = s.pop_ints::<3>();
                not_null(com)?;
                valid(&w.configs.objs, obj, "Obj")?;
                not_null(scale)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetObject { component: com, obj, scale });
            }
            op::IF_SETMODEL => {
                let [com, model] = s.pop_ints::<2>();
                not_null(com)?;
                not_null(model)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetModel { component: com, model });
            }
            op::IF_SETANIM => {
                let [com, seq] = s.pop_ints::<2>();
                not_null(com)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetAnim { component: com, seq });
            }
            op::IF_SETTAB => {
                let [com, tab] = s.pop_ints::<2>();
                not_null(tab)?;
                let slot = ap(w, s)?;
                w.set_tab(slot, com, tab);
            }
            op::IF_OPENOVERLAY => {
                let com = s.pop_int();
                let slot = ap(w, s)?;
                w.open_main_overlay(slot, com);
            }
            op::IF_SETTEXT => {
                let text = s.pop_string();
                let com = not_null(s.pop_int())?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetText { component: com, text: carry_colours(text) });
            }
            op::IF_SETNPCHEAD => {
                let [com, npc] = s.pop_ints::<2>();
                not_null(com)?;
                valid(&w.configs.npcs, npc, "Npc")?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetNpcHead { component: com, npc });
            }
            op::IF_SETPOSITION => {
                let [com, x, y] = s.pop_ints::<3>();
                not_null(com)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetPosition { component: com, x, y });
            }
            op::IF_SETSCROLLPOS => {
                let [com, y] = s.pop_ints::<2>();
                not_null(com)?;
                let slot = ap(w, s)?;
                w.write(slot, M::IfSetScrollPos { component: com, y });
            }
            op::STAT_ADVANCE => {
                let [st, xp] = s.pop_ints::<2>();
                not_null(st)?;
                not_null(xp)?;
                let slot = ap(w, s)?;
                w.player_add_xp(slot, st as usize, xp, true)?;
            }
            op::DAMAGE => {
                let amount = not_null(s.pop_int())?;
                let kind = valid(&w.configs.hitsplats, s.pop_int(), "Hitsplat")?.id as i32;
                let uid = not_null(s.pop_int())?;
                if let Some(slot) = w.player_by_uid(uid) {
                    w.player_apply_damage(slot, amount, kind);
                }
            }
            op::IF_ADDRESUMEBUTTON => {
                let com = s.pop_int();
                pl(w, s)?.resume_buttons.push(com);
            }
            op::TEXT_GENDER => {
                let [male, female] = s.pop_strings::<2>();
                let g = pl(w, s)?.gender;
                s.push_string(if g == 0 { male } else { female });
            }
            op::MIDI_SONG | op::MIDI_JINGLE => {
                let id = s.pop_int();
                let slot = ap(w, s)?;
                if !w.pl(slot).low_memory {
                    if opcode == op::MIDI_SONG {
                        w.write(slot, M::MidiSong { id });
                    } else {
                        w.play_jingle(slot, id);
                    }
                }
            }
            op::SOFTTIMER | op::SETTIMER => {
                let args = pop_script_args(s);
                let interval = s.pop_int();
                let id = s.pop_int();
                let script = timer_script(w, id)?;
                let slot = ap(w, s)?;
                w.set_player_timer(slot, if opcode == op::SOFTTIMER { 1 } else { 0 }, script, args, interval);
            }
            op::CLEARSOFTTIMER | op::CLEARTIMER => {
                let id = s.pop_int();
                let slot = ap(w, s)?;
                w.clear_player_timer(slot, id);
            }
            op::GETTIMER => {
                let id = s.pop_int();
                timer_script(w, id)?;
                let clock = pl(w, s)?.timers.iter().filter_map(|(_, t)| t.as_ref()).find(|t| t.script.id as i32 == id).map_or(-1, |t| t.clock);
                s.push_int(clock);
            }
            op::HINT_COORD => {
                let [offset, c, height] = s.pop_ints::<3>();
                let (_, x, z) = coord(c)?;
                let slot = ap(w, s)?;
                w.hint(slot, offset, 0, 0, x, z, height);
            }
            op::HINT_STOP => {
                let slot = ap(w, s)?;
                w.hint(slot, -1, 0, 0, 0, 0, 0);
            }
            op::HINT_NPC => {
                let slot = ap(w, s)?;
                let nid = w.npc(s.active_npc()?).ok_or("Attempt to access null active_npc")?.nid;
                w.hint(slot, 1, nid, 0, 0, 0, 0);
            }
            op::HINT_PL => {
                let slot = ap(w, s)?;
                let other = s.active_player[if s.int_operand() == 0 { 1 } else { 0 }].ok_or("Attempt to access null active_player")?;
                w.hint(slot, 10, 0, other, 0, 0, 0);
            }
            op::TUT_CLOSE => {
                let slot = ap(w, s)?;
                w.close_tutorial(slot)?;
            }
            op::P_EXACTMOVE => {
                let [start, end, sc, ec, dir] = s.pop_ints::<5>();
                let (_, sx, sz) = coord(start)?;
                let (_, ex, ez) = coord(end)?;
                let slot = ap(w, s)?;
                w.unset_map_flag(slot);
                w.player_exact_move(slot, sx, sz, ex, ez, sc, ec, dir);
            }
            op::BUSY => {
                let slot = ap(w, s)?;
                let v = w.busy(slot) || w.pl(slot).logging_out;
                s.push_int(v as i32);
            }
            op::BUSY2 => {
                let slot = ap(w, s)?;
                let p = w.pl(slot);
                let interaction = p.p.target.is_some() && p.p.target_op != trigger::APPLAYER3 && p.p.target_op != trigger::OPPLAYER3;
                let v = interaction || p.p.has_waypoints();
                s.push_int(v as i32);
            }
            op::GETQUEUE => {
                let id = s.pop_int();
                let slot = ap(w, s)?;
                let mut count = 0;
                for kind in [QueueType::Normal, QueueType::Weak] {
                    for n in w.queue_nodes(slot, kind) {
                        if w.pl(slot).queue_links.get(n).is_some_and(|r| r.script.id as i32 == id) {
                            count += 1;
                        }
                    }
                }
                s.push_int(count);
            }
            op::P_LOCMERGE => {
                let [sc, ec, se, nw] = s.pop_ints::<4>();
                let (_, sex, sez) = coord(se)?;
                let (_, nwx, nwz) = coord(nw)?;
                let loc = s.active_loc()?;
                let slot = ap(w, s)?;
                w.merge_loc(loc, slot, sc, ec, sez, sex, nwz, nwx);
            }
            op::LAST_LOGIN_INFO => {
                let slot = ap(w, s)?;
                w.last_login_info(slot);
            }
            op::READYANIM | op::TURNANIM | op::WALKANIM | op::WALKANIM_B | op::WALKANIM_L | op::WALKANIM_R | op::RUNANIM => {
                let v = s.pop_int();
                let id = if opcode == op::RUNANIM && v == -1 { -1 } else { valid(&w.configs.seqs, v, "Seq")?.id as i32 };
                let p = pl(w, s)?;
                *match opcode {
                    op::READYANIM => &mut p.readyanim,
                    op::TURNANIM => &mut p.turnanim,
                    op::WALKANIM => &mut p.walkanim,
                    op::WALKANIM_B => &mut p.walkanim_b,
                    op::WALKANIM_L => &mut p.walkanim_l,
                    op::WALKANIM_R => &mut p.walkanim_r,
                    _ => &mut p.runanim,
                } = id;
            }
            op::GENDER => {
                let v = pl(w, s)?.gender;
                s.push_int(v);
            }
            op::HEADICONS_GET => {
                let v = pl(w, s)?.headicons;
                s.push_int(v);
            }
            op::HEADICONS_SET => {
                let v = not_null(s.pop_int())?;
                pl(w, s)?.headicons = v;
            }
            op::ALLOWDESIGN => {
                let v = not_null(s.pop_int())?;
                pl(w, s)?.allow_design = v == 1;
            }
            op::WALKTRIGGER => {
                let v = s.pop_int();
                pl(w, s)?.p.walktrigger = v;
            }
            op::GETWALKTRIGGER => {
                let v = pl(w, s)?.p.walktrigger;
                s.push_int(v);
            }
            op::CLEARQUEUE => {
                let id = s.pop_int();
                let slot = ap(w, s)?;
                w.unlink_queued_script(slot, id, QueueType::Normal);
            }
            op::HEALENERGY => {
                let amount = not_null(s.pop_int())?;
                let p = pl(w, s)?;
                p.runenergy = (p.runenergy + amount).clamp(0, 10000);
            }
            op::AFK_EVENT => {
                let debug = w.debug;
                let p = pl(w, s)?;
                let v = (debug || p.staff_mod_level < 2) && p.afk_event_ready;
                p.afk_event_ready = false;
                s.push_int(v as i32);
            }
            op::LOWMEM => {
                let v = pl(w, s)?.low_memory;
                s.push_int(v as i32);
            }
            op::SETIDKIT => {
                let [idkit, colour] = s.pop_ints::<2>();
                let t = valid(&w.configs.idks, idkit, "Idk")?;
                let (kind, id) = (t.kind, t.id as i32);
                let p = pl(w, s)?;
                let ty = if p.gender == 1 { kind - 7 } else { kind };
                if let Some(b) = p.body.get_mut(ty as usize) {
                    *b = id;
                }
                let colour_slot = match ty {
                    0 | 1 => Some(0),
                    2 | 3 => Some(1),
                    5 => Some(2),
                    6 => Some(3),
                    _ => None,
                };
                if let Some(c) = colour_slot {
                    p.colors[c] = colour;
                }
            }
            op::SETGENDER => {
                let gender = in_range(s.pop_int(), 0, 1, "Gender")?;
                let p = pl(w, s)?;
                for i in 0..7 {
                    p.body[i] = if gender == 1 {
                        Player::male_to_female(p.body[i])
                    } else if i == 1 {
                        14
                    } else {
                        Player::female_to_male(p.body[i])
                    };
                }
                p.gender = gender;
            }
            op::SET_SKILL_LEVEL => {
                let v = not_null(s.pop_int())?;
                pl(w, s)?.skill_level = v;
            }
            op::SETIDKCOLOUR => {
                let [slot_i, colour] = s.pop_ints::<2>();
                if !(0..=5).contains(&slot_i) {
                    return Err(format!("Invalid idk slot: {slot_i}"));
                }
                // ponytail: slot 5 does not grow `colors` to 6 (an extra appearance byte); ignored
                if let Some(c) = pl(w, s)?.colors.get_mut(slot_i as usize) {
                    *c = colour;
                }
            }
            op::FINDHERO => {
                let hero = pl(w, s)?.hero_points.find_hero();
                match (hero != -1).then(|| w.player_slot_by_hash64(hero)).flatten() {
                    Some(other) => {
                        s.active_player[1] = Some(other);
                        s.pointer_add(PointerType::ActivePlayer2);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::BOTH_HEROPOINTS => {
                let damage = not_null(s.pop_int())?;
                let secondary = s.int_operand() == 1;
                let (from, to) = if secondary { (s.active_player[1], s.active_player[0]) } else { (s.active_player[0], s.active_player[1]) };
                let (Some(from), Some(to)) = (from, to) else { return Err("player is null".into()) };
                let hash = w.pl(from).hash64;
                w.pl(to).hero_points.add_hero(hash, damage);
            }
            op::P_ANIMPROTECT => {
                let v = not_null(s.pop_int())?;
                pl(w, s)?.anim_protect = v;
            }
            op::RUNENERGY => {
                let v = pl(w, s)?.runenergy;
                s.push_int(v);
            }
            op::WEIGHT => {
                let v = pl(w, s)?.runweight;
                s.push_int(v as i32);
            }
            op::SESSION_LOG => {
                // the event is dropped
                s.pop_int();
                s.pop_string();
            }
            op::WEALTH_EVENT => {
                s.pop_string();
                s.pop_ints::<3>();
            }
            op::P_RUN => {
                let v = s.pop_int();
                let slot = ap(w, s)?;
                w.pl(slot).run = v;
                let run_varp = w.run_varp;
                w.player_set_var(slot, run_varp, v);
            }
            op::P_TEMPRUN => pl(w, s)?.temp_run = 1,
            op::PLAYERMEMBER => {
                let v = pl(w, s)?.members;
                s.push_int(v as i32);
            }
            op::SET_PLAYER_OP => {
                let text = s.pop_string();
                let [index, primary] = s.pop_ints::<2>();
                in_range(index, 1, 8, "PlayerOpIndex")?;
                in_range(primary, 0, 7, "PlayerOpState")?;
                let slot = ap(w, s)?;
                w.write(slot, M::SetPlayerOp { op: index, value: text, primary });
            }
            op::HUNTALL => {
                let [c, distance, vis] = s.pop_ints::<3>();
                let (level, x, z) = coord(c)?;
                not_null(distance)?;
                let vis = in_range(vis, 0, 2, "HuntVis")?;
                s.player_iterator = Some(crate::script::iterators::ZoneIter::player_hunt_all(tick, level, x, z, distance, vis));
            }
            op::HUNTNEXT => {
                let next = match s.player_iterator.as_mut() {
                    Some(it) => it.next(w)?,
                    None => None,
                };
                match next {
                    Some(EntityRef::Player(slot)) => {
                        s.set_active_player(slot);
                        s.push_int(1);
                    }
                    _ => s.push_int(0),
                }
            }
            // ---- CoreOps var ops on players ----
            op::PUSH_VARP | op::POP_VARP | op::PUSH_VARBIT | op::POP_VARBIT => {
                let operand = s.int_operand();
                let secondary = ((operand >> 16) & 0x1) as usize;
                let slot = s.active_player[secondary].filter(|&p| w.player(p).is_some()).ok_or_else(|| format!("No {} active_player.", if secondary != 0 { "secondary" } else { "primary" }))?;
                let protected = s.pointer_get(if secondary == 0 { PointerType::PActivePlayer } else { PointerType::PActivePlayer2 });
                match opcode {
                    op::PUSH_VARP => {
                        let t = valid(&w.configs.varps, operand & 0xffff, "Varp")?;
                        match w.player_get_var(slot, t.id as i32) {
                            Ok(v) => s.push_int(v),
                            Err(Some(v)) => s.push_string(v),
                            Err(None) => s.push_string(""),
                        }
                    }
                    op::POP_VARP => {
                        let t = valid(&w.configs.varps, operand & 0xffff, "Varp")?;
                        if !protected && t.protect {
                            return Err(format!("%{} requires protected access", t.name.clone().unwrap_or_else(|| t.id.to_string())));
                        }
                        let (id, string) = (t.id as i32, t.kind == svt::STRING);
                        if string {
                            let v = s.pop_string();
                            w.player_set_var_string(slot, id, v);
                        } else {
                            let v = s.pop_int();
                            w.player_set_var(slot, id, v);
                        }
                    }
                    op::PUSH_VARBIT => {
                        let id = valid(&w.configs.varbits, operand & 0xffff, "VarBit")?.id as i32;
                        s.push_int(w.player_get_varbit(slot, id));
                    }
                    _ => {
                        let b = valid(&w.configs.varbits, operand & 0xffff, "VarBit")?;
                        let id = b.id as i32;
                        if !protected && w.configs.varps.get(b.basevar as usize).is_some_and(|v| v.protect) {
                            return Err(format!("%{} requires protected access", b.name.clone().unwrap_or_else(|| id.to_string())));
                        }
                        let v = s.pop_int();
                        w.player_set_varbit(slot, id, v);
                    }
                }
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}
