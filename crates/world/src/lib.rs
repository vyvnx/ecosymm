//! terrain, resources and climate. generated from a seed, then owned and
//! mutated for the length of the run.

mod climate;
mod resources;
mod terrain;

pub use resources::{DISPERSAL, REGROWTH};
pub use terrain::SEA_LEVEL;

use ecosym_core::Rng;
use resources::Resources;
use terrain::Terrain;

/// a tile at or below this fertility cannot support anything worth counting -
/// and cannot be walked on either. habitable and passable are the same
/// property here: the sea is barren *and* it is not a floor.
pub const HABITABLE_FERTILITY: f32 = 0.1;

pub struct World {
    width: usize,
    height: usize,
    terrain: Terrain,
    temperature: Vec<f32>,
    resources: Resources,
    /// every tile that can be stood on, in ascending order. built once, so
    /// placing a founder is one draw rather than a rejection loop.
    land: Vec<usize>,
}

/// what the run should print about the world it got
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSummary {
    pub width: usize,
    pub height: usize,
    pub habitable_tiles: usize,
    pub initial_biomass: f32,
    pub mean_temperature: f32,
}

impl World {
    pub fn generate(seed: u64, width: usize, height: usize) -> World {
        let mut terrain = Terrain::generate(seed, width, height);
        ensure_somewhere_to_stand(&mut terrain.fertility);
        let temperature = climate::temperature_field(&terrain, width, height);
        let resources = Resources::new(terrain.fertility.clone(), width, height);
        let land = (0..terrain.fertility.len())
            .filter(|i| terrain.fertility[*i] > HABITABLE_FERTILITY)
            .collect();
        World { width, height, terrain, temperature, resources, land }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// contiguous row-major fields. a future backend packs device buffers from
    /// these; nothing here knows what a buffer is.
    pub fn fertility(&self) -> &[f32] {
        &self.terrain.fertility
    }

    pub fn temperature(&self) -> &[f32] {
        &self.temperature
    }

    pub fn resources(&self) -> &[f32] {
        self.resources.standing()
    }

    /// wrapping tile lookup, so the map is a torus and nobody hits an edge.
    ///
    /// `floor`, not the `as isize` cast that was here first. the cast rounds
    /// toward zero, so every coordinate in `-1..0` landed on tile 0 along with
    /// every coordinate in `0..1` - two tiles' worth of world collapsed onto
    /// one, and only on the two sides of the map nearest the origin. an
    /// organism a step west of `x = 0` is on the eastern shore; it is not
    /// standing on itself.
    #[inline]
    pub fn idx(&self, x: f32, y: f32) -> usize {
        let xi = (x.floor() as isize).rem_euclid(self.width as isize) as usize;
        let yi = (y.floor() as isize).rem_euclid(self.height as isize) as usize;
        yi * self.width + xi
    }

    /// can an organism stand on this tile? the sea cannot be walked on, which
    /// is why `idx` wrapping the map into a torus is safe: there are no edges,
    /// only coastlines.
    #[inline]
    pub fn is_passable(&self, i: usize) -> bool {
        self.terrain.fertility[i] > HABITABLE_FERTILITY
    }

    /// somewhere an organism can legally be put, drawn from `rng`. always three
    /// draws, whatever the world looks like, so callers keep a stable stream.
    pub fn random_land(&self, rng: &mut Rng) -> (f32, f32) {
        let tile = self.land.get(rng.below(self.land.len())).copied().unwrap_or(0);
        let (jx, jy) = (rng.f32(), rng.f32());
        let width = self.width.max(1);
        ((tile % width) as f32 + jx, (tile / width) as f32 + jy)
    }

    /// every tile that can be stood on
    pub fn land(&self) -> &[usize] {
        &self.land
    }

    #[inline]
    pub fn resource_at(&self, i: usize) -> f32 {
        self.resources.get(i)
    }

    #[inline]
    pub fn capacity_at(&self, i: usize) -> f32 {
        self.resources.capacity(i)
    }

    #[inline]
    pub fn temperature_at(&self, i: usize) -> f32 {
        self.temperature[i]
    }

    /// take up to `want`, return what was actually there
    pub fn harvest(&mut self, i: usize, want: f32) -> f32 {
        self.resources.harvest(i, want)
    }

    pub fn regrow(&mut self) {
        self.resources.regrow();
    }

    pub fn biomass(&self) -> f32 {
        self.resources.total()
    }

    pub fn summary(&self) -> WorldSummary {
        let tiles = self.terrain.fertility.len().max(1) as f32;
        WorldSummary {
            width: self.width,
            height: self.height,
            // habitable and walkable are the same set
            habitable_tiles: self.land.len(),
            // tiles start full, so capacity is the biomass the run began with
            initial_biomass: self.terrain.fertility.iter().sum(),
            mean_temperature: self.temperature.iter().sum::<f32>() / tiles,
        }
    }
}

/// a world with nowhere to stand is not a world. if a seed generates nothing
/// but sea, its most fertile tile is promoted to land, so founder placement and
/// every invariant downstream of it can stay unconditional.
fn ensure_somewhere_to_stand(fertility: &mut [f32]) {
    if fertility.is_empty() || fertility.iter().any(|f| *f > HABITABLE_FERTILITY) {
        return;
    }
    let best = (0..fertility.len()).fold(0, |b, i| if fertility[i] > fertility[b] { i } else { b });
    fertility[best] = HABITABLE_FERTILITY * 2.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_world() {
        let a = World::generate(1234, 64, 64);
        let b = World::generate(1234, 64, 64);
        assert_eq!(a.fertility(), b.fertility());
        assert_eq!(a.temperature(), b.temperature());

        let other = World::generate(999, 64, 64);
        assert_ne!(a.fertility(), other.fertility());
    }

    #[test]
    fn fields_stay_in_range_and_regrow_is_capped() {
        let mut w = World::generate(1234, 48, 48);
        assert!(w.temperature().iter().all(|t| (0.0..=1.0).contains(t)));

        let i = w.idx(3.0, 4.0);
        w.harvest(i, 1e6);
        assert_eq!(w.resource_at(i), 0.0);
        for _ in 0..500 {
            w.regrow();
        }
        assert_eq!(w.resource_at(i), w.capacity_at(i));
    }

    /// the torus has no edges, so every tile has to be exactly one tile wide
    /// in the lookup - including the two nearest the origin, where a cast
    /// toward zero used to fold `-1..0` onto `0..1`
    #[test]
    fn the_seam_wraps_every_tile_to_the_same_width() {
        let w = World::generate(1234, 64, 64);
        // one step west of the origin is the eastern shore, not the origin
        assert_eq!(w.idx(-0.5, 0.5), w.idx(63.5, 0.5));
        assert_eq!(w.idx(-0.5, -0.5), w.idx(63.5, 63.5));
        assert_ne!(w.idx(-0.5, 0.5), w.idx(0.5, 0.5));

        // and every tile in a row is reached by exactly one unit of x, on
        // both sides of the seam
        let row: Vec<usize> = (-64..128).map(|i| w.idx(i as f32 + 0.5, 0.5)).collect();
        assert!(row.windows(2).all(|p| p[1] == (p[0] + 1) % 64));
    }

    #[test]
    fn summary_describes_the_generated_world() {
        let w = World::generate(1234, 64, 64);
        let s = w.summary();
        assert_eq!((s.width, s.height), (64, 64));
        assert!(s.habitable_tiles > 0 && s.habitable_tiles <= 64 * 64);
        assert!((s.initial_biomass - w.biomass()).abs() < 1e-3);
        assert!((0.0..=1.0).contains(&s.mean_temperature));
    }

    #[test]
    fn the_sea_cannot_be_walked_on_and_the_land_list_agrees() {
        let w = World::generate(1234, 64, 64);
        assert!(!w.land().is_empty());
        assert_eq!(w.land().len(), w.summary().habitable_tiles);
        assert!(w.land().iter().all(|i| w.is_passable(*i)));
        assert!(
            (0..64 * 64).filter(|i| w.is_passable(*i)).count() == w.land().len(),
            "the land list and is_passable disagree"
        );
        // this seed has both, or the test proves nothing
        assert!(w.land().len() < 64 * 64, "no sea in this world");
    }

    #[test]
    fn founders_are_only_ever_placed_on_land() {
        let w = World::generate(1234, 64, 64);
        let mut rng = ecosym_core::Rng::new(5);
        for _ in 0..2000 {
            let (x, y) = w.random_land(&mut rng);
            assert!(w.is_passable(w.idx(x, y)), "placed at {x},{y}, which is sea");
        }
    }

    /// even a world the noise made entirely of sea has to have a floor
    #[test]
    fn a_world_with_no_land_gets_one_tile_promoted() {
        let mut all_sea = vec![0.05f32; 16];
        ensure_somewhere_to_stand(&mut all_sea);
        assert_eq!(all_sea.iter().filter(|f| **f > HABITABLE_FERTILITY).count(), 1);
        // and a world that already has land is left alone
        let mut has_land = vec![0.05, 0.9, 0.05];
        ensure_somewhere_to_stand(&mut has_land);
        assert_eq!(has_land, vec![0.05, 0.9, 0.05]);
    }

    /// dispersal is the whole point: identical tiles, different neighbours
    #[test]
    fn a_tile_with_living_neighbours_recovers_faster_than_one_without() {
        let mut w = World::generate(1234, 64, 64);
        // find two equally fertile land tiles, strip one region bare and only
        // the single tile in the other
        let lush = w.land()[0];
        let (x, y) = ((lush % 64) as f32, (lush / 64) as f32);

        let alone = w.idx(x, y);
        let capacity = w.capacity_at(alone);
        w.harvest(alone, 1e6);
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            w.harvest(w.idx(x + dx, y + dy), 1e6);
        }
        w.regrow();
        let stripped_region = w.resource_at(alone) / capacity;

        let mut w = World::generate(1234, 64, 64);
        w.harvest(alone, 1e6);
        w.regrow();
        let lone_gap = w.resource_at(alone) / capacity;

        assert!(
            lone_gap > stripped_region,
            "seed rain did nothing: {lone_gap} against {stripped_region}"
        );
        assert!(
            (stripped_region - REGROWTH).abs() < 1e-5,
            "a bare region should grow at the base rate"
        );
    }

    #[test]
    fn coordinates_wrap() {
        let w = World::generate(1234, 16, 16);
        assert_eq!(w.idx(-1.0, -1.0), w.idx(15.0, 15.0));
        assert_eq!(w.idx(16.0, 16.0), w.idx(0.0, 0.0));
    }

    /// a fractional coordinate wraps the same way on both sides of zero, so
    /// the tile an organism is standing on and the tile a viewer draws it on
    /// are the same tile everywhere on the map
    #[test]
    fn a_negative_fraction_wraps_like_every_other_fraction() {
        let w = World::generate(1234, 16, 16);
        assert_eq!(w.idx(-0.5, -0.5), w.idx(15.5, 15.5));
        assert_eq!(w.idx(-0.5, 0.0), w.idx(15.5, 0.0));
        assert_eq!(w.idx(-2.0, 0.0), w.idx(14.0, 0.0));
        assert_eq!(w.idx(-1.5, 0.0), w.idx(14.5, 0.0));
    }
}
