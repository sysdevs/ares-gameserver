//! Enum, struct, loc/npc/obj config and debug ops.

use super::super::runner::OpResult;
use super::super::state::ScriptState;
use ares_shared::opcode::op;
use super::{int_param, not_null, string_param, valid};
use crate::cache::config::ParamValue;
use crate::engine::world::World;

/// Marks an opcode this family does not handle (never a real error message).
use super::UNHANDLED;

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let c = &w.configs;
    let r: OpResult = (|| {
        match opcode {
            op::ENUM => {
                let [input, output, id, key] = s.pop_ints::<4>();
                let e = valid(&c.enums, id, "Enum")?;
                if e.inputtype != input || e.outputtype != output {
                    return Err(format!(
                        "Type validation error: {} key: {key}. Expected input: {input} got: {}. Expected output: {output} got: {}",
                        e.name.as_deref().unwrap_or("null"),
                        e.inputtype,
                        e.outputtype
                    ));
                }
                // a missing key pushes the *int* default even for string
                // enums (a bug kept on purpose)
                match e.values.iter().find(|(k, _)| *k == key) {
                    Some((_, ParamValue::Str(v))) => s.push_string(v.clone()),
                    Some((_, ParamValue::Int(v))) => s.push_int(*v),
                    None => s.push_int(e.default_int),
                }
            }
            op::ENUM_GETOUTPUTCOUNT => {
                let id = s.pop_int();
                s.push_int(valid(&c.enums, id, "Enum")?.values.len() as i32);
            }
            op::STRUCT_PARAM => {
                let [struct_id, param_id] = s.pop_ints::<2>();
                let p = valid(&c.params, param_id, "Param")?;
                let st = valid(&c.structs, struct_id, "Struct")?;
                param(s, st.params.as_ref(), p);
            }
            op::LC_NAME | op::LC_DESC | op::LC_DEBUGNAME | op::LC_CATEGORY | op::LC_WIDTH | op::LC_LENGTH => {
                let id = s.pop_int();
                let t = valid(&c.locs, id, "Loc")?;
                match opcode {
                    op::LC_NAME => s.push_string(t.name.as_ref().or(t.debug_name.as_ref()).map_or("null".into(), String::clone)),
                    op::LC_DESC => s.push_string(t.desc.clone().unwrap_or("null".into())),
                    op::LC_DEBUGNAME => s.push_string(t.debug_name.clone().unwrap_or("null".into())),
                    op::LC_CATEGORY => s.push_int(t.category),
                    op::LC_WIDTH => s.push_int(t.width),
                    _ => s.push_int(t.length),
                }
            }
            op::LC_PARAM => {
                let [id, param_id] = s.pop_ints::<2>();
                let t = valid(&c.locs, id, "Loc")?;
                let p = valid(&c.params, param_id, "Param")?;
                param(s, Some(&t.params), p);
            }
            op::NC_NAME | op::NC_DESC | op::NC_DEBUGNAME | op::NC_CATEGORY | op::NC_SIZE | op::NC_VISLEVEL => {
                let id = s.pop_int();
                let t = valid(&c.npcs, id, "Npc")?;
                match opcode {
                    op::NC_NAME => s.push_string(t.name.as_ref().or(t.debug_name.as_ref()).map_or("null".into(), String::clone)),
                    op::NC_DESC => s.push_string(t.desc.clone().unwrap_or("null".into())),
                    op::NC_DEBUGNAME => s.push_string(t.debug_name.clone().unwrap_or("null".into())),
                    op::NC_CATEGORY => s.push_int(t.category),
                    op::NC_SIZE => s.push_int(t.size),
                    _ => s.push_int(t.vislevel),
                }
            }
            op::NC_PARAM => {
                let [id, param_id] = s.pop_ints::<2>();
                let t = valid(&c.npcs, id, "Npc")?;
                let p = valid(&c.params, param_id, "Param")?;
                param(s, Some(&t.params), p);
            }
            op::NC_OP => {
                let [id, n] = s.pop_ints::<2>();
                let t = valid(&c.npcs, id, "Npc")?;
                not_null(n)?;
                let v = t.op.as_ref().and_then(|ops| ops.get((n - 1) as usize).cloned().flatten()).unwrap_or_default();
                s.push_string(v);
            }
            op::OC_NAME | op::OC_DESC | op::OC_DEBUGNAME => {
                let id = s.pop_int();
                let t = valid(&c.objs, id, "Obj")?;
                s.push_string(match opcode {
                    op::OC_NAME => t.name.as_ref().or(t.debug_name.as_ref()).map_or("null".into(), String::clone),
                    op::OC_DESC => t.desc.clone().unwrap_or("null".into()),
                    _ => t.debug_name.clone().unwrap_or("null".into()),
                });
            }
            op::OC_PARAM => {
                let [id, param_id] = s.pop_ints::<2>();
                let t = valid(&c.objs, id, "Obj")?;
                let p = valid(&c.params, param_id, "Param")?;
                param(s, Some(&t.params), p);
            }
            op::OC_CATEGORY | op::OC_MEMBERS | op::OC_WEIGHT | op::OC_WEARPOS | op::OC_WEARPOS2 | op::OC_WEARPOS3 | op::OC_COST | op::OC_TRADEABLE | op::OC_CERT | op::OC_UNCERT | op::OC_STACKABLE => {
                let id = s.pop_int();
                let t = valid(&c.objs, id, "Obj")?;
                s.push_int(match opcode {
                    op::OC_CATEGORY => t.category,
                    op::OC_MEMBERS => t.members as i32,
                    op::OC_WEIGHT => t.weight,
                    op::OC_WEARPOS => t.wearpos,
                    op::OC_WEARPOS2 => t.wearpos2,
                    op::OC_WEARPOS3 => t.wearpos3,
                    op::OC_COST => t.cost,
                    op::OC_TRADEABLE => t.tradeable as i32,
                    op::OC_CERT if t.certtemplate == -1 && t.certlink >= 0 => t.certlink,
                    op::OC_UNCERT if t.certtemplate >= 0 && t.certlink >= 0 => t.certlink,
                    op::OC_CERT | op::OC_UNCERT => t.id as i32,
                    _ => t.stackable as i32,
                });
            }
            op::ERROR => return Err(s.pop_string()),
            op::CONSOLE => {
                let v = s.pop_string();
                tracing::info!("{v}");
            }
            op::TIMESPENT => s.timespent = w.perf_now_ms(),
            op::GETTIMESPENT => {
                let elapsed = w.perf_now_ms() - s.timespent;
                let micro = s.pop_int() == 1;
                s.push_f64(if micro { elapsed * 1000.0 } else { elapsed });
            }
            _ => return Err(UNHANDLED.into()),
        }
        Ok(())
    })();
    match r {
        Err(e) if e == UNHANDLED => None,
        r => Some(r),
    }
}

/// The `*_PARAM` ops: string or int by the param's type.
pub fn param(s: &mut ScriptState, params: Option<&crate::cache::config::ParamMap>, p: &crate::cache::config::param::ParamType) {
    if p.is_string() {
        s.push_string(string_param(params, p.id as i32, p.default_string.as_deref()));
    } else {
        s.push_int(int_param(params, p.id as i32, p.default_int));
    }
}
