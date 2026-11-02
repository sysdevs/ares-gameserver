//! The World thread: the tick loop of `World.cycle()` with its scheduling
//! arithmetic, connection events between ticks (socket data is handled as
//! it arrives, between cycles), the server links and the on-demand cache
//! service (`OnDemandThread`).

use super::world::{ClientId, Outgoing, World};
use crate::cache::ondemand::{OnDemand, Outcome, decode_request};
use crate::io::filestream::FileStream;
use crate::link::friend::{self, FriendOut, FriendRequest, Name37};
use crate::link::login::{self, LoginMsg, LoginReply, LoginRequest};
use crate::net::tcp::Inbound;
use crate::pack::dev::{DevMsg, DevWatcher};
use crate::server::ws;
use crate::util::base64;
use crate::util::jstring::to_base37;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Where login-thread requests go.
pub enum LoginLink {
    /// `login.enabled=false`: saves under `data/players/<profile>`.
    Inline { dir: PathBuf },
    /// `LoginClient` to an `ares-login` server.
    Ws { host: String, port: u16 },
}

/// The node identity sent on every link message.
pub struct Node {
    pub id: i64,
    pub profile: String,
}

enum Pending {
    Login { socket: String, username: String, low_memory: bool, reconnecting: bool },
    Logout { username: String },
}

struct WsLink {
    client: ws::Client,
    incoming: UnboundedReceiver<Value>,
}

impl WsLink {
    fn new(host: String, port: u16, hello: Value, once: bool) -> WsLink {
        let (tx, rx) = unbounded_channel();
        WsLink { client: ws::Client::spawn(host, port, tx, Some(hello), once), incoming: rx }
    }
}

pub struct Runtime {
    pub world: World,
    events: Receiver<Inbound>,
    senders: HashMap<ClientId, UnboundedSender<Outgoing>>,
    ondemand: OnDemand<ClientId>,
    cache: FileStream,
    node: Node,
    inline_dir: Option<PathBuf>,
    login: Option<WsLink>,
    pending: HashMap<String, (Pending, Instant)>,
    next_request: u64,
    friend: Option<WsLink>,
    pack: PathBuf,
    /// The live-reload watcher and the `<srcDir>/scripts/` prefix it strips from errors.
    dev: Option<(DevWatcher, String)>,
    stop: Arc<AtomicBool>,
    rt: tokio::runtime::Runtime,
    /// Session-UUID mulberry32 state, apart from the gameplay `Math.random` stream.
    uuid_state: i32,
}

