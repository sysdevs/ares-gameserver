//! Collision flags and path finding (from rsmod's Kotlin pathfinder).
//! Coordinates are `i32` (they go negative at the map edge), flags
//! `u32` (ROOF is bit 31). Unallocated zones read as `flag::NULL`.

pub mod collision;
pub mod line;
pub mod naive;
pub mod pathfinder;
pub mod reach;
pub mod step;

pub use collision::CollisionEngine;
pub use pathfinder::PathFinder;

/// `CollisionFlag`.
pub mod flag {
    pub const OPEN: u32 = 0x0;
    pub const WALL_NORTH_WEST: u32 = 0x1;
    pub const WALL_NORTH: u32 = 0x2;
    pub const WALL_NORTH_EAST: u32 = 0x4;
    pub const WALL_EAST: u32 = 0x8;
    pub const WALL_SOUTH_EAST: u32 = 0x10;
    pub const WALL_SOUTH: u32 = 0x20;
    pub const WALL_SOUTH_WEST: u32 = 0x40;
    pub const WALL_WEST: u32 = 0x80;
    pub const LOC: u32 = 0x100;
    pub const WALL_NORTH_WEST_PROJ_BLOCKER: u32 = 0x200;
    pub const WALL_NORTH_PROJ_BLOCKER: u32 = 0x400;
    pub const WALL_NORTH_EAST_PROJ_BLOCKER: u32 = 0x800;
    pub const WALL_EAST_PROJ_BLOCKER: u32 = 0x1000;
    pub const WALL_SOUTH_EAST_PROJ_BLOCKER: u32 = 0x2000;
    pub const WALL_SOUTH_PROJ_BLOCKER: u32 = 0x4000;
    pub const WALL_SOUTH_WEST_PROJ_BLOCKER: u32 = 0x8000;
    pub const WALL_WEST_PROJ_BLOCKER: u32 = 0x10000;
    pub const LOC_PROJ_BLOCKER: u32 = 0x20000;
    pub const FLOOR_DECORATION: u32 = 0x40000;
    pub const NPC_OCC: u32 = 0x80000;
    pub const BLOCK_NPC_AND_PLAYERS: u32 = 0x100000;
    pub const FLOOR: u32 = 0x200000;
    pub const PLAYER_OCC: u32 = 0x400000;
    pub const ROOF: u32 = 0x80000000;
    pub const FLOOR_BLOCKED: u32 = 0x240000;
    pub const WALK_BLOCKED: u32 = 0x240100;
    pub const BLOCK_WEST: u32 = 0x240108;
    pub const BLOCK_EAST: u32 = 0x240180;
    pub const BLOCK_SOUTH: u32 = 0x240102;
    pub const BLOCK_NORTH: u32 = 0x240120;
    pub const BLOCK_SOUTH_WEST: u32 = 0x24010e;
    pub const BLOCK_SOUTH_EAST: u32 = 0x240183;
    pub const BLOCK_NORTH_WEST: u32 = 0x240138;
    pub const BLOCK_NORTH_EAST: u32 = 0x2401e0;
    pub const BLOCK_NORTH_AND_SOUTH_EAST: u32 = 0x24013e;
    pub const BLOCK_NORTH_AND_SOUTH_WEST: u32 = 0x2401e3;
    pub const BLOCK_NORTH_EAST_AND_WEST: u32 = 0x24018f;
    pub const BLOCK_SOUTH_EAST_AND_WEST: u32 = 0x2401f8;
    pub const NULL: u32 = 0x7fffffff;
}

/// `BlockAccessFlag`.
pub mod block_access {
    pub const BLOCK_NORTH: i32 = 0x1;
    pub const BLOCK_EAST: i32 = 0x2;
    pub const BLOCK_SOUTH: i32 = 0x4;
    pub const BLOCK_WEST: i32 = 0x8;
}

/// `DirectionFlag`.
pub mod direction {
    pub const NORTH: i8 = 0x1;
    pub const EAST: i8 = 0x2;
    pub const SOUTH: i8 = 0x4;
    pub const WEST: i8 = 0x8;
    pub const SOUTH_WEST: i8 = 0xc;
    pub const NORTH_WEST: i8 = 0x9;
    pub const SOUTH_EAST: i8 = 0x6;
    pub const NORTH_EAST: i8 = 0x3;
}

/// `CollisionType`.
pub mod collision_type {
    pub const NORMAL: i32 = 0;
    pub const BLOCKED: i32 = 1;
    pub const INDOORS: i32 = 2;
    pub const OUTDOORS: i32 = 3;
    pub const LINE_OF_SIGHT: i32 = 4;
}

