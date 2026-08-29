//! the backend-independent gate every `EpochEngine` implementation must pass.
//!
//! it is shipped in the library, not behind `#[cfg(test)]`, so a future
//! `ecosym-gpu` backend can run exactly these checks against itself:
//!
//! ```no_run
//! # use ecosym_simulation::{conformance, CpuEngine, EpochEngine};
//! conformance::verify_engine(&|seed, ceiling| Box::new(CpuEngine::new(seed, ceiling)));
//! ```
//!
//! backend-specific tests still have to cover device init, buffer transfer and
//! kernel failure. this suite only covers the model contract.

use crate::epoch::{EpochEngine, EpochEvents, Tick};
use crate::run::default_blueprints;
use crate::state::SimulationState;
use crate::statistics;
use ecosym_core::SimConfig;
use ecosym_ecology::SpeciesBlueprint;
use ecosym_genetics::Genome;
use std::collections::{HashMap, HashSet};

/// builds an engine from (seed, population ceiling)
pub type EngineFactory<'a> = dyn Fn(u64, usize) -> Box<dyn EpochEngine> + 'a;

const TICKS: usize = 10;

fn config(per_species: usize) -> SimConfig {
    SimConfig {
        seed: 20260828,
        population_per_species: per_species,
        epochs: 3,
        width: 48,
        height: 48,
        ticks_per_epoch: TICKS,
    }
}

/// scenario blueprints for `n` species, cycling the default profiles so a triad
/// exercises the same execution path as a duel
fn blueprints(n: usize) -> Vec<SpeciesBlueprint> {
    let base = default_blueprints();
    (0..n)
        .map(|i| {
            let mut bp = base[i % base.len()].clone();
            bp.name = format!("Species {i}");
            bp
        })
        .collect()
}

/// run the whole suite. panics with the failing check.
pub fn verify_engine(new_engine: &EngineFactory<'_>) {
    for species in 0..=3 {
        genericity_case(new_engine, species);
    }
    determinism(new_engine);
    a_dead_world_does_not_panic(new_engine);
}

/// one full pass of the model contract for a scenario with `species` species
fn genericity_case(new_engine: &EngineFactory<'_>, species: usize) {
    let cfg = config(if species == 0 { 0 } else { 25 });
    let blueprints = blueprints(species);
    let mut state = SimulationState::found(&cfg, &blueprints);
    let mut engine = new_engine(cfg.seed, 10_000);
    let mut census = Census::new(&state);

    for epoch in 0..cfg.epochs {
        let before: Vec<usize> = state.species.iter().map(|s| s.population().len()).collect();
        let ages = census.ages(&state);
        let clock = state.time.epoch;

        let events = engine
            .advance_epoch(&mut state, cfg.ticks_per_epoch)
            .unwrap_or_else(|e| panic!("{} failed on epoch {epoch}: {e}", engine.id()));

        assert_eq!(
            events.births.len(),
            species,
            "{}: births must be reported per species",
            engine.id()
        );
        assert_eq!(
            events.deaths.len(),
            species,
            "{}: deaths must be reported per species",
            engine.id()
        );
        assert_eq!(
            state.time.tick,
            Tick(cfg.ticks_per_epoch),
            "{}: an epoch must run exactly ticks_per_epoch ticks",
            engine.id()
        );
        assert_eq!(
            state.time.epoch,
            clock,
            "{}: the runner owns the epoch counter, not the engine",
            engine.id()
        );

        assert_eq!(
            events.behavior.len(),
            species,
            "{}: behaviour must be reported per species",
            engine.id()
        );

        accounting_balances(&state, &before, &events, engine.id());
        every_survivor_was_visited_each_tick(&state, &ages, cfg.ticks_per_epoch, engine.id());
        every_survivor_ran_its_policy(&state, &ages, &events, cfg.ticks_per_epoch, engine.id());
        lifetime_memory_belongs_to_the_body(&state, &ages, engine.id());
        census.check(&state, engine.id());
        world_stays_within_bounds(&state, engine.id());
        everybody_is_standing_on_land(&state, engine.id());
        nothing_is_nan(&state, engine.id());

        // reporting must work for any species count, including zero
        let report = statistics::report(&state, &events);
        assert_eq!(report.species.len(), species);
        assert_eq!(report.population, state.population());
        assert!(report.biomass.is_finite());
        for s in &report.species {
            assert!(s.behavior.is_finite(), "{}: behaviour is not finite", engine.id());
            assert!(s.mean_brain.is_finite(), "{}: mean brain is not finite", engine.id());
        }

        state.time.epoch.0 += 1;
    }
}

