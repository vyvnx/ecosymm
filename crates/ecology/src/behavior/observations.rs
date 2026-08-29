//! what an organism can see from where it stands.
//!
//! locally observable information only. no global world state, no future
//! resource levels, no other organism's genome, no fitness score and no hint
//! about who is winning - a policy that cannot cheat is the only kind whose
//! success means anything.
//!
//! every input is normalised to `0..1`. one convention, no exceptions, so a
//! weight means the same thing wherever it sits.

use crate::{phenotype, CellIndex, Organism};
use ecosym_genetics::{Genes, INPUTS};
use ecosym_world::World;

/// which slot each observation occupies. named, because a bare `inputs[6]` at
/// the call site is how an input silently becomes a different input.
pub const ENERGY: usize = 0;
pub const MATURITY: usize = 1;
pub const FOOD_HERE: usize = 2;
pub const FOOD_NEXT: usize = 3;
pub const CLIMATE: usize = 4;
pub const KIN: usize = 5;
pub const RIVALS: usize = 6;
pub const MOMENTUM: usize = 7;

/// crowd size at which a density input reads 0.5. a normalisation constant and
/// nothing else - it is not a carrying capacity and nothing enforces it.
const CROWD: f32 = 4.0;

/// what one step out looks like from here
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbourhood {
    /// unit offset toward the best-scoring reachable neighbouring tile,
    /// `(0.0, 0.0)` when standing still already scores at least as well
    pub gradient: (f32, f32),
    /// the most food standing on any reachable neighbouring tile, 0..1
    pub best_food: f32,
}

/// probe one step out on each axis. the probe order is fixed and ties keep the
/// earlier probe, so this is deterministic.
///
/// tiles that cannot be walked on are skipped entirely. an organism is not told
/// about food it has no way to reach, and the gradient never points into the
/// sea - which is what stops food seeking turning into drowning.
pub fn scan(g: &Genes, world: &World, x: f32, y: f32) -> Neighbourhood {
    let step = phenotype::step_length(g);
    let mut gradient = (0.0, 0.0);
    let mut best_score = tile_score(g, world, x, y);
    let mut best_food = 0.0f32;

    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let (nx, ny) = (x + dx * step, y + dy * step);
        let i = world.idx(nx, ny);
        if !world.is_passable(i) {
            continue;
        }
        best_food = best_food.max(world.resource_at(i));
        let score = tile_score(g, world, nx, ny);
        if score > best_score {
            best_score = score;
            gradient = (dx, dy);
        }
    }
    Neighbourhood { gradient, best_food: best_food.clamp(0.0, 1.0) }
}

/// how attractive a tile looks: food standing there, discounted by climate
/// mismatch
pub fn tile_score(g: &Genes, world: &World, x: f32, y: f32) -> f32 {
    let i = world.idx(x, y);
    world.resource_at(i) * phenotype::climate_fit(g, world.temperature_at(i))
}

/// the eight numbers a policy gets to reason from
pub fn observe(
    o: &Organism,
    species: usize,
    world: &World,
    occupancy: &CellIndex,
    view: &Neighbourhood,
) -> [f32; INPUTS] {
    let g = o.genes();
    let tile = world.idx(o.x, o.y);
    let mut inputs = [0.0f32; INPUTS];

    // 0.5 means "exactly at the breeding threshold", and it never saturates
    inputs[ENERGY] = saturate(o.energy.max(0.0), phenotype::reproduction_threshold(g));
    inputs[MATURITY] = (o.age as f32 / phenotype::lifespan(g).max(1) as f32).clamp(0.0, 1.0);
    inputs[FOOD_HERE] = world.resource_at(tile).clamp(0.0, 1.0);
    inputs[FOOD_NEXT] = view.best_food;
    inputs[CLIMATE] = phenotype::climate_fit(g, world.temperature_at(tile)).clamp(0.0, 1.0);
    inputs[KIN] = saturate(occupancy.same(species, tile) as f32, CROWD);
    inputs[RIVALS] = saturate(occupancy.others(species, tile) as f32, CROWD);
    inputs[MOMENTUM] = (o.last_move / phenotype::step_length(g).max(1e-3)).clamp(0.0, 1.0);
    inputs
}

