use super::*;
use ecosym_genetics::Genes;
use ecosym_simulation::{BehaviorStats, SpeciesResult, SpeciesStats};

fn names() -> Vec<String> {
    vec!["Species A".into(), "Species B".into()]
}

fn stats(id: u32, population: usize) -> SpeciesStats {
    SpeciesStats {
        id,
        name: format!("Species {}", if id == 0 { "A" } else { "B" }),
        population,
        births: 0,
        deaths: 0,
        mean_energy: 5.0,
        mean_genes: Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 },
        behavior: BehaviorStats {
            movement: 0.5,
            resource_tracking: 0.5,
            reproduction: 0.5,
            resting: 0.5,
            competitor_exposure: 0.01,
            occupied_temperature: 0.5,
            climate_fit: 0.8,
        },
        behavior_variance: BehaviorStats::default(),
        mean_brain: 0.0,
    }
}

fn report(epoch: usize, species: Vec<SpeciesStats>) -> EpochReport {
    EpochReport {
        epoch,
        population: species.iter().map(|s| s.population).sum(),
        biomass: 1000.0,
        species,
    }
}

/// the same reports in the same order must produce the same events, ids
/// included, or an event id means nothing across a reconnect
#[test]
fn the_same_lifecycle_produces_the_same_event_sequence() {
    let reports: Vec<EpochReport> = (0..120)
        .map(|e| {
            let mut a = stats(0, 500usize.saturating_sub(e * 4).max(1));
            let mut b = stats(1, 500 + e * 12);
            a.births = 5;
            b.births = 40 + e;
            a.deaths = 20;
            b.deaths = 10;
            a.mean_genes.metabolism = 1.0 - e as f32 * 0.005;
            b.behavior.movement = 0.5 + e as f32 * 0.004;
            report(e, vec![a, b])
        })
        .collect();

    let run = |run_id: i64| {
        let mut t = Telemetry::start(run_id, &names());
        let mut all = Vec::new();
        for r in &reports {
            all.extend(t.push(r));
        }
        all
    };
    let first = run(7);
    assert_eq!(first, run(7));
    assert!(!first.is_empty(), "nothing fired, so this test proves nothing");
    // ids are dense and monotonic over the whole run
    assert!(first.iter().enumerate().all(|(i, e)| e.event_id == i as u64));
    assert!(first.windows(2).all(|w| w[0].epoch <= w[1].epoch));
}

/// a run change resets everything, so nothing from the last run can be read as
/// belonging to this one
#[test]
fn starting_a_run_carries_nothing_over_from_the_last_one() {
    let mut old = Telemetry::start(1, &names());
    for e in 0..60 {
        old.push(&report(e, vec![stats(0, 500 - e * 5), stats(1, 500 + e * 20)]));
    }
    assert!(old.events().count() > 0);

    let fresh = Telemetry::start(2, &names());
    assert_eq!(fresh.events().count(), 0);
    assert!(fresh.detectors.iter().all(|d| !d.primed && !d.active));
}

#[test]
fn the_event_ring_stops_at_its_capacity_however_long_the_run_is() {
    let mut t = Telemetry::start(1, &names());
    // a sawtooth loud enough on every channel to keep the detectors talking
    for e in 0..4_000 {
        let swing = if (e / 30) % 2 == 0 { 1.0 } else { 0.2 };
        let mut a = stats(0, (500.0 * swing) as usize + 1);
        let mut b = stats(1, (900.0 / swing) as usize);
        a.behavior.movement = 0.2 * swing;
        b.behavior.resting = 0.8 * swing;
        a.mean_genes.speed = 1.5 * swing;
        t.push(&report(e, vec![a, b]));
    }
    assert_eq!(t.events().count(), EVENT_CAPACITY);
    // and the ring holds the newest, not the oldest
    assert_eq!(t.events().last().map(|e| e.event_id), Some(t.next_event_id - 1));
}

#[test]
fn an_extinction_is_reported_once_and_the_species_falls_silent_after_it() {
    let mut t = Telemetry::start(1, &names());
    t.push(&report(0, vec![stats(0, 500), stats(1, 500)]));
    let dead = t.push(&report(1, vec![stats(0, 0), stats(1, 500)]));
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].kind, "extinction");
    assert_eq!(dead[0].severity, "major");
    assert_eq!(dead[0].species_id, Some(0));

    for e in 2..80 {
        for event in t.push(&report(e, vec![stats(0, 0), stats(1, 500)])) {
            assert_ne!(event.species_id, Some(0), "a dead species kept talking: {event:?}");
        }
    }
}

