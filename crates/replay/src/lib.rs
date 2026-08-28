//! deterministic runs: fold every epoch into one checksum.
//!
//! same seed + same config + same engine must give the same digest. if it stops
//! doing that, something in the epoch loop picked up an unordered iteration or
//! a wall clock.

use ecosym_core::{hash_bytes, hash_f32, hash_u64, SimConfig, HASH_INIT};
use ecosym_simulation::EpochReport;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recorder {
    pub config: SimConfig,
    /// the backend that produced this run. two engines are not promised to be
    /// bit-identical, so the digest is only meaningful together with this.
    pub engine: String,
    pub history: Vec<EpochReport>,
    digest: u64,
}

impl Recorder {
    pub fn new(config: SimConfig, engine: &str) -> Recorder {
        let digest = hash_bytes(hash_u64(HASH_INIT, config.seed), engine.as_bytes());
        Recorder { config, engine: engine.to_string(), history: Vec::new(), digest }
    }

    pub fn push(&mut self, report: EpochReport) {
        self.digest = hash_u64(self.digest, report.population as u64);
        self.digest = hash_f32(self.digest, report.biomass);
        // species are folded in list order, so reordering them is a different run
        for s in &report.species {
            self.digest = hash_u64(self.digest, s.id as u64);
            self.digest = hash_u64(self.digest, s.population as u64);
            self.digest = hash_u64(self.digest, s.births as u64);
            self.digest = hash_u64(self.digest, s.deaths as u64);
            self.digest = hash_f32(self.digest, s.mean_energy);
            self.digest = hash_f32(self.digest, s.mean_genes.speed);
            self.digest = hash_f32(self.digest, s.mean_genes.size);
            self.digest = hash_f32(self.digest, s.mean_genes.metabolism);
            self.digest = hash_f32(self.digest, s.mean_genes.heat_pref);
            // the evolving neural genes and what they did. a run whose brains
            // drift differently is a different run, and the digest has to say so.
            self.digest = hash_f32(self.digest, s.mean_brain);
            self.digest = hash_f32(self.digest, s.behavior.movement);
            self.digest = hash_f32(self.digest, s.behavior.food_seeking);
            self.digest = hash_f32(self.digest, s.behavior.reproduction);
            self.digest = hash_f32(self.digest, s.behavior.resting);
            self.digest = hash_f32(self.digest, s.behavior.competitor_exposure);
        }
        self.history.push(report);
    }

    pub fn digest(&self) -> u64 {
        self.digest
    }

    pub fn digest_hex(&self) -> String {
        format!("{:016x}", self.digest)
    }