/// `LocAngle`.
pub mod loc_angle {
    pub const WEST: i32 = 0;
    pub const NORTH: i32 = 1;
    pub const EAST: i32 = 2;
    pub const SOUTH: i32 = 3;
}

/// `LocLayer`.
pub mod loc_layer {
    pub const WALL: i32 = 0;
    pub const WALL_DECOR: i32 = 1;
    pub const GROUND: i32 = 2;
    pub const GROUND_DECOR: i32 = 3;
}

/// `LocShape`.
pub mod loc_shape {
    pub const WALL_STRAIGHT: i32 = 0;
    pub const WALL_DIAGONAL_CORNER: i32 = 1;
    pub const WALL_L: i32 = 2;
    pub const WALL_SQUARE_CORNER: i32 = 3;
    pub const WALLDECOR_STRAIGHT_NOOFFSET: i32 = 4;
    pub const WALLDECOR_STRAIGHT_OFFSET: i32 = 5;
    pub const WALLDECOR_DIAGONAL_OFFSET: i32 = 6;
    pub const WALLDECOR_DIAGONAL_NOOFFSET: i32 = 7;
    pub const WALLDECOR_DIAGONAL_BOTH: i32 = 8;
    pub const WALL_DIAGONAL: i32 = 9;
    pub const CENTREPIECE_STRAIGHT: i32 = 10;
    pub const CENTREPIECE_DIAGONAL: i32 = 11;
    pub const ROOF_STRAIGHT: i32 = 12;
    pub const ROOF_DIAGONAL_WITH_ROOFEDGE: i32 = 13;
    pub const ROOF_DIAGONAL: i32 = 14;
    pub const ROOF_L_CONCAVE: i32 = 15;
    pub const ROOF_L_CONVEX: i32 = 16;
    pub const ROOF_FLAT: i32 = 17;
    pub const ROOFEDGE_STRAIGHT: i32 = 18;
    pub const ROOFEDGE_DIAGONAL_CORNER: i32 = 19;
    pub const ROOFEDGE_L: i32 = 20;
    pub const ROOFEDGE_SQUARE_CORNER: i32 = 21;
    pub const GROUND_DECOR: i32 = 22;
}

/// `locShapeLayer(shape)`; an unknown shape is an error.
pub fn loc_shape_layer(shape: i32) -> i32 {
    match shape {
        0..=3 => loc_layer::WALL,
        4..=8 => loc_layer::WALL_DECOR,
        9..=21 => loc_layer::GROUND,
        22 => loc_layer::GROUND_DECOR,
        _ => panic!("Unknown loc shape {shape}"),
    }
}

/// `PackedCoord`: `z | x << 14 | level << 28`, each field masked.
pub fn pack_coord(level: i32, x: i32, z: i32) -> u32 {
    ((z & 0x3fff) | ((x & 0x3fff) << 14) | ((level & 0x3) << 28)) as u32
}

pub fn unpack_coord(packed: u32) -> (i32, i32, i32) {
    (((packed >> 28) & 0x3) as i32, ((packed >> 14) & 0x3fff) as i32, (packed & 0x3fff) as i32)
}

/// `Rotation.rotate`.
pub fn rotate(angle: i32, a: i32, b: i32) -> i32 {
    if angle & 0x1 != 0 { b } else { a }
}

/// `Rotation.rotateFlags`.
pub fn rotate_flags(angle: i32, block_access: i32) -> i32 {
    if angle == 0 {
        return block_access;
    }
    ((block_access.wrapping_shl(angle as u32) & 0xf) as u32 | ((block_access as u32) >> ((4 - angle) as u32 & 31))) as i32
}

const LINE_OF_SIGHT_MOVEMENT: u32 = flag::WALL_NORTH_WEST
    | flag::WALL_NORTH
    | flag::WALL_NORTH_EAST
    | flag::WALL_EAST
    | flag::WALL_SOUTH_EAST
    | flag::WALL_SOUTH
    | flag::WALL_SOUTH_WEST
    | flag::WALL_WEST
    | flag::LOC;

/// `CollisionStrategy.canMove`.
pub fn can_move(collision: i32, tile: u32, block: u32) -> bool {
    match collision {
        collision_type::NORMAL => tile & block == flag::OPEN,
        collision_type::BLOCKED => tile & (block & !flag::FLOOR) == flag::OPEN && tile & flag::FLOOR != flag::OPEN,
        collision_type::INDOORS => tile & block == flag::OPEN && tile & flag::ROOF != flag::OPEN,
        collision_type::OUTDOORS => tile & (block | flag::ROOF) == flag::OPEN,
        collision_type::LINE_OF_SIGHT => tile & ((block & LINE_OF_SIGHT_MOVEMENT) << 9) == flag::OPEN,
        _ => false,
    }
}
