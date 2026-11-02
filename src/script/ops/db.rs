//! Db ops.

use super::super::runner::OpResult;
use super::super::state::ScriptState;
use ares_shared::opcode::op;
use super::valid;
use crate::cache::config::{ParamValue, dbrow::DbKey, script_var_type as svt};
use crate::engine::world::World;

/// `db_find` / `db_find_with_count`.
fn find(w: &World, s: &mut ScriptState, count: bool, refine: bool) -> OpResult {
    let is_string = s.pop_int() == 2;
    let query = if is_string { DbKey::Str(s.pop_string()) } else { DbKey::Int(s.pop_int()) };
    let packed = s.pop_int();
    let found: Vec<usize> = match w.configs.dbindex.find(&query, packed) {
        Some(rows) => rows.to_vec(),
        None => {
            let table = (packed >> 12) & 0xffff;
            let column = (packed >> 4) & 0x7f;
            if let Some(t) = w.configs.dbtables.get(table as usize) {
                let name = t.column_names.get(column as usize).cloned().unwrap_or_else(|| column.to_string());
                tracing::warn!("dbtable {}:{name} is not INDEXED, finding will fail", t.name.as_deref().unwrap_or("null"));
            }
            Vec::new()
        }
    };
    if refine {
        let prev = std::mem::take(&mut s.db_row_query);
        s.db_row = -1;
        s.db_row_query = prev.into_iter().filter(|r| found.contains(r)).collect();
    } else {
        let table = valid(&w.configs.dbtables, (packed >> 12) & 0xffff, "Dbtable")?;
        s.db_table = Some(table.id as i32);
        s.db_row = -1;
        s.db_row_query = found;
    }
    if count {
        s.push_int(s.db_row_query.len() as i32);
    }
    Ok(())
}