    pub fn epochs(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_ecology::{BehaviorStats, SpeciesBlueprint};
    use ecosym_genetics::Genes;
    use ecosym_simulation::{default_blueprints, Simulation, SpeciesStats};

    fn stats(id: u32, population: usize) -> SpeciesStats {
        SpeciesStats {
            id,
            name: format!("S{id}"),
            population,
            births: 1,
            deaths: 1,
            mean_energy: 3.0,
            mean_genes: Genes { speed: population as f32 * 0.001, ..Genes::default() },
            behavior: BehaviorStats::default(),
            mean_brain: 0.0,
        }
    }

    fn rec(seed: u64, engine: &str, epochs: &[Vec<SpeciesStats>]) -> Recorder {
        let mut r = Recorder::new(SimConfig { seed, ..Default::default() }, engine);
        for (i, species) in epochs.iter().enumerate() {
            r.push(EpochReport {
                epoch: i,
                population: species.iter().map(|s| s.population).sum(),
                biomass: 100.0,
                species: species.clone(),
            });
        }
        r
    }

    fn run(seed: u64) -> Recorder {
        rec(seed, "cpu", &[vec![stats(0, 10), stats(1, 20)], vec![stats(0, 12), stats(1, 18)]])
    }

    #[test]
    fn digest_tracks_seed_engine_and_history() {
        assert_eq!(run(1).digest(), run(1).digest());
        assert_ne!(run(1).digest(), run(2).digest());
        assert_eq!(run(1).digest_hex().len(), 16);
        assert_eq!(run(1).epochs(), 2);

        let other_engine = rec("cpu".len() as u64, "wgpu", &[vec![stats(0, 10)]]);
        let same_data = rec("cpu".len() as u64, "cpu", &[vec![stats(0, 10)]]);
        assert_ne!(other_engine.digest(), same_data.digest());
    }

    /// the genericity matrix reaches replay too: zero, one, two and three
    /// species must all record without a panic or a special case
    #[test]
    fn any_species_count_records_and_stays_seed_deterministic() {
        let digest = |species: usize, seed: u64| {
            let cfg = SimConfig {
                seed,
                population_per_species: 20,
                epochs: 5,
                width: 48,
                height: 48,
                ticks_per_epoch: 10,
            };
            let base = default_blueprints();
            let scenario: Vec<_> = (0..species)
                .map(|i| SpeciesBlueprint {
                    name: format!("Species {i}"),
                    ..base[i % base.len()].clone()
                })
                .collect();
            let mut sim = Simulation::cpu_with(cfg.clone(), &scenario);
            let mut rec = Recorder::new(cfg.clone(), sim.engine_id());
            for _ in 0..cfg.epochs {
                let report = sim.advance_epoch().unwrap();
                assert_eq!(report.species.len(), species);
                rec.push(report);
            }
            assert_eq!(rec.epochs(), cfg.epochs);
            rec.digest()
        };

        let counts: Vec<u64> = (0..=3).map(|n| digest(n, 1234)).collect();
        for (n, expected) in counts.iter().enumerate() {
            assert_eq!(*expected, digest(n, 1234), "{n} species is not deterministic");
        }
        // and each scenario is a distinguishable run
        let mut unique = counts.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), counts.len(), "species count does not reach the digest");
        assert_ne!(digest(2, 1234), digest(2, 99));
    }

    /// neural weights and the behaviour they produce are part of a run, so a
    /// replay that reproduces the populations but not the brains is not a replay
    #[test]
    fn digest_detects_a_change_in_brains_or_behaviour_alone() {
        let with = |mutate: fn(&mut SpeciesStats)| {
            let mut s = stats(0, 10);
            mutate(&mut s);
            rec(1, "cpu", &[vec![s]]).digest()
        };
        let baseline = with(|_| {});
        assert_ne!(baseline, with(|s| s.mean_brain = 0.01), "neural weights miss the digest");
        assert_ne!(baseline, with(|s| s.behavior.movement = 0.01));
        assert_ne!(baseline, with(|s| s.behavior.food_seeking = 0.01));
        assert_ne!(baseline, with(|s| s.behavior.reproduction = 0.01));
        assert_ne!(baseline, with(|s| s.behavior.resting = 0.01));
        assert_ne!(baseline, with(|s| s.behavior.competitor_exposure = 0.01));
    }

    #[test]
    fn digest_detects_species_order_and_per_species_changes() {
        let ordered = rec(1, "cpu", &[vec![stats(0, 10), stats(1, 20)]]);
        let swapped = rec(1, "cpu", &[vec![stats(1, 20), stats(0, 10)]]);
        assert_ne!(ordered.digest(), swapped.digest());

        // the totals match, only the split between species moved
        let moved = rec(1, "cpu", &[vec![stats(0, 11), stats(1, 19)]]);
        assert_ne!(ordered.digest(), moved.digest());

        // and epoch order still matters
        let a = rec(1, "cpu", &[vec![stats(0, 10)], vec![stats(0, 20)]]);
        let b = rec(1, "cpu", &[vec![stats(0, 20)], vec![stats(0, 10)]]);
        assert_ne!(a.digest(), b.digest());
    }
}
