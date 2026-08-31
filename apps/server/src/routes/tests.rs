use super::*;
use crate::auth::Throttle;
use crate::coordinator;
use crate::hub::Hub;
use axum::body::Body;
use axum::http::{Method, Request};
use axum::routing::{get, post, put};
use axum::Router;
use ecosym_game::MarketRules;
use std::sync::Arc;
use tower::ServiceExt;

const PASSWORD: &str = "a long enough password";
const ORIGIN: &str = "http://localhost:5173";
const HOST: &str = "localhost:5173";

async fn app() -> (Router, AppState) {
    let state = AppState {
        db: store::open_memory().await.expect("in-memory database"),
        hub: Arc::new(Hub::default()),
        throttle: Arc::new(Throttle::default()),
        secure_cookies: false,
    };
    let router = Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/market/current", get(current_market))
        .route("/api/market/current/bet", put(place_bet))
        .with_state(state.clone());
    (router, state)
}

struct Reply {
    status: StatusCode,
    body: serde_json::Value,
    cookie: Option<String>,
}

impl Reply {
    fn token(&self) -> Option<String> {
        let raw = self.cookie.as_ref()?;
        let value = raw.split(';').next()?.trim_start_matches("ecosym_session=");
        (!value.is_empty()).then(|| value.to_string())
    }
}

async fn send(
    router: &Router,
    method: Method,
    path: &str,
    session: Option<&str>,
    origin: Option<&str>,
    body: serde_json::Value,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", HOST)
        .header("content-type", "application/json");
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(token) = session {
        request = request.header("cookie", format!("ecosym_session={token}"));
    }
    let mut request = request.body(Body::from(body.to_string())).expect("request");
    request.extensions_mut().insert(ConnectInfo("10.0.0.1:5000".parse::<SocketAddr>().unwrap()));

    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.expect("body");
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    Reply { status, body, cookie }
}

async fn signup(router: &Router, username: &str) -> Reply {
    send(
        router,
        Method::POST,
        "/api/auth/register",
        None,
        Some(ORIGIN),
        serde_json::json!({ "username": username, "password": PASSWORD }),
    )
    .await
}

async fn open_market(state: &AppState) -> store::MarketRow {
    let now = crate::now();
    store::open_market(
        &state.db,
        store::NewRun { config_json: r#"{"seed":7}"#, seed: 7, nonce_hex: "0011", engine: "cpu" },
        |id| coordinator::commitment(id, r#"{"seed":7}"#, 7, "0011"),
        &MarketRules::V1,
        now,
        now + 30,
    )
    .await
    .expect("open market")
}

#[tokio::test]
async fn registering_signs_the_player_in_and_grants_the_opening_balance() {
    let (router, _) = app().await;
    let reply = signup(&router, "Darwin").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["username"], "Darwin");
    assert_eq!(reply.body["balance"], store::INITIAL_GRANT);
    assert_eq!(reply.body["escrow"], 0);

    let cookie = reply.cookie.clone().expect("a session cookie");
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(flag), "{cookie} is missing {flag}");
    }
    assert!(!cookie.contains("Secure"), "local http development has no tls");

    let me = send(&router, Method::GET, "/api/me", reply.token().as_deref(), None, json!({})).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["balance"], store::INITIAL_GRANT);
}

#[tokio::test]
async fn a_username_is_taken_whatever_case_it_was_claimed_in() {
    let (router, _) = app().await;
    assert_eq!(signup(&router, "Darwin").await.status, StatusCode::OK);
    let again = signup(&router, "DARWIN").await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["error"]["code"], "username_taken");
    assert!(again.cookie.is_none(), "a refused registration signed someone in");
}

#[tokio::test]
async fn a_bad_username_or_password_is_named_before_anything_is_written() {
    let (router, _) = app().await;
    let short = signup(&router, "ab").await;
    assert_eq!(
        (short.status, &short.body["error"]["code"]),
        (StatusCode::BAD_REQUEST, &"bad_username".into())
    );

    let weak = send(
        &router,
        Method::POST,
        "/api/auth/register",
        None,
        Some(ORIGIN),
        json!({ "username": "darwin", "password": "short" }),
    )
    .await;
    assert_eq!(weak.body["error"]["code"], "bad_password");
}

/// which half was wrong, and whether the account exists at all, must not be
/// readable from the response
#[tokio::test]
async fn login_fails_the_same_way_for_a_wrong_password_and_a_missing_account() {
    let (router, _) = app().await;
    signup(&router, "darwin").await;

    let attempt = |username: &'static str, password: &'static str| {
        let router = router.clone();
        async move {
            send(
                &router,
                Method::POST,
                "/api/auth/login",
                None,
                Some(ORIGIN),
                json!({ "username": username, "password": password }),
            )
            .await
        }
    };

    let wrong = attempt("darwin", "a different password").await;
    let missing = attempt("wallace", "a different password").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body, missing.body);
    assert!(wrong.cookie.is_none() && missing.cookie.is_none());

    let good = attempt("darwin", PASSWORD).await;
    assert_eq!(good.status, StatusCode::OK);
    assert!(good.token().is_some());
}

