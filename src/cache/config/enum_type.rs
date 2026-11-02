//! Enum config (server-only). `values` is a `Map`: a
//! repeated key keeps its first position.

use super::{ConfigType, ParamMap, ParamValue, script_var_type as svt};
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct EnumType {
    pub id: usize,
    pub name: Option<String>,
    pub inputtype: i32,
    pub outputtype: i32,
    pub default_int: i32,
    pub default_string: String,
    pub values: ParamMap,
}

fn map_set(map: &mut ParamMap, key: i32, value: ParamValue) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => map.push((key, value)),
    }
}

impl ConfigType for EnumType {
    fn new(id: usize) -> Self {
        EnumType { id, name: None, inputtype: svt::INT, outputtype: svt::INT, default_int: 0, default_string: "null".into(), values: Vec::new() }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.inputtype = dat.g1(),
            2 => self.outputtype = dat.g1(),
            3 => self.default_string = dat.gjstr(10),
            4 => self.default_int = dat.g4s(),
            5 | 6 => {
                for _ in 0..dat.g2() {
                    let k = dat.g4s();
                    let v = if code == 5 { ParamValue::Str(dat.gjstr(10)) } else { ParamValue::Int(dat.g4s()) };
                    map_set(&mut self.values, k, v);
                }
            }
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized enum config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
