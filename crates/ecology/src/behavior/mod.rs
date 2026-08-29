//! what an organism does in one tick: observe, decide, move, forage, age,
//! breed, die.
//!
//! ```text
//! observations  -> what can it see from here?
//! neural_policy -> what does its inherited brain want to do about it?
//! actions       -> what will the world actually let it do?
//! ```
//!
//! these are the rules an engine *schedules* but must not own. an engine
//! decides who runs when, on what hardware, in what order and where the bytes
//! live. what a tick means lives here, once, so a second backend schedules
//! differently without re-deriving the model.

pub mod actions;
pub mod neural_policy;
pub mod observations;

pub use actions::{Act, BehaviorStats, BehaviorTally, Intent, Stride, CHANNELS};

use crate::{interactions, phenotype, CellIndex, Organism, OrganismId, Population};
use ecosym_core::Rng;
use ecosym_genetics::{
    mutate, mutate_brain, recombine, recombine_brain, Genes, Genome, GenomeId, NeuralGenome,
};
use ecosym_world::World;

/// half-width of the wander folded into a chosen heading, so organisms do not
/// collapse onto identical trajectories. it is part of the intended
/// displacement, so it is capped, suppressed by rest and paid for like any
/// other movement.
pub const MOVE_NOISE: f32 = 0.3;

pub fn can_reproduce(o: &Organism) -> bool {
    o.energy > phenotype::reproduction_threshold(o.genes())
}

pub fn is_dead(o: &Organism) -> bool {
    o.energy <= 0.0 || o.age >= phenotype::lifespan(o.genes())
}

/// fraction of its own energy a parent hands to each offspring
pub const BIRTH_ENERGY_SHARE: f32 = 0.5;

/// one organism's tick.
///
/// it looks around, its inherited brain returns tendencies, and the ecology
/// layer decides what those tendencies actually buy: how far the body moves,
/// what that costs, and what is left on the tile once everyone ahead of it in
/// the visit order has eaten. the policy never touches the world directly.
///
/// the organism is not mutated - a new value comes back carrying the same id
/// and the same immutable genome, brain included. the *world* is, because
/// foraging takes from a field everyone shares.
pub fn live_one_tick(
    o: &Organism,
    species: usize,
    world: &mut World,
    occupancy: &CellIndex,
    wander: &mut Rng,
) -> (Organism, Act) {
    let genes = *o.genes();
    let view = observations::scan(&genes, world, occupancy, species, o.x, o.y);
    let inputs = observations::observe(o, world, &view);

    // the one thing a tick may write back into the organism. `next` is taken
    // first so the memory advances on the body that goes on living, and the
    // caller's `o` is left exactly as it was.
    let mut next = *o;
    let intent = neural_policy::decide(o.brain(), &inputs, &mut next.hidden);

    let noise = (wander.between(-MOVE_NOISE, MOVE_NOISE), wander.between(-MOVE_NOISE, MOVE_NOISE));
    let stride = actions::stride(&genes, &intent, noise);
    let (x, y) = walkable(world, (o.x, o.y), (o.x + stride.dx, o.y + stride.dy));

    next.x = x;
    next.y = y;
    let moved = (next.x - o.x, next.y - o.y);
    next.last_move = (moved.0 * moved.0 + moved.1 * moved.1).sqrt();
    next.energy = o.energy + interactions::forage(&genes, world, next.x, next.y)
        - phenotype::upkeep(&genes, stride.effort);
    next.age = o.age + 1;

    let temperature = world.temperature_at(world.idx(next.x, next.y));
    let act = Act {
        moved: next.last_move,
        // measured from the displacement against the direction it was shown,
        // not read off an output: what it did, not what it wanted
        tracking: observations::resource_tracking(moved, view.resource),
        reproduction: intent.breed,
        resting: intent.rest,
        competitors: inputs[observations::RIVALS],
        temperature,
        climate_fit: phenotype::climate_fit(&genes, temperature),
    };
    (next, act)
}

