//! what an organism can see from where it stands.
//!
//! locally observable information only. no global world state, no future
//! resource levels, no other organism's genome, no fitness score and no hint
//! about who is winning - a policy that cannot cheat is the only kind whose
//! success means anything.
//!
//! two normalisation conventions, and no others. a magnitude is `0..1`; a
//! direction component is `-1..1`. within a convention a weight means the same
//! thing wherever it sits.
//!
//! # Direction is not instruction
//!
//! the signed pairs say where a local condition lies. they do not say what to
//! do about it. a rival vector may evolve into approach, avoidance, orbiting or
//! no response at all, and nothing here has an opinion about which - that is
//! the difference between adapting Reynolds' premise, that group motion can
//! arise from independent local actors, and importing his authored cohesion,
//! separation and alignment responses.

use crate::{phenotype, CellIndex, Organism};
use ecosym_genetics::{Genes, INPUTS};
use ecosym_world::World;

/// which slot each observation occupies. named, because a bare `inputs[6]` at
/// the call site is how an input silently becomes a different input.
pub const ENERGY: usize = 0;
pub const MATURITY: usize = 1;
pub const FOOD_HERE: usize = 2;
pub const CLIMATE: usize = 3;
pub const RESOURCE_X: usize = 4;
pub const RESOURCE_Y: usize = 5;
pub const KIN: usize = 6;
pub const KIN_X: usize = 7;
pub const KIN_Y: usize = 8;
pub const RIVALS: usize = 9;
pub const RIVAL_X: usize = 10;
pub const RIVAL_Y: usize = 11;

/// crowd size at which a density input reads 0.5. a normalisation constant and
/// nothing else - it is not a carrying capacity and nothing enforces it.
const CROWD: f32 = 4.0;

/// below this a direction has no meaning, so it is reported as `(0, 0)` -
/// which means "nothing measurable lies that way", not "indifference"
const EPSILON: f32 = 1e-4;

/// the fixed sensory stencil: eight wrapped directions in a fixed order, with
/// the unit vector each one lies along.
///
/// probed at one **stride**, not one tile. an organism senses where it can
/// reach: a body that covers 1.3 tiles in a step and only ever looks 1 tile out
/// keeps landing on cells it was never shown, which is a tax on speed that
/// nothing in the ecology ever meant to charge. the reach and the sense are the
/// same distance, so they scale together.
///
/// the centre tile is deliberately absent. it has no direction, and it is
/// already reported on its own as food underfoot and as part of local density.
/// the order is fixed because every sum below is a floating-point sum, and a
/// reordered sum is a different number.
const STENCIL: [(i32, i32, f32, f32); 8] = [
    (1, 0, 1.0, 0.0),
    (1, 1, SQRT_HALF, SQRT_HALF),
    (0, 1, 0.0, 1.0),
    (-1, 1, -SQRT_HALF, SQRT_HALF),
    (-1, 0, -1.0, 0.0),
    (-1, -1, -SQRT_HALF, -SQRT_HALF),
    (0, -1, 0.0, -1.0),
    (1, -1, SQRT_HALF, -SQRT_HALF),
];

const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// what the neighbourhood looks like from here: three signed directions, two
/// densities, and the attractiveness of the tile underfoot.
///
/// built once per organism-tick and handed to both the policy and the
/// measurement of what the organism then did with it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Neighbourhood {
    /// unit-bounded direction of edible resource in the stencil
    pub resource: (f32, f32),
    pub kin: (f32, f32),
    pub rivals: (f32, f32),
    /// organisms of the same species in the whole 3x3, including this tile
    pub kin_count: u32,
    pub rival_count: u32,
}

