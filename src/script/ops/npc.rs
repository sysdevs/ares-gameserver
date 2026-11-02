//! Npc ops.

use super::super::iterators::ZoneIter;
use super::super::runner::OpResult;
use super::super::state::{self, EntityRef, ScriptState};
use ares_shared::{opcode::op, trigger};
use super::config::param;
use super::{coord, duration, in_range, not_null, valid};
use crate::engine::coord as cg;
use crate::engine::entity::lifecycle;
use crate::engine::entity::npc::{HITPOINTS, Npc, npc_mode};
use crate::engine::world::World;

fn stat(v: i32) -> Result<usize, String> {
    in_range(v, 0, 5, "NpcStat").map(|v| v as usize)
}

fn vis(v: i32) -> Result<i32, String> {
    in_range(v, 0, 2, "HuntVis")
}

/// Picks the NPC nearest `(x, z)` by squared distance; ties go to the later one.
fn closest(w: &mut World, it: &mut ZoneIter, x: i32, z: i32, keep: impl Fn(&World, i32) -> bool) -> Result<Option<i32>, String> {
    let mut best: Option<i32> = None;
    let mut best_distance = i64::MAX;
    while let Some(e) = it.next(w)? {
        let EntityRef::Npc(nid) = e else { continue };
        if !keep(w, nid) {
            continue;
        }
        let n = w.npc(nid).expect("npc");
        let d = cg::euclidean_squared(x, z, n.e.x, n.e.z);
        if d <= best_distance {
            best = Some(nid);
            best_distance = d;
        }
    }
    Ok(best)
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let tick = w.current_tick;
    super::handled((|| {
        match opcode {
            op::NPC_FINDUID => {
                let uid = s.pop_int();
                let (slot, expected) = (uid & 0xffff, (uid >> 16) & 0xffff);
                match w.npc(slot) {
                    Some(n) if n.kind == expected => {
                        s.set_active_npc(slot);
                        s.push_int(1);
                    }
                    _ => s.push_int(0),
                }
            }
            op::NPC_ADD => {
                let [c, id, dur] = s.pop_ints::<3>();
                let (level, x, z) = coord(c)?;
                let t = valid(&w.configs.npcs, id, "Npc")?;
                duration(dur)?;
                let nid = w.npcs.next_id() as i32;
                let npc = Npc::new(level, x, z, t.size, t.size, lifecycle::DESPAWN, nid, t, t.blockwalk, w.configs.varns.count());
                w.add_npc(npc, dur);
                s.set_active_npc(nid);
            }
            op::NPC_ANIM => {
                let delay = not_null(s.pop_int())?;
                let seq = s.pop_int();
                let nid = s.active_npc()?;
                w.npc_play_animation(nid, seq, delay)?;
            }
            op::NPC_BASESTAT | op::NPC_STAT => {
                let st = stat(s.pop_int())?;
                let n = active(w, s)?;
                let v = if opcode == op::NPC_STAT { n.levels[st] } else { n.base_levels[st] };
                s.push_int(v as i32);
            }
            op::NPC_CATEGORY | op::NPC_TYPE | op::NPC_NAME | op::NPC_ATTACKRANGE => {
                let kind = active(w, s)?.kind;
                let t = valid(&w.configs.npcs, kind, "Npc")?;
                match opcode {
                    op::NPC_CATEGORY => s.push_int(t.category),
                    op::NPC_TYPE => s.push_int(t.id as i32),
                    op::NPC_NAME => s.push_string(t.name.clone().unwrap_or("null".into())),
                    _ => s.push_int(t.attackrange),
                }
            }
            op::NPC_COORD => {
                let n = active(w, s)?;
                s.push_int(cg::pack(n.e.level, n.e.x, n.e.z));
            }
            op::NPC_DEL => {
                let nid = s.active_npc()?;
                let kind = active(w, s)?.kind;
                let rate = valid(&w.configs.npcs, kind, "Npc")?.respawnrate;
                if let Some(npc) = w.remove_npc(nid, rate) {
                    let kept = w.keep_deleted_npc(npc);
                    for a in s.active_npc.iter_mut().filter(|a| **a == Some(nid)) {
                        *a = Some(kept);
                    }
                }
            }
            op::NPC_DELAY => {
                let nid = s.active_npc()?;
                active(w, s)?;
                let delay = not_null(s.pop_int())?;
                let n = w.npc_mut(nid).unwrap();
                n.p.delayed = true;
                n.p.delayed_until = tick + 1 + delay;
                s.execution = state::Execution::NpcSuspended;
            }
            op::NPC_FACESQUARE => {
                let (_, x, z) = coord(s.pop_int())?;
                active_mut(w, s)?.face_square(x, z);
            }
            op::NPC_FINDEXACT => {
                let [c, id] = s.pop_ints::<2>();
                let (level, x, z) = coord(c)?;
                let kind = valid(&w.configs.npcs, id, "Npc")?.id as i32;
                let mut it = ZoneIter::npc_zone(tick, level, x, z);
                let mut found = None;
                while let Some(e) = it.next(w)? {
                    let EntityRef::Npc(nid) = e else { continue };
                    let n = w.npc(nid).unwrap();
                    if n.kind == kind && n.e.x == x && n.e.level == level && n.e.z == z {
                        found = Some(nid);
                        break;
                    }
                }
                s.npc_iterator = Some(it);
                match found {
                    Some(nid) => {
                        s.set_active_npc(nid);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::NPC_FINDHERO => {
                let hero = active(w, s)?.hero_points.find_hero();
                match (hero != -1).then(|| w.player_slot_by_hash64(hero)).flatten() {
                    Some(slot) => {
                        s.set_active_player(slot);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::NPC_PARAM => {
                let p = valid(&w.configs.params, s.pop_int(), "Param")?;
                let kind = active(w, s)?.kind;
                let t = valid(&w.configs.npcs, kind, "Npc")?;
                param(s, Some(&t.params), p);
            }
            op::NPC_QUEUE => {
                let delay = not_null(s.pop_int())?;
                let arg = s.pop_int();
                let queue = in_range(s.pop_int(), 0, 19, "AIQueue")?;
                active_mut(w, s)?.enqueue_script(trigger::AI_QUEUE1 + queue - 1, delay, arg);
            }
            op::NPC_RANGE => {
                let (level, x, z) = coord(s.pop_int())?;
                let n = active(w, s)?;
                if level != n.e.level {
                    s.push_int(-1);
                } else {
                    s.push_int(cg::distance_to((n.e.x, n.e.z, n.e.width, n.e.length), (x, z, 1, 1)));
                }
            }
            op::NPC_SAY => {
                let text = s.pop_string();
                active_mut(w, s)?.say(text);
            }
            op::NPC_SETHUNT => {
                let v = not_null(s.pop_int())?;
                active_mut(w, s)?.huntrange = v;
            }
            op::NPC_SETHUNTMODE => {
                let id = s.pop_int();
                let mode = if id == -1 { -1 } else { valid(&w.configs.hunts, id, "Hunt")?.id as i32 };
                active_mut(w, s)?.hunt_mode = mode;
            }
            op::NPC_SETMODE => {
                let mode = in_range(s.pop_int(), npc_mode::NULL, npc_mode::APNPC5, "NpcMode")?;
                let nid = s.active_npc()?;
                active(w, s)?;
                if mode == npc_mode::NONE || mode == npc_mode::WANDER || mode == npc_mode::PATROL {
                    let n = w.npc_mut(nid).unwrap();
                    n.clear_interaction();
                    n.p.target_op = mode;
                    if mode == npc_mode::PATROL {
                        n.clear_patrol();
                    }
                    return Ok(());
                } else if mode == npc_mode::NULL {
                    w.npc_reset_defaults(nid);
                    return Ok(());
                }
                w.npc_mut(nid).unwrap().p.target_op = mode;
                let target = if mode >= npc_mode::OPNPC1 {
                    let n = if s.int_operand() == 0 { s.active_npc[1] } else { s.active_npc[0] };
                    n.map(EntityRef::Npc)
                } else if mode >= npc_mode::OPOBJ1 {
                    s.active_obj[0].map(EntityRef::Obj)
                } else if mode >= npc_mode::OPLOC1 {
                    s.active_loc[0].map(EntityRef::Loc)
                } else {
                    s.active_player[0].map(EntityRef::Player)
                };
                match target {
                    Some(t) => {
                        w.npc_set_interaction(nid, t, mode);
                    }
                    None => w.npc_reset_defaults(nid),
                }
            }
            op::NPC_STATHEAL | op::NPC_STATADD | op::NPC_STATSUB => {
                let [st, constant, percent] = s.pop_ints::<3>();
                let st = stat(st)?;
                not_null(constant)?;
                not_null(percent)?;
                let n = active_mut(w, s)?;
                let (base, current) = (n.base_levels[st] as i32, n.levels[st] as i32);
                let delta = crate::util::numbers::wrap_f64(constant as f64 + (base as f64 * percent as f64) / 100.0);
                let v = match opcode {
                    op::NPC_STATHEAL => (current + delta).min(base),
                    op::NPC_STATADD => (current + delta).min(255),
                    _ => (current - delta).max(0),
                };
                n.levels[st] = v as u16;
                if opcode == op::NPC_STATHEAL && st == HITPOINTS && n.levels[st] >= n.base_levels[st] {
                    n.hero_points.clear();
                }
            }
            op::NPC_DAMAGE => {
                let amount = not_null(s.pop_int())?;
                let kind = valid(&w.configs.hitsplats, s.pop_int(), "Hitsplat")?.id as i32;
                active_mut(w, s)?.apply_damage(amount, kind);
            }
            op::NPC_UID => {
                let uid = active(w, s)?.uid;
                s.push_int(uid);
            }
            op::NPC_SETTIMER => {
                let v = not_null(s.pop_int())?;
                active_mut(w, s)?.set_timer(v);
            }
            op::SPOTANIM_NPC => {
                let delay = not_null(s.pop_int())?;
                let height = not_null(s.pop_int())?;
                let id = valid(&w.configs.spotanims, s.pop_int(), "Spotanim")?.id as i32;
                active_mut(w, s)?.spotanim(id, height, delay);
            }
            op::NPC_HUNT => {
                let [c, distance, check_vis] = s.pop_ints::<3>();
                let (level, x, z) = coord(c)?;
                not_null(distance)?;
                let v = vis(check_vis)?;
                let mut it = ZoneIter::npc_hunt_all(tick, level, x, z, distance, v);
                match closest(w, &mut it, x, z, |_, _| true)? {
                    Some(nid) => {
                        s.set_active_npc(nid);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::NPC_HUNTALL => {
                let [c, distance, check_vis] = s.pop_ints::<3>();
                let (level, x, z) = coord(c)?;
                not_null(distance)?;
                let v = vis(check_vis)?;
                s.npc_iterator = Some(ZoneIter::npc_hunt_all(tick, level, x, z, distance, v));
            }
            op::NPC_FIND | op::NPC_FINDCAT => {
                let [c, id, distance, check_vis] = s.pop_ints::<4>();
                let (level, x, z) = coord(c)?;
                let id = if opcode == op::NPC_FIND { valid(&w.configs.npcs, id, "Npc")?.id as i32 } else { valid(&w.configs.categories, id, "Cat")?; id };
                not_null(distance)?;
                let v = vis(check_vis)?;
                let mut it = ZoneIter::npc_distance(tick, level, x, z, distance, v, None);
                let keep = |w: &World, nid: i32| {
                    let kind = w.npc(nid).unwrap().kind;
                    if opcode == op::NPC_FIND { kind == id } else { w.configs.npcs.get(kind as usize).is_some_and(|t| t.category == id) }
                };
                match closest(w, &mut it, x, z, keep)? {
                    Some(nid) => {
                        s.set_active_npc(nid);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::NPC_FINDALLANY => {
                let [c, distance, check_vis] = s.pop_ints::<3>();
                let (level, x, z) = coord(c)?;
                not_null(distance)?;
                let v = vis(check_vis)?;
                s.npc_iterator = Some(ZoneIter::npc_distance(tick, level, x, z, distance, v, None));
            }
            op::NPC_FINDALL => {
                let [c, id, distance, check_vis] = s.pop_ints::<4>();
                let (level, x, z) = coord(c)?;
                not_null(distance)?;
                let kind = valid(&w.configs.npcs, id, "Npc")?.id as i32;
                let v = vis(check_vis)?;
                s.npc_iterator = Some(ZoneIter::npc_distance(tick, level, x, z, distance, v, Some(kind)));
            }
            op::NPC_FINDALLZONE => {
                let (level, x, z) = coord(s.pop_int())?;
                s.npc_iterator = Some(ZoneIter::npc_zone(tick, level, x, z));
            }
            op::NPC_FINDNEXT => {
                let next = match s.npc_iterator.as_mut() {
                    Some(it) => it.next(w)?,
                    None => None,
                };
                match next {
                    Some(EntityRef::Npc(nid)) => {
                        s.set_active_npc(nid);
                        s.push_int(1);
                    }
                    _ => s.push_int(0),
                }
            }
            op::NPC_TELE => {
                let (level, x, z) = coord(s.pop_int())?;
                let nid = s.active_npc()?;
                active(w, s)?;
                w.npc_teleport(nid, x, z, level);
            }
            op::NPC_WALK => {
                let (_, x, z) = coord(s.pop_int())?;
                active_mut(w, s)?.p.queue_waypoint(x, z);
            }
            op::NPC_CHANGETYPE | op::NPC_CHANGETYPE_KEEPALL => {
                let [id, dur] = s.pop_ints::<2>();
                let kind = valid(&w.configs.npcs, id, "Npc")?.id as i32;
                duration(dur)?;
                let nid = s.active_npc()?;
                active(w, s)?;
                w.npc_change_type(nid, kind, dur, opcode == op::NPC_CHANGETYPE);
            }
            op::NPC_GETMODE => {
                let mode = active(w, s)?.p.target_op;
                s.push_int(mode);
            }
            op::NPC_HEROPOINTS => {
                active(w, s)?;
                // `state.activePlayer.hash64` is read before the points are popped
                let slot = s.active_player()?;
                let hash64 = w.player(slot).ok_or("Attempt to access null active_player")?.hash64;
                let points = not_null(s.pop_int())?;
                active_mut(w, s)?.hero_points.add_hero(hash64, points);
            }
            op::NPC_WALKTRIGGER => {
                let [queue, arg] = s.pop_ints::<2>();
                in_range(queue, 0, 19, "AIQueue")?;
                let n = active_mut(w, s)?;
                n.p.walktrigger = queue - 1;
                n.p.walktrigger_arg = arg;
            }
            op::NPC_HASOP => {
                let n = not_null(s.pop_int())?;
                let kind = active(w, s)?.kind;
                let t = w.configs.npcs.get(kind as usize).ok_or("npc type")?;
                let has = t.op.as_ref().and_then(|ops| ops.get((n - 1) as usize).cloned().flatten()).is_some_and(|o| !o.is_empty());
                s.push_int(has as i32);
            }
            op::NPC_ARRIVEDELAY => {
                let n = active_mut(w, s)?;
                if n.p.last_movement < tick - 1 {
                    return Ok(());
                }
                n.p.delayed = true;
                n.p.delayed_until = if n.p.last_movement == tick - 1 { tick + 1 } else { tick + 2 };
                s.execution = state::Execution::NpcSuspended;
            }
            op::NPC_INRANGE => {
                let nid = s.active_npc()?;
                active(w, s)?;
                s.push_int(w.npc_target_within_max_range(nid) as i32);
            }
            op::NPC_DESTINATION => {
                let n = active(w, s)?;
                let v = if n.p.has_waypoints() { n.p.waypoints[0] } else { cg::pack(n.e.level, n.e.x, n.e.z) };
                s.push_int(v);
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}

fn active<'a>(w: &'a World, s: &ScriptState) -> Result<&'a Npc, String> {
    let nid = s.active_npc()?;
    w.npc(nid).ok_or_else(|| "Attempt to access null active_npc".into())
}

fn active_mut<'a>(w: &'a mut World, s: &ScriptState) -> Result<&'a mut Npc, String> {
    let nid = s.active_npc()?;
    w.npc_mut(nid).ok_or_else(|| "Attempt to access null active_npc".into())
}
