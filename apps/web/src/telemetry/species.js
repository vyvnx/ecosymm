/**
 * a species card, derived from what already crosses the wire.
 *
 * two halves that must not be confused with each other: the **body** a species
 * inherited, and the **behaviour** it was last observed doing. one is genetic
 * and drifts over generations, the other is what its policies did in the epoch
 * just reported, and reading them as one number is exactly the mistake a
 * "power score" makes. there is no combined score here and there is not going
 * to be one.
 *
 * nothing here is a prediction, a fitness reading or a hint about the market.
 * every meter is descriptive, every badge says what it measured and over what,
 * and `CLASSIFIER_VERSION` moves whenever a threshold does - a label with no
 * version attached is an opinion.
 */

/** bumped whenever a threshold or rule below changes */
export const CLASSIFIER_VERSION = 1

/**
 * the food channel, and the one meter here that is signed.
 *
 * it is not a pressure the policy reports about itself. it is the alignment
 * between where the organism actually went and where the food actually was,
 * in -1..1: +1 walked straight at it, -1 walked straight away, 0 either no
 * measurable alignment or nothing to align with. drawn from the centre for
 * that reason - a bar filling from the left would read -0.4 as "a little bit
 * of tracking" when it is the opposite of tracking.
 */
const TRACKING_LABEL = 'resource tracking'

/** the player-facing name for each inherited trait, and its wire key */
const BODY = [
  { label: 'PACE', key: 'speed', means: 'how far one step reaches' },
  { label: 'BULK', key: 'size', means: 'lifespan and appetite together' },
  {
    label: 'BURN',
    key: 'metabolism',
    means: 'intake and upkeep together - higher is both, not better',
  },
]

/** how many segments a body meter is drawn in */
export const SEGMENTS = 5

/**
 * the observed half. `movement` is tiles per tick, which only means something
 * against the stride the body can actually take, so it is reported as the
 * fraction of a full stride spent - that is the number selection is acting on.
 */
const OBSERVED = [
  { label: 'movement', read: (b, stride) => stride },
  { label: TRACKING_LABEL, read: (b) => b.resource_tracking, signed: true },
  { label: 'breeding', read: (b) => b.reproduction },
  { label: 'resting', read: (b) => b.resting },
  { label: 'crowding', read: (b) => b.competitor_exposure },
  // the fraction of a tile's food this species' bodies could actually keep
  // where they were standing. low is a species living off its preference.
  { label: 'climate fit', read: (b) => b.climate_fit },
]

/**
 * what a species is currently behaving *as*, by threshold over its last
 * reported epoch. each rule is scored by how far past its own threshold the
 * measurement sits, and the furthest wins - so one species does not collect
 * the same label every run just because it is first in a list.
 */
const RULES = [
  {
    label: 'crowd exposed',
    metric: 'competitor exposure',
    read: (b) => b.competitor_exposure,
    at: 0.05,
  },
  // every threshold sits above 0.5, because 0.5 is what a few hundred random
  // founder policies average out to. a label handed to a species for having no
  // opinion yet would be describing the draw, not the species.
  { label: 'energy conserver', metric: 'resting pressure', read: (b) => b.resting, at: 0.55 },
  {
    label: 'reproduction-heavy',
    metric: 'breeding pressure',
    read: (b) => b.reproduction,
    at: 0.75,
  },
  { label: 'wide-ranging', metric: 'movement', read: (b, stride) => stride, at: 0.7 },
  // a signed channel, so its own no-opinion point is 0 and not 0.5. a few
  // hundred random founder policies land around 0.1 by accident of geometry;
  // survivors that evolved the behaviour run three to five times that.
  {
    label: 'resource tracker',
    metric: TRACKING_LABEL,
    read: (b) => b.resource_tracking,
    at: 0.35,
  },
]

/** epochs a `strategy_shift` event keeps a species labelled as shifting */
export const SHIFT_WINDOW = 30

/**
 * the badge, and the working behind it.
 *
 * "currently behaving as" and never "is": these are thresholds over one
 * reported epoch, and a species that crosses one back tomorrow was not lying
 * today. a recent `strategy_shift` from the server's own detector outranks
 * every threshold, because a species mid-move does not have a settled label.
 */
export function classify(behavior, { stride, founder, events = [], epoch = 0, id = null } = {}) {
  // the newest shift, and only one that has actually happened yet. the ring
  // and the epoch report arrive as separate messages, so a bootstrap routinely
  // holds events from further into the run than the report on screen - a shift
  // the viewer has not reached is not a shift the viewer is watching.
  const shifting = events.findLast(
    (e) =>
      e.kind === 'strategy_shift' &&
      e.species_id === id &&
      e.epoch <= epoch &&
      epoch - e.epoch <= SHIFT_WINDOW,
  )
  if (shifting) {
    return {
      label: 'strategy shifting',
      metric: 'the server detector',
      value: null,
      threshold: null,
      baseline: null,
      window: `a strategy shift at epoch ${shifting.epoch}, within ${SHIFT_WINDOW} epochs`,
      version: CLASSIFIER_VERSION,
      evidence: shifting.evidence,
    }
  }

  let best = null
  for (const rule of RULES) {
    const value = rule.read(behavior, stride)
    const score = rule.at > 0 ? value / rule.at : 0
    if (score >= 1 && (best === null || score > best.score)) best = { rule, value, score }
  }
  if (best === null) {
    return {
      label: 'mixed strategy',
      metric: 'every measured tendency',
      value: null,
      threshold: null,
      baseline: null,
      window: 'the last reported epoch',
      version: CLASSIFIER_VERSION,
      evidence: 'nothing it did stood out far enough to name',
    }
  }
  const baseline = founder ? best.rule.read(founder.behavior, founder.stride) : null
  return {
    label: best.rule.label,
    metric: best.rule.metric,
    value: best.value,
    threshold: best.rule.at,
    baseline,
    window: 'the last reported epoch',
    version: CLASSIFIER_VERSION,
    evidence:
      baseline === null
        ? `${best.rule.metric} ${fixed(best.value)}, over ${fixed(best.rule.at)}`
        : `${best.rule.metric} ${fixed(best.value)}, over ${fixed(best.rule.at)}, from ${fixed(baseline)} at founding`,
  }
}

