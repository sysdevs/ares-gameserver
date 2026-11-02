//! Obj config (server + client) and its load-time post
//! pass: certificates from their template and link, `dummyitem` → untradeable,
//! and the free-to-play rewrite of members objs when the world is not members.

use super::param::ParamType;
use super::{ConfigSet, ConfigType, ParamMap, decode_params};
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct ObjType {
    pub id: usize,
    pub debug_name: Option<String>,
    pub model: i32,
    pub name: Option<String>,
    pub desc: Option<String>,
    pub recol_s: Option<Vec<u16>>,
    pub recol_d: Option<Vec<u16>>,
    pub zoom2d: i32,
    pub xan2d: i32,
    pub yan2d: i32,
    pub zan2d: i32,
    pub xof2d: i32,
    pub yof2d: i32,
    pub code10: i32,
    pub stackable: bool,
    pub cost: i32,
    pub members: bool,
    pub op: Vec<Option<String>>,
    pub iop: Vec<Option<String>>,
    pub manwear: i32,
    pub manwear2: i32,
    pub manwear_offset: i32,
    pub womanwear: i32,
    pub womanwear2: i32,
    pub womanwear_offset: i32,
    pub manwear3: i32,
    pub womanwear3: i32,
    pub manhead: i32,
    pub manhead2: i32,
    pub womanhead: i32,
    pub womanhead2: i32,
    pub countobj: Option<Vec<u16>>,
    pub countco: Option<Vec<u16>>,
    pub certlink: i32,
    pub certtemplate: i32,
    pub resizex: i32,
    pub resizey: i32,
    pub resizez: i32,
    pub ambient: i32,
    pub contrast: i32,
    pub wearpos: i32,
    pub wearpos2: i32,
    pub wearpos3: i32,
    pub weight: i32,
    pub category: i32,
    pub dummyitem: i32,
    pub tradeable: bool,
    pub respawnrate: i32,
    pub params: ParamMap,
}

fn default_op() -> Vec<Option<String>> {
    vec![None, None, Some("Take".into()), None, None]
}

fn default_iop() -> Vec<Option<String>> {
    vec![None, None, None, None, Some("Drop".into())]
}

