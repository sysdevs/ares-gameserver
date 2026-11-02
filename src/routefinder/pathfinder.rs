//! Breadth-first search over a 128x128 window centred on
//! the source, with a 4096-entry ring buffer. Neighbour order (W, E, S, N,
//! SW, SE, NW, NE) fixes which of several equal routes wins.

use super::{CollisionEngine, can_move, direction as dir, flag, pack_coord, reach, rotate};
use std::collections::VecDeque;

const SIZE: i32 = 128;
const RING: usize = 4096;
const DEFAULT_DISTANCE: i32 = 99_999_999;
const SRC_DIRECTION: i8 = 99;
const MAX_ALT_LOWEST_COST: i32 = 1000;
const MAX_ALT_SEEK_RANGE: i32 = 100;
const MAX_ALT_DISTANCE: i32 = 10;

pub struct PathFinder {
    directions: Vec<i8>,
    distances: Vec<i32>,
    valid_x: Vec<i32>,
    valid_z: Vec<i32>,
    curr_x: i32,
    curr_z: i32,
    reader: usize,
    writer: usize,
}

impl Default for PathFinder {
    fn default() -> Self {
        let n = (SIZE * SIZE) as usize;
        PathFinder { directions: vec![0; n], distances: vec![DEFAULT_DISTANCE; n], valid_x: vec![0; RING], valid_z: vec![0; RING], curr_x: 0, curr_z: 0, reader: 0, writer: 0 }
    }
}

/// The search inputs that stay fixed during one `findPath`.
struct Query<'a> {
    f: &'a CollisionEngine,
    base_x: i32,
    base_z: i32,
    y: i32,
    dest_x: i32,
    dest_z: i32,
    dest_w: i32,
    dest_h: i32,
    size: i32,
    angle: i32,
    shape: i32,
    access: i32,
    collision: i32,
}

impl Query<'_> {
    /// `canMove(collision, flags.get(base + local), block)`.
    fn ok(&self, lx: i32, lz: i32, block: u32) -> bool {
        can_move(self.collision, self.f.get(self.base_x + lx, self.base_z + lz, self.y), block)
    }
}

fn idx(x: i32, z: i32) -> usize {
    (x * SIZE + z) as usize
}

impl PathFinder {
    /// `findPath`: waypoints from the source end, at most `max_waypoints`
    /// (when full, the one nearest the destination is dropped). Empty when
    /// no route exists (and `move_near` finds no approach point).
    #[allow(clippy::too_many_arguments)]
    pub fn find_path(
        &mut self,
        f: &CollisionEngine,
        y: i32,
        src_x: i32,
        src_z: i32,
        dest_x: i32,
        dest_z: i32,
        size: i32,
        dest_w: i32,
        dest_h: i32,
        angle: i32,
        shape: i32,
        move_near: bool,
        access: i32,
        max_waypoints: i32,
        collision: i32,
    ) -> Vec<u32> {
        self.directions.fill(0);
        self.distances.fill(DEFAULT_DISTANCE);
        self.reader = 0;
        self.writer = 0;

        let base_x = src_x - SIZE / 2;
        let base_z = src_z - SIZE / 2;
        let (local_src_x, local_src_z) = (src_x - base_x, src_z - base_z);
        let (local_dest_x, local_dest_z) = (dest_x - base_x, dest_z - base_z);
        self.append(local_src_x, local_src_z, SRC_DIRECTION, 0);

        let q = Query { f, base_x, base_z, y, dest_x: local_dest_x, dest_z: local_dest_z, dest_w, dest_h, size, angle, shape, access, collision };
        let found = match size {
            1 => self.search1(&q),
            2 => self.search2(&q),
            _ => self.search_n(&q),
        };
        if !found && (!move_near || !self.closest_approach(local_dest_x, local_dest_z, rotate(angle, dest_w, dest_h), rotate(angle, dest_h, dest_w))) {
            return Vec::new();
        }

        let mut waypoints = VecDeque::new();
        let mut next = self.directions[idx(self.curr_x, self.curr_z)];
        let mut curr: i8 = -1;
        for _ in 0..self.directions.len() {
            if self.curr_x == local_src_x && self.curr_z == local_src_z {
                break;
            }
            if curr != next {
                curr = next;
                if waypoints.len() as i32 >= max_waypoints {
                    waypoints.pop_back();
                }
                waypoints.push_front(pack_coord(y, base_x + self.curr_x, base_z + self.curr_z));
            }
            if curr & dir::EAST != 0 {
                self.curr_x += 1;
            } else if curr & dir::WEST != 0 {
                self.curr_x -= 1;
            }
            if curr & dir::NORTH != 0 {
                self.curr_z += 1;
            } else if curr & dir::SOUTH != 0 {
                self.curr_z -= 1;
            }
            next = self.directions[idx(self.curr_x, self.curr_z)];
        }
        waypoints.into()
    }

