//! one individual: a mutable body around an immutable genome.

use ecosym_genetics::{Genes, Genome, NeuralGenome, HIDDEN};

/// stable identity for one organism. never reused inside a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrganismId(u64);

impl OrganismId {
    pub fn get(self) -> u64 {
        self.0
    }
}

/// the only way to mint an `OrganismId`.
#[derive(Clone, Debug, Default)]
pub struct OrganismIds(u64);

impl OrganismIds {
    pub fn mint(&mut self) -> OrganismId {
        let id = OrganismId(self.0);
        self.0 += 1;
        id
    }

    pub fn issued(&self) -> u64 {
        self.0
    }
}

/// position, energy and age change every tick. `id` and `genome` never do -
/// they are private precisely so no tick can reach in and mutate them.
///
/// there is no `SpeciesId` here: membership is established by the owning
/// population, so it cannot drift out of sync.
#[derive(Clone, Copy, Debug)]
pub struct Organism {
    pub x: f32,
    pub y: f32,
    pub energy: f32,
    pub age: u32,
    /// how far it actually travelled last tick. reported, not observed.
    pub last_move: f32,
    /// the policy's working memory: last tick's hidden activations, fed back
    /// into this tick's.
    ///
    /// state, not genome. it starts at zero, persists across ticks and epochs
    /// until the body dies, and is **never inherited** - a newborn has no
    /// memory of places it has never been. that is the whole difference between
    /// a lifetime and a lineage, and putting it in `Genome` would erase it.
    pub hidden: [f32; HIDDEN],
    id: OrganismId,
    genome: Genome,
}

impl Organism {
    pub fn new(id: OrganismId, genome: Genome, x: f32, y: f32, energy: f32) -> Organism {
        Organism { x, y, energy, age: 0, last_move: 0.0, hidden: [0.0; HIDDEN], id, genome }
    }

    pub fn id(&self) -> OrganismId {
        self.id
    }

    pub fn genome(&self) -> &Genome {
        &self.genome
    }

    pub fn genes(&self) -> &Genes {
        self.genome.genes()
    }

    pub fn brain(&self) -> &NeuralGenome {
        self.genome.brain()
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.energy.is_finite()
            && self.last_move.is_finite()
            && self.hidden.iter().all(|h| h.is_finite())
            && self.genes().is_finite()
            && self.brain().is_finite()
    }
}
