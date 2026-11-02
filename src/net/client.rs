//! Client game messages: prots, models, decoders.
//!
//! Decoding reads from the one shared `NetworkPlayer.inBuf`; a decoder
//! that reads past its packet (an unterminated `ClientCheat` string) sees
//! whatever an earlier packet left there, so callers pass that shared buffer.

use crate::io::packet::Packet;
use serde::Serialize;

/// `ClientGameProt`: every opcode the client can send and its length
/// (`-1` byte-sized, `-2` short-sized, else fixed), indexed by id.
pub const CLIENT_PROTS: &[(&str, u8, i32)] = &[
    ("NO_TIMEOUT", 120, 0),
    ("IDLE_TIMER", 209, 0),
    ("EVENT_MOUSE_CLICK", 20, 4),
    ("EVENT_MOUSE_MOVE", 222, -1),
    ("EVENT_APPLET_FOCUS", 73, 1),
    ("EVENT_CAMERA_POSITION", 53, 4),
    ("ANTICHEAT_OPLOGIC1", 219, 4),
    ("ANTICHEAT_OPLOGIC2", 201, 2),
    ("ANTICHEAT_OPLOGIC3", 41, 4),
    ("ANTICHEAT_OPLOGIC4", 80, 1),
    ("ANTICHEAT_OPLOGIC5", 235, 1),
    ("ANTICHEAT_OPLOGIC6", 250, 2),
    ("ANTICHEAT_OPLOGIC7", 25, 4),
    ("ANTICHEAT_OPLOGIC8", 0, 1),
    ("ANTICHEAT_OPLOGIC9", 24, 3),
    ("ANTICHEAT_CYCLELOGIC1", 12, -1),
    ("ANTICHEAT_CYCLELOGIC2", 149, -1),
    ("ANTICHEAT_CYCLELOGIC3", 52, 1),
    ("ANTICHEAT_CYCLELOGIC4", 230, 1),
    ("ANTICHEAT_CYCLELOGIC5", 100, 0),
    ("ANTICHEAT_CYCLELOGIC6", 188, 1),
    ("ANTICHEAT_CYCLELOGIC7", 89, 0),
    ("OPOBJ1", 247, 6),
    ("OPOBJ2", 169, 6),
    ("OPOBJ3", 108, 6),
    ("OPOBJ4", 62, 6),
    ("OPOBJ5", 117, 6),
    ("OPOBJT", 91, 8),
    ("OPOBJU", 39, 12),
    ("OPNPC1", 236, 2),
    ("OPNPC2", 233, 2),
    ("OPNPC3", 223, 2),
    ("OPNPC4", 147, 2),
    ("OPNPC5", 189, 2),
    ("OPNPCT", 181, 4),
    ("OPNPCU", 150, 8),
    ("OPLOC1", 215, 6),
    ("OPLOC2", 103, 6),
    ("OPLOC3", 187, 6),
    ("OPLOC4", 157, 6),
    ("OPLOC5", 127, 6),
    ("OPLOCT", 213, 8),
    ("OPLOCU", 60, 12),
    ("OPPLAYER1", 109, 2),
    ("OPPLAYER2", 166, 2),
    ("OPPLAYER3", 196, 2),
    ("OPPLAYER4", 98, 2),
    ("OPPLAYER5", 174, 2),
    ("OPPLAYERT", 240, 4),
    ("OPPLAYERU", 36, 8),
    ("OPHELD1", 185, 6),
    ("OPHELD2", 2, 6),
    ("OPHELD3", 123, 6),
    ("OPHELD4", 216, 6),
    ("OPHELD5", 42, 6),
    ("OPHELDT", 135, 8),
    ("OPHELDU", 136, 12),
    ("INV_BUTTON1", 74, 6),
    ("INV_BUTTON2", 82, 6),
    ("INV_BUTTON3", 239, 6),
    ("INV_BUTTON4", 179, 6),
    ("INV_BUTTON5", 46, 6),
    ("IF_BUTTON", 9, 2),
    ("RESUME_PAUSEBUTTON", 72, 2),
    ("CLOSE_MODAL", 51, 0),
    ("RESUME_P_COUNTDIALOG", 102, 4),
    ("TUT_CLICKSIDE", 94, 1),
    ("MAP_BUILD_COMPLETE", 214, 0),
    ("MOVE_OPCLICK", 138, -1),
    ("REPORT_ABUSE", 137, 10),
    ("MOVE_MINIMAPCLICK", 86, -1),
    ("INV_BUTTOND", 93, 7),
    ("IGNORELIST_DEL", 101, 8),
    ("IGNORELIST_ADD", 255, 8),
    ("IDK_SAVEDESIGN", 125, 13),
    ("CHAT_SETMODE", 154, 3),
    ("MESSAGE_PRIVATE", 139, -1),
    ("FRIENDLIST_DEL", 106, 8),
    ("FRIENDLIST_ADD", 13, 8),
    ("CLIENT_CHEAT", 224, -1),
    ("MESSAGE_PUBLIC", 253, -1),
    ("MOVE_GAMECLICK", 207, -1),
];