impl Runtime {
    pub fn new(world: World, events: Receiver<Inbound>, pack: &std::path::Path, node: Node, login: LoginLink, friend: Option<(String, u16)>, dev: Option<(DevWatcher, String)>) -> Result<Self, String> {
        let cache = FileStream::open(pack, false).map_err(|e| format!("cache: {e}"))?;
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
        let _guard = rt.enter();
        // `world_startup` resets this node's logins, so only on the first connection;
        // the friend server needs WORLD_CONNECT on every socket
        let startup = LoginMsg { node_id: node.id, node_time: Some(world.clock_ms() as f64), profile: node.profile.clone(), req: LoginRequest::WorldStartup, node_members: None, reply_to: None };
        let startup = serde_json::to_value(startup).expect("login message");
        let (inline_dir, login) = match login {
            LoginLink::Inline { dir } => (Some(dir), None),
            LoginLink::Ws { host, port } => (None, Some(WsLink::new(host, port, startup, true))),
        };
        let hello = friend::to_wire(&FriendRequest::WorldConnect { world: node.id, profile: node.profile.clone() });
        let friend = friend.map(|(h, p)| WsLink::new(h, p, hello, false));
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                s.store(true, Ordering::SeqCst);
            }
        });
        let mut r = Runtime { world, events, senders: HashMap::new(), ondemand: OnDemand::default(), cache, node, inline_dir, login, pending: HashMap::new(), next_request: 0, friend, pack: pack.to_path_buf(), dev, stop, rt, uuid_state: std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), 0u8) as i32 };
        if r.inline_dir.is_some() {
            r.world.login_out.push(LoginRequest::WorldStartup);
        }
        Ok(r)
    }

    /// Runs ticks until shutdown completes: `sleep(max(0, tickRate - elapsed - drift))`.
    pub fn run(mut self) {
        let mut next_tick = Instant::now();
        loop {
            loop {
                self.between_ticks();
                let now = Instant::now();
                if now >= next_tick {
                    break;
                }
                std::thread::sleep((next_tick - now).min(Duration::from_millis(5)));
            }
            if self.stop.swap(false, Ordering::SeqCst) && self.world.shutdown_tick == -1 {
                self.world.reboot_timer(0);
            }
            let start = Instant::now();
            let drift = start.saturating_duration_since(next_tick);
            let done = self.world.cycle();
            self.flush();
            if done {
                // give the logout saves a moment to reach the login server
                std::thread::sleep(Duration::from_millis(500));
                return;
            }
            let elapsed = start.elapsed();
            let rate = Duration::from_secs_f64(self.world.tick_rate / 1000.0);
            next_tick = start + rate.saturating_sub(elapsed).saturating_sub(drift) + elapsed;
        }
    }

    /// Connection events, link replies and cache chunks.
    fn between_ticks(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Inbound::Connect { id, remote, out } => {
                    let uuid = format!("{:032x}", rand_u128(&mut self.uuid_state));
                    self.world.connect_as(id, &remote, uuid);
                    self.senders.insert(id, out);
                }
                Inbound::Data(id, bytes) => {
                    self.world.client_data(id, &bytes);
                    if self.world.client(id).is_some_and(|c| c.state == crate::engine::world::ClientState::OnDemand) {
                        self.ondemand_requests(id);
                    }
                }
                Inbound::Closed(id) => {
                    self.world.client_closed(id);
                    self.ondemand.client_closed(&id);
                    self.senders.remove(&id);
                }
            }
        }
        self.poll_links();
        self.poll_dev();
        if self.ondemand.has_work() {
            let mut out = Vec::new();
            self.ondemand.pump(&mut self.cache, &mut out);
            for (id, chunk) in out {
                if let Some(s) = self.senders.get(&id) {
                    let _ = s.send(Outgoing::Data(chunk));
                }
            }
        }
        self.flush();
    }

    /// `OnDemand.onClientData`: 4-byte `archive, file, priority` requests.
    fn ondemand_requests(&mut self, id: ClientId) {
        while let Some(b) = self.world.client_read(id, 4) {
            let Some((archive, file, priority)) = decode_request([b[0], b[1], b[2], b[3]]) else {
                self.world.close(id);
                return;
            };
            if let Outcome::Close = self.ondemand.request(&id, archive, file, priority) {
                self.world.close(id);
                return;
            }
        }
    }

    /// Replies from the login server (matched by `replyTo`, 10 s timeout as
    /// `fetchSync`) and messages from the friend server.
    fn poll_links(&mut self) {
        let mut replies = Vec::new();
        if let Some(l) = self.login.as_mut() {
            while let Ok(v) = l.incoming.try_recv() {
                let Some(id) = v["replyTo"].as_str() else { continue };
                if let Some((p, _)) = self.pending.remove(id) {
                    replies.push(reply_for(p, Some(&v), self.world.production));
                }
            }
        }
        let expired: Vec<String> = self.pending.iter().filter(|(_, (_, t))| t.elapsed() > Duration::from_secs(10)).map(|(k, _)| k.clone()).collect();
        for k in expired {
            let (p, _) = self.pending.remove(&k).unwrap();
            replies.push(reply_for(p, None, self.world.production));
        }
        for r in replies {
            self.world.on_login_reply(r);
        }
        if let Some(f) = self.friend.as_mut() {
            let mut msgs = Vec::new();
            while let Ok(v) = f.incoming.try_recv() {
                msgs.push(v);
            }
            for v in msgs {
                let op = v["type"].clone();
                match friend::from_wire(v) {
                    Some(m) => self.world.on_friend_message(m),
                    None => tracing::error!("Unknown friend message: {op}"),
                }
            }
        }
    }

    /// `createDevThread`'s message handler, `World.rebuild()`, and
    /// `OnDemand.reloadCache()` after any reload.
    fn poll_dev(&mut self) {
        let rebuild = std::mem::take(&mut self.world.rebuild_requested);
        if let Some((dev, prefix)) = &self.dev {
            if rebuild {
                dev.rebuild();
            }
            while let Ok(m) = dev.messages.try_recv() {
                match m {
                    DevMsg::Reload => self.world.reload(true),
                    DevMsg::Failure(e) => {
                        tracing::error!("{e}");
                        self.world.broadcast_mes(&e.replace(prefix.as_str(), ""));
                        self.world.broadcast_mes("Check the console for more information.");
                    }
                }
            }
        }
        if std::mem::take(&mut self.world.reloaded) {
            match FileStream::open(&self.pack, false) {
                Ok(c) => self.cache = c,
                Err(e) => tracing::error!("cache: {e}"),
            }
        }
    }

    /// Sends the World's queued output and its link requests.
    fn flush(&mut self) {
        // `nodeTime` goes out as a double (`1700000000000.0`)
        let now = self.world.clock_ms() as f64;
        for req in std::mem::take(&mut self.world.login_out) {
            if let Some(dir) = &self.inline_dir {
                match login::inline(dir, self.world.production, self.world.map.members, &req) {
                    Ok(Some(reply)) => self.world.on_login_reply(reply),
                    Ok(None) => {}
                    Err(e) => tracing::error!("login: {e}"),
                }
                continue;
            }
            let Some(link) = &self.login else { continue };
            let (mut node_members, mut reply_to) = (None, None);
            let pending = match &req {
                LoginRequest::WorldStartup => continue,
                LoginRequest::PlayerLogin { socket, username, low_memory, reconnecting, .. } => {
                    node_members = Some(self.world.map.members);
                    Some(Pending::Login { socket: socket.clone(), username: username.clone(), low_memory: *low_memory, reconnecting: *reconnecting })
                }
                LoginRequest::PlayerLogout { username, .. } => Some(Pending::Logout { username: username.clone() }),
                _ => None,
            };
            if let Some(p) = pending {
                self.next_request += 1;
                let id = format!("id_{}", self.next_request);
                self.pending.insert(id.clone(), (p, Instant::now()));
                reply_to = Some(id);
            }
            let msg = LoginMsg { node_id: self.node.id, node_time: Some(now), profile: self.node.profile.clone(), req, node_members, reply_to };
            link.client.send(serde_json::to_value(msg).expect("login message"));
        }
        for m in std::mem::take(&mut self.world.friend_out) {
            if let Some(f) = &self.friend {
                f.client.send(friend::to_wire(&friend_message(m, &self.node, now)));
            }
        }
        for (id, out) in std::mem::take(&mut self.world.outbox) {
            let done = !matches!(out, Outgoing::Data(_));
            if let Some(s) = self.senders.get(&id) {
                let _ = s.send(out);
            }
            if done {
                self.senders.remove(&id);
            }
        }
        let _ = &self.rt;
    }
}

