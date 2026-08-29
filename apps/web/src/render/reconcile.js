// two snapshots in, one GPU upload batch out.
//
// this is where identity becomes motion: an organism present in both snapshots
// travels, one only in the new snapshot is born, one only in the old is dead.
// the browser reconciles by stable id and never by array position - the
// simulation is free to reorder its populations, and does.
//
// pure javascript, no WebGL. `requestAnimationFrame` never comes in here.

// floats per organism in the interleaved buffer
export const STRIDE = 9

// field offsets within one organism's stride
export const X0 = 0
export const Y0 = 1
export const X1 = 2
export const Y1 = 3
export const ENERGY0 = 4
export const ENERGY1 = 5
export const ALPHA0 = 6
export const ALPHA1 = 7
export const SPECIES = 8

/**
 * @param previous decoded snapshot, or null for the very first one
 * @param current  decoded snapshot
 * @returns {{data: Float32Array, count: number}} interleaved endpoints
 */
export function reconcile(previous, current, width, height) {
  // ponytail: a fresh buffer per snapshot, ~15 times a second. pool it only if
  // a profile shows the allocation, not because it looks wasteful.
  const capacity = current.count + (previous ? previous.count : 0)
  const data = new Float32Array(capacity * STRIDE)

  const survivors = new Map()
  if (previous) {
    for (let i = 0; i < previous.count; i++) survivors.set(previous.ids[i], i)
  }

  let n = 0
  for (let i = 0; i < current.count; i++) {
    const at = n++ * STRIDE
    const was = survivors.get(current.ids[i])
    const found = was !== undefined
    if (found) survivors.delete(current.ids[i])

    // no previous snapshot means the first frame, which is shown standing
    // still rather than as a world where everything was just born
    const born = !found && previous !== null
    data[at + X0] = found ? nearest(previous.x[was], current.x[i], width) : current.x[i]
    data[at + Y0] = found ? nearest(previous.y[was], current.y[i], height) : current.y[i]
    data[at + X1] = current.x[i]
    data[at + Y1] = current.y[i]
    data[at + ENERGY0] = found ? previous.energy[was] : current.energy[i]
    data[at + ENERGY1] = current.energy[i]
    data[at + ALPHA0] = born ? 0 : 1
    data[at + ALPHA1] = 1
    data[at + SPECIES] = current.species[i]
  }

  // whatever is left in the map was alive last time and is not alive now. it
  // holds its last position and fades, for this one transition only.
  for (const i of survivors.values()) {
    const at = n++ * STRIDE
    data[at + X0] = previous.x[i]
    data[at + Y0] = previous.y[i]
    data[at + X1] = previous.x[i]
    data[at + Y1] = previous.y[i]
    data[at + ENERGY0] = previous.energy[i]
    data[at + ENERGY1] = previous.energy[i]
    data[at + ALPHA0] = 1
    data[at + ALPHA1] = 0
    data[at + SPECIES] = previous.species[i]
  }

  // a view, not a copy: the tail of the capacity estimate is never uploaded
  return { data: data.subarray(0, n * STRIDE), count: n }
}

/**
 * the world is a torus, so an organism that stepped over the seam has two
 * possible paths and only one of them is the one it took. move the start point
 * onto whichever copy of the world sits nearest the end point, per axis, and
 * let the shader wrap the result back into bounds.
 */
export function nearest(from, to, extent) {
  if (!(extent > 0)) return from
  const delta = to - from
  if (delta > extent / 2) return from + extent
  if (delta < -extent / 2) return from - extent
  return from
}
