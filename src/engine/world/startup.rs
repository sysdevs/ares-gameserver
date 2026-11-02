//! `World.start()` and `World.reload()`: a World loaded from a server root
//! (`data/pack`, `data/config`, `data/raw`) and the world config.

use super::World;
use crate::cache::crc_table::CrcTable;
use crate::cache::midi;
use crate::cache::wordenc::WordEnc;
use crate::engine::configs::Configs;
use crate::engine::game_map;
use crate::engine::inventory::Inventory;
use crate::io::filestream::FileStream;
use crate::io::packet::RsaKey;
use crate::script::provider::ScriptProvider;
use crate::util::config::WorldConfig;
use crate::util::java_random::JavaRandom;
use std::path::Path;

impl World {
    /// Loads the packed cache, scripts, map, keys and filters under `root`
    /// and applies the node settings of `config`.
    pub fn start(config: &WorldConfig, root: &Path) -> Result<World, String> {
        let pack = root.join("data/pack");
        if !pack.join("server/script.dat").exists() {
            return Err(format!("no packed cache in {} (run ares-pack build first)", pack.display()));
        }
        let node = &config.node;
        let configs = Configs::load(&pack, node.members)?;
        let mut w = World::new(node.members, configs);
        w.pack_dir = pack.clone();
        w.scripts = ScriptProvider::load(&pack)?;
        tracing::debug!("Loaded {} scripts.", w.scripts.loaded);
        let mut cache = FileStream::open(&pack, false).map_err(|e| format!("cache: {e}"))?;
        w.crc32 = CrcTable::new(&mut cache).crc32;
        w.midi_lengths = midi::load_lengths(&mut cache);
        w.wordenc = WordEnc::load(root.join("data/raw/wordenc")).unwrap_or_else(|e| {
            tracing::warn!("{e}");
            WordEnc::default()
        });
        let pem = std::fs::read_to_string(root.join("data/config/private.pem")).map_err(|e| format!("data/config/private.pem: {e} (copy the key pair the web client was built with, or run `ares-pack keygen` for a client built with the new key)"))?;
        w.rsa = Some(RsaKey::from_pem(&pem)?);
        w.production = node.production;
        w.debug = node.debug;
        w.revision = config.engine.revision as i32;
        w.client_routefinder = node.client_routefinder;
        w.max_connected = node.max_connected.max(0) as usize;
        w.xp_rate = node.xp_rate;
        w.friend_enabled = config.friend.enabled;
        let seed = w.clock_ms();
        w.random = JavaRandom::new(seed);
        tracing::debug!("Loading game map");
        game_map::init(&mut w, &root.join(&config.build.src_dir), &pack)?;
        Ok(w)
    }

    /// `reload(clearInvs)`: configs, shared and temp inventories, shared vars,
    /// scripts and the CRC table from `data/pack`. A config that fails to load
    /// keeps the old set (rather than aborting the reload half-done); scripts
    /// that fail keep the old scripts, as `ScriptProvider.load` returning -1.
    /// The on-demand cache is the runtime's to reopen.
    pub fn reload(&mut self, clear_invs: bool) {
        let pack = self.pack_dir.clone();
        let configs = match Configs::load_with_delays(&pack, self.map.members, self.configs.anim_delays.clone()) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("reload: {e}");
                return;
            }
        };
        self.configs = configs;
        self.run_varp = self.configs.varps.get_id("run").max(0);
        if clear_invs {
            self.invs.clear();
            for id in 0..self.configs.invs.count() {
                let Some(scope) = self.configs.invs.get(id).map(|t| t.scope) else { continue };
                if scope == crate::cache::config::inv::SCOPE_SHARED {
                    self.invs.push(Inventory::from_type(&self.configs.invs, id as i32));
                } else if scope == crate::cache::config::inv::SCOPE_TEMP {
                    for slot in self.player_loop() {
                        self.pl(slot).invs.shift_remove(&(id as i32));
                    }
                }
            }
        }
        let n = self.configs.varss.count();
        if self.vars.len() != n {
            // copy the old values, then reset every non-string var
            // (string slots keep the copied int, and `varsString` is copied
            // up to the old *int* length)
            let (old, old_string) = (std::mem::take(&mut self.vars), std::mem::take(&mut self.vars_string));
            self.vars = (0..n).map(|i| old.get(i).copied().unwrap_or(0)).collect();
            self.vars_string = (0..n).map(|i| if i < old.len() { old_string.get(i).cloned().flatten() } else { None }).collect();
            for i in 0..n {
                match self.configs.varss.get(i).map(|t| t.kind) {
                    Some(crate::cache::config::script_var_type::STRING) => {}
                    Some(crate::cache::config::script_var_type::INT) => self.vars[i] = 0,
                    _ => self.vars[i] = -1,
                }
            }
        }
        let count = match ScriptProvider::load(&pack) {
            Ok(s) => {
                let n = s.loaded as i64;
                self.scripts = s;
                n
            }
            Err(e) => {
                tracing::error!("{e}");
                -1
            }
        };
        let msg = if count == -1 { "There was an issue while reloading scripts.".to_string() } else { format!("Loaded {count} scripts.") };
        if self.debug {
            self.broadcast_mes(&msg);
        } else if count == -1 {
            tracing::error!("{msg}");
        } else {
            tracing::debug!("{msg}");
        }
        match FileStream::open(&pack, false) {
            Ok(mut cache) => self.crc32 = CrcTable::new(&mut cache).crc32,
            Err(e) => tracing::error!("cache: {e}"),
        }
        self.reloaded = true;
    }
}