    /// Pops the next tile; `true` if it reaches the target.
    fn pop(&mut self, q: &Query) -> bool {
        self.curr_x = self.valid_x[self.reader];
        self.curr_z = self.valid_z[self.reader];
        self.reader = (self.reader + 1) & (RING - 1);
        reach::reached(q.f, q.y, self.curr_x + q.base_x, self.curr_z + q.base_z, q.dest_x + q.base_x, q.dest_z + q.base_z, q.dest_w, q.dest_h, q.size, q.angle, q.shape, q.access)
    }

    fn unvisited(&self, x: i32, z: i32) -> bool {
        self.directions[idx(x, z)] == 0
    }

    fn search1(&mut self, q: &Query) -> bool {
        let rel = SIZE - 1;
        while self.writer != self.reader {
            if self.pop(q) {
                return true;
            }
            let (cx, cz) = (self.curr_x, self.curr_z);
            let next = self.distances[idx(cx, cz)] + 1;
            if cx > 0 && self.unvisited(cx - 1, cz) && q.ok(cx - 1, cz, flag::BLOCK_WEST) {
                self.append(cx - 1, cz, dir::EAST, next);
            }
            if cx < rel && self.unvisited(cx + 1, cz) && q.ok(cx + 1, cz, flag::BLOCK_EAST) {
                self.append(cx + 1, cz, dir::WEST, next);
            }
            if cz > 0 && self.unvisited(cx, cz - 1) && q.ok(cx, cz - 1, flag::BLOCK_SOUTH) {
                self.append(cx, cz - 1, dir::NORTH, next);
            }
            if cz < rel && self.unvisited(cx, cz + 1) && q.ok(cx, cz + 1, flag::BLOCK_NORTH) {
                self.append(cx, cz + 1, dir::SOUTH, next);
            }
            if cx > 0
                && cz > 0
                && self.unvisited(cx - 1, cz - 1)
                && q.ok(cx - 1, cz - 1, flag::BLOCK_SOUTH_WEST)
                && q.ok(cx - 1, cz, flag::BLOCK_WEST)
                && q.ok(cx, cz - 1, flag::BLOCK_SOUTH)
            {
                self.append(cx - 1, cz - 1, dir::NORTH_EAST, next);
            }
            if cx < rel
                && cz > 0
                && self.unvisited(cx + 1, cz - 1)
                && q.ok(cx + 1, cz - 1, flag::BLOCK_SOUTH_EAST)
                && q.ok(cx + 1, cz, flag::BLOCK_EAST)
                && q.ok(cx, cz - 1, flag::BLOCK_SOUTH)
            {
                self.append(cx + 1, cz - 1, dir::NORTH_WEST, next);
            }
            if cx > 0
                && cz < rel
                && self.unvisited(cx - 1, cz + 1)
                && q.ok(cx - 1, cz + 1, flag::BLOCK_NORTH_WEST)
                && q.ok(cx - 1, cz, flag::BLOCK_WEST)
                && q.ok(cx, cz + 1, flag::BLOCK_NORTH)
            {
                self.append(cx - 1, cz + 1, dir::SOUTH_EAST, next);
            }
            if cx < rel
                && cz < rel
                && self.unvisited(cx + 1, cz + 1)
                && q.ok(cx + 1, cz + 1, flag::BLOCK_NORTH_EAST)
                && q.ok(cx + 1, cz, flag::BLOCK_EAST)
                && q.ok(cx, cz + 1, flag::BLOCK_NORTH)
            {
                self.append(cx + 1, cz + 1, dir::SOUTH_WEST, next);
            }
        }
        false
    }

