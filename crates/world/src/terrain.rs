//! elevation, wetness and fertility, generated from the seed alone.

use ecosym_core::{hash_u64, HASH_INIT};

/// elevation below this is ocean: barren, and it stays that way all run
pub const SEA_LEVEL: f32 = 0.42;

pub struct Terrain {
    /// ceiling on how much resource a tile can hold
    pub fertility: Vec<f32>,
    pub elevation: Vec<f32>,
    pub wetness: Vec<f32>,
}

impl Terrain {
    pub fn generate(seed: u64, width: usize, height: usize) -> Terrain {
        let n = width * height;
        let mut fertility = Vec::with_capacity(n);
        let mut elevation = Vec::with_capacity(n);
        let mut wetness = Vec::with_capacity(n);

        for y in 0..height {
            for x in 0..width {
                let e = fbm(seed, x as f32, y as f32, 0.045);
                let w = fbm(seed ^ 0xA5A5, x as f32, y as f32, 0.025);
                // land above sea level is fertile where it is wet; ocean is barren.
                //
                // primary productivity is twice what it was while foraging was a
                // free actuator: a world tuned so that forced food-followers just
                // fit its carrying capacity is barren once foraging has to be
                // evolved, because the founder generation cannot find the food it
                // is standing next to. see experiments/2026-08-29-perception-costs-productivity.
                let f =
                    if e < SEA_LEVEL { 0.05 } else { (0.4 + 3.2 * w * (e - SEA_LEVEL)).min(2.0) };
                fertility.push(f);
                elevation.push(e);
                wetness.push(w);
            }
        }

        Terrain { fertility, elevation, wetness }
    }
}

/// two octaves of value noise, output roughly 0..1
fn fbm(seed: u64, x: f32, y: f32, freq: f32) -> f32 {
    let a = noise(seed, x * freq, y * freq);
    let b = noise(seed ^ 0x5EED, x * freq * 2.7, y * freq * 2.7);
    (a * 0.67 + b * 0.33).clamp(0.0, 1.0)
}

fn noise(seed: u64, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smoothstep(x - x0), smoothstep(y - y0));
    let (xi, yi) = (x0 as i64, y0 as i64);
    let top = lerp(lattice(seed, xi, yi), lattice(seed, xi + 1, yi), fx);
    let bot = lerp(lattice(seed, xi, yi + 1), lattice(seed, xi + 1, yi + 1), fx);
    lerp(top, bot, fy)
}

fn lattice(seed: u64, x: i64, y: i64) -> f32 {
    let h = hash_u64(hash_u64(hash_u64(HASH_INIT, seed), x as u64), y as u64);
    (h >> 40) as f32 / (1u32 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
