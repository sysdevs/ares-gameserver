//! Who is online on which
//! world, friend/ignore lists, private messages and the admin relays.
//! One task owns the state; connections feed it their messages in order.

use super::db::{Db, now_ms, to_db_date};
use super::ws::{self, Conn};
use crate::link::friend::{FriendReply, FriendRequest, Name37, from_wire, to_wire};
use crate::util::config::WorldConfig;
use crate::util::jstring::{from_base37, to_base37};
use indexmap::IndexMap;
use serde_json::Value;
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const WORLD_PLAYER_LIMIT: usize = 2000;

#[derive(Default)]
struct Repository {
    players_by_world: HashMap<i64, Vec<Option<i64>>>,
    world_by_player: HashMap<String, i64>,
    staff: HashSet<i64>,
    private_chat: HashMap<String, i64>,
    /// Iterated in insertion order when finding followers.
    friends: IndexMap<String, Vec<i64>>,
    ignores: HashMap<String, Vec<i64>>,
}

pub struct FriendServer {
    db: Db,
    config: WorldConfig,
    repo: Repository,
    socket_by_world: HashMap<i64, (u64, UnboundedSender<Value>)>,
}

fn n37(name: &str) -> i64 {
    to_base37(name) as i64
}

impl FriendServer {
    pub async fn new(config: WorldConfig, root: PathBuf) -> Result<Self, String> {
        let db = Db::connect(&config.db, &root).await?;
        Ok(FriendServer { db, config, repo: Repository::default(), socket_by_world: HashMap::new() })
    }

    pub async fn run(mut self) -> std::io::Result<()> {
        let port = self.config.friend.port as u16;
        tracing::info!("Friend server listening on port {port}");
        let (tx, mut rx) = unbounded_channel::<(u64, UnboundedSender<Value>, Value)>();
        tokio::spawn(ws::serve(port, move |mut conn: Conn| {
            let tx = tx.clone();
            async move {
                while let Some(msg) = conn.incoming.recv().await {
                    if tx.send((conn.id, conn.out.clone(), msg)).is_err() {
                        break;
                    }
                }
            }
        }));
        let mut world_of_conn: HashMap<u64, i64> = HashMap::new();
        while let Some((conn, out, msg)) = rx.recv().await {
            if let Err(e) = self.handle(conn, &out, &msg, &mut world_of_conn).await {
                tracing::error!("{e}");
            }
        }
        Ok(())
    }

    fn profile(&self) -> &str {
        &self.config.node.profile
    }

    fn initialize_world(&mut self, world: i64, conn: u64, out: &UnboundedSender<Value>) {
        // the previous socket of the world is replaced, not terminated
        self.socket_by_world.insert(world, (conn, out.clone()));
        self.repo.players_by_world.entry(world).or_insert_with(|| vec![None; WORLD_PLAYER_LIMIT]);
    }

