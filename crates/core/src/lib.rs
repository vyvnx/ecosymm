//! shared primitives: rng, hashing, run configuration, seed derivation.

use serde::{Deserialize, Serialize};

/// splitmix64. deterministic across platforms, no dependency.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// uniform in [0, 1)
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// uniform in [lo, hi)
    pub fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.f32() * (hi - lo)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }

    /// mean 0, sd ~1. irwin-hall(3), enough for mutation noise.
    pub fn normal(&mut self) -> f32 {
        (self.f32() + self.f32() + self.f32() - 1.5) * 2.0
    }
}

/// deterministic hash of one f32, used by replay checksums.
pub fn hash_f32(h: u64, v: f32) -> u64 {
    hash_u64(h, v.to_bits() as u64)
}

/// fnv-1a step over 8 bytes
pub fn hash_u64(h: u64, v: u64) -> u64 {
    hash_bytes(h, &v.to_le_bytes())
}

/// fnv-1a step over arbitrary bytes
pub fn hash_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

pub const HASH_INIT: u64 = 0xcbf2_9ce4_8422_2325;

/// a named substream of the run seed. world construction, each species'
/// founders and each engine stream draw their own, so adding one founder field
/// cannot silently shift every later random draw.
pub fn derive_seed(seed: u64, name: &str) -> u64 {
    hash_bytes(hash_u64(HASH_INIT, seed), name.as_bytes())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimConfig {
    pub seed: u64,
    /// founders spawned for each species in the scenario
    pub population_per_species: usize,
    /// batches of simulation ticks to run. simulation time, not generations.
    pub epochs: usize,
    pub width: usize,
    pub height: usize,
    /// simulation steps inside one epoch
    pub ticks_per_epoch: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            seed: 1234,
            population_per_species: 500,
            epochs: 500,
            width: 128,
            height: 128,
            ticks_per_epoch: 20,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.f32();
            assert_eq!(x, b.f32());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn derived_streams_are_stable_and_distinct() {
        assert_eq!(derive_seed(7, "world"), derive_seed(7, "world"));
        assert_ne!(derive_seed(7, "world"), derive_seed(7, "engine"));
        assert_ne!(derive_seed(7, "world"), derive_seed(8, "world"));
        // adjacent species streams must not collide
        assert_ne!(derive_seed(7, "species:0"), derive_seed(7, "species:1"));
    }
}
