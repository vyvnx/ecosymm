//! a species and the population it owns.

use crate::{Organism, OrganismIds, Population};
use ecosym_core::Rng;
use ecosym_genetics::{Genes, Genome, GenomeIds, NeuralGenome};
use ecosym_world::World;
use serde::{Deserialize, Serialize};

/// energy every founder starts with
pub const START_ENERGY: f32 = 5.0;

/// stable identity for one species, assigned by the scenario in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SpeciesId(u32);

impl SpeciesId {
    pub fn new(n: u32) -> SpeciesId {
        SpeciesId(n)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// the recipe for a founder population. used once, at construction, and never
/// consulted again - it is not a live template organisms drift back toward.
///
/// there is deliberately no brain here. founder policies come from a derived
/// seed and nothing else, so no species can be handed a hand-written strategy
/// and the scenario author cannot accidentally pick the winner.
#[derive(Clone, Debug)]
pub struct SpeciesBlueprint {
    pub name: String,
    pub genes: Genes,
    /// half-width of the uniform variation applied to each founder
    pub gene_spread: f32,
}

/// the two independent random streams a founder population is drawn from.
///
/// separate on purpose: bodies and brains must not share a stream, or adding
/// one morphology field silently re-rolls every founder policy after it.
pub struct FounderStreams<'a> {
    pub morphology: &'a mut Rng,
    pub brains: &'a mut Rng,
}

/// a species owns exactly one population, and that population owns its
/// organisms. reproductive isolation falls out of that ownership.
pub struct Species {
    id: SpeciesId,
    name: String,
    founder_genes: Genes,
    founder_brain: NeuralGenome,
    population: Population,
}

impl Species {
    /// morphology and brains draw from two separate streams. adding a founder
    /// field on one side therefore cannot silently shift every later draw on
    /// the other, which is the same discipline `derive_seed` exists for.
    pub fn found(
        id: SpeciesId,
        blueprint: &SpeciesBlueprint,
        count: usize,
        world: &World,
        streams: &mut FounderStreams<'_>,
        genome_ids: &mut GenomeIds,
        organism_ids: &mut OrganismIds,
    ) -> Species {
        // every founder draws its own policy from this species' brain stream.
        // a single shared base brain was tried first and is wrong: one unlucky
        // draw then makes a whole species inert before selection can act on it,
        // which decides runs by luck instead of by fitness. independent draws
        // hand selection a spread of strategies on tick one.
        let organisms: Vec<Organism> = (0..count)
            .map(|_| {
                let genes = blueprint.genes.varied(blueprint.gene_spread, streams.morphology);
                let brain = NeuralGenome::random(streams.brains);
                let genome = Genome::founder(genome_ids.mint(), genes, brain);
                // founders are placed on land only. there is no rule that
                // could rescue an organism spawned in the sea: it cannot move
                // out and there is nothing there to eat.
                let (x, y) = world.random_land(streams.morphology);
                Organism::new(organism_ids.mint(), genome, x, y, START_ENERGY)
            })
            .collect();

        Species {
            id,
            name: blueprint.name.clone(),
            founder_genes: blueprint.genes,
            founder_brain: NeuralGenome::centroid(organisms.iter().map(|o| o.brain())),
            population: Population::new(organisms),
        }
    }

    pub fn id(&self) -> SpeciesId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// the profile this species was founded from, for reporting genetic drift
    pub fn founder_genes(&self) -> &Genes {
        &self.founder_genes
    }

    /// where this species' founding policies sat, taken as one point. drift is
    /// measured against it: as selection concentrates the population on the
    /// descendants of whichever founders worked, the centroid moves away from
    /// here. never read back into the simulation.
    pub fn founder_brain(&self) -> &NeuralGenome {
        &self.founder_brain
    }

    pub fn population(&self) -> &Population {
        &self.population
    }

    pub fn population_mut(&mut self) -> &mut Population {
        &mut self.population
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blueprint() -> SpeciesBlueprint {
        SpeciesBlueprint {
            name: "Test".into(),
            genes: Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 },
            gene_spread: 0.1,
        }
    }

