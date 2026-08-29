//! http + websocket front door. streams aggregate reports as json and sampled
//! render state as binary frames.
//!
//! the epoch loop runs on a blocking thread and hands the socket task finished
//! messages through a two-deep channel. the simulation is the producer and the
//! browser is the consumer, so the channel is where backpressure lands: a slow
//! client slows the producer down instead of growing a queue behind it.

mod wire;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use ecosym_core::SimConfig;
use ecosym_replay::Recorder;
use ecosym_simulation::{RenderSnapshot, RenderWorld, Simulation};
use serde::Deserialize;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, Sender};

const ADDR: &str = "127.0.0.1:3001";

/// the wire contract the browser decoder is written against
const PROTOCOL_VERSION: u16 = wire::VERSION;

/// at most ~15 render samples a wall-clock second. the simulation is free to
/// run faster than that; the extra epochs still reach the browser as reports.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(66);

/// two finished messages in flight. deep enough that encoding and sending
/// overlap, shallow enough that server memory cannot grow with a slow client.
const QUEUE_DEPTH: usize = 2;

#[derive(Deserialize)]
struct Params {
    seed: Option<String>,
    population_per_species: Option<usize>,
    epochs: Option<usize>,
}

impl Params {
    fn config(&self) -> SimConfig {
        let d = SimConfig::default();
        SimConfig {
            seed: self.seed.as_deref().and_then(parse_seed).unwrap_or_else(random_seed),
            population_per_species: self
                .population_per_species
                .unwrap_or(d.population_per_species)
                .clamp(1, 100_000),
            epochs: self.epochs.unwrap_or(d.epochs).clamp(1, 10_000),
            ..d
        }
    }
}

/// the ui prints seeds in hex, so `?seed=0x7bd48e9f...` is what pins a run to
/// a world someone watched. decimal still parses, and anything else is treated
/// as no seed at all.
fn parse_seed(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// a run with no `?seed=` in its query gets a fresh world. runs follow one
/// another on reconnect, so without this every viewer would watch the same
/// world play out again. an explicit seed still pins the run, and the seed in
/// use always ships in the config message, so any run stays reproducible.
fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// one finished frame, ready to go out. the producer encodes; the socket task
/// only writes, so no simulation work ever runs on a tokio worker.
enum Out {
    Text(String),
    Binary(Vec<u8>),
}

/// why the producer stopped early
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    /// the browser went away. not an error - the run is simply over.
    Gone,
    /// state the viewer cannot describe. worth telling the browser about.
    Failed(String),
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/config", get(|| async { Json(SimConfig::default()) }))
        .route("/ws", get(ws_handler));

    let listener = tokio::net::TcpListener::bind(ADDR).await.expect("bind");
    println!("ecosym-server listening on http://{ADDR}  (ws://{ADDR}/ws)");
    axum::serve(listener, app).await.expect("serve");
}

async fn ws_handler(ws: WebSocketUpgrade, Query(p): Query<Params>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_run(socket, p.config()))
}

async fn stream_run(mut socket: WebSocket, cfg: SimConfig) {
    let (tx, mut rx) = mpsc::channel(QUEUE_DEPTH);
    let producer = tokio::task::spawn_blocking(move || produce(cfg, &tx));

    while let Some(out) = rx.recv().await {
        let sent = match out {
            Out::Text(s) => socket.send(Message::Text(s.into())).await,
            Out::Binary(b) => socket.send(Message::Binary(b.into())).await,
        };
        if sent.is_err() {
            break;
        }
    }
    // drop first: the producer may be parked on a full channel, and it only
    // learns the browser is gone when the receiver goes with it
    drop(rx);
    let _ = producer.await;
}

fn produce(cfg: SimConfig, tx: &Sender<Out>) {
    if let Err(Stop::Failed(why)) = run(cfg, tx) {
        let error = serde_json::json!({ "type": "error", "message": why });
        let _ = tx.blocking_send(Out::Text(error.to_string()));
    }
}

