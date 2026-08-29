//! where everyone is standing, as one contiguous index rebuilt every tick.
//!
//! the world is a bounded regular torus of a few thousand tiles, so this is a
//! **dense uniform grid**, not a spatial hash. a hash table would buy tolerance
//! for a sparse world we do not have and charge allocation, pointer chasing and
//! unspecified iteration order for it - and unspecified iteration order is a
//! determinism bug waiting for a population large enough to expose it.
//!
//! the layout is CSR: one exclusive prefix sum keyed by `(species, cell)`, and
//! one flat array of population indices sorted into it. a cell's members are
//! then a contiguous slice, which is what makes a local query a bounded read
//! rather than a scan.
//!
//! ```text
//! count organisms per species/cell
//!   -> exclusive prefix sum into offsets[S * C + 1]
//!   -> stable scatter of population indices into members[N]
//!
//! members[offsets[species * C + cell] .. offsets[species * C + cell + 1]]
//! ```
//!
//! # Determinism
//!
//! - input order is species vector order, then stable population order;
//! - members preserve that canonical order inside every cell;
//! - nothing here iterates a map or reduces floating point.
//!
//! it is a snapshot on purpose: every organism in a tick sees the same crowd,
//! so density is the one observation the visit order cannot skew. the resource
//! field stays first-come-first-served, because that *is* the competition.

use crate::Species;
use ecosym_world::World;

#[derive(Clone, Debug, Default)]
pub struct CellIndex {
    species: usize,
    cells: usize,
    /// exclusive prefix sums, `species * cells + 1` long
    offsets: Vec<u32>,
    /// population indices, grouped by `(species, cell)` and canonically
    /// ordered inside each group
    members: Vec<u32>,
    /// write cursors, one per bucket. scratch, reused between builds.
    cursor: Vec<u32>,
}

impl CellIndex {
    /// counting sort in three passes, over storage allocated once and reused.
    /// `O(N + SC)` time, `O(N + SC)` space, and no per-tick allocation once the
    /// population and the world have stopped changing shape.
    pub fn rebuild(&mut self, species: &[Species], world: &World) {
        let cells = world.width() * world.height();
        let buckets = cells * species.len();
        if self.cells != cells || self.species != species.len() {
            self.species = species.len();
            self.cells = cells;
            self.offsets = vec![0; buckets + 1];
            self.cursor = vec![0; buckets];
        } else {
            // ponytail: clearing the whole prefix array is one memset per tick
            // against a few thousand scattered writes. swap to clearing only
            // the buckets that were touched if the world ever gets much bigger
            // than the population.
            self.offsets.fill(0);
        }

        // 1. count. the count for bucket b lands in offsets[b + 1], which is
        // where the prefix sum wants it.
        for (s, species) in species.iter().enumerate() {
            for o in species.population().organisms() {
                self.offsets[s * cells + world.idx(o.x, o.y) + 1] += 1;
            }
        }

        // 2. exclusive prefix sum
        for b in 0..buckets {
            self.offsets[b + 1] += self.offsets[b];
        }
        self.cursor.copy_from_slice(&self.offsets[..buckets]);

        // 3. stable scatter. population order is preserved inside a cell
        // because the cursor only ever moves forward.
        self.members.clear();
        self.members.resize(self.offsets[buckets] as usize, 0);
        for (s, species) in species.iter().enumerate() {
            for (i, o) in species.population().organisms().iter().enumerate() {
                let bucket = s * cells + world.idx(o.x, o.y);
                self.members[self.cursor[bucket] as usize] = i as u32;
                self.cursor[bucket] += 1;
            }
        }
    }

    /// organisms of `species` standing on `cell`
    pub fn same(&self, species: usize, cell: usize) -> u32 {
        self.range(species, cell).map(|r| (r.end - r.start) as u32).unwrap_or(0)
    }

    /// organisms of every *other* species standing on `cell`
    pub fn others(&self, species: usize, cell: usize) -> u32 {
        (0..self.species).filter(|s| *s != species).map(|s| self.same(s, cell)).sum()
    }

    /// this cell's members, as population indices into that species, in
    /// canonical population order
    pub fn members(&self, species: usize, cell: usize) -> &[u32] {
        match self.range(species, cell) {
            Some(r) => &self.members[r],
            None => &[],
        }
    }

