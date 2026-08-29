import { speciesCss } from './render/WorldRenderer.js'
import { SEGMENTS } from './telemetry/species.js'

/**
 * one card per species: the body it inherited, and what it was last seen
 * doing. the two halves are drawn differently on purpose - a gene meter and a
 * behaviour meter are not the same kind of fact, and stacking them into one
 * bar would invent a species rating the simulation never measured.
 *
 * every meter carries a founder marker as well as a current one, which is the
 * whole point: evolution is the distance between the two, and reading it
 * should not require knowing what 1.2 metabolism means.
 *
 * nothing here is a prediction or a hint at the market. the badge says
 * "currently behaving as" and can show exactly what it measured to say it.
 */
export default function SpeciesProfiles({ cards }) {
  if (cards.length === 0) {
    return <p className="text-neutral-600">no species yet</p>
  }
  return (
    <div className="space-y-4">
      {cards.map((card) => (
        <Card key={card.id} card={card} />
      ))}
    </div>
  )
}

function Card({ card }) {
  return (
    <section aria-label={card.name} className="space-y-1.5">
      <div className="flex items-baseline gap-2">
        <span
          aria-hidden
          className="h-2 w-2 shrink-0 rounded-full"
          style={{ background: speciesCss(card.index) }}
        />
        <span className="truncate text-neutral-200">{card.name}</span>
        <span className="ml-auto shrink-0 tabular-nums text-neutral-500">
          {card.extinct ? 'extinct' : card.population.toLocaleString()}
        </span>
      </div>

      <Badge strategy={card.strategy} extinct={card.extinct} />

      <div className="space-y-0.5">
        {card.body.map((meter) => (
          <Segmented key={meter.label} meter={meter} index={card.index} />
        ))}
        <Climate climate={card.climate} index={card.index} />
      </div>

      <div className="space-y-0.5 pt-1">
        <p className="text-neutral-600">recent behaviour</p>
        {card.observed.map((meter) => (
          <Bar key={meter.label} meter={meter} index={card.index} />
        ))}
      </div>
    </section>
  )
}

/**
 * what it is doing now, and how that was decided. the disclosure is not
 * decoration: a label with no metric, window, baseline and version behind it
 * is an opinion, and this page does not get to have those.
 */
function Badge({ strategy, extinct }) {
  if (extinct) return <p className="text-neutral-600">no behaviour left to report</p>
  return (
    <details className="group">
      <summary className="flex min-h-6 cursor-pointer list-none items-baseline gap-1 text-neutral-400 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500">
        <span className="text-neutral-600">currently behaving as</span>
        <span className="text-emerald-400">{strategy.label}</span>
        <span aria-hidden className="text-neutral-700 group-open:hidden">
          ?
        </span>
      </summary>
      <dl className="mt-1 space-y-0.5 border-l border-neutral-800 pl-2 text-neutral-600">
        <Fact term="measured" value={strategy.metric} />
        <Fact term="over" value={strategy.window} />
        {strategy.evidence && <Fact term="because" value={strategy.evidence} />}
        <Fact term="classifier" value={`v${strategy.version}`} />
      </dl>
    </details>
  )
}

function Fact({ term, value }) {
  return (
    <div className="flex gap-2">
      <dt className="w-16 shrink-0 text-neutral-700">{term}</dt>
      <dd className="min-w-0 flex-1 break-words text-neutral-500">{value}</dd>
    </div>
  )
}

/**
 * an inherited trait, in five segments over the range genetics allows it. the
 * founder marker sits where the species started, so a filled meter that has
 * moved left of its own marker is a species getting cheaper to run.
 */
function Segmented({ meter, index }) {
  const filled = Math.round(meter.fraction * SEGMENTS)
  return (
    <Row
      label={meter.label}
      title={meter.means}
      from={meter.founderRaw}
      to={meter.raw}
      aria={`${meter.label}, ${fmt(meter.founderRaw)} at founding, now ${fmt(meter.raw)}`}
    >
      <span aria-hidden className="relative flex h-2 w-full gap-px">
        {Array.from({ length: SEGMENTS }, (_, i) => (
          <span
            key={i}
            className="flex-1 rounded-[1px]"
            style={{ background: i < filled ? speciesCss(index) : '#262626' }}
          />
        ))}
        <Marker at={meter.founderFraction} />
      </span>
    </Row>
  )
}

