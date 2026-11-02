//! The server-only config packers: `ParamConfig`, `Varp/Varbit/Varn/VarsConfig`,
//! `EnumConfig`, `StructConfig`, `DbTableConfig`, `DbRowConfig`, `InvConfig`,
//! `MesAnimConfig`. Each `parse_*` is a config's parse callback and each
//! `pack_*` its pack callback, writing `(client, server)`.

use super::shared::*;
use crate::cache::config::{dbtable, script_var_type as svt};
use crate::pack::pack_file::PackFile;
use crate::pack::parse::int;
use indexmap::IndexMap;

pub type Configs = IndexMap<String, ConfigLines>;

/// The per-config loop every packer shares: `PackedData(pack.max)`, the body
/// for configs that exist, then `250 debug_name` on the server and `next()`.
pub(super) fn each(pack: &PackFile, configs: &Configs, mut body: impl FnMut(&str, &ConfigLines, &mut PackedData, &mut PackedData) -> Result<(), String>) -> Result<(PackedData, PackedData), String> {
    let mut client = PackedData::new(pack.max);
    let mut server = PackedData::new(pack.max);
    for id in 0..pack.max {
        let debug_name = pack.get_by_id(id).to_string();
        if let Some(config) = configs.get(&debug_name) {
            body(&debug_name, config, &mut client, &mut server)?;
        }
        if !debug_name.is_empty() {
            server.p1(250);
            server.pjstr(&debug_name);
        }
        client.next();
        server.next();
    }
    Ok((client, server))
}

pub(super) use self::each as each_config;

fn find<'a>(config: &'a ConfigLines, key: &str) -> Option<&'a Value> {
    config.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

// ---------------------------------------------------------------- param

pub fn parse_param_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "autodisable" => parse_bool(value),
        "type" => type_char(value),
        "default" => Ok(Value::Str(value.to_string())),
        _ => Err(Bad::Key),
    }
}

pub fn pack_param_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.param, configs, |debug_name, config, _client, server| {
        // `config.find(type)!.value`: a missing `type` is impossible (required property).
        let kind = find(config, "type").map_or(0, Value::num) as i32;
        for (key, value) in config {
            match key.as_str() {
                "type" => {
                    server.p1(1);
                    server.p1(value.num());
                }
                "default" => {
                    let v = lookup_param_value(ctx, kind, value.str()).ok_or_else(|| pack_step_error(debug_name, &format!("Invalid default value: {}", value.str())))?;
                    match v {
                        Lookup::Str(s) if kind == svt::STRING => {
                            server.p1(5);
                            server.pjstr(&s);
                        }
                        v => {
                            server.p1(2);
                            server.p4(v.num());
                        }
                    }
                }
                "autodisable" if value.is_false() => server.p1(4),
                _ => {}
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- varp / varbit / varn / vars

pub fn parse_varp_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "clientcode" => parse_number(value),
        "protect" | "transmit" => parse_bool(value),
        "scope" => match value {
            "perm" => Ok(Value::Num(1)),
            "temp" => Ok(Value::Num(0)),
            _ => Err(Bad::Value),
        },
        "type" => type_char(value),
        _ => Err(Bad::Key),
    }
}

const PERSISTABLE: &[&str] = &["int", "coord", "boolean", "obj", "namedobj"];

pub fn pack_varp_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.varp, configs, |debug_name, config, client, server| {
        let mut scope = 0;
        let mut kind = svt::INT as i64;
        for (key, value) in config {
            match key.as_str() {
                "scope" => {
                    scope = value.num();
                    server.p1(1);
                    server.p1(scope);
                }
                "type" => {
                    kind = value.num();
                    server.p1(2);
                    server.p1(kind);
                }
                "protect" if value.is_false() => server.p1(4),
                "clientcode" => {
                    client.p1(5);
                    client.p2(value.num());
                }
                "transmit" if value.is_true() => server.p1(6),
                _ => {}
            }
        }
        let type_name = svt::get_type(kind as i32);
        if scope == 1 && !PERSISTABLE.contains(&type_name) {
            return Err(pack_step_error(debug_name, &format!("scope=perm varps cannot be type={type_name}")));
        }
        Ok(())
    })
}

