//! The game state, passed around as `&mut World`. So far: configs,
//! collision, the game map and its zones, static locs/objs and spawned NPCs.

mod cheats;
mod client;
mod handlers;
mod movement;
mod npc;
mod player;
mod player_api;
mod player_tick;
mod social;
mod startup;
mod stats;
mod zones;

pub use client::{ClientId, ClientSocket, ClientState, Outgoing};
pub use movement::Mover;
pub use player::{InvOwner, LogoutRequest};

pub use npc::{NpcEvent, ObjDelayed, WorldQueued, mulberry32};
pub use zones::{EntityKey, ZoneEvent};

use super::entity::{Loc, Npc, NpcList, Obj};
use super::game_map::GameMap;
use super::entity::Player;
use super::inventory::Inventory;
use super::linklist::{HashTable, Links, List, NodeId};
use crate::cache::wordenc::WordEnc;
use crate::io::packet::{Packet, RsaKey};
use crate::link::login::LoginRequest;
use crate::routefinder::PathFinder;
use crate::rsbuf::Rsbuf;
use super::zone::{ZoneArenas, ZoneMap};
use indexmap::{IndexMap, IndexSet};
use super::configs::Configs;
use crate::cache::config::script_var_type as svt;
use crate::script::provider::ScriptProvider;
use crate::script::state::ScriptState;
use crate::util::java_random::JavaRandom;
use crate::routefinder::CollisionEngine;
use std::collections::HashMap;

/// `Environment.runtime.maxNpcs`.
pub const MAX_NPCS: usize = 16383;

pub struct World {
    pub configs: Configs,
    pub collision: CollisionEngine,
    pub map: GameMap,
    pub zones: ZoneMap,
    pub arenas: ZoneArenas<Loc, Obj>,
    pub npcs: NpcList,
    /// Each NPC's node in the zone member lists.
    pub npc_nodes: HashMap<i32, NodeId>,
    pub current_tick: i32,
    /// `zonesTracking`: zones touched this tick, in first-touch order.
    pub zones_tracking: IndexSet<i32>,
    /// Each zone's queued events (`Zone.events`, `entityEvents`, `shared`).
    pub zone_events: HashMap<i32, ZoneEvents>,
    next_event: u64,
    /// `locObjTracker` and the `LocObjEvent` nodes on it.
    pub tracker_links: Links<EntityKey>,
    pub loc_obj_tracker: List,
    tracker_garbage: Vec<NodeId>,
    pub scripts: ScriptProvider,
    /// The shared `JavaRandom`.
    pub random: JavaRandom,
    /// `World.vars` / `varsString` (shared vars).
    pub vars: Vec<i32>,
    pub vars_string: Vec<Option<String>>,
    /// Wall clock for `date_*` ops (`Date.now()`); fixed by replays.
    pub clock: Clock,
    started: std::time::Instant,
    /// The last script error message (for tests and diagnostics).
    pub last_script_error: Option<String>,
    /// This cycle's script error messages (cleared when `cycle` starts).
    pub script_errors: Vec<String>,
    /// Despawned NPCs kept for their queued `ai_despawn` script (negative ids).
    pub dead_npcs: HashMap<i32, Npc>,
    next_dead_npc: i32,
    /// `dead_npcs` entries held for the script that ran `npc_del`, dropped at cleanup.
    deleted_npcs: Vec<i32>,
    /// `npcEventQueue`: spawn/despawn triggers.
    pub npc_events: Links<NpcEvent>,
    pub npc_event_queue: List,
    /// `World.queue`: world-suspended scripts.
    pub queue_links: Links<WorldQueued>,
    pub queue: List,
    /// `Math.random()` state (mulberry32).
    math_state: i32,
    /// `Environment.node.production`.
    pub production: bool,
    /// `Midi` track lengths (ms) for `midi_length`.
    pub midi_lengths: Vec<f64>,
    /// `World.players`: slots 1..2046 (0 and 2047 unused).
    pub players: Vec<Option<Box<Player>>>,
    /// `playerLoop`: players in bucket order (address key, then join order).
    pub player_loop: HashTable,
    pub player_loop_links: Links<i32>,
    pub player_loop_nodes: HashMap<i32, NodeId>,
    player_loop_garbage: Vec<NodeId>,
    /// Each player's node in the zone member lists.
    pub player_nodes: HashMap<i32, NodeId>,
    /// `newPlayers`: joining at the end of this tick.
    pub new_players: Vec<Player>,
    pub clients: HashMap<ClientId, ClientSocket>,
    next_client: ClientId,
    /// Everything sent to clients, drained by the transport.
    pub outbox: Vec<(ClientId, Outgoing)>,
    /// `loginRequests`: socket uuid → connection, waiting on the login server.
    pub login_requests: HashMap<String, ClientId>,
    pub logout_requests: IndexMap<String, LogoutRequest>,
    /// Requests for the login and friend threads, drained by the link.
    pub login_out: Vec<LoginRequest>,
    pub friend_out: Vec<crate::link::friend::FriendOut>,
    pub rsbuf: Rsbuf,
    /// `World.invs`: shared inventories, in `Set` order.
    pub invs: Vec<Inventory>,
    /// `CrcBuffer32` and the private key for logins.
    pub crc32: i32,
    pub rsa: Option<RsaKey>,
    pub wordenc: WordEnc,
    out_buf: Packet,
    pub shutdown_tick: i32,
    /// `Environment.engine.revision`, `node.maxConnected`, `node.clientRoutefinder`,
    /// `friend.enabled`, `node.xpRate`.
    pub revision: i32,
    pub max_connected: usize,
    pub client_routefinder: bool,
    pub friend_enabled: bool,
    pub xp_rate: f64,
    /// `VarPlayerType.RUN`.
    pub run_varp: i32,
    pub path_finder: PathFinder,
    /// `Environment.node.debug`.
    pub debug: bool,
    /// `data/pack`, for `reload()` (empty for worlds built in tests).
    pub pack_dir: std::path::PathBuf,
    /// Set by `::rebuild`: the runtime asks the dev watcher for a build.
    pub rebuild_requested: bool,
    /// Set by `reload()`: the runtime reopens the on-demand cache.
    pub reloaded: bool,
    /// `objDelayedQueue`.
    pub obj_delayed_links: Links<ObjDelayed>,
    pub obj_delayed: List,
    /// `World.tickRate` (ms; `::speed` changes it).
    pub tick_rate: f64,
    /// `pmCount`: the next private message id.
    pub pm_count: i32,
    /// `::serverstats` timings (ares only).
    pub stats: stats::CycleStats,
}

