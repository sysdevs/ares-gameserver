//! Step validation: can an entity of `size` take one step `(dx, dz)`.

use super::{CollisionEngine, can_move, flag};

#[allow(clippy::too_many_arguments)]
pub fn can_travel(flags: &CollisionEngine, y: i32, x: i32, z: i32, dx: i32, dz: i32, size: i32, extra: u32, collision: i32) -> bool {
    let blocked = match (dx, dz) {
        (0, -1) => south(flags, y, x, z, size, extra, collision),
        (0, 1) => north(flags, y, x, z, size, extra, collision),
        (-1, 0) => west(flags, y, x, z, size, extra, collision),
        (1, 0) => east(flags, y, x, z, size, extra, collision),
        (-1, -1) => south_west(flags, y, x, z, size, extra, collision),
        (-1, 1) => north_west(flags, y, x, z, size, extra, collision),
        (1, -1) => south_east(flags, y, x, z, size, extra, collision),
        (1, 1) => north_east(flags, y, x, z, size, extra, collision),
        _ => return false,
    };
    !blocked
}

/// `!canMove(collision, flags.get(x, z, y), block | extra)`.
fn no(flags: &CollisionEngine, collision: i32, x: i32, z: i32, y: i32, block: u32, extra: u32) -> bool {
    !can_move(collision, flags.get(x, z, y), block | extra)
}

fn south(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x, z - 1, y, flag::BLOCK_SOUTH, e),
        2 => no(f, c, x, z - 1, y, flag::BLOCK_SOUTH_WEST, e) || no(f, c, x + 1, z - 1, y, flag::BLOCK_SOUTH_EAST, e),
        _ => {
            no(f, c, x, z - 1, y, flag::BLOCK_SOUTH_WEST, e)
                || no(f, c, x + size - 1, z - 1, y, flag::BLOCK_SOUTH_EAST, e)
                || (x + 1..x + size - 1).any(|mx| no(f, c, mx, z - 1, y, flag::BLOCK_NORTH_EAST_AND_WEST, e))
        }
    }
}

fn north(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x, z + 1, y, flag::BLOCK_NORTH, e),
        2 => no(f, c, x, z + 2, y, flag::BLOCK_NORTH_WEST, e) || no(f, c, x + 1, z + 2, y, flag::BLOCK_NORTH_EAST, e),
        _ => {
            no(f, c, x, z + size, y, flag::BLOCK_NORTH_WEST, e)
                || no(f, c, x + size - 1, z + size, y, flag::BLOCK_NORTH_EAST, e)
                || (x + 1..x + size - 1).any(|mx| no(f, c, mx, z + size, y, flag::BLOCK_SOUTH_EAST_AND_WEST, e))
        }
    }
}

fn west(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x - 1, z, y, flag::BLOCK_WEST, e),
        2 => no(f, c, x - 1, z, y, flag::BLOCK_SOUTH_WEST, e) || no(f, c, x - 1, z + 1, y, flag::BLOCK_NORTH_WEST, e),
        _ => {
            no(f, c, x - 1, z, y, flag::BLOCK_SOUTH_WEST, e)
                || no(f, c, x - 1, z + size - 1, y, flag::BLOCK_NORTH_WEST, e)
                || (z + 1..z + size - 1).any(|mz| no(f, c, x - 1, mz, y, flag::BLOCK_NORTH_AND_SOUTH_EAST, e))
        }
    }
}

fn east(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x + 1, z, y, flag::BLOCK_EAST, e),
        2 => no(f, c, x + 2, z, y, flag::BLOCK_SOUTH_EAST, e) || no(f, c, x + 2, z + 1, y, flag::BLOCK_NORTH_EAST, e),
        _ => {
            no(f, c, x + size, z, y, flag::BLOCK_SOUTH_EAST, e)
                || no(f, c, x + size, z + size - 1, y, flag::BLOCK_NORTH_EAST, e)
                || (z + 1..z + size - 1).any(|mz| no(f, c, x + size, mz, y, flag::BLOCK_NORTH_AND_SOUTH_WEST, e))
        }
    }
}