pub fn parse_varbit_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "startbit" | "endbit" => parse_number(value),
        "basevar" => pack_index(ctx.packs.varp.get_by_name(value)),
        _ => Err(Bad::Key),
    }
}

pub fn pack_varbit_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.varbit, configs, |_, config, client, _server| {
        let (mut basevar, mut startbit, mut endbit) = (None, None, None);
        for (key, value) in config {
            match key.as_str() {
                "basevar" => basevar = Some(value.num()),
                "startbit" => startbit = Some(value.num()),
                "endbit" => endbit = Some(value.num()),
                _ => {}
            }
        }
        if let (Some(b), Some(s), Some(e)) = (basevar, startbit, endbit) {
            client.p1(1);
            client.p2(b);
            client.p1(s);
            client.p1(e);
        }
        Ok(())
    })
}

/// `parseVarnConfig` / `parseVarsConfig`.
pub fn parse_var_type_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "type" => type_char(value),
        _ => Err(Bad::Key),
    }
}

/// `packVarnConfigs` / `packVarsConfigs`.
pub fn pack_var_type_configs(pack: &PackFile, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(pack, configs, |_, config, _client, server| {
        for (key, value) in config {
            if key == "type" {
                server.p1(1);
                server.p1(value.num());
            }
        }
        Ok(())
    })
}

/// `.varo`: as varn, but a string var has no per-item storage (D2).
pub fn pack_varo_configs(pack: &PackFile, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    for (name, config) in configs {
        if find(config, "type").is_some_and(|v| v.num() == svt::STRING as i64) {
            return Err(format!("varo {name}: type=string is not supported"));
        }
    }
    pack_var_type_configs(pack, configs)
}

// ---------------------------------------------------------------- enum

pub fn parse_enum_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "inputtype" | "outputtype" => type_char(value),
        "default" | "val" => Ok(Value::Str(value.to_string())),
        _ => Err(Bad::Key),
    }
}

