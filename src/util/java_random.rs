//! java.util.Random (B.16): Java's 48-bit LCG. `nextInt(bound)`'s rejection
//! test can never be true in JS (no int overflow), so it is `bits % bound`.
//! The seed is module-global: every `new JavaRandom(seed)` reseeds the one
//! shared generator; the engine owns a single instance.

const MULT: u64 = 0x5_deec_e66d;
const MASK: u64 = (1 << 48) - 1;

#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom { seed: 0 };
        r.set_seed(seed);
        r
    }

    /// `setSeed(n)`: `checkIsPositiveInt` throws on negative seeds; the low 48 bits of `n`.
    pub fn set_seed(&mut self, n: i64) {
        assert!((0..=9007199254740991).contains(&n), "RangeError: number must be > 0");
        self.seed = (n as u64 ^ MULT) & MASK;
    }

    /// `_next()`: the new state's top 32 bits.
    fn step(&mut self) -> u64 {
        self.seed = (self.seed.wrapping_mul(MULT) + 0xb) & MASK;
        self.seed >> 16
    }

    fn next_signed(&mut self, bits: u32) -> i32 {
        (self.step() as u32 as i32) >> (32 - bits)
    }

    fn next(&mut self, bits: u32) -> u32 {
        (self.step() as u32) >> (32 - bits)
    }

    /// `nextInt()` without a bound.
    pub fn next_int(&mut self) -> i32 {
        self.next_signed(32)
    }

    /// `nextInt(bound)`; panics like `checkIsPositiveInt` outside `0..=0x7fffffff`.
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound >= 0, "RangeError: number must be > 0");
        if bound & bound.wrapping_neg() == bound {
            // `bound * (next(31) / 2^31)`, exact in a double
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        // `bits - val + (bound - 1) < 0` never holds for JS numbers
        (self.next(31) % bound as u32) as i32
    }

    pub fn next_long(&mut self) -> i64 {
        let msb = self.next_signed(32) as i64;
        let lsb = self.next_signed(32) as i64;
        msb * 4294967296 + lsb
    }

    pub fn next_boolean(&mut self) -> bool {
        self.next(1) != 0
    }

    pub fn next_float(&mut self) -> f64 {
        self.next(24) as f64 / 16777216.0
    }

    pub fn next_double(&mut self) -> f64 {
        (134217728.0 * self.next(26) as f64 + self.next(27) as f64) / 9007199254740992.0
    }
}
