//! Reach strategy: has an entity at `src`
//! reached a target loc/entity of the given shape.

use super::{CollisionEngine, block_access as ba, flag, loc_angle as a, loc_shape as s, rotate, rotate_flags};

const WALL: i32 = 0;
const WALL_DECOR: i32 = 1;
const RECTANGLE: i32 = 2;
const NONE: i32 = 3;
const RECTANGLE_EXCLUSIVE: i32 = 4;

pub fn altered_rotation(angle: i32, shape: i32) -> i32 {
    if shape == 7 { (angle + 2) & 0x3 } else { angle }
}

fn exit_strategy(shape: i32) -> i32 {
    match shape {
        -2 => RECTANGLE_EXCLUSIVE,
        -1 => NONE,
        0..=3 | 9 => WALL,
        _ if shape < 9 => WALL_DECOR,
        10 | 11 | 22 => RECTANGLE,
        _ => NONE,
    }
}

/// `ReachStrategy.reached`.
#[allow(clippy::too_many_arguments)]
pub fn reached(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, dw: i32, dh: i32, size: i32, angle: i32, shape: i32, access: i32) -> bool {
    let strategy = exit_strategy(shape);
    if strategy != RECTANGLE_EXCLUSIVE && sx == dx && sz == dz {
        return true;
    }
    match strategy {
        WALL => reach_wall(f, y, sx, sz, dx, dz, size, shape, angle),
        WALL_DECOR => reach_wall_decor(f, y, sx, sz, dx, dz, size, shape, angle),
        RECTANGLE => reach_rectangle(f, y, sx, sz, dx, dz, size, dw, dh, angle, access, false),
        RECTANGLE_EXCLUSIVE => reach_rectangle(f, y, sx, sz, dx, dz, size, dw, dh, angle, access, true),
        _ => false,
    }
}

/// `reachRectangle` / `reachExclusiveRectangle` (`exclusive`: overlapping does not count).
#[allow(clippy::too_many_arguments)]
pub fn reach_rectangle(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, size: i32, dw: i32, dh: i32, angle: i32, access: i32, exclusive: bool) -> bool {
    let rw = rotate(angle, dw, dh);
    let rh = rotate(angle, dh, dw);
    let racc = rotate_flags(angle, access);
    let intersects = collides(sx, sz, dx, dz, size, size, rw, rh);
    let edge = || if size == 1 { reach_rectangle1(f, y, sx, sz, dx, dz, rw, rh, racc) } else { reach_rectangle_n(f, y, sx, sz, dx, dz, size, size, rw, rh, racc) };
    if exclusive { !intersects && edge() } else { intersects || edge() }
}

#[allow(clippy::too_many_arguments)]
pub fn collides(sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32) -> bool {
    if sx >= dx + dw || sx + sw <= dx {
        return false;
    }
    sz < dz + dh && dz < sh + sz
}

fn open(v: u32, mask: u32) -> bool {
    v & mask == flag::OPEN
}

#[allow(clippy::too_many_arguments)]
fn reach_rectangle1(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, dw: i32, dh: i32, access: i32) -> bool {
    let east = dx + dw - 1;
    let north = dz + dh - 1;
    let here = f.get(sx, sz, y);
    if sx == dx - 1 && sz >= dz && sz <= north && open(here, flag::WALL_EAST) && access & ba::BLOCK_WEST == 0 {
        return true;
    }
    if sx == east + 1 && sz >= dz && sz <= north && open(here, flag::WALL_WEST) && access & ba::BLOCK_EAST == 0 {
        return true;
    }
    if sz + 1 == dz && sx >= dx && sx <= east && open(here, flag::WALL_NORTH) && access & ba::BLOCK_SOUTH == 0 {
        return true;
    }
    sz == north + 1 && sx >= dx && sx <= east && open(here, flag::WALL_SOUTH) && access & ba::BLOCK_NORTH == 0
}