pub fn pack_enum_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.enums, configs, |debug_name, config, _client, server| {
        let crash = || pack_step_error(debug_name, "enum needs inputtype and outputtype");
        let inputtype = find(config, "inputtype").ok_or_else(crash)?.num() as i32;
        let outputtype = find(config, "outputtype").ok_or_else(crash)?.num() as i32;
        let mut vals: Vec<&str> = Vec::new();
        for (key, value) in config {
            match key.as_str() {
                "val" => vals.push(value.str()),
                "inputtype" => {
                    server.p1(1);
                    server.p1(if value.num() as i32 == svt::AUTOINT { svt::INT as i64 } else { value.num() });
                }
                "outputtype" => {
                    server.p1(2);
                    server.p1(value.num());
                }
                "default" => {
                    let v = lookup_param_value(ctx, outputtype, value.str());
                    if outputtype == svt::STRING {
                        server.p1(3);
                        // `pjstr(null)` is an error.
                        match v {
                            Some(Lookup::Str(s)) => server.pjstr(&s),
                            _ => return Err(pack_step_error(debug_name, "invalid string default")),
                        }
                    } else {
                        server.p1(4);
                        // `p4(null)` writes 0.
                        server.p4(v.map_or(0, |v| v.num()));
                    }
                }
                _ => {}
            }
        }

        server.p1(if outputtype == svt::STRING { 5 } else { 6 });
        server.p2(vals.len() as i64);
        for (i, val) in vals.iter().enumerate() {
            if inputtype == svt::AUTOINT {
                server.p4(i as i64);
            } else {
                let v = val.split_once(',').and_then(|(k, _)| lookup_param_value(ctx, inputtype, k)).ok_or_else(|| pack_step_error(debug_name, &format!("Invalid value-key: {val}")))?;
                server.p4(v.num());
            }
            if outputtype == svt::AUTOINT {
                let v = lookup_param_value(ctx, outputtype, val).ok_or_else(|| pack_step_error(debug_name, &format!("Invalid value: {val}")))?;
                server.p4(v.num());
            } else {
                let v = lookup_param_value(ctx, outputtype, after_comma(val)).ok_or_else(|| pack_step_error(debug_name, &format!("Invalid value: {val}")))?;
                match v {
                    Lookup::Str(s) if outputtype == svt::STRING => server.pjstr(&s),
                    v => server.p4(v.num()),
                }
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- struct

pub fn parse_struct_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "param" => parse_param(ctx, value),
        _ => Err(Bad::Key),
    }
}

pub fn pack_struct_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.structs, configs, |_, config, _client, server| {
        let params: Vec<ParamValue> = config
            .iter()
            .filter_map(|(k, v)| match (k.as_str(), v) {
                ("param", Value::Param(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        if !params.is_empty() {
            write_params(server, &params);
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- dbtable / dbrow

/// `parseCsv`: commas split outside double quotes; quotes are dropped.
fn parse_csv(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        if c == ',' && !in_quotes {
            result.push(std::mem::take(&mut current));
        } else if c == '"' {
            in_quotes = !in_quotes;
        } else {
            current.push(c);
        }
    }
    result.push(current);
    result
}

pub fn parse_dbtable_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "column" | "default" => Ok(Value::Str(value.to_string())),
        _ => Err(Bad::Key),
    }
}

struct Column {
    name: String,
    types: Vec<i32>,
    properties: Vec<String>,
}

pub fn pack_dbtable_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.db_table, configs, |debug_name, config, _client, server| {
        let mut columns: Vec<Column> = Vec::new();
        let mut defaults: Vec<Option<Vec<String>>> = Vec::new();
        for (key, value) in config {
            match key.as_str() {
                "column" => {
                    let mut parts = parse_csv(value.str());
                    let name = parts.remove(0);
                    let (mut types, mut properties) = (Vec::new(), Vec::new());
                    for part in parts {
                        if part.to_uppercase() == part {
                            properties.push(part);
                        } else {
                            let t = svt::get_type_char(&part).ok_or_else(|| pack_step_error(debug_name, &format!("Invalid column type \"{part}\"")))?;
                            types.push(t);
                        }
                    }
                    if properties.iter().any(|p| p == "INDEXED") && !properties.iter().any(|p| p == "REQUIRED") {
                        return Err(pack_step_error(debug_name, "INDEXED columns must be marked REQUIRED as well"));
                    }
                    columns.push(Column { name, types, properties });
                }
                "default" => {
                    let mut parts = parse_csv(value.str());
                    let column = parts.remove(0);
                    let index = columns.iter().position(|c| c.name == column).ok_or_else(|| {
                        pack_step_error(debug_name, &format!("Could not assign default to column \"{column}\" - column does not exist"))
                    })?;
                    if columns[index].properties.iter().any(|p| p == "REQUIRED") {
                        return Err(pack_step_error(debug_name, &format!("Could not assign default to column \"{column}\" - no default value is allowed because the column is marked REQUIRED")));
                    }
                    set_at(&mut defaults, index, parts);
                }
                _ => {}
            }
        }

        if columns.is_empty() {
            return Ok(());
        }
        server.p1(1);
        server.p1(columns.len() as i64);
        for (i, column) in columns.iter().enumerate() {
            let default = defaults.get(i).and_then(Option::as_ref);
            let flags = i as i32 | if default.is_some() { 0x80 } else { 0 };
            server.p1(flags as i64);
            server.p1(column.types.len() as i64);
            for &t in &column.types {
                server.p1(t as i64);
            }
            if let Some(values) = default {
                server.p1(1);
                for (j, &t) in column.types.iter().enumerate() {
                    let raw = values.get(j).ok_or_else(|| pack_step_error(debug_name, "missing default value"))?;
                    let v = lookup_param_value(ctx, t, raw).ok_or_else(|| {
                        pack_step_error(debug_name, &format!("Data invalid in column, double-check the reference exists: default={},{}", column.name, values.join(",")))
                    })?;
                    match v {
                        Lookup::Str(s) if t == svt::STRING => server.pjstr(&s),
                        v => server.p4(v.num()),
                    }
                }
            }
        }
        server.p1(255);

        server.p1(251);
        server.p1(columns.len() as i64);
        for c in &columns {
            server.pjstr(&c.name);
        }

        server.p1(252);
        server.p1(columns.len() as i64);
        for c in &columns {
            let mut props = 0;
            for p in &c.properties {
                props |= match p.as_str() {
                    "INDEXED" => dbtable::INDEXED,
                    "REQUIRED" => dbtable::REQUIRED,
                    "LIST" => dbtable::LIST,
                    "CLIENTSIDE" => dbtable::CLIENTSIDE,
                    _ => 0,
                };
            }
            server.p1(props as i64);
        }
        Ok(())
    })
}

pub fn parse_dbrow_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "table" => pack_index(ctx.packs.db_table.get_by_name(value)),
        "data" => Ok(Value::Str(value.to_string())),
        _ => Err(Bad::Key),
    }
}

