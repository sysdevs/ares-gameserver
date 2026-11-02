//! Every server-to-client game message, its
//! prot (id, length) and encoder, and `NetworkPlayer.writeInner`'s framing.
//!
//! Messages are encoded at `write()` time, so messages that reference live state
//! (`UpdateInvFull`/`UpdateInvPartial` hold the inventory) carry a snapshot
//! taken when they are built.

use crate::cache::{word_pack, wordenc::WordEnc};
use crate::engine::inventory::{Inventory, Item};
use crate::io::isaac::Isaac;
use crate::io::packet::Packet;

/// `ServerGameProt` / `ServerGameZoneProt`: id and length (`-1` byte-sized,
/// `-2` short-sized, else fixed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prot {
    pub id: u8,
    pub length: i32,
}

const fn p(id: u8, length: i32) -> Prot {
    Prot { id, length }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    CamLookAt { x: i32, z: i32, height: i32, rate: i32, rate2: i32 },
    CamMoveTo { x: i32, z: i32, height: i32, rate: i32, rate2: i32 },
    CamReset,
    CamShake { axis: i32, random: i32, amplitude: i32, rate: i32 },
    ChatFilterSettings { public_chat: i32, private_chat: i32, trade_duel: i32 },
    FriendlistLoaded { status: i32 },
    HintArrow { kind: i32, nid: i32, player_slot: i32, x: i32, z: i32, y: i32 },
    IfClose,
    IfOpenChat { component: i32 },
    IfOpenMain { component: i32 },
    IfOpenMainSide { main: i32, side: i32 },
    IfOpenOverlay { component: i32 },
    IfOpenSide { component: i32 },
    IfSetAnim { component: i32, seq: i32 },
    IfSetColour { component: i32, colour: i32 },
    IfSetHide { component: i32, hidden: bool },
    IfSetModel { component: i32, model: i32 },
    IfSetNpcHead { component: i32, npc: i32 },
    IfSetObject { component: i32, obj: i32, scale: i32 },
    IfSetPlayerHead { component: i32 },
    IfSetPosition { component: i32, x: i32, y: i32 },
    IfSetScrollPos { component: i32, y: i32 },
    IfSetTab { component: i32, tab: i32 },
    IfSetTabActive { tab: i32 },
    IfSetText { component: i32, text: String },
    LastLoginInfo { last_login_ip: i64, days_since_login: i32, days_since_recovery_change: i32, unread_message_count: i32, warn_members_in_non_members: bool },
    LocAddChange { coord: i32, loc: i32, shape: i32, angle: i32 },
    LocAnim { coord: i32, shape: i32, angle: i32, seq: i32 },
    LocDel { coord: i32, shape: i32, angle: i32 },
    LocMerge { src_x: i32, src_z: i32, shape: i32, angle: i32, loc_id: i32, start_cycle: i32, end_cycle: i32, player_slot: i32, east: i32, south: i32, west: i32, north: i32 },
    Logout,
    MapAnim { coord: i32, spotanim: i32, height: i32, delay: i32 },
    MapProjAnim { src_x: i32, src_z: i32, dst_x: i32, dst_z: i32, target: i32, spotanim: i32, src_height: i32, dst_height: i32, start_delay: i32, end_delay: i32, peak: i32, arc: i32 },
    MessageGame { msg: String },
    MessagePrivate { from: i64, message_id: i32, staff_mod_level: i32, msg: String },
    MidiJingle { id: i32, delay: i32 },
    MidiSong { id: i32 },
    MinimapToggle { minimap_type: i32 },
    NpcInfo { bytes: Vec<u8> },
    ObjAdd { coord: i32, obj: i32, count: i64 },
    ObjCount { coord: i32, obj: i32, old_count: i64, new_count: i64 },
    ObjDel { coord: i32, obj: i32 },
    ObjReveal { coord: i32, obj: i32, count: i64, receiver_id: i32 },
    PCountDialog,
    PlayerInfo { bytes: Vec<u8> },
    RebuildNormal { zone_x: i32, zone_z: i32 },
    ResetAnims,
    ResetClientVarCache,
    SetMultiway { hidden: bool },
    SetPlayerOp { op: i32, value: String, primary: i32 },
    SynthSound { synth: i32, loops: i32, delay: i32 },
    TutFlash { tab: i32 },
    TutOpen { component: i32 },
    UnsetMapFlag,
    UpdateFriendList { name: i64, node_id: i32 },
    UpdateIgnoreList { names: Vec<i64> },
    /// The first `min(capacity, component width * height)` slots, at write time.
    UpdateInvFull { component: i32, items: Vec<Option<Item>> },
    /// The dirty slots and their contents, at write time.
    UpdateInvPartial { component: i32, slots: Vec<(i32, Option<Item>)> },
    UpdateInvStopTransmit { component: i32 },
    UpdatePid { uid: i32, members: bool },
    UpdateRebootTimer { ticks: i32 },
    UpdateRunEnergy { energy: i32 },
    UpdateRunWeight { kg: i32 },
    UpdateStat { stat: i32, exp: i32, level: i32 },
    UpdateZoneFullFollows { zone_x: i32, zone_z: i32, origin_x: i32, origin_z: i32 },
    UpdateZonePartialEnclosed { zone_x: i32, zone_z: i32, origin_x: i32, origin_z: i32, data: Vec<u8> },
    UpdateZonePartialFollows { zone_x: i32, zone_z: i32, origin_x: i32, origin_z: i32 },
    VarpLarge { varp: i32, value: i32 },
    VarpSmall { varp: i32, value: i32 },
}

