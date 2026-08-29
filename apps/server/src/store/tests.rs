use super::*;
use ecosym_core::Rng;
use ecosym_game::{MarketOutcome::*, Resolution, SpeciesTally};

const NOW: i64 = 1_800_000_000;

async fn db() -> SqlitePool {
    open_memory().await.expect("in-memory database")
}

async fn player(pool: &SqlitePool, name: &str) -> Account {
    register(pool, name, &name.to_ascii_lowercase(), "hash", NOW).await.expect("register")
}

async fn market(pool: &SqlitePool, now: i64) -> MarketRow {
    let run = NewRun {
        config_json: r#"{"seed":7}"#,
        seed: 7,
        nonce_hex: "00112233445566778899aabbccddeeff",
        engine: "cpu",
    };
    open_market(pool, run, |id| format!("commitment-{id}"), &MarketRules::V1, now, now + 30)
        .await
        .expect("open market")
}

fn contest(a: u64, b: u64) -> ContestResult {
    ContestResult::new(
        SpeciesTally { id: 0, initial: 500, final_population: a },
        SpeciesTally { id: 1, initial: 500, final_population: b },
    )
    .expect("founded species")
}

async fn ledger_of(pool: &SqlitePool, account_id: i64) -> Vec<(String, i64)> {
    sqlx::query("SELECT kind, amount FROM ledger_entries WHERE account_id = ? ORDER BY id")
        .bind(account_id)
        .fetch_all(pool)
        .await
        .expect("ledger")
        .iter()
        .map(|r| (r.get("kind"), r.get("amount")))
        .collect()
}

/// the cached balance is a cache. it has to equal the ledger, always.
async fn assert_ledger_explains_every_balance(pool: &SqlitePool) {
    for (id, cached, ledger) in ledger_balances(pool).await.expect("balances") {
        assert_eq!(cached, ledger, "account {id}: cached {cached}, ledger says {ledger}");
        assert!(cached >= 0, "account {id} went negative");
    }
}

async fn run_status(pool: &SqlitePool, run_id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn registration_grants_a_balance_and_the_entry_that_explains_it() {
    let pool = db().await;
    let account = player(&pool, "Darwin").await;
    assert_eq!(account.balance, INITIAL_GRANT);
    assert_eq!(account.username, "Darwin");
    assert_eq!(ledger_of(&pool, account.id).await, vec![("initial_grant".into(), 1_000)]);
    assert_ledger_explains_every_balance(&pool).await;
}

#[tokio::test]
async fn a_taken_username_leaves_no_account_and_no_grant_behind() {
    let pool = db().await;
    player(&pool, "darwin").await;
    let again = register(&pool, "Darwin", "darwin", "hash", NOW).await;
    assert!(matches!(again, Err(StoreError::Refused(Refusal::UsernameTaken))));

    let accounts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts").fetch_one(&pool).await.unwrap();
    assert_eq!(accounts, 1);
    assert_ledger_explains_every_balance(&pool).await;
}

#[tokio::test]
async fn a_session_is_found_by_its_hash_until_it_expires() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    let expires = create_session(&pool, account.id, "abc", NOW).await.unwrap();
    assert_eq!(expires, NOW + SESSION_LIFETIME);

    assert_eq!(session_account(&pool, "abc", NOW).await.unwrap().unwrap().id, account.id);
    assert!(session_account(&pool, "nope", NOW).await.unwrap().is_none());

    // and an expired one is not a session, nor is it left lying around
    assert!(session_account(&pool, "abc", expires).await.unwrap().is_none());
    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions").fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn logout_deletes_the_session_and_expiry_sweeping_is_bounded() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    create_session(&pool, account.id, "live", NOW).await.unwrap();
    create_session(&pool, account.id, "stale", NOW - SESSION_LIFETIME - 1).await.unwrap();

    assert_eq!(purge_expired_sessions(&pool, NOW).await.unwrap(), 1);
    delete_session(&pool, "live").await.unwrap();
    assert!(session_account(&pool, "live", NOW).await.unwrap().is_none());
}

