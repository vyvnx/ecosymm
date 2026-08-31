import { useEffect, useState } from 'react'
import { speciesCss } from './render/WorldRenderer.js'
import { formatCoins, payoutLine, settlementLine } from './game/coins.js'
import { evolutionSummary } from './telemetry/species.js'

/**
 * the payout phase: the only thing on screen between a run ending and the
 * next market opening.
 *
 * everything about the finished run is said here and nowhere else - who took
 * it, what it did to both species, where the pot went, and the seed and digest
 * that let anyone check the whole thing afterwards. when the next market
 * opens this leaves and the betting panel comes back, so the two phases never
 * share the screen.
 */
export default function RunResult({ done, market, bet, seedHex, status }) {
  const pot = payoutLine(market)
  const paid = useCountUp(pot?.amount ?? 0)
  const mine = settlementLine(market, bet)
  const won = bet && bet.outcome === market?.winning_outcome

  return (
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">
      <div className="w-80 max-w-full animate-[outcome-in_400ms_ease-out] rounded border border-neutral-800/80 bg-neutral-950/85 p-4 text-xs backdrop-blur motion-reduce:animate-none">
        <div className="flex items-baseline justify-between text-neutral-600">
          <span>{done.epochs.toLocaleString()} epochs</span>
          <span>{status}</span>
        </div>

        <p className="mt-2 text-sm text-neutral-100">{winnerLine(done.outcome)}</p>

        {mine && (
          <p className={`mt-1 ${won ? 'text-emerald-400' : 'text-neutral-500'}`}>{mine}</p>
        )}

        {/* what the run did to each species, in the order everything else
            uses, so a colour means one species everywhere */}
        <div className="mt-3 space-y-0.5 tabular-nums">
          {done.outcome.species.map((s, i) => (
            <div key={s.id} className="flex items-baseline gap-2">
              <span
                className="h-2 w-2 shrink-0 rounded-full"
                style={{ background: speciesCss(i) }}
              />
              <span className="text-neutral-300">{s.name}</span>
              <span className="ml-auto text-neutral-600">{s.initial.toLocaleString()} &rarr;</span>
              <span className="w-12 text-right text-neutral-100">
                {s.final_population.toLocaleString()}
              </span>
              <span
                className={`w-14 text-right ${
                  s.final_population >= s.initial ? 'text-neutral-500' : 'text-neutral-700'
                }`}
              >
                {change(s)}
              </span>
            </div>
          ))}
        </div>

        {/* what the run did to the bodies, not just the head count. the two
            traits that moved furthest, so a reader sees the selection rather
            than only its score line. */}
        <div className="mt-2 space-y-0.5 text-neutral-600">
          {done.outcome.species.map((s, i) => (
            <p key={s.id} className="flex items-baseline gap-2 tabular-nums">
              <span
                aria-hidden
                className="h-1.5 w-1.5 shrink-0 rounded-full"
                style={{ background: speciesCss(i) }}
              />
              <span className="truncate">
                {evolutionSummary(s)
                  .map((t) => `${t.label} ${t.from.toFixed(2)} \u2192 ${t.to.toFixed(2)}`)
                  .join('  ·  ')}
              </span>
            </p>
          ))}
        </div>

        {pot && (
          <div className="mt-3 border-t border-neutral-800/80 pt-2">
            <p className="tabular-nums">
              <span className="text-neutral-500">{pot.prefix} </span>
              <span className={amountClass(pot.tone)}>{formatCoins(paid)}</span>
            </p>
            <p className="text-neutral-600">{pot.suffix}</p>
          </div>
        )}

        {/* one identifier per line: a u64 seed is long enough to wrap a shared
            line mid-phrase */}
        <div className="mt-3 text-neutral-600">
          <p>seed {seedHex}</p>
          <p>
            digest <span className="text-emerald-400">{done.digest}</span>
          </p>
        </div>
      </div>
    </div>
  )
}

/**
 * counts a number up to where it lands, once. it is the pot arriving, so it
 * should look like arriving rather than appearing - but a reader who has
 * asked for less motion gets the number, not the trip.
 */
function useCountUp(target, ms = 1100) {
  const [value, setValue] = useState(target)

  useEffect(() => {
    const still = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
    if (still || target <= 0) {
      setValue(target)
      return
    }
    let frame
    const started = performance.now()
    const step = (now) => {
      const t = Math.min(1, (now - started) / ms)
      // fast out of the gate, settling onto the last coin
      setValue(Math.round(target * (1 - (1 - t) ** 3)))
      if (t < 1) frame = requestAnimationFrame(step)
    }
    frame = requestAnimationFrame(step)
    return () => cancelAnimationFrame(frame)
  }, [target, ms])

  return value
}

function amountClass(tone) {
  if (tone === 'pay') return 'payout-sweep font-bold'
  return tone === 'burn' ? 'text-amber-400' : 'text-neutral-300'
}

function change(s) {
  if (!s.initial) return ''
  const percent = Math.round((s.final_population / s.initial - 1) * 100)
  return `${percent >= 0 ? '+' : ''}${percent}%`
}

function winnerLine(outcome) {
  if (!outcome) return ''
  const name = (id) => outcome.species.find((s) => s.id === id)?.name ?? `species ${id}`
  if (outcome.winner === 'None') return 'no winner, everything died'
  if (outcome.winner.Species !== undefined) return `winner ${name(outcome.winner.Species)}`
  return `tie between ${outcome.winner.Tie.map(name).join(', ')}`
}
