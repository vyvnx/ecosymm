//! temperature. fixed for the whole run: climate change is a non-goal here.

use crate::terrain::Terrain;

/// latitude band corrected for wetness and altitude. 0.0 cold .. 1.0 hot.
pub fn temperature_field(terrain: &Terrain, width: usize, height: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        // poles cold, equator hot
        let lat = 1.0 - ((y as f32 / height as f32) * 2.0 - 1.0).abs();
        for x in 0..width {
            let i = y * width + x;
            let t = lat * 0.8 + 0.2 * terrain.wetness[i] - 0.35 * terrain.elevation[i];
            out.push(t.clamp(0.0, 1.0));
        }
    }
    out
}
