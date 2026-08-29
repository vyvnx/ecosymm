//! the shared lifecycle: one market, one run, one settlement, forever.
//!
//! ```text
//! recover interrupted markets
//!   -> grant recovery coins  -> open a market for 30s on a committed seed
//!   -> lock, and only then reveal the seed
//!   -> run 500 epochs paced over ~60s -> persist digest and outcome
//!   -> settle once -> open the next market
//! ```
//!
//! nothing in here reads a wager. the seed is drawn from the operating system
//! and committed to before the first bet, the market locks before the seed is
//! revealed, and the simulation is constructed only after that commit lands -
//! so a bettor can neither run the world ahead nor influence which world runs.

use crate::hub::{Hub, Slot};
use crate::store::{self, MarketRow, NewRun};
use crate::{auth, now, AppState};
use axum::extract::ws::Message;
use ecosym_core::SimConfig;
use ecosym_game::{ContestResult, MarketOutcome, MarketRules, Pool, SpeciesTally};
use ecosym_replay::Recorder;
use ecosym_simulation::{default_blueprints, RenderSnapshot, RenderWorld, RunOutcome, Simulation};
use serde::Serialize;
use serde_json::json;
use std::time::{Duration, Instant};

/// long enough to notice the market and decide, short enough to keep watching.
/// fixed constants, not runtime configuration.
pub const MARKET_WINDOW: Duration = Duration::from_secs(30);
pub const RUN_WINDOW: Duration = Duration::from_secs(60);
/// long enough to read what the market paid before the next one opens over
/// it. without it a settlement is broadcast and replaced in the same instant.
pub const RESULT_WINDOW: Duration = Duration::from_secs(8);

/// at most ~15 render samples a wall-clock second. when the server falls
/// behind it is samples that are dropped, never epochs.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(66);

/// how long the coordinator spends on each phase, and the scenario every run
/// uses. tests hand it a small world and near-zero windows; nothing else ever
/// changes it, which is why it is an argument rather than configuration.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub market: Duration,
    pub run: Duration,
    pub result: Duration,
    /// only the seed differs between runs; the rest is the fixed scenario
    pub config: SimConfig,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            market: MARKET_WINDOW,
            run: RUN_WINDOW,
            result: RESULT_WINDOW,
            config: SimConfig::default(),
        }
    }
}

/// the domain separator in the seed commitment. it is in the hash so a
/// commitment can only ever be read as a commitment to *this* protocol.
const COMMITMENT_TAG: &[u8] = b"ecosym-market-commitment-v1";

