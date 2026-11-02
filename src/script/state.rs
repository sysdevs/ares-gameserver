//! One script execution.

use ares_shared::script::ScriptFile;
use super::PointerType;
use crate::engine::linklist::NodeId;
use std::sync::Arc;

/// `ScriptState` execution states; serialized as their TS numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Aborted = -1,
    Running,
    Finished,
    /// Suspended to move to the player.
    Suspended,
    PauseButton,
    CountDialog,
    /// Suspended to move to the NPC.
    NpcSuspended,
    /// Suspended to move to the world queue.
    WorldSuspended,
}

impl serde::Serialize for Execution {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i32(*self as i32)
    }
}

/// A script argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i32),
    Str(String),
}

/// Any entity a script can point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityRef {
    Player(i32),
    Npc(i32),
    Loc(NodeId),
    Obj(NodeId),
}

/// `active_invobj`: an inventory slot, not the item in it (D8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvObjRef {
    pub pid: i32,
    pub inv: i32,
    pub slot: usize,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub script: Arc<ScriptFile>,
    pub pc: i32,
    pub int_locals: Vec<i32>,
    pub string_locals: Vec<Option<String>>,
}

#[derive(Clone, Debug)]
pub struct ScriptState {
    pub script: Arc<ScriptFile>,
    pub trigger: i32,
    pub execution: Execution,
    pub pc: i32,
    pub opcount: i32,
    pub frames: Vec<Frame>,
    pub int_stack: Vec<i32>,
    pub string_stack: Vec<String>,
    /// Set by a pop from an empty stack; the runner turns it into an error.
    pub underflow: Option<&'static str>,
    pub int_locals: Vec<i32>,
    pub string_locals: Vec<Option<String>>,
    pointers: u32,
    pub self_ref: Option<EntityRef>,
    pub active_player: [Option<i32>; 2],
    pub active_npc: [Option<i32>; 2],
    pub active_loc: [Option<NodeId>; 2],
    pub active_obj: [Option<NodeId>; 2],
    pub active_invobj: [Option<InvObjRef>; 2],
    pub split_pages: Vec<Vec<String>>,
    pub split_mesanim: i32,
    pub db_table: Option<i32>,
    pub db_column: i32,
    pub db_row: i32,
    pub db_row_query: Vec<usize>,
    pub timespent: f64,
    pub last_int: i32,
    pub npc_iterator: Option<super::iterators::ZoneIter>,
    pub player_iterator: Option<super::iterators::ZoneIter>,
    pub loc_iterator: Option<super::iterators::ZoneIter>,
    pub obj_iterator: Option<super::iterators::ZoneIter>,
}

impl ScriptState {
    /// `new ScriptState(script, args)`: ints and strings from `args` become
    /// the first locals (no padding to the local counts).
    pub fn new(script: Arc<ScriptFile>, args: &[Arg]) -> Self {
        let (mut int_locals, mut string_locals) = (Vec::new(), Vec::new());
        for a in args {
            match a {
                Arg::Int(v) => int_locals.push(*v),
                Arg::Str(v) => string_locals.push(Some(v.clone())),
            }
        }
        ScriptState {
            trigger: ares_shared::trigger::trigger_of(script.info.lookup_key),
            script,
            execution: Execution::Running,
            pc: -1,
            opcount: 0,
            frames: Vec::new(),
            int_stack: Vec::new(),
            string_stack: Vec::new(),
            underflow: None,
            int_locals,
            string_locals,
            pointers: 0,
            self_ref: None,
            active_player: [None; 2],
            active_npc: [None; 2],
            active_loc: [None; 2],
            active_obj: [None; 2],
            active_invobj: [None; 2],
            split_pages: Vec::new(),
            split_mesanim: -1,
            db_table: None,
            db_column: -1,
            db_row: -1,
            db_row_query: Vec::new(),
            timespent: 0.0,
            last_int: 0,
            npc_iterator: None,
            player_iterator: None,
            loc_iterator: None,
            obj_iterator: None,
        }
    }

    pub fn pointer_set(&mut self, pointers: &[PointerType]) {
        self.pointers = 0;
        for &p in pointers {
            self.pointers |= 1 << p as u32;
        }
    }

    pub fn pointer_add(&mut self, p: PointerType) {
        self.pointers |= 1 << p as u32;
    }

    pub fn pointer_remove(&mut self, p: PointerType) {
        self.pointers &= !(1 << p as u32);
    }

    pub fn pointer_get(&self, p: PointerType) -> bool {
        self.pointers & (1 << p as u32) != 0
    }

    /// 0 for instructions without one (strings); none of those reads an
    /// active entity, so the value never reaches the `active*` getters.
    pub fn int_operand(&self) -> i32 {
        self.script.int_operands.get(self.pc as usize).copied().unwrap_or(0)
    }

    pub fn string_operand(&self) -> String {
        self.script.string_operands.get(self.pc as usize).cloned().flatten().unwrap_or_default()
    }

    /// 0 on an empty stack, flagging the underflow for the runner.
    pub fn pop_int(&mut self) -> i32 {
        self.int_stack.pop().unwrap_or_else(|| {
            self.underflow = Some("int stack underflow");
            0
        })
    }

    /// `N` ints, in push order.
    pub fn pop_ints<const N: usize>(&mut self) -> [i32; N] {
        let mut out = [0; N];
        for i in (0..N).rev() {
            out[i] = self.pop_int();
        }
        out
    }