pub fn pack_dbrow_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.db_row, configs, |debug_name, config, _client, server| {
        let mut table = None;
        for (key, value) in config {
            if key == "table" {
                table = usize::try_from(value.num()).ok().and_then(|i| ctx.dbtables.get(i));
            }
        }
        let table = table.ok_or_else(|| pack_step_error(debug_name, "No table defined for dbrow"))?;

        let mut data: Vec<(String, Vec<String>)> = Vec::new();
        for (key, value) in config {
            if key == "data" {
                let mut parts = parse_csv(value.str());
                let column = parts.remove(0);
                if !table.column_names.contains(&column) {
                    return Err(pack_step_error(debug_name, &format!("Data invalid in row, double-check the column exists: data={}", value.str())));
                }
                data.push((column, parts));
            }
        }

        if !data.is_empty() {
            server.p1(3);
            server.p1(table.types.len() as i64);
            for i in 0..table.types.len() {
                server.p1(i as i64);
                let types = table.types[i].as_ref().ok_or_else(|| pack_step_error(debug_name, "dbtable column has no types"))?;
                server.p1(types.len() as i64);
                for &t in types {
                    server.p1(t as i64);
                }
                let column_name = table.column_names.get(i).map_or("", String::as_str);
                let fields: Vec<&(String, Vec<String>)> = data.iter().filter(|(c, _)| c == column_name).collect();
                let props = table.props.get(i).copied().unwrap_or(0);
                if props & dbtable::REQUIRED != 0 && fields.is_empty() {
                    return Err(pack_step_error(debug_name, &format!("{column_name} column is marked REQUIRED, please add data for it")));
                }
                if props & dbtable::LIST == 0 && fields.len() > 1 {
                    return Err(pack_step_error(debug_name, &format!("{column_name} column has multiple data values but is not marked as LIST")));
                }
                server.p1(fields.len() as i64);
                for (column, values) in fields {
                    for (k, raw) in values.iter().enumerate() {
                        // `types[k]` may be undefined: no lookup case matches, only `'null'` survives.
                        let t = types.get(k).copied().unwrap_or(i32::MIN);
                        let v = lookup_param_value(ctx, t, raw).ok_or_else(|| {
                            pack_step_error(debug_name, &format!("Data invalid in row, double-check the reference exists: data={column},{}", values.join(",")))
                        })?;
                        match v {
                            Lookup::Str(s) if t == svt::STRING => server.pjstr(&s),
                            v => server.p4(v.num()),
                        }
                    }
                }
            }
            server.p1(255);
        }

        server.p1(4);
        server.p2(table.id as i64);
        Ok(())
    })
}

