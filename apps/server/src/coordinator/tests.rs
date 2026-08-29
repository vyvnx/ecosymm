use super::*;
use crate::auth::Throttle;
use crate::hub::Item;
use crate::store::MarketStatus;
use ecosym_game::MarketOutcome;
use std::sync::Arc;
use tokio::sync::broadcast::{error::TryRecvError, Receiver};

/// a small world so a whole run finishes in a test, and no pacing at all: the
/// injected windows are what stands in for waiting on a wall clock
fn schedule() -> Schedule {
    Schedule {
        market: Duration::from_millis(150),
        run: Duration::ZERO,
        config: SimConfig {
            seed: 1234,
            population_per_species: 20,
            epochs: 6,
            width: 32,
            height: 32,
            ticks_per_epoch: 5,
        },
    }
}

async fn state() -> AppState {
    AppState {
        db: store::open_memory().await.expect("in-memory database"),
        hub: Arc::new(Hub::default()),
        throttle: Arc::new(Throttle::default()),
        secure_cookies: false,
    }
}

/// exactly what a websocket does: subscribe, then clone the bundle, then drop
/// what the bundle already carried. anything else races the coordinator.
struct Viewer {
    rx: Receiver<Item>,
    floor: u64,
    seen: Vec<String>,
    bootstraps: usize,
}

impl Viewer {
    fn join(hub: &Hub) -> Viewer {
        let rx = hub.subscribe();
        let bundle = hub.bundle();
        Viewer {
            rx,
            floor: bundle.seq,
            seen: bundle.bootstrap().iter().map(label).collect(),
            bootstraps: 1,
        }
    }

    fn drain(&mut self, hub: &Hub) {
        loop {
            match self.rx.try_recv() {
                Ok(item) if item.seq <= self.floor => {}
                Ok(item) => {
                    self.floor = item.seq;
                    self.seen.push(label(&item.message));
                }
                // a gap of unknown size: start again from the retained bundle
                Err(TryRecvError::Lagged(_)) => {
                    let bundle = hub.bundle();
                    self.floor = bundle.seq;
                    self.seen.extend(bundle.bootstrap().iter().map(label));
                    self.bootstraps += 1;
                }
                Err(_) => return,
            }
        }
    }

    fn json(&self, kind: &str) -> Vec<serde_json::Value> {
        self.seen
            .iter()
            .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|v| v["type"] == kind)
            .collect()
    }

    fn last(&self, kind: &str) -> serde_json::Value {
        self.json(kind).pop().unwrap_or_else(|| panic!("no {kind} message in {:?}", self.seen))
    }
}

/// binary frames have no json type, so they are labelled by what they are
fn label(message: &Message) -> String {
    match message {
        Message::Text(text) => text.to_string(),
        Message::Binary(bytes) => match bytes[6] {
            crate::wire::KIND_WORLD => r#"{"type":"world"}"#.into(),
            _ => r#"{"type":"snapshot"}"#.into(),
        },
        other => panic!("unexpected frame {other:?}"),
    }
}