/// `CoordGrid.packZoneCoord`: the tile within its zone.
pub fn pack_zone_coord(x: i32, z: i32) -> i32 {
    ((x & 0x7) << 4) | (z & 0x7)
}

/// `CoordGrid.zoneOrigin`: the build-area origin tile for `pos`.
fn zone_origin(pos: i32) -> i32 {
    ((pos >> 3) - 6) << 3
}

/// An inventory slot: `id + 1` (0 = empty), then the count in one byte or
/// 255 + four bytes.
fn put_item(buf: &mut Packet, item: Option<&Item>) {
    match item {
        Some(i) => {
            buf.p2(i.id as i64 + 1);
            if i.count >= 255 {
                buf.p1(255);
                buf.p4(i.count);
            } else {
                buf.p1(i.count);
            }
        }
        None => {
            buf.p2(0);
            buf.p1(0);
        }
    }
}

impl ServerMessage {
    /// `new UpdateInvFull(component, inv)`: `com_size` is the component's
    /// `width * height` (the client crashes past it).
    pub fn inv_full(component: i32, inv: &Inventory, com_size: i32) -> Self {
        let size = (inv.capacity as i32).min(com_size).max(0) as usize;
        ServerMessage::UpdateInvFull { component, items: (0..size).map(|s| inv.get(s)).collect() }
    }

    /// `new UpdateInvPartial(component, inv, ...inv.getDirtySlots())`.
    pub fn inv_partial(component: i32, inv: &Inventory) -> Self {
        ServerMessage::UpdateInvPartial { component, slots: inv.dirty_slots().into_iter().map(|s| (s as i32, inv.get(s))).collect() }
    }