    pub fn push_int(&mut self, v: i32) {
        self.int_stack.push(v);
    }

    /// Pushes a double through `ToInt32`.
    pub fn push_f64(&mut self, v: f64) {
        self.push_int(crate::util::numbers::wrap_f64(v));
    }

    /// `''` on an empty stack, flagging the underflow for the runner.
    pub fn pop_string(&mut self) -> String {
        self.string_stack.pop().unwrap_or_else(|| {
            self.underflow = Some("string stack underflow");
            String::new()
        })
    }

    pub fn pop_strings<const N: usize>(&mut self) -> [String; N] {
        let mut out: [String; N] = std::array::from_fn(|_| String::new());
        for i in (0..N).rev() {
            out[i] = self.pop_string();
        }
        out
    }

    pub fn push_string(&mut self, v: impl Into<String>) {
        self.string_stack.push(v.into());
    }

    pub fn pop_frame(&mut self) {
        let f = self.frames.pop().expect("frame");
        self.pc = f.pc;
        self.script = f.script;
        self.int_locals = f.int_locals;
        self.string_locals = f.string_locals;
    }

    pub fn gosub_frame(&mut self, proc: Arc<ScriptFile>) {
        self.frames.push(Frame {
            script: self.script.clone(),
            pc: self.pc,
            int_locals: std::mem::take(&mut self.int_locals),
            string_locals: std::mem::take(&mut self.string_locals),
        });
        self.setup_new_script(proc);
    }

    /// Replaces the call stack.
    pub fn goto_frame(&mut self, label: Arc<ScriptFile>) {
        self.frames.clear();
        self.setup_new_script(label);
    }

    fn setup_new_script(&mut self, script: Arc<ScriptFile>) {
        let mut int_locals = vec![0; script.int_local_count.max(0) as usize];
        let n = script.int_arg_count.max(0) as usize;
        for i in 0..n {
            let v = self.pop_int();
            let slot = n - i - 1;
            if slot >= int_locals.len() {
                int_locals.resize(slot + 1, 0);
            }
            int_locals[slot] = v;
        }
        let mut string_locals = vec![None; script.string_local_count.max(0) as usize];
        let n = script.string_arg_count.max(0) as usize;
        for i in 0..n {
            let v = self.pop_string();
            let slot = n - i - 1;
            if slot >= string_locals.len() {
                string_locals.resize(slot + 1, None);
            }
            string_locals[slot] = Some(v);
        }
        self.pc = -1;
        self.script = script;
        self.int_locals = int_locals;
        self.string_locals = string_locals;
    }

    /// 0 for the primary active entity, 1 for the secondary (`.command`).
    pub(crate) fn which(&self) -> usize {
        (self.int_operand() != 0) as usize
    }

    pub fn active_player(&self) -> Result<i32, String> {
        self.active_player[self.which()].ok_or_else(|| "Attempt to access null active_player".into())
    }

    pub fn active_npc(&self) -> Result<i32, String> {
        self.active_npc[self.which()].ok_or_else(|| "Attempt to access null active_npc".into())
    }

    pub fn active_loc(&self) -> Result<NodeId, String> {
        self.active_loc[self.which()].ok_or_else(|| "Attempt to access null active_loc".into())
    }

    pub fn active_obj(&self) -> Result<NodeId, String> {
        self.active_obj[self.which()].ok_or_else(|| "Attempt to access null active_obj".into())
    }

    pub fn active_invobj(&self) -> Result<InvObjRef, String> {
        self.active_invobj[self.which()].ok_or_else(|| "Attempt to access null active_invobj".into())
    }

    /// `state.activePlayer = player; pointerAdd(ActivePlayer[intOperand])`.
    pub fn set_active_player(&mut self, slot: i32) {
        let i = self.which();
        self.active_player[i] = Some(slot);
        self.pointer_add(if i == 0 { PointerType::ActivePlayer } else { PointerType::ActivePlayer2 });
    }

    /// `state.activeNpc = npc; pointerAdd(ActiveNpc[intOperand])`.
    pub fn set_active_npc(&mut self, nid: i32) {
        let i = self.which();
        self.active_npc[i] = Some(nid);
        self.pointer_add(if i == 0 { PointerType::ActiveNpc } else { PointerType::ActiveNpc2 });
    }

    /// `state.activeLoc = loc; pointerAdd(ActiveLoc[intOperand])`.
    pub fn set_active_loc(&mut self, id: NodeId) {
        let i = self.which();
        self.active_loc[i] = Some(id);
        self.pointer_add(if i == 0 { PointerType::ActiveLoc } else { PointerType::ActiveLoc2 });
    }

    /// `state.activeObj = obj; pointerAdd(ActiveObj[intOperand])`.
    pub fn set_active_obj(&mut self, id: NodeId) {
        let i = self.which();
        self.active_obj[i] = Some(id);
        self.pointer_add(if i == 0 { PointerType::ActiveObj } else { PointerType::ActiveObj2 });
    }

    pub fn set_active_invobj(&mut self, r: InvObjRef) {
        let i = self.which();
        self.active_invobj[i] = Some(r);
        self.pointer_add(if i == 0 { PointerType::ActiveInvObj } else { PointerType::ActiveInvObj2 });
    }

    /// The call depth (`fp`).
    pub fn fp(&self) -> usize {
        self.frames.len()
    }
}
