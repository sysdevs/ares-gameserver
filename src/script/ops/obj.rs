//! Obj ops (wealth logging is dropped).

use super::super::iterators::ZoneIter;
use super::super::runner::OpResult;
use super::super::state::{EntityRef, ScriptState};
use ares_shared::opcode::op;
use super::config::param;
use super::{coord, duration, in_range, valid};
use crate::engine::coord as cg;
use crate::engine::entity::{NO_RECEIVER, Obj, lifecycle};
use crate::engine::inventory::STACK_LIMIT;
use crate::engine::world::World;

fn active_hash(w: &World, s: &ScriptState) -> Result<i64, String> {
    let slot = s.active_player()?;
    w.player(slot).map(|p| p.hash64).ok_or_else(|| "Attempt to access null active_player".into())
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let tick = w.current_tick;
    super::handled((|| {
        match opcode {
            op::OBJ_ADD | op::OBJ_ADDALL => {
                let [c, id, count, dur] = s.pop_ints::<4>();
                if id == -1 || count == -1 {
                    return Ok(());
                }
                let t = valid(&w.configs.objs, id, "Obj")?;
                duration(dur)?;
                let (level, x, z) = coord(c)?;
                in_range(count, 1, STACK_LIMIT as i32, "ObjStack")?;
                if t.dummyitem != 0 {
                    return Err(format!("attempted to add dummy item: {}", t.debug_name.clone().unwrap_or_else(|| t.id.to_string())));
                }
                if t.members && !w.map.members {
                    return Ok(());
                }
                let stackable = t.stackable;
                let receiver = if opcode == op::OBJ_ADD { active_hash(w, s)? } else { NO_RECEIVER };
                if !stackable || count == 1 {
                    for _ in 0..count {
                        let o = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, id, 1));
                        w.add_obj(o, receiver, dur);
                        s.set_active_obj(o);
                    }
                } else {
                    let o = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, id, count as i64));
                    w.add_obj(o, receiver, dur);
                    s.set_active_obj(o);
                }
            }
            op::OBJ_PARAM => {
                let p = valid(&w.configs.params, s.pop_int(), "Param")?;
                let kind = w.obj(s.active_obj()?).kind;
                let t = valid(&w.configs.objs, kind, "Obj")?;
                param(s, Some(&t.params), p);
            }
            op::OBJ_NAME => {
                let kind = w.obj(s.active_obj()?).kind;
                let t = valid(&w.configs.objs, kind, "Obj")?;
                s.push_string(t.name.as_ref().or(t.debug_name.as_ref()).map_or("null".into(), String::clone));
            }
            op::OBJ_DEL => {
                let o = s.active_obj()?;
                let rate = w.configs.objs.get(w.obj(o).kind as usize).map_or(0, |t| t.respawnrate);
                w.remove_obj(o, rate);
            }
            op::OBJ_COUNT => {
                let o = s.active_obj()?;
                let hash = active_hash(w, s)?;
                let obj = w.obj(o);
                s.push_int(if obj.is_valid(Some(hash)) { obj.count as i32 } else { 0 });
            }
            op::OBJ_TYPE => {
                let kind = w.obj(s.active_obj()?).kind;
                s.push_int(valid(&w.configs.objs, kind, "Obj")?.id as i32);
            }
            op::OBJ_TAKEITEM => {
                let inv = valid(&w.configs.invs, s.pop_int(), "Inv")?.id as i32;
                let o = s.active_obj()?;
                let hash = active_hash(w, s)?;
                let obj = w.obj(o);
                if !obj.is_valid(Some(hash)) {
                    return Ok(());
                }
                let (kind, count, life) = (obj.kind, obj.count, obj.e.lifecycle);
                let slot = s.active_player()?;
                super::inv::inv_add(w, slot, inv, kind, count)?;
                let rate = w.configs.objs.get(kind as usize).map_or(0, |t| t.respawnrate);
                if life == lifecycle::RESPAWN {
                    w.remove_obj(o, rate);
                } else if life == lifecycle::DESPAWN {
                    w.remove_obj(o, 0);
                }
            }
            op::OBJ_COORD => {
                let obj = w.obj(s.active_obj()?);
                s.push_int(cg::pack(obj.e.level, obj.e.x, obj.e.z));
            }
            op::OBJ_FIND => {
                let [c, id] = s.pop_ints::<2>();
                let id = valid(&w.configs.objs, id, "Obj")?.id as i32;
                let (level, x, z) = coord(c)?;
                let hash = active_hash(w, s)?;
                match w.get_obj(x, z, level, id, hash) {
                    Some(o) => {
                        s.set_active_obj(o);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::OBJ_FINDALLZONE => {
                let (level, x, z) = coord(s.pop_int())?;
                s.obj_iterator = Some(ZoneIter::objs(tick, level, x, z));
            }
            op::OBJ_FINDNEXT => {
                let next = match s.obj_iterator.as_mut() {
                    Some(it) => it.next(w)?,
                    None => None,
                };
                match next {
                    Some(EntityRef::Obj(o)) => {
                        s.set_active_obj(o);
                        s.push_int(1);
                    }
                    _ => s.push_int(0),
                }
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}