/// `LoginThread`'s reply for a request: `-1` / `false` when the login server
/// did not answer.
fn reply_for(p: Pending, v: Option<&Value>, production: bool) -> LoginReply {
    match p {
        Pending::Login { socket, username, low_memory, reconnecting } => {
            let Some(v) = v else {
                return LoginReply::PlayerLogin { socket, username, low_memory, reconnecting, reply: -1, staffmodlevel: None, save: None, members: false, muted_until: None, message_count: None, remaining: None };
            };
            let staff = if production { v["staffmodlevel"].as_i64().map(|s| s as i32) } else { Some(4) };
            LoginReply::PlayerLogin {
                socket,
                username,
                low_memory,
                reconnecting,
                reply: v["response"].as_i64().unwrap_or(-1) as i32,
                staffmodlevel: staff,
                save: v["save"].as_str().map(base64::decode),
                members: v["members"].as_bool().unwrap_or_else(|| v["members"].as_i64().unwrap_or(0) != 0),
                muted_until: v["muted_until"].as_str().and_then(crate::server::db::parse_db_date),
                message_count: v["messageCount"].as_i64().map(|c| c as i32),
                remaining: v["remaining"].as_f64(),
            }
        }
        Pending::Logout { username } => LoginReply::PlayerLogout { username, success: v.is_some_and(|v| v["response"].as_i64() == Some(0)) },
    }
}