    fn search2(&mut self, q: &Query) -> bool {
        let rel = SIZE - 2;
        while self.writer != self.reader {
            if self.pop(q) {
                return true;
            }
            let (cx, cz) = (self.curr_x, self.curr_z);
            let next = self.distances[idx(cx, cz)] + 1;
            if cx > 0 && self.unvisited(cx - 1, cz) && q.ok(cx - 1, cz, flag::BLOCK_SOUTH_WEST) && q.ok(cx - 1, cz + 1, flag::BLOCK_NORTH_WEST) {
                self.append(cx - 1, cz, dir::EAST, next);
            }
            if cx < rel && self.unvisited(cx + 1, cz) && q.ok(cx + 2, cz, flag::BLOCK_SOUTH_EAST) && q.ok(cx + 2, cz + 1, flag::BLOCK_NORTH_EAST) {
                self.append(cx + 1, cz, dir::WEST, next);
            }
            if cz > 0 && self.unvisited(cx, cz - 1) && q.ok(cx, cz - 1, flag::BLOCK_SOUTH_WEST) && q.ok(cx + 1, cz - 1, flag::BLOCK_SOUTH_EAST) {
                self.append(cx, cz - 1, dir::NORTH, next);
            }
            if cz < rel && self.unvisited(cx, cz + 1) && q.ok(cx, cz + 2, flag::BLOCK_NORTH_WEST) && q.ok(cx + 1, cz + 2, flag::BLOCK_NORTH_EAST) {
                self.append(cx, cz + 1, dir::SOUTH, next);
            }
            if cx > 0
                && cz > 0
                && self.unvisited(cx - 1, cz - 1)
                && q.ok(cx - 1, cz, flag::BLOCK_NORTH_AND_SOUTH_EAST)
                && q.ok(cx - 1, cz - 1, flag::BLOCK_SOUTH_WEST)
                && q.ok(cx, cz - 1, flag::BLOCK_NORTH_EAST_AND_WEST)
            {
                self.append(cx - 1, cz - 1, dir::NORTH_EAST, next);
            }
            if cx < rel
                && cz > 0
                && self.unvisited(cx + 1, cz - 1)
                && q.ok(cx + 1, cz - 1, flag::BLOCK_NORTH_EAST_AND_WEST)
                && q.ok(cx + 2, cz - 1, flag::BLOCK_SOUTH_EAST)
                && q.ok(cx + 2, cz, flag::BLOCK_NORTH_AND_SOUTH_WEST)
            {
                self.append(cx + 1, cz - 1, dir::NORTH_WEST, next);
            }
            if cx > 0
                && cz < rel
                && self.unvisited(cx - 1, cz + 1)
                && q.ok(cx - 1, cz + 1, flag::BLOCK_NORTH_AND_SOUTH_EAST)
                && q.ok(cx - 1, cz + 2, flag::BLOCK_NORTH_WEST)
                && q.ok(cx, cz + 2, flag::BLOCK_SOUTH_EAST_AND_WEST)
            {
                self.append(cx - 1, cz + 1, dir::SOUTH_EAST, next);
            }
            if cx < rel
                && cz < rel
                && self.unvisited(cx + 1, cz + 1)
                && q.ok(cx + 1, cz + 2, flag::BLOCK_SOUTH_EAST_AND_WEST)
                && q.ok(cx + 2, cz + 2, flag::BLOCK_NORTH_EAST)
                && q.ok(cx + 2, cz + 1, flag::BLOCK_NORTH_AND_SOUTH_WEST)
            {
                self.append(cx + 1, cz + 1, dir::SOUTH_WEST, next);
            }
        }
        false
    }

