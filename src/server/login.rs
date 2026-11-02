//! Accounts, sessions, saves under
//! `data/players/<profile>` and hiscores, behind a JSON WebSocket.

use super::db::{Db, now_ms, parse_db_date, to_db_date};
use super::ws::{self, Conn};
use crate::cache::config::{ConfigSet, inv::InvType, var::{VarObjType, VarPlayerType}};
use crate::engine::entity::Player;
use crate::io::packet::Packet;
use crate::link::login::{LoginMsg, LoginRequest};
use crate::util::base64;
use crate::util::config::WorldConfig;
use crate::util::jstring::to_safe_name;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Which stats count for hiscores.
const STAT_ENABLED: [bool; 21] = [true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, false, false, true];

pub struct LoginServer {
    db: Db,
    config: WorldConfig,
    players_dir: PathBuf,
    varps: ConfigSet<VarPlayerType>,
    invs: ConfigSet<InvType>,
    varos: ConfigSet<VarObjType>,
    logins: Mutex<HashSet<String>>,
}

/// Checks a save file's header and checksum.
pub fn verify_save(sav: &[u8]) -> bool {
    if sav.len() < 8 {
        return false;
    }
    let mut p = Packet::new(sav.to_vec());
    if p.g2() != crate::engine::entity::player::SAV_MAGIC || p.g2() > crate::engine::entity::player::SAV_VERSION {
        return false;
    }
    p.pos = sav.len() - 4;
    p.g4s() == Packet::getcrc(sav, 0, sav.len() - 4)
}

struct Account {
    id: i64,
    password: String,
    staffmodlevel: i64,
    members: i64,
    muted_until: Option<String>,
    banned_until: Option<String>,
    login_account_id: Option<i64>,
    logged_in: Option<i64>,
    logged_out: Option<i64>,
    logout_time: Option<String>,
}

impl LoginServer {
    pub async fn new(config: WorldConfig, root: PathBuf) -> Result<Arc<Self>, String> {
        let db = Db::connect(&config.db, &root).await?;
        let pack = root.join("data/pack");
        let varps = ConfigSet::load_with_client(&pack, "varp.dat")?;
        let invs = ConfigSet::load(&pack, "inv.dat")?;
        let varos = ConfigSet::load(&pack, "varo.dat")?;
        let players_dir = root.join("data/players");
        Ok(Arc::new(LoginServer { db, config, players_dir, varps, invs, varos, logins: Mutex::new(HashSet::new()) }))
    }

    pub async fn run(self: Arc<Self>) -> std::io::Result<()> {
        let port = self.config.login.port as u16;
        tracing::info!("Login server listening on port {port}");
        ws::serve(port, move |conn| self.clone().connection(conn)).await
    }

    async fn connection(self: Arc<Self>, mut conn: Conn) {
        while let Some(msg) = conn.incoming.recv().await {
            let (this, out) = (self.clone(), conn.out.clone());
            tokio::spawn(async move {
                if let Err(e) = this.handle(&msg, &out).await {
                    tracing::error!("{e}");
                }
            });
        }
    }

    fn save_path(&self, profile: &str, username: &str) -> PathBuf {
        self.players_dir.join(profile).join(format!("{username}.sav"))
    }

