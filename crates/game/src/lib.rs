//! play money, market rules and pari-mutuel settlement.
//!
//! pure arithmetic. this crate has no simulation, database, network, clock or
//! randomness, which is what makes it structurally impossible for a balance or
//! a wager to reach the ecology. the server maps a finished run into
//! [`ContestResult`] and gets a [`Settlement`] back; nothing travels the other
//! way. `the_game_crate_cannot_reach_the_simulation` is the proof.

use serde::{Deserialize, Serialize};

/// whole darwin coins. play money: never negative, never fractional, never
/// bought, never redeemed.
///
/// stored as `i64` because sqlite integers are signed and a checked conversion
/// at that boundary is cheaper than a lossy one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Coins(i64);

impl Coins {
    pub const ZERO: Coins = Coins(0);

    /// the only way in. a negative balance is not a number this game has.
    pub fn new(amount: i64) -> Result<Coins, GameError> {
        if amount < 0 {
            return Err(GameError::Negative);
        }
        Ok(Coins(amount))
    }

    /// for constants written in this crate, where the value is visible
    const fn known(amount: i64) -> Coins {
        Coins(amount)
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, other: Coins) -> Result<Coins, GameError> {
        self.0.checked_add(other.0).map(Coins).ok_or(GameError::Overflow)
    }

    /// underflow is an error, not a saturation: a balance that would go
    /// negative means the caller validated something wrong.
    pub fn checked_sub(self, other: Coins) -> Result<Coins, GameError> {
        if other.0 > self.0 {
            return Err(GameError::Negative);
        }
        Ok(Coins(self.0 - other.0))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GameError {
    /// an amount below zero, or a subtraction that would go below zero
    Negative,
    /// the value left the range a persisted `i64` can hold
    Overflow,
    /// a species that never existed cannot be scored
    EmptyInitialPopulation,
    /// a stake outside the market's declared limits
    StakeOutOfRange,
}

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            GameError::Negative => "darwin coins cannot go negative",
            GameError::Overflow => "amount out of range",
            GameError::EmptyInitialPopulation => "a species with no founders cannot be scored",
            GameError::StakeOutOfRange => "stake outside the market limits",
        };
        f.write_str(text)
    }
}

impl std::error::Error for GameError {}

/// the three exhaustive selections a bettor can make. the species variants
/// name positions in the market's ordered species list, not database ids.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketOutcome {
    SpeciesA,
    Coexistence,
    SpeciesB,
}

impl MarketOutcome {
    /// the order the buttons are drawn in, and the order pools are stored in
    pub const ALL: [MarketOutcome; 3] =
        [MarketOutcome::SpeciesA, MarketOutcome::Coexistence, MarketOutcome::SpeciesB];

    pub fn index(self) -> usize {
        match self {
            MarketOutcome::SpeciesA => 0,
            MarketOutcome::Coexistence => 1,
            MarketOutcome::SpeciesB => 2,
        }
    }

    /// the name persisted in sqlite and sent on the wire
    pub fn as_str(self) -> &'static str {
        match self {
            MarketOutcome::SpeciesA => "species_a",
            MarketOutcome::Coexistence => "coexistence",
            MarketOutcome::SpeciesB => "species_b",
        }
    }

    pub fn parse(s: &str) -> Option<MarketOutcome> {
        MarketOutcome::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// what a settled market paid out on, once the run is over
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "outcome")]
pub enum Resolution {
    Won(MarketOutcome),
    /// nothing survived. every stake comes back and no fee is taken.
    Void,
}

impl Resolution {
    pub fn winner(self) -> Option<MarketOutcome> {
        match self {
            Resolution::Won(o) => Some(o),
            Resolution::Void => None,
        }
    }
}

/// the fixed v1 rule set. these are contract, not configuration: a market
/// persists the values it settled under so a later change cannot rewrite
/// history.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketRules {
    pub version: u32,
    /// house burn on a won market, in basis points
    pub fee_bps: u32,
    /// coexistence is `abs(ln(score_a / score_b)) <= margin`
    pub coexistence_margin: f64,
    pub min_stake: Coins,
    pub max_stake: Coins,
}

/// five percent, burned rather than paid to anyone. there is no house account.
pub const FEE_BPS_V1: u32 = 500;

/// calibrated over 1,000 default-config seeds; see
/// `experiments/2026-08-28-bet-outcome-calibration`. picked as the smallest
/// simple margin putting coexistence in the 20-35% band of non-void runs.
pub const COEXISTENCE_MARGIN_V1: f64 = 0.20;

impl MarketRules {
    pub const V1: MarketRules = MarketRules {
        version: 1,
        fee_bps: FEE_BPS_V1,
        coexistence_margin: COEXISTENCE_MARGIN_V1,
        min_stake: Coins::known(1),
        max_stake: Coins::known(100),
    };

    /// stake limits are re-checked inside the write transaction; a disabled
    /// button is not authorization.
    pub fn check_stake(&self, stake: Coins) -> Result<(), GameError> {
        if stake < self.min_stake || stake > self.max_stake {
            return Err(GameError::StakeOutOfRange);
        }
        Ok(())
    }
}

/// one species as the game layer sees it: an identity and two head counts.
/// everything else about the run stays in the simulation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeciesTally {
    pub id: u32,
    pub initial: u64,
    pub final_population: u64,
}

/// a finished two-species contest, in market order
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContestResult {
    pub species: [SpeciesTally; 2],
}

impl ContestResult {
    pub fn new(a: SpeciesTally, b: SpeciesTally) -> Result<ContestResult, GameError> {
        if a.initial == 0 || b.initial == 0 {
            return Err(GameError::EmptyInitialPopulation);
        }
        Ok(ContestResult { species: [a, b] })
    }

