//! NumPy-compatible `np.random.default_rng(entropy)` for the few calls the ported nodes make: `SeedSequence` (pool size
//! 4, no spawn key), the PCG64 bit generator (XSL-RR 128/64) and `Generator.random()` (53-bit doubles). Values are
//! bit-identical to NumPy (checked against NumPy 2.x output in the tests).

const INIT_A: u32 = 0x43b0_d7e5;
const MULT_A: u32 = 0x931e_8875;
const INIT_B: u32 = 0x8b51_f9dd;
const MULT_B: u32 = 0x58f3_8ded;
const MIX_MULT_L: u32 = 0xca01_f9dd;
const MIX_MULT_R: u32 = 0x4973_f715;
const XSHIFT: u32 = 16;
const POOL: usize = 4;

/// `SeedSequence(entropy).generate_state(n_words, uint32)` where `entropy` is a list of non-negative ints.
fn seed_state(entropy: &[u64], n_words: usize) -> Vec<u32> {
    // _coerce_to_uint32_array: each int becomes its 32-bit words, least significant first (0 -> [0])
    let mut words = Vec::new();
    for &e in entropy {
        if e == 0 {
            words.push(0);
        }
        let mut n = e;
        while n > 0 {
            words.push(n as u32);
            n >>= 32;
        }
    }
    let mut hash_const = INIT_A;
    let mut hashmix = |value: u32| -> u32 {
        let mut v = value ^ hash_const;
        hash_const = hash_const.wrapping_mul(MULT_A);
        v = v.wrapping_mul(hash_const);
        v ^ (v >> XSHIFT)
    };
    let mix = |x: u32, y: u32| -> u32 {
        let r = MIX_MULT_L.wrapping_mul(x).wrapping_sub(MIX_MULT_R.wrapping_mul(y));
        r ^ (r >> XSHIFT)
    };
    let mut pool = [0u32; POOL];
    for (i, p) in pool.iter_mut().enumerate() {
        *p = hashmix(words.get(i).copied().unwrap_or(0));
    }
    for src in 0..POOL {
        for dst in 0..POOL {
            if src != dst {
                let h = hashmix(pool[src]);
                pool[dst] = mix(pool[dst], h);
            }
        }
    }
    for &w in words.iter().skip(POOL) {
        for dst in 0..POOL {
            let h = hashmix(w);
            pool[dst] = mix(pool[dst], h);
        }
    }
    let mut hash_const = INIT_B;
    (0..n_words)
        .map(|i| {
            let mut v = pool[i % POOL] ^ hash_const;
            hash_const = hash_const.wrapping_mul(MULT_B);
            v = v.wrapping_mul(hash_const);
            v ^ (v >> XSHIFT)
        })
        .collect()
}

const PCG_MULT: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

/// `np.random.default_rng(entropy)` (PCG64).
pub(super) struct Rng {
    state: u128,
    inc: u128,
    has_u32: bool,
    uinteger: u32,
}

impl Rng {
    pub(super) fn new(entropy: &[u64]) -> Rng {
        let w = seed_state(entropy, 8);
        let u = |i: usize| (w[2 * i] as u64) | ((w[2 * i + 1] as u64) << 32);
        let seed = ((u(0) as u128) << 64) | u(1) as u128;
        let initseq = ((u(2) as u128) << 64) | u(3) as u128;
        let mut r = Rng { state: 0, inc: (initseq << 1) | 1, has_u32: false, uinteger: 0 };
        r.step();
        r.state = r.state.wrapping_add(seed);
        r.step();
        r
    }

    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.step();
        let s = self.state;
        ((s >> 64) as u64 ^ s as u64).rotate_right((s >> 122) as u32)
    }

    pub(super) fn next_u32(&mut self) -> u32 {
        if self.has_u32 {
            self.has_u32 = false;
            self.uinteger
        } else {
            let next = self.next_u64();
            self.has_u32 = true;
            self.uinteger = (next >> 32) as u32;
            next as u32
        }
    }

    /// `Generator.random()`: a double in [0, 1).
    pub(super) fn random(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }

    pub(super) fn random_interval(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        let mut mask = max;
        mask |= mask >> 1;
        mask |= mask >> 2;
        mask |= mask >> 4;
        mask |= mask >> 8;
        mask |= mask >> 16;
        mask |= mask >> 32;
        if max <= 0xFFFFFFFF {
            loop {
                let val = (self.next_u32() as usize) & mask;
                if val <= max {
                    return val;
                }
            }
        } else {
            loop {
                let val = (self.next_u64() as usize) & mask;
                if val <= max {
                    return val;
                }
            }
        }
    }

    pub(super) fn permutation(&mut self, n: usize) -> Vec<usize> {
        let mut arr: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = self.random_interval(i);
            arr.swap(i, j);
        }
        arr
    }

    pub(super) fn standard_normal(&mut self) -> f64 {
        let u1 = self.random().max(1e-15);
        let u2 = self.random();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// `Generator.integers(0, high)` for `high` <= 2^32, using Daniel Lemire's algorithm matching NumPy.
    pub(super) fn integers(&mut self, high: usize) -> usize {
        if high <= 1 {
            return 0;
        }
        let range = high as u32;
        let threshold = (0u32.wrapping_sub(range)) % range;
        let mut u = self.next_u32();
        let mut multi = (u as u64) * (range as u64);
        let mut leftover = multi as u32;
        if leftover < range {
            while leftover < threshold {
                u = self.next_u32();
                multi = (u as u64) * (range as u64);
                leftover = multi as u32;
            }
        }
        (multi >> 32) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn matches_numpy_default_rng() {
        // np.random.default_rng([5, 53]).random(3)
        let mut r = Rng::new(&[5, 53]);
        let got: Vec<f64> = (0..3).map(|_| r.random()).collect();
        assert_eq!(got, [0.174991606238858, 0.6089609637200527, 0.2133979172442666]);
    }

    #[test]
    fn matches_numpy_permutation() {
        let mut r = Rng::new(&[0]);
        let got = r.permutation(10);
        assert_eq!(got, [4, 6, 2, 7, 3, 5, 9, 0, 8, 1]);
    }

    #[test]
    fn matches_numpy_integers() {
        let mut r = Rng::new(&[0]);
        let got: Vec<usize> = (0..5).map(|_| r.integers(3)).collect();
        assert_eq!(got, [2, 1, 1, 0, 0]);
    }
}