/// `ClientGameProt.byId[id]?.length`.
pub fn prot_length(id: u8) -> Option<i32> {
    CLIENT_PROTS.iter().find(|p| p.1 == id).map(|p| p.2)
}

fn as_string<S: serde::Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Waypoint {
    pub x: i32,
    pub z: i32,
}

/// The decoded client messages.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ClientMessage {
    ChatSetMode { public_chat: i32, private_chat: i32, trade_duel: i32 },
    ClientCheat { input: String },
    CloseModal {},
    EventAppletFocus { focus: i32 },
    EventCameraPosition { pitch: i32, yaw: i32 },
    EventMouseClick { info: u32 },
    EventMouseMove { data: Vec<u8> },
    FriendListAdd { #[serde(serialize_with = "as_string")] username: i64 },
    FriendListDel { #[serde(serialize_with = "as_string")] username: i64 },
    IdkSaveDesign { gender: i32, idkit: Vec<i32>, color: Vec<i32> },
    IdleTimer {},
    IfButton { com: i32 },
    IgnoreListAdd { #[serde(serialize_with = "as_string")] username: i64 },
    IgnoreListDel { #[serde(serialize_with = "as_string")] username: i64 },
    InvButton { op: i32, obj: i32, slot: i32, com: i32 },
    InvButtonD { com: i32, slot: i32, target_slot: i32, mode: i32 },
    MessagePrivate { #[serde(serialize_with = "as_string")] username: i64, input: Vec<u8> },
    MessagePublic { input: Vec<u8>, colour: i32, effect: i32 },
    MoveClick { path: Vec<Waypoint>, ctrl_held: i32, op_click: bool },
    OpHeld { op: i32, obj: i32, slot: i32, com: i32 },
    OpHeldT { obj: i32, slot: i32, com: i32, spell_com: i32 },
    OpHeldU { obj: i32, slot: i32, com: i32, use_obj: i32, use_slot: i32, use_com: i32 },
    OpLoc { op: i32, x: i32, z: i32, loc: i32 },
    OpLocT { x: i32, z: i32, loc: i32, spell_com: i32 },
    OpLocU { x: i32, z: i32, loc: i32, use_obj: i32, use_slot: i32, use_com: i32 },
    OpNpc { op: i32, npc_slot: i32 },
    OpNpcT { npc_slot: i32, spell_com: i32 },
    OpNpcU { npc_slot: i32, use_obj: i32, use_slot: i32, use_com: i32 },
    OpObj { op: i32, x: i32, z: i32, obj: i32 },
    OpObjT { x: i32, z: i32, obj: i32, spell_com: i32 },
    OpObjU { x: i32, z: i32, obj: i32, use_obj: i32, use_slot: i32, use_com: i32 },
    OpPlayer { op: i32, player_slot: i32 },
    OpPlayerT { player_slot: i32, spell_com: i32 },
    OpPlayerU { player_slot: i32, use_obj: i32, use_slot: i32, use_com: i32 },
    ReportAbuse { #[serde(serialize_with = "as_string")] offender: i64, reason: i32, moderator_mute: bool },
    ResumePCountDialog { input: i32 },
    ResumePauseButton {},
    TutClickSide { tab: i32 },
}

/// `ClientGameProtCategory`: (id, per-tick limit).
pub const CLIENT_EVENT: (i32, i32) = (0, 20);
pub const USER_EVENT: (i32, i32) = (1, 5);
pub const RESTRICTED_EVENT: (i32, i32) = (2, 2);

impl ClientMessage {
    pub fn category(&self) -> (i32, i32) {
        use ClientMessage::*;
        match self {
            EventAppletFocus { .. } | EventCameraPosition { .. } | EventMouseClick { .. } | EventMouseMove { .. } | IdleTimer {} => CLIENT_EVENT,
            _ => USER_EVENT,
        }
    }
}

const MOVE_OPCLICK: u8 = 138;
const MOVE_MINIMAPCLICK: u8 = 86;

/// `ClientGameProtRepository.getDecoder(prot)?.decode(buf, len)`: `None`
/// for opcodes without a decoder (read and dropped by the caller).
pub fn decode(id: u8, buf: &mut Packet, len: usize) -> Option<ClientMessage> {
    use ClientMessage::*;
    let g2 = |b: &mut Packet| b.g2();
    Some(match id {
        154 => ChatSetMode { public_chat: buf.g1(), private_chat: buf.g1(), trade_duel: buf.g1() },
        224 => ClientCheat { input: buf.gjstr(10) },
        51 => CloseModal {},
        73 => EventAppletFocus { focus: buf.g1() },
        53 => EventCameraPosition { pitch: buf.g2(), yaw: buf.g2() },
        20 => EventMouseClick { info: buf.g4() },
        222 => {
            let mut data = vec![0; len];
            buf.gdata(&mut data, 0, len);
            EventMouseMove { data }
        }
        13 => FriendListAdd { username: buf.g8() },
        106 => FriendListDel { username: buf.g8() },
        125 => {
            let gender = buf.g1();
            let idkit = (0..7).map(|_| buf.g1()).map(|v| if v == 255 { -1 } else { v }).collect();
            let color = (0..5).map(|_| buf.g1()).collect();
            IdkSaveDesign { gender, idkit, color }
        }
        209 => IdleTimer {},
        9 => IfButton { com: buf.g2() },
        255 => IgnoreListAdd { username: buf.g8() },
        101 => IgnoreListDel { username: buf.g8() },
        74 | 82 | 239 | 179 | 46 => {
            let op = [74, 82, 239, 179, 46].iter().position(|&p| p == id).unwrap() as i32 + 1;
            InvButton { op, obj: g2(buf), slot: g2(buf), com: g2(buf) }
        }
        93 => InvButtonD { com: buf.g2(), slot: buf.g2(), target_slot: buf.g2(), mode: buf.g1() },
        139 => {
            let username = buf.g8();
            let n = len.saturating_sub(8);
            let input = buf.data[buf.pos..(buf.pos + n).min(buf.data.len())].to_vec();
            buf.pos += n;
            MessagePrivate { username, input }
        }
        253 => {
            let colour = buf.g1();
            let effect = buf.g1();
            let n = len.saturating_sub(2);
            let input = buf.data[buf.pos..(buf.pos + n).min(buf.data.len())].to_vec();
            buf.pos += n;
            MessagePublic { input, colour, effect }
        }
        207 | 138 | 86 => {
            let ctrl_held = buf.g1();
            let start_x = buf.g2();
            let start_z = buf.g2();
            let offset = if id == MOVE_MINIMAPCLICK { 14 } else { 0 };
            // `index <= (len - pos - offset) / 2` as a double: an odd length rounds down
            let waypoints2 = len as i64 - buf.pos as i64 - offset;
            let mut path = vec![Waypoint { x: start_x, z: start_z }];
            let mut index = 1;
            while 2 * index <= waypoints2 && index < 25 {
                path.push(Waypoint { x: start_x + buf.g1b(), z: start_z + buf.g1b() });
                index += 1;
            }
            MoveClick { path, ctrl_held, op_click: id == MOVE_OPCLICK }
        }
        185 | 2 | 123 | 216 | 42 => {
            let op = [185, 2, 123, 216, 42].iter().position(|&p| p == id).unwrap() as i32 + 1;
            OpHeld { op, obj: g2(buf), slot: g2(buf), com: g2(buf) }
        }
        135 => OpHeldT { obj: buf.g2(), slot: buf.g2(), com: buf.g2(), spell_com: buf.g2() },
        136 => OpHeldU { obj: buf.g2(), slot: buf.g2(), com: buf.g2(), use_obj: buf.g2(), use_slot: buf.g2(), use_com: buf.g2() },
        215 | 103 | 187 | 157 | 127 => {
            let op = [215, 103, 187, 157, 127].iter().position(|&p| p == id).unwrap() as i32 + 1;
            OpLoc { op, x: g2(buf), z: g2(buf), loc: g2(buf) }
        }
        213 => OpLocT { x: buf.g2(), z: buf.g2(), loc: buf.g2(), spell_com: buf.g2() },
        60 => OpLocU { x: buf.g2(), z: buf.g2(), loc: buf.g2(), use_obj: buf.g2(), use_slot: buf.g2(), use_com: buf.g2() },
        236 | 233 | 223 | 147 | 189 => {
            let op = [236, 233, 223, 147, 189].iter().position(|&p| p == id).unwrap() as i32 + 1;
            OpNpc { op, npc_slot: g2(buf) }
        }
        181 => OpNpcT { npc_slot: buf.g2(), spell_com: buf.g2() },
        150 => OpNpcU { npc_slot: buf.g2(), use_obj: buf.g2(), use_slot: buf.g2(), use_com: buf.g2() },
        247 | 169 | 108 | 62 | 117 => {
            let op = [247, 169, 108, 62, 117].iter().position(|&p| p == id).unwrap() as i32 + 1;
            OpObj { op, x: g2(buf), z: g2(buf), obj: g2(buf) }
        }
        91 => OpObjT { x: buf.g2(), z: buf.g2(), obj: buf.g2(), spell_com: buf.g2() },
        39 => OpObjU { x: buf.g2(), z: buf.g2(), obj: buf.g2(), use_obj: buf.g2(), use_slot: buf.g2(), use_com: buf.g2() },
        109 | 166 | 196 | 98 | 174 => {
            let op = [109, 166, 196, 98, 174].iter().position(|&p| p == id).unwrap() as i32 + 1;
            OpPlayer { op, player_slot: g2(buf) }
        }
        240 => OpPlayerT { player_slot: buf.g2(), spell_com: buf.g2() },
        36 => OpPlayerU { player_slot: buf.g2(), use_obj: buf.g2(), use_slot: buf.g2(), use_com: buf.g2() },
        137 => ReportAbuse { offender: buf.g8(), reason: buf.g1(), moderator_mute: buf.gbool() },
        102 => ResumePCountDialog { input: buf.g4s() },
        72 => ResumePauseButton {},
        94 => TutClickSide { tab: buf.g1() },
        _ => return None,
    })
}

/// What the client sends for opcode 16 (new login) or 18 (reconnect): the
/// RSA block (magic 10, ISAAC seeds, uid, credentials) behind the revision,
/// the low-memory flag and the first 36 bytes of the CRC buffer. For bots and
/// replays; the server side is `World::login_request`.
#[allow(clippy::too_many_arguments)]
pub fn login_packet(key: &crate::io::packet::RsaKey, crc_buffer: &[u8], seeds: &[i32; 4], uid: i32, username: &str, password: &str, low_memory: bool, reconnect: bool) -> Vec<u8> {
    let mut rsa = Packet::with_len(256);
    rsa.p1(10);
    for &s in seeds {
        rsa.p4(s as i64);
    }
    rsa.p4(uid as i64);
    rsa.pjstr(username);
    rsa.pjstr(password);
    rsa.rsaenc(key);
    let mut body = Packet::with_len(512);
    body.p1(0xff);
    body.p2(274);
    body.p1(low_memory as i64);
    body.pdata(crc_buffer, 0, 36);
    let rsa_len = rsa.pos;
    body.pdata(&rsa.data, 0, rsa_len);
    let mut out = vec![if reconnect { 18 } else { 16 }, body.pos as u8];
    out.extend_from_slice(&body.data[..body.pos]);
    out
}

/// One game packet as the client frames it: the ISAAC-shifted opcode, the
/// size prefix its prot declares, then the payload.
pub fn frame_packet(isaac: &mut crate::io::isaac::Isaac, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![(opcode as i32).wrapping_add(isaac.next_int()) as u8];
    match prot_length(opcode) {
        Some(-1) => bytes.push(payload.len() as u8),
        Some(-2) => bytes.extend_from_slice(&[(payload.len() >> 8) as u8, payload.len() as u8]),
        _ => {}
    }
    bytes.extend_from_slice(payload);
    bytes
}
