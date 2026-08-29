//! every sqlite transaction the game needs, and nothing else.
//!
//! two rules hold this file together. every change to a balance goes through
//! [`credit`], so an amount can never move without an append-only ledger entry
//! beside it; and `accounts.balance` is *available* balance, so an escrowed
//! stake has already left it and a payout brings the principal back with it.
//!
//! ponytail: the pool holds one connection, which serialises every query and
//! makes "lock the row" mean nothing more than "be inside the transaction".
//! split reads onto a second pool if viewer count ever makes that matter.

use ecosym_game::{settle, Coins, ContestResult, GameError, MarketOutcome, MarketRules, Wager};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::path::Path;

/// what registration puts in a new account
pub const INITIAL_GRANT: i64 = 1_000;
/// the anti-bankruptcy top-up, at most one a day
pub const RECOVERY_GRANT: i64 = 100;
pub const RECOVERY_INTERVAL: i64 = 24 * 60 * 60;
pub const SESSION_LIFETIME: i64 = 30 * 24 * 60 * 60;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug)]
pub enum StoreError {
    Db(sqlx::Error),
    Game(GameError),
    /// the persisted state does not allow what was asked for. these are the
    /// only failures a player is ever shown.
    Refused(Refusal),
}

/// every refusal a route can turn into a stable error code
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    UsernameTaken,
    AccountNotFound,
    MarketNotFound,
    MarketNotOpen,
    StakeOutOfRange,
    InsufficientBalance,
    RecoveryNotEligible,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::UsernameTaken => "username_taken",
            Refusal::AccountNotFound => "account_not_found",
            Refusal::MarketNotFound => "market_not_found",
            Refusal::MarketNotOpen => "market_not_open",
            Refusal::StakeOutOfRange => "stake_out_of_range",
            Refusal::InsufficientBalance => "insufficient_balance",
            Refusal::RecoveryNotEligible => "recovery_not_eligible",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Refusal::UsernameTaken => "that username is taken",
            Refusal::AccountNotFound => "no such account",
            Refusal::MarketNotFound => "that market is no longer the current one",
            Refusal::MarketNotOpen => "betting on this market has closed",
            Refusal::StakeOutOfRange => "stake outside the market limits",
            Refusal::InsufficientBalance => "not enough darwin coins",
            Refusal::RecoveryNotEligible => "no recovery grant available yet",
        }
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        StoreError::Db(e)
    }
}

impl From<GameError> for StoreError {
    fn from(e: GameError) -> Self {
        StoreError::Game(e)
    }
}

impl From<Refusal> for StoreError {
    fn from(r: Refusal) -> Self {
        StoreError::Refused(r)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "database: {e}"),
            StoreError::Game(e) => write!(f, "{e}"),
            StoreError::Refused(r) => f.write_str(r.message()),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MarketStatus {
    Open,
    Locked,
    Settled,
    Void,
}

impl MarketStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MarketStatus::Open => "open",
            MarketStatus::Locked => "locked",
            MarketStatus::Settled => "settled",
            MarketStatus::Void => "void",
        }
    }

    fn parse(s: &str) -> MarketStatus {
        match s {
            "open" => MarketStatus::Open,
            "locked" => MarketStatus::Locked,
            "settled" => MarketStatus::Settled,
            _ => MarketStatus::Void,
        }
    }

    /// a settled or void market never opens again
    pub fn is_terminal(self) -> bool {
        matches!(self, MarketStatus::Settled | MarketStatus::Void)
    }
}

/// an account as the server holds it. no password hash, so it cannot leak
/// through one.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub balance: i64,
    pub revision: i64,
    pub last_recovery_at: Option<i64>,
}

/// what `/api/me` is allowed to say
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AccountView {
    pub id: i64,
    pub username: String,
    pub balance: i64,
    pub escrow: i64,
    pub revision: i64,
    pub recovery_available: bool,
}

