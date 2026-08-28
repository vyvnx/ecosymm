//! genome identity, ancestry and the genes themselves.

use crate::NeuralGenome;
use ecosym_core::Rng;
use serde::{Deserialize, Serialize};

/// viable range per trait. mutation and founder variation both clamp here.
pub const SPEED_BOUNDS: (f32, f32) = (0.1, 3.0);
pub const SIZE_BOUNDS: (f32, f32) = (0.2, 3.0);
pub const METABOLISM_BOUNDS: (f32, f32) = (0.2, 2.5);
pub const HEAT_PREF_BOUNDS: (f32, f32) = (0.0, 1.0);

/// stable identity for one genome value. unique for the life of a run and
/// never reused, so ancestry stays resolvable after the organism dies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GenomeId(u64);

impl GenomeId {
    pub fn get(self) -> u64 {
        self.0
    }
}

/// the only way to mint a `GenomeId`. uniqueness is a property of the
/// allocator, not of caller discipline.
#[derive(Clone, Debug, Default)]
pub struct GenomeIds(u64);

impl GenomeIds {
    pub fn mint(&mut self) -> GenomeId {
        let id = GenomeId(self.0);
        self.0 += 1;
        id
    }

    /// how many ids have been handed out so far
    pub fn issued(&self) -> u64 {
        self.0
    }
}

/// four traits, all under selection pressure at once:
/// speed costs energy, size buys lifespan, metabolism trades intake for burn,
/// heat_pref decides which latitudes are cheap to live in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Genes {
    pub speed: f32,
    pub size: f32,
    pub metabolism: f32,
    pub heat_pref: f32,
}

impl Genes {
    pub fn clamped(self) -> Genes {
        Genes {
            speed: clamp(self.speed, SPEED_BOUNDS),
            size: clamp(self.size, SIZE_BOUNDS),
            metabolism: clamp(self.metabolism, METABOLISM_BOUNDS),
            heat_pref: clamp(self.heat_pref, HEAT_PREF_BOUNDS),
        }
    }

    pub fn in_bounds(&self) -> bool {
        within(self.speed, SPEED_BOUNDS)
            && within(self.size, SIZE_BOUNDS)
            && within(self.metabolism, METABOLISM_BOUNDS)
            && within(self.heat_pref, HEAT_PREF_BOUNDS)
    }

    pub fn is_finite(&self) -> bool {
        self.speed.is_finite()
            && self.size.is_finite()
            && self.metabolism.is_finite()
            && self.heat_pref.is_finite()
    }

    /// small bounded variation around a founder profile, so 500 founders of one
    /// species are siblings rather than clones. uniform, not gaussian: the
    /// spread must be a hard bound, not a tail.
    pub fn varied(&self, spread: f32, rng: &mut Rng) -> Genes {
        Genes {
            speed: self.speed + rng.between(-spread, spread),
            size: self.size + rng.between(-spread, spread),
            metabolism: self.metabolism + rng.between(-spread, spread),
            // heat_pref lives on a 0..1 scale, so it drifts at half the width
            heat_pref: self.heat_pref + rng.between(-spread * 0.5, spread * 0.5),
        }
        .clamped()
    }
}

/// an organism's genome: identity, both genetic parents, the physical genes and
/// the behavioural ones.
///
/// immutable by construction. genetic change happens only when a new offspring
/// genome is built; nothing may edit the genome a living organism holds - and
/// that now covers the brain, so there is no lifetime learning by the back door.
///
/// not `Serialize`: `brain` is a 117-float fixed array and serde stops deriving
/// at 32. nothing has ever put a `Genome` on the wire - reports carry `Genes`
/// and behavioural means - so the derive was dropped rather than worked around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Genome {
    id: GenomeId,
    parent_ids: [Option<GenomeId>; 2],
    genes: Genes,
    brain: NeuralGenome,
}

impl Genome {
    /// a parentless founder
    pub fn founder(id: GenomeId, genes: Genes, brain: NeuralGenome) -> Genome {
        Genome { id, parent_ids: [None, None], genes, brain }
    }

    /// an offspring, recording both genetic parents
    pub fn offspring(
        id: GenomeId,
        parents: [GenomeId; 2],
        genes: Genes,
        brain: NeuralGenome,
    ) -> Genome {
        Genome { id, parent_ids: [Some(parents[0]), Some(parents[1])], genes, brain }
    }

    pub fn id(&self) -> GenomeId {
        self.id
    }

    pub fn parent_ids(&self) -> [Option<GenomeId>; 2] {
        self.parent_ids
    }

    pub fn genes(&self) -> &Genes {
        &self.genes
    }

    pub fn brain(&self) -> &NeuralGenome {
        &self.brain
    }

    pub fn is_founder(&self) -> bool {
        self.parent_ids == [None, None]
    }
}

fn clamp(v: f32, (lo, hi): (f32, f32)) -> f32 {
    v.max(lo).min(hi)
}

fn within(v: f32, (lo, hi): (f32, f32)) -> bool {
    (lo..=hi).contains(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_never_reused() {
        let mut ids = GenomeIds::default();
        let issued: Vec<u64> = (0..1000).map(|_| ids.mint().get()).collect();
        let mut sorted = issued.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), issued.len());
        assert_eq!(ids.issued(), 1000);
    }

    #[test]
    fn founder_ancestry_is_empty_offspring_ancestry_is_both_parents() {
        let mut ids = GenomeIds::default();
        let a = Genome::founder(ids.mint(), Genes::default(), NeuralGenome::default());
        let b = Genome::founder(ids.mint(), Genes::default(), NeuralGenome::default());
        assert!(a.is_founder());
        assert_eq!(a.parent_ids(), [None, None]);

        let c = Genome::offspring(
            ids.mint(),
            [a.id(), b.id()],
            Genes::default(),
            NeuralGenome::default(),
        );
        assert!(!c.is_founder());
        assert_eq!(c.parent_ids(), [Some(a.id()), Some(b.id())]);
        assert_ne!(c.id(), a.id());
        assert_ne!(c.id(), b.id());
    }

    #[test]
    fn founder_variation_is_bounded_and_not_a_clone() {
        let mut rng = Rng::new(11);
        let base = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let varied: Vec<Genes> = (0..200).map(|_| base.varied(0.15, &mut rng)).collect();
        assert!(varied.iter().all(|g| g.in_bounds()));
        assert!(varied.iter().all(|g| (g.speed - base.speed).abs() <= 0.15));
        assert!(varied.iter().any(|g| *g != base), "every founder is a clone");
    }
}
