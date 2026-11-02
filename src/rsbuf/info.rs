//! The player and NPC info encoders.
//!
//! Both reuse their 5000-byte buffers without clearing them; that
//! is harmless because `pbit` also zeroes every later bit of each byte it
//! touches, so no stale padding can reach the output.

use super::build::{PREFERRED_NPCS, PREFERRED_PLAYERS, PREFERRED_VIEW_DISTANCE};
use super::renderer::{NpcRenderer, PlayerRenderer, face_coord, face_entity, write_exact_move};
use super::{Npc, Packet, Player, ZoneMap, fine, npc_prot as np, player_prot as pp, visibility};
use std::collections::HashMap;

const P_BITS_ADD: usize = 11 + 5 + 5 + 1 + 1;
const N_BITS_ADD: usize = 14 + 11 + 5 + 5 + 1;
const BITS_RUN: usize = 1 + 2 + 3 + 3 + 1;
const BITS_WALK: usize = 1 + 2 + 3 + 1;
const BITS_EXTEND: usize = 1 + 2;

struct Bufs {
    buf: Packet,
    updates: Packet,
}

impl Default for Bufs {
    fn default() -> Self {
        Bufs { buf: Packet::new(5000), updates: Packet::new(5000) }
    }
}

impl Bufs {
    fn reset(&mut self) {
        self.buf.pos = 0;
        self.buf.bit_pos = 0;
        self.updates.pos = 0;
        self.updates.bit_pos = 0;
        self.buf.bits();
    }

    /// The terminator id, then the update blocks after the bit section.
    fn finish(&mut self, bits: u32, terminator: i32) -> Vec<u8> {
        if self.updates.pos > 0 {
            self.buf.pbit(bits, terminator);
            self.buf.bytes();
            let n = self.updates.pos;
            self.buf.data[self.buf.pos..self.buf.pos + n].copy_from_slice(&self.updates.data[..n]);
            self.buf.pos += n;
        } else {
            self.buf.bytes();
        }
        self.buf.data[..self.buf.pos].to_vec()
    }

    fn fits(&self, bytes: i32, bits: usize, add: i32) -> bool {
        (((self.buf.bit_pos + bits + 7) >> 3) as i32) + bytes + add <= 4997
    }

    fn idle(&mut self) {
        self.buf.pbit(1, 0);
    }

    fn remove_bits(&mut self) {
        self.buf.pbit(1, 1);
        self.buf.pbit(2, 3);
    }
}

#[derive(Default)]
pub(crate) struct PlayerInfoEncoder(Bufs);