    async fn handle(&mut self, conn: u64, out: &UnboundedSender<Value>, msg: &Value, world_of: &mut HashMap<u64, i64>) -> Result<(), String> {
        use FriendRequest as R;
        let Some(req) = from_wire::<FriendRequest>(msg.clone()) else {
            tracing::error!("[Friend]: Unknown message type={}", msg["type"]);
            return Ok(());
        };
        if let R::FriendlistAdd { world, .. } | R::FriendlistDel { world, .. } | R::IgnorelistAdd { world, .. } | R::IgnorelistDel { world, .. } | R::PlayerLogin { world, .. } | R::PlayerLogout { world, .. } | R::PlayerChatSetmode { world, .. } | R::PrivateMessage { world, .. } = req
            && !world_of.contains_key(&conn)
        {
            world_of.insert(conn, world);
            self.initialize_world(world, conn, out);
        }
        match req {
            R::WorldConnect { world, profile } => {
                if world_of.contains_key(&conn) {
                    return Ok(());
                }
                world_of.insert(conn, world);
                if profile != self.profile() {
                    tracing::error!("[Friends]: World {world} tried to connect with incorrect profile: {profile:?}");
                    return Ok(());
                }
                self.initialize_world(world, conn, out);
            }
            R::PlayerLogin { username37: Name37(user), private_chat, staff_lvl, .. } => {
                let world = world_of[&conn];
                let private_chat = if (0..=2).contains(&private_chat) { private_chat } else { 0 };
                self.unregister(user);
                if !self.register(world, user, private_chat, staff_lvl).await? {
                    return Ok(());
                }
                let friends = self.get_friends(user).await?;
                let _ = out.send(to_wire(&FriendReply::UpdateFriendlist { username37: Name37(user), friends: friends.into_iter().map(|(w, f)| (Some(w), Name37(f))).collect() }));
                let ignores = self.load_ignores(user).await?;
                let _ = out.send(to_wire(&FriendReply::UpdateIgnorelist { username37: Name37(user), ignored: ignores.into_iter().map(Name37).collect() }));
                self.broadcast_world_to_followers(user);
            }
            R::PlayerLogout { username37: Name37(user), .. } => {
                self.unregister(user);
                self.broadcast_world_to_followers(user);
            }
            R::PlayerChatSetmode { username37: Name37(user), private_chat, .. } => {
                let mode = if (0..=2).contains(&private_chat) { private_chat } else { 0 };
                self.repo.private_chat.insert(from_base37(user), mode);
                self.broadcast_world_to_followers(user);
            }
            R::FriendlistAdd { username37: Name37(user), target_username37: Name37(target), .. } => {
                self.add_friend(user, target).await?;
                self.send_player_world_update(user, target);
                self.broadcast_world_to_followers(user);
            }
            R::FriendlistDel { username37: Name37(user), target_username37: Name37(target), .. } => {
                self.delete_friend(user, target).await?;
                self.broadcast_world_to_followers(user);
            }
            R::IgnorelistAdd { username37: Name37(user), target_username37: Name37(target), .. } => {
                self.add_ignore(user, target).await?;
                self.broadcast_world_to_followers(user);
            }
            R::IgnorelistDel { username37: Name37(user), target_username37: Name37(target), .. } => {
                self.delete_ignore(user, target).await?;
                self.broadcast_world_to_followers(user);
            }
            R::PrivateMessage { profile, username37: Name37(user), target_username37: Name37(target), staff_lvl, pm_id, chat, coord, .. } => {
                let id = |name: String| {
                    let pool = &self.db.pool;
                    async move { sqlx::query("SELECT id FROM account WHERE username = ?").bind(name).fetch_one(pool).await.map(|r| r.get::<i64, _>(0)) }
                };
                let from = id(from_base37(user)).await.map_err(|e| e.to_string())?;
                let to = id(from_base37(target)).await.map_err(|e| e.to_string())?;
                sqlx::query("INSERT INTO private_chat (account_id, profile, to_account_id, timestamp, coord, message) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(from)
                    .bind(profile)
                    .bind(to)
                    .bind(to_db_date(now_ms()))
                    .bind(coord)
                    .bind(&chat)
                    .execute(&self.db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(s) = self.player_world_socket(target) {
                    let _ = s.send(to_wire(&FriendReply::PrivateMessage { username37: Name37(user), target_username37: Name37(target), staff_lvl, pm_id, chat }));
                }
            }
            R::PublicChatLog { node_time, session_uuid, coord, chat, .. } => {
                sqlx::query("INSERT INTO public_chat (session_uuid, timestamp, coord, message) VALUES (?, ?, ?, ?)")
                    .bind(session_uuid)
                    .bind(to_db_date(node_time.map_or_else(now_ms, |t| t.floor() as i64)))
                    .bind(coord)
                    .bind(chat)
                    .execute(&self.db.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            R::RelayMute { node_id, username, muted_until } => self.relay(node_id, FriendReply::RelayMute { username, muted_until }),
            R::RelayKick { node_id, username } => self.relay(node_id, FriendReply::RelayKick { username }),
            R::RelayShutdown { node_id, duration } => self.relay(node_id, FriendReply::RelayShutdown { duration }),
            R::RelayBroadcast { node_id, broadcast } => self.relay(node_id, FriendReply::RelayBroadcast { message: broadcast }),
            R::RelayTrack { node_id, username, state } => self.relay(node_id, FriendReply::RelayTrack { username, state }),
            R::RelayReload { node_id } => self.relay(node_id, FriendReply::RelayReload),
            R::RelayClearlogins { node_id } => self.relay(node_id, FriendReply::RelayClearlogins),
            R::RelayClearlogouts { node_id } => self.relay(node_id, FriendReply::RelayClearlogouts),
            R::RelayQueuescript { node_id, username, script_name } => self.relay(node_id, FriendReply::RelayQueuescript { username, script_name }),
        }
        Ok(())
    }

    /// An admin relay, passed on to the world it names.
    fn relay(&self, node: i64, msg: FriendReply) {
        if let Some((_, s)) = self.socket_by_world.get(&node) {
            let _ = s.send(to_wire(&msg));
        }
    }

    // ---- repository ----

    async fn register(&mut self, world: i64, user: i64, private_chat: i64, staff: i64) -> Result<bool, String> {
        let name = from_base37(user);
        let Some(slot) = self.repo.players_by_world.get(&world).and_then(|w| w.iter().position(Option::is_none)) else {
            tracing::error!("[Friends]: World {world} is full");
            return Ok(false);
        };
        if staff > 1 {
            self.repo.staff.insert(user);
        }
        self.repo.players_by_world.get_mut(&world).unwrap()[slot] = Some(user);
        self.repo.world_by_player.insert(name.clone(), world);
        self.repo.private_chat.insert(name, private_chat);
        self.load_friends(user).await?;
        Ok(true)
    }

    fn unregister(&mut self, user: i64) {
        let name = from_base37(user);
        let r = &mut self.repo;
        let mut worlds: Vec<i64> = r.world_by_player.get(&name).copied().into_iter().collect();
        worlds.extend(r.players_by_world.keys().copied());
        for w in worlds {
            if let Some(slot) = r.players_by_world.get_mut(&w).and_then(|p| p.iter_mut().find(|p| **p == Some(user))) {
                *slot = None;
                r.world_by_player.remove(&name);
                r.private_chat.remove(&name);
                r.friends.shift_remove(&name);
                r.staff.remove(&user);
                return;
            }
        }
    }

    async fn load_friends(&mut self, user: i64) -> Result<(), String> {
        let name = from_base37(user);
        let rows = sqlx::query("SELECT a.username FROM account a INNER JOIN friendlist f ON a.id = f.friend_account_id INNER JOIN account local ON local.id = f.account_id WHERE local.username = ? AND f.profile = ? ORDER BY f.created ASC")
            .bind(&name)
            .bind(self.profile().to_string())
            .fetch_all(&self.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        self.repo.friends.insert(name, rows.iter().map(|r| n37(&r.get::<String, _>(0))).collect());
        Ok(())
    }

    async fn load_ignores(&mut self, user: i64) -> Result<Vec<i64>, String> {
        let name = from_base37(user);
        let rows = sqlx::query("SELECT i.value FROM account local INNER JOIN ignorelist i ON local.id = i.account_id WHERE local.username = ? AND i.profile = ? ORDER BY i.created ASC")
            .bind(&name)
            .bind(self.profile().to_string())
            .fetch_all(&self.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        let list: Vec<i64> = rows.iter().map(|r| n37(&r.get::<String, _>(0))).collect();
        self.repo.ignores.insert(name, list.clone());
        Ok(list)
    }

    /// Online friends by world, then the rest at world 0.
    async fn get_friends(&mut self, user: i64) -> Result<Vec<(i64, i64)>, String> {
        self.load_friends(user).await?;
        let name = from_base37(user);
        let mine = self.repo.friends.get(&name).cloned().unwrap_or_default();
        let mut worlds: Vec<i64> = self.repo.players_by_world.keys().copied().collect();
        worlds.sort();
        let mut out = Vec::new();
        for w in worlds {
            for p in self.repo.players_by_world[&w].iter().flatten() {
                if mine.contains(p) && self.is_visible_to(user, *p) {
                    out.push((w, *p));
                }
            }
        }
        for f in mine {
            if !out.iter().any(|(_, p)| *p == f) {
                out.push((0, f));
            }
        }
        Ok(out)
    }

    fn followers(&self, user: i64) -> Vec<i64> {
        self.repo.friends.iter().filter(|(_, f)| f.contains(&user)).map(|(n, _)| n37(n)).collect()
    }

    fn is_visible_to(&self, viewer: i64, other: i64) -> bool {
        if self.repo.staff.contains(&viewer) {
            return true;
        }
        let other_name = from_base37(other);
        if self.repo.ignores.get(&other_name).is_some_and(|i| i.contains(&viewer)) {
            return false;
        }
        match self.repo.private_chat.get(&other_name).copied().unwrap_or(2) {
            2 => false,
            1 => self.repo.friends.get(&other_name).is_some_and(|f| f.contains(&viewer)),
            _ => true,
        }
    }

    fn player_world_socket(&self, user: i64) -> Option<&UnboundedSender<Value>> {
        let world = self.repo.world_by_player.get(&from_base37(user))?;
        self.socket_by_world.get(world).map(|(_, s)| s)
    }

    fn send_player_world_update(&self, viewer: i64, other: i64) {
        let Some(s) = self.player_world_socket(viewer) else { return };
        let world = if self.is_visible_to(viewer, other) { self.repo.world_by_player.get(&from_base37(other)).copied() } else { Some(0) };
        let _ = s.send(to_wire(&FriendReply::UpdateFriendlist { username37: Name37(viewer), friends: vec![(world, Name37(other))] }));
    }

    fn broadcast_world_to_followers(&self, user: i64) {
        for f in self.followers(user) {
            self.send_player_world_update(f, user);
        }
    }

    async fn account_id(&self, name: &str) -> Result<Option<(i64, i64)>, String> {
        let r = sqlx::query("SELECT id, members + 0 FROM account WHERE username = ? LIMIT 1").bind(name).fetch_optional(&self.db.pool).await.map_err(|e| e.to_string())?;
        Ok(r.map(|r| (r.get::<i64, _>(0), r.get::<i64, _>(1))))
    }

    async fn add_friend(&mut self, user: i64, target: i64) -> Result<(), String> {
        let (name, target_name) = (from_base37(user), from_base37(target));
        if self.repo.friends.entry(name.clone()).or_default().contains(&target) {
            return Ok(());
        }
        let (Some((id, members)), Some((fid, _))) = (self.account_id(&name).await?, self.account_id(&target_name).await?) else { return Ok(()) };
        let count: i64 = sqlx::query("SELECT COUNT(*) FROM friendlist WHERE account_id = ?").bind(id).fetch_one(&self.db.pool).await.map_err(|e| e.to_string())?.get(0);
        if count >= if members != 0 { 200 } else { 100 } {
            return Ok(());
        }
        self.repo.friends.get_mut(&name).unwrap().push(target);
        sqlx::query("INSERT INTO friendlist (account_id, profile, friend_account_id) VALUES (?, ?, ?)")
            .bind(id)
            .bind(self.profile().to_string())
            .bind(fid)
            .execute(&self.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn delete_friend(&mut self, user: i64, target: i64) -> Result<(), String> {
        let name = from_base37(user);
        let list = self.repo.friends.entry(name.clone()).or_default();
        let Some(i) = list.iter().position(|&f| f == target) else { return Ok(()) };
        list.remove(i);
        sqlx::query("DELETE FROM friendlist WHERE profile = ? AND account_id IN (SELECT id FROM account WHERE username = ?) AND friend_account_id IN (SELECT id FROM account WHERE username = ?)")
            .bind(self.profile().to_string())
            .bind(name)
            .bind(from_base37(target))
            .execute(&self.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn add_ignore(&mut self, user: i64, value: i64) -> Result<(), String> {
        let name = from_base37(user);
        if self.repo.ignores.entry(name.clone()).or_default().contains(&value) {
            return Ok(());
        }
        let Some((id, _)) = self.account_id(&name).await? else { return Ok(()) };
        let count: i64 = sqlx::query("SELECT COUNT(*) FROM ignorelist WHERE account_id = ?").bind(id).fetch_one(&self.db.pool).await.map_err(|e| e.to_string())?.get(0);
        if count >= 100 {
            return Ok(());
        }
        self.repo.ignores.get_mut(&name).unwrap().push(value);
        let sql = if self.db.mysql {
            "INSERT INTO ignorelist (account_id, profile, value) VALUES (?, ?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)"
        } else {
            "INSERT INTO ignorelist (account_id, profile, value) VALUES (?, ?, ?) ON CONFLICT DO NOTHING"
        };
        sqlx::query(sql).bind(id).bind(self.profile().to_string()).bind(from_base37(value)).execute(&self.db.pool).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn delete_ignore(&mut self, user: i64, value: i64) -> Result<(), String> {
        let name = from_base37(user);
        let list = self.repo.ignores.entry(name.clone()).or_default();
        let Some(i) = list.iter().position(|&v| v == value) else { return Ok(()) };
        list.remove(i);
        sqlx::query("DELETE FROM ignorelist WHERE profile = ? AND value = ? AND account_id IN (SELECT id FROM account WHERE username = ?)")
            .bind(self.profile().to_string())
            .bind(from_base37(value))
            .bind(name)
            .execute(&self.db.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
