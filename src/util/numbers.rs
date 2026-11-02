//! Number helpers, trig tables and quick sort.

use std::sync::OnceLock;

/// The script VM's double-to-int rule (B.17, B.9; JS `x | 0`): truncate toward
/// zero and wrap modulo 2^32; NaN and ±inf give 0. Not a saturating `as i32`.
pub fn wrap_f64(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    // fmod is exact, so the result fits i64 and wraps like the spec's modulo 2^32.
    (x.trunc() % 4_294_967_296.0) as i64 as u32 as i32
}

/// `MASK[i]`: `(2^i - 1) | 0` for `i` in `0..=32`.
pub fn mask(i: usize) -> i32 {
    if i >= 32 { -1 } else { ((1u64 << i) - 1) as u32 as i32 }
}

/// `bitcount(num)`: set bits of the int32 `num`.
pub fn bitcount(num: i32) -> i32 {
    let mut n = num.wrapping_sub((num >> 1) & 0x5555_5555);
    n = (n & 0x3333_3333).wrapping_add((n >> 2) & 0x3333_3333);
    // the product is below 2^53, then `>> 24` applies ToInt32
    ((((n.wrapping_add(n >> 4)) & 0x0f0f_0f0f) as i64 * 0x0101_0101) as i32) >> 24
}

pub fn set_bit_range(num: i32, start: usize, end: usize) -> i32 {
    num | mask(end - start + 1).wrapping_shl(start as u32)
}

pub fn clear_bit_range(num: i32, start: usize, end: usize) -> i32 {
    num & !mask(end - start + 1).wrapping_shl(start as u32)
}

/// JS `Math.round`: nearest, ties toward +∞.
pub fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

fn tables() -> &'static (Vec<i32>, Vec<i32>) {
    static T: OnceLock<(Vec<i32>, Vec<i32>)> = OnceLock::new();
    T.get_or_init(|| {
        let size = 3.834951969714103e-4;
        let sin = (0..16384).map(|i| ((i as f64 * size).sin() * 16384.0) as i32).collect();
        let cos = (0..16384).map(|i| ((i as f64 * size).cos() * 16384.0) as i32).collect();
        (sin, cos)
    })
}

/// `Trig.sin(x)`: `x & 0x3fff` into the 16384-entry table.
pub fn trig_sin(x: i32) -> i32 {
    tables().0[(x & 0x3fff) as usize]
}

pub fn trig_cos(x: i32) -> i32 {
    tables().1[(x & 0x3fff) as usize]
}

/// `Trig.radians(x)`.
pub fn trig_radians(x: i32) -> f64 {
    ((x & 0x3fff) as f64 / 16384.0) * 6.283185307179586
}

/// `Trig.atan2(y, x)`.
pub fn trig_atan2(y: f64, x: f64) -> i32 {
    js_round(y.atan2(x) * 2607.5945876176133) as i32 & 0x3fff
}

/// `quicksort(low, high, arr, compare)`: the OSRS quicksort (unstable on
/// purpose: the `< (loop_index & 1)` test alternates the tie direction).
pub fn quicksort<T>(low: isize, high: isize, arr: &mut [T], compare: &mut impl FnMut(&T, &T) -> i64) {
    let pivot_index = (low + high) / 2;
    arr.swap(pivot_index as usize, high as usize);
    let mut counter = low;
    let mut loop_index = low;
    while loop_index < high {
        // compare against the pivot, which now sits at `high`
        let (head, tail) = arr.split_at_mut(high as usize);
        if compare(&head[loop_index as usize], &tail[0]) < (loop_index & 1) as i64 {
            head.swap(loop_index as usize, counter as usize);
            counter += 1;
        }
        loop_index += 1;
    }
    arr.swap(high as usize, counter as usize);
    if low < counter - 1 {
        quicksort(low, counter - 1, arr, compare);
    }
    if counter + 1 < high {
        quicksort(counter + 1, high, arr, compare);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wrap_f64_wraps_like_js() {
        use super::wrap_f64;
        assert_eq!(wrap_f64(f64::NAN), 0);
        assert_eq!(wrap_f64(f64::INFINITY), 0);
        assert_eq!(wrap_f64(2147483648.0), -2147483648);
        assert_eq!(wrap_f64(4294967295.0), -1);
        assert_eq!(wrap_f64(-1.9), -1);
        assert_eq!(wrap_f64(-4294967297.0), -1);
        assert_eq!(wrap_f64(1e20), 1661992960);
    }
}