/**
 * one card. `founder` is the run's first reported epoch plus the founding
 * genes from the config, so the markers are what the species started as rather
 * than a rolling average of itself.
 */
export function profile(current, founder, bounds, { index = 0, events = [], epoch = 0 } = {}) {
  const stride = strideUsed(current)
  // the wire carries variance; what is drawable beside a mean is its own unit,
  // so it is shown as a standard deviation. it is spread across organism-ticks
  // and nothing more: one organism behaving differently at different moments
  // reads exactly like two organisms behaving differently, and this cannot
  // tell them apart.
  const spread = current.behavior_variance ?? null
  const founderView = founder
    ? { behavior: founder.behavior, stride: strideUsed(founder), genes: founder.mean_genes }
    : null

  return {
    id: current.id,
    index,
    name: current.name,
    population: current.population,
    extinct: current.population === 0,
    strategy: classify(current.behavior, {
      stride,
      founder: founderView,
      events,
      epoch,
      id: current.id,
    }),
    body: BODY.map((trait) => {
      const range = bounds?.[trait.key] ?? [0, 1]
      const now = current.mean_genes[trait.key]
      const was = founderView?.genes?.[trait.key] ?? now
      return {
        ...trait,
        raw: now,
        founderRaw: was,
        fraction: within(now, range),
        founderFraction: within(was, range),
        range,
      }
    }),
    // the climate meter is an axis, not a power bar: neither end is better,
    // and a species moving along it is finding a latitude rather than winning
    climate: {
      label: 'CLIMATE',
      raw: current.mean_genes.heat_pref,
      founderRaw: founderView?.genes?.heat_pref ?? current.mean_genes.heat_pref,
      fraction: clamp(current.mean_genes.heat_pref),
      founderFraction: clamp(founderView?.genes?.heat_pref ?? current.mean_genes.heat_pref),
      // the realized niche: the mean temperature of the tiles it was actually
      // standing on, which is not the same fact as the temperature it prefers.
      // the gap between the two is a species living somewhere it would rather
      // not, and there is nowhere better within reach.
      occupied: clamp(current.behavior.occupied_temperature),
    },
    observed: OBSERVED.map((meter) => {
      const now = meter.read(current.behavior, stride)
      const was = founderView ? meter.read(founderView.behavior, founderView.stride) : now
      const scale = meter.signed ? bipolar : clamp
      return {
        label: meter.label,
        signed: meter.signed === true,
        raw: now,
        founderRaw: was,
        fraction: scale(now),
        founderFraction: scale(was),
        spread: halfWidth(meter, spread),
      }
    }),
    energy: current.mean_energy,
  }
}

/**
 * the fraction of a full stride this species spent per tick. distance alone
 * says nothing without the body that carried it - a slow species at full
 * effort and a fast one strolling report the same tiles.
 */
export function strideUsed(species) {
  const reach = species.mean_genes?.speed ?? 0
  return reach > 0 ? clamp(species.behavior.movement / reach) : 0
}

/**
 * founder to final, for the settled card. one line per species, and it says
 * what moved rather than who deserved to win.
 */
export function evolutionSummary(result) {
  const moved = [
    ['speed', result.founder_genes.speed, result.final_genes.speed],
    ['metabolism', result.founder_genes.metabolism, result.final_genes.metabolism],
    ['heat_pref', result.founder_genes.heat_pref, result.final_genes.heat_pref],
  ]
    .map(([label, from, to]) => ({ label, from, to, moved: Math.abs(to - from) }))
    .sort((a, b) => b.moved - a.moved)
  return moved.slice(0, 2)
}

/**
 * a meter's spread as a half-width on its own 0..1 track, or `null`.
 *
 * `movement` is missing on purpose: that meter is drawn as a fraction of the
 * body's stride, and the variance on the wire is in tiles per tick, so a band
 * from it would be the right number on the wrong axis. a signed track carries
 * two units of range in one, so its half-width is halved to match.
 */
const SPREAD_KEY = {
  'resource tracking': 'resource_tracking',
  breeding: 'reproduction',
  resting: 'resting',
  crowding: 'competitor_exposure',
  'climate fit': 'climate_fit',
}

function halfWidth(meter, variance) {
  const v = variance?.[SPREAD_KEY[meter.label]]
  if (!(v > 0)) return null
  return Math.min(0.5, Math.sqrt(v) / (meter.signed ? 2 : 1))
}

const clamp = (v) => Math.min(1, Math.max(0, Number.isFinite(v) ? v : 0))

/** a signed -1..1 measurement placed on a 0..1 track, so 0 sits in the middle */
const bipolar = (v) => clamp(((Number.isFinite(v) ? v : 0) + 1) / 2)

const within = (v, [low, high]) => (high > low ? clamp((v - low) / (high - low)) : 0)

const fixed = (v) => (v === null ? '-' : v.toFixed(v < 0.1 ? 3 : 2))
