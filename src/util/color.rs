//! Colour conversion. All arithmetic stays in `f64` in a fixed operation
//! order and converts with `| 0` at fixed points (B.9).

use crate::util::numbers::wrap_f64;
use std::sync::OnceLock;

pub fn hsl24to16(hue: i32, mut saturation: i32, lightness: i32) -> i32 {
    if lightness > 243 {
        saturation >>= 4;
    } else if lightness > 217 {
        saturation >>= 3;
    } else if lightness > 192 {
        saturation >>= 2;
    } else if lightness > 179 {
        saturation >>= 1;
    }
    (((hue & 0xff) >> 2) << 10) + ((saturation >> 5) << 7) + (lightness >> 1)
}

pub fn rgb15to24(rgb: i32) -> i32 {
    let r = (rgb >> 10) & 0x1f;
    let g = (rgb >> 5) & 0x1f;
    let b = rgb & 0x1f;
    ((r << 3) << 16) + ((g << 3) << 8) + (b << 3)
}

pub fn rgb15_to_hsl16(rgb: i32) -> i32 {
    let r = ((rgb >> 10) & 0x1f) as f64;
    let g = ((rgb >> 5) & 0x1f) as f64;
    let b = (rgb & 0x1f) as f64;
    rgb_to_hsl(r / 31.0, g / 31.0, b / 31.0)
}

pub fn rgb24to15(rgb: i32) -> i32 {
    let r = (rgb >> 16) & 0xff;
    let g = (rgb >> 8) & 0xff;
    let b = rgb & 0xff;
    ((r >> 3) << 10) + ((g >> 3) << 5) + (b >> 3)
}

pub fn rgb24_to_hsl16(rgb: i32) -> i32 {
    let r = ((rgb >> 16) & 0xff) as f64;
    let g = ((rgb >> 8) & 0xff) as f64;
    let b = (rgb & 0xff) as f64;
    rgb_to_hsl(r / 256.0, g / 256.0, b / 256.0)
}

pub fn rgb_to_hsl(red: f64, green: f64, blue: f64) -> i32 {
    let mut min = red;
    if green < min {
        min = green;
    }
    if blue < min {
        min = blue;
    }
    let mut max = red;
    if green > max {
        max = green;
    }
    if blue > max {
        max = blue;
    }

    let mut h_norm = 0.0;
    let mut s_norm = 0.0;
    let l_norm = (min + max) / 2.0;

    if min != max {
        if l_norm < 0.5 {
            s_norm = (max - min) / (max + min);
        } else if l_norm >= 0.5 {
            s_norm = (max - min) / (2.0 - max - min);
        }

        if red == max {
            h_norm = (green - blue) / (max - min);
        } else if green == max {
            h_norm = (blue - red) / (max - min) + 2.0;
        } else if blue == max {
            h_norm = (red - green) / (max - min) + 4.0;
        }
    }

    h_norm /= 6.0;

    let hue = wrap_f64(h_norm * 256.0);
    let saturation = wrap_f64(s_norm * 256.0).clamp(0, 255);
    let lightness = wrap_f64(l_norm * 256.0).clamp(0, 255);
    hsl24to16(hue, saturation, lightness)
}

/// `ColorConversion.RGB15_HSL16`.
pub fn rgb15_hsl16_table() -> &'static [i32; 32768] {
    static TABLE: OnceLock<Box<[i32; 32768]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([0; 32768]);
        for (rgb, v) in t.iter_mut().enumerate() {
            *v = rgb15_to_hsl16(rgb as i32);
        }
        t
    })
}

/// `ColorConversion.reverseHsl`: every rgb15 mapping to `hsl`, ascending.
pub fn reverse_hsl(hsl: i32) -> Vec<i32> {
    (0..32768).filter(|&rgb| rgb15_hsl16_table()[rgb as usize] == hsl).collect()
}