/// look one stride out in every direction.
///
/// tiles that cannot be walked on are skipped entirely: an organism is not told
/// about food it has no way to reach, and no direction ever points into the
/// sea. that is what stops resource tracking turning into drowning.
///
/// kin and rival directions come from **integer cell totals** accumulated
/// first and combined afterwards, not from averaging organism positions. it is
/// cheaper, it is exact, and it is one fewer place for a floating-point
/// reduction to become order-dependent.
pub fn scan(
    g: &Genes,
    world: &World,
    index: &CellIndex,
    species: usize,
    x: f32,
    y: f32,
) -> Neighbourhood {
    let mut food = [0.0f32; 8];
    let mut kin = [0u32; 8];
    let mut rivals = [0u32; 8];

    let step = phenotype::step_length(g);
    for (k, (dx, dy, _, _)) in STENCIL.iter().enumerate() {
        let cell = world.idx(x + *dx as f32 * step, y + *dy as f32 * step);
        if !world.is_passable(cell) {
            continue;
        }
        food[k] = world.resource_at(cell) * phenotype::climate_fit(g, world.temperature_at(cell));
        kin[k] = index.same(species, cell);
        rivals[k] = index.others(species, cell);
    }

    let here = world.idx(x, y);
    Neighbourhood {
        resource: direction(&food),
        kin: direction(&kin.map(|n| n as f32)),
        rivals: direction(&rivals.map(|n| n as f32)),
        kin_count: kin.iter().sum::<u32>() + index.same(species, here),
        rival_count: rivals.iter().sum::<u32>() + index.others(species, here),
    }
}

/// which way the stencil is *better*, as a convex combination of unit vectors.
///
/// the weights are contrasts, not levels: the smallest cell in the stencil is
/// subtracted from all eight before they are combined. levels do not work. a
/// resource field that is nearly full everywhere gives eight nearly equal
/// weights, whose unit vectors cancel, so an organism standing in plenty would
/// be told there is no direction at all - exactly when there is still a better
/// side to walk toward. against the local floor, only the difference votes,
/// and the direction stays scale-free: doubling the food everywhere does not
/// move it.
///
/// the result is inside the unit disc by construction, so it needs no clamp,
/// and `(0, 0)` genuinely means "no side of this stencil is better than
/// another" rather than "nothing here".
fn direction(weights: &[f32; 8]) -> (f32, f32) {
    let floor = weights.iter().copied().fold(f32::INFINITY, f32::min);
    let mut vx = 0.0;
    let mut vy = 0.0;
    let mut total = 0.0;
    for (w, (_, _, ux, uy)) in weights.iter().zip(&STENCIL) {
        let contrast = w - floor;
        vx += contrast * ux;
        vy += contrast * uy;
        total += contrast;
    }
    if total <= EPSILON {
        return (0.0, 0.0);
    }
    (vx / total, vy / total)
}

/// how well the movement an organism actually made lines up with the resource
/// direction it was shown, as a normalised dot product in `-1..1`.
///
/// this replaces the old `food_seeking` pressure, which reported an internal
/// number the policy emitted. what a viewer wants to know is not what an
/// organism wanted, it is what it did - and after the food gradient stopped
/// being wired to an actuator, wanting was no longer evidence of anything.
///
/// `0` when either vector is at or below the epsilon: that means no measurable
/// alignment, not active indifference.
pub fn resource_tracking(moved: (f32, f32), resource: (f32, f32)) -> f32 {
    let a = (moved.0 * moved.0 + moved.1 * moved.1).sqrt();
    let b = (resource.0 * resource.0 + resource.1 * resource.1).sqrt();
    if a <= EPSILON || b <= EPSILON {
        return 0.0;
    }
    ((moved.0 * resource.0 + moved.1 * resource.1) / (a * b)).clamp(-1.0, 1.0)
}

/// how attractive the tile underfoot is: food standing there, discounted by
/// climate mismatch
pub fn tile_score(g: &Genes, world: &World, x: f32, y: f32) -> f32 {
    let i = world.idx(x, y);
    world.resource_at(i) * phenotype::climate_fit(g, world.temperature_at(i))
}