/// a market and its run, seed included. deliberately not `Serialize`: the
/// public shape is built by the coordinator, which knows whether the market
/// has locked and the seed may be revealed.
#[derive(Clone, Debug)]
pub struct MarketRow {
    pub id: i64,
    pub run_id: i64,
    pub revision: i64,
    pub status: MarketStatus,
    pub rules: MarketRules,
    pub opened_at: i64,
    pub locks_at: i64,
    pub winning_outcome: Option<MarketOutcome>,
    pub gross_pool: Option<i64>,
    pub burn: Option<i64>,
    pub commitment: String,
    pub run_status: String,
    pub digest: Option<String>,
    seed: u64,
    nonce_hex: String,
}

impl MarketRow {
    /// the reveal is gated on the market having locked, in code as well as in
    /// the transaction that locked it: a bettor must never be able to run the
    /// simulation ahead of the market.
    pub fn reveal(&self) -> Option<(u64, &str)> {
        match self.status {
            MarketStatus::Open => None,
            _ => Some((self.seed, &self.nonce_hex)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BetRow {
    pub id: i64,
    pub outcome: MarketOutcome,
    pub stake: i64,
    pub payout: Option<i64>,
}

/// what one market's settlement did, in the shape the wire wants
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettlementView {
    pub market_id: i64,
    pub status: &'static str,
    pub winning_outcome: Option<MarketOutcome>,
    pub gross_pool: i64,
    pub burn: i64,
    pub pools: [i64; 3],
}

/// the run a market is about to be opened for
pub struct NewRun<'a> {
    pub config_json: &'a str,
    pub seed: u64,
    pub nonce_hex: &'a str,
    pub engine: &'a str,
}

/// wal, foreign keys, and exactly one connection. migrations run here, so a
/// database is either current or the process does not start.
pub async fn open(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await.map_err(sqlx::Error::from)?;
    Ok(pool)
}

/// the same database, in memory, for tests
#[cfg(test)]
pub async fn open_memory() -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true))
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await.map_err(sqlx::Error::from)?;
    Ok(pool)
}

// ---- accounts and sessions -------------------------------------------------

/// the account, its opening balance and the ledger entry that explains it, or
/// none of the three.
pub async fn register(
    pool: &SqlitePool,
    username: &str,
    username_key: &str,
    password_hash: &str,
    now: i64,
) -> Result<Account> {
    let mut tx = pool.begin().await?;
    let taken: Option<i64> = sqlx::query_scalar("SELECT id FROM accounts WHERE username_key = ?")
        .bind(username_key)
        .fetch_optional(&mut *tx)
        .await?;
    if taken.is_some() {
        return Err(Refusal::UsernameTaken.into());
    }

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO accounts (username, username_key, password_hash, balance, created_at)
         VALUES (?, ?, ?, 0, ?) RETURNING id",
    )
    .bind(username)
    .bind(username_key)
    .bind(password_hash)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    credit(&mut tx, id, None, None, "initial_grant", INITIAL_GRANT, None, now).await?;
    let account = load_account(&mut tx, id).await?.expect("just inserted");
    tx.commit().await?;
    Ok(account)
}

/// the id and stored hash for a login attempt, or nothing. the caller must
/// still spend the same work on a missing account as on a wrong password.
pub async fn credentials(pool: &SqlitePool, username_key: &str) -> Result<Option<(i64, String)>> {
    let row = sqlx::query("SELECT id, password_hash FROM accounts WHERE username_key = ?")
        .bind(username_key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get("id"), r.get("password_hash"))))
}

pub async fn account(pool: &SqlitePool, id: i64) -> Result<Option<Account>> {
    let mut tx = pool.begin().await?;
    let account = load_account(&mut tx, id).await?;
    tx.commit().await?;
    Ok(account)
}

pub async fn account_view(pool: &SqlitePool, id: i64, now: i64) -> Result<Option<AccountView>> {
    let Some(account) = self::account(pool, id).await? else { return Ok(None) };
    let escrow = escrow(pool, id).await?;
    Ok(Some(view(&account, escrow, now)))
}

