//! what genes an organism carries, and how they are inherited.
//!
//! genetics answers "what is inherited". what those genes *cost* in a given
//! world is `ecosym-ecology::phenotype`, and what an organism does with its
//! brain's outputs is `ecosym-ecology::behavior`, not here.

mod genome;
mod mutation;
mod neural_genome;
mod recombination;

pub use genome::{
    Genes, Genome, GenomeId, GenomeIds, HEAT_PREF_BOUNDS, METABOLISM_BOUNDS, SIZE_BOUNDS,
    SPEED_BOUNDS,
};
pub use mutation::{
    mutate, mutate_brain, BRAIN_BIG_SHARE, BRAIN_BIG_STEP, BRAIN_MUTATION_RATE, BRAIN_SMALL_STEP,
    MUTATION_RATE,
};
pub use neural_genome::{
    NeuralGenome, BIAS_COUNT, FOUNDER_WEIGHT, HIDDEN, INPUTS, OUTPUTS, WEIGHT_BOUNDS, WEIGHT_COUNT,
};
pub use recombination::{recombine, recombine_brain};