/// terrain has the last word on where a step ends.
///
/// the sea cannot be entered, but a shoreline should be something an organism
/// walks *along*, not something it sticks to: refusing the whole step would let
/// the wander noise alone pin a coastal organism in place, which is an artefact
/// of the noise rather than anything its policy chose. so a blocked step falls
/// back to whichever single axis still lands on solid ground, and only then to
/// standing still.
///
/// the effort is spent either way. walking into a cliff costs exactly what
/// walking anywhere else would.
fn walkable(world: &World, from: (f32, f32), wanted: (f32, f32)) -> (f32, f32) {
    for candidate in [wanted, (wanted.0, from.1), (from.0, wanted.1)] {
        if world.is_passable(world.idx(candidate.0, candidate.1)) {
            return candidate;
        }
    }
    from
}

/// an offspring that has been conceived but not yet admitted: both genetic
/// parents, the genes and the brain birth-time recombination and mutation
/// produced, and the energy the parent will hand over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conception {
    pub parents: [GenomeId; 2],
    pub genes: Genes,
    pub brain: NeuralGenome,
    pub energy: f32,
}

/// how many tiles out an organism can find a mate, in every direction.
///
/// mating is contact, so this is the smallest reach that keeps an isolated
/// population viable - which is the rule ADR 0008 asks for, and it is measured
/// rather than picked. at reach 1 a founding population at one organism per
/// nine tiles cannot find each other reliably and dies before selection starts;
/// see `experiments/2026-08-29-mating-is-local`.
pub const MATE_REACH: i32 = 4;

/// the wrapped square an organism can breed inside, centre included: standing
/// on the same tile is the closest two organisms in this model can get
const MATE_CELLS: usize = (2 * MATE_REACH as usize + 1).pow(2);

/// where a breeder is allowed to look, gathered so the two indices cannot be
/// swapped at a call site: `species` says which population the crowd snapshot
/// is keyed by, `breeder` is the parent's own slot inside it.
pub struct MateSearch<'a> {
    pub species: usize,
    pub breeder: usize,
    pub population: &'a Population,
    pub world: &'a World,
    /// the post-movement crowd, shared by every breeder in the tick
    pub index: &'a CellIndex,
}

/// a mate from the neighbourhood, by count, rank and index.
///
/// count the eligible in each stencil cell, draw one rank across the total,
/// then walk the fixed cell order to whoever holds it. one rng draw, no
/// candidate vector, and no path that degrades into scanning the population
/// when a thousand organisms pile onto one tile.
///
/// eligibility is: this species, alive in the post-movement snapshot, and not
/// the breeder. the breeder is subtracted before the draw and skipped when the
/// rank is located, so a lone organism cannot fertilise itself - **isolation is
/// now a way to fail**, which is the whole point of making mating spatial.
///
/// ponytail: the eligible are filtered out of the shared occupancy index rather
/// than sorted into an index of their own. a second `(species, cell)` counting
/// sort per tick would cost a full pass over the population to save reading a
/// few short slices, and only breeders ever pay for this one - widening the
/// reach from 1 to 4 cost 0.4% of wall clock. build the second index if a
/// filter ever gets more expensive than a liveness check.
fn find_mate<'a>(
    parent: &Organism,
    search: &MateSearch<'a>,
    rng: &mut Rng,
) -> Option<&'a Organism> {
    let MateSearch { species, breeder, population, world, index } = *search;
    let eligible =
        |m: u32| m as usize != breeder && population.get(m as usize).is_some_and(|o| !is_dead(o));

    // row-major over the wrapped square, which fixes the order every sum and
    // every rank below is taken in
    let mut cells = [0usize; MATE_CELLS];
    let mut counts = [0u32; MATE_CELLS];
    let mut total = 0u32;
    for k in 0..MATE_CELLS {
        let (dx, dy) = (
            (k % (2 * MATE_REACH as usize + 1)) as i32 - MATE_REACH,
            (k / (2 * MATE_REACH as usize + 1)) as i32 - MATE_REACH,
        );
        let cell = world.idx(parent.x + dx as f32, parent.y + dy as f32);
        cells[k] = cell;
        // a world narrower than the reach wraps onto itself, and the same cell
        // counted twice would weight its members by geometry
        if cells[..k].contains(&cell) {
            continue;
        }
        counts[k] =
            index.members(species, cell).iter().copied().filter(|m| eligible(*m)).count() as u32;
        total += counts[k];
    }
    if total == 0 {
        return None;
    }

    let mut rank = rng.below(total as usize) as u32;
    for (k, n) in counts.iter().enumerate() {
        if rank < *n {
            let m = index
                .members(species, cells[k])
                .iter()
                .copied()
                .filter(|m| eligible(*m))
                .nth(rank as usize)?;
            return population.get(m as usize);
        }
        rank -= n;
    }
    None
}