fn view(account: &Account, escrow: i64, now: i64) -> AccountView {
    AccountView {
        id: account.id,
        username: account.username.clone(),
        balance: account.balance,
        escrow,
        revision: account.revision,
        recovery_available: recovery_eligible(account, escrow, now),
    }
}

/// coins committed to markets that have neither settled nor voided
pub async fn escrow(pool: &SqlitePool, account_id: i64) -> Result<i64> {
    let total: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(b.stake) FROM bets b JOIN markets m ON m.id = b.market_id
         WHERE b.account_id = ? AND m.status IN ('open', 'locked')",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    Ok(total.unwrap_or(0))
}

pub async fn create_session(
    pool: &SqlitePool,
    account_id: i64,
    token_hash: &str,
    now: i64,
) -> Result<i64> {
    let expires_at = now + SESSION_LIFETIME;
    sqlx::query(
        "INSERT INTO sessions (token_hash, account_id, expires_at, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(account_id)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(expires_at)
}

/// an expired session is no session. it is also deleted on the way past, so
/// the table cannot grow without bound in a server that runs forever.
pub async fn session_account(
    pool: &SqlitePool,
    token_hash: &str,
    now: i64,
) -> Result<Option<Account>> {
    let row = sqlx::query("SELECT account_id, expires_at FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    if row.get::<i64, _>("expires_at") <= now {
        delete_session(pool, token_hash).await?;
        return Ok(None);
    }
    account(pool, row.get("account_id")).await
}

pub async fn delete_session(pool: &SqlitePool, token_hash: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(token_hash).execute(pool).await?;
    Ok(())
}

pub async fn purge_expired_sessions(pool: &SqlitePool, now: i64) -> Result<u64> {
    let done =
        sqlx::query("DELETE FROM sessions WHERE expires_at <= ?").bind(now).execute(pool).await?;
    Ok(done.rows_affected())
}

fn recovery_eligible(account: &Account, escrow: i64, now: i64) -> bool {
    account.balance < 1
        && escrow == 0
        && account.last_recovery_at.is_none_or(|at| now - at >= RECOVERY_INTERVAL)
}

/// the forever-running game's floor: an account that has lost everything gets
/// back in tomorrow. it is a server transaction, never a client-side clock,
/// and it is the one place a coin appears without a market.
pub async fn grant_recovery(pool: &SqlitePool, account_id: i64, now: i64) -> Result<AccountView> {
    let escrow = escrow(pool, account_id).await?;
    let mut tx = pool.begin().await?;
    let account = load_account(&mut tx, account_id).await?.ok_or(Refusal::AccountNotFound)?;
    if !recovery_eligible(&account, escrow, now) {
        return Err(Refusal::RecoveryNotEligible.into());
    }
    credit(&mut tx, account_id, None, None, "recovery_grant", RECOVERY_GRANT, None, now).await?;
    sqlx::query("UPDATE accounts SET last_recovery_at = ? WHERE id = ?")
        .bind(now)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    let account = load_account(&mut tx, account_id).await?.expect("still there");
    tx.commit().await?;
    Ok(view(&account, escrow, now))
}

// ---- runs, markets and bets ------------------------------------------------

/// the run row and its market are written together, before the first bet, and
/// only the commitment is public until the market locks.
///
/// `commit` is handed the run id the insert just allocated, because the id is
/// one of the things being committed to. its result is written inside this
/// same transaction, so a market can never be open without its commitment.
pub async fn open_market<F: Fn(i64) -> String>(
    pool: &SqlitePool,
    run: NewRun<'_>,
    commit: F,
    rules: &MarketRules,
    now: i64,
    locks_at: i64,
) -> Result<MarketRow> {
    let mut tx = pool.begin().await?;
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO runs (status, config, seed, nonce_hex, commitment, engine, created_at)
         VALUES ('pending', ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(run.config_json)
    .bind(run.seed as i64)
    .bind(run.nonce_hex)
    .bind("")
    .bind(run.engine)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("UPDATE runs SET commitment = ? WHERE id = ?")
        .bind(commit(run_id))
        .bind(run_id)
        .execute(&mut *tx)
        .await?;

    let market_id: i64 = sqlx::query_scalar(
        "INSERT INTO markets
           (run_id, status, rule_version, fee_bps, coexistence_margin,
            min_stake, max_stake, opened_at, locks_at)
         VALUES (?, 'open', ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(run_id)
    .bind(rules.version)
    .bind(rules.fee_bps)
    .bind(rules.coexistence_margin)
    .bind(rules.min_stake.get())
    .bind(rules.max_stake.get())
    .bind(now)
    .bind(locks_at)
    .fetch_one(&mut *tx)
    .await?;

    let market = load_market(&mut tx, market_id).await?.expect("just inserted");
    tx.commit().await?;
    Ok(market)
}

/// the newest market. run and market ids come from here and never from a
/// browser, so opening a second tab cannot start a second world.
pub async fn current_market(pool: &SqlitePool) -> Result<Option<MarketRow>> {
    let row = sqlx::query(&format!("{MARKET_SELECT} ORDER BY m.id DESC LIMIT 1"))
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(market_row).transpose()
}

pub async fn market(pool: &SqlitePool, id: i64) -> Result<Option<MarketRow>> {
    let mut tx = pool.begin().await?;
    let market = load_market(&mut tx, id).await?;
    tx.commit().await?;
    Ok(market)
}

pub async fn pools(pool: &SqlitePool, market_id: i64) -> Result<[i64; 3]> {
    let mut tx = pool.begin().await?;
    let totals = load_pools(&mut tx, market_id).await?;
    tx.commit().await?;
    Ok(totals)
}

/// how many accounts backed each outcome, in the same order as the pools.
/// the coins say how much is at stake; this says how many people are.
pub async fn bettors(pool: &SqlitePool, market_id: i64) -> Result<[i64; 3]> {
    let rows = sqlx::query(
        "SELECT outcome, COUNT(*) AS backers FROM bets WHERE market_id = ? GROUP BY outcome",
    )
    .bind(market_id)
    .fetch_all(pool)
    .await?;
    let mut counts = [0i64; 3];
    for row in &rows {
        if let Some(outcome) = MarketOutcome::parse(row.get("outcome")) {
            counts[outcome.index()] = row.get("backers");
        }
    }
    Ok(counts)
}

/// how one finished market ended. the whole public record of a run, and
/// deliberately nothing else: a bettor studies results, not seeds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FormRow {
    pub market_id: i64,
    /// `settled`, or `void` for a world where nothing survived
    pub status: &'static str,
    pub winning_outcome: Option<MarketOutcome>,
}

/// the last finished markets, newest first. every run draws its own seed, so
/// this is a sample of the distribution and never a tell about the next one.
pub async fn recent_form(pool: &SqlitePool, limit: i64) -> Result<Vec<FormRow>> {
    let rows = sqlx::query(
        "SELECT id, status, winning_outcome FROM markets
         WHERE status IN ('settled', 'void') ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| FormRow {
            market_id: r.get("id"),
            status: MarketStatus::parse(r.get("status")).as_str(),
            winning_outcome: r
                .get::<Option<String>, _>("winning_outcome")
                .as_deref()
                .and_then(MarketOutcome::parse),
        })
        .collect())
}

pub async fn bet_of(pool: &SqlitePool, market_id: i64, account_id: i64) -> Result<Option<BetRow>> {
    let mut tx = pool.begin().await?;
    let bet = load_bet(&mut tx, market_id, account_id).await?;
    tx.commit().await?;
    Ok(bet)
}

/// "make my bet exactly this": placing and replacing are the same call, and
/// only the difference in stake moves between balance and escrow. a repeat of
/// an identical request changes nothing and reserves nothing twice.
///
/// phase, stake limits and balance are all checked again in here. a disabled
/// button is not authorization.
pub async fn place_bet(
    pool: &SqlitePool,
    account_id: i64,
    market_id: i64,
    outcome: MarketOutcome,
    stake: i64,
    now: i64,
) -> Result<(BetRow, MarketRow, Account)> {
    let mut tx = pool.begin().await?;
    let market = load_market(&mut tx, market_id).await?.ok_or(Refusal::MarketNotFound)?;
    if market.status != MarketStatus::Open || now >= market.locks_at {
        return Err(Refusal::MarketNotOpen.into());
    }
    let stake = Coins::new(stake).map_err(|_| Refusal::StakeOutOfRange)?;
    market.rules.check_stake(stake).map_err(|_| Refusal::StakeOutOfRange)?;

    let existing = load_bet(&mut tx, market_id, account_id).await?;
    let held = existing.as_ref().map(|b| b.stake).unwrap_or(0);
    let difference = stake.get() - held;

    let bet_id =
        match &existing {
            Some(bet) => {
                sqlx::query("UPDATE bets SET outcome = ?, stake = ?, updated_at = ? WHERE id = ?")
                    .bind(outcome.as_str())
                    .bind(stake.get())
                    .bind(now)
                    .bind(bet.id)
                    .execute(&mut *tx)
                    .await?;
                bet.id
            }
            None => sqlx::query_scalar(
                "INSERT INTO bets (market_id, account_id, outcome, stake, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
            )
            .bind(market_id)
            .bind(account_id)
            .bind(outcome.as_str())
            .bind(stake.get())
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?,
        };

    match difference.cmp(&0) {
        std::cmp::Ordering::Greater => {
            credit(
                &mut tx,
                account_id,
                Some(market_id),
                Some(bet_id),
                "escrow",
                -difference,
                None,
                now,
            )
            .await?
        }
        std::cmp::Ordering::Less => {
            credit(
                &mut tx,
                account_id,
                Some(market_id),
                Some(bet_id),
                "escrow_release",
                -difference,
                None,
                now,
            )
            .await?
        }
        // only the selection moved, so no coin did. no balance change, no
        // ledger entry - but the account still has newer state to fetch.
        std::cmp::Ordering::Equal => bump_revision(&mut tx, account_id).await?,
    }

    sqlx::query("UPDATE markets SET revision = revision + 1 WHERE id = ?")
        .bind(market_id)
        .execute(&mut *tx)
        .await?;

    let bet = load_bet(&mut tx, market_id, account_id).await?.expect("just written");
    let market = load_market(&mut tx, market_id).await?.expect("still there");
    let account = load_account(&mut tx, account_id).await?.expect("still there");
    tx.commit().await?;
    Ok((bet, market, account))
}

/// the one-way door. only after this commits may the seed be revealed and the
/// simulation be built, which is what stops a bettor running the run ahead.
pub async fn lock_market(pool: &SqlitePool, market_id: i64, now: i64) -> Result<MarketRow> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE markets SET status = 'locked', locked_at = ?, revision = revision + 1
         WHERE id = ? AND status = 'open'",
    )
    .bind(now)
    .bind(market_id)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(Refusal::MarketNotOpen.into());
    }
    sqlx::query(
        "UPDATE runs SET status = 'running' WHERE id = (SELECT run_id FROM markets WHERE id = ?)",
    )
    .bind(market_id)
    .execute(&mut *tx)
    .await?;
    let market = load_market(&mut tx, market_id).await?.expect("just locked");
    tx.commit().await?;
    Ok(market)
}

