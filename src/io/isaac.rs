//! The ISAAC stream cipher used for opcode masking.
//!
//! Under JS semantics `a..h`, `a/b/c` are unwrapped numbers, but every value
//! stays far below 2^53 and is only observed through `ToInt32`, `>>>` or an
//! `Int32Array` store, so wrapping `u32` arithmetic is equivalent.

pub struct Isaac {
    count: usize,
    rsl: [u32; 256],
    mem: [u32; 256],
    a: u32,
    b: u32,
    c: u32,
}

fn mix(s: &mut [u32; 8]) {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *s;
    a ^= b << 11; d = d.wrapping_add(a); b = b.wrapping_add(c);
    b ^= c >> 2; e = e.wrapping_add(b); c = c.wrapping_add(d);
    c ^= d << 8; f = f.wrapping_add(c); d = d.wrapping_add(e);
    d ^= e >> 16; g = g.wrapping_add(d); e = e.wrapping_add(f);
    e ^= f << 10; h = h.wrapping_add(e); f = f.wrapping_add(g);
    f ^= g >> 4; a = a.wrapping_add(f); g = g.wrapping_add(h);
    g ^= h << 8; b = b.wrapping_add(g); h = h.wrapping_add(a);
    h ^= a >> 9; c = c.wrapping_add(h); a = a.wrapping_add(b);
    *s = [a, b, c, d, e, f, g, h];
}

impl Isaac {
    /// `new Isaac(seed)`: the seed words fill `rsl[0..]` (the login uses four).
    pub fn new(seed: &[i32]) -> Self {
        let mut r = Isaac { count: 0, rsl: [0; 256], mem: [0; 256], a: 0, b: 0, c: 0 };
        for (slot, &v) in r.rsl.iter_mut().zip(seed) {
            *slot = v as u32;
        }
        let mut s = [0x9e37_79b9u32; 8];
        for _ in 0..4 {
            mix(&mut s);
        }
        for pass in 0..2 {
            for i in (0..256).step_by(8) {
                let src = if pass == 0 { &r.rsl } else { &r.mem };
                for j in 0..8 {
                    s[j] = s[j].wrapping_add(src[i + j]);
                }
                mix(&mut s);
                r.mem[i..i + 8].copy_from_slice(&s);
            }
        }
        r.isaac();
        r.count = 256;
        r
    }

    fn isaac(&mut self) {
        self.c = self.c.wrapping_add(1);
        self.b = self.b.wrapping_add(self.c);
        for i in 0..256 {
            let x = self.mem[i];
            self.a ^= match i & 3 {
                0 => self.a << 13,
                1 => self.a >> 6,
                2 => self.a << 2,
                _ => self.a >> 16,
            };
            self.a = self.a.wrapping_add(self.mem[(i + 128) & 0xff]);
            let y = self.mem[((x >> 2) & 0xff) as usize].wrapping_add(self.a).wrapping_add(self.b);
            self.mem[i] = y;
            self.b = self.mem[((y >> 10) & 0xff) as usize].wrapping_add(x);
            self.rsl[i] = self.b;
        }
    }

    pub fn next_int(&mut self) -> i32 {
        if self.count == 0 {
            self.isaac();
            self.count = 256;
        }
        self.count -= 1;
        self.rsl[self.count] as i32
    }
}
