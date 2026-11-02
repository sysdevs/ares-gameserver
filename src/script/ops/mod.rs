//! The opcode handler families and the input validators. Each family's
//! `run` returns `None` for opcodes it does not handle.

pub mod config;
pub mod core;
pub mod db;
pub mod inv;
pub mod loc;
pub mod npc;
pub mod number;
pub mod obj;
pub mod player;
pub mod server;
pub mod string;

use super::runner::OpResult;
use crate::cache::config::{ConfigSet, ParamMap, ParamValue};

/// Returned by a family's closure for an opcode it does not own.
pub const UNHANDLED: &str = "\u{0}unhandled";

/// A family's result: `None` when the opcode was not its own.
pub fn handled(r: OpResult) -> Option<OpResult> {
    match r {
        Err(e) if e == UNHANDLED => None,
        r => Some(r),
    }
}

/// A valid coord as `(level, x, z)`.
pub fn coord(input: i32) -> Result<(i32, i32, i32), String> {
    in_range(input, 0, i32::MAX, "Coord").map(crate::engine::coord::unpack)
}

pub fn duration(input: i32) -> Result<i32, String> {
    in_range(input, 1, i32::MAX, "Duration")
}

/// The config, or the validation error.
pub fn valid<'a, T>(set: &'a ConfigSet<T>, input: i32, name: &str) -> Result<&'a T, String> {
    if input >= 0 && (input as usize) < set.count() {
        Ok(set.get(input as usize).unwrap())
    } else {
        Err(format!("An input for a {name} type was not valid to use. Input was {input}."))
    }
}

pub fn in_range(input: i32, min: i32, max: i32, name: &str) -> Result<i32, String> {
    if input >= min && input <= max {
        Ok(input)
    } else {
        Err(format!("An input for a {name} was out of range. Range should be: {min} to {max}. Input was {input}."))
    }
}

pub fn not_null(input: i32) -> Result<i32, String> {
    if input != -1 { Ok(input) } else { Err("An input number was null(-1).".into()) }
}

pub fn positive(input: i32) -> Result<i32, String> {
    if input >= 0 { Ok(input) } else { Err("An input number was negative.".into()) }
}

/// A missing or non-string value is the
/// default, and a null default is the string `"null"`.
pub fn string_param(params: Option<&ParamMap>, id: i32, default: Option<&str>) -> String {
    match params.and_then(|p| p.iter().find(|(k, _)| *k == id)) {
        Some((_, ParamValue::Str(s))) => s.clone(),
        _ => default.unwrap_or("null").to_string(),
    }
}

pub fn int_param(params: Option<&ParamMap>, id: i32, default: i32) -> i32 {
    match params.and_then(|p| p.iter().find(|(k, _)| *k == id)) {
        Some((_, ParamValue::Int(v))) => *v,
        _ => default,
    }
}
