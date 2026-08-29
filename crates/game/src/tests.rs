use super::*;

fn coins(n: i64) -> Coins {
    Coins::new(n).expect("non-negative")
}

fn tally(id: u32, initial: u64, final_population: u64) -> SpeciesTally {
    SpeciesTally { id, initial, final_population }
}

fn contest(a: u64, b: u64) -> ContestResult {
    ContestResult::new(tally(0, 500, a), tally(1, 500, b)).expect("founded species")
}

fn wager(key: i64, outcome: MarketOutcome, stake: i64) -> Wager {
    Wager { key, outcome, stake: coins(stake) }
}

#[test]
fn coins_are_whole_non_negative_and_checked() {
    assert_eq!(Coins::new(-1), Err(GameError::Negative));
    assert_eq!(coins(3).checked_sub(coins(4)), Err(GameError::Negative));
    assert_eq!(coins(3).checked_sub(coins(3)), Ok(Coins::ZERO));
    assert_eq!(coins(i64::MAX).checked_add(coins(1)), Err(GameError::Overflow));
    assert_eq!(coins(7).get(), 7);
}

#[test]
fn a_species_with_no_founders_cannot_be_scored() {
    assert_eq!(
        ContestResult::new(tally(0, 0, 10), tally(1, 500, 10)),
        Err(GameError::EmptyInitialPopulation)
    );
}

#[test]
fn the_three_outcomes_are_exhaustive_except_for_total_extinction() {
    let margin = MarketRules::V1.coexistence_margin;
    assert_eq!(contest(0, 0).resolve(margin), Resolution::Void);
    assert_eq!(contest(1, 0).resolve(margin), Resolution::Won(MarketOutcome::SpeciesA));
    assert_eq!(contest(0, 1).resolve(margin), Resolution::Won(MarketOutcome::SpeciesB));
    assert_eq!(contest(900, 900).resolve(margin), Resolution::Won(MarketOutcome::Coexistence));
    assert_eq!(contest(4000, 500).resolve(margin), Resolution::Won(MarketOutcome::SpeciesA));
    assert_eq!(contest(500, 4000).resolve(margin), Resolution::Won(MarketOutcome::SpeciesB));
}

/// a single survivor wins however small it is: one organism left beats none,
/// and the margin never applies when the other species is gone
#[test]
fn one_survivor_wins_without_consulting_the_margin() {
    assert_eq!(contest(1, 0).resolve(10.0), Resolution::Won(MarketOutcome::SpeciesA));
    assert_eq!(contest(0, 1).resolve(10.0), Resolution::Won(MarketOutcome::SpeciesB));
}

/// the log ratio is what makes the band symmetric: a run and its mirror image
/// have to land on mirrored outcomes, never both on the same side
#[test]
fn the_coexistence_band_is_symmetric() {
    for margin in [0.05f64, 0.2, 0.5] {
        for (a, b) in [(500u64, 900u64), (1000, 1100), (2000, 2001), (10, 5000)] {
            let forward = contest(a, b).resolve(margin);
            let mirrored = contest(b, a).resolve(margin);
            let expected = match forward {
                Resolution::Won(MarketOutcome::SpeciesA) => {
                    Resolution::Won(MarketOutcome::SpeciesB)
                }
                Resolution::Won(MarketOutcome::SpeciesB) => {
                    Resolution::Won(MarketOutcome::SpeciesA)
                }
                other => other,
            };
            assert_eq!(mirrored, expected, "margin {margin}, {a} vs {b}");
        }
    }
}

/// the boundary is inclusive, so a run exactly on the margin coexists
#[test]
fn a_run_exactly_on_the_margin_coexists() {
    let margin = 0.2f64;
    let b = 1000u64;
    let a = (b as f64 * margin.exp()).round() as u64;
    let inside = contest(a - 1, b).resolve(margin);
    assert_eq!(inside, Resolution::Won(MarketOutcome::Coexistence));
    let outside = contest(a + 30, b).resolve(margin);
    assert_eq!(outside, Resolution::Won(MarketOutcome::SpeciesA));
}

