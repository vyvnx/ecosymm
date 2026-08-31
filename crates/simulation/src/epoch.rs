//! simulation time and the backend-neutral execution contract.

use crate::state::SimulationState;
use ecosym_ecology::BehaviorTally;
use serde::{Deserialize, Serialize};
use std::fmt;

/// a batch of simulation ticks. this is simulation time, not a biological
/// generation: parents and descendants coexist continuously, so genealogical
/// depth has to be derived from `GenomeId` ancestry instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Epoch(pub usize);

/// one atomic state-advancement step
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tick(pub usize);

/// where the run is on the simulation clock, and nothing more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationTime {
    pub epoch: Epoch,
    pub tick: Tick,
}

/// what an engine produced over one epoch, indexed by the state's species
/// order. a `Vec`, not a map: stable order is part of replay determinism.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EpochEvents {
    pub births: Vec<usize>,
    pub deaths: Vec<usize>,
    /// what each species' policies actually did over the epoch. descriptive
    /// telemetry: the runner turns it into a report and nothing reads it back.
    pub behavior: Vec<BehaviorTally>,
    /// the hard population ceiling refused at least one birth this epoch. that
    /// is allocation protection binding, not carrying capacity.
    pub ceiling_bound: bool,
}

impl EpochEvents {
    pub fn for_species(n: usize) -> EpochEvents {
        EpochEvents {
            births: vec![0; n],
            deaths: vec![0; n],
            behavior: vec![BehaviorTally::default(); n],
            ceiling_bound: false,
        }
    }

    pub fn total_births(&self) -> usize {
        self.births.iter().sum()
    }

    pub fn total_deaths(&self) -> usize {
        self.deaths.iter().sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// the backend itself failed: device lost, kernel launch refused, transfer
    /// error. the cpu engine never produces one.
    Backend(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Backend(msg) => write!(f, "engine backend failure: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// the backend-neutral execution seam. cpu today; wgpu or cuda later without
/// touching genetics, ecology, world, the runner, replay or the reports.
///
/// this is a cpu-era contract. when a gpu-resident engine lands, the physical
/// ownership and synchronisation model here may change - `EpochReport`,
/// determinism requirements and application behaviour must not.
///
/// `Simulation` boxes this as `dyn EpochEngine + Send` so a run can sit on an
/// async task. an engine holding a `!Send` device handle has to own its own
/// thread; revisit the bound when one actually does.
/// # The tick contract
///
/// A backend owns **scheduling, storage, parallelism and its own random
/// streams**. It owns no rules. Everything numbered below that says "apply"
/// resolves to one function in `ecosym-ecology`, and a backend that reimplements
/// one has forked the model.
///
/// For each of the `ticks` ticks:
///
/// 1. snapshot each species' living population, so newborns can neither act nor
///    mate in their birth tick;
/// 2. visit every snapshot organism exactly once, in a permutation derived from
///    (seed, epoch, tick) on a stream independent of the behaviour and
///    reproduction streams;
/// 3. rebuild the per-cell spatial index every organism observes, with
///    [`ecosym_ecology::CellIndex::rebuild`];
/// 4. apply [`ecosym_ecology::behavior::live_one_tick`] - observe, run the
///    inherited policy against **this organism's own** hidden state, move,
///    forage the shared field, pay upkeep, age. the returned organism carries
///    the advanced memory and is what must be stored back; a shared or
///    per-thread scratch buffer forks the model. **buffer** the policy's
///    reproductive pressure rather than acting on it;
/// 5. rebuild the index a second time, now that everybody has moved. every
///    breeder in the tick must query this one snapshot: no breeder may see
///    moved positions while another sees the old ones;
/// 6. in the same visit order, apply
///    [`ecosym_ecology::behavior::conceive`] with the buffered pressure -
///    eligibility, a mate from within [`ecosym_ecology::behavior::MATE_REACH`]
///    of where the parent now stands, recombination then birth-time mutation of
///    genes and brain alike;
/// 7. after the pass, admit conceptions with
///    [`ecosym_ecology::Conception::birth`], subject to the backend's own
///    population ceiling, applied fairly across species. a newborn's hidden
///    state is zero: weights are inherited, memory is not;
/// 8. remove the dead with `Population::retain_living`, which is stable;
/// 9. regrow the world; and
/// 10. accumulate births, deaths and behavioural tallies per species.
///
/// A backend that cannot call Rust at all - a shader, a kernel - still has
/// exactly one place to port each rule *from*, and `CpuEngine` is the reference
/// its port is diffed against. `conformance::verify_engine` is the gate.
pub trait EpochEngine {
    fn id(&self) -> &'static str;

    /// advance exactly one epoch of `ticks` ticks, leaving the clock's tick
    /// counter at `ticks`. the runner owns the epoch counter.
    fn advance_epoch(
        &mut self,
        state: &mut SimulationState,
        ticks: usize,
    ) -> Result<EpochEvents, EngineError>;
}
