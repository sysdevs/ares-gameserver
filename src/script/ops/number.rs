//! Integer ops computed as JS doubles and then `ToInt32`
//! (B.17): a product past 2^53 loses its low bits before wrapping, division
//! or modulo by zero is 0, `pow` overflow wraps through `ToInt32`.

use super::super::runner::OpResult;
use super::super::state::ScriptState;
use ares_shared::opcode::op;
use crate::engine::world::World;
use crate::util::numbers::{bitcount, trig_atan2, trig_cos, trig_sin};

/// `MASK[i]`, which is `undefined` (0 in a shift) outside `0..=32`.
fn mask(i: i32) -> i32 {
    match i {
        0..=31 => ((1u64 << i) - 1) as u32 as i32,
        32 => -1,
        _ => 0,
    }
}

/// Bit range set/clear with JS shift-count masking.
fn set_bit_range(num: i32, start: i32, end: i32) -> i32 {
    num | mask(end - start + 1).wrapping_shl(start as u32)
}

fn clear_bit_range(num: i32, start: i32, end: i32) -> i32 {
    num & !mask(end - start + 1).wrapping_shl(start as u32)
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    let f = |v: i32| v as f64;
    match opcode {
        op::ADD | op::SUB | op::MULTIPLY | op::DIVIDE => {
            let b = s.pop_int();
            let a = s.pop_int();
            s.push_f64(match opcode {
                op::ADD => f(a) + f(b),
                op::SUB => f(a) - f(b),
                op::MULTIPLY => f(a) * f(b),
                _ => f(a) / f(b),
            });
        }
        op::RANDOM => {
            let n = s.pop_int();
            let r = w.random.next_double();
            s.push_f64(r * f(n));
        }
        op::RANDOMINC => {
            let n = s.pop_int();
            let r = w.random.next_double();
            s.push_f64(r * (f(n) + 1.0));
        }
        op::INTERPOLATE => {
            let [y0, y1, x0, x1, x] = s.pop_ints::<5>();
            s.push_f64(((f(y1) - f(y0)) / (f(x1) - f(x0))).floor() * (f(x) - f(x0)) + f(y0));
        }
        op::ADDPERCENT => {
            let [num, percent] = s.pop_ints::<2>();
            s.push_f64((f(num) * f(percent)) / 100.0 + f(num));
        }
        op::SETBIT | op::CLEARBIT | op::TESTBIT | op::TOGGLEBIT => {
            let [value, bit] = s.pop_ints::<2>();
            let m = 1i32.wrapping_shl(bit as u32);
            s.push_int(match opcode {
                op::SETBIT => value | m,
                op::CLEARBIT => value & !m,
                op::TESTBIT => (value & m != 0) as i32,
                _ => value ^ m,
            });
        }
        op::MODULO => {
            let [a, b] = s.pop_ints::<2>();
            s.push_f64(f(a) % f(b));
        }
        op::POW => {
            let [base, exponent] = s.pop_ints::<2>();
            s.push_f64(f(base).powf(f(exponent)));
        }
        op::INVPOW => {
            let [n1, n2] = s.pop_ints::<2>();
            s.push_f64(if n1 == 0 || n2 == 0 {
                0.0
            } else {
                match n2 {
                    1 => f(n1),
                    2 => f(n1).sqrt(),
                    3 => f(n1).cbrt(),
                    4 => f(n1).sqrt().sqrt(),
                    _ => f(n1).powf(1.0 / f(n2)),
                }
            });
        }
        op::AND | op::OR | op::MIN | op::MAX => {
            let [a, b] = s.pop_ints::<2>();
            s.push_int(match opcode {
                op::AND => a & b,
                op::OR => a | b,
                op::MIN => a.min(b),
                _ => a.max(b),
            });
        }
        op::SCALE => {
            let [a, b, c] = s.pop_ints::<3>();
            s.push_f64((f(a) * f(c)) / f(b));
        }
        op::BITCOUNT => {
            let v = s.pop_int();
            s.push_int(bitcount(v));
        }
        op::SETBIT_RANGE => {
            let [num, start, end] = s.pop_ints::<3>();
            s.push_int(set_bit_range(num, start, end));
        }
        op::CLEARBIT_RANGE => {
            let [num, start, end] = s.pop_ints::<3>();
            s.push_int(clear_bit_range(num, start, end));
        }
        op::GETBIT_RANGE => {
            let [num, start, end] = s.pop_ints::<3>();
            let a = 31 - end;
            s.push_int((num.wrapping_shl(a as u32) as u32).wrapping_shr((start + a) as u32) as i32);
        }
        op::SETBIT_RANGE_TOINT => {
            let [num, value, start, end] = s.pop_ints::<4>();
            let cleared = clear_bit_range(num, start, end);
            // `MASK[i]` is undefined out of range, and `value > undefined` is false
            let max = (0..=32).contains(&(end - start + 1)).then(|| mask(end - start + 1));
            let assign = match max {
                Some(m) if value > m => m,
                _ => value,
            };
            s.push_int(cleared | assign.wrapping_shl(start as u32));
        }
        op::SIN_DEG => {
            let v = s.pop_int();
            s.push_int(trig_sin(v));
        }
        op::COS_DEG => {
            let v = s.pop_int();
            s.push_int(trig_cos(v));
        }
        op::ATAN2_DEG => {
            // `Trig.atan2(popInt(), popInt())`: y is the top of the stack
            let y = s.pop_int();
            let x = s.pop_int();
            s.push_int(trig_atan2(f(y), f(x)));
        }
        op::ABS => {
            let v = s.pop_int();
            s.push_int(v.wrapping_abs());
        }
        op::DATE_MINUTES => {
            s.push_int(w.clock_ms().div_euclid(60000) as i32);
        }
        op::DATE_RUNEDAY => {
            s.push_int((w.clock_ms().div_euclid(86400000) - 11745) as i32);
        }
        _ => return None,
    }
    Some(Ok(()))
}
