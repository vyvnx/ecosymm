//! what genes cause in an environment: organisms, species, populations, and
//! the rules that turn genes plus surroundings into behaviour.
//!
//! ```text
//! genetics            what genes does the organism carry?
//! ecology/phenotype   what do those genes cause in this environment?
//! world               what environment does it experience?
//! ```

pub mod behavior;
pub mod interactions;
pub mod phenotype;
pub mod spatial;

mod organism;
mod population;
mod species;

pub use behavior::{Act, BehaviorStats, BehaviorTally, Conception, Intent, MateSearch};
pub use organism::{Organism, OrganismId, OrganismIds};
pub use population::Population;
pub use spatial::CellIndex;
pub use species::{FounderStreams, Species, SpeciesBlueprint, SpeciesId, START_ENERGY};
