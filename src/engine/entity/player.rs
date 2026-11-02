//! Player state and the save codec. Behaviour that touches the World is in
//! `world/player.rs`.

use super::EntityBase;
use super::lifecycle;
use super::npc::HeroPoints;
use super::pathing::{Pathing, move_strategy};
use crate::cache::config::inv::{self as inv_cfg, InvType};
use crate::cache::config::var::{self as var_cfg, VarObjType, VarPlayerType};
use crate::cache::config::{ConfigSet, script_var_type as svt};
use crate::engine::inventory::{Inventory, Item};
use crate::engine::linklist::{Links, List, NodeId};
use crate::engine::xp::{get_exp_by_level, get_level_by_exp};
use crate::io::packet::Packet;
use crate::rsbuf::player_prot;
use ares_shared::script::ScriptFile;
use crate::script::state::{Arg, EntityRef, ScriptState};
use crate::util::jstring::{from_base37, to_base37, to_display_name};
use indexmap::{IndexMap, IndexSet};
use std::sync::Arc;

pub const SAV_MAGIC: i32 = 0x2004;
pub const SAV_VERSION: i32 = 8;

/// `PlayerStat`.
pub mod stat {
    pub const ATTACK: usize = 0;
    pub const DEFENCE: usize = 1;
    pub const STRENGTH: usize = 2;
    pub const HITPOINTS: usize = 3;
    pub const RANGED: usize = 4;
    pub const PRAYER: usize = 5;
    pub const MAGIC: usize = 6;
    pub const AGILITY: usize = 16;
    pub const RUNECRAFT: usize = 20;
    pub const COUNT: usize = 21;
}

/// `PlayerQueueType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueType {
    Normal,
    Long,
    Engine,
    Weak,
    Strong,
    Soft,
}

/// `ModalState`.
pub mod modal {
    pub const NONE: i32 = 0x0;
    pub const MAIN: i32 = 0x1;
    pub const CHAT: i32 = 0x2;
    pub const SIDE: i32 = 0x4;
    pub const TUT: i32 = 0x8;
}

/// `PlayerQueueRequest`.
#[derive(Clone, Debug)]
pub struct PlayerQueueRequest {
    pub kind: QueueType,
    pub script: Arc<ScriptFile>,
    pub args: Vec<Arg>,
    pub delay: i32,
}

/// `EntityTimer` for players (`PlayerTimerType`: 0 normal, 1 soft).
#[derive(Clone, Debug)]
pub struct PlayerTimer {
    pub kind: i32,
    pub script: Arc<ScriptFile>,
    pub args: Vec<Arg>,
    pub interval: i32,
    pub clock: i32,
}

/// `CameraInfo`.
#[derive(Clone, Debug)]
pub struct CameraInfo {
    pub kind: i32,
    pub cam_x: i32,
    pub cam_z: i32,
    pub height: i32,
    pub rotation_speed: i32,
    pub rotation_multiplier: i32,
}

/// `InventoryListener`.
#[derive(Clone, Debug)]
pub struct InvListener {
    pub kind: i32,
    pub com: i32,
    pub source: i64,
    pub first_seen: bool,
}

/// The entity `BuildArea`: zones sent to the client.
#[derive(Clone, Debug, Default)]
pub struct BuildArea {
    pub loaded_zones: IndexSet<i32>,
    pub active_zones: IndexSet<i32>,
    pub mapsquares: IndexSet<i32>,
    pub last_build: i32,
}

/// The connection a player is bound to (`NetworkPlayer.client`).
pub type ClientId = u32;