/// ```text
/// config json -> world binary -> initial snapshot
///   -> (epoch json, sampled snapshot)*
///   -> final snapshot -> done json
/// ```
fn run(cfg: SimConfig, tx: &Sender<Out>) -> Result<(), Stop> {
    let mut sim = Simulation::cpu(cfg.clone());
    let mut rec = Recorder::new(cfg.clone(), sim.engine_id());

    let summary = sim.state.world.summary();
    let start = serde_json::json!({
        "type": "config",
        "protocol_version": PROTOCOL_VERSION,
        "config": cfg,
        // a u64 does not survive the browser's json number, so the seed ships
        // again as the hex string the ui shows and `?seed=` takes back
        "seed_hex": format!("{:#x}", cfg.seed),
        "engine": sim.engine_id(),
        "world": {
            "width": summary.width,
            "height": summary.height,
            "habitable_tiles": summary.habitable_tiles,
            "initial_biomass": summary.initial_biomass,
            "mean_temperature": summary.mean_temperature,
        },
        // stable order: the client indexes species by position, never by a map
        // key, and derives its palette from the same order
        "species": sim.state.species.iter().map(|s| serde_json::json!({
            "id": s.id().get(),
            "name": s.name(),
            "founder_genes": s.founder_genes(),
        })).collect::<Vec<_>>(),
    });
    send(tx, Out::Text(start.to_string()))?;

    let world = RenderWorld::extract(&sim.state.world).map_err(failed)?;
    send(tx, Out::Binary(wire::encode_world(&world).map_err(failed)?))?;

    let mut sampled = None;
    let mut last_sample = Instant::now();
    snapshot(tx, &sim, &world, &mut sampled)?;

    for _ in 0..cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        let extinct = report.population == 0;
        let message = serde_json::json!({ "type": "epoch", "report": report });
        rec.push(report);
        send(tx, Out::Text(message.to_string()))?;

        if last_sample.elapsed() >= SAMPLE_INTERVAL {
            last_sample = Instant::now();
            snapshot(tx, &sim, &world, &mut sampled)?;
        }
        if extinct {
            break;
        }
    }

    // the terminal state always ships, unless the last sample already was it -
    // an identical payload twice would restart the browser's interpolation
    // from a state it is already showing
    if sampled != Some(sim.epoch()) {
        snapshot(tx, &sim, &world, &mut sampled)?;
    }

    let done = serde_json::json!({
        "type": "done",
        "digest": rec.digest_hex(),
        "epochs": rec.epochs(),
        "outcome": sim.outcome(),
    });
    send(tx, Out::Text(done.to_string()))
}

fn snapshot(
    tx: &Sender<Out>,
    sim: &Simulation,
    world: &RenderWorld,
    sampled: &mut Option<usize>,
) -> Result<(), Stop> {
    let snap = RenderSnapshot::extract(&sim.state).map_err(failed)?;
    let bytes = wire::encode_snapshot(&snap, world).map_err(failed)?;
    *sampled = Some(sim.epoch());
    send(tx, Out::Binary(bytes))
}

fn send(tx: &Sender<Out>, out: Out) -> Result<(), Stop> {
    tx.blocking_send(out).map_err(|_| Stop::Gone)
}

