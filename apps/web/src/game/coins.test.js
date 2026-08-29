import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  checkStake,
  formatCoins,
  formatMultiplier,
  outcomeLabels,
  parseStake,
  projection,
  secondsUntil,
  settlementLine,
} from './coins.js'

test('a stake is whole darwin coins or it is not a stake', () => {
  assert.equal(parseStake('25'), 25)
  assert.equal(parseStake(' 25 '), 25)
  assert.equal(parseStake('0'), 0)
  for (const bad of ['12.5', '-3', '', 'ten', '1e3', '25x', null, undefined]) {
    assert.equal(parseStake(bad), null, `${bad} parsed as a stake`)
  }
})

test('the limits a player is shown are the ones the server enforces', () => {
  const rules = { min: 1, max: 100, balance: 40 }
  assert.equal(checkStake(25, rules), null)
  assert.equal(checkStake(null, rules), 'whole coins only')
  assert.equal(checkStake(0, rules), 'minimum 1 DC')
  assert.equal(checkStake(101, rules), 'maximum 100 DC')
  assert.equal(checkStake(41, rules), 'more than you have')
  // replacing a bet releases what it already holds, so that is spendable
  assert.equal(checkStake(70, { ...rules, held: 30 }), null)
  assert.equal(checkStake(71, { ...rules, held: 30 }), 'more than you have')
})

/// the same arithmetic as `Pool::projection`, on the same numbers as its test
test('a projection is the share of the pool one more coin would claim', () => {
  assert.equal(projection([100, 0, 100], 0, 1, 500), 191 / 101)
  // the first coin into an empty market takes the whole pool back, because a
  // burn of five percent of one coin rounds down to nothing
  assert.equal(projection([0, 0, 0], 1, 1, 500), 1)
  // and an outcome nobody can win pays nothing
  assert.equal(projection([0, 0, 0], 0, 0, 500), 0)
})

test('being right can still pay less than the stake', () => {
  // almost the whole pool is on the winner
  assert.ok(projection([1000, 1, 1], 0, 10, 500) < 1)
  assert.equal(formatMultiplier(2.145), '2.15x')
})

test('coins are shown as whole play money', () => {
  assert.equal(formatCoins(1000), '1,000 DC')
  assert.equal(formatCoins(0), '0 DC')
})

/// a sleeping tab, a skewed clock and a throttled timer all have to land on
/// the server's own absolute deadline
test('a countdown is corrected onto the server deadline', () => {
  assert.equal(secondsUntil(1000, 0, 970), 30)
  // this device is 5 minutes behind the server
  assert.equal(secondsUntil(1000, 300, 670), 30)
  // and a deadline already past never counts backwards
  assert.equal(secondsUntil(1000, 0, 1200), 0)
})

test('the three buttons keep the order the server sent species in', () => {
  const labels = outcomeLabels([{ name: 'Species A' }, { name: 'Species B' }])
  assert.deepEqual(
    labels.map((o) => [o.key, o.label]),
    [
      ['species_a', 'Species A'],
      ['coexistence', 'Coexistence'],
      ['species_b', 'Species B'],
    ],
  )
})

test('a settlement is only ever reported as what it did to your own coins', () => {
  const species = [{ name: 'Species A' }, { name: 'Species B' }]
  const settled = { phase: 'settled', winning_outcome: 'species_a', species }

  assert.equal(
    settlementLine(settled, { outcome: 'species_a', stake: 10, payout: 19 }),
    'you won 19 DC on a 10 DC stake',
  )
  assert.equal(
    settlementLine(settled, { outcome: 'species_b', stake: 10, payout: 0 }),
    'you lost 10 DC on Species B',
  )
  assert.equal(settlementLine({ phase: 'void', species }, { stake: 10 }), 'void - your 10 DC came back')

  // who took the market belongs to the run's own result, said once there
  assert.equal(settlementLine(settled, null), null)
  assert.equal(settlementLine({ phase: 'void', species }, null), null)
  assert.equal(settlementLine({ phase: 'open', species }, { stake: 10 }), null)
})