/// the whole birth rule in one place: eligibility, a mate from *this*
/// population and no other, recombination followed by birth-time mutation of
/// both the physical genes and the brain, and the parent's energy split.
///
/// `intent` is the policy's reproductive pressure, read as a per-tick rate: at
/// 1.0 an eligible parent always tries, at 0.0 it never does, and in between it
/// tries that often. a rate rather than a threshold, because a threshold makes
/// every founder whose output lands on the wrong side of it sterile for life,
/// which is an artefact of the cutoff rather than anything the world decided.
///
/// it is a veto, not a permit. wanting offspring only gets an organism as far
/// as the rule, which still demands the energy threshold and an available mate
/// of its own species; a brain that outputs 1.0 forever breeds no more often
/// than the world allows.
///
/// the mate comes from the wrapped 3x3 around where the parent is **now**, off
/// one coherent post-movement snapshot: every breeder in a tick queries the
/// same crowd, so nobody sees half the population where it used to be. that is
/// why conception is a pass of its own rather than something the visit does.
///
/// `breeder` is the parent's own index in `population`, which is what stops it
/// from fertilising itself.
pub fn conceive(
    parent: &Organism,
    intent: f32,
    search: &MateSearch<'_>,
    rng: &mut Rng,
) -> Option<Conception> {
    if !can_reproduce(parent) || rng.f32() >= intent {
        return None;
    }
    let mate = find_mate(parent, search, rng)?;
    Some(Conception {
        genes: mutate(recombine(parent.genes(), mate.genes(), rng), rng),
        brain: mutate_brain(recombine_brain(parent.brain(), mate.brain(), rng), rng),
        parents: [parent.genome().id(), mate.genome().id()],
        energy: parent.energy * BIRTH_ENERGY_SHARE,
    })
}

impl Conception {
    /// the offspring is born where its parent stands. an engine supplies the
    /// two fresh ids from its allocators and decides nothing else.
    pub fn birth(self, id: OrganismId, genome: GenomeId, parent: &Organism) -> Organism {
        Organism::new(
            id,
            Genome::offspring(genome, self.parents, self.genes, self.brain),
            parent.x,
            parent.y,
            self.energy,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FounderStreams, OrganismIds, SpeciesBlueprint, START_ENERGY};
    use ecosym_genetics::GenomeIds;

    fn brain(seed: u64) -> NeuralGenome {
        NeuralGenome::random(&mut Rng::new(seed))
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

    fn organism(ids: (&mut OrganismIds, &mut GenomeIds), energy: f32) -> Organism {
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let (x, y) = inland(&World::generate(1234, 32, 32));
        Organism::new(ids.0.mint(), Genome::founder(ids.1.mint(), genes, brain(4)), x, y, energy)
    }

    fn occupancy(world: &World) -> CellIndex {
        let mut o = CellIndex::default();
        o.rebuild(&[], world);
        o
    }

    /// one species holding exactly these organisms, in this order, plus the
    /// post-movement index a conception pass would query
    fn crowd(world: &World, organisms: Vec<Organism>) -> (Vec<crate::Species>, CellIndex) {
        let blueprint = SpeciesBlueprint {
            name: "S".into(),
            genes: Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 },
            gene_spread: 0.0,
        };
        let mut species = crate::Species::found(
            crate::SpeciesId::new(0),
            &blueprint,
            0,
            world,
            &mut FounderStreams { morphology: &mut Rng::new(1), brains: &mut Rng::new(2) },
            &mut GenomeIds::default(),
            &mut OrganismIds::default(),
        );
        *species.population_mut() = Population::new(organisms);
        let flock = vec![species];
        let mut index = CellIndex::default();
        index.rebuild(&flock, world);
        (flock, index)
    }

    #[test]
    fn a_tick_moves_feeds_and_ages_without_touching_the_genome() {
        let mut world = World::generate(1234, 32, 32);
        let seen = occupancy(&world);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let before = organism((&mut o, &mut g), START_ENERGY);

        let (after, act) = live_one_tick(&before, 0, &mut world, &seen, &mut Rng::new(4));
        assert_eq!(after.id(), before.id());
        assert_eq!(after.genome(), before.genome());
        assert_eq!(after.age, before.age + 1);
        assert!(after.is_finite());
        // it paid upkeep, so it cannot have gained more than the tile held
        assert!(after.energy < before.energy + phenotype::intake(before.genes()));
        assert!(act.moved.is_finite() && act.moved >= 0.0);
    }

    /// the whole point of the addon: the same body in the same place behaves
    /// differently because it inherited a different policy
    #[test]
    fn two_identical_bodies_with_different_brains_behave_differently() {
        let world = World::generate(1234, 32, 32);
        let seen = occupancy(&world);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };

        let (x, y) = inland(&world);
        let mut acts = Vec::new();
        for seed in [11u64, 12] {
            let subject = Organism::new(
                o.mint(),
                Genome::founder(g.mint(), genes, brain(seed)),
                x,
                y,
                START_ENERGY,
            );
            // same world, same wander stream: only the brain differs
            let mut world = World::generate(1234, 32, 32);
            acts.push(live_one_tick(&subject, 0, &mut world, &seen, &mut Rng::new(4)).1);
        }
        assert_ne!(acts[0], acts[1]);
        assert_ne!(acts[0].moved, acts[1].moved);
    }