pub struct Player {
    pub e: EntityBase,
    pub p: Pathing,
    pub username: String,
    pub username37: i64,
    pub hash64: i64,
    pub display_name: String,
    pub body: [i32; 7],
    pub colors: [i32; 5],
    pub gender: i32,
    pub run: i32,
    pub temp_run: i32,
    pub runenergy: i32,
    pub last_run_energy: i32,
    pub runweight: i64,
    pub playtime: i32,
    pub stats: [i32; 21],
    /// `Uint8Array`s: writes wrap.
    pub levels: [u8; 21],
    pub base_levels: [u8; 21],
    pub vars: Vec<i32>,
    pub vars_string: Vec<Option<String>>,
    pub invs: IndexMap<i32, Inventory>,
    pub next_target: Option<EntityRef>,
    pub public_chat: i32,
    pub private_chat: i32,
    pub trade_duel: i32,
    pub session: String,
    pub slot: i32,
    pub uid: u32,
    pub reconnecting: bool,
    pub low_memory: bool,
    pub combat_level: i32,
    pub skill_level: i32,
    pub headicons: i32,
    pub last_stats: [i32; 21],
    pub last_levels: [u8; 21],
    pub origin_x: i32,
    pub origin_z: i32,
    pub build_area: BuildArea,
    pub anim_protect: i32,
    pub inv_listeners: Vec<InvListener>,
    pub allow_design: bool,
    pub afk_event_ready: bool,
    pub move_click_request: bool,
    pub request_logout: bool,
    pub request_idle_logout: bool,
    pub logging_out: bool,
    pub prevent_logout_message: Option<String>,
    pub prevent_logout_until: i32,
    pub last_response: i32,
    pub last_connected: i32,
    pub log_message: Option<String>,
    pub queue_links: Links<PlayerQueueRequest>,
    pub queue: List,
    pub weak_queue: List,
    pub engine_queue: List,
    pub queue_garbage: Vec<NodeId>,
    pub camera_packets: Vec<CameraInfo>,
    /// `timers`: a `Map` keyed by script id; deleted entries stay as `None`
    /// until the next `processTimers` pass so iteration matches JS `Map`.
    pub timers: Vec<(i32, Option<PlayerTimer>)>,
    pub tabs: [i32; 14],
    pub modal_state: i32,
    pub modal_main: i32,
    pub last_modal_main: i32,
    pub modal_chat: i32,
    pub last_modal_chat: i32,
    pub modal_side: i32,
    pub last_modal_side: i32,
    pub modal_tutorial: i32,
    pub overlay: i32,
    pub last_overlay: i32,
    /// `::serverstats` is showing (ares only).
    pub server_stats: bool,
    pub refresh_modal: bool,
    pub refresh_modal_close: bool,
    pub request_modal_close: bool,
    pub protect: bool,
    pub active_script: Option<Box<ScriptState>>,
    pub resume_buttons: Vec<i32>,
    pub last_item: i32,
    pub last_slot: i32,
    /// The listener inv of the last opheld/inv_button click (`active_invobj`).
    pub last_inv: i32,
    pub last_use_item: i32,
    pub last_use_slot: i32,
    pub last_target_slot: i32,
    pub last_com: i32,
    pub staff_mod_level: i32,
    pub visibility: i32,
    pub hero_points: HeroPoints,
    pub afk_zones: [i32; 2],
    pub last_afk_zone: i32,
    pub last_map_zone: i32,
    pub last_zone: i32,
    pub muted_until: Option<i64>,
    pub members: bool,
    pub message_count: i32,
    pub social_protect: bool,
    pub report_abuse_protect: bool,
    pub last_login_time: i64,
    pub appearance_inv: i32,
    pub appearance_buf: Option<Vec<u8>>,
    pub last_appearance: i32,
    pub readyanim: i32,
    pub turnanim: i32,
    pub walkanim: i32,
    pub walkanim_b: i32,
    pub walkanim_l: i32,
    pub walkanim_r: i32,
    pub runanim: i32,
    pub chat_message: Option<Vec<u8>>,
    pub chat_colour: Option<i32>,
    pub chat_effect: Option<i32>,
    pub chat_rights: Option<i32>,
    pub npc_id: i32,
    // NetworkPlayer
    pub client: Option<ClientId>,
    pub user_limit: i32,
    pub client_limit: i32,
    pub restricted_limit: i32,
    pub user_path: Vec<i32>,
    pub opcalled: bool,
}

impl Player {
    /// `enqueueScript(script, type, delay, args)`.
    pub fn enqueue_script(&mut self, script: Arc<ScriptFile>, kind: QueueType, delay: i32, args: Vec<Arg>) {
        let list = match kind {
            QueueType::Engine => self.engine_queue,
            QueueType::Weak => self.weak_queue,
            _ => self.queue,
        };
        let delay = if kind == QueueType::Engine { 0 } else { delay };
        let node = self.queue_links.node(PlayerQueueRequest { kind, script, args, delay });
        self.queue_links.add_tail(list, node);
    }

