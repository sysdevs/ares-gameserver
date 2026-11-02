//! Db table config (server-only). `types` and
//! `defaultValues` are sparse JS arrays (holes serialize as `null`).

use super::{ConfigType, ParamValue, script_var_type as svt};
use crate::io::packet::Packet;
use serde::Serialize;

pub const INDEXED: i32 = 0x1;
pub const REQUIRED: i32 = 0x2;
pub const LIST: i32 = 0x4;
pub const CLIENTSIDE: i32 = 0x8;

#[derive(Clone, Debug, Serialize)]
pub struct DbTableType {
    pub id: usize,
    pub name: Option<String>,
    pub types: Vec<Option<Vec<i32>>>,
    pub default_values: Vec<Option<Vec<ParamValue>>>,
    pub column_names: Vec<String>,
    pub props: Vec<i32>,
}

fn set<T: Clone>(v: &mut Vec<Option<T>>, i: usize, x: T) {
    if v.len() <= i {
        v.resize(i + 1, None);
    }
    v[i] = Some(x);
}

impl ConfigType for DbTableType {
    fn new(id: usize) -> Self {
        DbTableType { id, name: None, types: Vec::new(), default_values: Vec::new(), column_names: Vec::new(), props: Vec::new() }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => {
                self.types = vec![None; dat.g1() as usize];
                let mut setting = dat.g1();
                while setting != 255 {
                    let column = (setting & 0x7f) as usize;
                    let has_default = setting & 0x80 != 0;
                    let n = dat.g1() as usize;
                    let column_types: Vec<i32> = (0..n).map(|_| dat.g1()).collect();
                    set(&mut self.types, column, column_types);
                    if has_default {
                        let values = self.decode_values(dat, column);
                        set(&mut self.default_values, column, values);
                    }
                    setting = dat.g1();
                }
            }
            250 => self.name = Some(dat.gjstr(10)),
            251 => {
                let n = dat.g1() as usize;
                self.column_names = (0..n).map(|_| dat.gjstr(10)).collect();
            }
            252 => {
                let n = dat.g1() as usize;
                self.props = (0..n).map(|_| dat.g1()).collect();
            }
            _ => return Err(format!("Unrecognized dbtable config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl DbTableType {
    fn column_types(&self, column: usize) -> &[i32] {
        self.types.get(column).and_then(Option::as_deref).unwrap_or(&[])
    }

    fn decode_values(&self, dat: &mut Packet, column: usize) -> Vec<ParamValue> {
        let types = self.column_types(column).to_vec();
        let field_count = dat.g1() as usize;
        let mut values = Vec::with_capacity(field_count * types.len());
        for _ in 0..field_count {
            for &t in &types {
                values.push(if t == svt::STRING { ParamValue::Str(dat.gjstr(10)) } else { ParamValue::Int(dat.g4s()) });
            }
        }
        values
    }

    /// `getDefault(column)`.
    pub fn get_default(&self, column: usize) -> Vec<ParamValue> {
        match self.default_values.get(column).and_then(Option::as_ref) {
            Some(v) => v.clone(),
            None => self.column_types(column).iter().map(|&t| svt::get_default(t)).collect(),
        }
    }
}