#[allow(clippy::too_many_arguments)]
fn reach_rectangle_n(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, sw: i32, sh: i32, dw: i32, dh: i32, access: i32) -> bool {
    let (src_east, src_north, dest_east, dest_north) = (sx + sw, sz + sh, dx + dw, dz + dh);
    if dest_east == sx && access & ba::BLOCK_EAST == 0 {
        (sz.max(dz)..src_north.min(dest_north)).any(|z| open(f.get(dest_east - 1, z, y), flag::WALL_EAST))
    } else if src_east == dx && access & ba::BLOCK_WEST == 0 {
        (sz.max(dz)..src_north.min(dest_north)).any(|z| open(f.get(dx, z, y), flag::WALL_WEST))
    } else if sz == dest_north && access & ba::BLOCK_NORTH == 0 {
        (sx.max(dx)..src_east.min(dest_east)).any(|x| open(f.get(x, dest_north - 1, y), flag::WALL_NORTH))
    } else if dz == src_north && access & ba::BLOCK_SOUTH == 0 {
        (sx.max(dx)..src_east.min(dest_east)).any(|x| open(f.get(x, dz, y), flag::WALL_SOUTH))
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, size: i32, shape: i32, angle: i32) -> bool {
    if size == 1 && sx == dx && sz == dz {
        return true;
    }
    if size != 1 && dx >= sx && sx + size - 1 >= dx && dz >= sz && sz + size - 1 >= dz {
        return true;
    }
    if size == 1 { reach_wall1(f, y, sx, sz, dx, dz, shape, angle) } else { reach_wall_n(f, y, sx, sz, dx, dz, size, shape, angle) }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall_decor(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, size: i32, shape: i32, angle: i32) -> bool {
    if size == 1 && sx == dx && sz == dz {
        return true;
    }
    if size != 1 && dx >= sx && sx + size - 1 >= dx && dz >= sz && sz + size - 1 >= dz {
        return true;
    }
    if size == 1 { reach_wall_decor1(f, y, sx, sz, dx, dz, shape, angle) } else { reach_wall_decor_n(f, y, sx, sz, dx, dz, size, shape, angle) }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall1(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, shape: i32, angle: i32) -> bool {
    let c = f.get(sx, sz, y);
    let at = |x: i32, z: i32| sx == x && sz == z;
    match shape {
        s::WALL_STRAIGHT => match angle {
            a::WEST => at(dx - 1, dz) || (at(dx, dz + 1) && open(c, flag::BLOCK_NORTH)) || (at(dx, dz - 1) && open(c, flag::BLOCK_SOUTH)),
            a::NORTH => at(dx, dz + 1) || (at(dx - 1, dz) && open(c, flag::BLOCK_WEST)) || (at(dx + 1, dz) && open(c, flag::BLOCK_EAST)),
            a::EAST => at(dx + 1, dz) || (at(dx, dz + 1) && open(c, flag::BLOCK_NORTH)) || (at(dx, dz - 1) && open(c, flag::BLOCK_SOUTH)),
            _ => at(dx, dz - 1) || (at(dx - 1, dz) && open(c, flag::BLOCK_WEST)) || (at(dx + 1, dz) && open(c, flag::BLOCK_EAST)),
        },
        s::WALL_L => match angle {
            a::WEST => at(dx - 1, dz) || at(dx, dz + 1) || (at(dx + 1, dz) && open(c, flag::BLOCK_EAST)) || (at(dx, dz - 1) && open(c, flag::BLOCK_SOUTH)),
            a::NORTH => (at(dx - 1, dz) && open(c, flag::BLOCK_WEST)) || at(dx, dz + 1) || at(dx + 1, dz) || (at(dx, dz - 1) && open(c, flag::BLOCK_SOUTH)),
            a::EAST => (at(dx - 1, dz) && open(c, flag::BLOCK_WEST)) || (at(dx, dz + 1) && open(c, flag::BLOCK_NORTH)) || at(dx + 1, dz) || at(dx, dz - 1),
            _ => at(dx - 1, dz) || (at(dx, dz + 1) && open(c, flag::BLOCK_NORTH)) || (at(dx + 1, dz) && open(c, flag::BLOCK_EAST)) || at(dx, dz - 1),
        },
        s::WALL_DIAGONAL => {
            (at(dx, dz + 1) && open(c, flag::WALL_SOUTH))
                || (at(dx, dz - 1) && open(c, flag::WALL_NORTH))
                || (at(dx - 1, dz) && open(c, flag::WALL_EAST))
                || (at(dx + 1, dz) && open(c, flag::WALL_WEST))
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall_n(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, size: i32, shape: i32, angle: i32) -> bool {
    let c = f.get(sx, sz, y);
    let east = sx + size - 1;
    let north = sz + size - 1;
    // the four approach sides of an N-tile entity
    let west_of = || sx == dx - size && sz <= dz && north >= dz;
    let east_of = || sx == dx + 1 && sz <= dz && north >= dz;
    let north_of = || dx >= sx && dx <= east && sz == dz + 1;
    let south_of = || dx >= sx && dx <= east && sz == dz - size;
    match shape {
        s::WALL_STRAIGHT => match angle {
            a::WEST => west_of() || (north_of() && open(c, flag::BLOCK_NORTH)) || (south_of() && open(c, flag::BLOCK_SOUTH)),
            a::NORTH => north_of() || (west_of() && open(c, flag::BLOCK_WEST)) || (east_of() && open(c, flag::BLOCK_EAST)),
            a::EAST => east_of() || (north_of() && open(c, flag::BLOCK_NORTH)) || (south_of() && open(c, flag::BLOCK_SOUTH)),
            _ => south_of() || (west_of() && open(c, flag::BLOCK_WEST)) || (east_of() && open(c, flag::BLOCK_EAST)),
        },
        s::WALL_L => match angle {
            a::WEST => west_of() || north_of() || (east_of() && open(c, flag::BLOCK_EAST)) || (south_of() && open(c, flag::BLOCK_SOUTH)),
            a::NORTH => (west_of() && open(c, flag::BLOCK_WEST)) || north_of() || east_of() || (south_of() && open(c, flag::BLOCK_SOUTH)),
            a::EAST => (west_of() && open(c, flag::BLOCK_WEST)) || (north_of() && open(c, flag::BLOCK_NORTH)) || east_of() || south_of(),
            _ => west_of() || (north_of() && open(c, flag::BLOCK_NORTH)) || (east_of() && open(c, flag::BLOCK_EAST)) || south_of(),
        },
        s::WALL_DIAGONAL => {
            (north_of() && open(c, flag::BLOCK_NORTH))
                || (south_of() && open(c, flag::BLOCK_SOUTH))
                || (west_of() && open(c, flag::BLOCK_WEST))
                || (east_of() && open(c, flag::BLOCK_EAST))
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall_decor1(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, shape: i32, angle: i32) -> bool {
    let c = f.get(sx, sz, y);
    let at = |x: i32, z: i32| sx == x && sz == z;
    match shape {
        s::WALLDECOR_DIAGONAL_OFFSET | s::WALLDECOR_DIAGONAL_NOOFFSET => match altered_rotation(angle, shape) {
            a::WEST => (at(dx + 1, dz) && open(c, flag::WALL_WEST)) || (at(dx, dz - 1) && open(c, flag::WALL_NORTH)),
            a::NORTH => (at(dx - 1, dz) && open(c, flag::WALL_EAST)) || (at(dx, dz - 1) && open(c, flag::WALL_NORTH)),
            a::EAST => (at(dx - 1, dz) && open(c, flag::WALL_EAST)) || (at(dx, dz + 1) && open(c, flag::WALL_SOUTH)),
            _ => (at(dx + 1, dz) && open(c, flag::WALL_WEST)) || (at(dx, dz + 1) && open(c, flag::WALL_SOUTH)),
        },
        s::WALLDECOR_DIAGONAL_BOTH => {
            (at(dx, dz + 1) && open(c, flag::WALL_SOUTH))
                || (at(dx, dz - 1) && open(c, flag::WALL_NORTH))
                || (at(dx - 1, dz) && open(c, flag::WALL_EAST))
                || (at(dx + 1, dz) && open(c, flag::WALL_WEST))
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn reach_wall_decor_n(f: &CollisionEngine, y: i32, sx: i32, sz: i32, dx: i32, dz: i32, size: i32, shape: i32, angle: i32) -> bool {
    let c = f.get(sx, sz, y);
    let east = sx + size - 1;
    let north = sz + size - 1;
    let west_of = || sx == dx - size && sz <= dz && north >= dz;
    let east_of = || sx == dx + 1 && sz <= dz && north >= dz;
    let north_of = || sx <= dx && sz == dz + 1 && east >= dx;
    let south_of = || sx <= dx && sz == dz - size && east >= dx;
    match shape {
        s::WALLDECOR_DIAGONAL_OFFSET | s::WALLDECOR_DIAGONAL_NOOFFSET => match altered_rotation(angle, shape) {
            a::WEST => (east_of() && open(c, flag::WALL_WEST)) || (south_of() && open(c, flag::WALL_NORTH)),
            a::NORTH => (west_of() && open(c, flag::WALL_EAST)) || (south_of() && open(c, flag::WALL_NORTH)),
            a::EAST => (west_of() && open(c, flag::WALL_EAST)) || (north_of() && open(c, flag::WALL_SOUTH)),
            _ => (east_of() && open(c, flag::WALL_WEST)) || (north_of() && open(c, flag::WALL_SOUTH)),
        },
        s::WALLDECOR_DIAGONAL_BOTH => {
            (north_of() && open(c, flag::WALL_SOUTH))
                || (south_of() && open(c, flag::WALL_NORTH))
                || (west_of() && open(c, flag::WALL_EAST))
                || (east_of() && open(c, flag::WALL_WEST))
        }
        _ => false,
    }
}
