//! Spotanim config (server + client).

use super::{ConfigType, idk::store};
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct SpotanimType {
    pub id: usize,
    pub name: Option<String>,
    pub model: i32,
    pub anim: i32,
    pub recol_s: [u16; 6],
    pub recol_d: [u16; 6],
    pub resizeh: i32,
    pub resizev: i32,
    pub angle: i32,
    pub ambient: i32,
    pub contrast: i32,
}

impl ConfigType for SpotanimType {
    fn new(id: usize) -> Self {
        SpotanimType { id, name: None, model: 0, anim: -1, recol_s: [0; 6], recol_d: [0; 6], resizeh: 128, resizev: 128, angle: 0, ambient: 0, contrast: 0 }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.model = dat.g2(),
            2 => self.anim = dat.g2(),
            4 => self.resizeh = dat.g2(),
            5 => self.resizev = dat.g2(),
            6 => self.angle = dat.g2(),
            7 => self.ambient = dat.g1(),
            8 => self.contrast = dat.g1(),
            40..50 => store(&mut self.recol_s, code - 40, dat.g2()),
            50..60 => store(&mut self.recol_d, code - 50, dat.g2()),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized spotanim config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