#[test]
fn a_won_market_burns_the_fee_and_the_remainder_and_pays_the_rest() {
    // 100 on the winner, 200 against. gross 300, burn 15, distributable 285.
    let wagers = [wager(1, MarketOutcome::SpeciesA, 100), wager(2, MarketOutcome::SpeciesB, 200)];
    let s = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();

    assert_eq!(s.resolution, Resolution::Won(MarketOutcome::SpeciesA));
    assert_eq!(s.gross, coins(300));
    assert_eq!(s.winning_pool, coins(100));
    assert_eq!(s.payouts, vec![coins(285), Coins::ZERO]);
    assert_eq!(s.burn, coins(15));
}

/// principal is inside the payout, and with a pari-mutuel pool being right can
/// still pay less than the stake. the ui has to say so before confirmation.
#[test]
fn a_correct_bet_can_return_less_than_its_stake() {
    let wagers = [
        wager(1, MarketOutcome::SpeciesA, 100),
        wager(2, MarketOutcome::SpeciesA, 100),
        wager(3, MarketOutcome::SpeciesB, 1),
    ];
    let s = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();
    assert!(s.payouts[0] < coins(100), "{:?} should be under the stake", s.payouts[0]);
    assert_eq!(s.payouts[0], s.payouts[1]);
}

#[test]
fn nobody_right_burns_the_whole_pool_without_inventing_a_winner() {
    let wagers = [wager(1, MarketOutcome::Coexistence, 40), wager(2, MarketOutcome::SpeciesB, 60)];
    let s = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();
    assert_eq!(s.resolution, Resolution::Won(MarketOutcome::SpeciesA));
    assert_eq!(s.winning_pool, Coins::ZERO);
    assert_eq!(s.payouts, vec![Coins::ZERO; 2]);
    assert_eq!(s.burn, coins(100));
}

#[test]
fn total_extinction_voids_and_refunds_every_stake_without_a_fee() {
    let wagers = [
        wager(1, MarketOutcome::SpeciesA, 7),
        wager(2, MarketOutcome::Coexistence, 100),
        wager(3, MarketOutcome::SpeciesB, 1),
    ];
    let s = settle(&contest(0, 0), &MarketRules::V1, &wagers).unwrap();
    assert_eq!(s.resolution, Resolution::Void);
    assert_eq!(s.payouts, vec![coins(7), coins(100), coins(1)]);
    assert_eq!(s.burn, Coins::ZERO);
}

#[test]
fn a_market_nobody_entered_settles_to_nothing() {
    let s = settle(&contest(4000, 500), &MarketRules::V1, &[]).unwrap();
    assert_eq!(s.gross, Coins::ZERO);
    assert_eq!(s.burn, Coins::ZERO);
    assert!(s.payouts.is_empty());
}

/// no coin is created and none escapes: payouts plus burn are always exactly
/// the pool, whatever the outcome and however the stakes divide
#[test]
fn every_coin_in_the_pool_is_either_paid_or_burned() {
    let stakes = [1i64, 3, 7, 11, 100, 99, 2];
    let outcomes = MarketOutcome::ALL;
    for (final_a, final_b) in [(0u64, 0u64), (4000, 500), (900, 900), (0, 3)] {
        let wagers: Vec<Wager> =
            stakes.iter().enumerate().map(|(i, &s)| wager(i as i64, outcomes[i % 3], s)).collect();
        let s = settle(&contest(final_a, final_b), &MarketRules::V1, &wagers).unwrap();
        let paid = s.payouts.iter().fold(Coins::ZERO, |sum, c| sum.checked_add(*c).unwrap());
        assert_eq!(paid.checked_add(s.burn).unwrap(), s.gross, "{final_a} vs {final_b}");
        assert_eq!(s.gross.get(), stakes.iter().sum::<i64>());
    }
}

/// settlement is arithmetic on a set, so the order the bets happen to be
/// loaded in cannot change what anybody is paid
#[test]
fn payouts_do_not_depend_on_the_order_the_bets_are_read_in() {
    let wagers = vec![
        wager(1, MarketOutcome::SpeciesA, 100),
        wager(2, MarketOutcome::SpeciesB, 3),
        wager(3, MarketOutcome::SpeciesA, 7),
        wager(4, MarketOutcome::Coexistence, 40),
    ];
    let forward = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();

    let mut reversed = wagers.clone();
    reversed.reverse();
    let backward = settle(&contest(4000, 500), &MarketRules::V1, &reversed).unwrap();

    assert_eq!(forward.burn, backward.burn);
    assert_eq!(forward.gross, backward.gross);
    let mut a: Vec<(i64, Coins)> = wagers.iter().map(|w| w.key).zip(forward.payouts).collect();
    let mut b: Vec<(i64, Coins)> = reversed.iter().map(|w| w.key).zip(backward.payouts).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
}