/// logging in again rotates this device's session and leaves other devices
/// signed in
#[tokio::test]
async fn logging_in_rotates_this_device_and_leaves_the_others_alone() {
    let (router, _) = app().await;
    let first = signup(&router, "darwin").await.token().unwrap();
    let second = send(
        &router,
        Method::POST,
        "/api/auth/login",
        None,
        Some(ORIGIN),
        json!({ "username": "darwin", "password": PASSWORD }),
    )
    .await;
    let second_token = second.token().unwrap();
    assert_ne!(first, second_token);

    for token in [&first, &second_token] {
        let me = send(&router, Method::GET, "/api/me", Some(token), None, json!({})).await;
        assert_eq!(me.status, StatusCode::OK, "session {token} was dropped");
    }

    // and logging in *from* the first device replaces only that one
    let third = send(
        &router,
        Method::POST,
        "/api/auth/login",
        Some(&first),
        Some(ORIGIN),
        json!({ "username": "darwin", "password": PASSWORD }),
    )
    .await;
    assert_eq!(
        send(&router, Method::GET, "/api/me", Some(&first), None, json!({})).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, Method::GET, "/api/me", third.token().as_deref(), None, json!({}))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn logout_expires_the_cookie_and_the_session_behind_it() {
    let (router, _) = app().await;
    let token = signup(&router, "darwin").await.token().unwrap();
    let out =
        send(&router, Method::POST, "/api/auth/logout", Some(&token), Some(ORIGIN), json!({}))
            .await;
    assert!(out.cookie.unwrap().contains("Max-Age=0"));
    assert_eq!(
        send(&router, Method::GET, "/api/me", Some(&token), None, json!({})).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_expired_session_is_not_a_session() {
    let (router, state) = app().await;
    let account =
        store::register(&state.db, "darwin", "darwin", "hash", crate::now()).await.unwrap();
    let stale = crate::now() - store::SESSION_LIFETIME - 1;
    store::create_session(&state.db, account.id, &crate::auth::token_hash("stale"), stale)
        .await
        .unwrap();
    assert_eq!(
        send(&router, Method::GET, "/api/me", Some("stale"), None, json!({})).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_mutating_request_from_another_origin_is_refused() {
    let (router, _) = app().await;
    for origin in [Some("https://evil.example"), None] {
        let reply = send(
            &router,
            Method::POST,
            "/api/auth/register",
            None,
            origin,
            json!({ "username": "darwin", "password": PASSWORD }),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{origin:?} got through");
        assert_eq!(reply.body["error"]["code"], "bad_origin");
    }
}

#[tokio::test]
async fn the_market_is_readable_signed_out_and_carries_your_bet_signed_in() {
    let (router, state) = app().await;
    let market = open_market(&state).await;

    let public = send(&router, Method::GET, "/api/market/current", None, None, json!({})).await;
    assert_eq!(public.status, StatusCode::OK);
    assert_eq!(public.body["market_id"], market.id);
    assert_eq!(public.body["phase"], "open");
    assert_eq!(public.body["pools"], json!([0, 0, 0]));
    assert!(public.body["bet"].is_null());
    assert!(public.body["seed_hex"].is_null(), "an open market published its seed");
    let species = public.body["species"].as_array().unwrap();
    assert_eq!(species.len(), 2);
    // the card reads the bodies off the market rather than off the run that
    // just ended, so an open market has to carry them
    assert_eq!(species[0]["genes"]["speed"], 1.3);

    let token = signup(&router, "darwin").await.token().unwrap();
    let bet = send(
        &router,
        Method::PUT,
        "/api/market/current/bet",
        Some(&token),
        Some(ORIGIN),
        json!({ "market_id": market.id, "outcome": "coexistence", "stake": 25 }),
    )
    .await;
    assert_eq!(bet.status, StatusCode::OK);
    assert_eq!(bet.body["account"]["balance"], store::INITIAL_GRANT - 25);
    assert_eq!(bet.body["account"]["escrow"], 25);
    assert_eq!(bet.body["market"]["pools"], json!([0, 25, 0]));

    let mine =
        send(&router, Method::GET, "/api/market/current", Some(&token), None, json!({})).await;
    assert_eq!(mine.body["bet"]["stake"], 25);
    assert_eq!(mine.body["bet"]["outcome"], "coexistence");
}

/// `PUT` means "make my bet exactly this", so the same request twice reserves
/// the stake once
#[tokio::test]
async fn repeating_a_bet_reserves_nothing_twice_and_a_late_one_is_refused() {
    let (router, state) = app().await;
    let market = open_market(&state).await;
    let token = signup(&router, "darwin").await.token().unwrap();
    let bet = |market_id: i64, stake: i64| {
        let router = router.clone();
        let token = token.clone();
        async move {
            send(
                &router,
                Method::PUT,
                "/api/market/current/bet",
                Some(&token),
                Some(ORIGIN),
                json!({ "market_id": market_id, "outcome": "species_a", "stake": stake }),
            )
            .await
        }
    };

    bet(market.id, 30).await;
    let twice = bet(market.id, 30).await;
    assert_eq!(twice.body["account"]["balance"], store::INITIAL_GRANT - 30);
    assert_eq!(twice.body["market"]["pools"], json!([30, 0, 0]));

    // a request naming a market that is no longer the current one
    let stale = bet(market.id + 99, 30).await;
    assert_eq!(stale.body["error"]["code"], "market_not_found");

    // and nothing lands once betting closes
    store::lock_market(&state.db, market.id, crate::now()).await.unwrap();
    let late = bet(market.id, 30).await;
    assert_eq!(late.status, StatusCode::CONFLICT);
    assert_eq!(late.body["error"]["code"], "market_not_open");
}

/// replacing a bet is an ordinary thing to do repeatedly, and several players
/// can share one address. neither may run into the credential throttle.
#[tokio::test]
async fn betting_is_not_rate_limited_the_way_credentials_are() {
    let (router, state) = app().await;
    let market = open_market(&state).await;
    let token = signup(&router, "darwin").await.token().unwrap();
    for stake in 1..=30 {
        let reply = send(
            &router,
            Method::PUT,
            "/api/market/current/bet",
            Some(&token),
            Some(ORIGIN),
            json!({ "market_id": market.id, "outcome": "species_a", "stake": stake }),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "replacement {stake} was refused");
    }
    assert_eq!(store::pools(&state.db, market.id).await.unwrap(), [30, 0, 0]);

    // credentials still are
    for attempt in 1..=12 {
        let reply = signup(&router, &format!("player{attempt}")).await;
        if reply.status == StatusCode::TOO_MANY_REQUESTS {
            return;
        }
    }
    panic!("registration was never throttled");
}

#[tokio::test]
async fn betting_needs_a_session_and_a_stake_within_the_rules() {
    let (router, state) = app().await;
    let market = open_market(&state).await;
    let anonymous = send(
        &router,
        Method::PUT,
        "/api/market/current/bet",
        None,
        Some(ORIGIN),
        json!({ "market_id": market.id, "outcome": "species_a", "stake": 10 }),
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let token = signup(&router, "darwin").await.token().unwrap();
    for stake in [0, -1, 101, 10_000] {
        let reply = send(
            &router,
            Method::PUT,
            "/api/market/current/bet",
            Some(&token),
            Some(ORIGIN),
            json!({ "market_id": market.id, "outcome": "species_a", "stake": stake }),
        )
        .await;
        assert_eq!(reply.body["error"]["code"], "stake_out_of_range", "stake {stake} was accepted");
    }
    assert_eq!(store::pools(&state.db, market.id).await.unwrap(), [0, 0, 0]);
}

/// nothing secret may leave through a response, whatever the route did
#[tokio::test]
async fn no_response_carries_a_secret() {
    let (router, state) = app().await;
    let market = open_market(&state).await;
    let token = signup(&router, "darwin").await.token().unwrap();
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM accounts")
        .fetch_one(&state.db)
        .await
        .unwrap();

    let replies = vec![
        send(&router, Method::GET, "/api/me", Some(&token), None, json!({})).await,
        send(&router, Method::GET, "/api/market/current", Some(&token), None, json!({})).await,
        send(
            &router,
            Method::PUT,
            "/api/market/current/bet",
            Some(&token),
            Some(ORIGIN),
            json!({ "market_id": market.id, "outcome": "species_a", "stake": 5 }),
        )
        .await,
        signup(&router, "darwin").await,
    ];

    for reply in &replies {
        let text = reply.body.to_string();
        for secret in [PASSWORD, hash.as_str(), "$argon2id$", "password_hash", "token_hash"] {
            assert!(!text.contains(secret), "a response leaked {secret}: {text}");
        }
        // the seed of an open market is the one that would let someone run the
        // world ahead of the bet
        assert!(!text.contains("\"seed_hex\":\"0x"), "an open market leaked its seed: {text}");
    }
}
