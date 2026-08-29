//! the backend-neutral runner and the default scenario.

use crate::cpu::CpuEngine;
use crate::epoch::{EngineError, Epoch, EpochEngine};
use crate::state::SimulationState;
use crate::statistics::{self, EpochReport, RunOutcome, SpeciesResult};
use ecosym_core::{derive_seed, SimConfig};
use ecosym_ecology::{BehaviorStats, SpeciesBlueprint};
use ecosym_genetics::Genes;

/// the first scenario: two reproductively isolated herbivores competing for one
/// shared resource field.
///
/// A is fast, warm-adapted and expensive to run. B is slow, cool-adapted and
/// thrifty. neither profile is tuned to win - both have to be viable alone, and
/// the shared world decides the rest. neither is given a policy either: founder
/// brains come from each species' derived brain seed and nothing else.
pub fn default_blueprints() -> Vec<SpeciesBlueprint> {
    vec![
        SpeciesBlueprint {
            name: "Species A".into(),
            genes: Genes { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 },
            gene_spread: 0.12,
        },
        SpeciesBlueprint {
            name: "Species B".into(),
            genes: Genes { speed: 0.7, size: 1.0, metabolism: 0.8, heat_pref: 0.38 },
            gene_spread: 0.12,
        },
    ]
}

/// the controlled experiment: identical bodies, different founder policies.
///
/// both species get Species A's physical profile, so morphology cannot explain
/// any divergence. their brains still come from separate derived seeds, which
/// leaves the evolved policy as the only thing that differs between them.
pub fn twin_blueprints() -> Vec<SpeciesBlueprint> {
    let base = default_blueprints().swap_remove(0);
    (0..2).map(|i| SpeciesBlueprint { name: format!("Twin {i}"), ..base.clone() }).collect()
}

/// the ceiling is allocation protection, not carrying capacity, so it has to
/// sit well above anything the ecology can reach on its own. what bounds a
/// population eating one shared field is tiles, not how many founders were
/// asked for: with the world's productivity where it is now, the old
/// founder-only guard started refusing births the world could still feed,
/// which makes the guard part of the model instead of a backstop.
fn safety_ceiling(cfg: &SimConfig, species: usize) -> usize {
    ((cfg.population_per_species * species).max(100) * 10).max(cfg.width * cfg.height * 4)
}

/// owns the engine, the state and the run-long totals. it increments the epoch,
/// calculates statistics and decides when the run is finished; the engine only
/// advances state.
pub struct Simulation {
    pub cfg: SimConfig,
    pub state: SimulationState,
    engine: Box<dyn EpochEngine + Send>,
    initial: Vec<usize>,
    founder_genes: Vec<Genes>,
    births: Vec<usize>,
    deaths: Vec<usize>,
    /// the first and most recent epoch's behavioural fingerprints, so the run
    /// can report what selection did to behaviour without keeping every report
    first_behavior: Option<Vec<BehaviorStats>>,
    last_behavior: Vec<BehaviorStats>,
    ceiling_bound: bool,
}

impl Simulation {
    pub fn new(
        cfg: SimConfig,
        blueprints: &[SpeciesBlueprint],
        engine: Box<dyn EpochEngine + Send>,
    ) -> Simulation {
        let state = SimulationState::found(&cfg, blueprints);
        let n = state.species.len();
        Simulation {
            initial: state.species.iter().map(|s| s.population().len()).collect(),
            founder_genes: state.species.iter().map(|s| *s.founder_genes()).collect(),
            births: vec![0; n],
            deaths: vec![0; n],
            first_behavior: None,
            last_behavior: vec![BehaviorStats::default(); n],
            ceiling_bound: false,
            cfg,
            state,
            engine,
        }
    }

    /// the default cli path: the two-species scenario on the cpu engine.
    /// constructor injection, so a second backend needs no new option.
    pub fn cpu(cfg: SimConfig) -> Simulation {
        let blueprints = default_blueprints();
        let engine =
            CpuEngine::new(derive_seed(cfg.seed, "engine"), safety_ceiling(&cfg, blueprints.len()));
        Simulation::new(cfg, &blueprints, Box::new(engine))
    }

    /// any scenario, on the cpu engine
    pub fn cpu_with(cfg: SimConfig, blueprints: &[SpeciesBlueprint]) -> Simulation {
        let engine =
            CpuEngine::new(derive_seed(cfg.seed, "engine"), safety_ceiling(&cfg, blueprints.len()));
        Simulation::new(cfg, blueprints, Box::new(engine))
    }