/// a full-width pool must not wrap. the multiply happens in i128 precisely so
/// that a pool near the storage ceiling still divides correctly.
#[test]
fn a_pool_at_the_storage_ceiling_still_divides() {
    let huge = i64::MAX / 4;
    let wagers = [wager(1, MarketOutcome::SpeciesA, huge), wager(2, MarketOutcome::SpeciesB, huge)];
    let s = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();
    assert_eq!(s.gross, coins(huge * 2));
    let paid = s.payouts.iter().fold(Coins::ZERO, |sum, c| sum.checked_add(*c).unwrap());
    assert_eq!(paid.checked_add(s.burn).unwrap(), s.gross);
    assert_eq!(coins(huge).checked_add(coins(i64::MAX)), Err(GameError::Overflow));
}

#[test]
fn stake_limits_are_enforced_against_the_rules_the_market_declared() {
    let rules = MarketRules::V1;
    assert_eq!(rules.check_stake(Coins::ZERO), Err(GameError::StakeOutOfRange));
    assert_eq!(rules.check_stake(coins(1)), Ok(()));
    assert_eq!(rules.check_stake(coins(100)), Ok(()));
    assert_eq!(rules.check_stake(coins(101)), Err(GameError::StakeOutOfRange));
}

#[test]
fn a_projection_is_the_share_of_the_pool_a_coin_would_claim() {
    let mut pool = Pool::default();
    pool.add(MarketOutcome::SpeciesA, coins(100)).unwrap();
    pool.add(MarketOutcome::SpeciesB, coins(100)).unwrap();

    // 201 gross, a 10-coin burn leaves 191 over a 101-coin winning side
    let p = pool.projection(MarketOutcome::SpeciesA, coins(1), &MarketRules::V1).unwrap();
    assert!((p - 191.0 / 101.0).abs() < 1e-9, "{p}");

    // the first coin into an empty market is the whole pool, and a burn that
    // rounds down to nothing is why it gets all of it back
    let empty = Pool::default();
    let p = empty.projection(MarketOutcome::Coexistence, coins(1), &MarketRules::V1).unwrap();
    assert!((p - 1.0).abs() < 1e-9, "{p}");

    // and an outcome nobody can win pays nothing
    assert_eq!(empty.projection(MarketOutcome::SpeciesA, Coins::ZERO, &MarketRules::V1), Ok(0.0));
}

/// the projection has to agree with what settlement actually pays, or the
/// number on the button is a lie
#[test]
fn a_projection_matches_what_settlement_pays_for_the_same_pools() {
    let existing = [wager(1, MarketOutcome::SpeciesA, 100), wager(2, MarketOutcome::SpeciesB, 300)];
    let mut pool = Pool::default();
    for w in &existing {
        pool.add(w.outcome, w.stake).unwrap();
    }
    let projected = pool.projection(MarketOutcome::SpeciesA, coins(25), &MarketRules::V1).unwrap();

    let mut wagers = existing.to_vec();
    wagers.push(wager(3, MarketOutcome::SpeciesA, 25));
    let s = settle(&contest(4000, 500), &MarketRules::V1, &wagers).unwrap();
    assert_eq!(s.payouts[2], coins((projected * 25.0) as i64));
}

#[test]
fn outcome_names_survive_the_round_trip_the_database_and_wire_use() {
    for o in MarketOutcome::ALL {
        assert_eq!(MarketOutcome::parse(o.as_str()), Some(o));
        assert_eq!(serde_json::to_string(&o).unwrap(), format!("\"{}\"", o.as_str()));
    }
    assert_eq!(MarketOutcome::parse("species_c"), None);
    assert_eq!(MarketOutcome::ALL.map(|o| o.index()), [0, 1, 2]);
}

/// the dependency boundary is the whole point of this crate: betting cannot
/// influence ecology if betting code cannot see it
#[test]
fn the_game_crate_cannot_reach_the_simulation() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "ecosym-simulation",
        "ecosym-ecology",
        "ecosym-world",
        "ecosym-core",
        "ecosym-replay",
        "sqlx",
        "axum",
        "tokio",
        "rand",
        "chrono",
    ] {
        assert!(!manifest.contains(forbidden), "ecosym-game must not depend on {forbidden}");
    }
}
