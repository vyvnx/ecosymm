// protocol v1 binary decoding. the other half of `apps/server/src/wire.rs`.
//
// no DOM, no WebGL: this file is pure enough to run under `node --test`, which
// is the only way the golden vectors are worth anything.
//
// every frame is validated whole before a single view is handed out. a
// half-applied snapshot would leave the renderer showing a world that never
// existed, so a bad frame throws and the run ends visibly instead.

const HEADER = 12
const VERSION = 1
const KIND_WORLD = 1
const KIND_SNAPSHOT = 2
const MAX_FRAME = 64 * 1024 * 1024

// "ECSY" read as one little-endian u32
const MAGIC = 0x59534345

// the id array is a BigUint64Array view straight onto the received buffer,
// which the browser only allows on an 8-byte boundary
const ID_ALIGNMENT = 8

// bytes per organism: u64 id + u32 species + 3 x f32
const PER_ORGANISM = 24

export function decode(buffer, { speciesCount } = {}) {
  if (!(buffer instanceof ArrayBuffer)) throw new Error('binary frame must be an ArrayBuffer')
  if (buffer.byteLength < HEADER) throw new Error(`frame shorter than its header`)
  if (buffer.byteLength > MAX_FRAME) throw new Error(`frame over ${MAX_FRAME} bytes`)

  const view = new DataView(buffer)
  if (view.getUint32(0, true) !== MAGIC) throw new Error('not an ecosym frame')

  const version = view.getUint16(4, true)
  if (version !== VERSION) throw new Error(`unsupported protocol version ${version}`)
  if (view.getUint8(7) !== 0) throw new Error('v1 declares no flags')

  const declared = view.getUint32(8, true)
  if (declared + HEADER !== buffer.byteLength) {
    throw new Error(`frame declares ${declared} payload bytes, got ${buffer.byteLength - HEADER}`)
  }

  const kind = view.getUint8(6)
  if (kind === KIND_WORLD) return decodeWorld(buffer, view)
  if (kind === KIND_SNAPSHOT) return decodeSnapshot(buffer, view, speciesCount)
  throw new Error(`unknown message kind ${kind}`)
}

function decodeWorld(buffer, view) {
  const width = view.getUint32(12, true)
  const height = view.getUint32(16, true)
  const cells = view.getUint32(20, true)
  if (width === 0 || height === 0) throw new Error('a world needs both dimensions')
  if (width * height !== cells) throw new Error(`${width}x${height} is not ${cells} cells`)
  if (24 + cells * 2 !== buffer.byteLength) throw new Error('world payload length mismatch')

  return {
    kind: 'world',
    width,
    height,
    // zero-copy: single-byte views need no alignment
    fertility: new Uint8Array(buffer, 24, cells),
    temperature: new Uint8Array(buffer, 24 + cells, cells),
  }
}

function decodeSnapshot(buffer, view, speciesCount) {
  const epoch = view.getUint32(12, true)
  const cells = view.getUint32(16, true)
  const count = view.getUint32(20, true)
  if (view.getUint32(24, true) !== 0) throw new Error('reserved field must be zero')

  const resourcesAt = 28
  // arithmetic, not bit twiddling: cells can exceed what `& ~7` survives
  const idsAt = Math.ceil((resourcesAt + cells) / ID_ALIGNMENT) * ID_ALIGNMENT
  if (idsAt + count * PER_ORGANISM !== buffer.byteLength) {
    throw new Error('snapshot payload length mismatch')
  }
  for (let i = resourcesAt + cells; i < idsAt; i++) {
    if (view.getUint8(i) !== 0) throw new Error('alignment padding must be zero')
  }

  const snapshot = {
    kind: 'snapshot',
    epoch,
    cells,
    count,
    resources: new Uint8Array(buffer, resourcesAt, cells),
    ids: new BigUint64Array(buffer, idsAt, count),
    species: new Uint32Array(buffer, idsAt + count * 8, count),
    x: new Float32Array(buffer, idsAt + count * 12, count),
    y: new Float32Array(buffer, idsAt + count * 16, count),
    energy: new Float32Array(buffer, idsAt + count * 20, count),
  }

  for (let i = 0; i < count; i++) {
    if (!Number.isFinite(snapshot.x[i]) || !Number.isFinite(snapshot.y[i])) {
      throw new Error(`organism ${i} has a non-finite position`)
    }
    if (!Number.isFinite(snapshot.energy[i])) {
      throw new Error(`organism ${i} has a non-finite energy`)
    }
    // a species the config never announced has no colour and no card
    if (speciesCount !== undefined && snapshot.species[i] >= speciesCount) {
      throw new Error(`organism ${i} belongs to unknown species ${snapshot.species[i]}`)
    }
  }
  return snapshot
}
