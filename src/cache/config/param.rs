//! Param config (server-only).

use super::{ConfigType, script_var_type as svt};
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ParamType {
    pub id: usize,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: i32,
    pub default_int: i32,
    pub default_string: Option<String>,
    pub autodisable: bool,
}

impl ConfigType for ParamType {
    fn new(id: usize) -> Self {
        ParamType { id, name: None, kind: svt::INT, default_int: -1, default_string: None, autodisable: true }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.kind = dat.g1(),
            2 => self.default_int = dat.g4s(),
            4 => self.autodisable = false,
            5 => self.default_string = Some(dat.gjstr(10)),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized param config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl ParamType {
    pub fn is_string(&self) -> bool {
        self.kind == svt::STRING
    }
}
