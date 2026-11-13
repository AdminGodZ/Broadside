//! xoshiro256++: small, fast, statistically solid. Seeded via SplitMix64 so any
//! `u64` (including 0) gives a good, reproducible stream.

#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        Self {
            s: [(); 4].map(|_| splitmix(&mut x)),
        }
    }

    /// Independent stream number `i` derived from `seed` (e.g. one per game).
    pub fn stream(seed: u64, i: u64) -> Self {
        let mut x = seed ^ i.wrapping_mul(0xD1B5_4A32_D192_ED03);
        Self::new(splitmix(&mut x))
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let out = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    /// Uniform in `0..n` (Lemire's nearly-divisionless method). `n` must be > 0.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        let mut m = (self.next_u64() as u128) * (n as u128);
        if (m as u64) < n {
            let threshold = n.wrapping_neg() % n;
            while (m as u64) < threshold {
                m = (self.next_u64() as u128) * (n as u128);
            }
        }
        (m >> 64) as u64
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniformly random set bit of a non-empty mask.
    #[inline]
    pub fn pick_bit(&mut self, mask: u128) -> usize {
        let n = mask.count_ones() as u64;
        debug_assert!(n > 0);
        let mut k = self.below(n);
        let mut m = mask;
        while k > 0 {
            m &= m - 1;
            k -= 1;
        }
        m.trailing_zeros() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_is_in_range_and_covers() {
        let mut r = Rng::new(1);
        let mut seen = [0u32; 7];
        for _ in 0..7000 {
            seen[r.below(7) as usize] += 1;
        }
        assert!(seen.iter().all(|&c| c > 800), "{seen:?}");
    }

    #[test]
    fn streams_differ() {
        assert_ne!(Rng::stream(5, 0).next_u64(), Rng::stream(5, 1).next_u64());
    }

    #[test]
    fn pick_bit_picks_set_bits() {
        let mut r = Rng::new(9);
        let mask = (1u128 << 3) | (1 << 77) | (1 << 99);
        for _ in 0..100 {
            assert!(mask >> r.pick_bit(mask) & 1 == 1);
        }
    }
}
