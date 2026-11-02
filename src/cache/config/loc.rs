//! Loc config (server + client) with its `postDecode`.

use super::{ConfigType, ParamMap, decode_params};
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct LocType {
    pub id: usize,
    pub debug_name: Option<String>,
    pub models: Option<Vec<u16>>,
    pub shapes: Option<Vec<u8>>,
    pub name: Option<String>,
    pub desc: Option<String>,
    pub recol_s: Option<Vec<u16>>,
    pub recol_d: Option<Vec<u16>>,
    pub width: i32,
    pub length: i32,
    pub blockwalk: bool,
    pub blockrange: bool,
    pub active: i32,
    pub hillskew: bool,
    pub sharelight: bool,
    pub occlude: bool,
    pub anim: i32,
    pub wallwidth: i32,
    pub ambient: i32,
    pub contrast: i32,
    pub op: Option<Vec<Option<String>>>,
    pub mapfunction: i32,
    pub mapscene: i32,
    pub mirror: bool,
    pub shadow: bool,
    pub resizex: i32,
    pub resizey: i32,
    pub resizez: i32,
    pub forceapproach: i32,
    pub offsetx: i32,
    pub offsety: i32,
    pub offsetz: i32,
    pub forcedecor: bool,
    pub breakroutefinding: bool,
    pub raiseobject: i32,
    pub category: i32,
    pub params: ParamMap,
}

impl ConfigType for LocType {
    fn new(id: usize) -> Self {
        LocType {
            id,
            debug_name: None,
            models: None,
            shapes: None,
            name: None,
            desc: None,
            recol_s: None,
            recol_d: None,
            width: 1,
            length: 1,
            blockwalk: true,
            blockrange: true,
            active: -1,
            hillskew: false,
            sharelight: false,
            occlude: false,
            anim: -1,
            wallwidth: 16,
            ambient: 0,
            contrast: 0,
            op: None,
            mapfunction: -1,
            mapscene: -1,
            mirror: false,
            shadow: true,
            resizex: 128,
            resizey: 128,
            resizez: 128,
            forceapproach: 0,
            offsetx: 0,
            offsety: 0,
            offsetz: 0,
            forcedecor: false,
            breakroutefinding: false,
            raiseobject: -1,
            category: -1,
            params: Vec::new(),
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => {
                let n = dat.g1() as usize;
                let (mut models, mut shapes) = (Vec::with_capacity(n), Vec::with_capacity(n));
                for _ in 0..n {
                    models.push(dat.g2() as u16);
                    shapes.push(dat.g1() as u8);
                }
                self.models = Some(models);
                self.shapes = Some(shapes);
            }
            2 => self.name = Some(dat.gjstr(10)),
            3 => self.desc = Some(dat.gjstr(10)),
            5 => {
                let n = dat.g1() as usize;
                self.models = Some((0..n).map(|_| dat.g2() as u16).collect());
                self.shapes = None;
            }
            14 => self.width = dat.g1(),
            15 => self.length = dat.g1(),
            17 => self.blockwalk = false,
            18 => self.blockrange = false,
            19 => self.active = dat.g1(),
            21 => self.hillskew = true,
            22 => self.sharelight = true,
            23 => self.occlude = true,
            24 => {
                self.anim = dat.g2();
                if self.anim == 65535 {
                    self.anim = -1;
                }
            }
            28 => self.wallwidth = dat.g1(),
            29 => self.ambient = dat.g1b(),
            39 => self.contrast = dat.g1b(),
            30..=34 => {
                let s = dat.gjstr(10);
                self.op.get_or_insert_with(|| vec![None; 5])[(code - 30) as usize] = Some(s);
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
            60 => self.mapfunction = dat.g2(),
            61 => self.category = dat.g2(),
            62 => self.mirror = true,
            64 => self.shadow = false,
            65 => self.resizex = dat.g2(),
            66 => self.resizey = dat.g2(),
            67 => self.resizez = dat.g2(),
            68 => self.mapscene = dat.g2(),
            69 => self.forceapproach = dat.g1(),
            70 => self.offsetx = dat.g2s(),
            71 => self.offsety = dat.g2s(),
            72 => self.offsetz = dat.g2s(),
            73 => self.forcedecor = true,
            74 => self.breakroutefinding = true,
            75 => self.raiseobject = dat.g1(),
            249 => self.params = decode_params(dat),
            250 => self.debug_name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized loc config code: {code}")),
        }
        Ok(())
    }

    fn post_decode(&mut self) {
        if self.active == -1 {
            self.active = 0;
            if self.models.is_some() && self.shapes.as_ref().is_none_or(|s| s.first() == Some(&10)) {
                self.active = 1;
            }
            if self.op.is_some() {
                self.active = 1;
            }
        }
    }

    fn debug_name(&self) -> Option<&str> {
        self.debug_name.as_deref()
    }
}
