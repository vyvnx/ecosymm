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

/** how a settled market reads for one player */
export function settlementLine(market, bet) {
  if (!market || (market.phase !== 'settled' && market.phase !== 'void')) return null
  if (market.phase === 'void') {
    return bet
      ? `void - everything died. ${formatCoins(bet.stake)} refunded.`
      : 'void - everything died. every stake refunded.'
  }
  const won = bet && bet.outcome === market.winning_outcome
  const name = outcomeLabels(market.species).find((o) => o.key === market.winning_outcome)
  if (!bet) return `${name?.label ?? 'nobody'} took the market`
  return won
    ? `won - ${formatCoins(bet.payout ?? 0)} paid on a ${formatCoins(bet.stake)} stake`
    : `lost - ${formatCoins(bet.stake)} on ${
        outcomeLabels(market.species).find((o) => o.key === bet.outcome)?.label ?? bet.outcome
      }`
}