/// `sha256(tag || run_id || config || seed || nonce)`, published before the
/// first bet and checkable by anyone once the reveal lands. the run id is in
/// there too, so a commitment cannot be replayed onto a different run.
pub fn commitment(run_id: i64, config_json: &str, seed: u64, nonce_hex: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(COMMITMENT_TAG);
    hash.update(run_id.to_le_bytes());
    hash.update((config_json.len() as u64).to_le_bytes());
    hash.update(config_json.as_bytes());
    hash.update(seed.to_le_bytes());
    hash.update(nonce_hex.as_bytes());
    auth::hex(&hash.finalize())
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SpeciesLabel {
    pub id: u32,
    pub name: String,
}

/// the two species a market is about, in the order the buttons are drawn
pub fn species_labels() -> Vec<SpeciesLabel> {
    default_blueprints()
        .iter()
        .enumerate()
        .map(|(i, b)| SpeciesLabel { id: i as u32, name: b.name.clone() })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct RulesView {
    pub version: u32,
    pub fee_bps: u32,
    pub coexistence_margin: f64,
    pub min_stake: i64,
    pub max_stake: i64,
}

/// everything public about the current market. the seed and nonce are `None`
/// until the market has locked, which is enforced by [`MarketRow::reveal`].
#[derive(Clone, Debug, Serialize)]
pub struct MarketView {
    pub run_id: i64,
    pub market_id: i64,
    pub revision: i64,
    pub phase: &'static str,
    pub run_status: String,
    /// absolute server time, never a countdown started on arrival
    pub server_time: i64,
    pub opened_at: i64,
    pub locks_at: i64,
    pub commitment: String,
    pub seed_hex: Option<String>,
    pub nonce_hex: Option<String>,
    pub species: Vec<SpeciesLabel>,
    pub rules: RulesView,
    pub pools: [i64; 3],
    /// decimal return one more coin would claim on each outcome, from the
    /// pools as they stand. an estimate that moves until lock.
    pub projected: [f64; 3],
    pub gross_pool: i64,
    pub burn: i64,
    pub winning_outcome: Option<MarketOutcome>,
    pub digest: Option<String>,
}

pub fn view(market: &MarketRow, pools: [i64; 3], now: i64) -> MarketView {
    let totals = pools.map(|p| ecosym_game::Coins::new(p).unwrap_or_default());
    let pool = Pool::new(totals);
    let one = ecosym_game::Coins::new(1).expect("one coin");
    let projected =
        MarketOutcome::ALL.map(|o| pool.projection(o, one, &market.rules).unwrap_or(0.0));
    let reveal = market.reveal();
    MarketView {
        run_id: market.run_id,
        market_id: market.id,
        revision: market.revision,
        phase: market.status.as_str(),
        run_status: market.run_status.clone(),
        server_time: now,
        opened_at: market.opened_at,
        locks_at: market.locks_at,
        commitment: market.commitment.clone(),
        seed_hex: reveal.map(|(seed, _)| format!("{seed:#x}")),
        nonce_hex: reveal.map(|(_, nonce)| nonce.to_string()),
        species: species_labels(),
        rules: RulesView {
            version: market.rules.version,
            fee_bps: market.rules.fee_bps,
            coexistence_margin: market.rules.coexistence_margin,
            min_stake: market.rules.min_stake.get(),
            max_stake: market.rules.max_stake.get(),
        },
        pools,
        projected,
        gross_pool: market.gross_pool.unwrap_or(0),
        burn: market.burn.unwrap_or(0),
        winning_outcome: market.winning_outcome,
        digest: market.digest.clone(),
    }
}

/// the whole market as one json message
pub fn market_message(kind: &str, view: &MarketView) -> Message {
    Message::Text(json!({ "type": kind, "market": view }).to_string().into())
}

/// read the current market out of the database and publish it. every route
/// that moves a pool calls this, so the retained bundle and the live stream
/// can never disagree about what the market is.
pub async fn republish(state: &AppState, market_id: i64, kind: &str) -> store::Result<MarketView> {
    let market =
        store::market(&state.db, market_id).await?.ok_or(store::Refusal::MarketNotFound)?;
    let pools = store::pools(&state.db, market_id).await?;
    let view = view(&market, pools, now());
    state.hub.publish(Slot::Market, market_message(kind, &view));
    Ok(view)
}

/// the process-long loop. it keeps going with nobody watching, which is the
/// whole point: the world does not wait for an audience.
pub async fn run_forever(state: AppState, schedule: Schedule) {
    // a restart lost whatever simulation was in flight, so anything still
    // live is refunded before a new run can start
    match store::recover_interrupted(&state.db, now()).await {
        Ok(ids) if !ids.is_empty() => {
            println!("refunded {} interrupted market(s) from the last run", ids.len())
        }
        Err(e) => eprintln!("recovery failed: {e}"),
        _ => {}
    }

    loop {
        if let Err(e) = one_run(&state, &schedule).await {
            eprintln!("run failed: {e}");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

async fn one_run(state: &AppState, schedule: &Schedule) -> store::Result<()> {
    let opened_at = now();

    // the anti-bankruptcy floor, on the server's clock and between runs
    for id in store::grant_recovery_to_eligible(&state.db, opened_at).await? {
        if let Some(account) = store::account(&state.db, id).await? {
            state.hub.account_changed(id, account.revision);
        }
    }

    let seed = u64::from_le_bytes(auth::random_bytes::<8>());
    let nonce = auth::hex(&auth::random_bytes::<16>());
    let cfg = SimConfig { seed, ..schedule.config.clone() };
    let config_json = serde_json::to_string(&cfg).expect("config serialises");

    let market = store::open_market(
        &state.db,
        NewRun { config_json: &config_json, seed, nonce_hex: &nonce, engine: "cpu" },
        |run_id| commitment(run_id, &config_json, seed, &nonce),
        &MarketRules::V1,
        opened_at,
        // the deadline the server rejects late bets against is in whole
        // seconds, so a sub-second window still leaves one second of grace.
        // only tests run windows that short.
        opened_at + (schedule.market.as_secs() as i64).max(1),
    )
    .await?;

    let pools = store::pools(&state.db, market.id).await?;
    state
        .hub
        .open_run(market.run_id, market_message("market_open", &view(&market, pools, opened_at)));

    tokio::time::sleep(schedule.market).await;

    // one-way, and the reveal comes only after it commits
    let market = store::lock_market(&state.db, market.id, now()).await?;
    republish(state, market.id, "market_locked").await?;

    let hub = state.hub.clone();
    let run_id = market.run_id;
    let epoch_pace = schedule.run.checked_div(cfg.epochs.max(1) as u32).unwrap_or_default();
    let (digest, outcome) =
        tokio::task::spawn_blocking(move || simulate(cfg, &hub, run_id, epoch_pace))
            .await
            .expect("simulation thread");

    let outcome_json = serde_json::to_string(&outcome).expect("outcome serialises");
    store::complete_run(&state.db, market.run_id, &digest, &outcome_json, now()).await?;

    // settlement reads a persisted, finished run. never a browser message and
    // never a render snapshot.
    match contest(&outcome) {
        Some(contest) if contest.resolve(market.rules.coexistence_margin).winner().is_some() => {
            store::settle_market(&state.db, market.id, &contest, now()).await?;
        }
        _ => {
            store::void_market(&state.db, market.id, now()).await?;
        }
    }

    for (account_id, revision) in store::accounts_in_market(&state.db, market.id).await? {
        state.hub.account_changed(account_id, revision);
    }
    republish(state, market.id, "market_settled").await?;

    // the settled market stays current while it can still be read. the next
    // one opening is what replaces it.
    tokio::time::sleep(schedule.result).await;
    Ok(())
}

/// the game layer's view of a finished run: two identities and four head
/// counts. nothing else crosses.
fn contest(outcome: &RunOutcome) -> Option<ContestResult> {
    let [a, b] = <&[_; 2]>::try_from(outcome.species.as_slice()).ok()?;
    let tally = |s: &ecosym_simulation::SpeciesResult| SpeciesTally {
        id: s.id,
        initial: s.initial as u64,
        final_population: s.final_population as u64,
    };
    ContestResult::new(tally(a), tally(b)).ok()
}

/// the run itself, on a blocking thread. it publishes as it goes and hands
/// back the digest and the outcome.
///
/// the pacing sleep sits between finished epochs. it changes when an epoch is
/// published, never how many run, in what order they visit, what the rng
/// draws, or what the digest folds - so a paced run and an unpaced one are the
/// same run.
fn simulate(cfg: SimConfig, hub: &Hub, run_id: i64, epoch_pace: Duration) -> (String, RunOutcome) {
    let mut sim = Simulation::cpu(cfg.clone());
    let mut recorder = Recorder::new(cfg.clone(), sim.engine_id());
    let summary = sim.state.world.summary();

    hub.publish(
        Slot::Config,
        Message::Text(
            json!({
                "type": "config",
                "protocol_version": crate::PROTOCOL_VERSION,
                "run_id": run_id,
                "config": cfg,
                // a u64 does not survive the browser's json number, so the
                // seed ships again as the hex string the ui shows
                "seed_hex": format!("{:#x}", cfg.seed),
                "engine": sim.engine_id(),
                "world": {
                    "width": summary.width,
                    "height": summary.height,
                    "habitable_tiles": summary.habitable_tiles,
                    "initial_biomass": summary.initial_biomass,
                    "mean_temperature": summary.mean_temperature,
                },
                // stable order: the client indexes species by position
                "species": sim.state.species.iter().map(|s| json!({
                    "id": s.id().get(),
                    "name": s.name(),
                    "founder_genes": s.founder_genes(),
                })).collect::<Vec<_>>(),
            })
            .to_string()
            .into(),
        ),
    );

    let world = match RenderWorld::extract(&sim.state.world) {
        Ok(world) => world,
        Err(e) => return failed(hub, e, sim),
    };
    match crate::wire::encode_world(&world) {
        Ok(bytes) => hub.publish(Slot::World, Message::Binary(bytes.into())),
        Err(e) => return failed(hub, e, sim),
    };

    let mut sampled = None;
    let mut last_sample = Instant::now();
    snapshot(hub, &sim, &world, &mut sampled);

    let started = Instant::now();
    for epoch in 0..cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        let extinct = report.population == 0;
        hub.publish(
            Slot::Report,
            Message::Text(
                json!({ "type": "epoch", "run_id": run_id, "report": report }).to_string().into(),
            ),
        );
        recorder.push(report);

        if last_sample.elapsed() >= SAMPLE_INTERVAL {
            last_sample = Instant::now();
            snapshot(hub, &sim, &world, &mut sampled);
        }
        if extinct {
            break;
        }
        // catch up to where this epoch was meant to finish. falling behind
        // costs samples, never epochs.
        if let Some(wait) =
            (started + epoch_pace * (epoch as u32 + 1)).checked_duration_since(Instant::now())
        {
            std::thread::sleep(wait);
        }
    }

    if sampled != Some(sim.epoch()) {
        snapshot(hub, &sim, &world, &mut sampled);
    }

    let outcome = sim.outcome();
    hub.publish(
        Slot::Result,
        Message::Text(
            json!({
                "type": "done",
                "run_id": run_id,
                "digest": recorder.digest_hex(),
                "epochs": recorder.epochs(),
                "outcome": outcome,
            })
            .to_string()
            .into(),
        ),
    );
    (recorder.digest_hex(), outcome)
}

fn snapshot(hub: &Hub, sim: &Simulation, world: &RenderWorld, sampled: &mut Option<usize>) {
    let bytes = RenderSnapshot::extract(&sim.state)
        .ok()
        .and_then(|snap| crate::wire::encode_snapshot(&snap, world).ok());
    if let Some(bytes) = bytes {
        *sampled = Some(sim.epoch());
        hub.publish(Slot::Snapshot, Message::Binary(bytes.into()));
    }
}

/// a run the viewer cannot be told about is still a run that has to end, so
/// the market settles on whatever the simulation actually did
fn failed(hub: &Hub, why: impl std::fmt::Display, sim: Simulation) -> (String, RunOutcome) {
    hub.publish(
        Slot::Result,
        Message::Text(json!({ "type": "error", "message": why.to_string() }).to_string().into()),
    );
    (String::new(), sim.outcome())
}

#[cfg(test)]
mod tests;
