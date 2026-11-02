//! Npc config (server + client) with its `postDecode`.
//! `op` starts as 5 nulls but opcodes 30..=39 grow it like a JS array.

use super::{ConfigType, ParamMap, decode_params};
use crate::io::packet::Packet;
use serde::Serialize;

/// `MoveRestrict.NORMAL`, `NpcMode.WANDER`, `BlockWalk.NPC`.
const MOVERESTRICT_NORMAL: i32 = 0;
const NPCMODE_WANDER: i32 = 1;
const BLOCKWALK_NPC: i32 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct NpcType {
    pub id: usize,
    pub debug_name: Option<String>,
    pub name: Option<String>,
    pub desc: Option<String>,
    pub size: i32,
    pub models: Option<Vec<u16>>,
    pub heads: Option<Vec<u16>>,
    pub readyanim: i32,
    pub walkanim: i32,
    pub walkanim_b: i32,
    pub walkanim_r: i32,
    pub walkanim_l: i32,
    pub recol_s: Option<Vec<u16>>,
    pub recol_d: Option<Vec<u16>>,
    pub op: Option<Vec<Option<String>>>,
    pub resizex: i32,
    pub resizey: i32,
    pub resizez: i32,
    pub minimap: bool,
    pub vislevel: i32,
    pub resizeh: i32,
    pub resizev: i32,
    pub alwaysontop: bool,
    pub ambient: i32,
    pub contrast: i32,
    pub headicon: i32,
    pub turnspeed: i32,
    pub regenrate: i32,
    pub category: i32,
    pub wanderrange: i32,
    pub maxrange: i32,
    pub huntrange: i32,
    pub timer: i32,
    pub respawnrate: i32,
    pub stats: [i32; 6],
    pub moverestrict: i32,
    pub attackrange: i32,
    pub huntmode: i32,
    pub defaultmode: i32,
    pub members: bool,
    pub blockwalk: i32,
    pub params: ParamMap,
    pub patrol_coord: Vec<i32>,
    pub patrol_delay: Vec<i32>,
    pub givechase: bool,
}

impl ConfigType for NpcType {
    fn new(id: usize) -> Self {
        NpcType {
            id,
            debug_name: None,
            name: None,
            desc: None,
            size: 1,
            models: None,
            heads: None,
            readyanim: -1,
            walkanim: -1,
            walkanim_b: -1,
            walkanim_r: -1,
            walkanim_l: -1,
            recol_s: None,
            recol_d: None,
            op: None,
            resizex: -1,
            resizey: -1,
            resizez: -1,
            minimap: true,
            vislevel: -1,
            resizeh: 128,
            resizev: 128,
            alwaysontop: false,
            ambient: 0,
            contrast: 0,
            headicon: -1,
            turnspeed: 32,
            regenrate: 100,
            category: -1,
            wanderrange: 5,
            maxrange: -1,
            huntrange: 0,
            timer: -1,
            respawnrate: 100,
            stats: [1; 6],
            moverestrict: MOVERESTRICT_NORMAL,
            attackrange: 0,
            huntmode: -1,
            defaultmode: NPCMODE_WANDER,
            members: false,
            blockwalk: BLOCKWALK_NPC,
            params: Vec::new(),
            patrol_coord: Vec::new(),
            patrol_delay: Vec::new(),
            givechase: true,
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        let u16s = |dat: &mut Packet| -> Vec<u16> {
            let n = dat.g1() as usize;
            (0..n).map(|_| dat.g2() as u16).collect()
        };
        match code {
            1 => self.models = Some(u16s(dat)),
            2 => self.name = Some(dat.gjstr(10)),
            3 => self.desc = Some(dat.gjstr(10)),
            12 => self.size = dat.g1(),
            13 => self.readyanim = dat.g2(),
            14 => self.walkanim = dat.g2(),
            17 => {
                self.walkanim = dat.g2();
                self.walkanim_b = dat.g2();
                self.walkanim_r = dat.g2();
                self.walkanim_l = dat.g2();
            }
            18 => self.category = dat.g2(),
            26 => self.wanderrange = dat.g2(),
            27 => self.maxrange = dat.g2(),
            30..=39 => {
                let s = dat.gjstr(10);
                let op = self.op.get_or_insert_with(|| vec![None; 5]);
                let i = (code - 30) as usize;
                if op.len() <= i {
                    op.resize(i + 1, None);
                }
                op[i] = Some(s);
            }
            40 => {
                let n = dat.g1() as usize;
                let (mut s, mut d) = (Vec::with_capacity(n), Vec::with_capacity(n));
                for _ in 0..n {
                    s.push(dat.g2() as u16);
                    d.push(dat.g2() as u16);
                }
                self.recol_s = Some(s);
                self.recol_d = Some(d);
            }
            60 => self.heads = Some(u16s(dat)),
            74..=79 => self.stats[(code - 74) as usize] = dat.g2(),
            90 => self.resizex = dat.g2(),
            91 => self.resizey = dat.g2(),
            92 => self.resizez = dat.g2(),
            93 => self.minimap = false,
            95 => self.vislevel = dat.g2(),
            97 => self.resizeh = dat.g2(),
            98 => self.resizev = dat.g2(),
            99 => self.alwaysontop = true,
            100 => self.ambient = dat.g1b(),
            101 => self.contrast = dat.g1b(),
            102 => self.headicon = dat.g2(),
            103 => self.turnspeed = dat.g2(),
            202 => self.huntrange = dat.g1(),
            203 => self.timer = dat.g2(),
            204 => self.respawnrate = dat.g2(),
            206 => self.moverestrict = dat.g1(),
            207 => self.attackrange = dat.g2(),
            208 => self.blockwalk = dat.g1(),
            209 => self.huntmode = dat.g1(),
            210 => self.defaultmode = dat.g1(),
            211 => self.members = true,
            212 => {
                let n = dat.g1() as usize;
                self.patrol_coord = Vec::with_capacity(n);
                self.patrol_delay = Vec::with_capacity(n);
                for _ in 0..n {
                    self.patrol_coord.push(dat.g4s());
                    self.patrol_delay.push(dat.g1());
                }
            }
            213 => self.givechase = false,
            214 => self.regenrate = dat.g2(),
            249 => self.params = decode_params(dat),
            250 => self.debug_name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized npc config code: {code}")),
        }
        Ok(())
    }

    fn post_decode(&mut self) {
        if self.maxrange == -1 {
            self.maxrange = self.wanderrange + 2;
        }
        if self.maxrange < self.wanderrange {
            self.maxrange = self.wanderrange;
        }
    }

    fn debug_name(&self) -> Option<&str> {
        self.debug_name.as_deref()
    }
}
