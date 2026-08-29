import test from 'node:test'
import assert from 'node:assert/strict'
import {
  CLASSIFIER_VERSION,
  classify,
  evolutionSummary,
  profile,
  SHIFT_WINDOW,
  strideUsed,
} from './species.js'

const BOUNDS = { speed: [0.1, 3.0], size: [0.2, 3.0], metabolism: [0.2, 2.5] }

const behavior = (over = {}) => ({
  movement: 0.5,
  resource_tracking: 0.1,
  reproduction: 0.5,
  resting: 0.5,
  competitor_exposure: 0.006,
  occupied_temperature: 0.44,
  climate_fit: 0.8,
  ...over,
})

const species = (over = {}) => ({
  id: 0,
  name: 'Species A',
  population: 500,
  births: 10,
  deaths: 8,
  mean_energy: 5.0,
  mean_genes: { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 },
  behavior: behavior(),
  mean_brain: 0,
  ...over,
})

test('movement is read against the stride the body can actually take', () => {
  // the same tiles per tick is a stroll for a fast body and a sprint for a slow one
  const fast = strideUsed(species({ mean_genes: { speed: 2.0, size: 1, metabolism: 1, heat_pref: 0.5 }, behavior: behavior({ movement: 0.8 }) }))
  const slow = strideUsed(species({ mean_genes: { speed: 0.8, size: 1, metabolism: 1, heat_pref: 0.5 }, behavior: behavior({ movement: 0.8 }) }))
  assert.ok(slow > fast)
  assert.equal(fast, 0.4)
  // and a body that cannot move at all reports nothing rather than dividing by zero
  assert.equal(strideUsed(species({ mean_genes: { speed: 0, size: 1, metabolism: 1, heat_pref: 0.5 } })), 0)
})

test('the furthest measurement past its own threshold names the species', () => {
  const resting = classify(behavior({ resting: 0.9 }), { stride: 0.2 })
  assert.equal(resting.label, 'energy conserver')
  assert.equal(resting.metric, 'resting pressure')
  assert.equal(resting.version, CLASSIFIER_VERSION)

  const roaming = classify(behavior({ resting: 0.05 }), { stride: 0.95 })
  assert.equal(roaming.label, 'wide-ranging')

  // crowding is rare, so a species that has it wins the label over one that is
  // merely a bit past a common threshold
  const crowded = classify(behavior({ competitor_exposure: 0.4, reproduction: 0.76 }), {
    stride: 0.1,
  })
  assert.equal(crowded.label, 'crowd exposed')
})

test('a species that stood out at nothing is not given a label it did not earn', () => {
  const middling = classify(behavior(), { stride: 0.4 })
  assert.equal(middling.label, 'mixed strategy')
  assert.equal(middling.value, null)
  assert.equal(middling.version, CLASSIFIER_VERSION)
})

test("the server's own shift detector outranks every threshold", () => {
  const events = [{ kind: 'strategy_shift', species_id: 0, epoch: 100, evidence: 'movement +40%' }]
  const mid = classify(behavior({ resting: 0.9 }), { stride: 0.1, events, epoch: 110, id: 0 })
  assert.equal(mid.label, 'strategy shifting')
  assert.equal(mid.evidence, 'movement +40%')

  // and it lapses rather than sticking for the rest of the run
  const later = classify(behavior({ resting: 0.9 }), {
    stride: 0.1,
    events,
    epoch: 101 + SHIFT_WINDOW,
    id: 0,
  })
  assert.equal(later.label, 'energy conserver')

  // another species' shift is not this one's
  const other = classify(behavior({ resting: 0.9 }), { stride: 0.1, events, epoch: 110, id: 1 })
  assert.equal(other.label, 'energy conserver')

  // the ring and the epoch report are separate messages, so a bootstrap holds
  // events from further into the run than the report on screen. one of those
  // has not happened yet as far as this viewer is concerned.
  const ahead = classify(behavior({ resting: 0.9 }), { stride: 0.1, events, epoch: 11, id: 0 })
  assert.equal(ahead.label, 'energy conserver')

  // and when several have landed it is the newest that describes the species
  const both = [
    { kind: 'strategy_shift', species_id: 0, epoch: 100, evidence: 'movement +40%' },
    { kind: 'strategy_shift', species_id: 0, epoch: 120, evidence: 'resting -55%' },
  ]
  assert.equal(
    classify(behavior(), { stride: 0.1, events: both, epoch: 125, id: 0 }).evidence,
    'resting -55%',
  )
})

test('every badge can show its metric, window, baseline and version', () => {
  const founder = { behavior: behavior({ resting: 0.49 }), stride: 0.4, genes: {} }
  const badge = classify(behavior({ resting: 0.9 }), { stride: 0.1, founder })
  assert.equal(badge.metric, 'resting pressure')
  assert.equal(badge.threshold, 0.55)
  assert.equal(badge.baseline, 0.49)
  assert.match(badge.window, /last reported epoch/)
  assert.match(badge.evidence, /0\.90.*0\.55.*0\.49/)
  assert.equal(badge.version, CLASSIFIER_VERSION)
})