#[tokio::test]
async fn a_bet_moves_coins_from_balance_into_escrow() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    let m = market(&pool, NOW).await;

    let (bet, after, account_now) =
        place_bet(&pool, account.id, m.id, SpeciesA, 40, NOW).await.unwrap();
    assert_eq!((bet.stake, bet.outcome), (40, SpeciesA));
    assert_eq!(account_now.balance, INITIAL_GRANT - 40);
    assert_eq!(escrow(&pool, account.id).await.unwrap(), 40);
    assert_eq!(pools(&pool, m.id).await.unwrap(), [40, 0, 0]);
    assert!(after.revision > m.revision, "the pools moved without a new revision");
    assert!(account_now.revision > account.revision);
    assert_ledger_explains_every_balance(&pool).await;
}

/// "make my bet exactly this": only the difference ever moves, and repeating
/// the same request must not reserve the stake twice
#[tokio::test]
async fn replacing_a_bet_moves_only_the_difference_and_repeating_it_moves_nothing() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    let m = market(&pool, NOW).await;

    place_bet(&pool, account.id, m.id, SpeciesA, 40, NOW).await.unwrap();
    place_bet(&pool, account.id, m.id, SpeciesA, 70, NOW).await.unwrap();
    place_bet(&pool, account.id, m.id, Coexistence, 25, NOW).await.unwrap();
    let (bet, _, account_now) =
        place_bet(&pool, account.id, m.id, Coexistence, 25, NOW).await.unwrap();

    assert_eq!((bet.stake, bet.outcome), (25, Coexistence));
    assert_eq!(account_now.balance, INITIAL_GRANT - 25);
    assert_eq!(escrow(&pool, account.id).await.unwrap(), 25);
    assert_eq!(pools(&pool, m.id).await.unwrap(), [0, 25, 0]);
    assert_eq!(
        ledger_of(&pool, account.id).await,
        vec![
            ("initial_grant".into(), 1_000),
            ("escrow".into(), -40),
            ("escrow".into(), -30),
            ("escrow_release".into(), 45),
        ],
        "an identical repeat wrote a ledger entry"
    );

    let bets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bets").fetch_one(&pool).await.unwrap();
    assert_eq!(bets, 1, "one account holds at most one bet per market");
    assert_ledger_explains_every_balance(&pool).await;
}

#[tokio::test]
async fn the_transaction_rechecks_phase_stake_and_balance() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    let m = market(&pool, NOW).await;

    async fn refused(r: Result<(BetRow, MarketRow, Account)>, want: Refusal) {
        match r {
            Err(StoreError::Refused(got)) => assert_eq!(got, want),
            Err(other) => panic!("expected {want:?}, got {other}"),
            Ok(_) => panic!("expected {want:?}, the bet went through"),
        }
    }

    refused(place_bet(&pool, account.id, m.id, SpeciesA, 0, NOW).await, Refusal::StakeOutOfRange)
        .await;
    refused(place_bet(&pool, account.id, m.id, SpeciesA, -5, NOW).await, Refusal::StakeOutOfRange)
        .await;
    refused(place_bet(&pool, account.id, m.id, SpeciesA, 101, NOW).await, Refusal::StakeOutOfRange)
        .await;
    refused(
        place_bet(&pool, account.id, m.id, SpeciesA, 1, m.locks_at).await,
        Refusal::MarketNotOpen,
    )
    .await;
    refused(place_bet(&pool, account.id, 999, SpeciesA, 1, NOW).await, Refusal::MarketNotFound)
        .await;

    // and once it is locked, nothing more comes in
    lock_market(&pool, m.id, NOW).await.unwrap();
    refused(place_bet(&pool, account.id, m.id, SpeciesA, 1, NOW).await, Refusal::MarketNotOpen)
        .await;
    assert_eq!(pools(&pool, m.id).await.unwrap(), [0, 0, 0]);
}