    fn range(&self, species: usize, cell: usize) -> Option<std::ops::Range<usize>> {
        if species >= self.species || cell >= self.cells {
            return None;
        }
        let bucket = species * self.cells + cell;
        Some(self.offsets[bucket] as usize..self.offsets[bucket + 1] as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FounderStreams, Organism, OrganismIds, Population, SpeciesBlueprint, SpeciesId};
    use ecosym_core::Rng;
    use ecosym_genetics::{Genes, Genome, GenomeIds, NeuralGenome};

    fn genes() -> Genes {
        Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 }
    }

    /// a species holding organisms at the given positions, in that order
    fn species(id: u32, world: &World, at: &[(f32, f32)]) -> Species {
        let blueprint =
            SpeciesBlueprint { name: format!("S{id}"), genes: genes(), gene_spread: 0.0 };
        let mut s = Species::found(
            SpeciesId::new(id),
            &blueprint,
            0,
            world,
            &mut FounderStreams { morphology: &mut Rng::new(1), brains: &mut Rng::new(2) },
            &mut GenomeIds::default(),
            &mut OrganismIds::default(),
        );
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let mut population = Population::default();
        for (x, y) in at {
            population.push(Organism::new(
                o.mint(),
                Genome::founder(g.mint(), genes(), NeuralGenome::default()),
                *x,
                *y,
                5.0,
            ));
        }
        *s.population_mut() = population;
        s
    }

    #[test]
    fn it_counts_kin_and_rivals_on_the_cell_it_is_asked_about() {
        let world = World::generate(1234, 32, 32);
        let flock = vec![
            species(0, &world, &[(4.0, 4.0), (4.2, 4.1), (4.4, 4.3)]),
            species(1, &world, &[(4.0, 4.0), (4.1, 4.4)]),
        ];
        let mut index = CellIndex::default();
        index.rebuild(&flock, &world);

        let here = world.idx(4.0, 4.0);
        assert_eq!(index.same(0, here), 3);
        assert_eq!(index.others(0, here), 2);
        assert_eq!(index.others(1, here), 3);
        assert_eq!(index.same(0, world.idx(9.0, 9.0)), 0);
        assert_eq!(index.members(0, world.idx(9.0, 9.0)), &[] as &[u32]);
    }

    /// members are population indices in population order, which is what every
    /// later rule that picks one of them depends on
    #[test]
    fn members_come_back_in_canonical_population_order() {
        let world = World::generate(1234, 32, 32);
        // indices 0, 2 and 3 share a cell; index 1 stands somewhere else
        let flock = vec![species(0, &world, &[(4.0, 4.0), (9.0, 9.0), (4.3, 4.2), (4.9, 4.9)])];
        let mut index = CellIndex::default();
        index.rebuild(&flock, &world);

        assert_eq!(index.members(0, world.idx(4.0, 4.0)), &[0, 2, 3]);
        assert_eq!(index.members(0, world.idx(9.0, 9.0)), &[1]);
    }

    #[test]
    fn rebuilding_clears_the_previous_tick() {
        let world = World::generate(1234, 32, 32);
        let mut index = CellIndex::default();
        index.rebuild(&[species(0, &world, &[(4.0, 4.0); 5])], &world);
        index.rebuild(&[species(0, &world, &[(9.0, 9.0); 5])], &world);
        assert_eq!(index.same(0, world.idx(4.0, 4.0)), 0);
        assert_eq!(index.members(0, world.idx(4.0, 4.0)), &[] as &[u32]);
        assert_eq!(index.same(0, world.idx(9.0, 9.0)), 5);
    }

    /// the storage is sized by the world and the population, not by how many
    /// times it has been rebuilt
    #[test]
    fn storage_is_allocated_once_and_reused() {
        let world = World::generate(1234, 32, 32);
        let flock =
            vec![species(0, &world, &[(4.0, 4.0); 40]), species(1, &world, &[(7.0, 2.0); 40])];
        let mut index = CellIndex::default();
        index.rebuild(&flock, &world);
        let (offsets, members) = (index.offsets.capacity(), index.members.capacity());
        for _ in 0..50 {
            index.rebuild(&flock, &world);
        }
        assert_eq!(index.offsets.capacity(), offsets);
        assert_eq!(index.members.capacity(), members);
        assert_eq!(index.offsets.len(), 32 * 32 * 2 + 1);
        assert_eq!(index.members.len(), 80);
    }

