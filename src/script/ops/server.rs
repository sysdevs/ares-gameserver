//! Server ops.

use super::super::runner::OpResult;
use super::super::state::{self, ScriptState};
use ares_shared::opcode::op;
use super::{coord, in_range, positive, valid};
use crate::engine::coord as cg;
use crate::engine::world::World;
use crate::routefinder::{flag, loc_angle, loc_layer};

/// `MAP_LOC` / `MAP_LOCADDUNSAFE`: is any active-type loc covering the tile?
fn map_loc(w: &mut World, level: i32, x: i32, z: i32, unsafe_: bool) -> Result<bool, String> {
    for dx in [-8, 0] {
        for dz in [-8, 0] {
            let list = w.zones.zone(x + dx, z + dz, level, &mut w.arenas).locs;
            for id in w.arenas.locs.collect(list) {
                let l = w.loc(id);
                if !unsafe_ && !l.e.is_active {
                    continue;
                }
                let t = valid(&w.configs.locs, l.kind(), "Loc")?;
                if t.active != 1 {
                    continue;
                }
                if unsafe_ && !l.e.is_active && l.layer() == loc_layer::WALL {
                    continue;
                }
                let swap = l.angle() == loc_angle::NORTH || l.angle() == loc_angle::SOUTH;
                let (width, length) = if swap { (l.e.length, l.e.width) } else { (l.e.width, l.e.length) };
                for index in 0..width * length {
                    if l.e.x + index % width == x && l.e.z + index / width == z {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let tick = w.current_tick;
    let members = w.map.members;
    super::handled((|| {
        match opcode {
            op::MAP_CLOCK => s.push_int(tick),
            op::MAP_MEMBERS => s.push_int(members as i32),
            op::MAP_LIVE => s.push_int(w.production as i32),
            op::MAP_PLAYERCOUNT => {
                let [c1, c2] = s.pop_ints::<2>();
                let (level, fx, fz) = coord(c1)?;
                let (_, tx, tz) = coord(c2)?;
                // the zones are created as they are visited
                let mut count = 0;
                for x in fx.div_euclid(8)..=(tx + 7).div_euclid(8) {
                    for z in fz.div_euclid(8)..=(tz + 7).div_euclid(8) {
                        let list = w.zones.zone(x << 3, z << 3, level, &mut w.arenas).players;
                        let mut c = w.arenas.members.iter(list, false);
                        while let Some(node) = w.arenas.members.next(&mut c) {
                            let Some(&crate::engine::zone::Member::Player(slot)) = w.arenas.members.get(node) else { continue };
                            if w.player_is_valid(slot) && w.player(slot).is_some_and(|p| p.e.x >= fx && p.e.x <= tx && p.e.z >= fz && p.e.z <= tz) {
                                count += 1;
                            }
                        }
                    }
                }
                s.push_int(count);
            }
            op::INZONE => {
                let [c1, c2, c3] = s.pop_ints::<3>();
                let (fl, fx, fz) = coord(c1)?;
                let (tl, tx, tz) = coord(c2)?;
                let (l, x, z) = coord(c3)?;
                let inside = (fx..=tx).contains(&x) && (fl..=tl).contains(&l) && (fz..=tz).contains(&z);
                s.push_int(inside as i32);
            }
            op::LINEOFWALK | op::LINEOFSIGHT => {
                let [c1, c2] = s.pop_ints::<2>();
                let (fl, fx, fz) = coord(c1)?;
                let (tl, tx, tz) = coord(c2)?;
                let v = if fl != tl || (!members && !w.map.is_free_to_play(tx, tz)) {
                    false
                } else if opcode == op::LINEOFWALK {
                    w.is_line_of_walk(fl, fx, fz, tx, tz)
                } else {
                    w.is_line_of_sight(fl, fx, fz, tx, tz)
                };
                s.push_int(v as i32);
            }
            op::SPOTANIM_MAP => {
                let [spotanim, c, height, delay] = s.pop_ints::<4>();
                let (level, x, z) = coord(c)?;
                let id = valid(&w.configs.spotanims, spotanim, "Spotanim")?.id as i32;
                w.anim_map(level, x, z, id, height, delay);
            }
            op::DISTANCE => {
                let [c1, c2] = s.pop_ints::<2>();
                let (_, fx, fz) = coord(c1)?;
                let (_, tx, tz) = coord(c2)?;
                s.push_int(cg::distance_to_sw(fx, fz, tx, tz));
            }
            op::MOVECOORD => {
                let [c, x, y, z] = s.pop_ints::<4>();
                let (level, px, pz) = coord(c)?;
                s.push_int(cg::pack(level.wrapping_add(y), px.wrapping_add(x), pz.wrapping_add(z)));
            }
            op::SEQLENGTH => {
                let d = valid(&w.configs.seqs, s.pop_int(), "Seq")?.duration;
                s.push_int(d);
            }
            op::COORDX | op::COORDY | op::COORDZ => {
                let (level, x, z) = coord(s.pop_int())?;
                s.push_int(match opcode {
                    op::COORDX => x,
                    op::COORDY => level,
                    _ => z,
                });
            }
            op::PLAYERCOUNT => s.push_int(w.player_count() as i32),
            op::MAP_BLOCKED => {
                let (level, x, z) = coord(s.pop_int())?;
                let v = (!members && !w.map.is_free_to_play(x, z)) || w.collision.is_flagged(x, z, level, flag::WALK_BLOCKED);
                s.push_int(v as i32);
            }
            op::MAP_INDOORS => {
                let (level, x, z) = coord(s.pop_int())?;
                s.push_int(w.collision.is_flagged(x, z, level, flag::ROOF) as i32);
            }
            op::WORLD_DELAY => s.execution = state::Execution::WorldSuspended,
            op::PROJANIM_PL | op::PROJANIM_NPC | op::PROJANIM_MAP => {
                let [src, dst, spotanim, src_height, dst_height, delay, duration, peak, arc] = s.pop_ints::<9>();
                let (sl, sx, sz);
                let id;
                let (dx, dz, target);
                if opcode == op::PROJANIM_MAP {
                    id = valid(&w.configs.spotanims, spotanim, "Spotanim")?.id as i32;
                    (sl, sx, sz) = coord(src)?;
                    let (_, x, z) = coord(dst)?;
                    (dx, dz, target) = (x, z, 0);
                } else {
                    (sl, sx, sz) = coord(src)?;
                    id = valid(&w.configs.spotanims, spotanim, "Spotanim")?.id as i32;
                    if opcode == op::PROJANIM_PL {
                        let p = w.player_by_uid(dst).and_then(|slot| w.player(slot)).ok_or_else(|| format!("attempted to use invalid player uid: {dst}"))?;
                        (dx, dz, target) = (p.e.x, p.e.z, -p.slot - 1);
                        w.map_proj_anim(sl, sx, sz, dx, dz, target, id, src_height, dst_height, delay, duration, peak, arc);
                        return Ok(());
                    }
                    let n = w.npc(dst & 0xffff).ok_or_else(|| format!("attempted to use invalid npc uid: {dst}"))?;
                    (dx, dz, target) = (n.e.x, n.e.z, n.nid + 1);
                }
                w.map_proj_anim(sl, sx, sz, dx, dz, target, id, src_height, dst_height, delay, duration, peak, arc);
            }
            op::MAP_LOCADDUNSAFE | op::MAP_LOC => {
                let (level, x, z) = coord(s.pop_int())?;
                let v = map_loc(w, level, x, z, opcode == op::MAP_LOCADDUNSAFE)?;
                s.push_int(v as i32);
            }
            op::MAP_FINDSQUARE => {
                let [c, min_radius, max_radius, kind] = s.pop_ints::<4>();
                positive(min_radius)?;
                positive(max_radius)?;
                in_range(kind, 0, 2, "FindSquare")?;
                let (level, ox, oz) = coord(c)?;
                let mut eligible = Vec::new();
                'outer: for x in ox - max_radius..=ox + max_radius {
                    for z in oz - max_radius..=oz + max_radius {
                        let d = (x - ox).abs().max((z - oz).abs());
                        if d < min_radius || d > max_radius {
                            continue;
                        }
                        if !members && !w.map.is_free_to_play(x, z) {
                            continue;
                        }
                        if w.collision.is_flagged(x, z, level, flag::WALK_BLOCKED) {
                            continue;
                        }
                        let ok = match kind {
                            0 => w.is_line_of_walk(level, x, z, ox, oz),
                            1 => w.is_line_of_sight(level, x, z, ox, oz),
                            _ => true,
                        };
                        if !ok {
                            continue;
                        }
                        eligible.push(cg::pack(level, x, z));
                        if eligible.len() >= 100 {
                            break 'outer;
                        }
                    }
                }
                if eligible.is_empty() {
                    s.push_int(c);
                } else {
                    let i = (w.math_random() * eligible.len() as f64) as usize;
                    s.push_int(eligible[i]);
                }
            }
            op::MAP_MULTIWAY => {
                let (level, x, z) = cg::unpack(s.pop_int());
                s.push_int(w.map.is_multi(x, z, level) as i32);
            }
            op::MIDI_LENGTH => {
                let track = s.pop_int();
                let v = if track < 0 { 1 } else { crate::cache::midi::tick_length(&w.midi_lengths, track as usize) };
                s.push_int(v);
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}
