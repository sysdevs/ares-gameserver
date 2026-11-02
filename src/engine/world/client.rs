//! Client socket buffering, the login handshake (`World.onClientData`)
//! and login replies (`World.onLoginMessage`).
//!
//! The World never touches a socket: bytes in arrive through
//! [`World::client_data`], and everything it sends, closes or terminates is
//! queued on `World.outbox` for the transport (or a replay) to drain.

use super::World;
use crate::engine::entity::Player;
use crate::io::isaac::Isaac;
use crate::io::packet::Packet;
use crate::link::login::{LoginReply, LoginRequest};
use crate::util::jstring::to_safe_name;

pub type ClientId = u32;

/// What the World asks the transport to do with a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    Data(Vec<u8>),
    Close,
    Terminate,
}

/// `ClientSocket.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    Login,
    Game,
    OnDemand,
    Closed,
}

/// `ClientSocket`: the input buffer and protocol state of one connection.
pub struct ClientSocket {
    pub id: ClientId,
    pub uuid: String,
    pub remote_address: String,
    pub state: ClientState,
    input: Vec<u8>,
    pub opcode: i32,
    pub waiting: i32,
    pub encryptor: Option<Isaac>,
    pub decryptor: Option<Isaac>,
    /// `out.pos` after the last write: the size of the last framed message.
    pub out_pos: usize,
}

/// `ClientSocket.in`: `Packet.alloc(65535)`.
const IN_CAPACITY: usize = 65535;

impl ClientSocket {
    pub fn available(&self) -> usize {
        self.input.len()
    }

    pub fn remaining(&self) -> usize {
        IN_CAPACITY - self.input.len()
    }

    /// `read(dest, 0, length)`: the oldest `length` bytes, if buffered.
    pub fn read(&mut self, length: usize) -> Option<Vec<u8>> {
        if self.input.len() < length {
            return None;
        }
        Some(self.input.drain(..length).collect())
    }
}

impl World {
    /// A new connection (`new TcpClientSocket`/`WSClientSocket`).
    pub fn connect(&mut self, remote_address: &str, uuid: String) -> ClientId {
        self.next_client += 1;
        let id = self.next_client;
        self.clients.insert(id, ClientSocket { id, uuid, remote_address: remote_address.into(), state: ClientState::Login, input: Vec::new(), opcode: -1, waiting: 0, encryptor: None, decryptor: None, out_pos: 0 });
        id
    }

    /// A connection whose id the transport chose.
    pub fn connect_as(&mut self, id: ClientId, remote_address: &str, uuid: String) {
        self.clients.insert(id, ClientSocket { id, uuid, remote_address: remote_address.into(), state: ClientState::Login, input: Vec::new(), opcode: -1, waiting: 0, encryptor: None, decryptor: None, out_pos: 0 });
    }

    /// `client.read(dest, 0, length)` for other protocols (on-demand).
    pub fn client_read(&mut self, id: ClientId, length: usize) -> Option<Vec<u8>> {
        self.clients.get_mut(&id)?.read(length)
    }

    pub fn client(&self, id: ClientId) -> Option<&ClientSocket> {
        self.clients.get(&id)
    }

    pub fn send(&mut self, id: ClientId, data: Vec<u8>) {
        self.outbox.push((id, Outgoing::Data(data)));
    }

