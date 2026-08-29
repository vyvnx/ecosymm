import test from 'node:test'
import assert from 'node:assert/strict'
import {
  describe,
  FEED_CAPACITY,
  initialFeed,
  markRead,
  reduceFeed,
  resetFeed,
  unread,
} from './events.js'

const event = (id, over = {}) => ({
  run_id: 1,
  event_id: id,
  epoch: id * 3,
  kind: 'strategy_shift',
  severity: 'info',
  species_id: 0,
  title: `event ${id}`,
  evidence: 'because the numbers said so',
  detector_version: 1,
  ...over,
})

const ring = (events, runId = 1) => ({ type: 'telemetry', run_id: runId, events })

test('the server ring is merged by event id, never appended blindly', () => {
  const first = reduceFeed(initialFeed, ring([event(0), event(1)]))
  assert.deepEqual(
    first.events.map((e) => e.event_id),
    [0, 1],
  )

  // the same ring again, plus one: a reconnect must not duplicate anything
  const again = reduceFeed(first, ring([event(0), event(1), event(2)]))
  assert.deepEqual(
    again.events.map((e) => e.event_id),
    [0, 1, 2],
  )

  // and a ring with nothing new in it is the same object, so React does not
  // re-render a feed that did not change
  assert.equal(reduceFeed(again, ring([event(0), event(1), event(2)])), again)
})

test('the feed stops at the server capacity however long the run is', () => {
  let state = initialFeed
  for (let id = 0; id < FEED_CAPACITY * 4; id++) {
    state = reduceFeed(state, ring([event(id)]))
  }
  assert.equal(state.events.length, FEED_CAPACITY)
  // the newest survive, not the oldest
  assert.equal(state.events.at(-1).event_id, FEED_CAPACITY * 4 - 1)
})

test('a bootstrap is history and is not announced, but what follows it is', () => {
  const joined = reduceFeed(initialFeed, ring([event(0), event(1), event(2)]))
  assert.equal(unread(joined).length, 0, 'the backlog was announced as news')

  const live = reduceFeed(joined, ring([event(0), event(1), event(2), event(3)]))
  assert.deepEqual(
    unread(live).map((e) => e.event_id),
    [3],
  )

  const read = markRead(live)
  assert.equal(unread(read).length, 0)
  // marking a feed that is already read changes nothing
  assert.equal(markRead(read), read)
})

test('a ring from another run replaces rather than merges', () => {
  const old = reduceFeed(initialFeed, ring([event(0), event(1)]))
  const next = reduceFeed(old, ring([event(0, { run_id: 2 })], 2))
  assert.equal(next.events.length, 1)
  assert.equal(next.runId, 2)
  assert.equal(unread(next).length, 0, 'a new run announced its own backlog')
})

test('a new run wipes the feed', () => {
  const held = reduceFeed(initialFeed, ring([event(0), event(1)]))
  const wiped = resetFeed(9)
  assert.deepEqual(wiped.events, [])
  assert.equal(wiped.runId, 9)
  assert.notEqual(held.events.length, 0)
})

test('anything that is not a telemetry message is ignored', () => {
  for (const message of [null, undefined, { type: 'epoch' }, { type: 'telemetry' }]) {
    assert.equal(reduceFeed(initialFeed, message), initialFeed)
  }
})

test('an entry is identifiable without its colour', () => {
  const major = describe(event(1, { severity: 'major', kind: 'extinction' }))
  assert.equal(major.mark, '!!')
  assert.equal(major.kind, 'extinction')
  assert.match(major.label, /major extinction, epoch 3/)

  // a world-level event has no species marker at all
  assert.equal(describe(event(2, { species_id: null })).species, null)
  // and an unknown kind still reads as something rather than nothing
  assert.equal(describe(event(3, { kind: 'meteor', severity: 'unheard-of' })).kind, 'meteor')
})