/// the run is persisted complete before anything is paid out, so a settlement
/// can never come from a browser message or a render snapshot.
pub async fn complete_run(
    pool: &SqlitePool,
    run_id: i64,
    digest: &str,
    outcome_json: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE runs SET status = 'complete', digest = ?, outcome = ?, completed_at = ?
         WHERE id = ? AND status = 'running'",
    )
    .bind(digest)
    .bind(outcome_json)
    .bind(now)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// pari-mutuel settlement, in one transaction. a retry finds the market
/// already settled and returns the same view without paying twice.
pub async fn settle_market(
    pool: &SqlitePool,
    market_id: i64,
    contest: &ContestResult,
    now: i64,
) -> Result<SettlementView> {
    let mut tx = pool.begin().await?;
    let market = load_market(&mut tx, market_id).await?.ok_or(Refusal::MarketNotFound)?;
    if market.status.is_terminal() {
        let totals = load_pools(&mut tx, market_id).await?;
        tx.commit().await?;
        return Ok(settled_view(&market, totals));
    }
    if market.status != MarketStatus::Locked {
        return Err(Refusal::MarketNotOpen.into());
    }

    let bets = load_bets(&mut tx, market_id).await?;
    let wagers: Vec<Wager> = bets
        .iter()
        .map(|(id, _, outcome, stake)| {
            Ok(Wager { key: *id, outcome: *outcome, stake: Coins::new(*stake)? })
        })
        .collect::<std::result::Result<_, GameError>>()?;
    let settlement = settle(contest, &market.rules, &wagers)?;

    for ((bet_id, account_id, _, _), payout) in bets.iter().zip(&settlement.payouts) {
        sqlx::query("UPDATE bets SET payout = ? WHERE id = ?")
            .bind(payout.get())
            .bind(bet_id)
            .execute(&mut *tx)
            .await?;
        if payout.get() > 0 {
            credit(
                &mut tx,
                *account_id,
                Some(market_id),
                Some(*bet_id),
                "payout",
                payout.get(),
                Some(format!("settle:{market_id}:{bet_id}")),
                now,
            )
            .await?;
        }
    }

    sqlx::query(
        "UPDATE markets SET status = 'settled', winning_outcome = ?, gross_pool = ?, burn = ?,
                            settled_at = ?, revision = revision + 1
         WHERE id = ? AND status = 'locked'",
    )
    .bind(settlement.resolution.winner().map(|o| o.as_str()))
    .bind(settlement.gross.get())
    .bind(settlement.burn.get())
    .bind(now)
    .bind(market_id)
    .execute(&mut *tx)
    .await?;

    let market = load_market(&mut tx, market_id).await?.expect("just settled");
    let totals = load_pools(&mut tx, market_id).await?;
    tx.commit().await?;
    Ok(settled_view(&market, totals))
}