impl PlayerInfoEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        pos: i32,
        r: &mut PlayerRenderer,
        players: &mut [Option<Player>],
        map: &mut ZoneMap,
        grid: &HashMap<u32, Vec<i32>>,
        pid: i32,
        dx: i32,
        dz: i32,
        rebuild: bool,
    ) -> Vec<u8> {
        // the local player leaves the table while it is encoded; every lookup
        // of it by id would have been filtered out as "self" anyway
        let mut me = players[pid as usize].take().expect("player");
        let (x, y, z) = (me.coord.x(), me.coord.y(), me.coord.z());
        if rebuild || dx > me.build.view_distance || dz > me.build.view_distance {
            me.build.rebuild_players(players, grid, pid, x, y, z);
        } else {
            me.build.resize();
        }
        self.0.reset();
        let bytes1 = self.write_local(r, &me);
        let bytes2 = self.write_players(players, r, &mut me, bytes1 + pos);
        self.write_new(map, players, r, grid, &mut me, bytes2);
        players[pid as usize] = Some(me);
        self.0.finish(11, 2047)
    }

    fn write_local(&mut self, r: &PlayerRenderer, me: &Player) -> i32 {
        let length = r.highs(me.pid);
        let u = &me.u;
        if u.tele {
            let x = me.coord.x() - (((me.origin.x() >> 3) - 6) << 3);
            let z = me.coord.z() - (((me.origin.z() >> 3) - 6) << 3);
            let b = &mut self.0.buf;
            b.pbit(1, 1);
            b.pbit(2, 3);
            b.pbit(2, me.coord.y());
            b.pbit(7, x);
            b.pbit(7, z);
            b.pbit(1, u.jump as i32);
            self.extend_bit(r, me, me, length > 0);
        } else if u.run_dir != -1 {
            self.run(r, me, me, length > 0);
        } else if u.walk_dir != -1 {
            self.walk(r, me, me, length > 0);
        } else if length > 0 {
            self.extend(r, me, me);
        } else {
            self.0.idle();
        }
        length
    }

    fn write_players(&mut self, players: &[Option<Player>], r: &PlayerRenderer, me: &mut Player, start: i32) -> i32 {
        let mut bytes = start;
        self.0.buf.pbit(8, me.build.players.len() as i32);
        for pid in me.build.players.iter() {
            let other = players[pid as usize].as_ref();
            let gone = other.is_none_or(|o| {
                o.pid == -1
                    || o.u.tele
                    || o.coord.y() != me.coord.y()
                    || !me.coord.within_distance_sw(o.coord, me.build.view_distance)
                    || !o.u.active
                    || o.u.visibility == visibility::HARD
            });
            if gone {
                self.0.remove_bits();
                me.build.players.remove(pid);
                continue;
            }
            let o = other.unwrap();
            let length = r.highs(pid);
            if o.u.run_dir != -1 {
                let ext = length > 0 && self.0.fits(bytes + 2, BITS_RUN, length);
                self.run(r, me, o, ext);
            } else if o.u.walk_dir != -1 {
                let ext = length > 0 && self.0.fits(bytes + 2, BITS_WALK, length);
                self.walk(r, me, o, ext);
            } else if length > 0 && self.0.fits(bytes + 2, BITS_EXTEND, length) {
                self.extend(r, me, o);
            } else {
                self.0.idle();
            }
            bytes += length + 2;
        }
        bytes
    }

    fn write_new(&mut self, map: &mut ZoneMap, players: &[Option<Player>], r: &mut PlayerRenderer, grid: &HashMap<u32, Vec<i32>>, me: &mut Player, start: i32) {
        let mut bytes = start;
        let (x, y, z) = (me.coord.x(), me.coord.y(), me.coord.z());
        for pid in me.build.nearby_players(players, grid, map, me.pid, x, y, z) {
            if me.build.players.contains(pid) {
                continue;
            }
            if me.build.players.len() >= PREFERRED_PLAYERS {
                return;
            }
            let Some(o) = players[pid as usize].as_ref().filter(|o| o.u.visibility != visibility::HARD) else { continue };
            let length = r.lows(pid) + r.highs(pid);
            if !self.0.fits(bytes + 2, P_BITS_ADD, length) {
                return;
            }
            let b = &mut self.0.buf;
            b.pbit(11, o.pid);
            b.pbit(5, o.coord.x() - x);
            b.pbit(5, o.coord.z() - z);
            b.pbit(1, o.u.jump as i32);
            b.pbit(1, 1);
            self.low_definition(r, me, o);
            me.build.players.insert(o.pid);
            bytes += length + 2;
        }
    }

    fn extend_bit(&mut self, r: &PlayerRenderer, me: &Player, o: &Player, extend: bool) {
        if extend {
            self.0.buf.pbit(1, 1);
            self.high_definition(r, me, o);
        } else {
            self.0.buf.pbit(1, 0);
        }
    }

    fn run(&mut self, r: &PlayerRenderer, me: &Player, o: &Player, extend: bool) {
        let b = &mut self.0.buf;
        b.pbit(1, 1);
        b.pbit(2, 2);
        b.pbit(3, o.u.walk_dir);
        b.pbit(3, o.u.run_dir);
        self.extend_bit(r, me, o, extend);
    }

    fn walk(&mut self, r: &PlayerRenderer, me: &Player, o: &Player, extend: bool) {
        let b = &mut self.0.buf;
        b.pbit(1, 1);
        b.pbit(2, 1);
        b.pbit(3, o.u.walk_dir);
        self.extend_bit(r, me, o, extend);
    }

    fn extend(&mut self, r: &PlayerRenderer, me: &Player, o: &Player) {
        self.0.buf.pbit(1, 1);
        self.0.buf.pbit(2, 0);
        self.high_definition(r, me, o);
    }

    fn high_definition(&mut self, r: &PlayerRenderer, me: &Player, o: &Player) {
        let mut masks = o.u.masks;
        if me.pid == o.pid {
            masks &= !pp::CHAT;
        }
        self.write_blocks(r, me, o, masks);
    }

    fn low_definition(&mut self, r: &mut PlayerRenderer, me: &mut Player, o: &Player) {
        let pid = o.pid;
        let mut masks = o.u.masks;
        let last = o.u.last_appearance as u32;
        if o.u.last_appearance != -1 && !me.build.has_appearance(pid, last) {
            me.build.save_appearance(pid, last);
            masks |= pp::APPEARANCE;
        } else {
            masks &= !pp::APPEARANCE;
        }
        if o.u.face_entity != -1 && !r.has(pid, pp::FACE_ENTITY) {
            r.cache(pid, face_entity(o.u.face_entity), pp::FACE_ENTITY);
            masks |= pp::FACE_ENTITY;
        }
        if !r.has(pid, pp::FACE_COORD) {
            let m = if o.u.face_x != -1 {
                face_coord(o.u.face_x, o.u.face_z)
            } else if o.u.orientation_x != -1 {
                face_coord(o.u.orientation_x, o.u.orientation_z)
            } else {
                face_coord(fine(o.coord.x(), 1), fine(o.coord.z(), 1))
            };
            r.cache(pid, m, pp::FACE_COORD);
        }
        masks |= pp::FACE_COORD;
        self.write_blocks(r, me, o, masks);
    }

    fn write_blocks(&mut self, r: &PlayerRenderer, me: &Player, o: &Player, masks: u32) {
        let up = &mut self.0.updates;
        if masks > 0xff {
            up.ip2((masks | pp::BIG) as i32);
        } else {
            up.p1(masks as i32);
        }
        for prot in [pp::APPEARANCE, pp::ANIM, pp::FACE_ENTITY, pp::SAY, pp::DAMAGE, pp::FACE_COORD, pp::CHAT, pp::SPOT_ANIM] {
            if masks & prot != 0 {
                r.write(up, o.pid, prot);
            }
        }
        if masks & pp::EXACT_MOVE != 0
            && let Some(e) = &o.u.exact_move
        {
            let x = ((me.origin.x() >> 3) - 6) << 3;
            let z = ((me.origin.z() >> 3) - 6) << 3;
            write_exact_move(up, e.start_x - x, e.start_z - z, e.end_x - x, e.end_z - z, e.begin, e.finish, e.dir);
        }
        if masks & pp::DAMAGE2 != 0 {
            r.write(up, o.pid, pp::DAMAGE2);
        }
    }
}

