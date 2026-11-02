//! `World.onFriendMessage` (friend server → world), broadcasts and the
//! shutdown sequence (`rebootTimer`, `processShutdown`).

use super::World;
use crate::engine::entity::player::QueueType;
use crate::link::friend::FriendReply;
use crate::net::server::ServerMessage as M;
use crate::util::jstring::from_base37;
use serde_json::Value;

impl World {
    /// `getPlayerByUsername(name)?.write(msg)`: online or still joining.
    fn write_named(&mut self, name: &str, msg: M) -> bool {
        let Some(p) = self.player_by_username(name) else { return false };
        if let Some(cid) = p.client {
            self.write_client(cid, &msg);
        }
        true
    }

    /// `rebootTimer(duration)`.
    pub fn reboot_timer(&mut self, duration: i32) {
        self.shutdown_tick = self.current_tick + duration;
        for slot in self.player_loop() {
            let ticks = self.shutdown_tick - self.current_tick;
            self.write(slot, M::UpdateRebootTimer { ticks });
        }
    }

    /// `broadcastMes(message)`.
    pub fn broadcast_mes(&mut self, message: &str) {
        for slot in self.player_loop() {
            for line in message.split('\n') {
                self.wrapped_message_game(slot, line);
            }
        }
    }

    /// `processShutdown()`: true once the world has emptied and may exit.
    pub fn process_shutdown(&mut self) -> bool {
        for slot in self.player_loop() {
            if let Some(cid) = self.player(slot).and_then(|p| p.client) {
                self.write_client(cid, &M::Logout);
                self.close(cid);
            }
        }
        let duration = self.current_tick - self.shutdown_tick;
        if duration >= 1024 {
            for slot in self.player_loop() {
                tracing::error!("Player '{}' force removed!", self.pl(slot).username);
                self.remove_player(slot);
            }
        }
        if self.player_count() == 0 && self.logout_requests.is_empty() {
            tracing::info!("Server shutdown complete");
            return true;
        }
        if duration > 2 {
            self.tick_rate = 0.0;
        }
        false
    }

    /// `onFriendMessage({ opcode, data })`.
    pub fn on_friend_message(&mut self, msg: FriendReply) {
        match msg {
            FriendReply::UpdateFriendlist { username37, .. } | FriendReply::UpdateIgnorelist { username37, .. } if self.player_by_username(&from_base37(username37.0)).is_none() => {
                tracing::error!("FriendThread: player {} not found", from_base37(username37.0));
            }
            FriendReply::UpdateFriendlist { username37, friends } => {
                let name = from_base37(username37.0);
                for (world, f) in friends {
                    self.write_named(&name, M::UpdateFriendList { name: f.0, node_id: world.unwrap_or(0) as i32 });
                }
                self.write_named(&name, M::FriendlistLoaded { status: 2 });
            }
            FriendReply::UpdateIgnorelist { username37, ignored } => {
                let names = ignored.into_iter().map(|n| n.0).collect();
                self.write_named(&from_base37(username37.0), M::UpdateIgnoreList { names });
            }
            FriendReply::PrivateMessage { username37, target_username37, staff_lvl, pm_id, chat } => {
                let target = from_base37(target_username37.0);
                let msg = M::MessagePrivate { from: username37.0, message_id: pm_id as i32, staff_mod_level: staff_lvl as i32, msg: chat };
                if !self.write_named(&target, msg) {
                    tracing::error!("FriendThread: player {target} not found");
                }
            }
            FriendReply::RelayMute { username, muted_until } => {
                // `muted_until ? new Date(muted_until) : null`
                let until = match muted_until {
                    Some(Value::String(t)) => crate::server::db::parse_db_date(&t),
                    // `new Date(n)` truncates
                    v => v.and_then(|v| v.as_f64()).filter(|t| *t != 0.0).map(|t| t as i64),
                };
                if let Some(p) = self.player_by_username_mut(&username) {
                    p.muted_until = until;
                }
            }
            FriendReply::RelayKick { username } => {
                if let Some(p) = self.player_by_username_mut(&username) {
                    p.logging_out = true;
                    if let Some(cid) = p.client {
                        self.write_client(cid, &M::Logout);
                        self.close(cid);
                    }
                }
            }
            FriendReply::RelayBroadcast { message } => self.broadcast_mes(&message),
            FriendReply::RelayShutdown { duration } => self.reboot_timer(duration as i32),
            // ponytail: input tracking is not ported (it only feeds the logger)
            FriendReply::RelayTrack { .. } => {}
            FriendReply::RelayReload => self.reload(false),
            FriendReply::RelayClearlogins => self.login_requests.clear(),
            FriendReply::RelayClearlogouts => self.logout_requests.clear(),
            FriendReply::RelayQueuescript { username, script_name } => {
                let script = self.scripts.get_by_name(&format!("[queue,{script_name}]"));
                if let Some(p) = self.player_by_username_mut(&username)
                    && let Some(script) = script
                {
                    p.enqueue_script(script, QueueType::Normal, 0, Vec::new());
                }
            }
        }
    }
}