impl ConfigType for ObjType {
    fn new(id: usize) -> Self {
        ObjType {
            id,
            debug_name: None,
            model: 0,
            name: None,
            desc: None,
            recol_s: None,
            recol_d: None,
            zoom2d: 2000,
            xan2d: 0,
            yan2d: 0,
            zan2d: 0,
            xof2d: 0,
            yof2d: 0,
            code10: -1,
            stackable: false,
            cost: 1,
            members: false,
            op: default_op(),
            iop: default_iop(),
            manwear: -1,
            manwear2: -1,
            manwear_offset: 0,
            womanwear: -1,
            womanwear2: -1,
            womanwear_offset: 0,
            manwear3: -1,
            womanwear3: -1,
            manhead: -1,
            manhead2: -1,
            womanhead: -1,
            womanhead2: -1,
            countobj: None,
            countco: None,
            certlink: -1,
            certtemplate: -1,
            resizex: 128,
            resizey: 128,
            resizez: 128,
            ambient: 0,
            contrast: 0,
            wearpos: -1,
            wearpos2: -1,
            wearpos3: -1,
            weight: 0,
            category: -1,
            dummyitem: 0,
            tradeable: true,
            respawnrate: 100,
            params: Vec::new(),
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.model = dat.g2(),
            2 => self.name = Some(dat.gjstr(10)),
            3 => self.desc = Some(dat.gjstr(10)),
            4 => self.zoom2d = dat.g2(),
            5 => self.xan2d = dat.g2(),
            6 => self.yan2d = dat.g2(),
            7 => self.xof2d = dat.g2s(),
            8 => self.yof2d = dat.g2s(),
            10 => self.code10 = dat.g2(),
            11 => self.stackable = true,
            12 => self.cost = dat.g4s(),
            13 => self.wearpos = dat.g1(),
            14 => self.wearpos2 = dat.g1(),
            15 => self.tradeable = false,
            16 => self.members = true,
            23 => {
                self.manwear = dat.g2();
                self.manwear_offset = dat.g1b();
            }
            24 => self.manwear2 = dat.g2(),
            25 => {
                self.womanwear = dat.g2();
                self.womanwear_offset = dat.g1b();
            }
            26 => self.womanwear2 = dat.g2(),
            27 => self.wearpos3 = dat.g1(),
            30..=34 => self.op[(code - 30) as usize] = Some(dat.gjstr(10)),
            35..=39 => self.iop[(code - 35) as usize] = Some(dat.gjstr(10)),
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
            75 => self.weight = dat.g2s(),
            78 => self.manwear3 = dat.g2(),
            79 => self.womanwear3 = dat.g2(),
            90 => self.manhead = dat.g2(),
            91 => self.womanhead = dat.g2(),
            92 => self.manhead2 = dat.g2(),
            93 => self.womanhead2 = dat.g2(),
            94 => self.category = dat.g2(),
            95 => self.zan2d = dat.g2(),
            96 => self.dummyitem = dat.g1(),
            97 => self.certlink = dat.g2(),
            98 => self.certtemplate = dat.g2(),
            100..=109 => {
                let i = (code - 100) as usize;
                self.countobj.get_or_insert_with(|| vec![0; 10])[i] = dat.g2() as u16;
                self.countco.get_or_insert_with(|| vec![0; 10])[i] = dat.g2() as u16;
            }
            110 => self.resizex = dat.g2(),
            111 => self.resizey = dat.g2(),
            112 => self.resizez = dat.g2(),
            113 => self.ambient = dat.g1b(),
            114 => self.contrast = dat.g1b(),
            201 => self.respawnrate = dat.g2(),
            249 => self.params = decode_params(dat),
            250 => self.debug_name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized obj config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.debug_name.as_deref()
    }
}

/// `ObjType.getWearPosId(name)`.
pub fn get_wear_pos_id(name: &str) -> i32 {
    match name {
        "hat" => 0,
        "back" => 1,
        "front" => 2,
        "righthand" => 3,
        "torso" => 4,
        "lefthand" => 5,
        "arms" => 6,
        "legs" => 7,
        "head" => 8,
        "hands" => 9,
        "feet" => 10,
        "jaw" => 11,
        "ring" => 12,
        "quiver" => 13,
        _ => -1,
    }
}

impl ConfigSet<ObjType> {
    /// `ObjType.load(dir)`: decode, then the post pass in id order.
    /// `members_world` is `Environment.node.members`; `params` must be loaded
    /// first (f2p strips `autodisable` params).
    pub fn load_objs(dir: impl AsRef<Path>, members_world: bool, params: &ConfigSet<ParamType>) -> Result<Self, String> {
        let dir = dir.as_ref();
        let path = dir.join("server/obj.dat");
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut dat = Packet::load(&path, false).map_err(|e| format!("{}: {e}", path.display()))?;
        let jag = Jagfile::load(dir.join("client/config")).map_err(|e| format!("client/config: {e}"))?;
        let mut set = Self::parse_with_client(&mut dat, &jag, "obj.dat")?;
        for id in 0..set.configs.len() {
            if set.configs[id].certtemplate != -1 {
                set.to_certificate(id);
            }
            let c = &mut set.configs[id];
            if c.dummyitem != 0 {
                c.tradeable = false;
            }
            if !members_world && c.members {
                c.tradeable = false;
                c.op = default_op();
                c.iop = default_iop();
                c.category = -1;
                c.params.retain(|(k, _)| !params.get(*k as usize).is_some_and(|p| p.autodisable));
            }
        }
        Ok(set)
    }

    /// `toCertificate()`: reads the template and link as they are at this point
    /// of the id-ordered pass.
    fn to_certificate(&mut self, id: usize) {
        let c = &self.configs[id];
        let template = self.configs[c.certtemplate as usize].clone();
        let link = self.configs[c.certlink as usize].clone();
        let c = &mut self.configs[id];
        c.model = template.model;
        c.zoom2d = template.zoom2d;
        c.xan2d = template.xan2d;
        c.yan2d = template.yan2d;
        c.zan2d = template.zan2d;
        c.xof2d = template.xof2d;
        c.yof2d = template.yof2d;
        c.recol_s = template.recol_s;
        c.recol_d = template.recol_d;
        c.name = link.name.clone();
        c.members = link.members;
        c.cost = link.cost;
        c.tradeable = link.tradeable;
        let first = link.name.as_deref().unwrap_or("").to_lowercase().chars().next();
        let article = if matches!(first, Some('a' | 'e' | 'i' | 'o' | 'u')) { "an" } else { "a" };
        c.desc = Some(format!("Swap this note at any bank for {article} {}.", link.name.as_deref().unwrap_or("null")));
        c.stackable = true;
    }
}
