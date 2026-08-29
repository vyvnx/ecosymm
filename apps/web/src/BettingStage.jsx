import { speciesCss } from './render/WorldRenderer.js'
import { outcomeLabels } from './game/coins.js'

/**
 * the betting phase, over a darkened world.
 *
 * what is on the map while a market is open is the run that just ended, and
 * nothing on it will move again - so it goes behind a dim rather than staying
 * lit and inviting a reading it can no longer support. the next world is not
 * shown in its place because it cannot be: its seed is committed to and stays
 * sealed until the market locks, which is the same rule that stops a bettor
 * running it ahead.
 *
 * what fills the gap is the only thing there is to study - how the last
 * markets ended. every run draws its own seed, so the record is a sample of
 * the distribution and never a tell about the run to come. it is the same
 * record for everyone, which is what keeps it information rather than an edge.
 */
export default function BettingStage({ market, form }) {
  if (!market || market.phase !== 'open') return null

  const outcomes = outcomeLabels(market.species)
  const legend = [
    ...outcomes.map((o) => ({
      key: o.key,
      label: o.label,
      count: form.filter((r) => won(r) === o.key).length,
    })),
    { key: null, label: 'died out', count: form.filter((r) => won(r) === null).length },
  ]

  return (
    <>
      {/* the dim itself. it covers the readout as well as the map, because
          those numbers are the finished run's too. */}
      <div className="pointer-events-none absolute inset-0 animate-[outcome-in_700ms_ease-out] bg-neutral-950/85 backdrop-blur-[3px] motion-reduce:animate-none" />

      <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">
        <div className="w-80 max-w-full animate-[outcome-in_500ms_ease-out] text-center text-xs motion-reduce:animate-none">
          <p className="text-sm text-neutral-100">the next world is sealed</p>
          <p className="mt-1 text-neutral-500">
            its seed is committed to now and revealed when betting closes. nobody
            can run it ahead - and nothing behind this belongs to it.
          </p>
          <p className="mt-1 break-all text-neutral-600">{market.commitment.slice(0, 24)}...</p>

          {form.length > 0 && (
            <div className="mt-4 border-t border-neutral-800/80 pt-3">
              <p className="text-neutral-600">the last {form.length} markets</p>
              {/* oldest on the left, so the newest result is the one nearest
                  the market you are about to bet into */}
              <div className="mt-2 flex flex-wrap justify-center gap-1.5">
                {form
                  .slice()
                  .reverse()
                  .map((r) => (
                    <span
                      key={r.market_id}
                      title={name(won(r), outcomes)}
                      className="h-2.5 w-2.5 rounded-full"
                      style={mark(won(r))}
                    />
                  ))}
              </div>
              <div className="mt-3 flex flex-wrap justify-center gap-x-3 gap-y-1 text-neutral-500">
                {legend.map((entry) => (
                  <span key={entry.label} className="flex items-center gap-1.5">
                    <span className="h-2 w-2 rounded-full" style={mark(entry.key)} />
                    {entry.label}
                    <span className="tabular-nums text-neutral-300">{entry.count}</span>
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  )
}

/** what a finished market paid on, or null for one where nothing survived */
const won = (row) => (row.status === 'void' ? null : row.winning_outcome)

/** a species keeps its colour here too; coexistence is both, and void neither */
function mark(outcome) {
  if (outcome === 'species_a') return { background: speciesCss(0) }
  if (outcome === 'species_b') return { background: speciesCss(1) }
  if (outcome === 'coexistence') {
    return { background: `linear-gradient(90deg, ${speciesCss(0)} 50%, ${speciesCss(1)} 50%)` }
  }
  return { boxShadow: 'inset 0 0 0 1px #525252' }
}

const name = (outcome, outcomes) =>
  outcomes.find((o) => o.key === outcome)?.label ?? 'nothing survived'
