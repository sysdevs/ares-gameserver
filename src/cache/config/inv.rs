//! Inventory config (server-only).

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

pub const SCOPE_TEMP: i32 = 0;
pub const SCOPE_PERM: i32 = 1;
pub const SCOPE_SHARED: i32 = 2;

#[derive(Clone, Debug, Serialize)]
pub struct InvType {
    pub id: usize,
    pub name: Option<String>,
    pub scope: i32,
    pub size: i32,
    pub stackall: bool,
    pub restock: bool,
    pub allstock: bool,
    pub stockobj: Option<Vec<u16>>,
    pub stockcount: Option<Vec<u16>>,
    pub stockrate: Option<Vec<i32>>,
    pub protect: bool,
    pub runweight: bool,
    pub dummyinv: bool,
}

impl ConfigType for InvType {
    fn new(id: usize) -> Self {
        InvType {
            id,
            name: None,
            scope: 0,
            size: 1,
            stackall: false,
            restock: false,
            allstock: false,
            stockobj: None,
            stockcount: None,
            stockrate: None,
            protect: true,
            runweight: false,
            dummyinv: false,
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.scope = dat.g1(),
            2 => self.size = dat.g2(),
            3 => self.stackall = true,
            4 => {
                let n = dat.g1() as usize;
                let (mut obj, mut count, mut rate) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
                for _ in 0..n {
                    obj.push(dat.g2() as u16);
                    count.push(dat.g2() as u16);
                    rate.push(dat.g4s());
                }
                self.stockobj = Some(obj);
                self.stockcount = Some(count);
                self.stockrate = Some(rate);
            }
            5 => self.restock = true,
            6 => self.allstock = true,
            7 => self.protect = false,
            8 => self.runweight = true,
            9 => self.dummyinv = true,
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized inv config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