/// the twelve numbers a policy gets to reason from
pub fn observe(o: &Organism, world: &World, view: &Neighbourhood) -> [f32; INPUTS] {
    let g = o.genes();
    let tile = world.idx(o.x, o.y);
    let mut inputs = [0.0f32; INPUTS];

    // 0.5 means "exactly at the breeding threshold", and it never saturates
    inputs[ENERGY] = saturate(o.energy.max(0.0), phenotype::reproduction_threshold(g));
    inputs[MATURITY] = (o.age as f32 / phenotype::lifespan(g).max(1) as f32).clamp(0.0, 1.0);
    // against this body's own mouthful, not against a fixed 1.0: tile capacity
    // is a world constant that has already moved once, and a clamp against it
    // silently flattens every tile richer than the clamp into the same number.
    // 0.5 means "exactly one tick's intake is standing here".
    inputs[FOOD_HERE] = saturate(world.resource_at(tile), phenotype::intake(g));
    inputs[CLIMATE] = phenotype::climate_fit(g, world.temperature_at(tile)).clamp(0.0, 1.0);
    inputs[RESOURCE_X] = view.resource.0;
    inputs[RESOURCE_Y] = view.resource.1;
    inputs[KIN] = saturate(view.kin_count as f32, CROWD);
    inputs[KIN_X] = view.kin.0;
    inputs[KIN_Y] = view.kin.1;
    inputs[RIVALS] = saturate(view.rival_count as f32, CROWD);
    inputs[RIVAL_X] = view.rivals.0;
    inputs[RIVAL_Y] = view.rivals.1;
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

    /// a land tile with land on all eight sides, so the whole stencil is real
    fn inland(world: &World) -> (f32, f32) {
        let w = world.width();
        let tile = world
            .land()
            .iter()
            .copied()
            .find(|i| {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                STENCIL.iter().all(|(dx, dy, _, _)| {
                    world.is_passable(world.idx(x + *dx as f32, y + *dy as f32))
                })
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

    fn species(id: u32, at: &[(f32, f32)], world: &World) -> Species {
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
        let mut population = Population::default();
        for (x, y) in at {
            population.push(organism(*x, *y));
        }
        *s.population_mut() = population;
        s
    }

    fn index(flock: &[Species], world: &World) -> CellIndex {
        let mut index = CellIndex::default();
        index.rebuild(flock, world);
        index
    }

    #[test]
    fn every_magnitude_is_zero_one_and_every_direction_is_inside_the_unit_disc() {
        let mut world = World::generate(1234, 48, 48);
        let flock =
            vec![species(0, &[(4.5, 4.5); 6], &world), species(1, &[(4.5, 5.5); 6], &world)];
        let seen = index(&flock, &world);

        let mut rng = Rng::new(3);
        for _ in 0..500 {
            let (x, y) = world.random_land(&mut rng);
            let mut o = organism(x, y);
            o.energy = rng.between(-2.0, 400.0);
            o.age = rng.below(500) as u32;
            let view = scan(o.genes(), &world, &seen, 0, o.x, o.y);
            let inputs = observe(&o, &world, &view);
            for slot in [ENERGY, MATURITY, FOOD_HERE, CLIMATE, KIN, RIVALS] {
                assert!((0.0..=1.0).contains(&inputs[slot]), "input {slot} was {}", inputs[slot]);
            }
            for (dx, dy) in [(RESOURCE_X, RESOURCE_Y), (KIN_X, KIN_Y), (RIVAL_X, RIVAL_Y)] {
                let len = inputs[dx].hypot(inputs[dy]);
                assert!(len <= 1.0 + 1e-5, "direction {dx}/{dy} had length {len}");
            }
            world.harvest(world.idx(o.x, o.y), 0.3);
        }
    }

    /// the direction has to point at the thing, not away from it and not at
    /// some fixed compass bearing
    #[test]
    fn each_direction_points_at_what_it_is_measuring() {
        let world = World::generate(1234, 48, 48);
        let (x, y) = inland(&world);
        // kin to the east, rivals to the west
        let flock = vec![
            species(0, &[(x + 1.0, y), (x + 1.0, y)], &world),
            species(1, &[(x - 1.0, y)], &world),
        ];
        let view = scan(&genes(), &world, &index(&flock, &world), 0, x, y);

        assert!(view.kin.0 > 0.9 && view.kin.1.abs() < 1e-5, "{:?}", view.kin);
        assert!(view.rivals.0 < -0.9 && view.rivals.1.abs() < 1e-5, "{:?}", view.rivals);
        assert_eq!(view.kin_count, 2);
        assert_eq!(view.rival_count, 1);
    }

    #[test]
    fn the_resource_direction_points_at_the_food_and_nowhere_when_there_is_none() {
        let mut world = World::generate(1234, 48, 48);
        let (x, y) = inland(&world);
        let empty = index(&[], &world);

        // strip everything one step out except the eastern neighbour
        for (dx, dy, _, _) in STENCIL {
            if (dx, dy) != (1, 0) {
                world.harvest(world.idx(x + dx as f32, y + dy as f32), 1e6);
            }
        }
        let view = scan(&genes(), &world, &empty, 0, x, y);
        assert!(view.resource.0 > 0.9 && view.resource.1.abs() < 1e-5, "{:?}", view.resource);

        // and with the whole stencil bare there is no direction at all
        for (dx, dy, _, _) in STENCIL {
            world.harvest(world.idx(x + dx as f32, y + dy as f32), 1e6);
        }
        assert_eq!(scan(&genes(), &world, &empty, 0, x, y).resource, (0.0, 0.0));
    }

    /// an organism is never shown, or steered toward, a tile it cannot enter
    #[test]
    fn the_scan_ignores_tiles_that_cannot_be_walked_on() {
        let world = World::generate(1234, 64, 64);
        let empty = index(&[], &world);
        let mut rng = Rng::new(11);
        for _ in 0..400 {
            let (x, y) = world.random_land(&mut rng);
            let view = scan(&genes(), &world, &empty, 0, x, y);
            // the resource direction is a convex combination of the passable
            // cells only, so it can never resolve onto a sea tile alone
            for (dx, dy, ux, uy) in STENCIL {
                if world.is_passable(world.idx(x + dx as f32, y + dy as f32)) {
                    continue;
                }
                let alone =
                    (view.resource.0 - ux).abs() < 1e-5 && (view.resource.1 - uy).abs() < 1e-5;
                assert!(!alone, "the resource direction pointed into the sea at {x},{y}");
            }
        }
    }

    #[test]
    fn resource_tracking_measures_alignment_and_says_zero_when_it_cannot() {
        assert!((resource_tracking((1.0, 0.0), (1.0, 0.0)) - 1.0).abs() < 1e-6);
        assert!((resource_tracking((-2.0, 0.0), (1.0, 0.0)) + 1.0).abs() < 1e-6);
        assert!(resource_tracking((1.0, 1.0), (1.0, 0.0)).abs() - 0.707 < 1e-3);
        // standing still, or a stencil with nothing in it, is not indifference
        assert_eq!(resource_tracking((0.0, 0.0), (1.0, 0.0)), 0.0);
        assert_eq!(resource_tracking((1.0, 0.0), (0.0, 0.0)), 0.0);
        // and it never leaves -1..1 however long the step was
        assert!((-1.0..=1.0).contains(&resource_tracking((900.0, -3.0), (0.1, 0.9))));
    }

    /// the whole point of the contract: nothing outside the stencil reaches the
    /// policy, so distance is real
    #[test]
    fn nothing_two_cells_away_is_visible() {
        let world = World::generate(1234, 64, 64);
        let (x, y) = inland(&world);
        let far = vec![species(0, &[(x + 3.0, y); 40], &world)];
        let view = scan(&genes(), &world, &index(&far, &world), 0, x, y);
        assert_eq!(view.kin_count, 0);
        assert_eq!(view.kin, (0.0, 0.0));
    }
}