test('a card keeps the inherited body and the observed behaviour apart', () => {
  const founder = species({ mean_genes: { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 } })
  const now = species({
    mean_genes: { speed: 1.08, size: 0.96, metabolism: 0.42, heat_pref: 0.44 },
    behavior: behavior({ movement: 0.9, resting: 0.07 }),
  })
  const card = profile(now, founder, BOUNDS, { index: 0 })

  const burn = card.body.find((m) => m.label === 'BURN')
  assert.equal(burn.raw, 0.42)
  assert.equal(burn.founderRaw, 1.2)
  assert.ok(burn.fraction < burn.founderFraction, 'the meter did not follow the gene down')
  // the fraction is against the genetics bounds, not against the founder
  assert.ok(Math.abs(burn.fraction - (0.42 - 0.2) / (2.5 - 0.2)) < 1e-9)

  // climate is an axis, so it is not scaled against anything
  assert.equal(card.climate.raw, 0.44)
  assert.equal(card.climate.fraction, 0.44)

  // and the observed half carries its own founder marker
  const movement = card.observed.find((m) => m.label === 'movement')
  assert.ok(movement.fraction > movement.founderFraction)

  // the realized niche rides the same axis as the preference it is drifting to
  assert.equal(card.climate.occupied, 0.44)

  // there is no single number claiming to rank the species
  assert.equal(card.score, undefined)
  assert.equal(card.rank, undefined)
})

test('a card with no founder yet reads as unchanged rather than as drift', () => {
  const card = profile(species(), null, BOUNDS)
  for (const meter of [...card.body, ...card.observed]) {
    assert.equal(meter.fraction, meter.founderFraction)
  }
  assert.equal(card.strategy.baseline, null)
})

test('an extinct species is marked rather than silently drawn as alive', () => {
  const card = profile(species({ population: 0 }), null, BOUNDS)
  assert.equal(card.extinct, true)
})

test('a meter carries the spread of what it is averaging, where that is drawable', () => {
  const varied = species({
    behavior: behavior({ resting: 0.5 }),
    behavior_variance: { resting: 0.04, movement: 0.09, resource_tracking: 0.16 },
  })
  const card = profile(varied, null, BOUNDS)
  // one standard deviation, not the variance
  assert.equal(card.observed.find((m) => m.label === 'resting').spread, 0.2)
  // a signed track holds two units of range, so its half-width is halved
  assert.equal(card.observed.find((m) => m.label === 'resource tracking').spread, 0.2)
  // movement is drawn as a fraction of stride and the variance is in tiles per
  // tick, so it is deliberately not banded on the wrong axis
  assert.equal(card.observed.find((m) => m.label === 'movement').spread, null)
  // and a run that never sent variance draws no band rather than a zero one
  assert.equal(profile(species(), null, BOUNDS).observed[1].spread, null)
})

test('resource tracking is drawn from the centre, because it has a sign', () => {
  const away = profile(species({ behavior: behavior({ resource_tracking: -0.6 }) }), null, BOUNDS)
  const meter = away.observed.find((m) => m.label === 'resource tracking')
  assert.equal(meter.signed, true)
  assert.equal(meter.raw, -0.6)
  // -1 sits at the left end, 0 in the middle, +1 at the right
  assert.ok(Math.abs(meter.fraction - 0.2) < 1e-9)

  const toward = profile(species({ behavior: behavior({ resource_tracking: 0.6 }) }), null, BOUNDS)
  assert.ok(Math.abs(toward.observed.find((m) => m.label === 'resource tracking').fraction - 0.8) < 1e-9)

  // no other meter picks the centre origin up by accident
  assert.equal(away.observed.find((m) => m.label === 'resting').signed, false)
})

test('a species is only called a tracker once it beats the founder draw by a distance', () => {
  // what a few hundred random policies land on: geometry, not a strategy
  assert.equal(classify(behavior({ resource_tracking: 0.12 }), { stride: 0.2 }).label, 'mixed strategy')
  const evolved = classify(behavior({ resource_tracking: 0.45 }), { stride: 0.2 })
  assert.equal(evolved.label, 'resource tracker')
  assert.equal(evolved.metric, 'resource tracking')
  // and walking away from food is not a strategy that earns the same badge
  assert.equal(classify(behavior({ resource_tracking: -0.9 }), { stride: 0.2 }).label, 'mixed strategy')
})

test('the settled summary reports what moved most, both directions', () => {
  const moved = evolutionSummary({
    founder_genes: { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 },
    final_genes: { speed: 1.08, size: 0.96, metabolism: 0.24, heat_pref: 0.44 },
  })
  assert.equal(moved.length, 2)
  assert.equal(moved[0].label, 'metabolism')
  assert.equal(moved[1].label, 'speed')
  assert.equal(moved[0].from, 1.2)
  assert.equal(moved[0].to, 0.24)
})