/// map an unbounded non-negative magnitude onto 0..1, reading 0.5 at `half`.
/// no clamp cliff, so the network keeps some signal above the midpoint.
fn saturate(v: f32, half: f32) -> f32 {
    if v <= 0.0 {
        0.0
    } else {
        v / (v + half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FounderStreams, OrganismIds, Population, Species, SpeciesBlueprint, SpeciesId};
    use ecosym_core::Rng;
    use ecosym_genetics::{Genome, GenomeIds, NeuralGenome};

    fn genes() -> Genes {
        Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 }
    }

    /// a land tile with land on all four sides, so a step in any direction is
    /// actually allowed and a test about movement is testing movement
    fn inland(world: &World) -> (f32, f32) {
        let w = world.width();
        let tile = world
            .land()
            .iter()
            .copied()
            .find(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                    .iter()
                    .all(|(dx, dy)| world.is_passable(world.idx(x + dx, y + dy)))
            })
            .expect("this world is all coastline, so the test proves nothing");
        ((tile % w) as f32 + 0.5, (tile / w) as f32 + 0.5)
    }

    fn organism(x: f32, y: f32) -> Organism {
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        Organism::new(
            o.mint(),
            Genome::founder(g.mint(), genes(), NeuralGenome::default()),
            x,
            y,
            5.0,
        )
    }

    fn species(n: usize, x: f32, y: f32, world: &World) -> Species {
        let blueprint = SpeciesBlueprint { name: "T".into(), genes: genes(), gene_spread: 0.0 };
        let mut s = Species::found(
            SpeciesId::new(0),
            &blueprint,
            0,
            world,
            &mut FounderStreams { morphology: &mut Rng::new(1), brains: &mut Rng::new(2) },
            &mut GenomeIds::default(),
            &mut OrganismIds::default(),
        );
        let mut population = Population::default();
        for _ in 0..n {
            population.push(organism(x, y));
        }
        *s.population_mut() = population;
        s
    }

    #[test]
    fn every_observation_is_normalised_to_zero_one() {
        let mut world = World::generate(1234, 48, 48);
        let flock = vec![species(6, 4.0, 4.0, &world), species(6, 4.0, 4.0, &world)];
        let mut occupancy = CellIndex::default();
        occupancy.rebuild(&flock, &world);

        let mut rng = Rng::new(3);
        for _ in 0..500 {
            let (x, y) = world.random_land(&mut rng);
            let mut o = organism(x, y);
            o.energy = rng.between(-2.0, 400.0);
            o.age = rng.below(500) as u32;
            o.last_move = rng.between(0.0, 12.0);
            let view = scan(o.genes(), &world, o.x, o.y);
            for (i, v) in observe(&o, 0, &world, &occupancy, &view).iter().enumerate() {
                assert!((0.0..=1.0).contains(v), "input {i} was {v}");
            }
            world.harvest(world.idx(o.x, o.y), 0.3);
        }
    }

    #[test]
    fn the_gradient_points_at_the_better_tile_and_nowhere_when_here_is_best() {
        let mut world = World::generate(1234, 32, 32);
        let g = genes();
        let (x, y) = inland(&world);
        // strip everything one step out, so standing still has to win
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            world.harvest(world.idx(x + dx, y + dy), 1e6);
        }
        assert_eq!(scan(&g, &world, x, y).gradient, (0.0, 0.0));
        assert_eq!(scan(&g, &world, x, y).best_food, 0.0);

        // and strip the tile underfoot instead
        let mut world = World::generate(1234, 32, 32);
        world.harvest(world.idx(x, y), 1e6);
        let view = scan(&g, &world, x, y);
        assert_ne!(view.gradient, (0.0, 0.0));
        assert!(view.best_food > 0.0);
    }

    /// an organism is never shown, or steered toward, a tile it cannot enter
    #[test]
    fn the_scan_ignores_tiles_that_cannot_be_walked_on() {
        let world = World::generate(1234, 64, 64);
        let g = genes();
        let coast = (0..64 * 64)
            .find(|i| {
                world.is_passable(*i)
                    && [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)].iter().any(|(dx, dy)| {
                        let (x, y) = ((i % 64) as f32 + dx, (i / 64) as f32 + dy);
                        !world.is_passable(world.idx(x, y))
                    })
            })
            .expect("this world has no coastline, so the test proves nothing");
        let (x, y) = ((coast % 64) as f32, (coast / 64) as f32);

        let view = scan(&g, &world, x, y);
        if view.gradient != (0.0, 0.0) {
            let (nx, ny) = (x + view.gradient.0, y + view.gradient.1);
            assert!(world.is_passable(world.idx(nx, ny)), "the gradient pointed into the sea");
        }
    }
}
