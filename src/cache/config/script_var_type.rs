//! Script var type codes (config dialect, from `ares_shared::var_type`) and their defaults.

pub use ares_shared::var_type::*;

/// `ScriptVarType.getDefault(code)`.
pub fn get_default(code: i32) -> super::ParamValue {
    match code {
        STRING => super::ParamValue::Str(String::new()),
        BOOLEAN => super::ParamValue::Int(0),
        _ => super::ParamValue::Int(-1),
    }
}
