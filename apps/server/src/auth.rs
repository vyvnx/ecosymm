//! credentials, password hashing, and opaque server-side sessions.
//!
//! the raw session token exists in exactly two places: the player's cookie and
//! the response that set it. what is stored is its sha-256, so a stolen
//! database is not a stolen set of logins. passwords are argon2id over an
//! os-random salt and are never trimmed, normalised or logged.

use crate::AppState;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub const COOKIE: &str = "ecosym_session";

/// one place for the cost of a password. owasp's argon2id floor: 19 mib, two
/// passes, one lane.
fn hasher() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("valid argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// bounded attempts per client address per window. it is a speed bump, not a
/// sybil defence, and it is deliberately not keyed on any forwarded header.
const THROTTLE_WINDOW: i64 = 60;
const THROTTLE_ATTEMPTS: u32 = 10;
const THROTTLE_CAPACITY: usize = 4_096;

/// os randomness. the only randomness on the server that is not the
/// simulation's own deterministic stream.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).expect("the operating system has randomness");
    bytes
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 256 opaque bits. it is not derived from the account, so it says nothing
/// about who holds it.
pub fn new_token() -> String {
    hex(&random_bytes::<32>())
}

pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// 3-24 characters of ascii letters, digits, `_` or `-`. returns the spelling
/// to show and the lowercase key uniqueness is decided on.
pub fn validate_username(raw: &str) -> Result<(String, String), &'static str> {
    if !(3..=24).contains(&raw.chars().count()) {
        return Err("username must be 3 to 24 characters");
    }
    if !raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err("username may use letters, digits, _ and - only");
    }
    Ok((raw.to_string(), raw.to_ascii_lowercase()))
}

/// bytes, not characters, and never rewritten: trimming or normalising a
/// password silently changes what the player typed.
pub fn validate_password(raw: &str) -> Result<(), &'static str> {
    if !(12..=128).contains(&raw.len()) {
        return Err("password must be 12 to 128 bytes");
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::encode_b64(&random_bytes::<16>())?;
    Ok(hasher().hash_password(password.as_bytes(), &salt)?.to_string())
}

/// something real to verify against when the username does not exist. a login
/// for a missing account then costs the same time as one for a real account,
/// so the clock cannot answer a question the response refuses to.
pub static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("no account holds this password").expect("argon2 can hash"));

pub fn verify_password(stored: &str, password: &str) -> bool {
    PasswordHash::new(stored)
        .map(|hash| hasher().verify_password(password.as_bytes(), &hash).is_ok())
        .unwrap_or(false)
}

/// the cookie the token travels in and nothing else does. `HttpOnly` keeps it
/// out of javascript, `SameSite=Lax` keeps it off cross-site posts, and
/// `Secure` is on wherever the deployment is not plain local http.
pub fn set_cookie(token: &str, max_age: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure}")
}

pub fn clear_cookie(secure: bool) -> String {
    set_cookie("", 0, secure)
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.is_empty())
}

/// a mutating request has to come from this origin. the same-site cookie is
/// the first line; this is the second, because a disabled button and a
/// well-behaved browser are not authorization.
pub fn same_origin(headers: &HeaderMap) -> bool {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(origin) = text(header::ORIGIN) else { return false };
    let Some(host) = text(header::HOST) else { return false };
    origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://")) == Some(host)
}

/// attempts per client address, swept on every check so a server that runs
/// forever cannot accumulate them.
///
/// ponytail: a full table resets rather than refusing, so nobody can fill it
/// to lock everyone else out. swap for a proper sliding window if the shape
/// of real abuse ever needs one.
#[derive(Default)]
pub struct Throttle {
    seen: Mutex<HashMap<String, (i64, u32)>>,
}

impl Throttle {
    /// true when the attempt is allowed
    pub fn allow(&self, key: &str, now: i64) -> bool {
        let mut seen = self.seen.lock().expect("throttle mutex");
        seen.retain(|_, (started, _)| now - *started < THROTTLE_WINDOW);
        if seen.len() >= THROTTLE_CAPACITY {
            seen.clear();
        }
        let entry = seen.entry(key.to_string()).or_insert((now, 0));
        entry.1 += 1;
        entry.1 <= THROTTLE_ATTEMPTS
    }
}

/// an authenticated account, extracted from the session cookie
pub struct Session(pub crate::store::Account);