    fn score(&self, i: usize) -> f64 {
        self.species[i].final_population as f64 / self.species[i].initial as f64
    }

    /// game-layer interpretation only. it neither reads nor replaces
    /// `ecosym_simulation::Winner`.
    ///
    /// the log ratio is what makes the margin symmetric: outrunning the other
    /// species by a factor and being outrun by its reciprocal are the same
    /// distance from the middle.
    pub fn resolve(&self, coexistence_margin: f64) -> Resolution {
        match (self.species[0].final_population, self.species[1].final_population) {
            (0, 0) => Resolution::Void,
            (0, _) => Resolution::Won(MarketOutcome::SpeciesB),
            (_, 0) => Resolution::Won(MarketOutcome::SpeciesA),
            _ => {
                let gap = (self.score(0) / self.score(1)).ln();
                if gap.abs() <= coexistence_margin {
                    Resolution::Won(MarketOutcome::Coexistence)
                } else if gap > 0.0 {
                    Resolution::Won(MarketOutcome::SpeciesA)
                } else {
                    Resolution::Won(MarketOutcome::SpeciesB)
                }
            }
        }
    }
}

/// escrowed stakes per outcome, in [`MarketOutcome::ALL`] order
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pool([Coins; 3]);

impl Pool {
    pub fn new(totals: [Coins; 3]) -> Pool {
        Pool(totals)
    }

    pub fn on(&self, outcome: MarketOutcome) -> Coins {
        self.0[outcome.index()]
    }

    pub fn totals(&self) -> [Coins; 3] {
        self.0
    }

    pub fn add(&mut self, outcome: MarketOutcome, stake: Coins) -> Result<(), GameError> {
        let slot = &mut self.0[outcome.index()];
        *slot = slot.checked_add(stake)?;
        Ok(())
    }

    pub fn gross(&self) -> Result<Coins, GameError> {
        self.0.iter().try_fold(Coins::ZERO, |sum, c| sum.checked_add(*c))
    }

    /// decimal return per coin if `added` more coins went on `outcome` and it
    /// then won. an estimate that moves with every later bet, never a promise:
    /// with a pari-mutuel pool it is below 1.0 when nearly everyone is right.
    pub fn projection(
        &self,
        outcome: MarketOutcome,
        added: Coins,
        rules: &MarketRules,
    ) -> Result<f64, GameError> {
        let gross = self.gross()?.checked_add(added)?;
        let winning = self.on(outcome).checked_add(added)?;
        if winning.is_zero() {
            return Ok(0.0);
        }
        let distributable = distributable(gross, rules.fee_bps);
        Ok(distributable.get() as f64 / winning.get() as f64)
    }
}

/// one escrowed bet. the key is opaque here - the server puts its own bet row
/// id in it and reads it back off the aligned payout.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wager {
    pub key: i64,
    pub outcome: MarketOutcome,
    pub stake: Coins,
}

/// what one market paid, and where every coin went
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settlement {
    pub resolution: Resolution,
    pub gross: Coins,
    /// the fee plus whatever integer division could not divide. no account
    /// receives it.
    pub burn: Coins,
    pub winning_pool: Coins,
    /// aligned with the wagers passed to [`settle`], same order, same length.
    /// a payout already contains the principal; it is not returned separately.
    pub payouts: Vec<Coins>,
}

fn distributable(gross: Coins, fee_bps: u32) -> Coins {
    // i128 so a full-width gross pool cannot overflow the multiply
    let burn = (gross.get() as i128 * fee_bps as i128) / 10_000;
    Coins(gross.get() - burn as i64)
}

/// pari-mutuel: winners divide what is left after the burn, in proportion to
/// their stakes. the server cannot go insolvent because nothing beyond the
/// pool is ever paid.
pub fn settle(
    result: &ContestResult,
    rules: &MarketRules,
    wagers: &[Wager],
) -> Result<Settlement, GameError> {
    let resolution = result.resolve(rules.coexistence_margin);
    let mut pool = Pool::default();
    for w in wagers {
        pool.add(w.outcome, w.stake)?;
    }
    let gross = pool.gross()?;

    let (winning_pool, payouts) = match resolution {
        // nothing survived: every stake comes back untouched, no fee
        Resolution::Void => (Coins::ZERO, wagers.iter().map(|w| w.stake).collect()),
        Resolution::Won(outcome) => {
            let winning = pool.on(outcome);
            if winning.is_zero() {
                // nobody was right. the pool burns; no winner is invented, no
                // loser is refunded, and nothing rolls forward.
                (winning, vec![Coins::ZERO; wagers.len()])
            } else {
                let distributable = distributable(gross, rules.fee_bps).get() as i128;
                let share = |w: &Wager| -> Result<Coins, GameError> {
                    if w.outcome != outcome {
                        return Ok(Coins::ZERO);
                    }
                    let paid = distributable * w.stake.get() as i128 / winning.get() as i128;
                    i64::try_from(paid).map(Coins).map_err(|_| GameError::Overflow)
                };
                (winning, wagers.iter().map(share).collect::<Result<Vec<_>, _>>()?)
            }
        }
    };

    // the burn is whatever did not leave: the fee and the division remainder
    let paid = payouts.iter().try_fold(Coins::ZERO, |sum, c| sum.checked_add(*c))?;
    Ok(Settlement { resolution, gross, burn: gross.checked_sub(paid)?, winning_pool, payouts })
}

#[cfg(test)]
mod tests;
