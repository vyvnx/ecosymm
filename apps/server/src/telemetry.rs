//! what the run looked like from outside it, as a bounded stream of events.
//!
//! this is presentation. it consumes finished `EpochReport` values and produces
//! text a spectator can read; nothing here reaches the simulation, the market,
//! the odds or the replay digest, and there is no path by which it could - the
//! only thing it is handed is a report that has already happened.
//!
//! the whole of its state is fixed-capacity. one detector per channel, one
//! bounded event ring, both sized at construction, so a server that runs
//! forever holds the same bytes on day 30 as on day 1.
//!
//! ```text
//! start(run_id, species)  -> every measurement, detector and ring reset
//! push(report)            -> zero or more events, deterministically ordered
//! finish(outcome)         -> the terminal result event, and only here
//! ```

use ecosym_genetics::{METABOLISM_BOUNDS, SIZE_BOUNDS, SPEED_BOUNDS};
use ecosym_simulation::{EpochReport, RunOutcome, Winner};
use serde::Serialize;
use std::collections::VecDeque;

/// bumped whenever a threshold, factor or template changes. it ships with
/// every event, because "Species A is shifting toward lower metabolism" is only
/// meaningful next to the rules that decided it.
pub const DETECTOR_VERSION: u32 = 1;

/// the feed a reconnecting viewer is given. a run is 500 epochs and the
/// detectors are deliberately quiet, so this is the whole of a normal run.
pub const EVENT_CAPACITY: usize = 64;

/// recent behaviour and baseline behaviour, as smoothing factors. fast is about
/// a six-epoch memory, slow about forty - the gap between them is the signal.
const FAST: f32 = 0.30;
const SLOW: f32 = 0.05;

/// epochs the gap has to hold before anything is emitted, and epochs before the
/// same channel may speak again. together they are what stops a noisy series
/// producing a wall of text.
const DWELL: u32 = 3;
const COOLDOWN: usize = 24;

/// a population this far below where it started is in trouble, and this far
/// back up has recovered. two thresholds, so the pair cannot oscillate.
const SCARCE: f32 = 0.10;
const RECOVERED: f32 = 0.25;

/// one thing that happened, ready for the wire.
///
/// `evidence` is not optional. an event that cannot show its working is
/// narration, and narration from an omniscient character is exactly what a
/// spectator cannot check.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    pub run_id: i64,
    pub event_id: u64,
    pub epoch: usize,
    pub kind: &'static str,
    pub severity: &'static str,
    pub species_id: Option<u32>,
    pub title: String,
    pub evidence: String,
    pub detector_version: u32,
}

/// severity ranks before kind in the emission order, so the loudest thing that
/// happened in an epoch is the first thing read
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Major,
    Notable,
    Info,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Major => "major",
            Severity::Notable => "notable",
            Severity::Info => "info",
        }
    }
}

/// what a detector watches. the discriminant is the sort key inside one kind,
/// so this order is part of the wire contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Feature {
    Population,
    Births,
    Deaths,
    Biomass,
    Energy,
    Movement,
    ResourceTracking,
    Breeding,
    Resting,
    Exposure,
    Metabolism,
    Speed,
    HeatPref,
}

/// every channel, in the fixed order they are built and sorted in
const FEATURES: [Feature; 13] = [
    Feature::Population,
    Feature::Births,
    Feature::Deaths,
    Feature::Biomass,
    Feature::Energy,
    Feature::Movement,
    Feature::ResourceTracking,
    Feature::Breeding,
    Feature::Resting,
    Feature::Exposure,
    Feature::Metabolism,
    Feature::Speed,
    Feature::HeatPref,
];

