//! The experience table.

use std::sync::OnceLock;

fn table() -> &'static [i32; 99] {
    static T: OnceLock<[i32; 99]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0; 99];
        let mut acc = 0.0;
        for (i, slot) in t.iter_mut().enumerate() {
            let level = (i + 1) as f64;
            acc += (level + 2f64.powf(level / 7.0) * 300.0).floor();
            *slot = (acc / 4.0).floor() as i32 * 10;
        }
        t
    })
}

pub fn get_level_by_exp(exp: i32) -> i32 {
    (0..99).rev().find(|&i| exp >= table()[i]).map_or(1, |i| (i as i32 + 2).min(99))
}

/// `getExpByLevel(level)` for `level` in `2..=100` (`levelExperience[level - 2]`).
pub fn get_exp_by_level(level: i32) -> i32 {
    table()[(level - 2) as usize]
}