fn push_value(s: &mut ScriptState, kind: i32, v: Option<&ParamValue>) {
    if kind == svt::STRING {
        match v {
            Some(ParamValue::Str(v)) => s.push_string(v.clone()),
            _ => s.push_string(""),
        }
    } else {
        s.push_int(match v {
            Some(ParamValue::Int(v)) => *v,
            _ => 0,
        });
    }
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    Some(match opcode {
        op::DB_FIND | op::DB_FIND_WITH_COUNT => find(w, s, opcode == op::DB_FIND_WITH_COUNT, false),
        op::DB_FIND_REFINE | op::DB_FIND_REFINE_WITH_COUNT => find(w, s, opcode == op::DB_FIND_REFINE_WITH_COUNT, true),
        op::DB_LISTALL | op::DB_LISTALL_WITH_COUNT => (|| {
            let table = s.pop_int();
            let t = valid(&w.configs.dbtables, table, "Dbtable")?;
            s.db_table = Some(t.id as i32);
            s.db_row = -1;
            s.db_row_query = w.configs.dbrows.configs.iter().filter(|r| r.table_id == table).map(|r| r.id).collect();
            if opcode == op::DB_LISTALL_WITH_COUNT {
                s.push_int(s.db_row_query.len() as i32);
            }
            Ok(())
        })(),
        op::DB_FINDNEXT => (|| {
            if s.db_table.is_none() {
                return Err("No table selected".into());
            }
            if s.db_row + 1 >= s.db_row_query.len() as i32 {
                s.push_int(-1);
                return Ok(());
            }
            s.db_row += 1;
            let row = s.db_row_query[s.db_row as usize] as i32;
            s.push_int(valid(&w.configs.dbrows, row, "Dbrow")?.id as i32);
            Ok(())
        })(),
        op::DB_GETFIELD => (|| {
            let [row, packed, list_index] = s.pop_ints::<3>();
            let table = (packed >> 12) & 0xffff;
            let column = ((packed >> 4) & 0x7f) as usize;
            let tuple = (packed & 0xf) - 1;
            let r = valid(&w.configs.dbrows, row, "Dbrow")?;
            let t = valid(&w.configs.dbtables, table, "Dbtable")?;
            let types = t.types.get(column).and_then(Option::as_deref).ok_or_else(|| format!("dbtable has no column {column}"))?;
            let (mut off, mut len) = (0, types.len() as i32);
            if tuple >= 0 {
                if tuple >= len {
                    return Err(format!("Tuple index out-of-bounds. Requested: {tuple}, Max: {len}"));
                }
                off = tuple;
                len = tuple + 1;
            }
            let values = if r.table_id != table { default(w, table as usize, column) } else { row_value(w, r, column, list_index) };
            for i in off..len {
                push_value(s, types[i as usize], values.get(i as usize).and_then(Option::as_ref));
            }
            Ok(())
        })(),
        op::DB_GETFIELDCOUNT => (|| {
            let [row, packed] = s.pop_ints::<2>();
            let table = (packed >> 12) & 0xffff;
            let column = ((packed >> 4) & 0x7f) as usize;
            let r = valid(&w.configs.dbrows, row, "Dbrow")?;
            let t = valid(&w.configs.dbtables, table, "Dbtable")?;
            if r.table_id != table {
                s.push_int(0);
                return Ok(());
            }
            let values = r.column_values.get(column).and_then(Option::as_deref).ok_or_else(|| format!("dbrow has no values for column {column}"))?;
            let types = t.types.get(column).and_then(Option::as_deref).ok_or_else(|| format!("dbtable has no column {column}"))?;
            s.push_int(values.len().checked_div(types.len()).unwrap_or(0) as i32); // x/0 is Infinity or NaN, ToInt32 0
            Ok(())
        })(),
        op::DB_FINDBYINDEX => (|| {
            if s.db_table.is_none() {
                return Err("No table selected".into());
            }
            let index = s.pop_int();
            if index < 0 || index >= s.db_row_query.len() as i32 {
                s.push_int(-1);
                return Ok(());
            }
            let row = s.db_row_query[index as usize] as i32;
            s.push_int(valid(&w.configs.dbrows, row, "Dbrow")?.id as i32);
            Ok(())
        })(),
        op::DB_GETROWTABLE => (|| {
            let row = s.pop_int();
            s.push_int(valid(&w.configs.dbrows, row, "Dbrow")?.table_id);
            Ok(())
        })(),
        _ => return None,
    })
}

/// A column's declared defaults, else each
/// type's default value.
fn default(w: &World, table: usize, column: usize) -> Vec<Option<ParamValue>> {
    let Some(t) = w.configs.dbtables.get(table) else { return Vec::new() };
    if let Some(Some(d)) = t.default_values.get(column) {
        return d.iter().cloned().map(Some).collect();
    }
    t.types.get(column).cloned().flatten().unwrap_or_default().into_iter().map(|ty| Some(svt::get_default(ty))).collect()
}

/// The `list_index`-th tuple of a row column, or the
/// table default when that slice is empty.
fn row_value(w: &World, r: &crate::cache::config::dbrow::DbRowType, column: usize, list_index: i32) -> Vec<Option<ParamValue>> {
    let values = r.column_values.get(column).and_then(Option::as_deref).unwrap_or_default();
    let width = r.types.get(column).and_then(Option::as_ref).map_or(0, |t| t.len()) as i64;
    let (a, b) = (list_index as i64 * width, (list_index as i64 + 1) * width);
    let n = values.len() as i64;
    // `slice(a, b)` with JS negative-index rules
    let clamp = |i: i64| if i < 0 { (n + i).max(0) } else { i.min(n) };
    let (a, b) = (clamp(a), clamp(b));
    let slice: Vec<Option<ParamValue>> = if a < b { values[a as usize..b as usize].iter().cloned().map(Some).collect() } else { Vec::new() };
    if slice.is_empty() { default(w, r.table_id as usize, column) } else { slice }
}