    #[test]
    fn founders_are_unique_parentless_and_owned_by_the_species() {
        let world = World::generate(1234, 32, 32);
        let (mut rng, mut brains) = (Rng::new(9), Rng::new(90));
        let (mut gids, mut oids) = (GenomeIds::default(), OrganismIds::default());
        let s = Species::found(
            SpeciesId::new(0),
            &blueprint(),
            50,
            &world,
            &mut FounderStreams { morphology: &mut rng, brains: &mut brains },
            &mut gids,
            &mut oids,
        );

        assert_eq!(s.population().len(), 50);
        let mut genomes: Vec<u64> =
            s.population().organisms().iter().map(|o| o.genome().id().get()).collect();
        let mut organisms: Vec<u64> =
            s.population().organisms().iter().map(|o| o.id().get()).collect();
        genomes.sort_unstable();
        genomes.dedup();
        organisms.sort_unstable();
        organisms.dedup();
        assert_eq!(genomes.len(), 50);
        assert_eq!(organisms.len(), 50);

        assert!(s.population().organisms().iter().all(|o| o.genome().is_founder()));
        assert!(s.population().organisms().iter().all(|o| o.genes().in_bounds()));
        assert!(s.population().organisms().iter().all(|o| o.brain().in_bounds()));
        assert!(
            s.population().organisms().iter().all(|o| world.is_passable(world.idx(o.x, o.y))),
            "a founder was spawned in the sea"
        );
    }

    /// no two founders share a policy, so selection has a spread of strategies
    /// to act on from the first tick
    #[test]
    fn founder_brains_are_all_different_and_centre_on_the_reported_founder() {
        let world = World::generate(1234, 32, 32);
        let (mut rng, mut brains) = (Rng::new(9), Rng::new(90));
        let (mut gids, mut oids) = (GenomeIds::default(), OrganismIds::default());
        let s = Species::found(
            SpeciesId::new(0),
            &blueprint(),
            200,
            &world,
            &mut FounderStreams { morphology: &mut rng, brains: &mut brains },
            &mut gids,
            &mut oids,
        );
        let organisms = s.population().organisms();
        assert_ne!(organisms[0].brain(), organisms[1].brain());
        assert!(organisms.iter().all(|o| o.brain().in_bounds()));

        let centroid = NeuralGenome::centroid(organisms.iter().map(|o| o.brain()));
        assert_eq!(&centroid, s.founder_brain());
        // 200 independent draws from -1..1 average near zero on every gene
        assert!(centroid.distance(&NeuralGenome::default()) < 0.15);
    }

    /// two species founded from the same physical blueprint but different brain
    /// streams must not share a policy - that is the controlled experiment
    #[test]
    fn a_different_brain_stream_founds_a_different_policy() {
        let world = World::generate(1234, 32, 32);
        let found = |brain_seed| {
            let (mut rng, mut brains) = (Rng::new(9), Rng::new(brain_seed));
            let (mut gids, mut oids) = (GenomeIds::default(), OrganismIds::default());
            Species::found(
                SpeciesId::new(0),
                &blueprint(),
                5,
                &world,
                &mut FounderStreams { morphology: &mut rng, brains: &mut brains },
                &mut gids,
                &mut oids,
            )
        };
        let a = found(90);
        let b = found(91);
        assert_eq!(a.founder_brain(), found(90).founder_brain());
        assert_ne!(a.founder_brain(), b.founder_brain());
        assert_ne!(a.population().organisms()[0].brain(), b.population().organisms()[0].brain());
    }

    #[test]
    fn the_same_stream_founds_the_same_population() {
        let world = World::generate(1234, 32, 32);
        let found = |seed| {
            let (mut rng, mut brains) = (Rng::new(seed), Rng::new(seed ^ 0xB4));
            let (mut g, mut o) = (GenomeIds::default(), OrganismIds::default());
            let s = Species::found(
                SpeciesId::new(0),
                &blueprint(),
                20,
                &world,
                &mut FounderStreams { morphology: &mut rng, brains: &mut brains },
                &mut g,
                &mut o,
            );
            s.population()
                .organisms()
                .iter()
                .map(|o| (*o.genes(), *o.brain(), o.x, o.y))
                .collect::<Vec<_>>()
        };
        assert_eq!(found(9), found(9));
        assert_ne!(found(9), found(10));
    }
}
