//! the organisms one species owns.

use crate::{behavior, Organism};

/// every organism in here belongs to the owning species, which is why a mate
/// lookup takes one of these rather than the global organism list: breeding
/// across species is not something a caller can do by accident.
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
