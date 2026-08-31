import { speciesCss } from './render/WorldRenderer.js'
import { outcomeLabels } from './game/coins.js'
import { traits } from './telemetry/species.js'

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
 *
 * the two bodies are the other half of it. they are the founder bodies the
 * sealed run will start from, which is the only thing about that run anyone
 * is allowed to know before it locks.
 */
export default function BettingStage({ market, form }) {
  if (!market || market.phase !== 'open') return null

  const outcomes = outcomeLabels(market.species)
  const [a, b] = market.species
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
          <p className="text-sm text-neutral-100">world #{market.market_id} is sealed</p>
          {/* the seal itself. it says nothing to read and everything to check:
              the same hash turns up beside the seed once the market locks. */}
          <p className="mt-1 break-all text-neutral-600">{market.commitment.slice(0, 24)}...</p>

          {/* what is actually being bet on, in words rather than gene values:
              a card nobody can read in thirty seconds is not information. */}
          {a && b && (
            <div className="mt-4 border-t border-neutral-800/80 pt-3">
              <p className="text-neutral-600">up next</p>
              <div className="mt-2 grid grid-cols-[1fr_auto_1fr] items-start gap-2">
                <Body species={a} index={0} />
                <span className="self-center text-neutral-700">vs</span>
                <Body species={b} index={1} />
              </div>
            </div>
          )}

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

/** one side of the matchup: who it is, and the body it starts the run with */
function Body({ species, index }) {
  return (
    <div className="min-w-0">
      <p className="flex items-center justify-center gap-1.5 text-neutral-200">
        <span
          aria-hidden
          className="h-2 w-2 shrink-0 rounded-full"
          style={{ background: speciesCss(index) }}
        />
        <span className="truncate">{species.name}</span>
      </p>
      <ul className="mt-1 space-y-0.5 text-neutral-500">
        {traits(species.genes).map((word) => (
          <li key={word}>{word}</li>
        ))}
      </ul>
    </div>
  )
}

/** what a finished market paid on, or null for one where nothing survived */
const won = (row) => (row.status === 'void' ? null : row.winning_outcome)

/** a species keeps its colour here too; coexistence is both, and void neither */
function mark(outcome) {
  if (outcome === 'species_a') return { background: speciesCss(0) }
  if (outcome === 'species_b') return { background: speciesCss(1) }
  // a split dot is unreadable at this size, so coexistence is one species
  // ringed by the other: both colours, still one glance
  if (outcome === 'coexistence') {
    return { background: speciesCss(0), boxShadow: `inset 0 0 0 2px ${speciesCss(1)}` }
  }
  return { boxShadow: 'inset 0 0 0 1px #525252' }
}

const name = (outcome, outcomes) =>
  outcomes.find((o) => o.key === outcome)?.label ?? 'nothing survived'