    pub fn engine_id(&self) -> &'static str {
        self.engine.id()
    }

    pub fn population(&self) -> usize {
        self.state.population()
    }

    pub fn epoch(&self) -> usize {
        self.state.time.epoch.0
    }

    /// the safety ceiling refused a birth at some point in the run
    pub fn ceiling_bound(&self) -> bool {
        self.ceiling_bound
    }

    pub fn advance_epoch(&mut self) -> Result<EpochReport, EngineError> {
        let events = self.engine.advance_epoch(&mut self.state, self.cfg.ticks_per_epoch)?;
        self.state.time.epoch = Epoch(self.state.time.epoch.0 + 1);
        for (total, n) in self.births.iter_mut().zip(&events.births) {
            *total += n;
        }
        for (total, n) in self.deaths.iter_mut().zip(&events.deaths) {
            *total += n;
        }
        self.ceiling_bound |= events.ceiling_bound;
        let report = statistics::report(&self.state, &events);

        self.last_behavior = report.species.iter().map(|s| s.behavior).collect();
        self.first_behavior.get_or_insert_with(|| self.last_behavior.clone());
        Ok(report)
    }

    pub fn outcome(&self) -> RunOutcome {
        let species: Vec<SpeciesResult> = self
            .state
            .species
            .iter()
            .enumerate()
            .map(|(i, s)| SpeciesResult {
                id: s.id().get(),
                name: s.name().to_string(),
                initial: self.initial[i],
                final_population: s.population().len(),
                births: self.births[i],
                deaths: self.deaths[i],
                founder_genes: self.founder_genes[i],
                final_genes: statistics::mean_genes(s),
                final_energy: statistics::mean_energy(s),
                founder_behavior: behavior_at(self.first_behavior.as_deref(), i),
                final_behavior: behavior_at(Some(&self.last_behavior), i),
                brain_drift: statistics::brain_drift(s),
            })
            .collect();
        RunOutcome {
            winner: statistics::winner(&species),
            epochs: self.state.time.epoch.0,
            species,
        }
    }
}

