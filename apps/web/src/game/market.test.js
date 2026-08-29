import { test } from 'node:test'
import assert from 'node:assert/strict'
import { canBet, fromAnotherRun, initialMarket, reduceMarket } from './market.js'

const market = (over = {}) => ({
  run_id: 1,
  market_id: 1,
  revision: 1,
  phase: 'open',
  locks_at: 1000,
  server_time: 970,
  ...over,
})

const feed = (messages, state = initialMarket, now = 970) =>
  messages.reduce((s, m) => reduceMarket(s, m, now), state)

test('a bootstrap only counts when its own sync_end closes it', () => {
  const done = feed([
    { type: 'sync_begin', run_id: 1, server_time: 970 },
    { type: 'market_open', market: market() },
    { type: 'sync_end', run_id: 1, server_time: 970 },
  ])
  assert.equal(done.synced, true)
  assert.equal(done.syncing, null)
  assert.equal(done.resync, 0)
  assert.equal(done.market.market_id, 1)
})

test('a sync_end from another run asks for a fresh bootstrap', () => {
  const mismatched = feed([
    { type: 'sync_begin', run_id: 1 },
    { type: 'sync_end', run_id: 2 },
  ])
  assert.equal(mismatched.synced, false)
  assert.equal(mismatched.resync, 1)
})

test('a duplicate revision changes nothing and an older market is ignored', () => {
  const state = feed([{ type: 'market_open', market: market({ revision: 4 }) }])
  const again = reduceMarket(state, { type: 'market_pool', market: market({ revision: 4 }) }, 970)
  assert.equal(again.market, state.market)
  assert.equal(again.resync, 0)

  const older = reduceMarket(
    state,
    { type: 'market_open', market: market({ market_id: 0, revision: 99 }) },
    970,
  )
  assert.equal(older.market.market_id, 1, 'a market that is already over came back')
})

test('a revision that moves backwards asks for a fresh bootstrap', () => {
  const state = feed([{ type: 'market_open', market: market({ revision: 9 }) }])
  const backwards = reduceMarket(
    state,
    { type: 'market_pool', market: market({ revision: 8 }) },
    970,
  )
  assert.equal(backwards.resync, 1)
  assert.equal(backwards.market.revision, 9, 'stale state was applied anyway')
})

test('the next market replaces the last one', () => {
  const state = feed([
    { type: 'market_open', market: market({ revision: 3 }) },
    { type: 'market_settled', market: market({ revision: 4, phase: 'settled' }) },
    { type: 'market_open', market: market({ run_id: 2, market_id: 2, revision: 1 }) },
  ])
  assert.equal(state.market.market_id, 2)
  assert.equal(state.market.phase, 'open')
  assert.equal(state.resync, 0)
})

test('an http response that arrives behind the socket is dropped, not escalated', () => {
  const state = feed([{ type: 'market_open', market: market({ revision: 9 }) }])
  const behind = reduceMarket(
    state,
    { type: 'market_fetched', market: market({ revision: 8 }) },
    970,
  )
  assert.equal(behind.market.revision, 9)
  assert.equal(behind.resync, 0, 'a stale fetch asked for a bootstrap')

  const ahead = reduceMarket(
    state,
    { type: 'market_fetched', market: market({ revision: 10 }) },
    970,
  )
  assert.equal(ahead.market.revision, 10)
})

test('the clock offset comes from the server, not from this device', () => {
  // the server says 970 while this device thinks it is 1270
  const state = reduceMarket(initialMarket, { type: 'market_open', market: market() }, 1270)
  assert.equal(state.offset, -300)
  // a message with no server time leaves the estimate alone
  assert.equal(reduceMarket(state, { type: 'epoch' }, 5000).offset, -300)
})

test('a message from a run this client is not watching is spotted', () => {
  assert.equal(fromAnotherRun({ type: 'epoch', run_id: 2 }, 1), true)
  assert.equal(fromAnotherRun({ type: 'epoch', run_id: 1 }, 1), false)
  // before the client knows which run it is on, nothing is stale
  assert.equal(fromAnotherRun({ type: 'epoch', run_id: 2 }, null), false)
  assert.equal(fromAnotherRun({ type: 'snapshot' }, 1), false)
})

test('betting is off unless everything about the moment allows it', () => {
  const ok = {
    market: market(),
    account: { id: 1 },
    synced: true,
    connected: true,
    submitting: false,
  }
  assert.equal(canBet(ok, 970), true)
  assert.equal(canBet({ ...ok, account: null }, 970), false, 'signed out')
  assert.equal(canBet({ ...ok, synced: false }, 970), false, 'mid-bootstrap')
  assert.equal(canBet({ ...ok, connected: false }, 970), false, 'disconnected')
  assert.equal(canBet({ ...ok, submitting: true }, 970), false, 'already submitting')
  assert.equal(canBet({ ...ok, market: market({ phase: 'locked' }) }, 970), false, 'locked')
  assert.equal(canBet(ok, 1001), false, 'past the deadline')
})
