//! Struct config (server-only).

use super::{ConfigType, ParamMap, decode_params};
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct StructType {
    pub id: usize,
    pub name: Option<String>,
    pub params: Option<ParamMap>,
}

impl ConfigType for StructType {
    fn new(id: usize) -> Self {
        StructType { id, name: None, params: None }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            249 => self.params = Some(decode_params(dat)),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized struct config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