    /// `new Player(username, username37, hash64)`: on Tutorial Island, with
    /// int varps at 0 and other non-string varps at -1.
    pub fn new(username: String, username37: i64, hash64: i64, varps: &ConfigSet<VarPlayerType>, client_routefinder: bool) -> Self {
        let n = varps.count();
        let vars = (0..n).map(|i| match varps.get(i).map(|v| v.kind) {
            Some(svt::INT) => 0,
            Some(svt::STRING) => 0,
            _ => -1,
        }).collect();
        let mut queue_links = Links::default();
        let (queue, weak_queue, engine_queue) = (queue_links.list(), queue_links.list(), queue_links.list());
        let strategy = if client_routefinder { move_strategy::NAIVE } else { move_strategy::SMART };
        Player {
            e: EntityBase::new(0, 3094, 3106, 1, 1, lifecycle::FOREVER),
            p: Pathing::new(3094, 3106, super::block_walk::PLAYER, strategy, player_prot::FACE_COORD, player_prot::FACE_ENTITY),
            display_name: to_display_name(&username),
            username,
            username37,
            hash64,
            body: [0, 10, 18, 26, 33, 36, 42],
            colors: [0; 5],
            gender: 0,
            run: 0,
            temp_run: 0,
            runenergy: 10000,
            last_run_energy: -1,
            runweight: 0,
            playtime: 0,
            stats: [0; 21],
            levels: [0; 21],
            base_levels: [0; 21],
            vars,
            vars_string: vec![None; n],
            invs: IndexMap::new(),
            next_target: None,
            public_chat: 0,
            private_chat: 0,
            trade_duel: 0,
            session: "headless".into(),
            slot: -1,
            uid: u32::MAX,
            reconnecting: false,
            low_memory: false,
            combat_level: 3,
            skill_level: 0,
            headicons: 0,
            last_stats: [-1; 21],
            last_levels: [255; 21],
            origin_x: -1,
            origin_z: -1,
            build_area: BuildArea { last_build: -1, ..Default::default() },
            anim_protect: 0,
            inv_listeners: Vec::new(),
            allow_design: false,
            afk_event_ready: false,
            move_click_request: false,
            request_logout: false,
            request_idle_logout: false,
            logging_out: false,
            prevent_logout_message: None,
            prevent_logout_until: -1,
            last_response: -1,
            last_connected: -1,
            log_message: None,
            queue_links,
            queue,
            weak_queue,
            engine_queue,
            queue_garbage: Vec::new(),
            camera_packets: Vec::new(),
            timers: Vec::new(),
            tabs: [-1; 14],
            modal_state: modal::NONE,
            modal_main: -1,
            last_modal_main: -1,
            modal_chat: -1,
            last_modal_chat: -1,
            modal_side: -1,
            last_modal_side: -1,
            modal_tutorial: -1,
            overlay: -1,
            last_overlay: -1,
            server_stats: false,
            refresh_modal: false,
            refresh_modal_close: false,
            request_modal_close: false,
            protect: false,
            active_script: None,
            resume_buttons: Vec::new(),
            last_item: -1,
            last_slot: -1,
            last_inv: -1,
            last_use_item: -1,
            last_use_slot: -1,
            last_target_slot: -1,
            last_com: -1,
            staff_mod_level: 0,
            visibility: 0,
            hero_points: HeroPoints::default(),
            afk_zones: [0; 2],
            last_afk_zone: 0,
            last_map_zone: -1,
            last_zone: -1,
            muted_until: None,
            members: true,
            message_count: 0,
            social_protect: false,
            report_abuse_protect: false,
            last_login_time: 0,
            appearance_inv: -1,
            appearance_buf: None,
            last_appearance: 0,
            readyanim: -1,
            turnanim: -1,
            walkanim: -1,
            walkanim_b: -1,
            walkanim_l: -1,
            walkanim_r: -1,
            runanim: -1,
            chat_message: None,
            chat_colour: None,
            chat_effect: None,
            chat_rights: None,
            npc_id: -1,
            client: None,
            user_limit: 0,
            client_limit: 0,
            restricted_limit: 0,
            user_path: Vec::new(),
            opcalled: false,
        }
    }

    /// A non-shared inventory, created on first use (`getInventory` minus
    /// the World's shared ones).
    pub fn own_inventory(&mut self, invs: &ConfigSet<InvType>, inv: i32) -> Option<&mut Inventory> {
        let t = invs.get(usize::try_from(inv).ok()?)?;
        if t.scope == inv_cfg::SCOPE_SHARED {
            return None;
        }
        Some(self.invs.entry(inv).or_insert_with(|| Inventory::from_type(invs, inv)))
    }

