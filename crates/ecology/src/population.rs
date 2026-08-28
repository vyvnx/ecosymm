//! the organisms one species owns.

use crate::{behavior, Organism};
use ecosym_core::Rng;

/// every organism in here belongs to the owning species. mate selection is a
/// method on the population precisely so a caller cannot hand it the global
/// organism list and breed across species by accident.
#[derive(Clone, Debug, Default)]
pub struct Population {
    organisms: Vec<Organism>,
}

impl Population {
    pub fn new(organisms: Vec<Organism>) -> Population {
        Population { organisms }
    }

    pub fn organisms(&self) -> &[Organism] {
        &self.organisms
    }

    pub fn get(&self, i: usize) -> Option<&Organism> {
        self.organisms.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut Organism> {
        self.organisms.get_mut(i)
    }

    pub fn len(&self) -> usize {
        self.organisms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.organisms.is_empty()
    }

    /// a mate drawn from this population and no other, skipping `breeder`'s own
    /// index whenever somebody else is available.
    pub fn select_mate(&self, breeder: usize, rng: &mut Rng) -> Option<&Organism> {
        match self.organisms.len() {
            0 => None,
            1 => self.organisms.first(),
            n => {
                let mut j = rng.below(n - 1);
                if j >= breeder {
                    j += 1;
                }
                self.organisms.get(j)
            }
        }
    }

    /// newborns are appended after a visit pass, never during it
    pub fn push(&mut self, organism: Organism) {
        self.organisms.push(organism);
    }

    /// stable retention. returns how many died.
    pub fn retain_living(&mut self) -> usize {
        let before = self.organisms.len();
        self.organisms.retain(|o| !behavior::is_dead(o));
        before - self.organisms.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrganismIds;
    use ecosym_genetics::{Genes, Genome, GenomeIds, NeuralGenome};

    fn population(n: usize) -> Population {
        let mut gids = GenomeIds::default();
        let mut oids = OrganismIds::default();
        Population::new(
            (0..n)
                .map(|i| {
                    let genes = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
                    let genome = Genome::founder(gids.mint(), genes, NeuralGenome::default());
                    Organism::new(oids.mint(), genome, i as f32, 0.0, 5.0)
                })
                .collect(),
        )
    }

    #[test]
    fn mate_selection_never_leaves_the_population_and_skips_self() {
        let p = population(6);
        let mut rng = Rng::new(5);
        for _ in 0..200 {
            let mate = p.select_mate(2, &mut rng).unwrap();
            assert_ne!(mate.id(), p.get(2).unwrap().id());
            assert!(p.organisms().iter().any(|o| o.id() == mate.id()));
        }
    }

    #[test]
    fn a_lone_organism_can_only_mate_with_itself_and_an_empty_one_cannot() {
        let mut rng = Rng::new(5);
        assert!(population(1).select_mate(0, &mut rng).is_some());
        assert!(population(0).select_mate(0, &mut rng).is_none());
    }

    #[test]
    fn retention_is_stable_and_counts_the_dead() {
        let mut p = population(5);
        p.get_mut(1).unwrap().energy = -1.0;
        p.get_mut(3).unwrap().energy = 0.0;
        let survivors: Vec<u64> =
            [0usize, 2, 4].iter().map(|&i| p.get(i).unwrap().id().get()).collect();
        assert_eq!(p.retain_living(), 2);
        assert_eq!(p.organisms().iter().map(|o| o.id().get()).collect::<Vec<_>>(), survivors);
    }
}