fn behavior_at(all: Option<&[BehaviorStats]>, i: usize) -> BehaviorStats {
    all.and_then(|b| b.get(i)).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::Winner;

    fn small() -> SimConfig {
        SimConfig {
            seed: 1234,
            population_per_species: 60,
            epochs: 10,
            width: 64,
            height: 64,
            ticks_per_epoch: 10,
        }
    }

    fn run(cfg: SimConfig) -> (Vec<EpochReport>, RunOutcome) {
        let mut sim = Simulation::cpu(cfg.clone());
        let reports = (0..cfg.epochs).map(|_| sim.advance_epoch().unwrap()).collect();
        (reports, sim.outcome())
    }

    #[test]
    fn the_default_scenario_founds_two_species_of_equal_size() {
        let cfg = small();
        let sim = Simulation::cpu(cfg.clone());
        assert_eq!(sim.engine_id(), "cpu");
        assert_eq!(sim.state.species.len(), 2);
        for s in &sim.state.species {
            assert_eq!(s.population().len(), cfg.population_per_species);
            assert!(s.population().organisms().iter().all(|o| o.genome().is_founder()));
        }
        assert_ne!(sim.state.species[0].founder_genes(), sim.state.species[1].founder_genes());
    }

    #[test]
    fn founders_of_one_species_are_varied_not_cloned() {
        let sim = Simulation::cpu(small());
        let genes: Vec<_> =
            sim.state.species[0].population().organisms().iter().map(|o| *o.genes()).collect();
        assert!(genes.iter().any(|g| *g != genes[0]), "all founders identical");
        assert!(genes.iter().all(|g| g.in_bounds()));
    }

    #[test]
    fn the_same_seed_replays_identically_and_a_new_seed_does_not() {
        let (a, _) = run(small());
        let (b, _) = run(small());
        assert_eq!(a, b);
        let (c, _) = run(SimConfig { seed: 99, ..small() });
        assert_ne!(a, c);
    }

    #[test]
    fn the_same_seed_founds_byte_identical_worlds_and_populations() {
        let sample = |seed| {
            let sim = Simulation::cpu(SimConfig { seed, ..small() });
            let world = sim.state.world.fertility().to_vec();
            let founders: Vec<_> = sim
                .state
                .species
                .iter()
                .flat_map(|s| s.population().organisms().iter().map(|o| (*o.genes(), o.x, o.y)))
                .collect();
            (world, founders)
        };
        assert!(sample(1234) == sample(1234));
        let (w, f) = sample(1234);
        let (w2, f2) = sample(4321);
        assert_ne!(w, w2);
        assert_ne!(f, f2);
    }

    #[test]
    fn population_accounting_balances_per_species_and_globally() {
        let cfg = small();
        let mut sim = Simulation::cpu(cfg.clone());
        let mut running: Vec<usize> =
            sim.state.species.iter().map(|s| s.population().len()).collect();
        for _ in 0..cfg.epochs {
            let report = sim.advance_epoch().unwrap();
            for (i, s) in report.species.iter().enumerate() {
                running[i] = running[i] + s.births - s.deaths;
                assert_eq!(running[i], s.population, "species {i} accounting drifted");
            }
            assert_eq!(report.population, running.iter().sum::<usize>());
        }
    }

    #[test]
    fn each_profile_is_viable_on_its_own() {
        for blueprint in default_blueprints() {
            let cfg = small();
            let mut sim = Simulation::cpu_with(cfg.clone(), std::slice::from_ref(&blueprint));
            for _ in 0..cfg.epochs {
                sim.advance_epoch().unwrap();
            }
            let outcome = sim.outcome();
            assert!(
                outcome.species[0].final_population > 0,
                "{} went extinct alone: {outcome:?}",
                blueprint.name
            );
            assert!(outcome.species[0].births > 0, "{} never reproduced alone", blueprint.name);
            assert_eq!(outcome.winner, Winner::Species(0));
        }
    }

    /// immediate harvesting rewards whoever reaches a tile first, so storage
    /// position must not be a persistent advantage.
    ///
    /// the control is two species with the *same name*, which is what makes
    /// them genuinely identical: founder streams are keyed by name, so both
    /// draw the same bodies, the same brains and the same starting positions.
    /// The only thing left that differs is which slot they sit in, so any
    /// systematic gap between them has to have come from order alone.
    #[test]
    fn storage_position_is_not_an_advantage() {
        let twins: Vec<SpeciesBlueprint> = vec![default_blueprints().swap_remove(0); 2];
        let first_wins = [1234u64, 99, 7, 20260828, 555, 31337]
            .into_iter()
            .filter(|seed| {
                // enough founders that the early cull of unviable policies
                // leaves something to compare
                let cfg = SimConfig { seed: *seed, population_per_species: 200, ..small() };
                let mut sim = Simulation::cpu_with(cfg.clone(), &twins);
                for _ in 0..25 {
                    sim.advance_epoch().unwrap();
                }
                let o = sim.outcome();
                o.species[0].final_population > o.species[1].final_population
            })
            .count();
        assert!(
            (1..6).contains(&first_wins),
            "position 0 won {first_wins}/6 runs of two identical species"
        );
    }

    /// and the same profile must win whichever slot it is stored in.
    ///
    /// a species' founding policies are keyed to its name rather than its slot
    /// precisely so this stays true: moving a species down the list must not
    /// re-roll the brains its founders were born with.
    #[test]
    fn swapping_the_scenario_order_does_not_swap_the_winner() {
        let forward = default_blueprints();
        let mut reversed = forward.clone();
        reversed.reverse();

        for seed in [1234u64, 99, 7] {
            let champion = |order: &[SpeciesBlueprint]| {
                // the full founder count: seed 99 generates a poor world, and
                // a small founding population there does not survive the early
                // cull of unviable random policies long enough to prove anything
                let cfg = SimConfig { seed, population_per_species: 500, ..small() };
                let mut sim = Simulation::cpu_with(cfg, order);
                for _ in 0..60 {
                    sim.advance_epoch().unwrap();
                }
                let outcome = sim.outcome();
                match outcome.winner {
                    Winner::Species(id) => {
                        outcome.species.iter().find(|s| s.id == id).unwrap().name.clone()
                    }
                    other => panic!("seed {seed} produced {other:?}, so this proves nothing"),
                }
            };
            assert_eq!(
                champion(&forward),
                champion(&reversed),
                "seed {seed}: the winner followed the slot, not the profile"
            );
        }
    }

    /// acceptance: behaviour itself is under selection. neither the direction
    /// nor the strategy is asserted - only that the fingerprint measurably
    /// moved and the policies drifted with it.
    #[test]
    fn behaviour_measurably_changes_over_a_long_run() {
        let cfg = SimConfig { population_per_species: 300, epochs: 60, ..small() };
        let mut sim = Simulation::cpu(cfg.clone());
        for _ in 0..cfg.epochs {
            sim.advance_epoch().unwrap();
        }

        let outcome = sim.outcome();
        let survivors: Vec<_> = outcome.species.iter().filter(|s| s.final_population > 0).collect();
        assert!(!survivors.is_empty(), "everything died, so this proves nothing");

        for s in survivors {
            let (from, to) = (s.founder_behavior, s.final_behavior);
            let moved = [
                (to.movement - from.movement).abs(),
                (to.resource_tracking - from.resource_tracking).abs(),
                (to.reproduction - from.reproduction).abs(),
                (to.resting - from.resting).abs(),
            ];
            // 0.05 of a 0..1 tendency. the bar is not 0.1 any more because
            // how big a behaviour change *looks* depends on the activation:
            // softsign saturates slowly, so the same weight drift shows as a
            // smaller output move than `tanh` gave. `brain_drift` below is the
            // activation-independent half of the same claim.
            assert!(
                moved.iter().any(|d| *d > 0.05),
                "{}: behaviour barely moved, {from:?} -> {to:?}",
                s.name
            );
            assert!(s.brain_drift > 0.02, "{}: brains did not drift ({})", s.name, s.brain_drift);
        }
    }

    /// acceptance: no species is handed a better policy. founder brains are
    /// random draws, so a few hundred of them average out to no opinion at all
    /// on any tendency - whatever a species ends up doing, it got there itself.
    #[test]
    fn no_species_is_founded_with_a_deliberately_better_policy() {
        let cfg = SimConfig { population_per_species: 500, epochs: 1, ..small() };
        let mut sim = Simulation::cpu(cfg);
        sim.advance_epoch().unwrap();
        for s in &sim.outcome().species {
            let b = s.founder_behavior;
            for (name, v) in [("reproduction", b.reproduction), ("rest", b.resting)] {
                assert!(
                    (v - 0.5).abs() < 0.1,
                    "{} was founded leaning {name} at {v}, not neutral",
                    s.name
                );
            }
            // resource tracking is an alignment in -1..1, and a founder draw
            // does not sit at exactly 0: an organism eats the tile it is
            // standing on, so the food it left behind is always slightly
            // downhill of the food ahead of it, and any heading it holds for
            // more than a tick agrees with that wake a little. that is grazing,
            // not a policy - it is an order of magnitude under the 0.5 an
            // evolved tracker reaches, and neither species is handed more of
            // it than the other.
            assert!(
                b.resource_tracking.abs() < 0.15,
                "{} was founded tracking resources at {}, which is a strategy and not a wake",
                s.name,
                b.resource_tracking
            );
        }
    }

    /// acceptance: replay covers the brains too, not just the head count
    #[test]
    fn the_same_seed_reproduces_the_same_brains_and_the_same_behaviour() {
        let sample = || {
            let cfg = SimConfig { population_per_species: 120, epochs: 20, ..small() };
            let mut sim = Simulation::cpu(cfg.clone());
            for _ in 0..cfg.epochs {
                sim.advance_epoch().unwrap();
            }
            let brains: Vec<f32> = sim
                .state
                .species
                .iter()
                .flat_map(|s| s.population().organisms())
                .flat_map(|o| o.brain().genes().collect::<Vec<_>>())
                .collect();
            (sim.outcome(), brains)
        };
        let (outcome, brains) = sample();
        assert!(!brains.is_empty(), "everything died, so this proves nothing");
        assert_eq!(sample(), (outcome, brains));
    }

    /// the controlled experiment: identical bodies, so any divergence between
    /// the two is down to the policies evolution found for them
    #[test]
    fn the_twin_scenario_leaves_the_policy_as_the_only_difference() {
        let twins = twin_blueprints();
        assert_eq!(twins[0].genes, twins[1].genes, "the twins are not physically identical");

        // the default world, not the small one the rest of the suite uses.
        // mating is local now, so a founding colony has to be dense enough to
        // still find itself after its own founders die of old age - and two
        // identical species sharing one world halve each other's mate density,
        // which makes this the strictest density test here. at 300 founders on
        // 64x64, or 500 on 96x96, both twins go extinct in the trough and the
        // comparison this test exists for has nothing left to compare.
        let cfg = SimConfig {
            population_per_species: 500,
            epochs: 40,
            width: 128,
            height: 128,
            ticks_per_epoch: 20,
            ..small()
        };
        let mut sim = Simulation::cpu_with(cfg.clone(), &twins);
        assert_ne!(
            sim.state.species[0].founder_brain(),
            sim.state.species[1].founder_brain(),
            "the twins were founded on the same policies"
        );

        for _ in 0..cfg.epochs {
            sim.advance_epoch().unwrap();
        }
        let outcome = sim.outcome();
        assert_eq!(outcome.species[0].founder_genes, outcome.species[1].founder_genes);
        assert_ne!(
            outcome.species[0].final_behavior, outcome.species[1].final_behavior,
            "identical bodies converged on identical behaviour"
        );
    }

    #[test]
    fn the_outcome_reports_raw_numbers_alongside_the_winner() {
        let (_, outcome) = run(small());
        assert_eq!(outcome.epochs, small().epochs);
        assert_eq!(outcome.species.len(), 2);
        for s in &outcome.species {
            assert_eq!(s.initial, small().population_per_species);
            assert!(s.final_genes.is_finite());
            assert!(s.founder_genes.is_finite());
            assert!(s.founder_behavior.is_finite() && s.final_behavior.is_finite());
            assert!(s.brain_drift.is_finite() && s.brain_drift >= 0.0);
        }
        match outcome.winner {
            Winner::Species(id) => assert!(id < 2),
            Winner::Tie(ref ids) => assert!(ids.len() > 1),
            Winner::None => assert!(outcome.species.iter().all(|s| s.final_population == 0)),
        }
    }
}