#[test]
fn a_first_birth_is_announced_once_per_species() {
    let mut t = Telemetry::start(1, &names());
    let quiet = t.push(&report(0, vec![stats(0, 500), stats(1, 500)]));
    assert!(quiet.iter().all(|e| e.kind != "first_birth"));

    let mut a = stats(0, 500);
    a.births = 3;
    let first = t.push(&report(1, vec![a.clone(), stats(1, 500)]));
    assert_eq!(first.iter().filter(|e| e.kind == "first_birth").count(), 1);
    let again = t.push(&report(2, vec![a, stats(1, 500)]));
    assert!(again.iter().all(|e| e.kind != "first_birth"));
}

#[test]
fn scarcity_and_recovery_need_different_thresholds_to_cross() {
    let mut t = Telemetry::start(1, &names());
    t.push(&report(0, vec![stats(0, 500), stats(1, 500)]));

    let scarce = t.push(&report(1, vec![stats(0, 40), stats(1, 500)]));
    assert_eq!(scarce.iter().filter(|e| e.kind == "near_extinction").count(), 1);

    // back over the scarcity line but under the recovery line: still nothing
    let between = t.push(&report(2, vec![stats(0, 80), stats(1, 500)]));
    assert!(between.iter().all(|e| e.kind != "recovery"));

    let back = t.push(&report(3, vec![stats(0, 200), stats(1, 500)]));
    assert_eq!(back.iter().filter(|e| e.kind == "recovery").count(), 1);
}

/// the detector exists to be quiet. a flat series must produce nothing at all,
/// or the feed is noise with a run id attached.
#[test]
fn a_flat_run_says_nothing_after_its_first_birth() {
    let mut t = Telemetry::start(1, &names());
    for e in 0..500 {
        let mut a = stats(0, 500);
        let mut b = stats(1, 400);
        a.births = 10;
        b.births = 8;
        a.deaths = 10;
        b.deaths = 8;
        let events = t.push(&report(e, vec![a, b]));
        assert!(events.iter().all(|ev| ev.kind == "first_birth"), "a flat run emitted {events:?}");
    }
}

#[test]
fn the_terminal_result_comes_only_from_finish_and_only_once() {
    let mut t = Telemetry::start(1, &names());
    for e in 0..40 {
        for event in t.push(&report(e, vec![stats(0, 500), stats(1, 500)])) {
            assert_ne!(event.kind, "result");
        }
    }
    let outcome = RunOutcome {
        epochs: 40,
        species: vec![result(0, 500, 900), result(1, 500, 120)],
        winner: Winner::Species(0),
    };
    let done = t.finish(40, &outcome);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].kind, "result");
    assert!(done[0].title.contains("Species A"));
    assert!(done[0].evidence.contains("500 -> 900"));
    assert!(t.finish(40, &outcome).is_empty());
}

#[test]
fn every_event_carries_its_evidence_and_the_rules_that_produced_it() {
    let mut t = Telemetry::start(3, &names());
    for e in 0..200 {
        let mut a = stats(0, 500 + e * 30);
        let mut b = stats(1, 500usize.saturating_sub(e * 2).max(1));
        a.births = 10 + e;
        b.behavior.resting = 0.5 + (e as f32 * 0.01).min(0.4);
        for event in t.push(&report(e, vec![a, b])) {
            assert_eq!(event.run_id, 3);
            assert_eq!(event.detector_version, DETECTOR_VERSION);
            assert!(!event.title.is_empty() && !event.evidence.is_empty(), "{event:?}");
            assert!(["major", "notable", "info"].contains(&event.severity));
        }
    }
    assert!(t.events().count() > 0, "nothing fired, so this test proves nothing");
}

/// events inside one epoch are ordered before they are numbered, so the loudest
/// thing that happened is the lowest id
#[test]
fn events_in_one_epoch_are_ordered_by_severity_before_they_are_numbered() {
    let mut t = Telemetry::start(1, &names());
    t.push(&report(0, vec![stats(0, 500), stats(1, 500)]));
    let mut a = stats(0, 0);
    a.births = 4;
    let mut b = stats(1, 20);
    b.births = 4;
    let events = t.push(&report(1, vec![a, b]));
    assert!(events.len() > 1, "one event orders trivially, so this test proves nothing");
    let severities: Vec<&str> = events.iter().map(|e| e.severity).collect();
    assert!(severities.windows(2).all(|w| rank(w[0]) <= rank(w[1])), "{severities:?}");
    assert!(events.windows(2).all(|w| w[0].event_id < w[1].event_id));
}

fn rank(severity: &str) -> u8 {
    match severity {
        "major" => 0,
        "notable" => 1,
        _ => 2,
    }
}

fn result(id: u32, initial: usize, final_population: usize) -> SpeciesResult {
    SpeciesResult {
        id,
        name: format!("Species {}", if id == 0 { "A" } else { "B" }),
        initial,
        final_population,
        births: 0,
        deaths: 0,
        founder_genes: Genes::default(),
        final_genes: Genes::default(),
        final_energy: 0.0,
        founder_behavior: BehaviorStats::default(),
        final_behavior: BehaviorStats::default(),
        brain_drift: 0.0,
    }
}