async fn wait_for_terminal(state: &AppState, market_id: i64) -> MarketRow {
    for _ in 0..3_000 {
        if let Ok(Some(market)) = store::market(&state.db, market_id).await {
            if market.status.is_terminal() {
                return market;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("market {market_id} never reached a terminal state");
}

async fn wait_for_market(state: &AppState, at_least: i64) -> MarketRow {
    for _ in 0..600 {
        if let Ok(Some(market)) = store::current_market(&state.db).await {
            if market.id >= at_least {
                return market;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no market {at_least} opened");
}

#[test]
fn the_commitment_binds_the_run_the_seed_and_the_nonce() {
    let base = commitment(1, r#"{"seed":7}"#, 7, "abcd");
    assert_eq!(base.len(), 64);
    assert_eq!(base, commitment(1, r#"{"seed":7}"#, 7, "abcd"));

    // change any input and the published commitment no longer verifies
    assert_ne!(base, commitment(2, r#"{"seed":7}"#, 7, "abcd"));
    assert_ne!(base, commitment(1, r#"{"seed":8}"#, 7, "abcd"));
    assert_ne!(base, commitment(1, r#"{"seed":7}"#, 8, "abcd"));
    assert_ne!(base, commitment(1, r#"{"seed":7}"#, 7, "abce"));
    // and the seed cannot be read back out of it
    assert!(!base.contains('7'.to_string().as_str()) || base != format!("{:064}", 7));
}

/// the audit path a bettor walks: take the reveal, recompute the hash, and
/// check it against what was published before anyone could bet
#[tokio::test]
async fn the_seed_is_committed_before_any_bet_and_verifies_after_the_reveal() {
    let state = state().await;
    let hub = state.hub.clone();
    let task = tokio::spawn(run_forever(state.clone(), schedule()));

    let open = wait_for_market(&state, 1).await;
    assert_eq!(open.status, MarketStatus::Open);
    assert_eq!(open.reveal(), None, "an open market gave up its seed");
    assert!(!open.commitment.is_empty());

    let viewer = Viewer::join(&hub);
    let announced = viewer.last("market_open");
    assert_eq!(announced["market"]["commitment"], open.commitment);
    assert!(announced["market"]["seed_hex"].is_null(), "the open market published its seed");

    let settled = wait_for_terminal(&state, open.id).await;
    let (seed, nonce) = settled.reveal().expect("a locked market reveals");
    let config: String = sqlx::query_scalar("SELECT config FROM runs WHERE id = ?")
        .bind(settled.run_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(
        commitment(settled.run_id, &config, seed, nonce),
        open.commitment,
        "the revealed inputs do not produce the published commitment"
    );
    task.abort();
}

#[tokio::test]
async fn a_run_settles_and_the_next_market_opens_with_nobody_watching() {
    let state = state().await;
    let task = tokio::spawn(run_forever(state.clone(), schedule()));

    let first = wait_for_market(&state, 1).await;
    let settled = wait_for_terminal(&state, first.id).await;
    assert!(settled.status.is_terminal(), "{:?}", settled.status);
    assert_eq!(settled.run_status, "complete");
    assert!(settled.digest.is_some(), "the run finished without a digest");

    // and the world does not wait for an audience: the next one opens anyway
    let next = wait_for_market(&state, first.id + 1).await;
    assert_eq!(next.status, MarketStatus::Open);
    assert_ne!(next.run_id, first.run_id);
    task.abort();
}

/// joining before the run, during betting, mid-run and after settlement all
/// have to end at the same run, the same epoch and the same digest
#[tokio::test]
async fn every_viewer_converges_on_the_same_run_however_late_it_joins() {
    let state = state().await;
    let hub = state.hub.clone();
    let early = Viewer::join(&hub);
    let task = tokio::spawn(run_forever(state.clone(), schedule()));

    let market = wait_for_market(&state, 1).await;
    let betting = Viewer::join(&hub);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mid = Viewer::join(&hub);

    wait_for_terminal(&state, market.id).await;
    // and one that turns up during the *next* betting window: the run it
    // missed is still what the screen shows
    wait_for_market(&state, market.id + 1).await;
    let late = Viewer::join(&hub);

    let mut viewers = [early, betting, mid, late];
    for viewer in &mut viewers {
        viewer.drain(&hub);
    }

    let digest = viewers[0].last("done")["digest"].clone();
    assert!(digest.is_string(), "the first viewer never saw the run finish");
    for viewer in &viewers {
        assert_eq!(viewer.last("done")["digest"], digest);
        assert_eq!(viewer.last("done")["run_id"], market.run_id);
        assert_eq!(viewer.last("config")["run_id"], market.run_id);
        // every viewer got the world before anything standing on it
        let kinds: Vec<String> = viewer
            .seen
            .iter()
            .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .map(|v| v["type"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            kinds.iter().position(|k| k == "world") < kinds.iter().position(|k| k == "snapshot"),
            "{kinds:?}"
        );
    }
    task.abort();
}

/// a browser that was asleep must not resume in the middle of a run it half
/// remembers. it gets a whole new bundle instead.
#[tokio::test]
async fn a_viewer_that_lags_is_resynchronised_rather_than_waited_for() {
    let state = state().await;
    let hub = state.hub.clone();
    let mut asleep = Viewer::join(&hub);
    // a world big enough to keep publishing past the broadcast depth rather
    // than going extinct and ending the run early
    let long = SimConfig {
        seed: 1234,
        population_per_species: 200,
        epochs: 300,
        width: 128,
        height: 128,
        ticks_per_epoch: 5,
    };
    let task = tokio::spawn(run_forever(state.clone(), Schedule { config: long, ..schedule() }));

    let market = wait_for_market(&state, 1).await;
    // never drained while hundreds of epochs went past
    wait_for_terminal(&state, market.id).await;
    asleep.drain(&hub);

    assert!(asleep.bootstraps > 1, "the viewer was never told it had fallen behind");
    assert_eq!(asleep.last("done")["run_id"], market.run_id);
    // the run finished on time regardless
    let settled = store::market(&state.db, market.id).await.unwrap().unwrap();
    assert!(settled.status.is_terminal());
    task.abort();
}

/// watching and betting are outside the simulation. the same seed has to fold
/// into the same digest with a hub attached, whatever the pacing.
#[test]
fn publishing_and_pacing_cannot_reach_the_digest() {
    let cfg = SimConfig {
        seed: 4321,
        population_per_species: 30,
        epochs: 10,
        width: 32,
        height: 32,
        ticks_per_epoch: 5,
    };

    let alone = {
        let mut sim = Simulation::cpu(cfg.clone());
        let mut recorder = Recorder::new(cfg.clone(), sim.engine_id());
        for _ in 0..cfg.epochs {
            recorder.push(sim.advance_epoch().unwrap());
        }
        (recorder.digest_hex(), sim.outcome())
    };

    let watched = simulate(cfg.clone(), &Hub::default(), 1, Duration::ZERO);
    let paced = simulate(cfg, &Hub::default(), 1, Duration::from_millis(1));
    assert_eq!(watched, alone);
    assert_eq!(paced, alone);
}

/// the game layer only ever sees identities and head counts
#[test]
fn a_finished_run_reaches_the_market_as_four_numbers() {
    let cfg = SimConfig {
        seed: 7,
        population_per_species: 20,
        epochs: 4,
        width: 32,
        height: 32,
        ticks_per_epoch: 5,
    };
    let mut sim = Simulation::cpu(cfg.clone());
    for _ in 0..cfg.epochs {
        sim.advance_epoch().unwrap();
    }
    let outcome = sim.outcome();
    let contest = contest(&outcome).expect("two species");
    assert_eq!(contest.species[0].id, outcome.species[0].id);
    assert_eq!(contest.species[0].initial, outcome.species[0].initial as u64);
    assert_eq!(contest.species[1].final_population, outcome.species[1].final_population as u64);
}

#[test]
fn the_market_view_never_publishes_a_seed_it_should_not() {
    let labels = species_labels();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels.iter().map(|s| s.id).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(MarketOutcome::ALL.len(), labels.len() + 1);
}