    /// `close()`: ends the connection after flushing.
    pub fn close(&mut self, id: ClientId) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.state = ClientState::Closed;
        }
        self.outbox.push((id, Outgoing::Close));
    }

    /// `terminate()`: drops the connection.
    pub fn terminate(&mut self, id: ClientId) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.state = ClientState::Closed;
        }
        self.outbox.push((id, Outgoing::Terminate));
    }

    /// The transport saw the connection close (`socket.on('close')`): the
    /// player keeps playing without a client until it times out.
    pub fn client_closed(&mut self, id: ClientId) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.state = ClientState::Closed;
        }
        for slot in 1..self.players.len() as i32 {
            if let Some(p) = self.player_mut(slot)
                && p.client == Some(id)
            {
                p.client = None;
            }
        }
        for p in self.new_players.iter_mut() {
            if p.client == Some(id) {
                p.client = None;
            }
        }
    }

    /// `socket.on('data')`: buffer and, while logging in, run the handshake.
    /// A packet read past the buffer (a garbled login block) panics; the
    /// panic is caught and only this client is terminated.
    pub fn client_data(&mut self, id: ClientId, data: &[u8]) {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.client_data_inner(id, data)));
        if r.is_err() {
            tracing::warn!("client {id}: malformed input, terminated");
            self.terminate(id);
        }
    }

    fn client_data_inner(&mut self, id: ClientId, data: &[u8]) {
        self.stats.bytes_in += data.len();
        let Some(c) = self.clients.get_mut(&id) else { return };
        if c.state == ClientState::Closed || c.remaining() == 0 {
            self.terminate(id);
            return;
        }
        if data.len() + c.input.len() > IN_CAPACITY {
            self.close(id);
            return;
        }
        c.input.extend_from_slice(data);
        if c.state == ClientState::Login {
            self.on_client_data(id);
        }
    }

    /// `World.onClientData(client)`: one login-protocol message.
    fn on_client_data(&mut self, id: ClientId) {
        let c = self.clients.get_mut(&id).expect("client");
        if c.state != ClientState::Login || c.available() < 1 {
            return;
        }
        if c.opcode == -1 {
            c.opcode = c.read(1).unwrap()[0] as i32;
            c.waiting = match c.opcode {
                14 => 1,
                16 | 18 => -1,
                _ => 0,
            };
        }
        if c.waiting == -1 {
            c.waiting = c.read(1).map_or(0, |b| b[0] as i32);
        } else if c.waiting == -2 {
            c.waiting = c.read(2).map_or(0, |b| ((b[0] as i32) << 8) | b[1] as i32);
        }
        if (c.available() as i32) < c.waiting {
            return;
        }
        let body = c.read(c.waiting.max(0) as usize).unwrap_or_default();
        let opcode = c.opcode;
        let mut buf = Packet::alloc(1);
        buf.data[..body.len()].copy_from_slice(&body);
        match opcode {
            14 => {
                self.send(id, vec![0; 8]);
                let _login_server = buf.g1();
                self.send(id, vec![0]);
                let mut seed = Packet::with_len(8);
                seed.p4((self.math_random() * 0x00ff_ffff as f64).floor() as i64);
                seed.p4((self.math_random() * 0xffff_ffffu32 as f64).floor() as i64);
                self.send(id, seed.data);
            }
            16 | 18 => {
                if !self.login_request(id, opcode, &mut buf) {
                    return;
                }
            }
            15 => {
                self.clients.get_mut(&id).unwrap().state = ClientState::OnDemand;
                self.send(id, vec![0; 8]);
            }
            _ => self.terminate(id),
        }
        if let Some(c) = self.clients.get_mut(&id) {
            c.opcode = -1;
        }
    }

    fn reject(&mut self, id: ClientId, code: u8) -> bool {
        self.send(id, vec![code]);
        self.close(id);
        false
    }

    /// The opcode 16/18 body: false when the connection was rejected (the
    /// opcode is then left set).
    fn login_request(&mut self, id: ClientId, opcode: i32, buf: &mut Packet) -> bool {
        let mut rev = buf.g1();
        if rev == 0xff {
            rev = buf.g2();
        }
        if rev != self.revision {
            return self.reject(id, 6);
        }
        let info = buf.g1();
        let low_memory = info & 0x1 != 0;
        let mut crcs = vec![0; 36];
        buf.gdata(&mut crcs, 0, 36);
        if self.crc32 != Packet::getcrc(&crcs, 0, 36) {
            return self.reject(id, 6);
        }
        let Some(key) = self.rsa.as_ref() else { return self.reject(id, 6) };
        buf.rsadec(key);
        if buf.g1() != 10 {
            return self.reject(id, 6);
        }
        let mut seed = [0i32; 4];
        for s in seed.iter_mut() {
            *s = buf.g4s();
        }
        let decryptor = Isaac::new(&seed);
        for s in seed.iter_mut() {
            *s = s.wrapping_add(50);
        }
        let encryptor = Isaac::new(&seed);
        let c = self.clients.get_mut(&id).unwrap();
        c.decryptor = Some(decryptor);
        c.encryptor = Some(encryptor);
        let uid = buf.g4s();
        let username = buf.gjstr(10);
        let password = buf.gjstr(10);
        let units = |s: &str| s.encode_utf16().count();
        if units(&username) < 1 || units(&username) > 12 || units(&password) < 1 || units(&password) > 20 {
            return self.reject(id, 3);
        }
        if self.player_count() > self.max_connected {
            return self.reject(id, 7);
        }
        if self.logout_requests.contains_key(&username) {
            return self.reject(id, 5);
        }
        let c = self.clients.get(&id).unwrap();
        let (uuid, remote) = (c.uuid.clone(), c.remote_address.clone());
        let has_save = opcode == 18 && self.player_by_username(&username).is_some();
        self.login_requests.insert(uuid.clone(), id);
        self.login_out.push(LoginRequest::PlayerLogin { socket: uuid, remote_address: remote, username: to_safe_name(&username), password, uid, low_memory, reconnecting: opcode == 18, has_save });
        true
    }

    /// `World.onLoginMessage(msg)`.
    pub fn on_login_reply(&mut self, reply: LoginReply) {
        match reply {
            LoginReply::PlayerLogout { username, success } => {
                if success {
                    self.logout_requests.shift_remove(&username);
                }
            }
            LoginReply::PlayerLogin { socket, username, low_memory, reconnecting, reply, staffmodlevel, save, members, muted_until, message_count, remaining } => {
                let Some(id) = self.login_requests.remove(&socket) else { return };
                let code = match reply {
                    -1 => Some(8),
                    1 => Some(3),
                    3 => Some(5),
                    5 => Some(4),
                    6 => Some(9),
                    7 => Some(11),
                    8 => Some(16),
                    9 => Some(12),
                    _ => None,
                };
                if let Some(code) = code {
                    self.reject(id, code);
                    return;
                }
                if reply == 10 {
                    let secs = (remaining.unwrap_or(0.0) / 1000.0).min(255.0);
                    self.send(id, vec![21, secs as u8]);
                    self.close(id);
                    return;
                }
                let save = save.unwrap_or_default();
                let loaded = Player::load(&username, &save, &self.configs.varps, &self.configs.invs, &self.configs.varos, self.client_routefinder);
                let mut player = match loaded {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("{username} {e}");
                        self.reject(id, 13);
                        self.login_out.push(LoginRequest::PlayerForceLogout { username });
                        return;
                    }
                };
                let tick = self.current_tick;
                player.last_connected = tick;
                player.last_response = tick;
                player.client = Some(id);
                player.session = self.clients.get(&id).map_or_else(String::new, |c| c.uuid.clone());
                player.reconnecting = reconnecting;
                player.staff_mod_level = staffmodlevel.unwrap_or(0);
                player.low_memory = low_memory;
                player.muted_until = muted_until;
                player.members = members;
                player.message_count = message_count.unwrap_or(0);
                if self.logout_requests.contains_key(&username) {
                    self.reject(id, 5);
                    return;
                }
                if !self.map.members && !self.map.is_free_to_play(player.e.x, player.e.z) {
                    if player.members {
                        self.reject(id, 17);
                        self.login_out.push(LoginRequest::PlayerForceLogout { username });
                        return;
                    }
                    // ponytail: a teleport of the unattached player; only its position matters here
                    player.e.x = 3221;
                    player.e.z = 3219;
                    player.e.level = 0;
                }
                self.new_players.push(player);
                if let Some(c) = self.clients.get_mut(&id) {
                    c.state = ClientState::Game;
                }
            }
        }
    }
}