// ---------------------------------------------------------------- inv

pub fn parse_inv_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "size" => parse_number(value),
        "stackall" | "restock" | "allstock" | "protect" | "runweight" | "dummyinv" => parse_bool(value),
        "scope" => match value {
            "shared" => Ok(Value::Num(2)),
            "perm" => Ok(Value::Num(1)),
            "temp" => Ok(Value::Num(0)),
            _ => Err(Bad::Value),
        },
        _ if key.starts_with("stock") => {
            let parts: Vec<&str> = value.split(',').collect();
            if key_index(key, "stock").is_none() {
                return Err(Bad::Key);
            }
            if !(2..=3).contains(&parts.len()) {
                return Err(Bad::Value);
            }
            let obj = ctx.packs.obj.get_by_name(parts[0]).ok_or(Bad::Value)?;
            let rest = parts[1..].iter().map(|p| int(p).ok_or(Bad::Value));
            Ok(Value::Nums(std::iter::once(Ok(obj)).chain(rest).collect::<Result<_, _>>()?))
        }
        _ => Err(Bad::Key),
    }
}

pub fn pack_inv_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.inv, configs, |debug_name, config, _client, server| {
        let mut stock: Vec<Option<&[i64]>> = Vec::new();
        let size = config.iter().filter(|(k, _)| k == "size").map(|(_, v)| v.num()).last().unwrap_or(0);
        for (key, value) in config {
            match key.as_str() {
                "scope" => {
                    server.p1(1);
                    server.p1(value.num());
                }
                "size" => {
                    server.p1(2);
                    server.p2(value.num());
                }
                _ if key.starts_with("stock") => {
                    let Some(index) = key_index(key, "stock") else { continue };
                    if stock.get(index).is_some_and(Option::is_some) {
                        return Err(pack_step_error(debug_name, &format!("Duplicate stock{} lines, one will overwrite the other.", index + 1)));
                    }
                    if index as i64 >= size {
                        return Err(pack_step_error(debug_name, &format!("stock{} is larger than size={size}", index + 1)));
                    }
                    set_at(&mut stock, index, value.nums());
                }
                "stackall" if value.is_true() => server.p1(3),
                "restock" if value.is_true() => server.p1(5),
                "allstock" if value.is_true() => server.p1(6),
                "protect" if value.is_false() => server.p1(7),
                "runweight" if value.is_true() => server.p1(8),
                "dummyinv" if value.is_true() => server.p1(9),
                _ => {}
            }
        }
        if !stock.is_empty() {
            server.p1(4);
            server.p1(stock.len() as i64);
            for s in &stock {
                match s {
                    None => {
                        server.p2(-1);
                        server.p2(0);
                        server.p4(0);
                    }
                    Some(s) => {
                        server.p2(s[0]);
                        server.p2(s[1]);
                        server.p4(s.get(2).copied().unwrap_or(0));
                    }
                }
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- mesanim

pub fn parse_mesanim_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    if key_index(key, "len").is_some() { pack_index(ctx.packs.seq.get_by_name(value)) } else { Err(Bad::Key) }
}

pub fn pack_mesanim_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each(&ctx.packs.mes_anim, configs, |_, config, _client, server| {
        for (key, value) in config {
            if let Some(i) = key_index(key, "len") {
                server.p1(i as i64 + 1);
                server.p2(value.num());
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varo_rejects_string() {
        let configs: Configs = [("x".to_string(), vec![("type".to_string(), Value::Num(svt::STRING as i64))])].into_iter().collect();
        assert_eq!(pack_varo_configs(&PackFile::new("varo"), &configs).err().as_deref(), Some("varo x: type=string is not supported"));
    }
}