fn accounting_balances(
    state: &SimulationState,
    before: &[usize],
    events: &EpochEvents,
    engine: &str,
) {
    for (i, species) in state.species.iter().enumerate() {
        assert_eq!(
            species.population().len(),
            before[i] + events.births[i] - events.deaths[i],
            "{engine}: species {i} does not satisfy end = start + births - deaths"
        );
    }
    let start: usize = before.iter().sum();
    assert_eq!(
        state.population(),
        start + events.total_births() - events.total_deaths(),
        "{engine}: global population accounting drifted"
    );
}

/// every organism alive before and after the epoch must have aged exactly one
/// tick per tick, which is only true if the visit order covered it each time
fn every_survivor_was_visited_each_tick(
    state: &SimulationState,
    ages_before: &HashMap<u64, u32>,
    ticks: usize,
    engine: &str,
) {
    for species in &state.species {
        for o in species.population().organisms() {
            if let Some(before) = ages_before.get(&o.id().get()) {
                assert_eq!(
                    o.age,
                    before + ticks as u32,
                    "{engine}: organism {} was visited {} times in {ticks} ticks",
                    o.id().get(),
                    o.age - before
                );
            }
        }
    }
}

/// the inherited policy has to run for everybody, every tick. a backend that
/// quietly skipped the forward pass for some organisms would still balance its
/// books, so count the recorded acts against the organisms that lived through
/// the whole epoch.
fn every_survivor_ran_its_policy(
    state: &SimulationState,
    ages_before: &HashMap<u64, u32>,
    events: &EpochEvents,
    ticks: usize,
    engine: &str,
) {
    for (i, species) in state.species.iter().enumerate() {
        let carried = species
            .population()
            .organisms()
            .iter()
            .filter(|o| ages_before.contains_key(&o.id().get()))
            .count();
        let acts = events.behavior[i].acts() as usize;
        assert!(
            acts >= carried * ticks,
            "{engine}: species {i} carried {carried} organisms through {ticks} ticks but \
             recorded only {acts} decisions"
        );
    }
}

/// hidden state is the organism's, not the genome's. a backend that kept one
/// scratch buffer per thread, or wrote memory back into the brain, or handed a
/// newborn its parent's state, would still balance its books and still run
/// everybody's policy - so check the two ends directly.
///
/// a newborn is any organism the previous epoch had never seen. its memory has
/// to be zero, because it has not observed anything yet. a survivor's memory
/// has to be bounded, because `tanh` produced it.
fn lifetime_memory_belongs_to_the_body(
    state: &SimulationState,
    ages_before: &HashMap<u64, u32>,
    engine: &str,
) {
    for species in &state.species {
        for o in species.population().organisms() {
            assert!(
                o.hidden.iter().all(|h| (-1.0..=1.0).contains(h)),
                "{engine}: organism {} carries a hidden activation outside -1..1: {:?}",
                o.id().get(),
                o.hidden
            );
            // born during this epoch, and not yet through a full one
            if !ages_before.contains_key(&o.id().get()) && o.age == 0 {
                assert_eq!(
                    o.hidden,
                    [0.0; ecosym_genetics::HIDDEN],
                    "{engine}: newborn {} was handed memory it did not earn",
                    o.id().get()
                );
            }
        }
    }
}

fn world_stays_within_bounds(state: &SimulationState, engine: &str) {
    for (i, standing) in state.world.resources().iter().enumerate() {
        assert!(
            *standing >= 0.0 && *standing <= state.world.capacity_at(i) + 1e-6,
            "{engine}: tile {i} holds {standing}, capacity {}",
            state.world.capacity_at(i)
        );
    }
}

/// terrain is not advisory. no organism may be founded, born, moved or left
/// anywhere it cannot stand, whatever its policy asked for.
fn everybody_is_standing_on_land(state: &SimulationState, engine: &str) {
    for species in &state.species {
        for o in species.population().organisms() {
            assert!(
                state.world.is_passable(state.world.idx(o.x, o.y)),
                "{engine}: organism {} is standing at {}, {}, which cannot be walked on",
                o.id().get(),
                o.x,
                o.y
            );
        }
    }
}

fn nothing_is_nan(state: &SimulationState, engine: &str) {
    assert!(state.world.biomass().is_finite(), "{engine}: biomass is not finite");
    assert!(
        state.world.resources().iter().all(|r| r.is_finite()),
        "{engine}: a tile is not finite"
    );
    for species in &state.species {
        for o in species.population().organisms() {
            assert!(
                o.is_finite(),
                "{engine}: organism {} carries a non-finite value",
                o.id().get()
            );
            assert!(o.genes().in_bounds(), "{engine}: genes escaped their viable range");
            assert!(o.brain().in_bounds(), "{engine}: neural weights escaped their viable range");
        }
    }
}