#[tokio::test]
async fn a_stake_larger_than_the_available_balance_is_refused() {
    let pool = db().await;
    let account = player(&pool, "darwin").await;
    sqlx::query("UPDATE accounts SET balance = 10 WHERE id = ?")
        .bind(account.id)
        .execute(&pool)
        .await
        .unwrap();
    let m = market(&pool, NOW).await;
    let refused = place_bet(&pool, account.id, m.id, SpeciesA, 40, NOW).await;
    assert!(matches!(refused, Err(StoreError::Refused(Refusal::InsufficientBalance))));
    assert_eq!(escrow(&pool, account.id).await.unwrap(), 0, "a refused bet still reserved coins");
    assert_eq!(pools(&pool, m.id).await.unwrap(), [0, 0, 0]);
}

/// the seed is the whole reason the market locks before the run starts
#[tokio::test]
async fn the_seed_is_revealed_only_after_the_market_locks() {
    let pool = db().await;
    let m = market(&pool, NOW).await;
    assert_eq!(m.reveal(), None, "an open market revealed its seed");

    let locked = lock_market(&pool, m.id, NOW).await.unwrap();
    assert_eq!(locked.status, MarketStatus::Locked);
    assert_eq!(locked.reveal().map(|(seed, _)| seed), Some(7));
    assert_eq!(locked.run_status, "running");

    // and locking is a one-way door, so a retry cannot re-open it
    assert!(matches!(
        lock_market(&pool, m.id, NOW).await,
        Err(StoreError::Refused(Refusal::MarketNotOpen))
    ));
}

#[tokio::test]
async fn settlement_pays_the_winners_once_however_many_times_it_is_retried() {
    let pool = db().await;
    let winner = player(&pool, "winner").await;
    let loser = player(&pool, "loser").await;
    let m = market(&pool, NOW).await;
    place_bet(&pool, winner.id, m.id, SpeciesA, 100, NOW).await.unwrap();
    place_bet(&pool, loser.id, m.id, SpeciesB, 100, NOW).await.unwrap();
    lock_market(&pool, m.id, NOW).await.unwrap();
    complete_run(&pool, m.run_id, "deadbeef", "{}", NOW).await.unwrap();
    assert_eq!(run_status(&pool, m.run_id).await, "complete");

    let first = settle_market(&pool, m.id, &contest(4_000, 500), NOW).await.unwrap();
    assert_eq!(first.winning_outcome, Some(SpeciesA));
    assert_eq!((first.gross_pool, first.burn), (200, 10));

    let paid = account(&pool, winner.id).await.unwrap().unwrap().balance;
    assert_eq!(paid, INITIAL_GRANT - 100 + 190);
    assert_eq!(account(&pool, loser.id).await.unwrap().unwrap().balance, INITIAL_GRANT - 100);
    assert_eq!(escrow(&pool, winner.id).await.unwrap(), 0, "a settled market still holds escrow");

    // the retry finds it settled and pays nobody again
    let again = settle_market(&pool, m.id, &contest(4_000, 500), NOW).await.unwrap();
    assert_eq!(again, first);
    assert_eq!(account(&pool, winner.id).await.unwrap().unwrap().balance, paid);
    assert_ledger_explains_every_balance(&pool).await;
}

#[tokio::test]
async fn total_extinction_voids_the_market_and_refunds_every_stake() {
    let pool = db().await;
    let a = player(&pool, "a").await;
    let b = player(&pool, "b").await;
    let m = market(&pool, NOW).await;
    place_bet(&pool, a.id, m.id, SpeciesA, 30, NOW).await.unwrap();
    place_bet(&pool, b.id, m.id, Coexistence, 70, NOW).await.unwrap();
    lock_market(&pool, m.id, NOW).await.unwrap();

    assert_eq!(contest(0, 0).resolve(MarketRules::V1.coexistence_margin), Resolution::Void);
    let view = void_market(&pool, m.id, NOW).await.unwrap();
    assert_eq!((view.status, view.burn), ("void", 0));
    for who in [a.id, b.id] {
        assert_eq!(account(&pool, who).await.unwrap().unwrap().balance, INITIAL_GRANT);
        assert_eq!(escrow(&pool, who).await.unwrap(), 0);
    }
    assert_ledger_explains_every_balance(&pool).await;
}