    /// the boundary again, one layer down: the policy can ask to walk into the
    /// sea all it likes, and the terrain still refuses
    #[test]
    fn no_policy_can_walk_into_the_sea() {
        let mut world = World::generate(1234, 64, 64);
        let seen = occupancy(&world);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let mut wander = Rng::new(4);

        for seed in 0..60u64 {
            let (x, y) = world.random_land(&mut wander);
            let subject = Organism::new(
                o.mint(),
                Genome::founder(g.mint(), genes, brain(seed)),
                x,
                y,
                START_ENERGY,
            );
            let (after, _) = live_one_tick(&subject, 0, &mut world, &seen, &mut wander);
            assert!(
                world.is_passable(world.idx(after.x, after.y)),
                "organism ended a tick at {},{}, which is sea",
                after.x,
                after.y
            );
        }
    }

    #[test]
    fn a_blocked_step_slides_along_the_shore_before_it_gives_up() {
        let world = World::generate(1234, 64, 64);
        let w = world.width();
        // a land tile whose eastern neighbour is sea and northern neighbour is not
        let coast = world
            .land()
            .iter()
            .copied()
            .find(|i| {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                !world.is_passable(world.idx(x + 1.0, y))
                    && world.is_passable(world.idx(x, y + 1.0))
            })
            .expect("no such coastline here, so the test proves nothing");
        let (x, y) = ((coast % w) as f32 + 0.5, (coast / w) as f32 + 0.5);

        // north-east into the sea: the eastward half is refused, the northward
        // half is not, so it slides north rather than stopping
        let slid = walkable(&world, (x, y), (x + 1.0, y + 1.0));
        assert_eq!(slid, (x, y + 1.0));
        assert!(world.is_passable(world.idx(slid.0, slid.1)));

        // and a step with no way through at all leaves it where it stood
        let sea = (0..w * w).find(|i| !world.is_passable(*i)).unwrap();
        let (sx, sy) = ((sea % w) as f32 + 0.5, (sea / w) as f32 + 0.5);
        assert_eq!(walkable(&world, (x, y), (sx, sy)), (x, y));
    }

    /// a brain with no hidden layer contribution and fixed output biases, so a
    /// movement test can say exactly what the policy asked for
    fn fixed_intent(output_biases: [f32; ecosym_genetics::OUTPUTS]) -> NeuralGenome {
        let mut brain = NeuralGenome::default();
        brain.biases[ecosym_genetics::HIDDEN..].copy_from_slice(&output_biases);
        brain
    }