fn failed(e: impl std::fmt::Display) -> Stop {
    Stop::Failed(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_simulation::Winner;

    fn small() -> SimConfig {
        SimConfig {
            seed: 1234,
            population_per_species: 20,
            epochs: 4,
            width: 32,
            height: 32,
            ticks_per_epoch: 5,
        }
    }

    /// what a client would see, in order: json message types by name, binary
    /// frames by kind and epoch
    #[derive(Debug, PartialEq)]
    enum Seen {
        Json(String),
        World,
        Snapshot(u32),
    }

    fn label(out: &Out) -> Seen {
        match out {
            Out::Text(s) => {
                let v: serde_json::Value = serde_json::from_str(s).expect("valid json");
                Seen::Json(v["type"].as_str().unwrap().to_string())
            }
            Out::Binary(b) => match b[6] {
                wire::KIND_WORLD => Seen::World,
                wire::KIND_SNAPSHOT => {
                    Seen::Snapshot(u32::from_le_bytes(b[12..16].try_into().unwrap()))
                }
                other => panic!("unknown binary kind {other}"),
            },
        }
    }

    async fn collect(cfg: SimConfig) -> (Vec<Seen>, Result<(), Stop>) {
        let (tx, mut rx) = mpsc::channel(QUEUE_DEPTH);
        let producer = tokio::task::spawn_blocking(move || run(cfg, &tx));
        let mut seen = Vec::new();
        while let Some(out) = rx.recv().await {
            seen.push(label(&out));
        }
        (seen, producer.await.unwrap())
    }

    #[test]
    fn params_map_onto_the_epoch_config() {
        let p = Params { seed: Some("7".into()), population_per_species: Some(9), epochs: Some(3) };
        let cfg = p.config();
        assert_eq!((cfg.seed, cfg.population_per_species, cfg.epochs), (7, 9, 3));

        let empty = Params { seed: None, population_per_species: None, epochs: None };
        let d = SimConfig::default();
        assert_eq!(empty.config().population_per_species, d.population_per_species);
        assert_eq!(empty.config().epochs, d.epochs);
        // no seed asked for means the next run is a different world
        assert_ne!(empty.config().seed, empty.config().seed);
    }

    /// what the ui prints is what pins the run: the hex seed in the config
    /// message has to come back through `?seed=` as the same u64
    #[test]
    fn a_seed_survives_the_round_trip_through_hex() {
        let seed = 0xdead_beef_1234_5678u64;
        let shown = format!("{seed:#x}");
        let back = Params { seed: Some(shown), population_per_species: None, epochs: None };
        assert_eq!(back.config().seed, seed);

        assert_eq!(parse_seed("1234"), Some(1234));
        assert_eq!(parse_seed("0xff"), Some(255));
        assert_eq!(parse_seed("not a seed"), None);
    }

    /// the frontend indexes species by array position, so the wire format has
    /// to stay an ordered list and never become an object keyed by id or name
    #[test]
    fn every_species_is_serialised_in_stable_order() {
        let cfg = SimConfig { population_per_species: 5, epochs: 2, ..SimConfig::default() };
        let mut sim = Simulation::cpu(cfg);
        let report = sim.advance_epoch().unwrap();
        let json = serde_json::to_value(&report).unwrap();

        let species = json["species"].as_array().expect("species must be an array");
        assert_eq!(species.len(), 2);
        assert_eq!(species[0]["id"], 0);
        assert_eq!(species[1]["id"], 1);
        assert!(species[0]["mean_genes"]["speed"].is_number());
        assert!(json["epoch"].is_number() && json["biomass"].is_number());
        // behavioural means reach the wire too, ready for the client to read
        assert!(species[0]["behavior"]["movement"].is_number());
        assert!(species[0]["behavior"]["food_seeking"].is_number());
        assert!(species[0]["behavior"]["resting"].is_number());
        assert!(species[0]["mean_brain"].is_number());
    }

    #[test]
    fn the_winner_wire_format_is_what_the_frontend_reads() {
        assert_eq!(serde_json::to_string(&Winner::None).unwrap(), r#""None""#);
        assert_eq!(serde_json::to_string(&Winner::Species(1)).unwrap(), r#"{"Species":1}"#);
        assert_eq!(serde_json::to_string(&Winner::Tie(vec![0, 1])).unwrap(), r#"{"Tie":[0,1]}"#);
    }

    #[tokio::test]
    async fn the_run_streams_config_world_and_an_initial_snapshot_before_any_epoch() {
        let (seen, result) = collect(small()).await;
        assert_eq!(result, Ok(()));
        assert_eq!(seen[0], Seen::Json("config".into()));
        assert_eq!(seen[1], Seen::World);
        assert_eq!(seen[2], Seen::Snapshot(0), "the initial state ships before epoch 1");
        assert_eq!(seen.last(), Some(&Seen::Json("done".into())));
        assert_eq!(
            seen.iter().filter(|s| **s == Seen::Json("epoch".into())).count(),
            small().epochs
        );
    }

    /// the terminal state always ships, and never twice: sampling is on a wall
    /// clock, so the last sample may or may not already be the final one
    #[test]
    fn the_final_snapshot_ships_exactly_once_however_the_clock_fell() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for epochs in [1usize, 4, 40] {
            let (seen, _) = runtime.block_on(collect(SimConfig { epochs, ..small() }));
            let snapshots: Vec<u32> = seen
                .iter()
                .filter_map(|s| match s {
                    Seen::Snapshot(e) => Some(*e),
                    _ => None,
                })
                .collect();

            assert_eq!(snapshots.first(), Some(&0));
            assert_eq!(
                snapshots.last(),
                Some(&(epochs as u32)),
                "{epochs} epochs: the terminal state did not ship"
            );
            assert!(
                snapshots.windows(2).all(|w| w[0] < w[1]),
                "{epochs} epochs: a snapshot repeated an epoch: {snapshots:?}"
            );
        }
    }

    /// backpressure's other half: when the browser goes, the simulation stops
    /// rather than filling a queue nobody is draining
    #[tokio::test]
    async fn the_producer_stops_when_the_browser_disconnects() {
        let (tx, rx) = mpsc::channel(QUEUE_DEPTH);
        drop(rx);
        let cfg = SimConfig { epochs: 10_000, population_per_species: 500, ..small() };
        let result = tokio::task::spawn_blocking(move || run(cfg, &tx)).await.unwrap();
        assert_eq!(result, Err(Stop::Gone));
    }

    #[tokio::test]
    async fn the_config_message_declares_the_protocol_version() {
        let (tx, mut rx) = mpsc::channel(QUEUE_DEPTH);
        let producer = tokio::task::spawn_blocking(move || run(small(), &tx));
        let first = rx.recv().await.unwrap();
        let Out::Text(json) = first else { panic!("config must be a text frame") };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["type"], "config");
        assert!(v["species"].as_array().unwrap().len() == 2);
        drop(rx);
        let _ = producer.await;
    }
}
