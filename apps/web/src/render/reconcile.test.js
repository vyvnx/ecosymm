import test from 'node:test'
import assert from 'node:assert/strict'
import { reconcile, nearest, STRIDE, X0, Y0, X1, Y1, ENERGY0, ENERGY1, ALPHA0, ALPHA1, SPECIES } from './reconcile.js'

/** a decoded snapshot, built by hand: [id, species, x, y, energy] per organism */
function snapshot(epoch, organisms) {
  return {
    kind: 'snapshot',
    epoch,
    cells: 0,
    count: organisms.length,
    resources: new Uint8Array(0),
    ids: BigUint64Array.from(organisms.map((o) => o[0])),
    species: Uint32Array.from(organisms.map((o) => o[1])),
    x: Float32Array.from(organisms.map((o) => o[2])),
    y: Float32Array.from(organisms.map((o) => o[3])),
    energy: Float32Array.from(organisms.map((o) => o[4])),
  }
}

/** the batch row for one stable id, found by the endpoint its snapshot gives it */
function find(batch, previous, current, id) {
  const live = index(current, id)
  const snap = live !== null ? current : previous
  const i = live !== null ? live : index(previous, id)
  for (let r = 0; r < batch.count; r++) {
    const at = r * STRIDE
    if (batch.data[at + X1] === snap.x[i] && batch.data[at + Y1] === snap.y[i]) {
      return batch.data.slice(at, at + STRIDE)
    }
  }
  return null
}

function index(snap, id) {
  if (!snap) return null
  for (let i = 0; i < snap.count; i++) if (snap.ids[i] === id) return i
  return null
}

test('a survivor carries both endpoints and stays fully opaque', () => {
  const a = snapshot(1, [[7n, 0, 2, 3, 5]])
  const b = snapshot(2, [[7n, 0, 4, 3.5, 6]])
  const batch = reconcile(a, b, 128, 128)

  assert.equal(batch.count, 1)
  assert.deepEqual([...batch.data], [2, 3, 4, 3.5, 5, 6, 1, 1, 0])
})

test('a birth appears at its first position and fades in', () => {
  const a = snapshot(1, [])
  const b = snapshot(2, [[9n, 1, 10, 20, 4]])
  const batch = reconcile(a, b, 128, 128)

  assert.equal(batch.count, 1)
  const o = batch.data
  assert.equal(o[X0], 10)
  assert.equal(o[X1], 10)
  assert.equal(o[Y0], 20)
  assert.equal(o[Y1], 20)
  assert.equal(o[ALPHA0], 0)
  assert.equal(o[ALPHA1], 1)
  assert.equal(o[ENERGY0], o[ENERGY1], 'a birth has only one known energy')
  assert.equal(o[SPECIES], 1)
})

test('a death holds its last position and fades out', () => {
  const a = snapshot(1, [[3n, 1, 6, 7, 2]])
  const b = snapshot(2, [])
  const batch = reconcile(a, b, 128, 128)

  assert.equal(batch.count, 1)
  assert.deepEqual([...batch.data], [6, 7, 6, 7, 2, 2, 1, 0, 1])
})

test('the first snapshot is shown standing still, not as a world of newborns', () => {
  const first = snapshot(0, [[1n, 0, 5, 5, 3], [2n, 1, 6, 6, 3]])
  const batch = reconcile(null, first, 128, 128)

  assert.equal(batch.count, 2)
  for (let i = 0; i < 2; i++) {
    const o = batch.data.slice(i * STRIDE, (i + 1) * STRIDE)
    assert.equal(o[ALPHA0], 1, 'the opening frame must not fade in')
    assert.equal(o[X0], o[X1])
    assert.equal(o[Y0], o[Y1])
  }
})

test('survivors, births and deaths are reconciled together by id, not by order', () => {
  // the same three organisms, deliberately reordered between snapshots
  const a = snapshot(1, [[1n, 0, 10, 10, 5], [2n, 0, 20, 20, 5], [3n, 1, 30, 30, 5]])
  const b = snapshot(2, [[3n, 1, 31, 30, 4], [4n, 1, 40, 40, 9], [1n, 0, 11, 10, 6]])
  const batch = reconcile(a, b, 128, 128)

  assert.equal(batch.count, 4, 'two survivors, one birth, one death')

  const survivor = find(batch, a, b, 1n)
  assert.deepEqual([survivor[X0], survivor[X1]], [10, 11])
  assert.deepEqual([survivor[ENERGY0], survivor[ENERGY1]], [5, 6])
  assert.equal(survivor[ALPHA0], 1)

  const born = find(batch, a, b, 4n)
  assert.equal(born[ALPHA0], 0)
  assert.equal(born[SPECIES], 1)

  const died = find(batch, a, b, 2n)
  assert.equal(died[ALPHA1], 0)
  assert.deepEqual([died[X0], died[X1]], [20, 20])
})

