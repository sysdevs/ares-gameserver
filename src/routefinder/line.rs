//! Bresenham-style rays
//! in 16.16 fixed point for line of sight and line of walk.

use super::{CollisionEngine, flag, pack_coord};

const HALF_TILE: i32 = (1 << 16) / 2;

/// `Line.coordinate(a, b, size)`: the edge of `a`'s footprint nearest `b`.
fn coordinate(a: i32, b: i32, size: i32) -> i32 {
    if a >= b {
        a
    } else if a + size - 1 <= b {
        a + size - 1
    } else {
        b
    }
}

/// The per-direction block masks for sight or walk, each `| extra`.
struct Masks {
    west: u32,
    east: u32,
    south: u32,
    north: u32,
    loc: u32,
    proj: u32,
    los: bool,
}

fn masks(los: bool, extra: u32) -> Masks {
    let (w, e, s, n) = if los {
        (flag::WALL_WEST_PROJ_BLOCKER, flag::WALL_EAST_PROJ_BLOCKER, flag::WALL_SOUTH_PROJ_BLOCKER, flag::WALL_NORTH_PROJ_BLOCKER)
    } else {
        (flag::WALL_WEST, flag::WALL_EAST, flag::WALL_SOUTH, flag::WALL_NORTH)
    };
    let base = if los { flag::LOC_PROJ_BLOCKER } else { flag::WALK_BLOCKED };
    Masks { west: w | base | extra, east: e | base | extra, south: s | base | extra, north: n | base | extra, loc: flag::LOC | extra, proj: flag::LOC_PROJ_BLOCKER | extra, los }
}

/// `rayCastLine` (`path = None`) and `rayCastPath` (`path = Some`): they
/// differ only in that the path variant checks (and records) the second tile
/// of a step only when the minor axis actually moved.
#[allow(clippy::too_many_arguments)]
fn ray(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, m: Masks, mut path: Option<&mut Vec<u32>>) -> bool {
    let start_x = coordinate(sx, dx, sw);
    let start_z = coordinate(sz, dz, sh);
    let end_x = coordinate(dx, sx, dw);
    let end_z = coordinate(dz, sz, dh);
    if start_x == end_x && start_z == end_z {
        return true;
    }
    if m.los && f.is_flagged(start_x, start_z, y, m.loc) {
        return false;
    }
    let delta_x = end_x - start_x;
    let delta_z = end_z - start_z;
    let (abs_x, abs_z) = (delta_x.abs(), delta_z.abs());
    let travel_east = delta_x >= 0;
    let travel_north = delta_z >= 0;
    let mut x_flags = if travel_east { m.west } else { m.east };
    let mut z_flags = if travel_north { m.south } else { m.north };
    let record = path.is_some();

    if abs_x > abs_z {
        let offset_x = if travel_east { 1 } else { -1 };
        let offset_z = if travel_north { 0 } else { -1 };
        let mut scaled_z = (start_z << 16) + HALF_TILE + offset_z;
        let tangent = (delta_z << 16) / abs_x;
        let mut curr_x = start_x;
        while curr_x != end_x {
            curr_x += offset_x;
            let curr_z = scaled_z >> 16;
            if m.los && curr_x == end_x && curr_z == end_z {
                x_flags &= !m.proj;
            }
            if f.is_flagged(curr_x, curr_z, y, x_flags) {
                return false;
            }
            if let Some(p) = path.as_deref_mut() {
                p.push(pack_coord(y, curr_x, curr_z));
            }
            scaled_z += tangent;
            let next_z = scaled_z >> 16;
            if record && next_z == curr_z {
                continue;
            }
            if m.los && curr_x == end_x && next_z == end_z {
                z_flags &= !m.proj;
            }
            if next_z != curr_z && f.is_flagged(curr_x, next_z, y, z_flags) {
                return false;
            }
            if let Some(p) = path.as_deref_mut() {
                p.push(pack_coord(y, curr_x, next_z));
            }
        }
    } else {
        let offset_x = if travel_east { 0 } else { -1 };
        let offset_z = if travel_north { 1 } else { -1 };
        let mut scaled_x = (start_x << 16) + HALF_TILE + offset_x;
        let tangent = (delta_x << 16) / abs_z;
        let mut curr_z = start_z;
        while curr_z != end_z {
            curr_z += offset_z;
            let curr_x = scaled_x >> 16;
            if m.los && curr_x == end_x && curr_z == end_z {
                z_flags &= !m.proj;
            }
            if f.is_flagged(curr_x, curr_z, y, z_flags) {
                return false;
            }
            if let Some(p) = path.as_deref_mut() {
                p.push(pack_coord(y, curr_x, curr_z));
            }
            scaled_x += tangent;
            let next_x = scaled_x >> 16;
            if record && next_x == curr_x {
                continue;
            }
            if m.los && next_x == end_x && curr_z == end_z {
                x_flags &= !m.proj;
            }
            if next_x != curr_x && f.is_flagged(next_x, curr_z, y, x_flags) {
                return false;
            }
            if let Some(p) = path.as_deref_mut() {
                p.push(pack_coord(y, next_x, curr_z));
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub fn has_line_of_sight(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, extra: u32) -> bool {
    ray(f, y, sx, sz, dx, dz, sw, sh, dw, dh, masks(true, extra), None)
}

#[allow(clippy::too_many_arguments)]
pub fn has_line_of_walk(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, extra: u32) -> bool {
    ray(f, y, sx, sz, dx, dz, sw, sh, dw, dh, masks(false, extra), None)
}

/// `lineOfSight`: the tiles crossed, or empty when blocked or already there.
#[allow(clippy::too_many_arguments)]
pub fn line_of_sight(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, extra: u32) -> Vec<u32> {
    let mut path = Vec::new();
    if ray(f, y, sx, sz, dx, dz, sw, sh, dw, dh, masks(true, extra), Some(&mut path)) { path } else { Vec::new() }
}

#[allow(clippy::too_many_arguments)]
pub fn line_of_walk(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, extra: u32) -> Vec<u32> {
    let mut path = Vec::new();
    if ray(f, y, sx, sz, dx, dz, sw, sh, dw, dh, masks(false, extra), Some(&mut path)) { path } else { Vec::new() }
}
