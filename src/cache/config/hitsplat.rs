//! Hitsplat config (server half: names only; the sprite and recolours are client-only).

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct HitsplatType {
    pub id: usize,
    pub name: Option<String>,
}

impl ConfigType for HitsplatType {
    fn new(id: usize) -> Self {
        HitsplatType { id, name: None }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized hitsplat config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