/// a market that cannot be settled gives every coin back. used both by total
/// extinction and by a restart that lost the simulation it was watching.
pub async fn void_market(pool: &SqlitePool, market_id: i64, now: i64) -> Result<SettlementView> {
    let mut tx = pool.begin().await?;
    let market = load_market(&mut tx, market_id).await?.ok_or(Refusal::MarketNotFound)?;
    if market.status.is_terminal() {
        let totals = load_pools(&mut tx, market_id).await?;
        tx.commit().await?;
        return Ok(settled_view(&market, totals));
    }

    let bets = load_bets(&mut tx, market_id).await?;
    let mut gross = 0i64;
    for (bet_id, account_id, _, stake) in &bets {
        gross += stake;
        sqlx::query("UPDATE bets SET payout = ? WHERE id = ?")
            .bind(stake)
            .bind(bet_id)
            .execute(&mut *tx)
            .await?;
        credit(
            &mut tx,
            *account_id,
            Some(market_id),
            Some(*bet_id),
            "refund",
            *stake,
            Some(format!("void:{market_id}:{bet_id}")),
            now,
        )
        .await?;
    }

    sqlx::query(
        "UPDATE markets SET status = 'void', gross_pool = ?, burn = 0, settled_at = ?,
                            revision = revision + 1
         WHERE id = ?",
    )
    .bind(gross)
    .bind(now)
    .bind(market_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE runs SET status = 'void' WHERE id = ? AND status IN ('pending', 'running')",
    )
    .bind(market.run_id)
    .execute(&mut *tx)
    .await?;

    let market = load_market(&mut tx, market_id).await?.expect("just voided");
    let totals = load_pools(&mut tx, market_id).await?;
    tx.commit().await?;
    Ok(settled_view(&market, totals))
}