/// the form guide is the betting phase's only content, so it has to read
/// finished markets and only those: an open one has no result to report.
#[tokio::test]
async fn the_form_guide_reads_finished_markets_newest_first() {
    let pool = db().await;
    let won = market(&pool, NOW).await;
    lock_market(&pool, won.id, NOW).await.unwrap();
    settle_market(&pool, won.id, &contest(400, 100), NOW).await.unwrap();

    let died = market(&pool, NOW + 100).await;
    lock_market(&pool, died.id, NOW + 100).await.unwrap();
    void_market(&pool, died.id, NOW + 100).await.unwrap();

    let open = market(&pool, NOW + 200).await;

    let form = recent_form(&pool, 10).await.unwrap();
    assert_eq!(form.iter().map(|f| f.market_id).collect::<Vec<_>>(), vec![died.id, won.id]);
    assert!(!form.iter().any(|f| f.market_id == open.id));
    assert_eq!((form[0].status, form[0].winning_outcome), ("void", None));
    assert_eq!((form[1].status, form[1].winning_outcome), ("settled", Some(SpeciesA)));

    // the limit is the newest end of the record, not the oldest
    assert_eq!(recent_form(&pool, 1).await.unwrap()[0].market_id, died.id);
}

/// a restart lost the simulation, so the market it was watching cannot settle.
/// recovery has to give every coin back, and running it again must not.
#[tokio::test]
async fn a_restart_refunds_interrupted_markets_exactly_once() {
    let pool = db().await;
    let a = player(&pool, "a").await;
    let open = market(&pool, NOW).await;
    place_bet(&pool, a.id, open.id, SpeciesA, 30, NOW).await.unwrap();

    let running = market(&pool, NOW + 100).await;
    place_bet(&pool, a.id, running.id, SpeciesB, 20, NOW + 100).await.unwrap();
    lock_market(&pool, running.id, NOW + 100).await.unwrap();
    assert_eq!(account(&pool, a.id).await.unwrap().unwrap().balance, INITIAL_GRANT - 50);

    let recovered = recover_interrupted(&pool, NOW + 200).await.unwrap();
    assert_eq!(recovered, vec![open.id, running.id]);
    assert_eq!(account(&pool, a.id).await.unwrap().unwrap().balance, INITIAL_GRANT);

    // the second restart finds nothing left to refund
    assert!(recover_interrupted(&pool, NOW + 300).await.unwrap().is_empty());
    assert_eq!(account(&pool, a.id).await.unwrap().unwrap().balance, INITIAL_GRANT);
    assert_eq!(run_status(&pool, open.run_id).await, "void");
    assert_ledger_explains_every_balance(&pool).await;
}

/// losing a whole market's stake, the only way an account actually goes broke
async fn lose(pool: &SqlitePool, account_id: i64, now: i64) {
    let m = market(pool, now).await;
    place_bet(pool, account_id, m.id, SpeciesA, 100, now).await.unwrap();
    lock_market(pool, m.id, now).await.unwrap();
    settle_market(pool, m.id, &contest(500, 4_000), now).await.unwrap();
}

#[tokio::test]
async fn a_recovery_grant_needs_a_broke_account_with_nothing_at_stake() {
    let pool = db().await;
    let id = player(&pool, "darwin").await.id;

    // solvent: there is nothing to recover from
    assert!(matches!(
        grant_recovery(&pool, id, NOW).await,
        Err(StoreError::Refused(Refusal::RecoveryNotEligible))
    ));

    // one live bet, then lose everything else
    let live = market(&pool, NOW).await;
    place_bet(&pool, id, live.id, SpeciesA, 100, NOW).await.unwrap();
    for _ in 0..9 {
        lose(&pool, id, NOW).await;
    }
    assert_eq!(account(&pool, id).await.unwrap().unwrap().balance, 0);

    // broke, but that bet could still pay: no grant while coins are at stake
    assert!(matches!(
        grant_recovery(&pool, id, NOW).await,
        Err(StoreError::Refused(Refusal::RecoveryNotEligible))
    ));

    lock_market(&pool, live.id, NOW).await.unwrap();
    settle_market(&pool, live.id, &contest(500, 4_000), NOW).await.unwrap();
    let granted = grant_recovery(&pool, id, NOW).await.unwrap();
    assert_eq!((granted.balance, granted.escrow), (RECOVERY_GRANT, 0));

    // and once a day, not once a request
    lose(&pool, id, NOW).await;
    assert!(matches!(
        grant_recovery(&pool, id, NOW + RECOVERY_INTERVAL - 1).await,
        Err(StoreError::Refused(Refusal::RecoveryNotEligible))
    ));
    assert!(grant_recovery(&pool, id, NOW + RECOVERY_INTERVAL).await.is_ok());
    assert_ledger_explains_every_balance(&pool).await;
}

