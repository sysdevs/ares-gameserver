//! Friend-link messages: the World's queue for the friend thread, and the
//! friend-server protocol both ways. The protocol's `type` is a number (the
//! opcode); the enums carry it as the decimal string serde's tag needs, and
//! [`to_wire`] / [`from_wire`] swap it for the number at the socket.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// World → friend thread (`FriendThread` messages).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FriendOut {
    PlayerLogin { username: String, chat_mode_private: i32, staff_lvl: i32 },
    PlayerLogout { username: String },
    PlayerFriendslistAdd { username: String, target: Name37 },
    PlayerFriendslistRemove { username: String, target: Name37 },
    PlayerIgnorelistAdd { username: String, target: Name37 },
    PlayerIgnorelistRemove { username: String, target: Name37 },
    PlayerChatSetmode { username: String, chat_mode_private: i32 },
    PrivateMessage { username: String, staff_lvl: i32, pm_id: i32, target: Name37, message: String, coord: i32 },
    PublicMessage {
        #[serde(rename = "session_uuid")]
        session_uuid: String,
        coord: i32,
        chat: String,
    },
}

/// World → friend server (`FriendClientProtocol`), and the admin relays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum FriendRequest {
    #[serde(rename = "0")]
    WorldConnect {
        world: i64,
        #[serde(default)]
        profile: String,
    },
    #[serde(rename = "1")]
    FriendlistAdd { world: i64, username37: Name37, target_username37: Name37 },
    #[serde(rename = "2")]
    FriendlistDel { world: i64, username37: Name37, target_username37: Name37 },
    #[serde(rename = "3")]
    IgnorelistAdd { world: i64, username37: Name37, target_username37: Name37 },
    #[serde(rename = "4")]
    IgnorelistDel { world: i64, username37: Name37, target_username37: Name37 },
    #[serde(rename = "5")]
    PlayerLogin { world: i64, username37: Name37, private_chat: i64, staff_lvl: i64 },
    #[serde(rename = "6")]
    PlayerLogout { world: i64, username37: Name37 },
    #[serde(rename = "7")]
    PlayerChatSetmode { world: i64, username37: Name37, private_chat: i64 },
    /// `nodeTime` is a double on the wire (`1700000000000.0`).
    #[serde(rename = "8")]
    PrivateMessage { world: i64, profile: String, node_time: f64, username37: Name37, target_username37: Name37, staff_lvl: i64, pm_id: i64, chat: String, coord: i64 },
    #[serde(rename = "9")]
    PublicChatLog {
        node_id: i64,
        profile: String,
        node_time: Option<f64>,
        #[serde(rename = "session_uuid")]
        session_uuid: String,
        coord: i64,
        chat: String,
    },
    /// `muted_until` is passed through as sent: a date string or ms.
    #[serde(rename = "10")]
    RelayMute {
        node_id: i64,
        username: String,
        #[serde(rename = "muted_until")]
        muted_until: Option<Value>,
    },
    #[serde(rename = "11")]
    RelayKick { node_id: i64, username: String },
    #[serde(rename = "12")]
    RelayShutdown { node_id: i64, duration: i64 },
    #[serde(rename = "13")]
    RelayBroadcast { node_id: i64, broadcast: String },
    #[serde(rename = "14")]
    RelayTrack { node_id: i64, username: String, state: bool },
    #[serde(rename = "15")]
    RelayReload { node_id: i64 },
    #[serde(rename = "16")]
    RelayClearlogins { node_id: i64 },
    #[serde(rename = "17")]
    RelayClearlogouts { node_id: i64 },
    #[serde(rename = "18")]
    RelayQueuescript { node_id: i64, username: String, script_name: String },
}

/// Friend server → world (`FriendServerProtocol`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum FriendReply {
    /// `[world, name]` pairs; `null` world for an unknown one.
    #[serde(rename = "0")]
    UpdateFriendlist { username37: Name37, friends: Vec<(Option<i64>, Name37)> },
    #[serde(rename = "1")]
    UpdateIgnorelist { username37: Name37, ignored: Vec<Name37> },
    #[serde(rename = "2")]
    PrivateMessage { username37: Name37, target_username37: Name37, staff_lvl: i64, pm_id: i64, chat: String },
    #[serde(rename = "3")]
    RelayMute {
        username: String,
        #[serde(rename = "muted_until", skip_serializing_if = "Option::is_none")]
        muted_until: Option<Value>,
    },
    #[serde(rename = "4")]
    RelayKick { username: String },
    #[serde(rename = "5")]
    RelayShutdown { duration: i64 },
    #[serde(rename = "6")]
    RelayBroadcast { message: String },
    #[serde(rename = "7")]
    RelayTrack { username: String, state: bool },
    #[serde(rename = "8")]
    RelayReload,
    #[serde(rename = "9")]
    RelayClearlogins,
    #[serde(rename = "10")]
    RelayClearlogouts,
    #[serde(rename = "11")]
    RelayQueuescript { username: String, script_name: String },
}

/// A base-37 name, sent as its decimal string; a number is accepted too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Name37(pub i64);

impl Serialize for Name37 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Name37 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Num(i64),
        }
        Ok(Name37(match Raw::deserialize(d)? {
            Raw::Text(s) => s.parse().unwrap_or(0),
            Raw::Num(n) => n,
        }))
    }
}

/// The message with its numeric `type`.
pub fn to_wire<T: Serialize>(msg: &T) -> Value {
    let mut v = serde_json::to_value(msg).expect("friend message");
    let op: i64 = v["type"].as_str().and_then(|t| t.parse().ok()).expect("numeric tag");
    v["type"] = op.into();
    v
}

/// The message for a numeric `type`; `None` for an unknown or malformed one.
pub fn from_wire<T: DeserializeOwned>(mut v: Value) -> Option<T> {
    let op = v.get("type")?.as_i64()?;
    v["type"] = op.to_string().into();
    serde_json::from_value(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_tags_round_trip() {
        let m = FriendReply::UpdateFriendlist { username37: Name37(5), friends: vec![(None, Name37(7)), (Some(10), Name37(8))] };
        let v = to_wire(&m);
        assert_eq!(v.to_string(), r#"{"type":0,"username37":"5","friends":[[null,"7"],[10,"8"]]}"#);
        assert_eq!(from_wire::<FriendReply>(v), Some(m));
        assert_eq!(from_wire::<FriendReply>(serde_json::json!({ "type": 8 })), Some(FriendReply::RelayReload));
        assert_eq!(from_wire::<FriendReply>(serde_json::json!({ "type": 99 })), None);
        assert_eq!(from_wire::<FriendRequest>(serde_json::json!({ "type": 6, "world": 1, "username37": 37 })), Some(FriendRequest::PlayerLogout { world: 1, username37: Name37(37) }));
    }
}
