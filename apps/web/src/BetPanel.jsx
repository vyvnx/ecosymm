import { useEffect, useState } from 'react'
import { speciesCss } from './render/WorldRenderer.js'
import {
  checkStake,
  formatCoins,
  formatMultiplier,
  outcomeLabels,
  parseStake,
  projection,
  secondsUntil,
  settlementLine,
} from './game/coins.js'
import { canBet } from './game/market.js'

/**
 * the market, bottom centre. three outcomes, one amount, and a countdown that
 * is corrected onto the server's own deadline rather than kept locally.
 *
 * it is only large while it is useful. the moment the market locks, the
 * controls fold away and what is left is one line - phase, your bet, the pool
 * - because for the next sixty seconds the world is the thing to look at and
 * this panel is in front of it.
 *
 * nothing here is ever optimistic: coins move when the server says they have.
 */
export default function BetPanel({ market, account, bet, synced, connected, offset, onBet }) {
  const [amount, setAmount] = useState('10')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState(null)
  const [now, setNow] = useState(Date.now() / 1000)

  // twice a second is enough for a countdown and cheap enough to keep in
  // React state; per-frame data never comes through here
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now() / 1000), 500)
    return () => clearInterval(id)
  }, [])

  if (!market) return null

  const corrected = now + offset
  const rules = market.rules
  const stake = parseStake(amount)
  const held = bet && market.phase === 'open' ? bet.stake : 0
  const invalid = account
    ? checkStake(stake, {
        min: rules.min_stake,
        max: rules.max_stake,
        balance: account.balance,
        held,
      })
    : null
  const open = canBet({ market, account, synced, connected, submitting }, corrected)
  const outcomes = outcomeLabels(market.species)
  const settled = settlementLine(market, bet)
  // the controls are worth their space only while a bet can still be placed
  const expanded = market.phase === 'open'
  const mine = bet && outcomes.find((o) => o.key === bet.outcome)

  async function place(outcome) {
    if (!open || invalid) return
    setSubmitting(true)
    setError(null)
    try {
      await onBet(market.market_id, outcome, stake)
    } catch (e) {
      setError(e.message)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="pointer-events-auto absolute inset-x-0 bottom-0 flex justify-center px-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
      <div
        className={`w-full rounded border border-neutral-800/80 bg-neutral-950/80 p-3 text-xs backdrop-blur transition-[max-width] duration-500 ease-out motion-reduce:transition-none ${
          expanded ? 'max-w-md' : 'max-w-md sm:max-w-sm'
        }`}
      >
        {/* the line that never folds away */}
        <div className="flex items-baseline gap-2">
          <span className="text-neutral-300">{phase(market, corrected)}</span>
          {!expanded && mine && (
            <span className="flex items-baseline gap-1.5 truncate text-neutral-500">
              <span className="text-neutral-700">·</span>
              <span className="text-emerald-400">{formatCoins(bet.stake)}</span>
              <span className="truncate">on {mine.label}</span>
            </span>
          )}
          <span className="ml-auto shrink-0 tabular-nums text-neutral-600">
            {market.pools.reduce((a, b) => a + b, 0).toLocaleString()} DC pool
          </span>
          <Caveat />
        </div>

        {settled && <p className="mt-2 text-neutral-100">{settled}</p>}

        {/* the controls, folded away the moment they stop being useful */}
        <div
          aria-hidden={!expanded}
          className={`grid transition-[grid-template-rows,opacity] duration-500 ease-out motion-reduce:transition-none ${
            expanded ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0'
          }`}
        >
          <div className="overflow-hidden">
            <div className="flex items-center gap-2 pt-2">
              <label htmlFor="stake" className="text-neutral-600">
                stake
              </label>
              <input
                id="stake"
                inputMode="numeric"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                disabled={!account || !expanded}
                tabIndex={expanded ? 0 : -1}
                className="min-h-11 w-20 rounded border border-neutral-800 bg-neutral-900/60 px-2 py-1 text-right tabular-nums text-neutral-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-50 sm:min-h-9"
              />
              <span className="text-neutral-600">DC</span>
              <span className="ml-auto tabular-nums text-neutral-500">
                {account ? `${formatCoins(account.balance)} available` : 'sign in to bet'}
              </span>
            </div>

            <div className="mt-2 grid gap-1.5 sm:grid-cols-3">
              {outcomes.map((outcome, index) => {
                const chosen = bet?.outcome === outcome.key
                return (
                  <button
                    key={outcome.key}
                    type="button"
                    onClick={() => place(outcome.key)}
                    disabled={!open || Boolean(invalid)}
                    tabIndex={expanded ? 0 : -1}
                    aria-pressed={chosen}
                    className={`flex min-h-11 items-center gap-2 rounded border px-2 py-1.5 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-40 sm:flex-col sm:items-start sm:gap-0.5 ${
                      chosen
                        ? 'border-emerald-700 bg-emerald-950/40 text-emerald-200'
                        : 'border-neutral-800 text-neutral-300 hover:border-neutral-700 hover:text-neutral-100'
                    }`}
                  >
                    <span className="flex items-center gap-1.5">
                      {outcome.species !== null && (
                        <span
                          className="h-2 w-2 shrink-0 rounded-full"
                          style={{ background: speciesCss(outcome.species) }}
                        />
                      )}
                      {outcome.label}
                    </span>
                    <span className="ml-auto tabular-nums text-neutral-500 sm:ml-0">
                      {formatMultiplier(
                        projection(market.pools, index, stake ?? 0, rules.fee_bps),
                      )}
                    </span>
                  </button>
                )
              })}
            </div>

            {(invalid || error || mine) && (
              <p className="mt-2 text-neutral-600">
                {invalid || error ? (
                  <span className="text-amber-400">{invalid || error}</span>
                ) : (
                  `holding ${formatCoins(bet.stake)} on ${mine.label} - tap another to replace it`
                )}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/**
 * the pari-mutuel caveat, out of the way but never gone. it is the one thing
 * on this panel a player can be genuinely misled by, so it stays reachable by
 * hover, by keyboard focus, and by a screen reader.
 */
function Caveat() {
  return (
    <span className="group relative shrink-0">
      <button
        type="button"
        aria-label="how the projected returns work"
        className="grid h-4 w-4 place-items-center rounded-full border border-neutral-700 text-[9px] leading-none text-neutral-500 hover:border-neutral-500 hover:text-neutral-300 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
      >
        i
      </button>
      <span
        role="tooltip"
        className="pointer-events-none absolute right-0 bottom-6 w-64 rounded border border-neutral-800 bg-neutral-950/95 p-2 text-neutral-400 opacity-0 shadow-lg backdrop-blur transition-opacity duration-150 group-focus-within:opacity-100 group-hover:opacity-100 motion-reduce:transition-none"
      >
        projections are estimates from the pool as it stands and move with every
        bet until the market locks. winners divide the pool after a 5% burn, so a
        correct bet can still pay less than its stake.
      </span>
    </span>
  )
}

function phase(market, now) {
  if (market.phase === 'open') {
    const left = secondsUntil(market.locks_at, 0, now)
    return left > 0 ? `betting closes in ${left}s` : 'closing'
  }
  if (market.phase === 'locked') {
    return market.run_status === 'complete' ? 'settling' : 'running'
  }
  return market.phase === 'void' ? 'market void' : 'settled'
}
