//! Each entity's update blocks, encoded
//! once per tick and copied into every observer's packet.

use super::{Npc, Packet, Player, npc_prot as np, player_prot as pp};

/// One encoded block (`InfoMessage`): `persists` blocks (appearance) are
/// re-encoded every time; the others only once per tick.
pub(crate) struct Message {
    bytes: Vec<u8>,
    persists: bool,
}

fn encode(len: usize, persists: bool, f: impl FnOnce(&mut Packet)) -> Message {
    let mut p = Packet::new(len);
    f(&mut p);
    Message { bytes: p.data, persists }
}

pub(crate) fn appearance(bytes: &[u8]) -> Message {
    encode(1 + bytes.len(), true, |p| {
        p.p1(bytes.len() as i32);
        p.pdata(bytes);
    })
}

pub(crate) fn face_entity(entity: i32) -> Message {
    encode(2, false, |p| p.p2(entity))
}

pub(crate) fn face_coord(x: i32, z: i32) -> Message {
    encode(4, false, |p| {
        p.p2(x);
        p.p2(z);
    })
}

fn anim(id: i32, delay: i32) -> Message {
    encode(3, false, |p| {
        p.p2(id);
        p.p1(delay);
    })
}

/// `say.length` counts UTF-16 units, one byte each.
fn say(s: &str) -> Message {
    encode(1 + s.encode_utf16().count(), false, |p| p.pjstr(s, 10))
}

fn damage(taken: i32, kind: i32, current: i32, base: i32) -> Message {
    encode(4, false, |p| {
        for v in [taken, kind, current, base] {
            p.p1(v);
        }
    })
}

fn chat(bytes: &[u8], color: i32, effect: i32, ignored: i32) -> Message {
    encode(4 + bytes.len(), false, |p| {
        p.p1(color);
        p.p1(effect);
        p.p1(ignored);
        p.p1(bytes.len() as i32);
        p.pdata(bytes);
    })
}

fn spotanim(id: i32, height: i32, delay: i32) -> Message {
    encode(6, false, |p| {
        p.p2(id);
        p.p4(height.wrapping_shl(16) | delay);
    })
}

