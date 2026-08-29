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
      <div className="w-full max-w-md rounded border border-neutral-800/80 bg-neutral-950/80 p-3 text-xs backdrop-blur">
        <div className="flex items-baseline gap-2">
          <span className="text-neutral-300">{phase(market, corrected)}</span>
          <span className="ml-auto text-neutral-600">
            {market.pools.reduce((a, b) => a + b, 0).toLocaleString()} DC in the pool
          </span>
        </div>

        {settled ? (
          <p className="mt-2 text-neutral-100">{settled}</p>
        ) : (
          <>
            <div className="mt-2 flex items-center gap-2">
              <label htmlFor="stake" className="text-neutral-600">
                stake
              </label>
              <input
                id="stake"
                inputMode="numeric"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                disabled={!account}
                className="min-h-11 w-20 rounded border border-neutral-800 bg-neutral-900/60 px-2 py-1 text-right tabular-nums text-neutral-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-50 sm:min-h-9"
              />
              <span className="text-neutral-600">DC</span>
              <span className="ml-auto tabular-nums text-neutral-500">
                {account ? `${formatCoins(account.balance)} available` : 'sign in to bet'}
              </span>
            </div>

            <div className="mt-2 grid gap-1.5 sm:grid-cols-3">
              {outcomes.map((outcome, index) => {
                const mine = bet?.outcome === outcome.key
                return (
                  <button
                    key={outcome.key}
                    type="button"
                    onClick={() => place(outcome.key)}
                    disabled={!open || Boolean(invalid)}
                    aria-pressed={mine}
                    className={`flex min-h-11 items-center gap-2 rounded border px-2 py-1.5 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-40 sm:flex-col sm:items-start sm:gap-0.5 ${
                      mine
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

            <p className="mt-2 text-neutral-600">
              {invalid ? (
                <span className="text-amber-400">{invalid}</span>
              ) : error ? (
                <span className="text-amber-400">{error}</span>
              ) : bet ? (
                `holding ${formatCoins(bet.stake)} on ${
                  outcomes.find((o) => o.key === bet.outcome)?.label ?? bet.outcome
                } - tap another to replace it`
              ) : (
                'estimates only. they move with every bet until the market locks, and a correct bet can return less than its stake.'
              )}
            </p>
          </>
        )}
      </div>
    </div>
  )
}

function phase(market, now) {
  if (market.phase === 'open') {
    const left = secondsUntil(market.locks_at, 0, now)
    return left > 0 ? `betting closes in ${left}s` : 'closing'
  }
  if (market.phase === 'locked') {
    return market.run_status === 'complete' ? 'settling' : 'running - bets are locked'
  }
  return market.phase === 'void' ? 'market void' : 'settled'
}