    /// `MALE_FEMALE_MAP.get(part) ?? -1`.
    pub fn male_to_female(part: i32) -> i32 {
        match part {
            0..=9 => 45 + part + (part > 0) as i32,
            18..=25 => 56,
            26 => 61,
            27 => 63,
            28 => 62,
            29 => 65,
            30 => 64,
            31 => 63,
            32..=38 => part + 34,
            39 => 76,
            40 => 75,
            41..=44 => part + 37,
            _ => -1,
        }
    }

    /// `FEMALE_MALE_MAP.get(part) ?? -1`.
    pub fn female_to_male(part: i32) -> i32 {
        match part {
            45 | 46 => 0,
            47..=55 => part - 46,
            56..=60 => 18,
            61 => 26,
            62 => 27,
            63 => 28,
            64 | 65 => 29,
            66..=72 => part - 34,
            73 | 74 | 77 => 36,
            75 => 40,
            76 => 39,
            78..=81 => part - 37,
            _ => -1,
        }
    }

    /// `getCombatLevel()`.
    pub fn compute_combat_level(&self) -> i32 {
        let b = |s: usize| self.base_levels[s] as f64;
        let base = 0.25 * (b(stat::DEFENCE) + b(stat::HITPOINTS) + (b(stat::PRAYER) / 2.0).floor());
        let melee = 0.325 * (b(stat::ATTACK) + b(stat::STRENGTH));
        let range = 0.325 * ((b(stat::RANGED) / 2.0).floor() + b(stat::RANGED));
        let magic = 0.325 * ((b(stat::MAGIC) / 2.0).floor() + b(stat::MAGIC));
        (base + melee.max(range).max(magic)).floor() as i32
    }

    /// `Player.save()`.
    pub fn save(&self, varps: &ConfigSet<VarPlayerType>, invs: &ConfigSet<InvType>, varos: &ConfigSet<VarObjType>) -> Vec<u8> {
        let defaults = Item::default_vars(varos);
        let mut sav = Packet::alloc(2);
        sav.p2(SAV_MAGIC as i64);
        sav.p2(SAV_VERSION as i64);
        sav.p2(self.e.x as i64);
        sav.p2(self.e.z as i64);
        sav.p1(self.e.level as i64);
        for &b in &self.body {
            sav.p1(b as i64);
        }
        for &c in &self.colors {
            sav.p1(c as i64);
        }
        sav.p1(self.gender as i64);
        sav.p2(self.runenergy as i64);
        sav.p4(self.playtime as i64);
        for i in 0..21 {
            sav.p4(self.stats[i] as i64);
            sav.p1(self.levels[i] as i64);
        }
        let perm = |id: usize| varps.get(id).is_some_and(|v| v.scope == var_cfg::SCOPE_PERM) && self.vars[id] != 0;
        let saved: Vec<usize> = (0..self.vars.len()).filter(|&id| perm(id)).collect();
        sav.p2(saved.len() as i64);
        for id in saved {
            sav.p2(id as i64);
            sav.p_var_int(self.vars[id] as i64);
        }
        let start = sav.pos;
        sav.p1(0);
        let mut count = 0;
        for (&kind, inv) in &self.invs {
            if invs.get(kind as usize).map(|t| t.scope) != Some(inv_cfg::SCOPE_PERM) {
                continue;
            }
            sav.p2(kind as i64);
            sav.p2(inv.capacity as i64);
            for slot in 0..inv.capacity {
                match inv.get(slot) {
                    None => sav.p2(0),
                    Some(Item { id, count, vars }) => {
                        sav.p2(id as i64 + 1);
                        if count >= 255 {
                            sav.p1(255);
                            sav.p4(count);
                        } else {
                            sav.p1(count);
                        }
                        // v8: the vars that differ from their default
                        let set: Vec<(usize, i32)> = vars.iter().flat_map(|v| v.iter().copied().enumerate()).filter(|&(i, v)| defaults.get(i) != Some(&v)).collect();
                        sav.p1(set.len() as i64);
                        for (i, v) in set {
                            sav.p2(i as i64);
                            sav.p4(v as i64);
                        }
                    }
                }
            }
            count += 1;
        }
        sav.data[start] = count as u8;
        sav.p1(self.afk_zones.len() as i64);
        for &z in &self.afk_zones {
            sav.p4(z as i64);
        }
        sav.p2(self.last_afk_zone as i64);
        sav.p1(((self.public_chat << 4) | (self.private_chat << 2) | self.trade_duel) as i64);
        sav.p8(self.last_login_time);
        let crc = Packet::getcrc(&sav.data, 0, sav.pos);
        sav.p4(crc as i64);
        sav.data[..sav.pos].to_vec()
    }