    async fn account(&self, username: &str, profile: &str) -> Result<Option<Account>, String> {
        let d = &self.db;
        let sql = format!(
            "SELECT a.id, a.password, a.staffmodlevel, a.members + 0 AS members, {}, {}, l.account_id, l.logged_in, l.logged_out, {} \
             FROM account a LEFT JOIN account_login l ON l.account_id = a.id AND l.profile = ? WHERE a.username = ?",
            d.text("a.muted_until"),
            d.text("a.banned_until"),
            d.text("l.logout_time")
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(profile).bind(username).fetch_optional(&d.pool).await.map_err(|e| e.to_string())?;
        Ok(row.map(|r| Account {
            id: r.get::<i64, _>(0),
            password: r.get(1),
            staffmodlevel: r.get::<i64, _>(2),
            members: r.get::<i64, _>(3),
            muted_until: r.get(4),
            banned_until: r.get(5),
            login_account_id: r.get(6),
            logged_in: r.get(7),
            logged_out: r.get(8),
            logout_time: r.get(9),
        }))
    }

    async fn exec(&self, sql: &str, binds: Vec<Value>) -> Result<(), String> {
        // only fixed statements and column names are formatted in; values are bound
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
        for b in binds {
            q = match b {
                Value::Null => q.bind(None::<String>),
                Value::Number(n) if n.is_i64() => q.bind(n.as_i64().unwrap()),
                Value::Number(n) => q.bind(n.as_f64().unwrap()),
                Value::String(s) => q.bind(s),
                Value::Bool(b) => q.bind(b as i64),
                other => q.bind(other.to_string()),
            };
        }
        q.execute(&self.db.pool).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Refuses a save whose playtime went backwards.
    fn would_reset(&self, raw: &[u8], profile: &str, username: &str) -> bool {
        let Ok(existing) = std::fs::read(self.save_path(profile, username)) else { return false };
        let load = |b: &[u8]| Player::load("tmp", b, &self.varps, &self.invs, &self.varos, true).ok();
        matches!((load(&existing), load(raw)), (Some(a), Some(b)) if a.playtime > b.playtime)
    }

    fn write_save(&self, profile: &str, username: &str, raw: &[u8]) {
        if verify_save(raw) && !self.would_reset(raw, profile, username) {
            let path = self.save_path(profile, username);
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            if let Err(e) = std::fs::write(&path, raw) {
                tracing::error!("{username}: {e}");
            }
        } else {
            tracing::error!("{username} Invalid save file");
        }
    }

    async fn handle(&self, msg: &Value, out: &tokio::sync::mpsc::UnboundedSender<Value>) -> Result<(), String> {
        let msg = serde_json::from_value::<LoginMsg>(msg.clone()).map_err(|e| format!("[Login]: bad message: {e}"))?;
        let LoginMsg { node_id: node, profile, req, reply_to, .. } = &msg;
        let (node, profile) = (*node, profile.as_str());
        let send = |mut v: Value| {
            v["replyTo"] = json!(reply_to);
            let _ = out.send(v);
        };
        match req {
            LoginRequest::WorldStartup => {
                self.exec("UPDATE account_login SET logged_in = 0, login_time = NULL WHERE logged_in = ? AND profile = ?", vec![json!(node), json!(profile)]).await?;
            }
            LoginRequest::PlayerLogin { username, .. } => {
                let safe = to_safe_name(username);
                if !self.logins.lock().unwrap().insert(safe.clone()) {
                    send(json!({ "response": 8 }));
                    return Ok(());
                }
                let r = self.player_login(&msg, &send).await;
                self.logins.lock().unwrap().remove(&safe);
                r?;
            }
            LoginRequest::PlayerLogout { username, save: raw } => {
                self.write_save(profile, username, raw);
                let account = self.account(&username, &profile).await?;
                if let Some(a) = account.as_ref().filter(|a| a.login_account_id.is_some()) {
                    self.exec(
                        "UPDATE account_login SET logged_in = 0, login_time = NULL, logged_out = ?, logout_time = ? WHERE account_id = ? AND profile = ?",
                        vec![json!(node), json!(to_db_date(now_ms())), json!(a.id), json!(profile)],
                    )
                    .await?;
                }
                send(json!({ "response": 0 }));
                if let (Some(a), Ok(p)) = (account, Player::load(username, raw, &self.varps, &self.invs, &self.varos, true)) {
                    self.update_hiscores(&a, &p, profile).await?;
                }
            }
            LoginRequest::PlayerAutosave { username, save } => self.write_save(profile, username, save),
            LoginRequest::PlayerForceLogout { username } => {
                if let Some(a) = self.account(username, profile).await?.filter(|a| a.login_account_id.is_some()) {
                    self.exec("UPDATE account_login SET logged_in = 0, login_time = NULL WHERE account_id = ? AND profile = ?", vec![json!(a.id), json!(profile)]).await?;
                }
            }
            LoginRequest::PlayerBan { username, until, .. } | LoginRequest::PlayerMute { username, until, .. } => {
                let col = if matches!(req, LoginRequest::PlayerBan { .. }) { "banned_until" } else { "muted_until" };
                self.exec(&format!("UPDATE account SET {col} = ? WHERE username = ?"), vec![json!(to_db_date(*until)), json!(username)]).await?;
            }
        }
        Ok(())
    }

    async fn player_login(&self, msg: &LoginMsg, send: &impl Fn(Value)) -> Result<(), String> {
        let LoginRequest::PlayerLogin { socket, remote_address: remote, uid, username, password, reconnecting, has_save, .. } = &msg.req else { return Ok(()) };
        let (profile, node, uid, reconnecting, has_save) = (msg.profile.as_str(), msg.node_id, *uid, *reconnecting, *has_save);
        let node_members = msg.node_members.unwrap_or(false);
        let node_time = msg.node_time.map_or_else(now_ms, |t| t.floor() as i64);

        let banned_ip = sqlx::query("SELECT ip FROM ipban WHERE ip = ?").bind(&remote).fetch_optional(&self.db.pool).await.map_err(|e| e.to_string())?;
        if banned_ip.is_some() {
            send(json!({ "response": 7 }));
            return Ok(());
        }
        let mut account = self.account(username, profile).await?;
        if !self.config.website.registration && account.is_none() {
            let hash = bcrypt::hash(password.to_lowercase(), 10).map_err(|e| e.to_string())?;
            self.exec(
                "INSERT INTO account (username, password, registration_ip, registration_date) VALUES (?, ?, ?, ?)",
                vec![json!(username), json!(hash), json!(remote), json!(to_db_date(now_ms()))],
            )
            .await?;
            account = self.account(username, profile).await?;
        }
        let Some(mut a) = account.filter(|a| bcrypt::verify(password.to_lowercase(), &a.password).unwrap_or(false)) else {
            send(json!({ "response": 1 }));
            return Ok(());
        };
        if a.banned_until.as_deref().and_then(parse_db_date).is_some_and(|t| t > now_ms()) {
            send(json!({ "response": 5 }));
            return Ok(());
        }
        if node_members && a.members == 0 {
            if self.config.node.auto_subscribe_members {
                self.exec("UPDATE account SET members = 1 WHERE id = ?", vec![json!(a.id)]).await?;
                a.members = 1;
            } else {
                send(json!({ "response": 9 }));
                return Ok(());
            }
        }
        let session_binds = vec![json!(socket), json!(a.id), json!(profile), json!(node), json!(to_db_date(node_time)), json!(uid), json!(remote)];
        const SESSION: &str = "INSERT INTO session (uuid, account_id, profile, world, timestamp, uid, ip) VALUES (?, ?, ?, ?, ?, ?, ?)";
        let common = json!({ "account_id": a.id, "staffmodlevel": a.staffmodlevel, "muted_until": a.muted_until, "members": a.members, "messageCount": 0 });
        let with = |mut base: Value, extra: Value| {
            for (k, v) in extra.as_object().unwrap() {
                base[k] = v.clone();
            }
            base
        };
        let path = self.save_path(profile, username);
        if reconnecting && a.logged_in == Some(node) {
            self.exec(SESSION, session_binds).await?;
            if !has_save {
                let save = std::fs::read(&path).unwrap_or_default();
                if !verify_save(&save) {
                    tracing::error!("on reconnect, account_id {} had invalid save data on disk", a.id);
                    send(json!({ "response": 7 }));
                }
                send(with(common, json!({ "response": 2, "save": base64::encode(&save) })));
            } else {
                send(with(common, json!({ "response": 2 })));
            }
            return Ok(());
        } else if a.logged_in.is_some_and(|n| n != 0) {
            send(json!({ "response": 3 }));
            return Ok(());
        } else if a.staffmodlevel < 2
            && a.logged_out.is_some_and(|n| n != 0 && n != node)
            && let Some(t) = a.logout_time.as_deref().and_then(parse_db_date)
        {
            let remaining = t - (now_ms() - self.config.node.hop_time);
            if remaining > 0 {
                // a double on the wire (`1000.0`)
                send(json!({ "response": 10, "remaining": remaining as f64 }));
                return Ok(());
            }
        }
        self.exec(SESSION, session_binds).await?;
        if !path.exists() {
            if a.logout_time.is_some() {
                tracing::error!("on login, account_id {} had no save data on disk!", a.id);
                send(json!({ "response": 7 }));
                return Ok(());
            }
            send(json!({ "response": 4, "account_id": a.id, "staffmodlevel": a.staffmodlevel, "muted_until": a.muted_until, "messageCount": 0 }));
        } else {
            let save = std::fs::read(&path).unwrap_or_default();
            if !verify_save(&save) {
                tracing::error!("on login, account_id {} had invalid save data on disk!", a.id);
                send(json!({ "response": 7 }));
                return Ok(());
            }
            send(with(common, json!({ "response": 0, "save": base64::encode(&save) })));
        }
        let now = to_db_date(now_ms());
        if a.login_account_id.is_some() {
            self.exec("UPDATE account_login SET logged_in = ?, login_time = ? WHERE account_id = ? AND profile = ?", vec![json!(node), json!(now), json!(a.id), json!(profile)]).await?;
        } else {
            self.exec("INSERT INTO account_login (account_id, profile, logged_in, login_time) VALUES (?, ?, ?, ?)", vec![json!(a.id), json!(profile), json!(node), json!(now)]).await?;
        }
        Ok(())
    }

    async fn update_hiscores(&self, a: &Account, p: &Player, profile: &str) -> Result<(), String> {
        if a.staffmodlevel > 1 || a.banned_until.as_deref().and_then(parse_db_date).is_some_and(|t| t >= now_ms()) {
            return Ok(());
        }
        let (mut total_xp, mut total_level) = (0i64, 0i64);
        for i in 0..21 {
            if STAT_ENABLED[i] {
                total_xp += p.stats[i] as i64;
                total_level += p.base_levels[i] as i64;
            }
        }
        let now = to_db_date(now_ms());
        let existing = |table: &str, kind: i64| {
            let sql = format!("SELECT value FROM {table} WHERE account_id = ? AND type = ? AND profile = ?");
            async move { sqlx::query(sqlx::AssertSqlSafe(sql)).bind(a.id).bind(kind).bind(profile).fetch_optional(&self.db.pool).await.map(|r| r.map(|r| r.get::<i64, _>(0))) }
        };
        match existing("hiscore_large", 0).await.map_err(|e| e.to_string())? {
            Some(v) if v != total_xp => {
                self.exec("UPDATE hiscore_large SET type = 0, level = ?, value = ?, date = ? WHERE account_id = ? AND type = 0 AND profile = ?", vec![json!(total_level), json!(total_xp), json!(now), json!(a.id), json!(profile)]).await?;
            }
            Some(_) => {}
            None => {
                self.exec("INSERT INTO hiscore_large (account_id, profile, type, level, value) VALUES (?, ?, 0, ?, ?)", vec![json!(a.id), json!(profile), json!(total_level), json!(total_xp)]).await?;
            }
        }
        for st in 0..21 {
            if !STAT_ENABLED[st] || p.base_levels[st] < 15 {
                continue;
            }
            let kind = st as i64 + 1;
            let (level, value) = (p.base_levels[st] as i64, p.stats[st] as i64);
            match existing("hiscore", kind).await.map_err(|e| e.to_string())? {
                Some(v) if v != value => {
                    self.exec("UPDATE hiscore SET type = ?, level = ?, value = ?, date = ? WHERE account_id = ? AND type = ? AND profile = ?", vec![json!(kind), json!(level), json!(value), json!(now), json!(a.id), json!(kind), json!(profile)]).await?;
                }
                Some(_) => {}
                None => {
                    self.exec("INSERT INTO hiscore (account_id, profile, type, level, value) VALUES (?, ?, ?, ?, ?)", vec![json!(a.id), json!(profile), json!(kind), json!(level), json!(value)]).await?;
                }
            }
        }
        Ok(())
    }
}