/// every account the automatic recovery grant is due to. broke, nothing at
/// stake, and not topped up in the last day - the query is the eligibility
/// rule, so no client clock is involved in deciding it.
pub async fn grant_recovery_to_eligible(pool: &SqlitePool, now: i64) -> Result<Vec<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT a.id FROM accounts a
          WHERE a.balance < 1
            AND (a.last_recovery_at IS NULL OR ? - a.last_recovery_at >= ?)
            AND NOT EXISTS (
                SELECT 1 FROM bets b JOIN markets m ON m.id = b.market_id
                 WHERE b.account_id = a.id AND m.status IN ('open', 'locked'))
          ORDER BY a.id",
    )
    .bind(now)
    .bind(RECOVERY_INTERVAL)
    .fetch_all(pool)
    .await?;
    for id in &ids {
        grant_recovery(pool, *id, now).await?;
    }
    Ok(ids)
}

/// a restart cannot resume a simulation it never checkpointed, so every market
/// that was still live is voided and refunded before a new run starts. the
/// refunds carry idempotency keys, so running this twice pays nobody twice.
pub async fn recover_interrupted(pool: &SqlitePool, now: i64) -> Result<Vec<i64>> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM markets WHERE status IN ('open', 'locked') ORDER BY id")
            .fetch_all(pool)
            .await?;
    for id in &ids {
        void_market(pool, *id, now).await?;
    }
    Ok(ids)
}