/// `FriendThread` → `FriendClient`: the world's message as a friend-server packet.
fn friend_message(m: FriendOut, node: &Node, now: f64) -> FriendRequest {
    use FriendRequest as R;
    let world = node.id;
    let n37 = |u: &str| Name37(to_base37(u) as i64);
    match m {
        FriendOut::PlayerLogin { username, chat_mode_private, staff_lvl } => R::PlayerLogin { world, username37: n37(&username), private_chat: chat_mode_private as i64, staff_lvl: staff_lvl as i64 },
        FriendOut::PlayerLogout { username } => R::PlayerLogout { world, username37: n37(&username) },
        FriendOut::PlayerFriendslistAdd { username, target } => R::FriendlistAdd { world, username37: n37(&username), target_username37: target },
        FriendOut::PlayerFriendslistRemove { username, target } => R::FriendlistDel { world, username37: n37(&username), target_username37: target },
        FriendOut::PlayerIgnorelistAdd { username, target } => R::IgnorelistAdd { world, username37: n37(&username), target_username37: target },
        FriendOut::PlayerIgnorelistRemove { username, target } => R::IgnorelistDel { world, username37: n37(&username), target_username37: target },
        FriendOut::PlayerChatSetmode { username, chat_mode_private } => R::PlayerChatSetmode { world, username37: n37(&username), private_chat: chat_mode_private as i64 },
        FriendOut::PrivateMessage { username, staff_lvl, pm_id, target, message, coord } => R::PrivateMessage {
            world,
            profile: node.profile.clone(),
            node_time: now,
            username37: n37(&username),
            target_username37: target,
            staff_lvl: staff_lvl as i64,
            pm_id: pm_id as i64,
            chat: message,
            coord: coord as i64,
        },
        FriendOut::PublicMessage { session_uuid, coord, chat } => R::PublicChatLog { node_id: world, profile: node.profile.clone(), node_time: Some(now), session_uuid, coord: coord as i64, chat },
    }
}

/// `randomUUID()` stand-in for session ids (not part of any replay).
fn rand_u128(state: &mut i32) -> u128 {
    (0..4).fold(0, |acc, _| (acc << 32) | (crate::engine::world::mulberry32(state) * 4294967296.0) as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link messages print as the hand-built JSON did.
    #[test]
    fn link_messages_wire() {
        let node = Node { id: 10, profile: "main".into() };
        let pm = FriendOut::PrivateMessage { username: "alice".into(), staff_lvl: 2, pm_id: 77, target: Name37(3295), message: "hi".into(), coord: 12345 };
        assert_eq!(
            friend::to_wire(&friend_message(pm, &node, 1700000000000.0)).to_string(),
            r#"{"type":8,"world":10,"profile":"main","nodeTime":1700000000000.0,"username37":"2494434","targetUsername37":"3295","staffLvl":2,"pmId":77,"chat":"hi","coord":12345}"#
        );
        let ban = LoginMsg { node_id: 10, node_time: Some(1700000000000.0), profile: "main".into(), req: LoginRequest::PlayerBan { staff: "mod".into(), username: "bob".into(), until: 1700000123000 }, node_members: None, reply_to: None };
        assert_eq!(serde_json::to_string(&ban).unwrap(), r#"{"nodeId":10,"nodeTime":1700000000000.0,"profile":"main","type":"player_ban","staff":"mod","username":"bob","until":"2023-11-14 22:15:23"}"#);
        let logout = LoginMsg { req: LoginRequest::PlayerLogout { username: "bob".into(), save: vec![1, 2, 3] }, reply_to: Some("id_1".into()), ..ban };
        assert_eq!(serde_json::to_string(&logout).unwrap(), r#"{"nodeId":10,"nodeTime":1700000000000.0,"profile":"main","type":"player_logout","username":"bob","save":"AQID","replyTo":"id_1"}"#);
        assert_eq!(serde_json::from_value::<LoginMsg>(serde_json::to_value(&logout).unwrap()).unwrap(), logout);
    }
}