fn south_west(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x - 1, z - 1, y, flag::BLOCK_SOUTH_WEST, e) || no(f, c, x - 1, z, y, flag::BLOCK_WEST, e) || no(f, c, x, z - 1, y, flag::BLOCK_SOUTH, e),
        2 => {
            no(f, c, x - 1, z, y, flag::BLOCK_NORTH_AND_SOUTH_EAST, e)
                || no(f, c, x - 1, z - 1, y, flag::BLOCK_SOUTH_WEST, e)
                || no(f, c, x, z - 1, y, flag::BLOCK_NORTH_EAST_AND_WEST, e)
        }
        _ => {
            no(f, c, x - 1, z - 1, y, flag::BLOCK_SOUTH_WEST, e)
                || (1..size).any(|m| {
                    no(f, c, x - 1, z + m - 1, y, flag::BLOCK_NORTH_AND_SOUTH_EAST, e) || no(f, c, x + m - 1, z - 1, y, flag::BLOCK_NORTH_EAST_AND_WEST, e)
                })
        }
    }
}

fn north_west(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x - 1, z + 1, y, flag::BLOCK_NORTH_WEST, e) || no(f, c, x - 1, z, y, flag::BLOCK_WEST, e) || no(f, c, x, z + 1, y, flag::BLOCK_NORTH, e),
        2 => {
            no(f, c, x - 1, z + 1, y, flag::BLOCK_NORTH_AND_SOUTH_EAST, e)
                || no(f, c, x - 1, z + 2, y, flag::BLOCK_NORTH_WEST, e)
                || no(f, c, x, z + 2, y, flag::BLOCK_SOUTH_EAST_AND_WEST, e)
        }
        _ => {
            no(f, c, x - 1, z + size, y, flag::BLOCK_NORTH_WEST, e)
                || (1..size).any(|m| {
                    no(f, c, x - 1, z + m, y, flag::BLOCK_NORTH_AND_SOUTH_EAST, e) || no(f, c, x + m - 1, z + size, y, flag::BLOCK_SOUTH_EAST_AND_WEST, e)
                })
        }
    }
}

fn south_east(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x + 1, z - 1, y, flag::BLOCK_SOUTH_EAST, e) || no(f, c, x + 1, z, y, flag::BLOCK_EAST, e) || no(f, c, x, z - 1, y, flag::BLOCK_SOUTH, e),
        2 => {
            no(f, c, x + 1, z - 1, y, flag::BLOCK_NORTH_EAST_AND_WEST, e)
                || no(f, c, x + 2, z - 1, y, flag::BLOCK_SOUTH_EAST, e)
                || no(f, c, x + 2, z, y, flag::BLOCK_NORTH_AND_SOUTH_WEST, e)
        }
        _ => {
            no(f, c, x + size, z - 1, y, flag::BLOCK_SOUTH_EAST, e)
                || (1..size).any(|m| {
                    no(f, c, x + size, z + m - 1, y, flag::BLOCK_NORTH_AND_SOUTH_WEST, e) || no(f, c, x + m, z - 1, y, flag::BLOCK_NORTH_EAST_AND_WEST, e)
                })
        }
    }
}

fn north_east(f: &CollisionEngine, y: i32, x: i32, z: i32, size: i32, e: u32, c: i32) -> bool {
    match size {
        1 => no(f, c, x + 1, z + 1, y, flag::BLOCK_NORTH_EAST, e) || no(f, c, x + 1, z, y, flag::BLOCK_EAST, e) || no(f, c, x, z + 1, y, flag::BLOCK_NORTH, e),
        2 => {
            no(f, c, x + 1, z + 2, y, flag::BLOCK_SOUTH_EAST_AND_WEST, e)
                || no(f, c, x + 2, z + 2, y, flag::BLOCK_NORTH_EAST, e)
                || no(f, c, x + 2, z + 1, y, flag::BLOCK_NORTH_AND_SOUTH_WEST, e)
        }
        _ => {
            no(f, c, x + size, z + size, y, flag::BLOCK_NORTH_EAST, e)
                || (1..size).any(|m| {
                    no(f, c, x + m, z + size, y, flag::BLOCK_SOUTH_EAST_AND_WEST, e) || no(f, c, x + size, z + m, y, flag::BLOCK_NORTH_AND_SOUTH_WEST, e)
                })
        }
    }
}
