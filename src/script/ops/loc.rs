//! Loc ops.

use super::super::iterators::ZoneIter;
use super::super::runner::OpResult;
use super::super::state::{EntityRef, ScriptState};
use ares_shared::opcode::op;
use super::config::param;
use super::{coord, duration, in_range, valid};
use crate::engine::coord as cg;
use crate::engine::entity::{Loc, lifecycle};
use crate::engine::world::World;
use crate::routefinder::loc_shape_layer;

fn angle(v: i32) -> Result<i32, String> {
    in_range(v, 0, 3, "LocAngle")
}

fn shape(v: i32) -> Result<i32, String> {
    in_range(v, 0, 22, "LocShape")
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let tick = w.current_tick;
    super::handled((|| {
        match opcode {
            op::LOC_ADD => {
                let [c, kind, a, sh, dur] = s.pop_ints::<5>();
                let (level, x, z) = coord(c)?;
                let t = valid(&w.configs.locs, kind, "Loc")?;
                let (width, length, id) = (t.width, t.length, t.id as i32);
                let a = angle(a)?;
                let sh = shape(sh)?;
                let layer = loc_shape_layer(sh);
                duration(dur)?;
                let list = w.zones.zone(x, z, level, &mut w.arenas).locs;
                let tile = cg::pack_zone_coord(x, z);
                let existing = w.arenas.locs.collect(list).into_iter().find(|&l| {
                    let l = w.loc(l);
                    cg::pack_zone_coord(l.e.x, l.e.z) == tile && l.layer() == layer
                });
                let loc = match existing {
                    Some(l) => {
                        w.change_loc(l, kind, sh, a, dur);
                        l
                    }
                    None => {
                        let l = w.new_loc(Loc::new(level, x, z, width, length, lifecycle::DESPAWN, id, sh, a));
                        w.add_loc(l, dur);
                        l
                    }
                };
                s.set_active_loc(loc);
            }
            op::LOC_ANGLE => {
                let a = w.loc(s.active_loc()?).angle();
                s.push_int(angle(a)?);
            }
            op::LOC_ANIM => {
                let seq = valid(&w.configs.seqs, s.pop_int(), "Seq")?.id as i32;
                w.anim_loc(s.active_loc()?, seq);
            }
            op::LOC_CATEGORY | op::LOC_TYPE | op::LOC_NAME => {
                let kind = w.loc(s.active_loc()?).kind();
                let t = valid(&w.configs.locs, kind, "Loc")?;
                match opcode {
                    op::LOC_CATEGORY => s.push_int(t.category),
                    op::LOC_TYPE => s.push_int(t.id as i32),
                    _ => s.push_string(t.name.clone().unwrap_or("null".into())),
                }
            }
            op::LOC_CHANGE => {
                let [kind, dur] = s.pop_ints::<2>();
                duration(dur)?;
                valid(&w.configs.locs, kind, "Loc")?;
                let id = s.active_loc()?;
                let (sh, a) = (w.loc(id).shape(), w.loc(id).angle());
                w.change_loc(id, kind, sh, a, dur);
            }
            op::LOC_COORD => {
                let l = w.loc(s.active_loc()?);
                s.push_int(cg::pack(l.e.level, l.e.x, l.e.z));
            }
            op::LOC_DEL => {
                let dur = duration(s.pop_int())?;
                w.remove_loc(s.active_loc()?, dur);
            }
            op::LOC_FIND => {
                let [c, kind] = s.pop_ints::<2>();
                let kind = valid(&w.configs.locs, kind, "Loc")?.id as i32;
                let (level, x, z) = coord(c)?;
                match w.get_loc(x, z, level, kind) {
                    Some(l) => {
                        s.set_active_loc(l);
                        s.push_int(1);
                    }
                    None => s.push_int(0),
                }
            }
            op::LOC_FINDALLZONE => {
                let (level, x, z) = coord(s.pop_int())?;
                s.loc_iterator = Some(ZoneIter::locs(tick, level, x, z));
            }
            op::LOC_FINDNEXT => {
                let next = match s.loc_iterator.as_mut() {
                    Some(it) => it.next(w)?,
                    None => None,
                };
                match next {
                    Some(EntityRef::Loc(l)) => {
                        s.set_active_loc(l);
                        s.push_int(1);
                    }
                    _ => s.push_int(0),
                }
            }
            op::LOC_PARAM => {
                let p = valid(&w.configs.params, s.pop_int(), "Param")?;
                let kind = w.loc(s.active_loc()?).kind();
                let t = valid(&w.configs.locs, kind, "Loc")?;
                param(s, Some(&t.params), p);
            }
            op::LOC_SHAPE => {
                let sh = w.loc(s.active_loc()?).shape();
                s.push_int(shape(sh)?);
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}