impl Feature {
    fn label(self) -> &'static str {
        match self {
            Feature::Population => "population",
            Feature::Births => "birth rate",
            Feature::Deaths => "death rate",
            Feature::Biomass => "standing biomass",
            Feature::Energy => "energy reserves",
            Feature::Movement => "movement",
            Feature::ResourceTracking => "resource tracking",
            Feature::Breeding => "breeding pressure",
            Feature::Resting => "resting",
            Feature::Exposure => "competitor exposure",
            Feature::Metabolism => "metabolism",
            Feature::Speed => "speed",
            Feature::HeatPref => "heat preference",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Feature::Population | Feature::Births | Feature::Deaths => "population_trend",
            Feature::Biomass => "world_trend",
            Feature::Energy
            | Feature::Movement
            | Feature::ResourceTracking
            | Feature::Breeding
            | Feature::Resting
            | Feature::Exposure => "strategy_shift",
            Feature::Metabolism | Feature::Speed | Feature::HeatPref => "trait_drift",
        }
    }

    /// entry threshold, release threshold, and the scale guard that keeps a
    /// relative divergence finite when the baseline is near zero.
    ///
    /// these are hypotheses calibrated against recorded runs, not knobs to be
    /// tuned until the feed reads nicely - `DETECTOR_VERSION` moves when they
    /// do, so an old event can still be read against the rules that made it.
    fn thresholds(self) -> (f32, f32, f32) {
        match self {
            Feature::Population => (0.25, 0.10, 10.0),
            Feature::Births | Feature::Deaths => (0.35, 0.15, 5.0),
            Feature::Biomass => (0.25, 0.10, 50.0),
            Feature::Energy => (0.20, 0.08, 0.5),
            Feature::Movement | Feature::ResourceTracking | Feature::Breeding => (0.15, 0.06, 0.05),
            Feature::Resting => (0.20, 0.08, 0.05),
            Feature::Exposure => (0.40, 0.15, 0.01),
            Feature::Metabolism | Feature::Speed | Feature::HeatPref => (0.10, 0.04, 0.05),
        }
    }

    /// how a change in this feature reads in a title. traits drift, strategies
    /// shift, head counts climb and fall.
    fn phrase(self, rising: bool) -> &'static str {
        match (self.kind(), rising) {
            ("trait_drift", true) => "is drifting toward higher",
            ("trait_drift", false) => "is drifting toward lower",
            ("strategy_shift", true) => "is shifting toward more",
            ("strategy_shift", false) => "is shifting toward less",
            (_, true) => "is climbing on",
            (_, false) => "is falling on",
        }
    }
}

/// one channel's memory: two exponentially weighted means and the hysteresis
/// that stops the gap between them chattering.
#[derive(Clone, Debug)]
struct Detector {
    feature: Feature,
    species: Option<u32>,
    primed: bool,
    fast: f32,
    slow: f32,
    active: bool,
    dwell: u32,
    quiet_until: usize,
}

/// what the gap looked like when it crossed
struct Crossing {
    divergence: f32,
    fast: f32,
    slow: f32,
}

impl Detector {
    fn new(feature: Feature, species: Option<u32>) -> Detector {
        Detector {
            feature,
            species,
            primed: false,
            fast: 0.0,
            slow: 0.0,
            active: false,
            dwell: 0,
            quiet_until: 0,
        }
    }

    fn update(&mut self, value: f32, epoch: usize) -> Option<Crossing> {
        if !value.is_finite() {
            return None;
        }
        if !self.primed {
            self.primed = true;
            self.fast = value;
            self.slow = value;
            return None;
        }
        self.fast += FAST * (value - self.fast);
        self.slow += SLOW * (value - self.slow);

        let (high, release, floor) = self.feature.thresholds();
        let scale = self.slow.abs().max(floor);
        let divergence = (self.fast - self.slow) / scale;

        // active is the released half of the hysteresis: nothing new is said
        // until the gap has closed again
        if self.active {
            if divergence.abs() < release {
                self.active = false;
                self.dwell = 0;
            }
            return None;
        }
        if divergence.abs() < high {
            self.dwell = 0;
            return None;
        }
        self.dwell += 1;
        if self.dwell < DWELL || epoch < self.quiet_until {
            return None;
        }
        self.active = true;
        self.dwell = 0;
        self.quiet_until = epoch + COOLDOWN;
        Some(Crossing { divergence, fast: self.fast, slow: self.slow })
    }
}

/// an event before it has an id. ordering happens over these, and only then is
/// a run-local counter spent - so the id sequence is a property of the run and
/// never of arrival time.
struct Pending {
    severity: Severity,
    kind: &'static str,
    kind_id: u16,
    subtype_id: u16,
    species_id: Option<u32>,
    title: String,
    evidence: String,
}

pub struct Telemetry {
    run_id: i64,
    names: Vec<String>,
    initial: Vec<f32>,
    detectors: Vec<Detector>,
    events: VecDeque<Event>,
    next_event_id: u64,
    born: Vec<bool>,
    extinct: Vec<bool>,
    scarce: Vec<bool>,
    leader: Option<u32>,
    lead_dwell: u32,
    finished: bool,
}

impl Telemetry {
    /// a fresh run. every measurement, detector, cooldown and ring starts here
    /// and nothing survives from the last one - which is why this takes the run
    /// it is for rather than being reset in place.
    pub fn start(run_id: i64, names: &[String]) -> Telemetry {
        let n = names.len();
        let mut detectors = Vec::with_capacity((n + 1) * FEATURES.len());
        for s in 0..n {
            for f in FEATURES {
                if f != Feature::Biomass {
                    detectors.push(Detector::new(f, Some(s as u32)));
                }
            }
        }
        detectors.push(Detector::new(Feature::Biomass, None));

        Telemetry {
            run_id,
            names: names.to_vec(),
            initial: vec![0.0; n],
            detectors,
            events: VecDeque::with_capacity(EVENT_CAPACITY),
            next_event_id: 0,
            born: vec![false; n],
            extinct: vec![false; n],
            scarce: vec![false; n],
            leader: None,
            lead_dwell: 0,
            finished: false,
        }
    }