test('an organism keeps the species of the snapshot it is alive in', () => {
  const a = snapshot(1, [[1n, 0, 1, 1, 1]])
  const b = snapshot(2, [[2n, 3, 2, 2, 2]])
  const batch = reconcile(a, b, 128, 128)
  const born = find(batch, a, b, 2n)
  const died = find(batch, a, b, 1n)
  assert.equal(born[SPECIES], 3)
  assert.equal(died[SPECIES], 0, 'a dead organism keeps its own species while it fades')
})

test('everything born and everything dead are both valid transitions', () => {
  const a = snapshot(1, [[1n, 0, 1, 1, 1], [2n, 0, 2, 2, 1]])
  const b = snapshot(2, [[8n, 0, 8, 8, 1], [9n, 0, 9, 9, 1]])
  const batch = reconcile(a, b, 128, 128)
  assert.equal(batch.count, 4)

  const empty = reconcile(snapshot(1, []), snapshot(2, []), 128, 128)
  assert.equal(empty.count, 0)
})

// the seam is where naive interpolation sends an organism sprinting the wrong
// way across the entire map
test('a survivor crosses the seam the short way on each axis', () => {
  const a = snapshot(1, [
    [1n, 0, 127.5, 5, 1], // east over the seam
    [2n, 0, 5, 0.5, 1], //   north over the seam
    [3n, 0, 127.5, 127.5, 1], // both at once
  ])
  const b = snapshot(2, [
    [1n, 0, 0.5, 5, 1],
    [2n, 0, 5, 127.5, 1],
    [3n, 0, 0.5, 0.5, 1],
  ])
  const batch = reconcile(a, b, 128, 128)

  const east = find(batch, a, b, 1n)
  assert.equal(east[X0], -0.5, 'east crossing must travel one unit, not 127')
  assert.equal(east[X1], 0.5)
  assert.equal(east[Y0], 5)

  const north = find(batch, a, b, 2n)
  assert.equal(north[Y0], 128.5)
  assert.equal(north[X0], 5, 'the untouched axis stays where it was')

  const diagonal = find(batch, a, b, 3n)
  assert.deepEqual([diagonal[X0], diagonal[Y0]], [-0.5, -0.5])
})

test('nearest picks the closer copy of the world and leaves ordinary moves alone', () => {
  assert.equal(nearest(2, 5, 128), 2)
  assert.equal(nearest(127.5, 0.5, 128), -0.5)
  assert.equal(nearest(0.5, 127.5, 128), 128.5)
  // exactly half the world is ambiguous; it must not loop or produce a nan
  assert.equal(nearest(0, 64, 128), 0)
  assert.equal(nearest(5, 5, 0), 5)
})

test('reconciling does not touch either input snapshot', () => {
  const a = snapshot(1, [[1n, 0, 127.5, 3, 5]])
  const b = snapshot(2, [[1n, 0, 0.5, 3, 6], [2n, 0, 9, 9, 1]])
  const before = [[...a.x], [...a.y], [...a.energy], [...b.x], [...b.y], [...b.energy]]

  reconcile(a, b, 128, 128)

  assert.deepEqual([[...a.x], [...a.y], [...a.energy], [...b.x], [...b.y], [...b.energy]], before)
})

test('the batch never exceeds the union of the two snapshots', () => {
  const a = snapshot(1, Array.from({ length: 40 }, (_, i) => [BigInt(i), 0, i, i, 1]))
  const b = snapshot(2, Array.from({ length: 60 }, (_, i) => [BigInt(i + 20), 0, i, i, 1]))
  const batch = reconcile(a, b, 128, 128)

  assert.equal(batch.count, 80, '40 + 60 with 20 shared')
  assert.equal(batch.data.length, batch.count * STRIDE)
  assert.ok(batch.data.length <= (a.count + b.count) * STRIDE)
})

// snapshots arrive faster than transitions finish. reconciliation is
// stateless, so a newer pair simply replaces the older one.
test('a newer snapshot replaces an in-progress transition', () => {
  const a = snapshot(1, [[1n, 0, 0, 0, 1]])
  const b = snapshot(2, [[1n, 0, 10, 0, 1]])
  const c = snapshot(3, [[1n, 0, 20, 0, 1]])

  const first = reconcile(a, b, 128, 128)
  const second = reconcile(b, c, 128, 128)
  assert.deepEqual([first.data[X0], first.data[X1]], [0, 10])
  assert.deepEqual([second.data[X0], second.data[X1]], [10, 20])
})
