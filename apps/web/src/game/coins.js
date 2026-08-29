/**
 * darwin coin arithmetic and the labels around it.
 *
 * play money: whole coins only, never bought, never redeemed, never worth
 * anything. the projection here mirrors `Pool::projection` in `ecosym-game`
 * so the number under a button is the number settlement would pay, and
 * `a_projection_matches_what_settlement_pays` pins the two together.
 */

export const UNIT = 'DC'

export const formatCoins = (n) => `${Math.trunc(n).toLocaleString()} ${UNIT}`

/** whole coins or nothing. "12.5", "-3" and "" are not stakes. */
export function parseStake(text) {
  const trimmed = String(text ?? '').trim()
  if (!/^\d+$/.test(trimmed)) return null
  const stake = Number(trimmed)
  return Number.isSafeInteger(stake) ? stake : null
}

/**
 * the same limits the write transaction enforces, checked early so the player
 * reads why rather than watching a button do nothing. `held` is what this
 * account already has escrowed on this market: replacing a bet releases it,
 * so it is spendable again.
 */
export function checkStake(stake, { min, max, balance, held = 0 }) {
  if (stake === null) return 'whole coins only'
  if (stake < min) return `minimum ${formatCoins(min)}`
  if (stake > max) return `maximum ${formatCoins(max)}`
  if (stake > balance + held) return 'more than you have'
  return null
}

/**
 * decimal return one more `stake` on `index` would claim if it won, from the
 * pools as they stand. it moves with every later bet, and it is below 1.0
 * when nearly the whole pool is on the same outcome.
 */
export function projection(pools, index, stake, feeBps) {
  const gross = pools.reduce((total, p) => total + p, 0) + stake
  const winning = pools[index] + stake
  if (winning <= 0) return 0
  return (gross - Math.floor((gross * feeBps) / 10000)) / winning
}

export const formatMultiplier = (x) => `${x.toFixed(2)}x`

/**
 * seconds left on a server deadline, corrected by the estimated clock offset.
 * a sleeping tab, a skewed device clock and a throttled timer all resume onto
 * the server's own absolute deadline rather than a countdown they kept.
 */
export function secondsUntil(deadline, offset, now = Date.now() / 1000) {
  return Math.max(0, Math.round(deadline - (now + offset)))
}

/** what the three buttons say, in the server's species order */
export function outcomeLabels(species) {
  return [
    { key: 'species_a', label: species?.[0]?.name ?? 'Species A', species: 0 },
    { key: 'coexistence', label: 'Coexistence', species: null },
    { key: 'species_b', label: species?.[1]?.name ?? 'Species B', species: 1 },
  ]
}

/**
 * where the pot went, for the payout phase. `null` when there is nothing to
 * say, because nobody bet on this one.
 *
 * three endings, and the losing two are the interesting ones: a pool nobody
 * backed burns whole, and a world that died gives everything back.
 */
export function payoutLine(market) {
  if (!market) return null
  const gross = market.gross_pool ?? 0
  if (gross <= 0) return null

  if (market.phase === 'void') {
    return { amount: gross, prefix: 'refunding', suffix: 'nothing survived', tone: 'void' }
  }
  if (market.phase !== 'settled') return null

  const outcomes = outcomeLabels(market.species)
  const index = outcomes.findIndex((o) => o.key === market.winning_outcome)
  const winners = index < 0 ? 0 : (market.bettors?.[index] ?? 0)
  if (winners === 0) {
    return {
      amount: gross,
      prefix: 'burning',
      suffix: `nobody backed ${outcomes[index]?.label ?? 'it'}`,
      tone: 'burn',
    }
  }
  return {
    amount: gross - (market.burn ?? 0),
    prefix: 'paying out',
    suffix: `to ${winners} ${winners === 1 ? 'backer' : 'backers'} of ${outcomes[index].label}`,
    tone: 'pay',
  }
}

/**
 * what a settled market did to *your* coins, or nothing.
 *
 * who took the market is the run's own result and is said once, where the run
 * is reported. this line only ever adds what the reader cannot see there.
 */
export function settlementLine(market, bet) {
  if (!market || !bet) return null
  if (market.phase === 'void') return `void - your ${formatCoins(bet.stake)} came back`
  if (market.phase !== 'settled') return null
  if (bet.outcome === market.winning_outcome) {
    return `you won ${formatCoins(bet.payout ?? 0)} on a ${formatCoins(bet.stake)} stake`
  }
  const chose = outcomeLabels(market.species).find((o) => o.key === bet.outcome)
  return `you lost ${formatCoins(bet.stake)} on ${chose?.label ?? bet.outcome}`
}