/// the same seed must reproduce the same run for this backend. cross-backend
/// bit equality is explicitly not promised.
fn determinism(new_engine: &EngineFactory<'_>) {
    let run = || {
        let cfg = config(25);
        let mut state = SimulationState::found(&cfg, &blueprints(2));
        let mut engine = new_engine(cfg.seed, 10_000);
        let mut trace = Vec::new();
        for _ in 0..cfg.epochs {
            let events = engine.advance_epoch(&mut state, cfg.ticks_per_epoch).unwrap();
            trace.push(statistics::report(&state, &events));
            state.time.epoch.0 += 1;
        }
        (trace, snapshot(&state))
    };
    assert_eq!(run(), run(), "the engine is not deterministic for a fixed seed");
}

/// (organism id, genome id, x, y, energy, age, mean brain, hidden state)
type OrganismRow = (u64, u64, f32, f32, f32, u32, f32, [f32; ecosym_genetics::HIDDEN]);

fn snapshot(state: &SimulationState) -> Vec<(u32, Vec<OrganismRow>)> {
    state
        .species
        .iter()
        .map(|s| {
            (
                s.id().get(),
                s.population()
                    .organisms()
                    .iter()
                    .map(|o| {
                        (
                            o.id().get(),
                            o.genome().id().get(),
                            o.x,
                            o.y,
                            o.energy,
                            o.age,
                            o.brain().mean(),
                            o.hidden,
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

/// a world where everything just died must keep running without panicking
fn a_dead_world_does_not_panic(new_engine: &EngineFactory<'_>) {
    let cfg = config(10);
    let mut state = SimulationState::found(&cfg, &blueprints(2));
    for species in &mut state.species {
        for i in 0..species.population().len() {
            species.population_mut().get_mut(i).unwrap().energy = -1.0;
        }
    }
    let mut engine = new_engine(cfg.seed, 10_000);
    for _ in 0..3 {
        let events = engine.advance_epoch(&mut state, cfg.ticks_per_epoch).unwrap();
        let report = statistics::report(&state, &events);
        assert!(report.biomass.is_finite());
    }
    assert_eq!(state.population(), 0, "the dead came back");
}

/// tracks identity across epochs: ids are never reused, a living genome never
/// changes, and no offspring ever has a parent from another species.
struct Census {
    genome_species: HashMap<u64, usize>,
    genomes: HashMap<u64, Genome>,
    organism_ids: HashSet<u64>,
    organism_species: HashMap<u64, usize>,
}

impl Census {
    fn new(state: &SimulationState) -> Census {
        let mut census = Census {
            genome_species: HashMap::new(),
            genomes: HashMap::new(),
            organism_ids: HashSet::new(),
            organism_species: HashMap::new(),
        };
        census.check(state, "founding");
        census
    }

    fn ages(&self, state: &SimulationState) -> HashMap<u64, u32> {
        state
            .species
            .iter()
            .flat_map(|s| s.population().organisms())
            .map(|o| (o.id().get(), o.age))
            .collect()
    }

    fn check(&mut self, state: &SimulationState, engine: &str) {
        let mut live_organisms = HashSet::new();
        let mut live_genomes = HashSet::new();

        for (index, species) in state.species.iter().enumerate() {
            for o in species.population().organisms() {
                let oid = o.id().get();
                let genome = *o.genome();
                let gid = genome.id().get();

                assert!(live_organisms.insert(oid), "{engine}: organism id {oid} is alive twice");
                assert!(live_genomes.insert(gid), "{engine}: genome id {gid} is alive twice");

                match self.organism_species.get(&oid) {
                    Some(known) => {
                        assert_eq!(*known, index, "{engine}: organism {oid} changed species")
                    }
                    None => {
                        assert!(
                            self.organism_ids.insert(oid),
                            "{engine}: organism id {oid} was reused"
                        );
                        self.organism_species.insert(oid, index);
                    }
                }

                match self.genomes.get(&gid) {
                    Some(known) => assert_eq!(
                        *known, genome,
                        "{engine}: genome {gid} changed while its organism was alive"
                    ),
                    None => {
                        // a genome id is minted once, for one species, and both
                        // genetic parents must already belong to that species
                        assert!(
                            self.genome_species.insert(gid, index).is_none(),
                            "{engine}: genome id {gid} was reused"
                        );
                        for parent in genome.parent_ids().into_iter().flatten() {
                            assert_eq!(
                                self.genome_species.get(&parent.get()),
                                Some(&index),
                                "{engine}: genome {gid} has a parent outside its own species"
                            );
                        }
                        self.genomes.insert(gid, genome);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::CpuEngine;

    #[test]
    fn the_cpu_engine_conforms() {
        verify_engine(&|seed, ceiling| Box::new(CpuEngine::new(seed, ceiling)));
    }
}