    /// the retained feed, oldest first
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }

    /// one completed epoch. returns the events it produced, already ordered and
    /// already folded into the ring.
    pub fn push(&mut self, report: &EpochReport) -> Vec<Event> {
        let mut pending = Vec::new();
        if self.initial.iter().all(|v| *v == 0.0) {
            for (i, s) in report.species.iter().enumerate() {
                if let Some(slot) = self.initial.get_mut(i) {
                    *slot = s.population as f32;
                }
            }
        }

        self.exact_transitions(report, &mut pending);
        self.lead_change(report, &mut pending);
        self.trends(report, &mut pending);
        self.emit(report.epoch, pending)
    }

    /// the terminal result, and the only place it comes from. a run that ended
    /// is a fact about the run, not a threshold anyone crossed.
    pub fn finish(&mut self, epoch: usize, outcome: &RunOutcome) -> Vec<Event> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let title = match &outcome.winner {
            Winner::None => "the run ended with nothing alive".to_string(),
            Winner::Species(id) => format!("{} took the run", self.name(*id)),
            Winner::Tie(ids) => {
                let names: Vec<String> = ids.iter().map(|id| self.name(*id)).collect();
                format!("{} finished level", names.join(" and "))
            }
        };
        let evidence = outcome
            .species
            .iter()
            .map(|s| format!("{} {} -> {}", s.name, s.initial, s.final_population))
            .collect::<Vec<_>>()
            .join(", ");
        let pending = vec![Pending {
            severity: Severity::Major,
            kind: "result",
            kind_id: 0,
            subtype_id: 0,
            species_id: None,
            title,
            evidence,
        }];
        self.emit(epoch, pending)
    }

    fn name(&self, id: u32) -> String {
        self.names.get(id as usize).cloned().unwrap_or_else(|| format!("species {id}"))
    }

    /// state transitions that are facts rather than trends: a first birth, an
    /// extinction, a population falling to scarcity and climbing back out
    fn exact_transitions(&mut self, report: &EpochReport, pending: &mut Vec<Pending>) {
        for (i, s) in report.species.iter().enumerate() {
            let name = self.name(s.id);
            if !self.born[i] && s.births > 0 {
                self.born[i] = true;
                pending.push(Pending {
                    severity: Severity::Info,
                    kind: "first_birth",
                    kind_id: 2,
                    subtype_id: 0,
                    species_id: Some(s.id),
                    title: format!("{name} bred for the first time"),
                    evidence: format!("{} births in epoch {}", s.births, report.epoch),
                });
            }
            if !self.extinct[i] && s.population == 0 {
                self.extinct[i] = true;
                pending.push(Pending {
                    severity: Severity::Major,
                    kind: "extinction",
                    kind_id: 1,
                    subtype_id: 0,
                    species_id: Some(s.id),
                    title: format!("{name} is extinct"),
                    evidence: format!(
                        "{} founders, {} births and {} deaths over the run so far",
                        self.initial[i] as i64, s.births, s.deaths
                    ),
                });
                continue;
            }
            if s.population == 0 {
                continue;
            }

            let share = s.population as f32 / self.initial[i].max(1.0);
            if !self.scarce[i] && share < SCARCE {
                self.scarce[i] = true;
                pending.push(Pending {
                    severity: Severity::Major,
                    kind: "near_extinction",
                    kind_id: 3,
                    subtype_id: 0,
                    species_id: Some(s.id),
                    title: format!("{name} is close to gone"),
                    evidence: format!(
                        "{} left of {} founders, {:.0}%",
                        s.population,
                        self.initial[i] as i64,
                        share * 100.0
                    ),
                });
            } else if self.scarce[i] && share > RECOVERED {
                self.scarce[i] = false;
                pending.push(Pending {
                    severity: Severity::Notable,
                    kind: "recovery",
                    kind_id: 4,
                    subtype_id: 0,
                    species_id: Some(s.id),
                    title: format!("{name} is back from the edge"),
                    evidence: format!(
                        "{} alive, {:.0}% of its founding population",
                        s.population,
                        share * 100.0
                    ),
                });
            }
        }
    }

    /// who is ahead on head count, once it has stayed that way long enough to
    /// mean something
    fn lead_change(&mut self, report: &EpochReport, pending: &mut Vec<Pending>) {
        let Some(top) = report
            .species
            .iter()
            .filter(|s| s.population > 0)
            .max_by_key(|s| (s.population, std::cmp::Reverse(s.id)))
        else {
            return;
        };
        if Some(top.id) == self.leader {
            self.lead_dwell = 0;
            return;
        }
        self.lead_dwell += 1;
        if self.lead_dwell < DWELL {
            return;
        }
        let previous = self.leader;
        self.leader = Some(top.id);
        self.lead_dwell = 0;
        if previous.is_none() {
            return;
        }
        let behind: Vec<String> = report
            .species
            .iter()
            .filter(|s| s.id != top.id)
            .map(|s| format!("{} {}", s.name, s.population))
            .collect();
        pending.push(Pending {
            severity: Severity::Notable,
            kind: "lead_change",
            kind_id: 5,
            subtype_id: 0,
            species_id: Some(top.id),
            title: format!("{} is ahead", self.name(top.id)),
            evidence: format!(
                "{} {} against {}, held for {DWELL} epochs",
                top.name,
                top.population,
                behind.join(", ")
            ),
        });
    }

    fn trends(&mut self, report: &EpochReport, pending: &mut Vec<Pending>) {
        for detector in self.detectors.iter_mut() {
            let value = match detector.species {
                None => report.biomass,
                Some(id) => {
                    let Some(s) = report.species.iter().find(|s| s.id == id) else {
                        continue;
                    };
                    // a dead species has no behaviour, only a zero, and a
                    // detector fed zeros would announce its own extinction
                    // twice
                    if s.population == 0 {
                        continue;
                    }
                    match detector.feature {
                        Feature::Population => s.population as f32,
                        Feature::Births => s.births as f32,
                        Feature::Deaths => s.deaths as f32,
                        Feature::Energy => s.mean_energy,
                        Feature::Movement => s.behavior.movement,
                        Feature::ResourceTracking => s.behavior.resource_tracking,
                        Feature::Breeding => s.behavior.reproduction,
                        Feature::Resting => s.behavior.resting,
                        Feature::Exposure => s.behavior.competitor_exposure,
                        Feature::Metabolism => s.mean_genes.metabolism,
                        Feature::Speed => s.mean_genes.speed,
                        Feature::HeatPref => s.mean_genes.heat_pref,
                        Feature::Biomass => continue,
                    }
                }
            };
            let Some(crossing) = detector.update(value, report.epoch) else {
                continue;
            };
            let feature = detector.feature;
            let rising = crossing.divergence > 0.0;
            let (high, _, _) = feature.thresholds();
            let severity = match crossing.divergence.abs() / high {
                r if r >= 3.0 => Severity::Major,
                r if r >= 1.5 => Severity::Notable,
                _ => Severity::Info,
            };
            let subject = match detector.species {
                Some(id) => self.names.get(id as usize).cloned().unwrap_or_default(),
                None => "the world".to_string(),
            };
            pending.push(Pending {
                severity,
                kind: feature.kind(),
                kind_id: 6,
                subtype_id: FEATURES.iter().position(|f| *f == feature).unwrap_or(0) as u16,
                species_id: detector.species,
                title: format!("{subject} {} {}", feature.phrase(rising), feature.label()),
                evidence: format!(
                    "{} {:+.0}% against its running baseline, {:.3} -> {:.3}",
                    feature.label(),
                    crossing.divergence * 100.0,
                    crossing.slow,
                    crossing.fast
                ),
            });
        }
    }

    /// one complete order, and only then an id. arrival time, thread order and
    /// detector iteration order are all deliberately absent from this.
    fn emit(&mut self, epoch: usize, mut pending: Vec<Pending>) -> Vec<Event> {
        pending.sort_by_key(|p| {
            (p.severity, p.kind_id, p.subtype_id, p.species_id.map(|id| id as i64).unwrap_or(-1))
        });
        let mut emitted = Vec::with_capacity(pending.len());
        for p in pending {
            let event = Event {
                run_id: self.run_id,
                event_id: self.next_event_id,
                epoch,
                kind: p.kind,
                severity: p.severity.as_str(),
                species_id: p.species_id,
                title: p.title,
                evidence: p.evidence,
                detector_version: DETECTOR_VERSION,
            };
            self.next_event_id += 1;
            if self.events.len() == EVENT_CAPACITY {
                self.events.pop_front();
            }
            self.events.push_back(event.clone());
            emitted.push(event);
        }
        emitted
    }
}

/// the inherited-body meters a species profile draws, as the bounds they are
/// drawn against. the browser needs the scale to put a marker on it, and the
/// scale is a genetics constant rather than anything the run decides.
pub fn gene_bounds() -> serde_json::Value {
    serde_json::json!({
        "speed": [SPEED_BOUNDS.0, SPEED_BOUNDS.1],
        "size": [SIZE_BOUNDS.0, SIZE_BOUNDS.1],
        "metabolism": [METABOLISM_BOUNDS.0, METABOLISM_BOUNDS.1],
    })
}

#[cfg(test)]
mod tests;