    pub fn prot(&self) -> Prot {
        use ServerMessage::*;
        match self {
            IfOpenChat { .. } => p(166, 2),
            IfOpenMainSide { .. } => p(158, 4),
            IfClose => p(171, 0),
            IfSetTab { .. } => p(215, 3),
            IfSetTabActive { .. } => p(241, 1),
            IfOpenMain { .. } => p(211, 2),
            IfOpenSide { .. } => p(16, 2),
            IfOpenOverlay { .. } => p(240, 2),
            IfSetColour { .. } => p(183, 4),
            IfSetHide { .. } => p(10, 3),
            IfSetObject { .. } => p(28, 6),
            IfSetModel { .. } => p(129, 4),
            IfSetAnim { .. } => p(134, 4),
            IfSetPlayerHead { .. } => p(192, 2),
            IfSetText { .. } => p(44, -2),
            IfSetNpcHead { .. } => p(142, 4),
            IfSetPosition { .. } => p(77, 6),
            IfSetScrollPos { .. } => p(54, 4),
            TutFlash { .. } => p(90, 1),
            TutOpen { .. } => p(130, 2),
            UpdateInvStopTransmit { .. } => p(227, 2),
            UpdateInvFull { .. } => p(106, -2),
            UpdateInvPartial { .. } => p(172, -2),
            CamLookAt { .. } => p(233, 6),
            CamShake { .. } => p(64, 4),
            CamMoveTo { .. } => p(200, 6),
            CamReset => p(101, 0),
            NpcInfo { .. } => p(197, -2),
            PlayerInfo { .. } => p(167, -2),
            FriendlistLoaded { .. } => p(185, 1),
            MessageGame { .. } => p(161, -1),
            UpdateIgnoreList { .. } => p(3, -2),
            ChatFilterSettings { .. } => p(114, 3),
            MessagePrivate { .. } => p(235, -1),
            UpdateFriendList { .. } => p(247, 9),
            UnsetMapFlag => p(115, 0),
            UpdateRunWeight { .. } => p(67, 2),
            HintArrow { .. } => p(156, 6),
            UpdateRebootTimer { .. } => p(89, 2),
            UpdateStat { .. } => p(105, 6),
            UpdateRunEnergy { .. } => p(83, 1),
            ResetAnims => p(47, 0),
            UpdatePid { .. } => p(133, 3),
            LastLoginInfo { .. } => p(91, 10),
            Logout => p(88, 0),
            PCountDialog => p(210, 0),
            SetMultiway { .. } => p(207, 1),
            SetPlayerOp { .. } => p(17, -1),
            MinimapToggle { .. } => p(194, 1),
            RebuildNormal { .. } => p(231, 4),
            VarpSmall { .. } => p(203, 3),
            VarpLarge { .. } => p(245, 6),
            ResetClientVarCache => p(190, 0),
            SynthSound { .. } => p(34, 5),
            MidiSong { .. } => p(23, 2),
            MidiJingle { .. } => p(15, 4),
            UpdateZonePartialFollows { .. } => p(32, 2),
            UpdateZoneFullFollows { .. } => p(153, 2),
            UpdateZonePartialEnclosed { .. } => p(195, -2),
            LocMerge { .. } => p(176, 14),
            LocAnim { .. } => p(48, 4),
            ObjDel { .. } => p(52, 3),
            ObjReveal { .. } => p(219, 7),
            LocAddChange { .. } => p(138, 4),
            MapProjAnim { .. } => p(107, 15),
            LocDel { .. } => p(173, 2),
            ObjCount { .. } => p(95, 7),
            MapAnim { .. } => p(85, 6),
            ObjAdd { .. } => p(81, 5),
        }
    }

    /// Zone messages (`ServerGameZoneMessage`) can be enclosed in an
    /// `UpdateZonePartialEnclosed` block.
    pub fn is_zone(&self) -> bool {
        use ServerMessage::*;
        matches!(self, LocMerge { .. } | LocAnim { .. } | ObjDel { .. } | ObjReveal { .. } | LocAddChange { .. } | MapProjAnim { .. } | LocDel { .. } | ObjCount { .. } | MapAnim { .. } | ObjAdd { .. })
    }

