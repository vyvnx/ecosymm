//! everything a backend advances. no backend types appear here.

use crate::epoch::SimulationTime;
use ecosym_core::{derive_seed, Rng, SimConfig};
use ecosym_ecology::{FounderStreams, OrganismIds, Species, SpeciesBlueprint, SpeciesId};
use ecosym_genetics::GenomeIds;
use ecosym_world::World;

pub struct SimulationState {
    pub world: World,
    /// ordered. species 0 is not special; nothing here knows how many there are.
    pub species: Vec<Species>,
    pub time: SimulationTime,
    pub genome_ids: GenomeIds,
    pub organism_ids: OrganismIds,
}

impl SimulationState {
    /// world generation, each species' morphology, each species' founder
    /// brains and the engine all draw from their own named stream, so adding a
    /// founder field cannot silently shift every later random draw.
    pub fn found(cfg: &SimConfig, blueprints: &[SpeciesBlueprint]) -> SimulationState {
        let world = World::generate(derive_seed(cfg.seed, "world"), cfg.width, cfg.height);
        let mut genome_ids = GenomeIds::default();
        let mut organism_ids = OrganismIds::default();

        let species = blueprints
            .iter()
            .enumerate()
            .map(|(i, blueprint)| {
                // both streams are keyed by name, not by slot: which founders a
                // species gets is part of what that species *is*, so moving it
                // down the scenario list must not re-roll its bodies or its
                // brains. two blueprints sharing a name would therefore share
                // their founders, which is why every scenario here names its
                // species distinctly.
                //
                // morphology and brains stay separate streams, so adding a
                // founder field on one side cannot shift every later draw on
                // the other.
                let mut rng =
                    Rng::new(derive_seed(cfg.seed, &format!("species:{}", blueprint.name)));
                let mut brain_rng =
                    Rng::new(derive_seed(cfg.seed, &format!("brain:{}", blueprint.name)));
                Species::found(
                    SpeciesId::new(i as u32),
                    blueprint,
                    cfg.population_per_species,
                    &world,
                    &mut FounderStreams { morphology: &mut rng, brains: &mut brain_rng },
                    &mut genome_ids,
                    &mut organism_ids,
                )
            })
            .collect();

        SimulationState {
            world,
            species,
            time: SimulationTime::default(),
            genome_ids,
            organism_ids,
        }
    }

    pub fn population(&self) -> usize {
        self.species.iter().map(|s| s.population().len()).sum()
    }

    pub fn is_extinct(&self) -> bool {
        self.population() == 0
    }
}
