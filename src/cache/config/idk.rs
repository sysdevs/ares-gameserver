//! Identity kit config (server + client). `heads` and the recolour
//! arrays are fixed-size `Uint16Array`s: writes past the end are dropped.

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct IdkType {
    pub id: usize,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: i32,
    pub models: Option<Vec<u16>>,
    pub heads: [u16; 5],
    pub recol_s: [u16; 6],
    pub recol_d: [u16; 6],
    pub disable: bool,
}

/// A typed-array store: out-of-range indices are ignored.
pub(crate) fn store(arr: &mut [u16], i: i32, v: i32) {
    if let Some(slot) = arr.get_mut(i as usize) {
        *slot = v as u16;
    }
}

impl ConfigType for IdkType {
    fn new(id: usize) -> Self {
        IdkType { id, name: None, kind: -1, models: None, heads: [65535; 5], recol_s: [0; 6], recol_d: [0; 6], disable: false }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.kind = dat.g1(),
            2 => {
                let n = dat.g1();
                self.models = Some((0..n).map(|_| dat.g2() as u16).collect());
            }
            3 => self.disable = true,
            40..50 => store(&mut self.recol_s, code - 40, dat.g2()),
            50..60 => store(&mut self.recol_d, code - 50, dat.g2()),
            60..70 => store(&mut self.heads, code - 60, dat.g2()),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized idk config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