impl FromRequestParts<AppState> for Session {
    type Rejection = (StatusCode, axum::Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthenticated = || {
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": { "code": "unauthenticated", "message": "sign in to do that" }
                })),
            )
        };
        let token = cookie_token(&parts.headers).ok_or_else(unauthenticated)?;
        let account = crate::store::session_account(&state.db, &token_hash(&token), crate::now())
            .await
            .map_err(|_| unauthenticated())?
            .ok_or_else(unauthenticated)?;
        Ok(Session(account))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_keeps_its_spelling_and_is_unique_on_its_lowercase() {
        assert_eq!(validate_username("Darwin"), Ok(("Darwin".into(), "darwin".into())));
        assert_eq!(validate_username("a_b-9"), Ok(("a_b-9".into(), "a_b-9".into())));
        // the display spelling survives, so two people cannot both be "darwin"
        assert_eq!(validate_username("DARWIN").unwrap().1, validate_username("darwin").unwrap().1);

        for bad in ["ab", &"x".repeat(25), "has space", "emoji🙂", "semi;colon", ""] {
            assert!(validate_username(bad).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_password_is_measured_in_bytes_and_never_rewritten() {
        assert!(validate_password("twelve chars").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"x".repeat(129)).is_err());
        assert!(validate_password(&"x".repeat(128)).is_ok());
        // four bytes each, so three of them are under the floor on bytes even
        // though they look like more than twelve characters
        assert!(validate_password("🙂🙂🙂").is_ok());

        // whitespace is part of the secret
        let hash = hash_password("  padded secret  ").unwrap();
        assert!(verify_password(&hash, "  padded secret  "));
        assert!(!verify_password(&hash, "padded secret"));
    }

    #[test]
    fn a_password_hash_is_argon2id_over_a_fresh_salt() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$"), "{hash}");
        assert!(!hash.contains("correct horse battery"));
        assert!(verify_password(&hash, "correct horse battery"));
        assert!(!verify_password(&hash, "Correct horse battery"));
        assert!(!verify_password("not a hash at all", "correct horse battery"));

        // the same password twice must not produce the same stored hash
        assert_ne!(hash, hash_password("correct horse battery").unwrap());
    }

    #[test]
    fn a_missing_account_is_still_verified_against_something() {
        assert!(DUMMY_HASH.starts_with("$argon2id$"));
        assert!(!verify_password(&DUMMY_HASH, "no account holds this password's neighbour"));
    }

    #[test]
    fn a_session_token_is_opaque_and_only_its_hash_is_storable() {
        let token = new_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token, new_token());

        let hash = token_hash(&token);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, token, "the raw token must never be what is stored");
        assert_eq!(hash, token_hash(&token));
    }

    #[test]
    fn the_cookie_carries_the_flags_that_keep_the_token_out_of_reach() {
        let set = set_cookie("abc", 100, true);
        for flag in ["ecosym_session=abc", "HttpOnly", "SameSite=Lax", "Path=/", "Secure"] {
            assert!(set.contains(flag), "{set} is missing {flag}");
        }
        // local http development has no tls to be secure over
        assert!(!set_cookie("abc", 100, false).contains("Secure"));
        assert!(clear_cookie(true).contains("Max-Age=0"));
    }

    #[test]
    fn the_token_is_read_back_out_of_a_cookie_header() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::COOKIE, value.parse().unwrap());
            cookie_token(&headers)
        };
        assert_eq!(with("ecosym_session=abc"), Some("abc".into()));
        assert_eq!(with("other=1; ecosym_session=abc; more=2"), Some("abc".into()));
        assert_eq!(with("ecosym_session="), None);
        assert_eq!(with("other=1"), None);
        assert_eq!(cookie_token(&HeaderMap::new()), None);
    }

    #[test]
    fn a_mutating_request_has_to_come_from_this_origin() {
        let headers = |origin: Option<&str>, host: Option<&str>| {
            let mut h = HeaderMap::new();
            if let Some(o) = origin {
                h.insert(header::ORIGIN, o.parse().unwrap());
            }
            if let Some(host) = host {
                h.insert(header::HOST, host.parse().unwrap());
            }
            h
        };
        assert!(same_origin(&headers(Some("http://localhost:5173"), Some("localhost:5173"))));
        assert!(same_origin(&headers(Some("https://ecosym.example"), Some("ecosym.example"))));
        assert!(!same_origin(&headers(Some("https://evil.example"), Some("ecosym.example"))));
        // a port is part of an origin
        assert!(!same_origin(&headers(Some("http://localhost:5173"), Some("localhost:3001"))));
        assert!(!same_origin(&headers(None, Some("localhost:5173"))));
        assert!(!same_origin(&headers(Some("http://localhost:5173"), None)));
    }

    #[test]
    fn the_throttle_is_bounded_and_forgets_old_windows() {
        let throttle = Throttle::default();
        for attempt in 1..=THROTTLE_ATTEMPTS {
            assert!(throttle.allow("10.0.0.1", 0), "attempt {attempt} was refused");
        }
        assert!(!throttle.allow("10.0.0.1", 0), "the window never closed");
        // a different client is unaffected, and the window expires
        assert!(throttle.allow("10.0.0.2", 0));
        assert!(throttle.allow("10.0.0.1", THROTTLE_WINDOW));

        for i in 0..THROTTLE_CAPACITY * 2 {
            throttle.allow(&format!("10.1.{}.{}", i / 256, i % 256), 0);
        }
        assert!(
            throttle.seen.lock().unwrap().len() <= THROTTLE_CAPACITY,
            "the throttle table grew without bound"
        );
    }
}
