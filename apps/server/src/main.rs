//! http + websocket front door. streams one message per epoch.

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
use ecosym_simulation::Simulation;
use serde::Deserialize;

const ADDR: &str = "127.0.0.1:3001";

#[derive(Deserialize)]
struct Params {
    seed: Option<u64>,
    population_per_species: Option<usize>,
    epochs: Option<usize>,
}

impl Params {
    fn config(&self) -> SimConfig {
        let d = SimConfig::default();
        SimConfig {
            seed: self.seed.unwrap_or(d.seed),
            population_per_species: self
                .population_per_species
                .unwrap_or(d.population_per_species)
                .clamp(1, 100_000),
            epochs: self.epochs.unwrap_or(d.epochs).clamp(1, 10_000),
            ..d
        }
    }
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

/// ponytail: the epoch loop runs inline on the socket's task. it is cpu-bound,
/// so one heavy client occupies one tokio worker. move it to spawn_blocking
/// with a channel when more than a handful of clients connect at once.
async fn stream_run(mut socket: WebSocket, cfg: SimConfig) {
    let mut sim = Simulation::cpu(cfg.clone());
    let mut rec = Recorder::new(cfg.clone(), sim.engine_id());

    let world = sim.state.world.summary();
    let start = serde_json::json!({
        "type": "config",
        "config": cfg,
        "engine": sim.engine_id(),
        "world": {
            "width": world.width,
            "height": world.height,
            "habitable_tiles": world.habitable_tiles,
            "initial_biomass": world.initial_biomass,
            "mean_temperature": world.mean_temperature,
        },
        // stable order: the client indexes species by position, never by a map key
        "species": sim.state.species.iter().map(|s| serde_json::json!({
            "id": s.id().get(),
            "name": s.name(),
            "founder_genes": s.founder_genes(),
        })).collect::<Vec<_>>(),
    });
    if send(&mut socket, &start).await.is_err() {
        return;
    }

    for _ in 0..cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        let extinct = report.population == 0;
        let message = serde_json::json!({ "type": "epoch", "report": report });
        rec.push(report);
        if send(&mut socket, &message).await.is_err() {
            return;
        }
        if extinct {
            break;
        }
    }

    let done = serde_json::json!({
        "type": "done",
        "digest": rec.digest_hex(),
        "epochs": rec.epochs(),
        "outcome": sim.outcome(),
    });
    let _ = send(&mut socket, &done).await;
}

async fn send(socket: &mut WebSocket, v: &serde_json::Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(v.to_string().into())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_simulation::Winner;

    #[test]
    fn params_map_onto_the_epoch_config() {
        let p = Params { seed: Some(7), population_per_species: Some(9), epochs: Some(3) };
        let cfg = p.config();
        assert_eq!((cfg.seed, cfg.population_per_species, cfg.epochs), (7, 9, 3));

        let empty = Params { seed: None, population_per_species: None, epochs: None };
        let d = SimConfig::default();
        assert_eq!(empty.config().population_per_species, d.population_per_species);
        assert_eq!(empty.config().epochs, d.epochs);
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
}