/// Where time comes from: the system, or a replay's fixed clock.
#[derive(Clone, Copy, Debug)]
pub enum Clock {
    System,
    Fixed(i64),
}

/// A zone's event queue for the current tick.
#[derive(Default)]
pub struct ZoneEvents {
    /// `Zone.events`: a `Set` in insertion order.
    pub events: IndexMap<u64, ZoneEvent>,
    entity_events: IndexMap<EntityKey, Vec<u64>>,
    /// `Zone.shared`: the encoded enclosed events, once computed.
    pub shared: Option<Vec<u8>>,
}

impl ZoneEvents {
    /// `Zone.entityEvents.get(entity)`: the entity's queued event ids.
    pub fn entity_events_of(&self, key: EntityKey) -> Option<&Vec<u64>> {
        self.entity_events.get(&key)
    }
}

impl World {
    pub fn new(members: bool, configs: Configs) -> Self {
        let mut tracker_links = Links::default();
        let loc_obj_tracker = tracker_links.list();
        let mut npc_events = Links::default();
        let npc_event_queue = npc_events.list();
        let mut queue_links = Links::default();
        let queue = queue_links.list();
        let mut obj_delayed_links = Links::default();
        let obj_delayed = obj_delayed_links.list();
        let mut player_loop_links = Links::default();
        let player_loop = HashTable::new(&mut player_loop_links, 8);
        let mut w = World {
            configs,
            collision: CollisionEngine::default(),
            map: GameMap { members, ..Default::default() },
            zones: ZoneMap::default(),
            arenas: ZoneArenas::default(),
            npcs: NpcList::new(MAX_NPCS),
            npc_nodes: HashMap::new(),
            current_tick: 0,
            zones_tracking: IndexSet::new(),
            zone_events: HashMap::new(),
            next_event: 0,
            tracker_links,
            loc_obj_tracker,
            tracker_garbage: Vec::new(),
            scripts: ScriptProvider::default(),
            random: JavaRandom::new(0),
            vars: Vec::new(),
            vars_string: Vec::new(),
            clock: Clock::System,
            started: std::time::Instant::now(),
            stats: Default::default(),
            last_script_error: None,
            script_errors: Vec::new(),
            dead_npcs: HashMap::new(),
            next_dead_npc: -1,
            deleted_npcs: Vec::new(),
            npc_events,
            npc_event_queue,
            queue_links,
            queue,
            math_state: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as i32),
            production: false,
            midi_lengths: Vec::new(),
            players: (0..2048).map(|_| None).collect(),
            player_loop,
            player_loop_links,
            player_loop_nodes: HashMap::new(),
            player_loop_garbage: Vec::new(),
            player_nodes: HashMap::new(),
            new_players: Vec::new(),
            clients: HashMap::new(),
            next_client: 0,
            outbox: Vec::new(),
            login_requests: HashMap::new(),
            logout_requests: IndexMap::new(),
            login_out: Vec::new(),
            friend_out: Vec::new(),
            rsbuf: Rsbuf::default(),
            invs: Vec::new(),
            crc32: 0,
            rsa: None,
            wordenc: WordEnc::default(),
            out_buf: Packet::alloc(1),
            shutdown_tick: -1,
            revision: 274,
            max_connected: 1000,
            client_routefinder: true,
            friend_enabled: false,
            xp_rate: 1.0,
            run_varp: 0,
            path_finder: PathFinder::default(),
            debug: true,
            pack_dir: std::path::PathBuf::new(),
            rebuild_requested: false,
            reloaded: false,
            obj_delayed_links,
            obj_delayed,
            tick_rate: 600.0,
            pm_count: 1,
        };
        w.run_varp = w.configs.varps.get_id("run").max(0);
        for id in 0..w.configs.invs.count() {
            if w.configs.invs.get(id).is_some_and(|t| t.scope == crate::cache::config::inv::SCOPE_SHARED) {
                w.invs.push(Inventory::from_type(&w.configs.invs, id as i32));
            }
        }
        w.init_vars();
        w
    }

    /// The `vars` reset in `World.reload()`: an int var starts at 0, any
    /// other non-string type at -1 (strings stay unset).
    fn init_vars(&mut self) {
        let n = self.configs.varss.count();
        self.vars = (0..n).map(|i| match self.configs.varss.get(i).map(|t| t.kind) {
            Some(svt::STRING) => 0,
            Some(svt::INT) => 0,
            _ => -1,
        }).collect();
        self.vars_string = vec![None; n];
    }

    /// One `World.cycle()`: every phase in order, then the tick advances
    /// (timing and stats are the caller's). True once a shutdown has completed.
    pub fn cycle(&mut self) -> bool {
        let mut times = [0.0; stats::PHASES];
        let mut mark = std::time::Instant::now();
        let mut lap = |i: usize| {
            let now = std::time::Instant::now();
            times[i] = (now - mark).as_secs_f64() * 1000.0;
            mark = now;
        };
        self.stats.last_in = std::mem::take(&mut self.stats.bytes_in);
        self.script_errors.clear();
        self.process_world();
        lap(0);
        self.process_clients_in();
        lap(1);
        self.process_npc_event_queue();
        lap(2);
        self.process_npcs();
        lap(3);
        self.process_players();
        lap(4);
        self.process_logouts();
        lap(5);
        self.process_logins();
        lap(6);
        self.process_zones();
        lap(7);
        self.process_info();
        lap(8);
        self.process_clients_out();
        self.update_server_stats();
        lap(9);
        self.process_cleanup();
        lap(10);
        let out = self.outbox_bytes();
        self.stats.record(times, out);
        self.log_server_stats();
        let tick = self.current_tick;
        if self.shutdown_tick != -1 && tick >= self.shutdown_tick && self.process_shutdown() {
            return true;
        }
        if tick % 1500 == 0 && tick > 0 {
            for slot in self.player_loop() {
                let p = self.player(slot).expect("player");
                let save = p.save(&self.configs.varps, &self.configs.invs, &self.configs.varos);
                self.login_out.push(LoginRequest::PlayerAutosave { username: p.username.clone(), save });
            }
        }
        self.current_tick += 1;
        false
    }

    /// `processCleanup()`.
    pub fn process_cleanup(&mut self) {
        self.cleanup_zones();
        self.cleanup_players();
        self.cleanup_npcs();
        self.cleanup_world_invs();
        self.rsbuf.cleanup();
    }

    /// `Date.now()`.
    pub fn clock_ms(&self) -> i64 {
        match self.clock {
            Clock::System => crate::server::db::now_ms(),
            Clock::Fixed(ms) => ms,
        }
    }

    /// `performance.now()` (only `timespent` reads it).
    pub fn perf_now_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    /// The script error report of `ScriptRunner.execute`'s catch. The
    /// player-facing lines and the production logout/NPC removal arrive with
    /// the entities; for now it is logged.
    pub fn script_error(&mut self, s: &ScriptState, message: String) {
        tracing::error!("script error: {message} ({}:{:?})", s.script.file_name(), s.script.line_number(s.pc));
        self.script_errors.push(message.clone());
        self.last_script_error = Some(message);
    }


    /// `Zone.addStaticLoc` on the loc's zone.
    pub fn add_static_loc(&mut self, mut loc: Loc) -> NodeId {
        loc.e.is_active = true;
        let (x, z, level) = (loc.e.x, loc.e.z, loc.e.level);
        let id = self.arenas.locs.node(loc);
        let zone = self.zones.zone(x, z, level, &mut self.arenas);
        let list = zone.locs;
        zone.locs_count += 1;
        self.arenas.locs.add_tail(list, id);
        id
    }

    /// `Zone.addStaticObj` on the obj's zone.
    pub fn add_static_obj(&mut self, mut obj: Obj) -> NodeId {
        obj.e.is_active = true;
        let (x, z, level) = (obj.e.x, obj.e.z, obj.e.level);
        let id = self.arenas.objs.node(obj);
        let zone = self.zones.zone(x, z, level, &mut self.arenas);
        let list = zone.objs;
        zone.objs_count += 1;
        self.arenas.objs.add_tail(list, id);
        id
    }
}
