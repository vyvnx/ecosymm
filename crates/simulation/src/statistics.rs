//! per-species reporting and the winner rule. calculated outside the backend,
//! from canonical state plus `EpochEvents`.

use crate::epoch::EpochEvents;
use crate::state::SimulationState;
use ecosym_ecology::{BehaviorStats, Species};
use ecosym_genetics::{Genes, NeuralGenome};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesStats {
    pub id: u32,
    pub name: String,
    pub population: usize,
    pub births: usize,
    pub deaths: usize,
    pub mean_energy: f32,
    pub mean_genes: Genes,
    /// what this species' policies did over the epoch. descriptive.
    pub behavior: BehaviorStats,
    /// how much those actions varied across organism-ticks. **not** proof that
    /// individuals hold different strategies - one organism behaving
    /// differently at different moments reads exactly the same.
    pub behavior_variance: BehaviorStats,
    /// mean of every weight and bias across the living population. a compact
    /// signature that puts the evolving neural genes into the replay digest
    /// without hashing 140 floats per organism.
    pub mean_brain: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EpochReport {
    pub epoch: usize,
    pub population: usize,
    /// total resource left standing in the world
    pub biomass: f32,
    /// stable order, never a map: the order is part of the replay digest
    pub species: Vec<SpeciesStats>,
}

pub fn mean_genes(species: &Species) -> Genes {
    let organisms = species.population().organisms();
    if organisms.is_empty() {
        return Genes::default();
    }
    let mut sum = Genes::default();
    for o in organisms {
        let g = o.genes();
        sum.speed += g.speed;
        sum.size += g.size;
        sum.metabolism += g.metabolism;
        sum.heat_pref += g.heat_pref;
    }
    let n = organisms.len() as f32;
    Genes {
        speed: sum.speed / n,
        size: sum.size / n,
        metabolism: sum.metabolism / n,
        heat_pref: sum.heat_pref / n,
    }
}

/// mean weight and bias across the living population
pub fn mean_brain(species: &Species) -> f32 {
    let organisms = species.population().organisms();
    if organisms.is_empty() {
        return 0.0;
    }
    organisms.iter().map(|o| o.brain().mean()).sum::<f32>() / organisms.len() as f32
}

/// how far this species' policies have moved, as the distance between the
/// living population's centroid and the founding population's.
///
/// it starts at zero and grows: selection concentrating a population on the
/// descendants of the founders that worked is exactly what pulls the centroid
/// off its starting point.
pub fn brain_drift(species: &Species) -> f32 {
    let organisms = species.population().organisms();
    if organisms.is_empty() {
        return 0.0;
    }
    NeuralGenome::centroid(organisms.iter().map(|o| o.brain())).distance(species.founder_brain())
}

/// mean energy carried by a living organism
pub fn mean_energy(species: &Species) -> f32 {
    let organisms = species.population().organisms();
    if organisms.is_empty() {
        return 0.0;
    }
    organisms.iter().map(|o| o.energy).sum::<f32>() / organisms.len() as f32
}

pub fn species_stats(
    species: &Species,
    births: usize,
    deaths: usize,
    behavior: BehaviorStats,
    behavior_variance: BehaviorStats,
) -> SpeciesStats {
    let organisms = species.population().organisms();
    SpeciesStats {
        id: species.id().get(),
        name: species.name().to_string(),
        population: organisms.len(),
        births,
        deaths,
        mean_energy: mean_energy(species),
        mean_genes: mean_genes(species),
        behavior,
        behavior_variance,
        mean_brain: mean_brain(species),
    }
}

pub fn report(state: &SimulationState, events: &EpochEvents) -> EpochReport {
    EpochReport {
        epoch: state.time.epoch.0,
        population: state.population(),
        biomass: state.world.biomass(),
        species: state
            .species
            .iter()
            .enumerate()
            .map(|(i, s)| {
                species_stats(
                    s,
                    events.births.get(i).copied().unwrap_or(0),
                    events.deaths.get(i).copied().unwrap_or(0),
                    events.behavior.get(i).map(|b| b.mean()).unwrap_or_default(),
                    events.behavior.get(i).map(|b| b.variance()).unwrap_or_default(),
                )
            })
            .collect(),
    }
}

/// how one species did over the whole run. descriptive only - none of this is
/// read back into the simulation as a fitness function.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesResult {
    pub id: u32,
    pub name: String,
    pub initial: usize,
    pub final_population: usize,
    pub births: usize,
    pub deaths: usize,
    pub founder_genes: Genes,
    pub final_genes: Genes,
    /// mean energy carried by a surviving organism. how efficiently the evolved
    /// policy converts a grazed-down world into a living body.
    pub final_energy: f32,
    /// the species' behavioural fingerprint in its first and last recorded
    /// epoch. the pair is the evidence that behaviour itself was selected on.
    pub founder_behavior: BehaviorStats,
    pub final_behavior: BehaviorStats,
    /// mean absolute drift per neural gene away from the founder policy
    pub brain_drift: f32,
}

