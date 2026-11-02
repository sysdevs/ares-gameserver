//! Db row config (server-only) and the db table index.
//! `types` and `columnValues` are sparse JS arrays (holes serialize as `null`).

use super::dbtable::{DbTableType, INDEXED};
use super::{ConfigSet, ConfigType, ParamValue, script_var_type as svt};
use crate::io::packet::Packet;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
pub struct DbRowType {
    pub id: usize,
    pub name: Option<String>,
    pub table_id: i32,
    pub types: Vec<Option<Vec<i32>>>,
    pub column_values: Vec<Option<Vec<ParamValue>>>,
}

fn set<T: Clone>(v: &mut Vec<Option<T>>, i: usize, x: T) {
    if v.len() <= i {
        v.resize(i + 1, None);
    }
    v[i] = Some(x);
}

impl ConfigType for DbRowType {
    fn new(id: usize) -> Self {
        DbRowType { id, name: None, table_id: 0, types: Vec::new(), column_values: Vec::new() }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            3 => {
                let n = dat.g1() as usize;
                self.types = vec![None; n];
                self.column_values = vec![None; n];
                let mut column = dat.g1();
                while column != 255 {
                    let count = dat.g1();
                    let types: Vec<i32> = (0..count).map(|_| dat.g1()).collect();
                    let fields = dat.g1();
                    let mut values = Vec::with_capacity(fields as usize * types.len());
                    for _ in 0..fields {
                        for &t in &types {
                            values.push(if t == svt::STRING { ParamValue::Str(dat.gjstr(10)) } else { ParamValue::Int(dat.g4s()) });
                        }
                    }
                    set(&mut self.types, column as usize, types);
                    set(&mut self.column_values, column as usize, values);
                    column = dat.g1();
                }
            }
            4 => self.table_id = dat.g2(),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized dbrow config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// A lookup key: `Map` keys compare by value and type, so `1` and `"1"` differ.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DbKey {
    Int(i32),
    Str(String),
}

impl From<&ParamValue> for DbKey {
    fn from(v: &ParamValue) -> Self {
        match v {
            ParamValue::Int(i) => DbKey::Int(*i),
            ParamValue::Str(s) => DbKey::Str(s.clone()),
        }
    }
}

/// `DbTableIndex`: `tableColumnPacked -> value -> row ids` for INDEXED columns.
#[derive(Debug, Default)]
pub struct DbTableIndex {
    pub rows: HashMap<i32, HashMap<DbKey, Vec<usize>>>,
}

impl DbTableIndex {
    /// `DbTableIndex.init()`.
    pub fn new(tables: &ConfigSet<DbTableType>, rows: &ConfigSet<DbRowType>) -> Self {
        let mut index = DbTableIndex::default();
        for table in &tables.configs {
            let indexed = |c: usize| table.props.get(c).is_some_and(|p| p & INDEXED != 0);
            if !(0..table.types.len()).any(indexed) {
                continue;
            }
            for row in rows.configs.iter().filter(|r| r.table_id as usize == table.id) {
                for (column, values) in row.column_values.iter().enumerate() {
                    if !indexed(column) {
                        continue;
                    }
                    // a hole in an INDEXED column is an error
                    let values = values.as_ref().expect("dbrow column hole in an INDEXED column");
                    let width = row.types[column].as_ref().map_or(0, Vec::len);
                    let base = ((table.id as i32 & 0xffff) << 12) | ((column as i32 & 0x7f) << 4);
                    for (i, v) in values.iter().enumerate() {
                        // a tuple indexes each element under its own type id
                        let packed = if width > 1 { base | ((i % width) as i32 & 0xf) } else { base };
                        index.rows.entry(packed).or_default().entry(v.into()).or_default().push(row.id);
                    }
                }
            }
        }
        index
    }

    /// `DbTableIndex.find(query, tableColumnPacked)`: a tuple column looks up
    /// `packed - 1`; `None` (with a warning) when the column is not INDEXED.
    pub fn find(&self, query: &DbKey, table_column_packed: i32) -> Option<&[usize]> {
        let key = if table_column_packed & 0xf == 0 { table_column_packed } else { table_column_packed - 1 };
        self.rows.get(&key).map(|m| m.get(query).map_or(&[][..], Vec::as_slice))
    }
}
