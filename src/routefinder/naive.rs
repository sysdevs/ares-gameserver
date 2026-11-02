//! NPC-style "step towards the target" pathing.

use super::{CollisionEngine, pack_coord, step::can_travel, unpack_coord};

const DIRECTIONS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, 1), (0, -1)];

/// `findNaivePath`. `random` stands in for `Math.random()` (one draw, only
/// when source and target overlap).
#[allow(clippy::too_many_arguments)]
pub fn find_naive_path(
    f: &CollisionEngine,
    y: i32,
    sx: i32,
    sz: i32,
    dx: i32,
    dz: i32,
    sw: i32,
    sh: i32,
    dw: i32,
    dh: i32,
    extra: u32,
    collision: i32,
    random: &mut dyn FnMut() -> f64,
) -> Vec<u32> {
    if intersects(sx, sz, sw, sh, dx, dz, dw, dh) {
        let (ox, oz) = DIRECTIONS[(random() * DIRECTIONS.len() as f64) as usize];
        return vec![pack_coord(y, sx + ox, sz + oz)];
    }
    let dest = naive_destination(y, sx, sz, sw, sh, dx, dz, 1, 1);
    // an empty result reads `dest[0]` as undefined, which packs to 0
    let (_, nx, nz) = unpack_coord(dest.first().copied().unwrap_or(0));
    if is_diagonal(nx, nz, sw, sh, dx, dz, dw, dh) || intersects(nx, nz, sw, sh, dx, dz, dw, dh) {
        return dest;
    }
    let (mut cx, mut cz) = (nx, nz);
    while cx != dx && cz != dz {
        let step_x = (dx - cx).signum();
        let step_z = (dz - cz).signum();
        if can_travel(f, y, cx, cz, step_x, step_z, sw, extra, collision) {
            cx += step_x;
            cz += step_z;
        } else if step_x != 0 && can_travel(f, y, cx, cz, step_x, 0, sw, extra, collision) {
            cx += step_x;
        } else if step_z != 0 && can_travel(f, y, cx, cz, 0, step_z, sw, extra, collision) {
            cz += step_z;
        } else {
            break;
        }
    }
    vec![pack_coord(y, cx, cz)]
}

#[allow(clippy::too_many_arguments)]
fn intersects(sx: i32, sz: i32, sw: i32, sh: i32, dx: i32, dz: i32, dw: i32, dh: i32) -> bool {
    !(dx >= sx + sw || dx + dw <= sx || dz >= sz + sh || dz + dh <= sz)
}

#[allow(clippy::too_many_arguments)]
fn is_diagonal(sx: i32, sz: i32, sw: i32, sh: i32, dx: i32, dz: i32, dw: i32, dh: i32) -> bool {
    (sx + sw == dx && sz + sh == dz)
        || (sx - 1 == dx + dw - 1 && sz - 1 == dz + dh - 1)
        || (sx + sw == dx && sz - 1 == dz + dh - 1)
        || (sx - 1 == dx + dw - 1 && sz + sh == dz)
}

#[allow(clippy::too_many_arguments)]
fn naive_destination(y: i32, sx: i32, sz: i32, sw: i32, sh: i32, dx: i32, dz: i32, dw: i32, dh: i32) -> Vec<u32> {
    let diagonal = sx - dx + (sz - dz);
    let anti = sx - dx - (sz - dz);
    let south_west_cw = anti < 0;
    let north_west_cw = diagonal >= dh - 1 - (sw - 1);
    let north_east_cw = anti > sw - sh;
    let south_east_cw = diagonal <= dw - 1 - (sh - 1);

    if south_west_cw && !north_west_cw {
        let mut off_z = 0;
        if diagonal >= -sw {
            off_z = (diagonal + sw).min(dh - 1);
        } else if anti > -sw {
            off_z = -(sw + anti);
        }
        return vec![pack_coord(y, -sw + dx, off_z + dz)];
    }
    if north_west_cw && !north_east_cw {
        let mut off_x = 0;
        if anti >= -dh {
            off_x = (anti + dh).min(dw - 1);
        } else if diagonal < dh {
            off_x = (diagonal - dh).max(-(sw - 1));
        }
        return vec![pack_coord(y, off_x + dx, dh + dz)];
    }
    if north_east_cw && !south_east_cw {
        let mut off_z = 0;
        if anti <= dw {
            off_z = dh - anti;
        } else if diagonal < dw {
            off_z = (diagonal - dw).max(-(sh - 1));
        }
        return vec![pack_coord(y, dw + dx, off_z + dz)];
    }
    if !(south_east_cw && !south_west_cw) {
        return Vec::new();
    }
    let mut off_x = 0;
    if diagonal > -sh {
        off_x = (diagonal + sh).min(dw - 1);
    } else if anti < sh {
        off_x = (anti - sh).max(-(sh - 1));
    }
    vec![pack_coord(y, off_x + dx, -sh + dz)]
}