    /// run one tick on a world with nothing left to eat, so intake cannot mask
    /// the metabolic bill, and report what the tick charged and where it ended
    fn tick_bill(world: &mut World, o: &Organism) -> (f32, (f32, f32)) {
        for i in 0..world.width() * world.height() {
            world.harvest(i, 1e6);
        }
        let seen = occupancy(world);
        let (after, _) = live_one_tick(o, 0, world, &seen, &mut Rng::new(4));
        (o.energy - after.energy, (after.x, after.y))
    }

    fn subject(brain: NeuralGenome, x: f32, y: f32) -> Organism {
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        Organism::new(o.mint(), Genome::founder(g.mint(), genes, brain), x, y, 50.0)
    }

    /// resting is the only thing that makes a tick cheap, and it has to
    /// suppress the wander too - otherwise every organism drifts for free and
    /// standing still is not a strategy anyone can be selected for.
    #[test]
    fn resting_pays_the_basal_bill_and_nothing_for_the_wander() {
        let mut world = World::generate(1234, 32, 32);
        let (x, y) = inland(&world);
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let resting = subject(fixed_intent([0.0, 0.0, 0.0, 4.0]), x, y);

        let (paid, ended) = tick_bill(&mut world, &resting);
        assert!(
            (paid - phenotype::basal_cost(&genes)).abs() < 1e-3,
            "a resting tick charged {paid}, not the basal {}",
            phenotype::basal_cost(&genes)
        );
        assert!(
            (ended.0 - x).abs() < 1e-2 && (ended.1 - y).abs() < 1e-2,
            "it drifted while resting"
        );
    }

    /// the wander is displacement the organism did not ask for but still made,
    /// so it is displacement it pays for
    #[test]
    fn an_indifferent_policy_still_pays_for_the_distance_the_wander_moved_it() {
        let mut world = World::generate(1234, 32, 32);
        let (x, y) = inland(&world);
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let idle = subject(NeuralGenome::default(), x, y);

        let (paid, ended) = tick_bill(&mut world, &idle);
        assert_ne!(ended, (x, y), "the wander moved nobody, so this test proves nothing");
        assert!(paid > phenotype::basal_cost(&genes), "wandering was free: {paid}");
        assert!(paid <= phenotype::upkeep(&genes, 1.0) + 1e-6);
    }

    #[test]
    fn a_policy_that_walks_pays_for_the_stride_it_took() {
        let mut world = World::generate(1234, 32, 32);
        let (x, y) = inland(&world);
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let walker = subject(fixed_intent([4.0, 4.0, 0.0, -4.0]), x, y);

        let (paid, ended) = tick_bill(&mut world, &walker);
        assert_ne!(ended, (x, y));
        assert!((paid - phenotype::upkeep(&genes, 1.0)).abs() < 1e-3, "full effort charged {paid}");
    }