#[derive(Default)]
pub(crate) struct NpcInfoEncoder(Bufs);

impl NpcInfoEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn encode(&mut self, pos: i32, r: &mut NpcRenderer, npcs: &mut [Option<Npc>], map: &mut ZoneMap, me: &mut Player, dx: i32, dz: i32, rebuild: bool) -> Vec<u8> {
        if rebuild || dx > PREFERRED_VIEW_DISTANCE || dz > PREFERRED_VIEW_DISTANCE {
            me.build.rebuild_npcs();
        }
        self.0.reset();
        let bytes = self.write_npcs(npcs, r, me, pos);
        self.write_new(map, npcs, r, me, bytes);
        self.0.finish(14, 16383)
    }

    fn write_npcs(&mut self, npcs: &mut [Option<Npc>], r: &NpcRenderer, me: &mut Player, start: i32) -> i32 {
        let mut bytes = start;
        self.0.buf.pbit(8, me.build.npcs.len() as i32);
        for nid in me.build.npcs.iter() {
            let gone = npcs[nid as usize].as_ref().is_none_or(|o| {
                o.nid == -1 || o.u.tele || o.coord.y() != me.coord.y() || !me.coord.within_distance_sw(o.coord, PREFERRED_VIEW_DISTANCE) || !o.u.active
            });
            if gone {
                self.0.remove_bits();
                me.build.npcs.remove(nid);
                if let Some(o) = &mut npcs[nid as usize] {
                    o.observers = (o.observers - 1).max(0);
                }
                continue;
            }
            let o = npcs[nid as usize].as_ref().unwrap();
            let length = r.highs(nid);
            if o.u.run_dir != -1 {
                let ext = length > 0 && self.0.fits(bytes + 1, BITS_RUN, length);
                let b = &mut self.0.buf;
                b.pbit(1, 1);
                b.pbit(2, 2);
                b.pbit(3, o.u.walk_dir);
                b.pbit(3, o.u.run_dir);
                self.extend_bit(r, o, ext);
            } else if o.u.walk_dir != -1 {
                let ext = length > 0 && self.0.fits(bytes + 1, BITS_WALK, length);
                let b = &mut self.0.buf;
                b.pbit(1, 1);
                b.pbit(2, 1);
                b.pbit(3, o.u.walk_dir);
                self.extend_bit(r, o, ext);
            } else if length > 0 && self.0.fits(bytes + 1, BITS_EXTEND, length) {
                self.0.buf.pbit(1, 1);
                self.0.buf.pbit(2, 0);
                self.write_blocks(r, o.nid, o.u.masks);
            } else {
                self.0.idle();
            }
            bytes += length + 1;
        }
        bytes
    }

    fn write_new(&mut self, map: &mut ZoneMap, npcs: &mut [Option<Npc>], r: &mut NpcRenderer, me: &mut Player, start: i32) {
        let mut bytes = start;
        let (x, y, z) = (me.coord.x(), me.coord.y(), me.coord.z());
        for nid in me.build.nearby_npcs(npcs, map, x, y, z) {
            if me.build.npcs.contains(nid) {
                continue;
            }
            if me.build.npcs.len() >= PREFERRED_NPCS {
                return;
            }
            let Some(o) = &mut npcs[nid as usize] else { continue };
            let length = r.lows(nid) + r.highs(nid);
            if !self.0.fits(bytes + 1, N_BITS_ADD, length) {
                return;
            }
            let b = &mut self.0.buf;
            b.pbit(14, o.nid);
            b.pbit(11, o.u.ntype);
            b.pbit(5, o.coord.x() - x);
            b.pbit(5, o.coord.z() - z);
            b.pbit(1, o.u.jump as i32);
            b.pbit(1, 1);
            self.low_definition(r, o);
            me.build.npcs.insert(o.nid);
            o.observers += 1;
            bytes += length + 1;
        }
    }

    fn extend_bit(&mut self, r: &NpcRenderer, o: &Npc, extend: bool) {
        if extend {
            self.0.buf.pbit(1, 1);
            self.write_blocks(r, o.nid, o.u.masks);
        } else {
            self.0.buf.pbit(1, 0);
        }
    }

    fn low_definition(&mut self, r: &mut NpcRenderer, o: &Npc) {
        let nid = o.nid;
        let mut masks = o.u.masks;
        if o.u.face_entity != -1 && !r.has(nid, np::FACE_ENTITY) {
            r.cache(nid, face_entity(o.u.face_entity), np::FACE_ENTITY);
            masks |= np::FACE_ENTITY;
        }
        if !r.has(nid, np::FACE_COORD) {
            let m = if o.u.face_x != -1 {
                face_coord(o.u.face_x, o.u.face_z)
            } else if o.u.orientation_x != -1 {
                face_coord(o.u.orientation_x, o.u.orientation_z)
            } else {
                face_coord(fine(o.coord.x(), 1), fine(o.coord.z(), 1))
            };
            r.cache(nid, m, np::FACE_COORD);
        }
        masks |= np::FACE_COORD;
        self.write_blocks(r, nid, masks);
    }

    fn write_blocks(&mut self, r: &NpcRenderer, nid: i32, masks: u32) {
        let up = &mut self.0.updates;
        up.p1((masks & 0xff) as i32);
        for prot in [np::DAMAGE2, np::ANIM, np::FACE_ENTITY, np::SAY, np::DAMAGE, np::CHANGE_TYPE, np::SPOT_ANIM, np::FACE_COORD] {
            if masks & prot != 0 {
                r.write(up, nid, prot);
            }
        }
    }
}
