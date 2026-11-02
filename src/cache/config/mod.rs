//! Opcode-stream config decoders.
//!
//! Each config is an opcode stream ending in `0`; server data comes from
//! `data/pack/server/<type>.dat` (`g2 count` then the streams), client-visible
//! types also continue with the same id's stream from the client `config` jag
//! entry (starting at offset 2). They live in a [`ConfigSet`] owned by the
//! caller.
//!
//! Serialized with camelCase field names.

pub mod category;
pub mod component;
pub mod dbrow;
pub mod dbtable;
pub mod enum_type;
pub mod flo;
pub mod hitsplat;
pub mod hunt;
pub mod idk;
pub mod inv;
pub mod loc;
pub mod mesanim;
pub mod npc;
pub mod obj;
pub mod param;
pub mod script_var_type;
pub mod seq;
pub mod spotanim;
pub mod struct_type;
pub mod var;

use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// A `params` value (`ParamMap = Map<number, number | string>`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    Int(i32),
    Str(String),
}

/// `ParamMap` in insertion order; serializes as `[[key, value], ...]`.
pub type ParamMap = Vec<(i32, ParamValue)>;

/// `ParamHelper.decodeParams`.
pub fn decode_params(dat: &mut Packet) -> ParamMap {
    let count = dat.g1();
    let mut params: ParamMap = Vec::new();
    for _ in 0..count {
        let key = dat.g3();
        let value = if dat.gbool() { ParamValue::Str(dat.gjstr(10)) } else { ParamValue::Int(dat.g4s()) };
        // `Map.set`: an existing key keeps its position.
        match params.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => params.push((key, value)),
        }
    }
    params
}

pub fn get_int_param(params: &ParamMap, id: i32, default: i32) -> i32 {
    match params.iter().find(|(k, _)| *k == id) {
        Some((_, ParamValue::Int(v))) => *v,
        _ => default,
    }
}

/// `ParamHelper.getStringParam`: a missing default becomes the string `"null"`.
pub fn get_string_param(params: &ParamMap, id: i32, default: Option<&str>) -> String {
    match params.iter().find(|(k, _)| *k == id) {
        Some((_, ParamValue::Str(v))) => v.clone(),
        _ => default.unwrap_or("null").to_string(),
    }
}

pub trait ConfigType: Sized {
    fn new(id: usize) -> Self;
    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String>;
    fn post_decode(&mut self) {}
    fn debug_name(&self) -> Option<&str>;
}

/// `ConfigType.decodeType`: opcodes until `0` or the end of the buffer.
pub fn decode_type<T: ConfigType>(config: &mut T, dat: &mut Packet) -> Result<(), String> {
    while dat.available() > 0 {
        let code = dat.g1();
        if code == 0 {
            break;
        }
        config.decode(code, dat)?;
    }
    Ok(())
}

/// The loaded configs of one type plus the debug name → id index.
#[derive(Debug)]
pub struct ConfigSet<T> {
    pub configs: Vec<T>,
    names: HashMap<String, usize>,
}

impl<T> Default for ConfigSet<T> {
    fn default() -> Self {
        ConfigSet { configs: Vec::new(), names: HashMap::new() }
    }
}

impl<T: ConfigType> ConfigSet<T> {
    fn index(configs: Vec<T>) -> Self {
        let mut names = HashMap::new();
        for (id, c) in configs.iter().enumerate() {
            if let Some(n) = c.debug_name().filter(|n| !n.is_empty()) {
                names.insert(n.to_string(), id);
            }
        }
        ConfigSet { configs, names }
    }

    /// `parse(dat)` for server-only types.
    pub fn parse(dat: &mut Packet) -> Result<Self, String> {
        let count = dat.g2() as usize;
        let mut configs = Vec::with_capacity(count);
        for id in 0..count {
            let mut c = T::new(id);
            decode_type(&mut c, dat)?;
            c.post_decode();
            configs.push(c);
        }
        Ok(Self::index(configs))
    }

    /// `parse(server, jag)` for types with a client half in the config jag.
    pub fn parse_with_client(server: &mut Packet, jag: &Jagfile, entry: &str) -> Result<Self, String> {
        Self::parse_with_client_init(server, jag, entry, |_| {})
    }

    /// [`Self::parse_with_client`] with `init` run on each new config before decoding.
    fn parse_with_client_init(server: &mut Packet, jag: &Jagfile, entry: &str, init: impl Fn(&mut T)) -> Result<Self, String> {
        let count = server.g2() as usize;
        let mut client = jag.read(entry).ok_or_else(|| format!("config jag has no {entry}"))?;
        client.pos = 2;
        let mut configs = Vec::with_capacity(count);
        for id in 0..count {
            let mut c = T::new(id);
            init(&mut c);
            decode_type(&mut c, server)?;
            decode_type(&mut c, &mut client)?;
            c.post_decode();
            configs.push(c);
        }
        Ok(Self::index(configs))
    }

    /// `load(dir)` for a server-only type: empty when the file is missing.
    pub fn load(dir: impl AsRef<Path>, file: &str) -> Result<Self, String> {
        let path = dir.as_ref().join("server").join(file);
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut dat = Packet::load(&path, false).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&mut dat)
    }

    /// `load(dir)` for a type with a client half: `server/<file>` + `client/config`.
    pub fn load_with_client(dir: impl AsRef<Path>, file: &str) -> Result<Self, String> {
        Self::load_with_client_init(dir, file, |_| {})
    }

    fn load_with_client_init(dir: impl AsRef<Path>, file: &str, init: impl Fn(&mut T)) -> Result<Self, String> {
        let dir = dir.as_ref();
        let path = dir.join("server").join(file);
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut dat = Packet::load(&path, false).map_err(|e| format!("{}: {e}", path.display()))?;
        let jag = Jagfile::load(dir.join("client/config")).map_err(|e| format!("client/config: {e}"))?;
        Self::parse_with_client_init(&mut dat, &jag, file, init)
    }
}

impl<T> ConfigSet<T> {
    pub fn get(&self, id: usize) -> Option<&T> {
        self.configs.get(id)
    }

    /// `getId(name)`: `-1` when absent.
    pub fn get_id(&self, name: &str) -> i32 {
        self.names.get(name).map_or(-1, |&i| i as i32)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&T> {
        self.names.get(name).map(|&i| &self.configs[i])
    }

    pub fn count(&self) -> usize {
        self.configs.len()
    }
}

/// `printError` for an unknown opcode in the var types: logged, the operand
/// is *not* consumed, decoding continues (unlike the other types, which throw).
pub(crate) fn unknown_logged(kind: &str, code: i32) -> Result<(), String> {
    tracing::error!("Unrecognized {kind} config code: {code}");
    Ok(())
}
