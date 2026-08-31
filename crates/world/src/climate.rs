//! temperature. fixed for the whole run: climate change is a non-goal here.

use crate::terrain::{Terrain, SEA_LEVEL};

/// latitude band corrected for wetness and altitude. 0.0 cold .. 1.0 hot.
pub fn temperature_field(terrain: &Terrain, width: usize, height: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        // poles cold, equator hot
        let lat = 1.0 - ((y as f32 / height as f32) * 2.0 - 1.0).abs();
        for x in 0..width {
            let i = y * width + x;
            // the lapse rate is measured from sea level, not from zero. charging
            // land for its full elevation made every habitable tile colder than
            // the water around it, so the land band sat near 0.28 while the
            // scenario profiles were written as if it straddled 0.5 - a
            // permanent climate tax on any warm-adapted species.
            let altitude = (terrain.elevation[i] - SEA_LEVEL).max(0.0);
            let t = lat * 0.8 + 0.2 * terrain.wetness[i] - 0.35 * altitude;
            out.push(t.clamp(0.0, 1.0));
        }
    }
    out
}