/**
 * heat preference, drawn as the axis it is. neither end is better and there is
 * no "more" of it - a species moving along it is finding a latitude, and a
 * power bar would say it was winning.
 */
function Climate({ climate, index }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-14 shrink-0 text-neutral-600">CLIMATE</span>
      <span
        className="relative flex h-2 flex-1 items-center rounded-[1px]"
        style={{ background: 'linear-gradient(90deg, #1e3a5f, #3f2d1e)' }}
      >
        <span
          aria-hidden
          className="absolute h-2.5 w-1 -translate-x-1/2 rounded-[1px]"
          style={{ left: `${climate.fraction * 100}%`, background: speciesCss(index) }}
        />
        <Marker at={climate.founderFraction} />
        <span
          aria-hidden
          className="absolute h-2 w-2 -translate-x-1/2 rotate-45 border border-neutral-300"
          style={{ left: `${climate.occupied * 100}%` }}
        />
      </span>
      <span className="sr-only">
        heat preference {fmt(climate.founderRaw)} at founding, now {fmt(climate.raw)}, on a cold to
        warm axis; last living at {fmt(climate.occupied)}
      </span>
      <span aria-hidden className="w-20 shrink-0 text-right tabular-nums text-neutral-600">
        {climate.raw < 0.4 ? 'cooler' : climate.raw > 0.6 ? 'warmer' : 'middling'}
      </span>
    </div>
  )
}

/**
 * an observed tendency. most are 0..1 and fill from the left, continuous
 * because crowding lives near zero. a signed one fills from the centre in
 * whichever direction it went, because half a bar of "walked away from the
 * food" is not half a bar of tracking it.
 */
function Bar({ meter, index }) {
  const origin = meter.signed ? 0.5 : 0
  const left = Math.min(origin, meter.fraction)
  const width = Math.abs(meter.fraction - origin)
  return (
    <Row
      label={meter.label}
      from={meter.founderRaw}
      to={meter.raw}
      aria={`${meter.label}, ${fmt(meter.founderRaw)} at founding, now ${fmt(meter.raw)}${
        meter.signed ? ', where zero is no measurable alignment' : ''
      }${meter.spread === null ? '' : `, spread ${fmt(meter.spread)} across organism-ticks`}`}
    >
      <span aria-hidden className="relative flex h-1.5 w-full rounded-[1px] bg-neutral-800">
        {meter.signed && (
          <span className="absolute left-1/2 h-full w-px -translate-x-1/2 bg-neutral-700" />
        )}
        <span
          className="absolute h-full rounded-[1px]"
          style={{
            left: `${left * 100}%`,
            width: `${width * 100}%`,
            background: speciesCss(index),
          }}
        />
        {/* one standard deviation either side of the mean, across
            organism-ticks. a wide band is a species doing several things at
            once - or one organism doing them at different moments, which this
            cannot tell apart and does not claim to. */}
        {meter.spread !== null && (
          <span
            className="absolute top-1/2 h-px -translate-y-1/2 opacity-40"
            style={{
              left: `${Math.max(0, meter.fraction - meter.spread) * 100}%`,
              width: `${(Math.min(1, meter.fraction + meter.spread) - Math.max(0, meter.fraction - meter.spread)) * 100}%`,
              background: speciesCss(index),
            }}
          />
        )}
        <Marker at={meter.founderFraction} />
      </span>
    </Row>
  )
}

/** where the species started. the whole story is the gap to the fill. */
function Marker({ at }) {
  return (
    <span
      aria-hidden
      className="absolute top-1/2 h-3 w-px -translate-x-1/2 -translate-y-1/2 bg-neutral-400"
      style={{ left: `${at * 100}%` }}
    />
  )
}

function Row({ label, title, from, to, aria, children }) {
  return (
    <div className="flex items-center gap-2" title={title}>
      <span aria-hidden className="w-14 shrink-0 truncate text-neutral-600">
        {label}
      </span>
      <span className="sr-only">{aria}</span>
      <span className="min-w-0 flex-1">{children}</span>
      <span aria-hidden className="w-20 shrink-0 text-right tabular-nums text-neutral-600">
        {fmt(from)} &rarr; <span className="text-neutral-300">{fmt(to)}</span>
      </span>
    </div>
  )
}

const fmt = (v) => (Number.isFinite(v) ? v.toFixed(v < 0.1 && v > 0 ? 3 : 2) : '-')
