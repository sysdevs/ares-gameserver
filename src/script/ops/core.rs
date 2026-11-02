//! Core ops (the player and NPC var ops live with those families).

use super::super::runner::OpResult;
use super::super::state::{self, ScriptState};
use ares_shared::opcode::op;
use super::valid;
use crate::cache::config::script_var_type as svt;
use crate::engine::entity::npc::varn_reset;
use crate::engine::inventory::Item;
use crate::engine::world::World;

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let operand = s.int_operand();
    Some(match opcode {
        op::PUSH_CONSTANT_INT => {
            s.push_int(operand);
            Ok(())
        }
        op::PUSH_CONSTANT_STRING => {
            let v = s.string_operand();
            s.push_string(v);
            Ok(())
        }
        op::PUSH_INT_LOCAL => {
            let v = s.int_locals.get(operand as usize).copied().unwrap_or(0);
            s.push_int(v);
            Ok(())
        }
        op::POP_INT_LOCAL => {
            let v = s.pop_int();
            let i = operand as usize;
            if s.int_locals.len() <= i {
                s.int_locals.resize(i + 1, 0);
            }
            s.int_locals[i] = v;
            Ok(())
        }
        op::PUSH_STRING_LOCAL => {
            let v = s.string_locals.get(operand as usize).cloned().flatten().unwrap_or_default();
            s.push_string(v);
            Ok(())
        }
        op::POP_STRING_LOCAL => {
            let v = s.pop_string();
            let i = operand as usize;
            if s.string_locals.len() <= i {
                s.string_locals.resize(i + 1, None);
            }
            s.string_locals[i] = Some(v);
            Ok(())
        }
        op::BRANCH => {
            s.pc += operand;
            Ok(())
        }
        op::BRANCH_NOT | op::BRANCH_EQUALS | op::BRANCH_LESS_THAN | op::BRANCH_GREATER_THAN | op::BRANCH_LESS_THAN_OR_EQUALS | op::BRANCH_GREATER_THAN_OR_EQUALS => {
            let b = s.pop_int();
            let a = s.pop_int();
            let take = match opcode {
                op::BRANCH_NOT => a != b,
                op::BRANCH_EQUALS => a == b,
                op::BRANCH_LESS_THAN => a < b,
                op::BRANCH_GREATER_THAN => a > b,
                op::BRANCH_LESS_THAN_OR_EQUALS => a <= b,
                _ => a >= b,
            };
            if take {
                s.pc += operand;
            }
            Ok(())
        }
        op::POP_INT_DISCARD => {
            s.pop_int();
            Ok(())
        }
        op::POP_STRING_DISCARD => {
            s.pop_string();
            Ok(())
        }
        op::RETURN => {
            if s.fp() == 0 {
                s.execution = state::Execution::Finished;
            } else {
                s.pop_frame();
            }
            Ok(())
        }
        op::JOIN_STRING => {
            let mut parts: Vec<String> = (0..operand).map(|_| s.pop_string()).collect();
            parts.reverse();
            s.push_string(parts.concat());
            Ok(())
        }
        op::GOSUB | op::GOSUB_WITH_PARAMS => {
            if s.fp() >= 50 {
                return Some(Err("stack overflow".into()));
            }
            let id = if opcode == op::GOSUB { s.pop_int() } else { operand };
            match w.scripts.get(id) {
                Some(proc) => {
                    s.gosub_frame(proc);
                    Ok(())
                }
                None => Err(format!("unable to find proc {id}")),
            }
        }
        op::JUMP | op::JUMP_WITH_PARAMS => {
            let id = if opcode == op::JUMP { s.pop_int() } else { operand };
            match w.scripts.get(id) {
                Some(label) => {
                    s.goto_frame(label);
                    Ok(())
                }
                None => Err(format!("unable to find proc {id}")),
            }
        }
        op::DEFINE_ARRAY | op::PUSH_ARRAY_INT | op::POP_ARRAY_INT => Err("unimplemented".into()),
        op::SWITCH => {
            let key = s.pop_int();
            // a missing key or a 0 offset does not jump
            if let Some(&off) = s.script.switch_tables.get(operand as usize).and_then(|t| t.get(&key))
                && off != 0
            {
                s.pc += off;
            }
            Ok(())
        }
        op::PUSH_VARS => (|| {
            let t = valid(&w.configs.varss, operand & 0xffff, "Vars")?;
            if t.kind == svt::STRING {
                let v = w.vars_string.get(t.id).cloned().flatten().unwrap_or_default();
                s.push_string(v);
            } else {
                let v = w.vars.get(t.id).copied().unwrap_or(0);
                s.push_int(v);
            }
            Ok(())
        })(),
        op::POP_VARS => (|| {
            let t = valid(&w.configs.varss, operand & 0xffff, "Vars")?;
            let id = t.id;
            if t.kind == svt::STRING {
                let v = s.pop_string();
                if w.vars_string.len() <= id {
                    w.vars_string.resize(id + 1, None);
                }
                w.vars_string[id] = Some(v);
            } else {
                let v = s.pop_int();
                if w.vars.len() <= id {
                    w.vars.resize(id + 1, 0);
                }
                w.vars[id] = v;
            }
            Ok(())
        })(),
        op::PUSH_VARN | op::POP_VARN => (|| {
            let secondary = (operand >> 16) & 0x1;
            let nid = s.active_npc[secondary as usize].ok_or_else(|| format!("No {} active_npc.", if secondary != 0 { "secondary" } else { "primary" }))?;
            let t = valid(&w.configs.varns, operand & 0xffff, "Varn")?;
            let (id, string) = (t.id, t.kind == svt::STRING);
            let n = w.npc_mut(nid).ok_or("Attempt to access null active_npc")?;
            match (opcode == op::PUSH_VARN, string) {
                (true, true) => s.push_string(n.vars_string.get(id).cloned().flatten().unwrap_or_default()),
                (true, false) => s.push_int(n.vars.get(id).copied().unwrap_or(0)),
                (false, true) => {
                    let v = s.pop_string();
                    if let Some(slot) = n.vars_string.get_mut(id) {
                        *slot = Some(v);
                    }
                }
                (false, false) => {
                    let v = s.pop_int();
                    if let Some(slot) = n.vars.get_mut(id) {
                        *slot = v;
                    }
                }
            }
            Ok(())
        })(),
        op::PUSH_VARO | op::POP_VARO => (|| {
            let secondary = ((operand >> 16) & 0x1) as usize;
            let r = s.active_invobj[secondary].ok_or("Attempt to access null active_invobj")?;
            let t = valid(&w.configs.varos, operand & 0xffff, "Varo")?;
            let (id, unset) = (t.id, varn_reset(t.kind).unwrap_or(-1));
            if opcode == op::POP_VARO {
                // writes follow the inv write ops' protected access; reads, like inv reads, don't
                let it = w.configs.invs.get(r.inv as usize).ok_or("Attempt to access null active_invobj")?;
                super::inv::protect(s, it, secondary, "%varo")?;
            }
            let o = w.player_inventory(r.pid, r.inv).ok_or("Attempt to access null active_invobj")?;
            let (c, configs) = w.inv_with_configs(o).ok_or("Attempt to access null active_invobj")?;
            let item = c.items.get_mut(r.slot).and_then(Option::as_mut).ok_or("active_invobj slot is empty")?;
            if opcode == op::PUSH_VARO {
                s.push_int(item.vars.as_ref().and_then(|v| v.get(id)).copied().unwrap_or(unset));
            } else {
                let v = s.pop_int();
                let vars = item.vars.get_or_insert_with(|| Item::default_vars(&configs.varos));
                if vars.len() <= id {
                    // a live reload added varos since the buffer was made: grow it with the defaults
                    let mut grown = Item::default_vars(&configs.varos).into_vec();
                    grown[..vars.len()].copy_from_slice(vars);
                    *vars = grown.into_boxed_slice();
                }
                vars[id] = v;
            }
            Ok(())
        })(),
        _ => return None,
    })
}