/// the grant is the server's own transaction on the server's own clock, never
/// a button a player presses
#[tokio::test]
async fn the_sweep_grants_to_exactly_the_eligible_accounts() {
    let pool = db().await;
    let broke = player(&pool, "broke").await.id;
    let solvent = player(&pool, "solvent").await.id;
    let staked = player(&pool, "staked").await.id;

    for _ in 0..10 {
        lose(&pool, broke, NOW).await;
    }
    for _ in 0..9 {
        lose(&pool, staked, NOW).await;
    }
    let live = market(&pool, NOW).await;
    place_bet(&pool, staked, live.id, SpeciesA, 100, NOW).await.unwrap();

    assert_eq!(grant_recovery_to_eligible(&pool, NOW).await.unwrap(), vec![broke]);
    assert_eq!(account(&pool, broke).await.unwrap().unwrap().balance, RECOVERY_GRANT);
    assert_eq!(account(&pool, solvent).await.unwrap().unwrap().balance, INITIAL_GRANT);
    assert_eq!(account(&pool, staked).await.unwrap().unwrap().balance, 0);

    // and it does not grant twice within the day
    assert!(grant_recovery_to_eligible(&pool, NOW + 1).await.unwrap().is_empty());
    assert_ledger_explains_every_balance(&pool).await;
}

/// the invariant test: throw every operation at the store in a deterministic
/// but arbitrary order and the ledger still has to explain every balance
#[tokio::test]
async fn the_ledger_always_explains_the_cached_balance() {
    let pool = db().await;
    let mut accounts = Vec::new();
    for i in 0..6 {
        accounts.push(player(&pool, &format!("player{i}")).await.id);
    }

    let mut rng = Rng::new(20_260_828);
    let mut now = NOW;
    for round in 0..12 {
        let m = market(&pool, now).await;
        for _ in 0..20 {
            let who = accounts[rng.below(accounts.len())];
            let outcome = MarketOutcome::ALL[rng.below(3)];
            let stake = 1 + rng.below(100) as i64;
            // refusals are part of the sequence: a rejected bet must leave
            // nothing behind either
            let _ = place_bet(&pool, who, m.id, outcome, stake, now).await;
        }
        lock_market(&pool, m.id, now).await.unwrap();

        match round % 4 {
            0 => {
                void_market(&pool, m.id, now).await.unwrap();
                void_market(&pool, m.id, now).await.unwrap();
            }
            // nobody can have chosen this, so the whole pool burns
            1 => {
                settle_market(&pool, m.id, &contest(0, 1), now).await.unwrap();
            }
            _ => {
                let result = contest(1 + rng.below(4_000) as u64, 1 + rng.below(4_000) as u64);
                settle_market(&pool, m.id, &result, now).await.unwrap();
                settle_market(&pool, m.id, &result, now).await.unwrap();
            }
        }
        assert_ledger_explains_every_balance(&pool).await;
        now += 1_000;
    }

    // and nothing was left escrowed once every market reached a terminal state
    for id in accounts {
        assert_eq!(escrow(&pool, id).await.unwrap(), 0);
    }
    let burned: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(burn), 0) FROM markets")
        .fetch_one(&pool)
        .await
        .unwrap();
    let held: i64 =
        sqlx::query_scalar("SELECT SUM(balance) FROM accounts").fetch_one(&pool).await.unwrap();
    assert_eq!(held + burned, 6 * INITIAL_GRANT, "coins were created or destroyed");
}
