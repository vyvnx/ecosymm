import { useRef, useState } from 'react'

const DEFAULTS = { seed: 1234, population_per_species: 500, epochs: 500 }
const COLORS = ['#34d399', '#60a5fa', '#f472b6', '#fbbf24', '#a78bfa']

export default function App() {
  const [params, setParams] = useState(DEFAULTS)
  const [start, setStart] = useState(null)
  const [history, setHistory] = useState([])
  const [status, setStatus] = useState('idle')
  const [done, setDone] = useState(null)
  const socket = useRef(null)

  function run() {
    socket.current?.close()
    setHistory([])
    setDone(null)
    setStart(null)
    setStatus('running')

    const qs = new URLSearchParams(params)
    const ws = new WebSocket(`${location.origin.replace('http', 'ws')}/ws?${qs}`)
    socket.current = ws

    ws.onmessage = (e) => {
      const msg = JSON.parse(e.data)
      if (msg.type === 'config') setStart(msg)
      if (msg.type === 'epoch') setHistory((h) => [...h, msg.report])
      if (msg.type === 'done') {
        setDone(msg)
        setStatus('done')
      }
    }
    ws.onerror = () => setStatus('server unreachable - is `npm run server` up?')
    ws.onclose = () => setStatus((s) => (s === 'running' ? 'stopped' : s))
  }

  const last = history[history.length - 1]
  const species = last?.species ?? start?.species ?? []

  return (
    <div className="min-h-screen bg-neutral-950 text-neutral-200 p-8 font-mono">
      <header className="flex items-baseline gap-4">
        <h1 className="text-2xl font-bold text-emerald-400">ecosym</h1>
        <span className="text-sm text-neutral-500">{status}</span>
        {start && <span className="text-xs text-neutral-600">engine {start.engine}</span>}
      </header>

      {start && (
        <p className="mt-2 text-xs text-neutral-600">
          world {start.world.width}x{start.world.height}, {start.world.habitable_tiles} habitable
          tiles, {start.world.initial_biomass.toFixed(0)} initial biomass
        </p>
      )}

      <div className="mt-6 flex flex-wrap items-end gap-4">
        {Object.keys(DEFAULTS).map((k) => (
          <label key={k} className="flex flex-col gap-1 text-xs uppercase text-neutral-500">
            {k.replace(/_/g, ' ')}
            <input
              type="number"
              value={params[k]}
              onChange={(e) => setParams({ ...params, [k]: Number(e.target.value) })}
              className="w-40 rounded bg-neutral-900 px-3 py-2 text-sm text-neutral-100 outline-none focus:ring-1 focus:ring-emerald-500"
            />
          </label>
        ))}
        <button
          onClick={run}
          className="rounded bg-emerald-600 px-4 py-2 text-sm font-bold text-neutral-950 hover:bg-emerald-500"
        >
          run
        </button>
      </div>

      <div className="mt-8 grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Stat label="epoch" value={last?.epoch ?? 0} />
        <Stat label="total population" value={last?.population ?? 0} />
        <Stat label="biomass" value={last?.biomass} />
        <Stat label="species" value={species.length} />
      </div>

      {/* species stay in the order the server sent them; never keyed by a map */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {species.map((s, i) => (
          <SpeciesCard key={s.id} species={s} color={COLORS[i % COLORS.length]} />
        ))}
      </div>

      <Chart history={history} count={species.length} />

      {done && (
        <div className="mt-6 space-y-1 text-xs text-neutral-500">
          <p>{winnerLine(done.outcome)}</p>
          <p className="text-neutral-600">
            winning is relative: it does not mean the winner is ecologically healthy.
          </p>
          <p>
            replay digest <span className="text-emerald-400">{done.digest}</span> over {done.epochs}{' '}
            epochs — same seed, same digest
          </p>
        </div>
      )}
    </div>
  )
}

function winnerLine(outcome) {
  if (!outcome) return ''
  const name = (id) => outcome.species.find((s) => s.id === id)?.name ?? `species ${id}`
  if (outcome.winner === 'None') return 'winner: none - every species went extinct'
  if (outcome.winner.Species !== undefined) return `winner: ${name(outcome.winner.Species)}`
  return `winner: tie between ${outcome.winner.Tie.map(name).join(', ')}`
}

function Stat({ label, value, digits = 0 }) {
  return (
    <div className="rounded border border-neutral-800 bg-neutral-900/50 p-3">
      <div className="text-[10px] uppercase tracking-wide text-neutral-500">{label}</div>
      <div className="mt-1 text-xl text-neutral-100">
        {value === undefined ? '—' : Number(value).toFixed(digits)}
      </div>
    </div>
  )
}

function SpeciesCard({ species, color }) {
  const genes = species.mean_genes ?? species.founder_genes
  return (
    <div className="rounded border border-neutral-800 bg-neutral-900/50 p-3">
      <div className="flex items-baseline justify-between">
        <span className="text-sm" style={{ color }}>
          {species.name}
        </span>
        <span className="text-xl text-neutral-100">{species.population ?? '—'}</span>
      </div>
      <div className="mt-2 grid grid-cols-2 gap-x-3 text-[11px] text-neutral-500">
        <span>speed {genes.speed.toFixed(3)}</span>
        <span>size {genes.size.toFixed(3)}</span>
        <span>metab {genes.metabolism.toFixed(3)}</span>
        <span>heat {genes.heat_pref.toFixed(3)}</span>
      </div>
      {species.births !== undefined && (
        <div className="mt-2 text-[11px] text-neutral-600">
          +{species.births} / -{species.deaths} this epoch
        </div>
      )}
    </div>
  )
}

function Chart({ history, count }) {
  if (history.length < 2) {
    return <div className="mt-6 h-64 rounded border border-neutral-800 bg-neutral-900/30" />
  }
  const peak = Math.max(...history.flatMap((r) => r.species.map((s) => s.population))) || 1
  const line = (i) =>
    history
      .map(
        (r, x) =>
          `${(x / (history.length - 1)) * 100},${100 - ((r.species[i]?.population ?? 0) / peak) * 100}`
      )
      .join(' ')

  return (
    <div className="mt-6 rounded border border-neutral-800 bg-neutral-900/30 p-4">
      <div className="mb-2 flex justify-between text-xs text-neutral-500">
        <span>population per species</span>
        <span>peak {peak}</span>
      </div>
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-56 w-full">
        {Array.from({ length: count }, (_, i) => (
          <polyline
            key={i}
            points={line(i)}
            fill="none"
            stroke={COLORS[i % COLORS.length]}
            strokeWidth="0.6"
            vectorEffect="non-scaling-stroke"
          />
        ))}
      </svg>
    </div>
  )
}