/// everyone holding a bet in one market, with the revision their state now
/// carries. settlement uses it to tell each device its account moved.
pub async fn accounts_in_market(pool: &SqlitePool, market_id: i64) -> Result<Vec<(i64, i64)>> {
    let rows = sqlx::query(
        "SELECT a.id, a.revision FROM accounts a
          JOIN bets b ON b.account_id = a.id WHERE b.market_id = ? ORDER BY a.id",
    )
    .bind(market_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("id"), r.get("revision"))).collect())
}

/// every account's balance, recomputed from its ledger. the cached balance has
/// to agree with this at all times.
#[cfg(test)]
pub async fn ledger_balances(pool: &SqlitePool) -> Result<Vec<(i64, i64, i64)>> {
    let rows = sqlx::query(
        "SELECT a.id, a.balance, COALESCE(SUM(l.amount), 0) AS ledger
         FROM accounts a LEFT JOIN ledger_entries l ON l.account_id = a.id
         GROUP BY a.id ORDER BY a.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("id"), r.get("balance"), r.get("ledger"))).collect())
}

// ---- the pieces every transaction above is built from ----------------------

/// the only path a balance may change by. an amount never moves without the
/// ledger entry that explains it, and the account's revision moves with it.
#[allow(clippy::too_many_arguments)]
async fn credit(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    market_id: Option<i64>,
    bet_id: Option<i64>,
    kind: &str,
    amount: i64,
    idempotency_key: Option<String>,
    now: i64,
) -> Result<()> {
    let balance: i64 = sqlx::query_scalar("SELECT balance FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await?;
    let next = balance.checked_add(amount).ok_or(GameError::Overflow)?;
    if next < 0 {
        return Err(Refusal::InsufficientBalance.into());
    }

    sqlx::query("UPDATE accounts SET balance = ?, revision = revision + 1 WHERE id = ?")
        .bind(next)
        .bind(account_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO ledger_entries
           (account_id, market_id, bet_id, kind, amount, idempotency_key, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(account_id)
    .bind(market_id)
    .bind(bet_id)
    .bind(kind)
    .bind(amount)
    .bind(idempotency_key)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn bump_revision(tx: &mut Transaction<'_, Sqlite>, account_id: i64) -> Result<()> {
    sqlx::query("UPDATE accounts SET revision = revision + 1 WHERE id = ?")
        .bind(account_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn load_account(tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<Option<Account>> {
    let row = sqlx::query(
        "SELECT id, username, balance, revision, last_recovery_at FROM accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|r| Account {
        id: r.get("id"),
        username: r.get("username"),
        balance: r.get("balance"),
        revision: r.get("revision"),
        last_recovery_at: r.get("last_recovery_at"),
    }))
}

const MARKET_SELECT: &str = "SELECT m.id, m.run_id, m.revision, m.status, m.rule_version,
        m.fee_bps, m.coexistence_margin, m.min_stake, m.max_stake, m.opened_at, m.locks_at,
        m.winning_outcome, m.gross_pool, m.burn,
        r.commitment, r.status AS run_status, r.digest, r.seed, r.nonce_hex
   FROM markets m JOIN runs r ON r.id = m.run_id";

async fn load_market(tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<Option<MarketRow>> {
    let row = sqlx::query(&format!("{MARKET_SELECT} WHERE m.id = ?"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref().map(market_row).transpose()
}

fn market_row(r: &SqliteRow) -> Result<MarketRow> {
    Ok(MarketRow {
        id: r.get("id"),
        run_id: r.get("run_id"),
        revision: r.get("revision"),
        status: MarketStatus::parse(r.get("status")),
        rules: MarketRules {
            version: r.get::<i64, _>("rule_version") as u32,
            fee_bps: r.get::<i64, _>("fee_bps") as u32,
            coexistence_margin: r.get("coexistence_margin"),
            min_stake: Coins::new(r.get("min_stake"))?,
            max_stake: Coins::new(r.get("max_stake"))?,
        },
        opened_at: r.get("opened_at"),
        locks_at: r.get("locks_at"),
        winning_outcome: r
            .get::<Option<String>, _>("winning_outcome")
            .as_deref()
            .and_then(MarketOutcome::parse),
        gross_pool: r.get("gross_pool"),
        burn: r.get("burn"),
        commitment: r.get("commitment"),
        run_status: r.get("run_status"),
        digest: r.get("digest"),
        seed: r.get::<i64, _>("seed") as u64,
        nonce_hex: r.get("nonce_hex"),
    })
}

async fn load_pools(tx: &mut Transaction<'_, Sqlite>, market_id: i64) -> Result<[i64; 3]> {
    let rows = sqlx::query(
        "SELECT outcome, SUM(stake) AS total FROM bets WHERE market_id = ? GROUP BY outcome",
    )
    .bind(market_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut totals = [0i64; 3];
    for row in &rows {
        if let Some(outcome) = MarketOutcome::parse(row.get("outcome")) {
            totals[outcome.index()] = row.get("total");
        }
    }
    Ok(totals)
}

async fn load_bet(
    tx: &mut Transaction<'_, Sqlite>,
    market_id: i64,
    account_id: i64,
) -> Result<Option<BetRow>> {
    let row = sqlx::query(
        "SELECT id, outcome, stake, payout FROM bets WHERE market_id = ? AND account_id = ?",
    )
    .bind(market_id)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.and_then(|r| {
        Some(BetRow {
            id: r.get("id"),
            outcome: MarketOutcome::parse(r.get("outcome"))?,
            stake: r.get("stake"),
            payout: r.get("payout"),
        })
    }))
}

/// ordered by id so settlement reads bets in one fixed order
async fn load_bets(
    tx: &mut Transaction<'_, Sqlite>,
    market_id: i64,
) -> Result<Vec<(i64, i64, MarketOutcome, i64)>> {
    let rows = sqlx::query(
        "SELECT id, account_id, outcome, stake FROM bets WHERE market_id = ? ORDER BY id",
    )
    .bind(market_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some((
                r.get("id"),
                r.get("account_id"),
                MarketOutcome::parse(r.get("outcome"))?,
                r.get("stake"),
            ))
        })
        .collect())
}

fn settled_view(market: &MarketRow, pools: [i64; 3]) -> SettlementView {
    SettlementView {
        market_id: market.id,
        status: market.status.as_str(),
        winning_outcome: market.winning_outcome,
        gross_pool: market.gross_pool.unwrap_or(0),
        burn: market.burn.unwrap_or(0),
        pools,
    }
}

#[cfg(test)]
mod tests;
