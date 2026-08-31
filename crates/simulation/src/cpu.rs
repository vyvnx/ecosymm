//! the cpu epoch engine.
//!
//! this file is scheduling, storage and the population ceiling - who runs when,
//! in what order, and what happens when births outrun memory. every rule it
//! applies comes from `ecosym-ecology`; none is written here. that is the line
//! a second backend has to hold too: see the tick contract on `EpochEngine`.

use crate::epoch::{EngineError, EpochEngine, EpochEvents, Tick};
use crate::state::SimulationState;
use ecosym_core::{derive_seed, hash_u64, Rng};
use ecosym_ecology::{behavior, CellIndex, Conception, MateSearch};

/// a birth conceived during the visit pass and admitted afterwards. ids are
/// minted at admission, so a birth the ceiling refuses never burns one.
#[derive(Clone, Copy)]
struct QueuedBirth {
    parent: usize,
    conception: Conception,
}

pub struct CpuEngine {
    /// visit order is re-derived per tick from (seed, epoch, tick), so it is
    /// independent of how many organisms bred last tick.
    order_seed: u64,
    behavior: Rng,
    reproduction: Rng,
    /// ponytail: hard ceiling so a runaway bloom cannot eat all the ram.
    /// carrying capacity normally binds first. this is allocation protection,
    /// not the ecological model - `EpochEvents::ceiling_bound` says when it hit.
    max_population: usize,
    /// scratch, reused every tick
    handles: Vec<(usize, usize)>,
    /// each visited organism's reproductive pressure, aligned with `handles`.
    /// buffered rather than acted on, so conception can wait for everybody to
    /// have finished moving.
    intent: Vec<f32>,
    /// who is standing where, as of the start of the current tick. derived
    /// state, so it belongs to the backend rather than to `SimulationState` -
    /// what it *means* is ecology's, in `CellIndex`.
    occupancy: CellIndex,
}

impl CpuEngine {
    pub const ID: &'static str = "cpu";

    pub fn new(seed: u64, max_population: usize) -> CpuEngine {
        CpuEngine {
            order_seed: derive_seed(seed, "order"),
            behavior: Rng::new(derive_seed(seed, "behavior")),
            reproduction: Rng::new(derive_seed(seed, "reproduction")),
            max_population,
            handles: Vec::new(),
            intent: Vec::new(),
            occupancy: CellIndex::default(),
        }
    }

    pub fn max_population(&self) -> usize {
        self.max_population
    }

    /// deterministic fisher-yates over every snapshot organism handle. it must
    /// cover each exactly once and leave no species or storage slot with a
    /// persistent first-mover advantage on the shared resource field.
    fn shuffle_handles(&mut self, state: &SimulationState, tick: usize) {
        self.handles.clear();
        for (s, species) in state.species.iter().enumerate() {
            self.handles.extend((0..species.population().len()).map(|i| (s, i)));
        }
        let mut rng =
            Rng::new(hash_u64(hash_u64(self.order_seed, state.time.epoch.0 as u64), tick as u64));
        for i in (1..self.handles.len()).rev() {
            let j = rng.below(i + 1);
            self.handles.swap(i, j);
        }
    }

    fn tick(&mut self, state: &mut SimulationState, tick: usize, events: &mut EpochEvents) {
        // 1 + 2. snapshot the living population and fix a visit order over it.
        // newborns are appended after the pass, so they cannot act or mate in
        // their birth tick.
        self.shuffle_handles(state, tick);

        // 3. one crowd snapshot for the whole tick, so density is the one
        // observation the visit order cannot skew
        self.occupancy.rebuild(&state.species, &state.world);

        self.intent.clear();
        self.intent.resize(self.handles.len(), 0.0);

        for k in 0..self.handles.len() {
            let (s, i) = self.handles[k];
            let SimulationState { world, species, .. } = &mut *state;
            let Some(current) = species[s].population().get(i).copied() else {
                continue;
            };

            // 4. observe, decide, move, forage, pay upkeep, age. the rule is
            // ecology's, and so is the policy that chose the action.
            let (next, act) =
                behavior::live_one_tick(&current, s, world, &self.occupancy, &mut self.behavior);
            events.behavior[s].record(&act);
            // the policy's reproductive pressure is buffered, not acted on. it
            // is an input to the rule in pass two, never a bypass.
            self.intent[k] = act.reproduction;

            if let Some(slot) = species[s].population_mut().get_mut(i) {
                *slot = next;
            }
        }

        // 5. everybody has moved, so rebuild before anyone looks for a mate.
        // one coherent snapshot for the whole conception pass: no breeder may
        // see moved positions while another sees the old ones.
        self.occupancy.rebuild(&state.species, &state.world);

        // 6. resolve conceptions, in the same visit order the pass above used
        let mut queued: Vec<Vec<QueuedBirth>> = vec![Vec::new(); state.species.len()];
        for k in 0..self.handles.len() {
            let (s, i) = self.handles[k];
            let Some(parent) = state.species[s].population().get(i).copied() else {
                continue;
            };
            if let Some(conception) = behavior::conceive(
                &parent,
                self.intent[k],
                &MateSearch {
                    species: s,
                    breeder: i,
                    population: state.species[s].population(),
                    world: &state.world,
                    index: &self.occupancy,
                },
                &mut self.reproduction,
            ) {
                queued[s].push(QueuedBirth { parent: i, conception });
            }
        }

        // 7 + 8. append births, then remove the dead with stable retention
        self.admit_births(state, &queued, events);
        for (s, species) in state.species.iter_mut().enumerate() {
            events.deaths[s] += species.population_mut().retain_living();
        }

        // 9. the world grows back
        state.world.regrow();
        state.time.tick = Tick(tick + 1);
    }

