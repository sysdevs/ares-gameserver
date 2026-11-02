//! Coordinate packing and distances.

/// `packCoord(level, x, z)`: each field masked to its width (wider inputs wrap).
pub fn pack_coord(level: i64, x: i64, z: i64) -> i32 {
    ((z & 0x3fff) | ((x & 0x3fff) << 14) | ((level & 0x3) << 28)) as i32
}

/// `Direction`.
pub mod direction {
    pub const NORTH_WEST: i32 = 0;
    pub const NORTH: i32 = 1;
    pub const NORTH_EAST: i32 = 2;
    pub const WEST: i32 = 3;
    pub const EAST: i32 = 4;
    pub const SOUTH_WEST: i32 = 5;
    pub const SOUTH: i32 = 6;
    pub const SOUTH_EAST: i32 = 7;
}

/// `CoordGrid.packCoord` on ints.
pub fn pack(level: i32, x: i32, z: i32) -> i32 {
    (z & 0x3fff) | ((x & 0x3fff) << 14) | ((level & 0x3) << 28)
}

/// `CoordGrid.unpackCoord`: `(level, x, z)`.
pub fn unpack(coord: i32) -> (i32, i32, i32) {
    ((coord >> 28) & 0x3, (coord >> 14) & 0x3fff, coord & 0x3fff)
}

pub fn zone(pos: i32) -> i32 {
    pos >> 3
}

pub fn pack_zone_coord(x: i32, z: i32) -> i32 {
    ((x & 0x7) << 4) | (z & 0x7)
}

/// `CoordGrid.face`: -1 when the tiles are the same.
pub fn face(sx: i32, sz: i32, dx: i32, dz: i32) -> i32 {
    use direction::*;
    use std::cmp::Ordering::*;
    match (sx.cmp(&dx), sz.cmp(&dz)) {
        (Equal, Greater) => SOUTH,
        (Equal, Less) => NORTH,
        (Equal, Equal) => -1,
        (Greater, Greater) => SOUTH_WEST,
        (Greater, Less) => NORTH_WEST,
        (Greater, Equal) => WEST,
        (Less, Greater) => SOUTH_EAST,
        (Less, Less) => NORTH_EAST,
        (Less, Equal) => EAST,
    }
}

pub fn delta_x(dir: i32) -> i32 {
    use direction::*;
    match dir {
        SOUTH_EAST | NORTH_EAST | EAST => 1,
        SOUTH_WEST | NORTH_WEST | WEST => -1,
        _ => 0,
    }
}

pub fn delta_z(dir: i32) -> i32 {
    use direction::*;
    match dir {
        NORTH_WEST | NORTH_EAST | NORTH => 1,
        SOUTH_WEST | SOUTH_EAST | SOUTH => -1,
        _ => 0,
    }
}

pub fn fine(pos: i32, size: i32) -> i32 {
    pos * 2 + size
}

/// A tile area: `(x, z, width, length)`.
pub type Area = (i32, i32, i32, i32);

fn closest(pos: Area, other: Area) -> (i32, i32) {
    let (ox, oz) = (pos.0 + pos.2 - 1, pos.1 + pos.3 - 1);
    let x = if other.0 <= pos.0 { pos.0 } else if other.0 >= ox { ox } else { other.0 };
    let z = if other.1 <= pos.1 { pos.1 } else if other.1 >= oz { oz } else { other.1 };
    (x, z)
}

/// `CoordGrid.distanceTo`: Chebyshev distance between the nearest tiles.
pub fn distance_to(pos: Area, other: Area) -> i32 {
    let p1 = closest(pos, other);
    let p2 = closest(other, pos);
    (p1.0 - p2.0).abs().max((p1.1 - p2.1).abs())
}

/// `CoordGrid.distanceToSW`.
pub fn distance_to_sw(x: i32, z: i32, ox: i32, oz: i32) -> i32 {
    (x - ox).abs().max((z - oz).abs())
}

/// Squared euclidean distance, exact in `i64`.
pub fn euclidean_squared(x: i32, z: i32, ox: i32, oz: i32) -> i64 {
    let (dx, dz) = ((x - ox).abs() as i64, (z - oz).abs() as i64);
    dx * dx + dz * dz
}

/// `CoordGrid.intersects`.
#[allow(clippy::too_many_arguments)]
pub fn intersects(sx: i32, sz: i32, sw: i32, sh: i32, dx: i32, dz: i32, dw: i32, dh: i32) -> bool {
    !(dx >= sx + sw || dx + dw <= sx || dz >= sz + sh || dz + dh <= sz)
}