    /// the pathological case for a uniform grid: everybody on one tile. it has
    /// to stay correct and stay ordered, whatever it costs.
    #[test]
    fn a_fully_clustered_population_still_indexes_correctly() {
        let world = World::generate(1234, 32, 32);
        let flock = vec![species(0, &world, &[(4.5, 4.5); 2_000])];
        let mut index = CellIndex::default();
        index.rebuild(&flock, &world);

        let cell = world.idx(4.5, 4.5);
        assert_eq!(index.same(0, cell), 2_000);
        assert_eq!(index.members(0, cell), (0..2_000u32).collect::<Vec<_>>());
        let elsewhere: u32 = (0..32 * 32).filter(|c| *c != cell).map(|c| index.same(0, c)).sum();
        assert_eq!(elsewhere, 0);
    }

    /// the world wraps, so positions past the edge index onto the far side
    /// rather than onto nothing. the index inherits its seam from
    /// `World::idx` and does not add one of its own.
    #[test]
    fn positions_past_the_seam_land_on_the_far_side_of_the_torus() {
        let world = World::generate(1234, 32, 32);
        let flock = vec![
            species(0, &world, &[(0.5, 0.5), (32.5, 32.5), (64.5, 0.5)]),
            species(1, &world, &[(31.5, 31.5), (-0.5, -0.5)]),
        ];
        let mut index = CellIndex::default();
        index.rebuild(&flock, &world);

        assert_eq!(index.same(0, world.idx(0.5, 0.5)), 3);
        assert_eq!(index.members(0, world.idx(0.5, 0.5)), &[0, 1, 2]);
        // the far corner and the tile before the seam are still distinct cells
        assert_eq!(index.same(1, world.idx(31.5, 31.5)), 1);
        let occupied: usize = (0..32 * 32).filter(|c| index.same(1, *c) > 0).count();
        assert_eq!(occupied, 2, "the seam collapsed two distinct tiles into one");
    }

    /// what the index costs at a population the default world cannot sustain,
    /// and in the clustered worst case a uniform grid is supposed to be bad at.
    ///
    /// ponytail: `#[ignore]`d and printed rather than asserted. a wall-clock
    /// threshold in ci is a flaky test, and this exists to be read next to
    /// `benchmarks/`, not to fail a build. run it with
    /// `cargo test --release -p ecosym-ecology -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn index_construction_cost() {
        let world = World::generate(1234, 128, 128);
        let mut rng = Rng::new(9);
        for (label, spread) in [("spread over the map", true), ("all on one tile", false)] {
            for n in [5_000usize, 10_000] {
                let at: Vec<(f32, f32)> = (0..n / 2)
                    .map(|_| if spread { world.random_land(&mut rng) } else { (64.5, 64.5) })
                    .collect();
                let flock = vec![species(0, &world, &at), species(1, &world, &at)];
                let mut index = CellIndex::default();
                index.rebuild(&flock, &world);

                let started = std::time::Instant::now();
                let rebuilds = 2_000;
                for _ in 0..rebuilds {
                    index.rebuild(&flock, &world);
                }
                let per = started.elapsed().as_secs_f64() / rebuilds as f64;
                println!("{n:>6} organisms, {label:<20} {:>8.1} us/rebuild", per * 1e6);
            }
        }
    }

    /// an empty world is a real state - everything died - and it has to index
    /// without panicking or reporting anybody
    #[test]
    fn an_empty_population_indexes_to_nothing() {
        let world = World::generate(1234, 32, 32);
        let mut index = CellIndex::default();
        index.rebuild(&[species(0, &world, &[])], &world);
        assert_eq!(index.same(0, world.idx(4.0, 4.0)), 0);
        assert_eq!(index.others(0, world.idx(4.0, 4.0)), 0);
        index.rebuild(&[], &world);
        assert_eq!(index.same(0, 0), 0);
    }
}
