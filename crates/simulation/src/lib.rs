//! what happens over time: simulation state, the backend-neutral epoch
//! contract, the cpu engine that implements it, and the runner on top.
//!
//! the backend seam lives here, not in `ecosym-gpu`: cpu, wgpu and cuda are all
//! execution backends and none of them should define the contract the others
//! have to honour. this crate must never depend on `ecosym-gpu`, wgpu or cuda.

pub mod conformance;
pub mod cpu;
pub mod epoch;
pub mod run;
pub mod state;
pub mod statistics;

pub use cpu::CpuEngine;
pub use ecosym_ecology::BehaviorStats;
pub use epoch::{EngineError, Epoch, EpochEngine, EpochEvents, SimulationTime, Tick};
pub use run::{default_blueprints, twin_blueprints, Simulation};
pub use state::SimulationState;
pub use statistics::{EpochReport, RunOutcome, SpeciesResult, SpeciesStats, Winner};
