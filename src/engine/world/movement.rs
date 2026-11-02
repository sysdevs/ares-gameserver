//! Movement shared by players and NPCs: steps, collision
//! by move restriction and block-walk, zone presence and teleports. The
//! abstract methods (`blockWalkFlag`, collision strategy) are a `match` on
//! [`Mover`].

use super::World;
use crate::engine::coord;
use crate::engine::entity::block_walk;
use crate::engine::entity::pathing::{Pathing, move_restrict, move_speed, move_strategy};
use crate::engine::entity::EntityBase;
use crate::engine::zone::Member;
use crate::routefinder::{collision_type, flag, line, step::can_travel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mover {
    Npc(i32),
    Player(i32),
}

impl World {
    pub fn mover(&self, m: Mover) -> (&EntityBase, &Pathing) {
        match m {
            Mover::Npc(n) => {
                let n = self.npc(n).expect("npc");
                (&n.e, &n.p)
            }
            Mover::Player(p) => {
                let p = self.player(p).expect("player");
                (&p.e, &p.p)
            }
        }
    }

    pub fn mover_mut(&mut self, m: Mover) -> (&mut EntityBase, &mut Pathing) {
        match m {
            Mover::Npc(n) => {
                let n = self.npc_mut(n).expect("npc");
                (&mut n.e, &mut n.p)
            }
            Mover::Player(p) => {
                let p = self.player_mut(p).expect("player");
                (&mut p.e, &mut p.p)
            }
        }
    }

    /// `getCollisionStrategy()`: `None` for a `nomove` NPC.
    pub(crate) fn collision_strategy(&self, m: Mover) -> Option<i32> {
        let Mover::Npc(nid) = m else { return Some(collision_type::NORMAL) };
        let t = self.configs.npcs.get(self.npc(nid)?.kind as usize).expect("npc type");
        Some(match t.moverestrict {
            move_restrict::BLOCKED => collision_type::BLOCKED,
            move_restrict::BLOCKED_NORMAL => collision_type::LINE_OF_SIGHT,
            move_restrict::INDOORS => collision_type::INDOORS,
            move_restrict::OUTDOORS => collision_type::OUTDOORS,
            move_restrict::NOMOVE => return None,
            _ => collision_type::NORMAL,
        })
    }

    /// `blockWalkFlag()`.
    pub(crate) fn block_walk_flag(&self, m: Mover) -> u32 {
        let Mover::Npc(nid) = m else { return flag::BLOCK_NPC_AND_PLAYERS };
        let n = self.npc(nid).expect("npc");
        let restrict = self.configs.npcs.get(n.kind as usize).expect("npc type").moverestrict;
        match restrict {
            move_restrict::BLOCKED => flag::OPEN,
            move_restrict::NORMAL | move_restrict::BLOCKED_NORMAL | move_restrict::INDOORS | move_restrict::OUTDOORS | move_restrict::PASSTHRU => {
                let npc_occ = if n.block_walk() == block_walk::NONE { flag::OPEN } else { flag::NPC_OCC };
                let player_occ = if restrict == move_restrict::PASSTHRU { flag::OPEN } else { flag::PLAYER_OCC };
                flag::BLOCK_NPC_AND_PLAYERS | npc_occ | player_occ
            }
            _ => flag::NULL,
        }
    }

    /// `GameMap.canTravel`: a free world also refuses members tiles.
    #[allow(clippy::too_many_arguments)]
    pub fn can_travel(&self, level: i32, x: i32, z: i32, dx: i32, dz: i32, size: i32, extra: u32, collision: i32) -> bool {
        if !self.map.members && !self.map.is_free_to_play(x + dx, z + dz) {
            return false;
        }
        can_travel(&self.collision, level, x, z, dx, dz, size, extra, collision)
    }

    pub fn is_line_of_sight(&self, level: i32, sx: i32, sz: i32, dx: i32, dz: i32) -> bool {
        line::has_line_of_sight(&self.collision, level, sx, sz, dx, dz, 1, 1, 1, 1, 0)
    }

    pub fn is_line_of_walk(&self, level: i32, sx: i32, sz: i32, dx: i32, dz: i32) -> bool {
        line::has_line_of_walk(&self.collision, level, sx, sz, dx, dz, 1, 1, 1, 1, 0)
    }

    fn member_node(&mut self, m: Mover) -> crate::engine::linklist::NodeId {
        let (map, member) = match m {
            Mover::Npc(n) => (&mut self.npc_nodes, Member::Npc(n)),
            Mover::Player(p) => (&mut self.player_nodes, Member::Player(p)),
        };
        let key = match m {
            Mover::Npc(n) | Mover::Player(n) => n,
        };
        *map.entry(key).or_insert_with(|| self.arenas.members.node(member))
    }

    /// `Zone.enter(entity)`.
    pub(crate) fn zone_enter(&mut self, m: Mover, x: i32, z: i32, level: i32) {
        let node = self.member_node(m);
        let zone = self.zones.zone(x, z, level, &mut self.arenas);
        let (zx, zz) = (zone.x, zone.z);
        let list = match m {
            Mover::Npc(_) => {
                zone.npcs_count += 1;
                zone.npcs
            }
            Mover::Player(_) => {
                zone.players_count += 1;
                zone.players
            }
        };
        self.arenas.members.add_tail(list, node);
        if let Mover::Player(_) = m {
            self.zones.grid(level).flag(zx, zz);
        }
    }

    /// `Zone.leave(entity)`.
    pub(crate) fn zone_leave(&mut self, m: Mover, x: i32, z: i32, level: i32) {
        let node = match m {
            Mover::Npc(n) => self.npc_nodes.get(&n),
            Mover::Player(p) => self.player_nodes.get(&p),
        };
        if let Some(&node) = node {
            self.arenas.members.unlink(node);
        }
        let zone = self.zones.zone(x, z, level, &mut self.arenas);
        match m {
            Mover::Npc(_) => zone.npcs_count -= 1,
            Mover::Player(_) => {
                zone.players_count -= 1;
                if zone.players_count == 0 {
                    let (zx, zz) = (zone.x, zone.z);
                    self.zones.grid(level).unflag(zx, zz);
                }
            }
        }
    }

    /// `processMovement()`.
    pub(crate) fn process_movement(&mut self, m: Mover) -> bool {
        let p = self.mover(m).1;
        if !p.has_waypoints() || p.move_speed == move_speed::STATIONARY || p.move_speed == move_speed::INSTANT {
            return false;
        }
        if p.move_speed == move_speed::CRAWL {
            let p = self.mover_mut(m).1;
            p.last_crawl = !p.last_crawl;
            if p.last_crawl && p.walk_dir == -1 {
                let d = self.validate_and_advance_step(m);
                self.mover_mut(m).1.walk_dir = d;
            }
        } else if p.walk_dir == -1 {
            let d = self.validate_and_advance_step(m);
            self.mover_mut(m).1.walk_dir = d;
            let p = self.mover(m).1;
            if p.move_speed == move_speed::RUN && p.walk_dir != -1 && p.run_dir == -1 {
                let d = self.validate_and_advance_step(m);
                self.mover_mut(m).1.run_dir = d;
            }
        }
        true
    }

    /// `refreshZonePresence(previousX, previousZ, previousLevel)`.
    pub(crate) fn refresh_zone_presence(&mut self, m: Mover, px: i32, pz: i32, plevel: i32) {
        let (e, p) = self.mover(m);
        let (x, z, level, width, block) = (e.x, e.z, e.level, e.width, p.block_walk);
        let c = &mut self.collision;
        match block {
            block_walk::NPC => {
                c.change_npc(px, pz, plevel, width, false);
                c.change_npc(x, z, level, width, true);
            }
            block_walk::ALL => {
                c.change_npc(px, pz, plevel, width, false);
                c.change_npc(x, z, level, width, true);
                c.change_block(px, pz, plevel, width, false);
                c.change_block(x, z, level, width, true);
            }
            block_walk::PLAYER => {
                c.change_npc(px, pz, plevel, width, false);
                c.change_player_occ(px, pz, plevel, width, false);
                c.change_player_occ(x, z, level, width, true);
            }
            _ => {}
        }
        let p = self.mover_mut(m).1;
        p.last_step_x = px;
        p.last_step_z = pz;
        if coord::zone(px) != coord::zone(x) || coord::zone(pz) != coord::zone(z) || plevel != level {
            self.zone_leave(m, px, pz, plevel);
            self.zone_enter(m, x, z, level);
        }
    }

    fn validate_and_advance_step(&mut self, m: Mover) -> i32 {
        let strategy = self.collision_strategy(m);
        let extra = self.block_walk_flag(m);
        if strategy.is_none() || extra == flag::NULL {
            self.mover_mut(m).1.waypoint_index = -1;
        }
        if self.mover(m).1.waypoint_index == -1 {
            return -1;
        }
        let (dx, dz) = self.take_step(m);
        let e = self.mover_mut(m).0;
        let (sx, sz, level) = (e.x, e.z, e.level);
        e.x += dx;
        e.z += dz;
        self.refresh_zone_presence(m, sx, sz, level);
        let (e, p) = self.mover_mut(m);
        if p.waypoint_index != -1 {
            let (_, wx, wz) = coord::unpack(p.waypoints[p.waypoint_index as usize]);
            if wx == e.x && wz == e.z {
                p.waypoint_index -= 1;
            }
        }
        if e.x != sx || e.z != sz {
            let (x, z, w, l) = (e.x, e.z, e.width, e.length);
            p.focus(coord::fine(x + dx, w), coord::fine(z + dz, l), false);
            p.steps_taken += 1;
            return coord::face(sx, sz, x, z);
        }
        -1
    }

    fn take_step(&self, m: Mover) -> (i32, i32) {
        let (e, p) = self.mover(m);
        if p.waypoint_index == -1 {
            return (0, 0);
        }
        let extra = self.block_walk_flag(m);
        let Some(strategy) = self.collision_strategy(m) else { return (0, 0) };
        let (x, z, level, width) = (e.x, e.z, e.level, e.width);
        let (_, wx, wz) = coord::unpack(p.waypoints[p.waypoint_index as usize]);
        let dir = coord::face(x, z, wx, wz);
        let (dx, dz) = (coord::delta_x(dir), coord::delta_z(dir));
        if p.move_strategy == move_strategy::FLY {
            return (dx, dz);
        }
        if width == 1 && self.can_travel(level, x, z, dx, dz, width, extra, strategy) {
            return (dx, dz);
        }
        if dx != 0 && self.can_travel(level, x, z, dx, 0, width, extra, strategy) {
            return (dx, 0);
        }
        if dz != 0 && self.can_travel(level, x, z, 0, dz, width, extra, strategy) {
            return (0, dz);
        }
        (0, 0)
    }

    /// `teleport(x, z, level)`: refused into an unallocated zone (players
    /// below staff level 3 are told so).
    pub fn teleport(&mut self, m: Mover, x: i32, z: i32, level: i32) {
        let level = level.clamp(0, 3);
        if !self.collision.is_zone_allocated(x, z, level) {
            match m {
                Mover::Npc(_) => return,
                Mover::Player(p) => {
                    if self.player(p).expect("player").staff_mod_level < 3 {
                        self.message_game(p, "Invalid teleport!");
                        return;
                    }
                }
            }
        }
        let e = self.mover_mut(m).0;
        let (px, pz, plevel) = (e.x, e.z, e.level);
        e.x = x;
        e.z = z;
        e.level = level;
        let dir = coord::face(px, pz, x, z);
        let (e, p) = self.mover_mut(m);
        let (w, l) = (e.width, e.length);
        p.focus(coord::fine(x + coord::delta_x(dir), w), coord::fine(z + coord::delta_z(dir), l), false);
        self.refresh_zone_presence(m, px, pz, plevel);
        let p = self.mover_mut(m).1;
        p.last_step_x = x - 1;
        p.last_step_z = z;
        p.tele = true;
        if plevel != level {
            p.move_speed = move_speed::INSTANT;
            p.jump = true;
        }
    }

    /// `validateDistanceWalked()`.
    pub(crate) fn validate_distance_walked(&mut self, m: Mover) {
        let (e, p) = self.mover_mut(m);
        let (x, z, w, l) = (e.x, e.z, e.width, e.length);
        if coord::distance_to((x, z, w, l), (p.last_tick_x, p.last_tick_z, w, l)) > 2 {
            p.jump = true;
        }
    }
}
