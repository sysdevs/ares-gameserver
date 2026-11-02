//! Varp, varbit (server + client), varn and vars (server-only) configs. Unknown opcodes are logged and skipped
//! without consuming their operands.

use super::{ConfigType, script_var_type as svt, unknown_logged};
use crate::io::packet::Packet;
use serde::Serialize;

pub const SCOPE_TEMP: i32 = 0;
pub const SCOPE_PERM: i32 = 1;

#[derive(Clone, Debug, Serialize)]
pub struct VarPlayerType {
    pub id: usize,
    pub name: Option<String>,
    pub clientcode: i32,
    pub scope: i32,
    #[serde(rename = "type")]
    pub kind: i32,
    pub protect: bool,
    pub transmit: bool,
}

impl ConfigType for VarPlayerType {
    fn new(id: usize) -> Self {
        VarPlayerType { id, name: None, clientcode: 0, scope: SCOPE_TEMP, kind: svt::INT, protect: true, transmit: false }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.scope = dat.g1(),
            2 => self.kind = dat.g1(),
            4 => self.protect = false,
            5 => self.clientcode = dat.g2(),
            6 => self.transmit = true,
            250 => self.name = Some(dat.gjstr(10)),
            _ => return unknown_logged("varp", code),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// `VarPlayerType.RUN`: the last varp with `clientcode == 7` (0 when none).
pub fn run_varp(varps: &super::ConfigSet<VarPlayerType>) -> usize {
    varps.configs.iter().filter(|v| v.clientcode == 7).map(|v| v.id).last().unwrap_or(0)
}

#[derive(Clone, Debug, Serialize)]
pub struct VarBitType {
    pub id: usize,
    pub name: Option<String>,
    pub basevar: i32,
    pub startbit: i32,
    pub endbit: i32,
}

impl ConfigType for VarBitType {
    fn new(id: usize) -> Self {
        VarBitType { id, name: None, basevar: -1, startbit: -1, endbit: -1 }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => {
                self.basevar = dat.g2();
                self.startbit = dat.g1();
                self.endbit = dat.g1();
            }
            250 => self.name = Some(dat.gjstr(10)),
            _ => return unknown_logged("varbit", code),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// `VarNpcType`, `VarSharedType` and `VarObjType` have the same shape;
/// `KIND` is 0 varn, 1 vars, 2 varo.
#[derive(Clone, Debug, Serialize)]
pub struct VarTyped<const KIND: u8> {
    pub id: usize,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: i32,
}

pub type VarNpcType = VarTyped<0>;
pub type VarSharedType = VarTyped<1>;
pub type VarObjType = VarTyped<2>;

impl<const KIND: u8> ConfigType for VarTyped<KIND> {
    fn new(id: usize) -> Self {
        VarTyped { id, name: None, kind: svt::INT }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.kind = dat.g1(),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return unknown_logged(["varn", "vars", "varo"][KIND as usize], code),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