    /// walking into a cliff costs exactly what walking anywhere else would.
    /// the effort is spent on the attempt, not refunded by the terrain.
    #[test]
    fn a_step_the_shore_refuses_costs_what_the_same_step_inland_would() {
        let mut world = World::generate(1234, 64, 64);
        let w = world.width();
        // land with sea north, east and north-east: every fallback `walkable`
        // has for a north-east step is also sea, so the step cannot land
        let cornered = world
            .land()
            .iter()
            .copied()
            .find(|i| {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                [(1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                    .iter()
                    .all(|(dx, dy)| !world.is_passable(world.idx(x + dx, y + dy)))
            })
            .expect("no such corner of coast here, so the test proves nothing");
        let (x, y) = ((cornered % w) as f32 + 0.5, (cornered / w) as f32 + 0.5);

        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        // hard north-east, no seeking, no rest: the wander cannot flip either sign
        let blocked = subject(fixed_intent([4.0, 4.0, 0.0, -4.0]), x, y);

        let (paid, ended) = tick_bill(&mut world, &blocked);
        assert_eq!(ended, (x, y), "the shore let it through, so this test proves nothing");
        assert!(
            (paid - phenotype::upkeep(&genes, 1.0)).abs() < 1e-3,
            "a refused step was charged {paid}, not the {} it intended",
            phenotype::upkeep(&genes, 1.0)
        );
    }

    #[test]
    fn conception_needs_the_threshold_and_records_both_parents() {
        let world = World::generate(1234, 32, 32);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let hungry = organism((&mut o, &mut g), 1.0);
        let fat = organism((&mut o, &mut g), 100.0);
        let (flock, index) = crowd(&world, vec![fat, hungry]);
        let population = flock[0].population();
        let mut rng = Rng::new(4);

        assert!(
            conceive(
                &hungry,
                1.0,
                &MateSearch { species: 0, breeder: 1, population, world: &world, index: &index },
                &mut rng,
            )
            .is_none(),
            "bred while starving"
        );

        let c = conceive(
            &fat,
            1.0,
            &MateSearch { species: 0, breeder: 0, population, world: &world, index: &index },
            &mut rng,
        )
        .expect("a fat parent should breed");
        assert_eq!(c.parents, [fat.genome().id(), hungry.genome().id()]);
        assert_eq!(c.energy, fat.energy * BIRTH_ENERGY_SHARE);
        assert!(c.genes.in_bounds());
        assert!(c.brain.in_bounds() && c.brain.is_finite());

        let child = c.birth(o.mint(), g.mint(), &fat);
        assert_eq!((child.x, child.y), (fat.x, fat.y));
        assert_eq!(child.age, 0);
        assert_eq!(
            child.genome().parent_ids(),
            [Some(fat.genome().id()), Some(hungry.genome().id())]
        );
        assert_ne!(child.genome().id(), fat.genome().id());
    }

    /// the boundary the addon exists to hold: intent is a veto the ecology
    /// rules sit behind, never a permit that gets in front of them
    #[test]
    fn reproductive_intent_cannot_buy_a_birth_the_rules_refuse() {
        let world = World::generate(1234, 32, 32);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let starving = organism((&mut o, &mut g), 1.0);
        let fat = organism((&mut o, &mut g), 100.0);
        let (flock, index) = crowd(&world, vec![starving, fat]);
        let population = flock[0].population();
        let mut rng = Rng::new(4);

        // maximum possible pressure, still not enough energy - every time
        for _ in 0..200 {
            assert!(conceive(
                &starving,
                1.0,
                &MateSearch { species: 0, breeder: 0, population, world: &world, index: &index },
                &mut rng,
            )
            .is_none());
        }
        // energy to spare, but the policy never asks
        for _ in 0..200 {
            assert!(conceive(
                &fat,
                0.0,
                &MateSearch { species: 0, breeder: 1, population, world: &world, index: &index },
                &mut rng,
            )
            .is_none());
        }
        // and nobody within reach means no birth however much it wants one
        let (alone, empty) = crowd(&world, vec![fat]);
        assert!(conceive(
            &fat,
            1.0,
            &MateSearch {
                species: 0,
                breeder: 0,
                population: alone[0].population(),
                world: &world,
                index: &empty,
            },
            &mut rng,
        )
        .is_none());
        // only energy, a mate and the will together
        assert!(conceive(
            &fat,
            1.0,
            &MateSearch { species: 0, breeder: 1, population, world: &world, index: &index },
            &mut rng,
        )
        .is_some());
    }

    /// mating is contact. an organism on the far side of the map is not a mate,
    /// however willing both of them are, and an isolated one leaves no
    /// descendants at all - which is the whole reason for making it spatial.
    #[test]
    fn a_mate_has_to_be_within_reach_and_cannot_be_yourself() {
        // wide enough to hold two places further apart than the reach. the
        // 32x32 the other tests use is one small island at this seed.
        let world = World::generate(1234, 64, 64);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let mut here = organism((&mut o, &mut g), 100.0);
        (here.x, here.y) = inland(&world);
        let mut far = organism((&mut o, &mut g), 100.0);
        (far.x, far.y) = inland_away_from(&world, (here.x, here.y));

        let (flock, index) = crowd(&world, vec![here, far]);
        let population = flock[0].population();
        let mut rng = Rng::new(4);
        for _ in 0..200 {
            assert!(
                conceive(
                    &here,
                    1.0,
                    &MateSearch {
                        species: 0,
                        breeder: 0,
                        population,
                        world: &world,
                        index: &index,
                    },
                    &mut rng,
                )
                .is_none(),
                "bred with an organism it cannot reach"
            );
        }

        // a neighbour one tile away is reachable, and it is the only candidate
        let mut close = organism((&mut o, &mut g), 100.0);
        (close.x, close.y) = (here.x + 1.0, here.y);
        if world.is_passable(world.idx(close.x, close.y)) {
            let (flock, index) = crowd(&world, vec![here, close]);
            let c = conceive(
                &here,
                1.0,
                &MateSearch {
                    species: 0,
                    breeder: 0,
                    population: flock[0].population(),
                    world: &world,
                    index: &index,
                },
                &mut rng,
            )
            .expect("a neighbour one tile away should be reachable");
            assert_eq!(c.parents[1], close.genome().id());
        }
    }

    /// pressure is a rate, so a policy that half wants offspring has half as
    /// many. that is the gradient selection acts on.
    #[test]
    fn reproductive_pressure_sets_the_rate_it_tries_at() {
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let a = organism((&mut o, &mut g), 100.0);
        let b = organism((&mut o, &mut g), 100.0);
        let mut rng = Rng::new(8);

        let world = World::generate(1234, 32, 32);
        let (flock, index) = crowd(&world, vec![a, b]);
        let population = flock[0].population();
        let tries = |intent: f32, rng: &mut Rng| {
            (0..1000)
                .filter(|_| {
                    let search = MateSearch {
                        species: 0,
                        breeder: 0,
                        population,
                        world: &world,
                        index: &index,
                    };
                    conceive(&a, intent, &search, rng).is_some()
                })
                .count()
        };
        assert_eq!(tries(0.0, &mut rng), 0);
        assert_eq!(tries(1.0, &mut rng), 1000);
        assert!((400..600).contains(&tries(0.5, &mut rng)));
    }

    #[test]
    fn an_offspring_carries_a_brain_built_from_both_parents() {
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let a =
            Organism::new(o.mint(), Genome::founder(g.mint(), genes, brain(1)), 8.0, 8.0, 100.0);
        let b =
            Organism::new(o.mint(), Genome::founder(g.mint(), genes, brain(2)), 8.0, 8.0, 100.0);
        let world = World::generate(1234, 32, 32);
        let (flock, index) = crowd(&world, vec![a, b]);
        let c = conceive(
            &a,
            1.0,
            &MateSearch {
                species: 0,
                breeder: 0,
                population: flock[0].population(),
                world: &world,
                index: &index,
            },
            &mut Rng::new(6),
        )
        .unwrap();
        assert_ne!(c.brain, *a.brain());
        assert_ne!(c.brain, *b.brain());
        // and it is nearer to its parents than two unrelated brains are to each other
        assert!(c.brain.distance(a.brain()) < a.brain().distance(b.brain()));
    }

    #[test]
    fn a_lone_survivor_cannot_conceive_from_an_empty_population() {
        let world = World::generate(1234, 32, 32);
        let (mut o, mut g) = (OrganismIds::default(), GenomeIds::default());
        let fat = organism((&mut o, &mut g), 100.0);
        let (flock, index) = crowd(&world, vec![fat]);
        assert!(conceive(
            &fat,
            1.0,
            &MateSearch {
                species: 0,
                breeder: 0,
                population: flock[0].population(),
                world: &world,
                index: &index,
            },
            &mut Rng::new(4),
        )
        .is_none());
    }

    /// somewhere inland further than `MATE_REACH` from `from` on both axes,
    /// counting the short way round the torus, so no wrapped stencil reaches it
    fn inland_away_from(world: &World, from: (f32, f32)) -> (f32, f32) {
        let w = world.width();
        let gap = |a: f32, b: f32, span: usize| {
            let d = (a - b).abs();
            d.min(span as f32 - d)
        };
        let tile = world
            .land()
            .iter()
            .copied()
            .find(|i| {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let far = MATE_REACH as f32 + 1.0;
                gap(x, from.0, w) > far && gap(y, from.1, world.height()) > far
            })
            .expect("this world has no second inland tile, so the test proves nothing");
        ((tile % w) as f32 + 0.5, (tile / w) as f32 + 0.5)
    }
}