    /// `PlayerLoading.load(name, sav, client)`: a fresh account when the save
    /// is shorter than 2 bytes.
    pub fn load(name: &str, sav: &[u8], varps: &ConfigSet<VarPlayerType>, invs: &ConfigSet<InvType>, varos: &ConfigSet<VarObjType>, client_routefinder: bool) -> Result<Player, String> {
        let name37 = to_base37(name) as i64;
        let safe = from_base37(name37);
        let mut p = Player::new(safe, name37, name37, varps, client_routefinder);
        if sav.len() < 2 {
            for i in 0..21 {
                p.stats[i] = 0;
                p.base_levels[i] = 1;
                p.levels[i] = 1;
            }
            p.stats[stat::HITPOINTS] = get_exp_by_level(10);
            p.base_levels[stat::HITPOINTS] = 10;
            p.levels[stat::HITPOINTS] = 10;
            return Ok(p);
        }
        let mut s = Packet::new(sav.to_vec());
        if s.g2() != SAV_MAGIC {
            return Err("Invalid save file".into());
        }
        let version = s.g2();
        if version > SAV_VERSION {
            return Err("Unsupported save version".into());
        }
        s.pos = sav.len() - 4;
        if s.g4s() != Packet::getcrc(sav, 0, sav.len() - 4) {
            return Err("Incorrect save checksum".into());
        }
        s.pos = 4;
        p.e.x = s.g2();
        p.e.z = s.g2();
        p.e.level = s.g1();
        for b in p.body.iter_mut() {
            *b = s.g1();
            if *b == 255 {
                *b = -1;
            }
        }
        for c in p.colors.iter_mut() {
            *c = s.g1();
        }
        p.gender = s.g1();
        p.runenergy = s.g2();
        p.playtime = if version >= 2 { s.g4s() } else { s.g2() };
        for i in 0..21 {
            p.stats[i] = s.g4s();
            p.base_levels[i] = get_level_by_exp(p.stats[i]) as u8;
            p.levels[i] = s.g1() as u8;
        }
        let varp_count = s.g2();
        for i in 0..varp_count {
            let (id, v) = if version >= 7 { (s.g2() as usize, s.g_var_int() as i32) } else { (i as usize, s.g4s()) };
            if let Some(slot) = p.vars.get_mut(id) {
                *slot = v;
            }
        }
        for _ in 0..s.g1() {
            let kind = s.g2();
            let t = invs.get(kind as usize);
            let size = match (version >= 5, t) {
                (true, _) => s.g2(),
                (false, Some(t)) => t.size,
                (false, None) => return Err(format!("save has inventory {kind} with no InvType")),
            };
            let mut objs = Vec::new();
            for slot in 0..size {
                let id = s.g2() - 1;
                if id == -1 {
                    continue;
                }
                let mut count = s.g1() as i64;
                if count == 255 {
                    count = s.g4s() as i64;
                }
                let mut item = Item::new(id, count);
                if version >= 8 {
                    for _ in 0..s.g1() {
                        let (var, v) = (s.g2() as usize, s.g4s());
                        // a varo the pack no longer has is dropped
                        if var < varos.count() {
                            item.vars.get_or_insert_with(|| Item::default_vars(varos))[var] = v;
                        }
                    }
                }
                objs.push((slot as usize, item));
            }
            let t = t.ok_or_else(|| format!("save has inventory {kind} with no InvType"))?;
            if t.scope == inv_cfg::SCOPE_PERM
                && let Some(inv) = p.own_inventory(invs, kind)
            {
                for (slot, item) in objs {
                    if slot < inv.items.len() {
                        inv.set(slot, Some(item));
                    }
                }
            }
        }
        if version >= 3 {
            for i in 0..s.g1() as usize {
                let v = s.g4s();
                if let Some(z) = p.afk_zones.get_mut(i) {
                    *z = v;
                }
            }
            p.last_afk_zone = s.g2();
        }
        if version >= 4 {
            let modes = s.g1();
            p.public_chat = (modes >> 4) & 0b11;
            p.private_chat = (modes >> 2) & 0b11;
            p.trade_duel = modes & 0b11;
        }
        if version >= 6 {
            p.last_login_time = s.g8();
        }
        p.combat_level = p.compute_combat_level();
        Ok(p)
    }
}