impl SpeciesResult {
    /// final over initial, and nothing more. a species that fell from 500 to 2
    /// outranks one that fell to 1; neither of them thrived.
    pub fn ratio(&self) -> f32 {
        if self.initial == 0 {
            0.0
        } else {
            self.final_population as f32 / self.initial as f32
        }
    }

    pub fn change(&self) -> i64 {
        self.final_population as i64 - self.initial as i64
    }

    pub fn change_pct(&self) -> f32 {
        if self.initial == 0 {
            0.0
        } else {
            (self.ratio() - 1.0) * 100.0
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Winner {
    Species(u32),
    /// several species share the leading ratio
    Tie(Vec<u32>),
    /// every final population is zero. total extinction, nobody won.
    None,
}

/// rank by final population over initial population. winning is relative and
/// says nothing about ecological health.
pub fn winner(results: &[SpeciesResult]) -> Winner {
    let mut best = 0.0f32;
    let mut leaders: Vec<u32> = Vec::new();
    for r in results {
        if r.final_population == 0 {
            continue;
        }
        let ratio = r.ratio();
        if leaders.is_empty() || ratio > best {
            best = ratio;
            leaders.clear();
            leaders.push(r.id);
        } else if ratio == best {
            leaders.push(r.id);
        }
    }
    match leaders.len() {
        0 => Winner::None,
        1 => Winner::Species(leaders[0]),
        _ => Winner::Tie(leaders),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub epochs: usize,
    pub species: Vec<SpeciesResult>,
    pub winner: Winner,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: u32, initial: usize, final_population: usize) -> SpeciesResult {
        SpeciesResult {
            id,
            name: format!("S{id}"),
            initial,
            final_population,
            births: 0,
            deaths: 0,
            founder_genes: Genes::default(),
            final_genes: Genes::default(),
            final_energy: 0.0,
            founder_behavior: BehaviorStats::default(),
            final_behavior: BehaviorStats::default(),
            brain_drift: 0.0,
        }
    }

    #[test]
    fn the_greatest_ratio_wins() {
        let r = [result(0, 500, 120), result(1, 500, 340)];
        assert_eq!(winner(&r), Winner::Species(1));
    }

    #[test]
    fn equal_leading_ratios_tie_and_losers_are_left_out() {
        let r = [result(0, 500, 300), result(1, 500, 300), result(2, 500, 10)];
        assert_eq!(winner(&r), Winner::Tie(vec![0, 1]));
    }

    #[test]
    fn total_extinction_has_no_winner() {
        assert_eq!(winner(&[result(0, 500, 0), result(1, 500, 0)]), Winner::None);
        assert_eq!(winner(&[]), Winner::None);
    }

    #[test]
    fn a_declining_winner_is_still_reported_as_a_decline() {
        let r = result(0, 500, 2);
        assert_eq!(winner(std::slice::from_ref(&r)), Winner::Species(0));
        assert_eq!(r.change(), -498);
        assert!((r.change_pct() - -99.6).abs() < 0.01);
    }

    #[test]
    fn ratio_ranks_unequal_founder_counts_by_share_kept() {
        let r = [result(0, 100, 50), result(1, 500, 200)];
        assert_eq!(winner(&r), Winner::Species(0));
    }
}
