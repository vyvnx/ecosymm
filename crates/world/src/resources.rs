//! the shared, finite, regrowing resource field. every species eats from this
//! one field, which is the whole competition in the MVP.

use crate::HABITABLE_FERTILITY;

/// how much of a tile's capacity grows back per tick on its own
pub const REGROWTH: f32 = 0.06;

/// extra regrowth per tick from seed rain off the four neighbouring habitable
/// tiles, scaled by how full they are.
///
/// this is what makes grazing a *spatial* problem. a tile with untouched
/// neighbours refills faster than the identical tile in the middle of a
/// grazed-out region, so the edges of a grazed patch recover first and the
/// middle recovers last. no biomass moves between tiles - seed rain lets a tile
/// grow, it does not feed it.
pub const DISPERSAL: f32 = 0.04;

pub struct Resources {
    standing: Vec<f32>,
    capacity: Vec<f32>,
    width: usize,
    height: usize,
    /// scratch, reused every tick: the fullness of each tile *before* this
    /// tick's growth, so dispersal reads one consistent field instead of a
    /// half-updated one where the answer depends on iteration order
    fullness: Vec<f32>,
}

impl Resources {
    /// tiles start full
    pub fn new(capacity: Vec<f32>, width: usize, height: usize) -> Resources {
        Resources {
            standing: capacity.clone(),
            fullness: vec![0.0; capacity.len()],
            capacity,
            width,
            height,
        }
    }

    /// contiguous row-major field, for the cpu engine and future device packing
    pub fn standing(&self) -> &[f32] {
        &self.standing
    }

    pub fn get(&self, i: usize) -> f32 {
        self.standing[i]
    }

    pub fn capacity(&self, i: usize) -> f32 {
        self.capacity[i]
    }

    /// take up to `want`, return what was actually there
    pub fn harvest(&mut self, i: usize, want: f32) -> f32 {
        let got = self.standing[i].min(want).max(0.0);
        self.standing[i] -= got;
        got
    }

    /// one tick of growth: a fixed local share of capacity, plus seed rain from
    /// whichever of the four neighbours can support growth at all. barren tiles
    /// seed nothing, so the sea does not reseed the coast.
    pub fn regrow(&mut self) {
        for ((fullness, standing), capacity) in
            self.fullness.iter_mut().zip(&self.standing).zip(&self.capacity)
        {
            *fullness = if *capacity > HABITABLE_FERTILITY { standing / capacity } else { 0.0 };
        }

        for y in 0..self.height {
            let up = if y == 0 { self.height - 1 } else { y - 1 } * self.width;
            let down = if y + 1 == self.height { 0 } else { y + 1 } * self.width;
            let here = y * self.width;
            for x in 0..self.width {
                let left = if x == 0 { self.width - 1 } else { x - 1 };
                let right = if x + 1 == self.width { 0 } else { x + 1 };
                let rain = 0.25
                    * (self.fullness[here + left]
                        + self.fullness[here + right]
                        + self.fullness[up + x]
                        + self.fullness[down + x]);

                let i = here + x;
                let capacity = self.capacity[i];
                self.standing[i] =
                    (self.standing[i] + capacity * (REGROWTH + DISPERSAL * rain)).min(capacity);
            }
        }
    }

    pub fn total(&self) -> f32 {
        self.standing.iter().sum()
    }
}
