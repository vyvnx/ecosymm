//! the http surface: accounts, sessions and one market.
//!
//! every failure leaves through [`error`], so a route can only ever return a
//! machine-readable code and a message safe to show a player. password hashes,
//! session hashes, unrevealed seeds and database text never reach a response.

use crate::auth::{self, Session};
use crate::coordinator::{self, MarketView};
use crate::store::{self, AccountView, BetRow, Refusal, StoreError};
use crate::{now, AppState};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ecosym_game::MarketOutcome;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct BetRequest {
    /// which market this bet is for. a request that arrives after the next
    /// market opened is rejected rather than applied to the wrong one.
    market_id: i64,
    outcome: MarketOutcome,
    stake: i64,
}

#[derive(Serialize)]
pub struct MarketResponse {
    #[serde(flatten)]
    market: MarketView,
    /// the requesting account's bet, when it has one and is signed in
    bet: Option<BetRow>,
}

#[derive(Serialize)]
pub struct BetResponse {
    account: AccountView,
    bet: BetRow,
    market: MarketView,
}

pub fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({ "error": { "code": code, "message": message } }))).into_response()
}

/// the only failure a player is shown for anything credential-shaped. which
/// half was wrong, and whether the account exists at all, stays here.
fn bad_credentials() -> Response {
    error(StatusCode::UNAUTHORIZED, "bad_credentials", "wrong username or password")
}

fn refused(e: StoreError) -> Response {
    match e {
        StoreError::Refused(r) => error(StatusCode::CONFLICT, r.code(), r.message()),
        // a database or arithmetic failure is ours, and its text is not the
        // player's business
        other => {
            eprintln!("store error: {other}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "something went wrong")
        }
    }
}

/// every mutating route needs the expected origin. the same-site cookie is the
/// first line and this is the second.
fn guard(headers: &HeaderMap) -> Option<Response> {
    (!auth::same_origin(headers))
        .then(|| error(StatusCode::FORBIDDEN, "bad_origin", "request came from another origin"))
}

/// credential routes need a speed bump on top, because those are the ones
/// worth guessing at. betting is not throttled: it is already bounded by a
/// session, a stake cap and a balance, and one player behind a shared address
/// must not cost another their replacements.
fn throttled(state: &AppState, who: SocketAddr) -> Option<Response> {
    (!state.throttle.allow(&who.ip().to_string(), now()))
        .then(|| error(StatusCode::TOO_MANY_REQUESTS, "slow_down", "too many attempts"))
}

fn with_cookie(cookie: String, body: impl Serialize) -> Response {
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::to_value(body).expect("response serialises")),
    )
        .into_response()
}

async fn sign_in(state: &AppState, account_id: i64) -> Result<String, StoreError> {
    let token = auth::new_token();
    store::create_session(&state.db, account_id, &auth::token_hash(&token), now()).await?;
    Ok(auth::set_cookie(&token, store::SESSION_LIFETIME, state.secure_cookies))
}

pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    if let Some(stop) = guard(&headers).or_else(|| throttled(&state, who)) {
        return stop;
    }
    let (username, key) = match auth::validate_username(&body.username) {
        Ok(pair) => pair,
        Err(why) => return error(StatusCode::BAD_REQUEST, "bad_username", why),
    };
    if let Err(why) = auth::validate_password(&body.password) {
        return error(StatusCode::BAD_REQUEST, "bad_password", why);
    }
    let Ok(hash) = auth::hash_password(&body.password) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "could not create account",
        );
    };

    let account = match store::register(&state.db, &username, &key, &hash, now()).await {
        Ok(account) => account,
        Err(e) => return refused(e),
    };
    match sign_in(&state, account.id).await {
        Ok(cookie) => match store::account_view(&state.db, account.id, now()).await {
            Ok(Some(view)) => with_cookie(cookie, view),
            other => refused(other.err().unwrap_or(StoreError::Refused(Refusal::AccountNotFound))),
        },
        Err(e) => refused(e),
    }
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Response {
    if let Some(stop) = guard(&headers).or_else(|| throttled(&state, who)) {
        return stop;
    }
    let key = body.username.to_ascii_lowercase();
    let found = match store::credentials(&state.db, &key).await {
        Ok(found) => found,
        Err(e) => return refused(e),
    };

    let Some((account_id, hash)) = found else {
        // spend the same work on a username that does not exist, so the clock
        // does not answer a question the response refuses to
        auth::verify_password(&auth::DUMMY_HASH, &body.password);
        return bad_credentials();
    };
    if !auth::verify_password(&hash, &body.password) {
        return bad_credentials();
    }

    // rotation: this device's old session goes, other devices keep theirs
    if let Some(token) = auth::cookie_token(&headers) {
        let _ = store::delete_session(&state.db, &auth::token_hash(&token)).await;
    }
    let _ = store::purge_expired_sessions(&state.db, now()).await;

    match sign_in(&state, account_id).await {
        Ok(cookie) => match store::account_view(&state.db, account_id, now()).await {
            Ok(Some(view)) => with_cookie(cookie, view),
            other => refused(other.err().unwrap_or(StoreError::Refused(Refusal::AccountNotFound))),
        },
        Err(e) => refused(e),
    }
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = auth::cookie_token(&headers) {
        let _ = store::delete_session(&state.db, &auth::token_hash(&token)).await;
    }
    (
        StatusCode::OK,
        [(header::SET_COOKIE, auth::clear_cookie(state.secure_cookies))],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

pub async fn me(State(state): State<AppState>, Session(account): Session) -> Response {
    match store::account_view(&state.db, account.id, now()).await {
        Ok(Some(view)) => Json(view).into_response(),
        other => refused(other.err().unwrap_or(StoreError::Refused(Refusal::AccountNotFound))),
    }
}

/// the current market, plus the caller's bet on it when they have one. it is
/// readable signed out, because watching does not need an account.
pub async fn current_market(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let market = match store::current_market(&state.db).await {
        Ok(Some(market)) => market,
        Ok(None) => {
            return error(StatusCode::SERVICE_UNAVAILABLE, "no_market", "no market has opened yet")
        }
        Err(e) => return refused(e),
    };
    let pools = match store::pools(&state.db, market.id).await {
        Ok(pools) => pools,
        Err(e) => return refused(e),
    };

    let bettors = match store::bettors(&state.db, market.id).await {
        Ok(bettors) => bettors,
        Err(e) => return refused(e),
    };

    let bet = match signed_in(&state, &headers).await {
        Some(account) => store::bet_of(&state.db, market.id, account.id).await.ok().flatten(),
        None => None,
    };
    Json(MarketResponse { market: coordinator::view(&market, pools, bettors, now()), bet })
        .into_response()
}

/// how many finished markets the form guide reads back. long enough to show a
/// streak, short enough to stay one line of dots.
const FORM_LENGTH: i64 = 12;

/// how the last markets ended. readable signed out and identical for everyone:
/// it is the record, not an edge, and a fresh seed each run keeps it that way.
pub async fn recent_form(State(state): State<AppState>) -> Response {
    match store::recent_form(&state.db, FORM_LENGTH).await {
        Ok(form) => Json(form).into_response(),
        Err(e) => refused(e),
    }
}

/// "make my bet exactly this". a repeat of the same request reserves nothing
/// twice, and the market id makes a late request fail rather than land on the
/// next market.
pub async fn place_bet(
    State(state): State<AppState>,
    Session(account): Session,
    headers: HeaderMap,
    Json(body): Json<BetRequest>,
) -> Response {
    if let Some(stop) = guard(&headers) {
        return stop;
    }
    let placed =
        store::place_bet(&state.db, account.id, body.market_id, body.outcome, body.stake, now())
            .await;
    let (bet, _, account) = match placed {
        Ok(placed) => placed,
        Err(e) => return refused(e),
    };

    // the pools moved, so every viewer needs the new market and every device
    // signed into this account needs to refetch it
    let market = match coordinator::republish(&state, body.market_id, "market_pool").await {
        Ok(view) => view,
        Err(e) => return refused(e),
    };
    state.hub.account_changed(account.id, account.revision);

    match store::account_view(&state.db, account.id, now()).await {
        Ok(Some(view)) => Json(BetResponse { account: view, bet, market }).into_response(),
        other => refused(other.err().unwrap_or(StoreError::Refused(Refusal::AccountNotFound))),
    }
}

/// the session behind a request, when there is one. readable routes use this
/// instead of the extractor so that signing out does not make them fail.
pub async fn signed_in(state: &AppState, headers: &HeaderMap) -> Option<store::Account> {
    let token = auth::cookie_token(headers)?;
    store::session_account(&state.db, &auth::token_hash(&token), now()).await.ok().flatten()
}

#[cfg(test)]
mod tests;
