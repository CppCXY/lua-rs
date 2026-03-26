/// xoshiro256** RNG matching C Lua's implementation exactly
#[derive(Debug, Clone)]
pub struct LuaRng {
    pub state: [u64; 4],
}

impl LuaRng {
    /// Seed from two integers, matching C Lua's setseed
    pub fn from_seed(n1: i64, n2: i64) -> Self {
        let mut rng = LuaRng {
            state: [n1 as u64, 0xff, n2 as u64, 0],
        };
        // Warm up: discard 16 values to spread the seed
        for _ in 0..16 {
            rng.next_rand();
        }
        rng
    }

    /// Seed from a time value (for default initialization)
    pub fn from_seed_time(time: u64) -> Self {
        Self::from_seed(time as i64, 0)
    }

    /// Generate next random u64 using xoshiro256**
    pub fn next_rand(&mut self) -> u64 {
        let s = &mut self.state;
        let s0 = s[0];
        let s1 = s[1];
        let s2 = s[2] ^ s0;
        let s3 = s[3] ^ s1;
        // result = s1 * 5, rotate left 7, then * 9
        let res = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        s[0] = s0 ^ s3;
        s[1] = s1 ^ s2;
        s[2] = s2 ^ (s1 << 17);
        s[3] = s3.rotate_left(45);
        res
    }

    /// Convert random u64 to float in [0, 1)
    /// Takes the top 53 bits (DBL_MANT_DIG) and scales to [0,1)
    pub fn next_float(&mut self) -> f64 {
        let rv = self.next_rand();
        // Take top 53 bits
        let mantissa = rv >> (64 - 53); // = rv >> 11
        (mantissa as f64) * f64::from_bits(0x3CA0000000000000) // 2^-53
    }
}
