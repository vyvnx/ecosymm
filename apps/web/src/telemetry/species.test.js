import test from 'node:test'
import assert from 'node:assert/strict'
import { evolutionSummary, traits } from './species.js'

// the two blueprints every market is drawn from, verbatim from
// `default_blueprints` in ecosym-simulation
const A = { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 }
const B = { speed: 0.7, size: 1.0, metabolism: 0.8, heat_pref: 0.38 }

test('the two species read as opposites on the market card', () => {
  assert.deepEqual(traits(A), ['fast', 'hungry', 'likes warmth'])
  assert.deepEqual(traits(B), ['slow', 'thrifty', 'likes cold'])
})

test('a trait on the neutral body is left out rather than named', () => {
  // both start at size 1.0, so neither card claims a build
  assert.ok(!traits(A).includes('large'))
  assert.deepEqual(traits({ speed: 1.0, size: 2.0, metabolism: 1.0, heat_pref: 0.5 }), ['large'])
})

test('an ordinary body still says something', () => {
  assert.deepEqual(traits({ speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 }), [
    'unremarkable',
  ])
})

test('a market with no bodies on it draws nothing', () => {
  assert.deepEqual(traits(undefined), [])
  assert.deepEqual(traits({}), ['unremarkable'])
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