    /// admit queued births up to the safety ceiling, round-robin across species
    /// so a large population cannot crowd a small one out of the last slots.
    fn admit_births(
        &mut self,
        state: &mut SimulationState,
        queued: &[Vec<QueuedBirth>],
        events: &mut EpochEvents,
    ) {
        let capacity = self.max_population.saturating_sub(state.population());
        let wanted: usize = queued.iter().map(|q| q.len()).sum();
        if wanted > capacity {
            events.ceiling_bound = true;
        }

        let mut cursors = vec![0usize; queued.len()];
        let mut admitted = 0;
        while admitted < capacity {
            let mut progressed = false;
            for s in 0..queued.len() {
                if admitted >= capacity {
                    break;
                }
                let Some(birth) = queued[s].get(cursors[s]) else {
                    continue;
                };
                cursors[s] += 1;
                admitted += 1;
                progressed = true;

                let SimulationState { species, genome_ids, organism_ids, .. } = &mut *state;
                let population = species[s].population_mut();
                let Some(parent) = population.get(birth.parent).copied() else {
                    continue;
                };
                let child = birth.conception.birth(organism_ids.mint(), genome_ids.mint(), &parent);
                if let Some(parent) = population.get_mut(birth.parent) {
                    parent.energy -= birth.conception.energy;
                }
                population.push(child);
                events.births[s] += 1;
            }
            if !progressed {
                break;
            }
        }
    }
}

impl EpochEngine for CpuEngine {
    fn id(&self) -> &'static str {
        CpuEngine::ID
    }

    fn advance_epoch(
        &mut self,
        state: &mut SimulationState,
        ticks: usize,
    ) -> Result<EpochEvents, EngineError> {
        // 9. births, deaths and behaviour accumulate across the whole epoch,
        // by species
        let mut events = EpochEvents::for_species(state.species.len());
        state.time.tick = Tick(0);
        for t in 0..ticks {
            self.tick(state, t, &mut events);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::default_blueprints;
    use ecosym_core::SimConfig;
    use ecosym_ecology::SpeciesBlueprint;

    fn state(species: usize, per_species: usize) -> (SimConfig, SimulationState) {
        let cfg = SimConfig {
            seed: 1234,
            population_per_species: per_species,
            epochs: 2,
            width: 48,
            height: 48,
            ticks_per_epoch: 5,
        };
        let blueprints = default_blueprints();
        // distinct names, because founders are keyed by species name
        let chosen: Vec<_> = (0..species)
            .map(|i| SpeciesBlueprint {
                name: format!("Species {i}"),
                ..blueprints[i % blueprints.len()].clone()
            })
            .collect();
        let state = SimulationState::found(&cfg, &chosen);
        (cfg, state)
    }

    fn visit_order(
        engine: &mut CpuEngine,
        state: &SimulationState,
        tick: usize,
    ) -> Vec<(usize, usize)> {
        engine.shuffle_handles(state, tick);
        engine.handles.clone()
    }

    #[test]
    fn the_permutation_covers_every_snapshot_organism_exactly_once() {
        let (_, state) = state(3, 20);
        let mut engine = CpuEngine::new(7, 10_000);
        let mut order = visit_order(&mut engine, &state, 0);
        assert_eq!(order.len(), 60);
        order.sort_unstable();
        order.dedup();
        assert_eq!(order.len(), 60);
    }

    #[test]
    fn a_fixed_seed_reproduces_the_order_and_position_zero_does_not_lead() {
        let (_, state) = state(2, 30);
        let mut a = CpuEngine::new(7, 10_000);
        let mut b = CpuEngine::new(7, 10_000);
        assert_eq!(visit_order(&mut a, &state, 4), visit_order(&mut b, &state, 4));
        assert_ne!(visit_order(&mut a, &state, 4), visit_order(&mut a, &state, 5));

        let firsts: Vec<(usize, usize)> =
            (0..40).map(|t| visit_order(&mut a, &state, t)[0]).collect();
        assert!(firsts.iter().filter(|h| **h == (0, 0)).count() < 5, "{firsts:?}");
        assert!(firsts.iter().any(|h| h.0 == 1), "species 1 never went first");
    }

    #[test]
    fn an_epoch_runs_every_tick_and_leaves_the_epoch_counter_to_the_runner() {
        let (cfg, mut state) = state(2, 20);
        let mut engine = CpuEngine::new(7, 10_000);
        engine.advance_epoch(&mut state, cfg.ticks_per_epoch).unwrap();
        assert_eq!(state.time.tick, Tick(cfg.ticks_per_epoch));
        assert_eq!(state.time.epoch.0, 0);
    }

    #[test]
    fn the_ceiling_binds_fairly_instead_of_starving_one_species() {
        let (cfg, mut state) = state(2, 40);
        let ceiling = state.population() + 3;
        let mut engine = CpuEngine::new(7, ceiling);
        let mut bound = false;
        for _ in 0..20 {
            let events = engine.advance_epoch(&mut state, cfg.ticks_per_epoch).unwrap();
            bound |= events.ceiling_bound;
            assert!(state.population() <= ceiling);
        }
        assert!(bound, "the ceiling never bound, so this test proved nothing");
    }
}