    /// The payload (`encoder.encode`). `wordenc` filters private messages.
    pub fn encode(&self, buf: &mut Packet, wordenc: &WordEnc) {
        use ServerMessage::*;
        let b = buf;
        let i = |v: i32| v as i64;
        match self {
            CamLookAt { x, z, height, rate, rate2 } | CamMoveTo { x, z, height, rate, rate2 } => {
                b.p1(i(*x));
                b.p1(i(*z));
                b.p2(i(*height));
                b.p1(i(*rate));
                b.p1(i(*rate2));
            }
            CamShake { axis, random, amplitude, rate } => {
                for v in [axis, random, amplitude, rate] {
                    b.p1(i(*v));
                }
            }
            ChatFilterSettings { public_chat, private_chat, trade_duel } => {
                for v in [public_chat, private_chat, trade_duel] {
                    b.p1(i(*v));
                }
            }
            FriendlistLoaded { status } => b.p1(i(*status)),
            HintArrow { kind, nid, player_slot, x, z, y } => match *kind {
                1 => {
                    b.p1(1);
                    b.p2(i(*nid));
                    b.p2(0);
                    b.p1(0);
                }
                2..=6 => {
                    b.p1(i(*kind));
                    b.p2(i(*x));
                    b.p2(i(*z));
                    b.p1(i(*y));
                }
                10 => {
                    b.p1(10);
                    b.p2(i(*player_slot));
                    b.p2(0);
                    b.p1(0);
                }
                -1 => {
                    b.p1(-1);
                    b.p2(0);
                    b.p2(0);
                    b.p1(0);
                }
                _ => {}
            },
            IfOpenChat { component } | IfOpenMain { component } | IfOpenOverlay { component } | IfOpenSide { component } | IfSetPlayerHead { component } | TutOpen { component } | UpdateInvStopTransmit { component } => b.p2(i(*component)),
            IfOpenMainSide { main, side } => {
                b.p2(i(*main));
                b.p2(i(*side));
            }
            IfSetAnim { component, seq: v } | IfSetColour { component, colour: v } | IfSetModel { component, model: v } | IfSetNpcHead { component, npc: v } | IfSetScrollPos { component, y: v } => {
                b.p2(i(*component));
                b.p2(i(*v));
            }
            IfSetHide { component, hidden } => {
                b.p2(i(*component));
                b.pbool(*hidden);
            }
            IfSetObject { component, obj: v1, scale: v2 } | IfSetPosition { component, x: v1, y: v2 } => {
                b.p2(i(*component));
                b.p2(i(*v1));
                b.p2(i(*v2));
            }
            IfSetTab { component, tab } => {
                b.p2(i(*component));
                b.p1(i(*tab));
            }
            IfSetTabActive { tab } | TutFlash { tab } => b.p1(i(*tab)),
            IfSetText { component, text } => {
                b.p2(i(*component));
                b.pjstr(text);
            }
            LastLoginInfo { last_login_ip, days_since_login, days_since_recovery_change, unread_message_count, warn_members_in_non_members } => {
                b.p4(*last_login_ip);
                b.p2(i(*days_since_login));
                b.p1(i(*days_since_recovery_change));
                b.p2(i(*unread_message_count));
                b.pbool(*warn_members_in_non_members);
            }
            LocAddChange { coord, loc, shape, angle } => {
                b.p1(i(*coord));
                b.p1(i((shape << 2) | (angle & 0x3)));
                b.p2(i(*loc));
            }
            LocAnim { coord, shape, angle, seq } => {
                b.p1(i(*coord));
                b.p1(i((shape << 2) | (angle & 0x3)));
                b.p2(i(*seq));
            }
            LocDel { coord, shape, angle } => {
                b.p1(i(*coord));
                b.p1(i((shape << 2) | (angle & 0x3)));
            }
            LocMerge { src_x, src_z, shape, angle, loc_id, start_cycle, end_cycle, player_slot, east, south, west, north } => {
                b.p1(i(pack_zone_coord(*src_x, *src_z)));
                b.p1(i((shape << 2) | (angle & 0x3)));
                b.p2(i(*loc_id));
                b.p2(i(*start_cycle));
                b.p2(i(*end_cycle));
                b.p2(i(*player_slot));
                b.p1(i(east - src_x));
                b.p1(i(south - src_z));
                b.p1(i(west - src_x));
                b.p1(i(north - src_z));
            }
            MapAnim { coord, spotanim, height, delay } => {
                b.p1(i(*coord));
                b.p2(i(*spotanim));
                b.p1(i(*height));
                b.p2(i(*delay));
            }
            MapProjAnim { src_x, src_z, dst_x, dst_z, target, spotanim, src_height, dst_height, start_delay, end_delay, peak, arc } => {
                b.p1(i(pack_zone_coord(*src_x, *src_z)));
                b.p1(i(dst_x - src_x));
                b.p1(i(dst_z - src_z));
                b.p2(i(*target));
                b.p2(i(*spotanim));
                b.p1(i(*src_height));
                b.p1(i(*dst_height));
                b.p2(i(*start_delay));
                b.p2(i(*end_delay));
                b.p1(i(*peak));
                b.p1(i(*arc));
            }
            MessageGame { msg } => b.pjstr(msg),
            MessagePrivate { from, message_id, staff_mod_level, msg } => {
                b.p8(*from);
                b.p4(i(*message_id));
                b.p1(i((*staff_mod_level).min(3)));
                word_pack::pack(b, &wordenc.filter(msg));
            }
            MidiJingle { id, delay } => {
                b.p2(i(*id));
                b.p2(i(*delay));
            }
            MidiSong { id } => b.p2(i(*id)),
            MinimapToggle { minimap_type } => b.p1(i(*minimap_type)),
            NpcInfo { bytes } | PlayerInfo { bytes } => b.pdata(bytes, 0, bytes.len()),
            ObjAdd { coord, obj, count } => {
                b.p1(i(*coord));
                b.p2(i(*obj));
                b.p2((*count).min(65535));
            }
            ObjCount { coord, obj, old_count, new_count } => {
                b.p1(i(*coord));
                b.p2(i(*obj));
                b.p2((*old_count).min(65535));
                b.p2((*new_count).min(65535));
            }
            ObjDel { coord, obj } => {
                b.p1(i(*coord));
                b.p2(i(*obj));
            }
            ObjReveal { coord, obj, count, receiver_id } => {
                b.p1(i(*coord));
                b.p2(i(*obj));
                b.p2((*count).min(65535));
                b.p2(i(*receiver_id));
            }
            RebuildNormal { zone_x, zone_z } => {
                b.p2(i(*zone_x));
                b.p2(i(*zone_z));
            }
            SetMultiway { hidden } => b.pbool(*hidden),
            SetPlayerOp { op, value, primary } => {
                b.p1(i(*op));
                b.p1(i(*primary));
                b.pjstr(value);
            }
            SynthSound { synth, loops, delay } => {
                b.p2(i(*synth));
                b.p1(i(*loops));
                b.p2(i(*delay));
            }
            UpdateFriendList { name, node_id } => {
                b.p8(*name);
                b.p1(i(*node_id));
            }
            UpdateIgnoreList { names } => {
                for n in names {
                    b.p8(*n);
                }
            }
            UpdateInvFull { component, items } => {
                // only up to the last slot in use; the client clears the rest
                let max = items.iter().rposition(Option::is_some).map_or(0, |s| s + 1);
                b.p2(i(*component));
                b.p1(max as i64);
                for item in &items[..max] {
                    put_item(b, item.as_ref());
                }
            }
            UpdateInvPartial { component, slots } => {
                b.p2(i(*component));
                for (slot, item) in slots {
                    b.p1(i(*slot));
                    put_item(b, item.as_ref());
                }
            }
            UpdatePid { uid, members } => {
                b.p2(i(*uid));
                b.pbool(*members);
            }
            UpdateRebootTimer { ticks } => b.p2(i(*ticks)),
            UpdateRunEnergy { energy } => b.p1(i(energy / 100)),
            UpdateRunWeight { kg } => b.p2(i(*kg)),
            UpdateStat { stat, exp, level } => {
                b.p1(i(*stat));
                b.p4(i(exp / 10));
                b.p1(i(*level));
            }
            UpdateZoneFullFollows { zone_x, zone_z, origin_x, origin_z } | UpdateZonePartialFollows { zone_x, zone_z, origin_x, origin_z } => {
                b.p1(i((zone_x << 3) - zone_origin(*origin_x)));
                b.p1(i((zone_z << 3) - zone_origin(*origin_z)));
            }
            UpdateZonePartialEnclosed { zone_x, zone_z, origin_x, origin_z, data } => {
                b.p1(i((zone_x << 3) - zone_origin(*origin_x)));
                b.p1(i((zone_z << 3) - zone_origin(*origin_z)));
                b.pdata(data, 0, data.len());
            }
            VarpLarge { varp, value } => {
                b.p2(i(*varp));
                b.p4(i(*value));
            }
            VarpSmall { varp, value } => {
                b.p2(i(*varp));
                b.p1(i(*value));
            }
            CamReset | IfClose | Logout | PCountDialog | ResetAnims | ResetClientVarCache | UnsetMapFlag => {}
        }
    }

    /// `ServerGameZoneMessageEncoder.enclose`: prot id then payload, for the
    /// shared `UpdateZonePartialEnclosed` block.
    pub fn enclose(&self, buf: &mut Packet, wordenc: &WordEnc) {
        buf.p1(self.prot().id as i64);
        self.encode(buf, wordenc);
    }
}

/// `NetworkPlayer.writeInner`: opcode (+ ISAAC), the size placeholder for
/// variable-length prots, the payload, then the size. Returns the bytes sent.
pub fn frame(msg: &ServerMessage, isaac: Option<&mut Isaac>, wordenc: &WordEnc, buf: &mut Packet) -> Vec<u8> {
    let prot = msg.prot();
    buf.pos = 0;
    match isaac {
        Some(isaac) => buf.p1(prot.id as i64 + isaac.next_int() as i64),
        None => buf.p1(prot.id as i64),
    }
    match prot.length {
        -1 => buf.pos += 1,
        -2 => buf.pos += 2,
        _ => {}
    }
    let start = buf.pos;
    msg.encode(buf, wordenc);
    match prot.length {
        -1 => buf.psize1(buf.pos - start),
        -2 => buf.psize2(buf.pos - start),
        _ => {}
    }
    buf.data[..buf.pos].to_vec()
}
