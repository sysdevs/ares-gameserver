//! The interpreter loop.
//!
//! Handlers return `Err(message)` on failure; the runner turns that into
//! `Aborted` with the message decorated. (The player-facing error lines
//! and the production logout/NPC removal arrive with the entities.)

use super::state::{self, ScriptState};
use crate::engine::world::World;
use ares_shared::opcode::{self, op};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

pub type OpResult = Result<(), String>;

/// Runs until the script leaves `Running`.
pub fn execute(w: &mut World, s: &mut ScriptState) -> state::Execution {
    s.execution = state::Execution::Running;
    s.underflow = None;
    while s.execution == state::Execution::Running {
        if let Err(message) = step(w, s) {
            s.execution = state::Execution::Aborted;
            w.script_error(s, decorate(s, message));
        }
    }
    s.execution
}

fn step(w: &mut World, s: &mut ScriptState) -> OpResult {
    let len = s.script.opcodes.len() as i32;
    if s.pc >= len || s.pc < -1 {
        return Err(format!("Invalid program counter: {}, max expected: {}", s.pc, len));
    }
    if s.opcount > 500_000 {
        return Err("Too many instructions".into());
    }
    s.opcount += 1;
    s.pc += 1;
    let opcode = s.script.opcodes.get(s.pc as usize).copied().unwrap_or(-1);
    let r = dispatch(w, s, opcode);
    match s.underflow.take() {
        Some(e) => Err(e.into()),
        None => r,
    }
}

/// Error decoration: the failing opcode's lower-case name, and a leading `.`
/// for the secondary form of a var op.
fn decorate(s: &ScriptState, message: String) -> String {
    let Some(&opcode) = s.script.opcodes.get(s.pc.max(0) as usize).filter(|_| s.pc >= 0) else { return message };
    let operand = s.int_operand();
    let secondary = if matches!(opcode, op::POP_VARP | op::POP_VARN | op::PUSH_VARP | op::PUSH_VARN | op::PUSH_VARO | op::POP_VARO) {
        (operand >> 16) & 0x1
    } else if opcode <= op::POP_ARRAY_INT {
        0
    } else {
        operand
    };
    let name = opcode::name(opcode).map_or(opcode.to_string(), str::to_lowercase);
    let message = format!("{name} {message}");
    if secondary != 0 { format!(".{message}") } else { message }
}

/// `ScriptRunner.HANDLERS[opcode]`: the op modules in order; the first to
/// return `Some` handles it.
const MODULES: [fn(&mut World, &mut ScriptState, i32) -> Option<OpResult>; 11] = {
    use super::ops::*;
    [core::run, number::run, string::run, config::run, db::run, server::run, player::run, npc::run, loc::run, inv::run, obj::run]
};

/// The `MODULES` index + 1 that owns each opcode, learnt on first dispatch
/// (0 = not seen yet). Ownership is fixed per opcode, so later dispatches skip
/// straight to the owner instead of failing through every module before it.
/// Opcodes past the table just take the slow path.
static OWNER: [AtomicU8; op::TIMESPENT as usize + 1] = [const { AtomicU8::new(0) }; op::TIMESPENT as usize + 1];

fn dispatch(w: &mut World, s: &mut ScriptState, opcode: i32) -> OpResult {
    let owner = usize::try_from(opcode).ok().and_then(|i| OWNER.get(i));
    if let Some(k) = owner.map(|o| o.load(Relaxed)).filter(|&k| k != 0)
        && let Some(r) = MODULES[k as usize - 1](w, s, opcode)
    {
        return r;
    }
    for (k, run) in MODULES.iter().enumerate() {
        if let Some(r) = run(w, s, opcode) {
            if let Some(o) = owner {
                o.store(k as u8 + 1, Relaxed);
            }
            return r;
        }
    }
    Err(format!("Unhandled command {}", opcode::name(opcode).map_or(opcode.to_string(), str::to_string)))
}