fn change_type(ntype: i32) -> Message {
    encode(2, false, |p| p.p2(ntype))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_exact_move(buf: &mut Packet, sx: i32, sz: i32, ex: i32, ez: i32, begin: i32, finish: i32, dir: i32) {
    buf.p1(sx);
    buf.p1(sz);
    buf.p1(ex);
    buf.p1(ez);
    buf.p2(begin);
    buf.p2(finish);
    buf.p1(dir);
}

fn header(masks: u32) -> i32 {
    if masks > 0xff { 2 } else { 1 }
}

/// `playerInfoProtIndex` (BIG and EXACT_MOVE have no cache).
fn player_index(prot: u32) -> usize {
    match prot {
        pp::APPEARANCE => 0,
        pp::ANIM => 1,
        pp::FACE_ENTITY => 2,
        pp::SAY => 3,
        pp::DAMAGE => 4,
        pp::DAMAGE2 => 5,
        pp::FACE_COORD => 6,
        pp::CHAT => 7,
        pp::SPOT_ANIM => 8,
        _ => unreachable!("no cache for player prot {prot:#x}"),
    }
}

fn npc_index(prot: u32) -> usize {
    match prot {
        np::ANIM => 0,
        np::FACE_ENTITY => 1,
        np::SAY => 2,
        np::DAMAGE => 3,
        np::DAMAGE2 => 4,
        np::CHANGE_TYPE => 5,
        np::SPOT_ANIM => 6,
        np::FACE_COORD => 7,
        _ => unreachable!("no cache for npc prot {prot:#x}"),
    }
}

/// The block caches shared by both renderers; `highs`/`lows` are the
/// `Uint16Array`s of precomputed block sizes.
struct Caches {
    caches: Vec<Vec<Option<Vec<u8>>>>,
    highs: Vec<u16>,
    lows: Vec<u16>,
}

impl Caches {
    fn new(kinds: usize, ids: usize) -> Self {
        Caches { caches: (0..kinds).map(|_| vec![None; ids]).collect(), highs: vec![0; ids], lows: vec![0; ids] }
    }

    fn cache(&mut self, kind: usize, id: i32, m: Message) -> i32 {
        let slot = &mut self.caches[kind][id as usize];
        if slot.is_some() && !m.persists {
            return 0;
        }
        let len = m.bytes.len() as i32;
        *slot = Some(m.bytes);
        len
    }

    fn write(&self, buf: &mut Packet, kind: usize, id: i32) {
        let bytes = self.caches[kind][id as usize].as_ref().expect("Tried to write a buf not cached!");
        buf.pdata(bytes);
    }
}

pub(crate) struct PlayerRenderer(Caches);

impl Default for PlayerRenderer {
    fn default() -> Self {
        PlayerRenderer(Caches::new(9, 2048))
    }
}

impl PlayerRenderer {
    pub fn compute_info(&mut self, p: &Player) {
        let (masks, pid, u) = (p.u.masks, p.pid, &p.u);
        if pid == -1 || masks == 0 {
            return;
        }
        let (mut highs, mut lows) = (0, 0);
        if masks & pp::APPEARANCE != 0 {
            let len = self.cache(pid, appearance(&u.appearance), pp::APPEARANCE);
            highs += len;
            lows += len;
        }
        if masks & pp::ANIM != 0 {
            highs += self.cache(pid, anim(u.anim_id, u.anim_delay), pp::ANIM);
        }
        if masks & pp::FACE_ENTITY != 0 {
            let len = self.cache(pid, face_entity(u.face_entity), pp::FACE_ENTITY);
            highs += len;
            lows += len;
        }
        if masks & pp::SAY != 0
            && let Some(s) = &u.say
        {
            highs += self.cache(pid, say(s), pp::SAY);
        }
        if masks & pp::DAMAGE != 0 {
            highs += self.cache(pid, damage(u.damage_taken, u.damage_type, u.current_hitpoints, u.base_hitpoints), pp::DAMAGE);
        }
        if masks & pp::DAMAGE2 != 0 {
            highs += self.cache(pid, damage(u.damage_taken2, u.damage_type2, u.current_hitpoints, u.base_hitpoints), pp::DAMAGE2);
        }
        if masks & pp::FACE_COORD != 0 {
            let len = self.cache(pid, face_coord(u.face_x, u.face_z), pp::FACE_COORD);
            highs += len;
            lows += len;
        }
        if masks & pp::CHAT != 0
            && let Some(c) = &u.chat
        {
            highs += self.cache(pid, chat(&c.bytes, c.color, c.effect, c.ignored), pp::CHAT);
        }
        if masks & pp::SPOT_ANIM != 0 {
            highs += self.cache(pid, spotanim(u.graphic_id, u.graphic_height, u.graphic_delay), pp::SPOT_ANIM);
        }
        if masks & pp::EXACT_MOVE != 0 {
            highs += 9;
        }
        if highs > 0 {
            self.0.highs[pid as usize] = (highs + header(masks)) as u16;
        }
        if lows > 0 {
            let appearance = self.0.caches[0][pid as usize].as_ref().map_or(0, Vec::len) as i32;
            self.0.lows[pid as usize] = (header(pp::APPEARANCE + pp::FACE_ENTITY + pp::FACE_COORD) + appearance + 2 + 4) as u16;
        }
    }

    pub fn cache(&mut self, id: i32, m: Message, prot: u32) -> i32 {
        self.0.cache(player_index(prot), id, m)
    }

    pub fn write(&self, buf: &mut Packet, id: i32, prot: u32) {
        self.0.write(buf, player_index(prot), id);
    }

    pub fn has(&self, id: i32, prot: u32) -> bool {
        self.0.caches[player_index(prot)][id as usize].is_some()
    }

    pub fn highs(&self, id: i32) -> i32 {
        self.0.highs[id as usize] as i32
    }

    pub fn lows(&self, id: i32) -> i32 {
        self.0.lows[id as usize] as i32
    }

    pub fn remove_temporary(&mut self) {
        self.0.highs.fill(0);
        for kind in 1..9 {
            self.0.caches[kind].fill(None);
        }
    }

    pub fn remove_permanent(&mut self, id: i32) {
        self.0.highs[id as usize] = 0;
        self.0.lows[id as usize] = 0;
        self.0.caches[0][id as usize] = None;
    }
}

pub(crate) struct NpcRenderer(Caches);

impl Default for NpcRenderer {
    fn default() -> Self {
        NpcRenderer(Caches::new(8, 16384))
    }
}

impl NpcRenderer {
    pub fn compute_info(&mut self, n: &Npc) {
        let (masks, nid, u) = (n.u.masks, n.nid, &n.u);
        if nid == -1 || masks == 0 {
            return;
        }
        let (mut highs, mut lows) = (0, 0);
        if masks & np::ANIM != 0 {
            highs += self.cache(nid, anim(u.anim_id, u.anim_delay), np::ANIM);
        }
        if masks & np::FACE_ENTITY != 0 {
            let len = self.cache(nid, face_entity(u.face_entity), np::FACE_ENTITY);
            highs += len;
            lows += len;
        }
        if masks & np::SAY != 0
            && let Some(s) = &u.say
        {
            highs += self.cache(nid, say(s), np::SAY);
        }
        if masks & np::DAMAGE != 0 {
            highs += self.cache(nid, damage(u.damage_taken, u.damage_type, u.current_hitpoints, u.base_hitpoints), np::DAMAGE);
        }
        if masks & np::DAMAGE2 != 0 {
            highs += self.cache(nid, damage(u.damage_taken2, u.damage_type2, u.current_hitpoints, u.base_hitpoints), np::DAMAGE2);
        }
        if masks & np::CHANGE_TYPE != 0 {
            highs += self.cache(nid, change_type(u.ntype), np::CHANGE_TYPE);
        }
        if masks & np::SPOT_ANIM != 0 {
            highs += self.cache(nid, spotanim(u.graphic_id, u.graphic_height, u.graphic_delay), np::SPOT_ANIM);
        }
        if masks & np::FACE_COORD != 0 {
            let len = self.cache(nid, face_coord(u.face_x, u.face_z), np::FACE_COORD);
            highs += len;
            lows += len;
        }
        if highs > 0 {
            self.0.highs[nid as usize] = (highs + header(masks)) as u16;
        }
        if lows > 0 {
            self.0.lows[nid as usize] = (header(np::FACE_ENTITY + np::FACE_COORD) + 2 + 4) as u16;
        }
    }

    pub fn cache(&mut self, id: i32, m: Message, prot: u32) -> i32 {
        self.0.cache(npc_index(prot), id, m)
    }

    pub fn write(&self, buf: &mut Packet, id: i32, prot: u32) {
        self.0.write(buf, npc_index(prot), id);
    }

    pub fn has(&self, id: i32, prot: u32) -> bool {
        self.0.caches[npc_index(prot)][id as usize].is_some()
    }

    pub fn highs(&self, id: i32) -> i32 {
        self.0.highs[id as usize] as i32
    }

    pub fn lows(&self, id: i32) -> i32 {
        self.0.lows[id as usize] as i32
    }

    pub fn remove_temporary(&mut self) {
        self.0.highs.fill(0);
        for kind in 0..8 {
            self.0.caches[kind].fill(None);
        }
    }

    pub fn remove_permanent(&mut self, id: i32) {
        self.0.highs[id as usize] = 0;
        self.0.lows[id as usize] = 0;
    }
}