    fn search_n(&mut self, q: &Query) -> bool {
        let s = q.size;
        let rel = SIZE - s;
        while self.writer != self.reader {
            if self.pop(q) {
                return true;
            }
            let (cx, cz) = (self.curr_x, self.curr_z);
            let next = self.distances[idx(cx, cz)] + 1;
            if cx > 0
                && self.unvisited(cx - 1, cz)
                && q.ok(cx - 1, cz, flag::BLOCK_SOUTH_WEST)
                && q.ok(cx - 1, cz + s - 1, flag::BLOCK_NORTH_WEST)
                && (1..s - 1).all(|i| q.ok(cx - 1, cz + i, flag::BLOCK_NORTH_AND_SOUTH_EAST))
            {
                self.append(cx - 1, cz, dir::EAST, next);
            }
            if cx < rel
                && self.unvisited(cx + 1, cz)
                && q.ok(cx + s, cz, flag::BLOCK_SOUTH_EAST)
                && q.ok(cx + s, cz + s - 1, flag::BLOCK_NORTH_EAST)
                && (1..s - 1).all(|i| q.ok(cx + s, cz + i, flag::BLOCK_NORTH_AND_SOUTH_WEST))
            {
                self.append(cx + 1, cz, dir::WEST, next);
            }
            if cz > 0
                && self.unvisited(cx, cz - 1)
                && q.ok(cx, cz - 1, flag::BLOCK_SOUTH_WEST)
                && q.ok(cx + s - 1, cz - 1, flag::BLOCK_SOUTH_EAST)
                && (1..s - 1).all(|i| q.ok(cx + i, cz - 1, flag::BLOCK_NORTH_EAST_AND_WEST))
            {
                self.append(cx, cz - 1, dir::NORTH, next);
            }
            if cz < rel
                && self.unvisited(cx, cz + 1)
                && q.ok(cx, cz + s, flag::BLOCK_NORTH_WEST)
                && q.ok(cx + s - 1, cz + s, flag::BLOCK_NORTH_EAST)
                && (1..s - 1).all(|i| q.ok(cx + i, cz + s, flag::BLOCK_SOUTH_EAST_AND_WEST))
            {
                self.append(cx, cz + 1, dir::SOUTH, next);
            }
            if cx > 0
                && cz > 0
                && self.unvisited(cx - 1, cz - 1)
                && q.ok(cx - 1, cz - 1, flag::BLOCK_SOUTH_WEST)
                && (1..s).all(|i| q.ok(cx - 1, cz + i - 1, flag::BLOCK_NORTH_AND_SOUTH_EAST) && q.ok(cx + i - 1, cz - 1, flag::BLOCK_NORTH_EAST_AND_WEST))
            {
                self.append(cx - 1, cz - 1, dir::NORTH_EAST, next);
            }
            if cx < rel
                && cz > 0
                && self.unvisited(cx + 1, cz - 1)
                && q.ok(cx + s, cz - 1, flag::BLOCK_SOUTH_EAST)
                && (1..s).all(|i| q.ok(cx + s, cz + i - 1, flag::BLOCK_NORTH_AND_SOUTH_WEST) && q.ok(cx + i, cz - 1, flag::BLOCK_NORTH_EAST_AND_WEST))
            {
                self.append(cx + 1, cz - 1, dir::NORTH_WEST, next);
            }
            if cx > 0
                && cz < rel
                && self.unvisited(cx - 1, cz + 1)
                && q.ok(cx - 1, cz + s, flag::BLOCK_NORTH_WEST)
                && (1..s).all(|i| q.ok(cx - 1, cz + i, flag::BLOCK_NORTH_AND_SOUTH_EAST) && q.ok(cx + i - 1, cz + s, flag::BLOCK_SOUTH_EAST_AND_WEST))
            {
                self.append(cx - 1, cz + 1, dir::SOUTH_EAST, next);
            }
            if cx < rel
                && cz < rel
                && self.unvisited(cx + 1, cz + 1)
                && q.ok(cx + s, cz + s, flag::BLOCK_NORTH_EAST)
                && (1..s).all(|i| q.ok(cx + i, cz + s, flag::BLOCK_SOUTH_EAST_AND_WEST) && q.ok(cx + s, cz + i, flag::BLOCK_NORTH_AND_SOUTH_WEST))
            {
                self.append(cx + 1, cz + 1, dir::SOUTH_WEST, next);
            }
        }
        false
    }

    /// `findClosestApproachPoint`: the visited tile within 10 of the target
    /// with the lowest squared distance to it, ties to the shorter route.
    fn closest_approach(&mut self, dest_x: i32, dest_z: i32, w: i32, h: i32) -> bool {
        let mut lowest = MAX_ALT_LOWEST_COST;
        let mut max_alt = MAX_ALT_SEEK_RANGE;
        for x in dest_x - MAX_ALT_DISTANCE..=dest_x + MAX_ALT_DISTANCE {
            for z in dest_z - MAX_ALT_DISTANCE..=dest_z + MAX_ALT_DISTANCE {
                if !(0..SIZE).contains(&x) || !(0..SIZE).contains(&z) || self.distances[idx(x, z)] >= MAX_ALT_SEEK_RANGE {
                    continue;
                }
                let dx = if x < dest_x {
                    dest_x - x
                } else if x > dest_x + w - 1 {
                    x - (w + dest_x - 1)
                } else {
                    0
                };
                let dz = if z < dest_z {
                    dest_z - z
                } else if z > dest_z + h - 1 {
                    z - (h + dest_z - 1)
                } else {
                    0
                };
                let cost = dx * dx + dz * dz;
                let distance = self.distances[idx(x, z)];
                if cost < lowest || (cost == lowest && max_alt > distance) {
                    self.curr_x = x;
                    self.curr_z = z;
                    lowest = cost;
                    max_alt = distance;
                }
            }
        }
        lowest != MAX_ALT_LOWEST_COST
    }

    fn append(&mut self, x: i32, z: i32, direction: i8, distance: i32) {
        let i = idx(x, z);
        self.directions[i] = direction;
        self.distances[i] = distance;
        self.valid_x[self.writer] = x;
        self.valid_z[self.writer] = z;
        self.writer = (self.writer + 1) & (RING - 1);
    }
}
