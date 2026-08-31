//! http + websocket front door for one shared, server-owned world.
//!
//! the browser never starts anything. one coordinator task owns the run and
//! market lifecycle, publishes to a hub, and keeps going with nobody watching;
//! a socket is a subscriber to that hub and cannot ask for a seed, a
//! population or an epoch count. accounts, darwin coins and the market live in
//! sqlite beside it.

mod auth;
mod coordinator;
mod hub;
mod routes;
mod store;
mod telemetry;
mod wire;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use ecosym_core::SimConfig;
use hub::Hub;
use serde_json::json;
use sqlx::SqlitePool;
use std::fs::File;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::error::RecvError;

const ADDR: &str = "127.0.0.1:3001";

/// the wire contract the browser decoder is written against
pub const PROTOCOL_VERSION: u16 = wire::VERSION;

/// credentials and bets are small. anything larger is not one of them.
const BODY_LIMIT: usize = 4_096;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub hub: Arc<Hub>,
    pub throttle: Arc<auth::Throttle>,
    /// off for local http development, on everywhere there is tls to be
    /// secure over. production must set `ECOSYM_SECURE_COOKIES=1`.
    pub secure_cookies: bool,
}

/// unix seconds. every deadline and expiry in the game is one of these, sent
/// absolutely so a browser's own clock cannot move a market phase.
pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tokio::main]
async fn main() {
    let path: PathBuf = std::env::var("ECOSYM_DB").unwrap_or_else(|_| "ecosym.db".into()).into();
    // held for the process lifetime: exactly one coordinator may own a
    // database, or two servers would run two worlds into one set of markets
    let _lock = exclusive_lock(&path);
    let db = store::open(&path).await.expect("open the database");

    let state = AppState {
        db,
        hub: Arc::new(Hub::default()),
        throttle: Arc::new(auth::Throttle::default()),
        secure_cookies: matches!(
            std::env::var("ECOSYM_SECURE_COOKIES").as_deref(),
            Ok("1") | Ok("true")
        ),
    };

    tokio::spawn(coordinator::run_forever(state.clone(), coordinator::Schedule::default()));

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/config", get(|| async { Json(SimConfig::default()) }))
        .route("/api/auth/register", post(routes::register))
        .route("/api/auth/login", post(routes::login))
        .route("/api/auth/logout", post(routes::logout))
        .route("/api/me", get(routes::me))
        .route("/api/market/current", get(routes::current_market))
        .route("/api/market/current/bet", put(routes::place_bet))
        .route("/api/market/form", get(routes::recent_form))
        .route("/ws", get(ws_handler))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(ADDR).await.expect("bind");
    println!("ecosym-server listening on http://{ADDR}  (ws://{ADDR}/ws), database {path:?}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("serve");
}

/// a second server pointed at the same database has to fail here rather than
/// start a second run against the same markets. multi-host replicas would need
/// a distributed coordinator; this is the single-process guard.
fn exclusive_lock(db: &std::path::Path) -> File {
    let path = db.with_extension("lock");
    let file =
        File::create(&path).unwrap_or_else(|e| stop(&format!("cannot create {path:?}: {e}")));
    if file.try_lock().is_err() {
        stop(&format!(
            "another ecosym-server already owns {}.\n  \
             two coordinators would run two worlds into one set of markets. stop\n  \
             the other one, or point this one somewhere else with ECOSYM_DB.",
            db.display()
        ));
    }
    file
}

/// a startup problem is the operator's to fix, so it leaves as a message
/// rather than as a panic and a backtrace
fn stop(why: &str) -> ! {
    eprintln!("ecosym-server: {why}");
    std::process::exit(1)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    // the socket is authenticated once, at connect. signing in or out reopens
    // it, which is also what refreshes which account it listens for.
    let account = routes::signed_in(&state, &headers).await.map(|a| a.id);
    ws.on_upgrade(move |socket| subscribe(socket, state, account)).into_response()
}

/// subscribe first, clone the retained bundle second, send it third, then drop
/// everything the bundle already contained. doing it in that order is what
/// makes joining mid-run safe.
async fn subscribe(mut socket: WebSocket, state: AppState, account: Option<i64>) {
    let mut live = state.hub.subscribe();
    let mut accounts = state.hub.subscribe_accounts();
    let Ok(mut floor) = bootstrap(&mut socket, &state).await else { return };

    loop {
        tokio::select! {
            item = live.recv() => match item {
                Ok(item) => {
                    if item.seq <= floor {
                        continue;
                    }
                    floor = item.seq;
                    if socket.send(item.message).await.is_err() {
                        return;
                    }
                }
                // a gap of unknown size. never continue from one: resend the
                // retained bundle instead, and never slow the coordinator for
                // this viewer.
                Err(RecvError::Lagged(_)) => match bootstrap(&mut socket, &state).await {
                    Ok(seq) => floor = seq,
                    Err(()) => return,
                },
                Err(RecvError::Closed) => return,
            },
            changed = accounts.recv() => {
                let revision = match changed {
                    Ok(changed) if Some(changed.account_id) == account => changed.revision,
                    Ok(_) => continue,
                    // a missed invalidation is still an invalidation. zero
                    // means "refetch, the revision is whatever you find".
                    Err(RecvError::Lagged(_)) if account.is_some() => 0,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let message = json!({ "type": "account_changed", "account_revision": revision });
                if socket.send(Message::Text(message.to_string().into())).await.is_err() {
                    return;
                }
            }
            // the browser has nothing to say. reading it is how a closed
            // socket is noticed.
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

/// `sync_begin`, the retained run, `sync_end`. the browser resets its
/// controller on the first and trusts nothing until the matching last.
async fn bootstrap(socket: &mut WebSocket, state: &AppState) -> Result<u64, ()> {
    let bundle = state.hub.bundle();
    let frame = |kind: &str| {
        Message::Text(
            json!({
                "type": kind,
                "run_id": bundle.run_id,
                "revision": bundle.seq,
                "server_time": now(),
            })
            .to_string()
            .into(),
        )
    };

    let mut send = vec![frame("sync_begin")];
    send.extend(bundle.bootstrap());
    send.push(frame("sync_end"));
    for message in send {
        socket.send(message).await.map_err(|_| ())?;
    }
    Ok(bundle.seq)
}
