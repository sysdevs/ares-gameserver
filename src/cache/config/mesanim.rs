//! Mesanim config (server-only).

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MesanimType {
    pub id: usize,
    pub name: Option<String>,
    pub len: [i32; 4],
}

impl ConfigType for MesanimType {
    fn new(id: usize) -> Self {
        MesanimType { id, name: None, len: [-1; 4] }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1..5 => self.len[(code - 1) as usize] = dat.g2(),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized mesanim config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
