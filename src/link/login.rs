//! Login-thread messages and its `login.enabled=false`
//! branch, which reads and writes `data/players/<profile>/<user>.sav`.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// World → login thread, and the body of a [`LoginMsg`]: saves go out as
/// base64 and ban/mute times as `DATETIME` strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginRequest {
    WorldStartup,
    PlayerLogin {
        socket: String,
        #[serde(rename = "remoteAddress")]
        remote_address: String,
        uid: i32,
        username: String,
        password: String,
        /// The World's own; not sent.
        #[serde(skip)]
        low_memory: bool,
        reconnecting: bool,
        #[serde(rename = "hasSave")]
        has_save: bool,
    },
    PlayerLogout {
        username: String,
        #[serde(with = "b64")]
        save: Vec<u8>,
    },
    PlayerAutosave {
        username: String,
        #[serde(with = "b64")]
        save: Vec<u8>,
    },
    PlayerForceLogout { username: String },
    PlayerBan {
        staff: String,
        username: String,
        #[serde(with = "crate::server::db::date")]
        until: i64,
    },
    PlayerMute {
        staff: String,
        username: String,
        #[serde(with = "crate::server::db::date")]
        until: i64,
    },
}

/// World → login server: a request with the node's identity.
/// `nodeTime` is a double on the wire (`1700000000000.0`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoginMsg {
    #[serde(rename = "nodeId", default)]
    pub node_id: i64,
    #[serde(rename = "nodeTime")]
    pub node_time: Option<f64>,
    #[serde(default)]
    pub profile: String,
    #[serde(flatten)]
    pub req: LoginRequest,
    /// `player_login` only.
    #[serde(rename = "nodeMembers", default, skip_serializing_if = "Option::is_none")]
    pub node_members: Option<bool>,
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

mod b64 {
    use crate::util::base64;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::encode(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Ok(base64::decode(&String::deserialize(d)?))
    }
}

/// Login thread → World.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginReply {
    PlayerLogin {
        socket: String,
        username: String,
        #[serde(rename = "lowMemory")]
        low_memory: bool,
        reconnecting: bool,
        reply: i32,
        staffmodlevel: Option<i32>,
        save: Option<Vec<u8>>,
        members: bool,
        muted_until: Option<i64>,
        #[serde(rename = "messageCount")]
        message_count: Option<i32>,
        remaining: Option<f64>,
    },
    PlayerLogout { username: String, success: bool },
}

/// The file branch of `LoginThread` (`login.enabled=false`): replies to
/// a request, saving to `<dir>/<username>.sav`. Staff level 4 outside
/// production. A write failure is returned.
pub fn inline(dir: &Path, production: bool, members: bool, req: &LoginRequest) -> std::io::Result<Option<LoginReply>> {
    let file = |u: &str| dir.join(format!("{u}.sav"));
    Ok(match req {
        LoginRequest::PlayerLogin { socket, username, low_memory, reconnecting, .. } => {
            std::fs::create_dir_all(dir)?;
            let save = std::fs::read(file(username)).ok();
            Some(LoginReply::PlayerLogin {
                socket: socket.clone(),
                username: username.clone(),
                low_memory: *low_memory,
                reconnecting: *reconnecting,
                reply: if save.is_some() { 0 } else { 4 },
                staffmodlevel: Some(if production { 0 } else { 4 }),
                save,
                members,
                muted_until: None,
                message_count: None,
                remaining: None,
            })
        }
        LoginRequest::PlayerLogout { username, save } => {
            std::fs::create_dir_all(dir)?;
            std::fs::write(file(username), save)?;
            Some(LoginReply::PlayerLogout { username: username.clone(), success: true })
        }
        LoginRequest::PlayerAutosave { username, save } => {
            std::fs::create_dir_all(dir)?;
            std::fs::write(file(username), save)?;
            None
        }
        LoginRequest::WorldStartup | LoginRequest::PlayerForceLogout { .. } | LoginRequest::PlayerBan { .. } | LoginRequest::PlayerMute { .. } => None,
    })
}
