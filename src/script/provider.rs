//! `server/script.{dat,idx}` and the
//! id / name / trigger lookups.
//!
//! The "skip -1" test compares the signed `g4s` key with `0xffffffff`, which
//! never matches, so `-1` keys are inserted too (B.21); the last script with a
//! given key or name wins. When a load fails the caller keeps its old
//! provider on `Err`.

use ares_shared::script::ScriptFile;
use ares_shared::trigger::{Subject, lookup_key};
use crate::io::packet::Packet;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const COMPILER_VERSION: i32 = 27;

#[derive(Default)]
pub struct ScriptProvider {
    scripts: Vec<Option<Arc<ScriptFile>>>,
    lookup: HashMap<i32, usize>,
    names: HashMap<String, usize>,
    /// Scripts loaded (non-empty idx entries).
    pub loaded: usize,
}

impl ScriptProvider {
    /// An empty cache, a compiler version mismatch and an undecodable script
    /// are all errors.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, String> {
        let server = dir.as_ref().join("server");
        let read = |f: &str| Packet::load(server.join(f), false).map_err(|e| format!("server/{f}: {e}"));
        Self::parse(&mut read("script.dat")?, &mut read("script.idx")?)
    }

    pub fn parse(dat: &mut Packet, idx: &mut Packet) -> Result<Self, String> {
        if dat.data.is_empty() || idx.data.is_empty() {
            return Err("No server cache found. Please build the cache first.".into());
        }
        let entries = dat.g4s().max(0) as usize;
        idx.pos += 4;
        if dat.g4s() != COMPILER_VERSION {
            return Err("Scripts were compiled with an incompatible script compiler.".into());
        }
        let mut p = ScriptProvider { scripts: Vec::with_capacity(entries), ..Default::default() };
        for id in 0..entries {
            let size = idx.g4s();
            if size == 0 {
                p.scripts.push(None);
                continue;
            }
            let mut data = vec![0; size as usize];
            dat.gdata(&mut data, 0, size as usize);
            let script = ScriptFile::decode(id, data).map_err(|e| format!("Failed to load script {id}: {e}"))?;
            p.names.insert(script.info.script_name.clone(), id);
            p.lookup.insert(script.info.lookup_key, id);
            p.scripts.push(Some(Arc::new(script)));
            p.loaded += 1;
        }
        Ok(p)
    }

    /// The script for the VM as a shared handle (a suspended script keeps
    /// its file across a reload).
    pub fn get(&self, id: i32) -> Option<Arc<ScriptFile>> {
        self.scripts.get(usize::try_from(id).ok()?)?.clone()
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<ScriptFile>> {
        self.get(*self.names.get(name)? as i32)
    }

    fn lookup(&self, trigger: i32, subject: Subject) -> Option<Arc<ScriptFile>> {
        self.get(*self.lookup.get(&lookup_key(trigger, subject))? as i32)
    }

    /// The type-specific script, else the category one, else the global one.
    /// `-1 << 10` is negative, so an absent type/category produces keys no
    /// compiled script has.
    pub fn get_by_trigger(&self, trigger: i32, kind: i32, category: i32) -> Option<Arc<ScriptFile>> {
        self.lookup(trigger, Subject::Type(kind))
            .or_else(|| self.lookup(trigger, Subject::Category(category)))
            .or_else(|| self.lookup(trigger, Subject::Global))
    }

    /// Only the combination asked for.
    pub fn get_by_trigger_specific(&self, trigger: i32, kind: i32, category: i32) -> Option<Arc<ScriptFile>> {
        let subject = if kind != -1 {
            Subject::Type(kind)
        } else if category != -1 {
            Subject::Category(category)
        } else {
            Subject::Global
        };
        self.lookup(trigger, subject)
    }

    pub fn count(&self) -> usize {
        self.scripts.len()
    }
}
