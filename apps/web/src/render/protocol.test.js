import test from 'node:test'
import assert from 'node:assert/strict'
import { decode } from './protocol.js'

// the same literals live in `apps/server/src/wire.rs`. if a field order, an
// endianness, a quantisation step or the id alignment moves, both sides fail
// at once instead of one side drifting silently.
const GOLDEN_WORLD =
  '45435359' + '0100' + '01' + '00' + '14000000' + // header: ECSY v1 world, 20 byte payload
  '02000000' + '02000000' + '04000000' + // 2 x 2, 4 cells
  '0080ff40' + // fertility 0, 0.5, 1, 0.25
  'ffbf8000' //   temperature 1, 0.75, 0.5, 0

const GOLDEN_SNAPSHOT =
  '45435359' + '0100' + '02' + '00' + '44000000' + // header: ECSY v1 snapshot, 68 bytes
  '07000000' + '04000000' + '02000000' + '00000000' + // epoch 7, 4 cells, 2 organisms
  '0080ff80' + // fullness: sea reads empty, then 0.5, 1, 0.5
  // 28 + 4 cells lands on 32, so this frame needs no alignment padding
  '0500000002000000' + '0900000000000000' + // ids 2^33 + 5 and 9
  '00000000' + '01000000' + // species 0 and 1
  '0000c03f' + '0000803e' + // x: -0.5 and 2.25 wrapped into a width of 2
  '0000c03f' + '00000000' + // y
  '0000a040' + '0000803e' //   energy 5 and 0.25

function bytes(hex) {
  const out = new Uint8Array(hex.length / 2)
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return out
}

const frame = (hex) => bytes(hex).buffer

/** the golden frame with one byte replaced, for the header rejection table */
function patched(hex, offset, value) {
  const b = bytes(hex)
  b[offset] = value
  return b.buffer
}

test('the world golden vector decodes field for field', () => {
  const world = decode(frame(GOLDEN_WORLD))
  assert.equal(world.kind, 'world')
  assert.equal(world.width, 2)
  assert.equal(world.height, 2)
  assert.deepEqual([...world.fertility], [0, 128, 255, 64])
  assert.deepEqual([...world.temperature], [255, 191, 128, 0])
})

test('the snapshot golden vector decodes field for field', () => {
  const snap = decode(frame(GOLDEN_SNAPSHOT))
  assert.equal(snap.kind, 'snapshot')
  assert.equal(snap.epoch, 7)
  assert.equal(snap.cells, 4)
  assert.equal(snap.count, 2)
  assert.deepEqual([...snap.resources], [0, 128, 255, 128])
  assert.deepEqual([...snap.species], [0, 1])
  assert.deepEqual([...snap.x], [1.5, 0.25])
  assert.deepEqual([...snap.y], [1.5, 0])
  assert.deepEqual([...snap.energy], [5, 0.25])
})

// ids past 2^53 survive as BigInt, which is the whole reason they are not
// decoded as numbers
test('organism ids stay exact above the safe integer range', () => {
  const snap = decode(frame(GOLDEN_SNAPSHOT))
  assert.equal(snap.ids[0], 8589934597n)
  assert.equal(snap.ids[1], 9n)
  assert.ok(snap.ids instanceof BigUint64Array)
})

test('an asymmetric world keeps row-major order, so orientation is testable', () => {
  const world = decode(frame(GOLDEN_WORLD))
  // row 0 is the first two cells and row 1 the last two, never transposed
  assert.deepEqual([...world.fertility.slice(0, 2)], [0, 128])
  assert.deepEqual([...world.fertility.slice(2)], [255, 64])
})

test('every header field is checked', () => {
  assert.throws(() => decode(patched(GOLDEN_WORLD, 0, 0x46)), /not an ecosym frame/)
  assert.throws(() => decode(patched(GOLDEN_WORLD, 4, 2)), /unsupported protocol version/)
  assert.throws(() => decode(patched(GOLDEN_WORLD, 6, 9)), /unknown message kind/)
  assert.throws(() => decode(patched(GOLDEN_WORLD, 7, 1)), /no flags/)
  assert.throws(() => decode(patched(GOLDEN_WORLD, 8, 21)), /payload bytes/)
  assert.throws(() => decode(new ArrayBuffer(4)), /shorter than its header/)
  assert.throws(() => decode(new Uint8Array(32)), /must be an ArrayBuffer/)
})

test('truncation at any boundary is refused', () => {
  for (const hex of [GOLDEN_WORLD, GOLDEN_SNAPSHOT]) {
    for (let cut = 2; cut < hex.length; cut += 2) {
      assert.throws(() => decode(frame(hex.slice(0, cut))), `truncating to ${cut / 2} bytes decoded`)
    }
  }
})

test('trailing bytes are refused, not ignored', () => {
  assert.throws(() => decode(frame(GOLDEN_SNAPSHOT + '00')), /payload/)
})

test('a world whose dimensions do not match its cell count is refused', () => {
  assert.throws(() => decode(patched(GOLDEN_WORLD, 12, 3)), /is not 4 cells/)
  assert.throws(() => decode(patched(GOLDEN_WORLD, 12, 0)), /needs both dimensions/)
})

test('a nonzero reserved field is refused', () => {
  assert.throws(() => decode(patched(GOLDEN_SNAPSHOT, 24, 1)), /reserved field/)
})

// padding only appears when 28 + cells misses an 8-byte boundary, so this
// needs a 3-cell world rather than the 2 x 2 golden one
test('nonzero alignment padding is refused', () => {
  const clean =
    '45435359' + '0100' + '02' + '00' + '2c000000' +
    '01000000' + '03000000' + '01000000' + '00000000' +
    '0080ff' + '00' + // 3 resource bytes end at 31, so one padding byte follows
    '0100000000000000' + '00000000' + '0000803f' + '0000803f' + '0000803f'
  assert.equal(decode(frame(clean)).count, 1)
  assert.throws(() => decode(patched(clean, 31, 1)), /alignment padding/)
})

test('an empty snapshot decodes to no organisms', () => {
  const empty =
    '45435359' + '0100' + '02' + '00' + '14000000' +
    '00000000' + '04000000' + '00000000' + '00000000' + '00000000'
  const snap = decode(frame(empty))
  assert.equal(snap.count, 0)
  assert.equal(snap.ids.length, 0)
})

test('an organism count larger than the frame is refused', () => {
  assert.throws(() => decode(patched(GOLDEN_SNAPSHOT, 20, 200)), /payload length mismatch/)
})

test('a species the config never announced is refused', () => {
  assert.doesNotThrow(() => decode(frame(GOLDEN_SNAPSHOT), { speciesCount: 2 }))
  assert.throws(() => decode(frame(GOLDEN_SNAPSHOT), { speciesCount: 1 }), /unknown species 1/)
})

test('a non-finite position is refused', () => {
  // overwrite the first x with a nan
  const b = bytes(GOLDEN_SNAPSHOT)
  new DataView(b.buffer).setFloat32(56, NaN, true)
  assert.throws(() => decode(b.buffer), /non-finite position/)

  const e = bytes(GOLDEN_SNAPSHOT)
  new DataView(e.buffer).setFloat32(72, Infinity, true)
  assert.throws(() => decode(e.buffer), /non-finite energy/)
})

test('decoded arrays are views onto the received buffer, not copies', () => {
  const buffer = frame(GOLDEN_SNAPSHOT)
  const snap = decode(buffer)
  assert.equal(snap.x.buffer, buffer)
  assert.equal(snap.ids.buffer, buffer)
  assert.equal(snap.ids.byteOffset % 8, 0, 'BigUint64Array needs an 8-byte offset')
})
