//! Floor config (server + client). The debug name is opcode 6.

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct FloType {
    pub id: usize,
    pub name: Option<String>,
    pub rgb: i32,
    pub texture: i32,
    pub overlay: bool,
    pub occlude: bool,
}

impl ConfigType for FloType {
    fn new(id: usize) -> Self {
        FloType { id, name: None, rgb: 0, texture: -1, overlay: false, occlude: true }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.rgb = dat.g3(),
            2 => self.texture = dat.g1(),
            3 => self.overlay = true,
            5 => self.occlude = false,
            6 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized flo config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
