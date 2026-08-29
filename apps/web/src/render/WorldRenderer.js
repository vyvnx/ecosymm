// the whole renderer: two draw calls a frame, no scene graph, no library.
//
// one textured quad for the world and one point draw for every organism alive
// or just dead, whatever the species count. an ordinary animation frame moves
// a single uniform - buffers and textures are only touched when a snapshot
// arrives, which is what keeps 10,000 organisms inside a frame budget.
//
// exactly one copy of the world is ever drawn. the map is a torus and tiling it
// to fill a wide screen was tried and removed: two identical landmasses side by
// side read as two worlds, and a spectator cannot tell which one they are
// watching. the world is centred and the canvas keeps whatever is left over.

import { reconcile, STRIDE } from './reconcile.js'

// `ecosym_world::HABITABLE_FERTILITY`: below this a tile is sea. it decides
// the coastline here exactly as it decides passability there.
const HABITABLE = 0.1

// a snapshot's transition lasts about as long as the last gap between
// snapshots, held inside these bounds so a stall does not freeze the world and
// a burst does not turn movement into a twitch
const MIN_TRANSITION = 40
const MAX_TRANSITION = 1000

// retina is worth it, 4x display scaling is not
const MAX_PIXEL_RATIO = 2

const FLOAT = 4
const VERTEX_BYTES = STRIDE * FLOAT

// the first colours match the species cards; past them the hue circle is
// walked by the golden angle so any number of species stays distinguishable
const BASE_COLORS = [
  [52, 211, 153],
  [96, 165, 250],
  [244, 114, 182],
  [251, 191, 36],
  [167, 139, 250],
]

export function speciesColor(i) {
  if (i < BASE_COLORS.length) return BASE_COLORS[i]
  return hsl(((i - BASE_COLORS.length) * 137.508 + 20) % 360, 0.62, 0.62)
}

export const speciesCss = (i) => `rgb(${speciesColor(i).join(', ')})`

function hsl(h, s, l) {
  const c = (1 - Math.abs(2 * l - 1)) * s
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1))
  const m = l - c / 2
  const [r, g, b] = [
    [c, x, 0],
    [x, c, 0],
    [0, c, x],
    [0, x, c],
    [x, 0, c],
    [c, 0, x],
  ][Math.floor(h / 60) % 6]
  return [r, g, b].map((v) => Math.round((v + m) * 255))
}

const WORLD_VERTEX = `#version 300 es
out vec2 v_uv;
void main() {
  // four corners of a triangle strip, straight off the vertex index
  vec2 p = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
  // row 0 of the field is the top of the screen, as the wire wrote it
  v_uv = vec2(p.x, 1.0 - p.y);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`

const WORLD_FRAGMENT = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_terrain;    // r fertility, g temperature
uniform sampler2D u_before;     // resource fullness, previous snapshot
uniform sampler2D u_after;      // resource fullness, current snapshot
uniform float u_alpha;
out vec4 color;

void main() {
  vec2 ground = texture(u_terrain, v_uv).rg;
  float fertility = ground.r;
  float temperature = ground.g;
  float fullness = mix(texture(u_before, v_uv).r, texture(u_after, v_uv).r, u_alpha);

  // the sea is flat and cold, and nothing that happens on land can be
  // mistaken for it
  vec3 sea = mix(vec3(0.027, 0.063, 0.118), vec3(0.055, 0.129, 0.216),
                 smoothstep(0.0, ${HABITABLE.toFixed(3)}, fertility));

  // grazed-out land is bare ground, not water: it keeps its warmth
  vec3 bare = mix(vec3(0.180, 0.176, 0.180), vec3(0.290, 0.235, 0.180), temperature);
  vec3 lush = mix(vec3(0.184, 0.427, 0.322), vec3(0.478, 0.529, 0.180), temperature);
  vec3 land = mix(bare, lush, fullness);

  float shore = smoothstep(${(HABITABLE - 0.02).toFixed(3)}, ${(HABITABLE + 0.02).toFixed(3)}, fertility);
  color = vec4(mix(sea, land, shore), 1.0);
}`

const ORGANISM_VERTEX = `#version 300 es
layout(location = 0) in vec2 a_from;
layout(location = 1) in vec2 a_to;
layout(location = 2) in vec2 a_energy;
layout(location = 3) in vec2 a_fade;
layout(location = 4) in float a_species;

uniform float u_alpha;
uniform vec2 u_world;
uniform float u_scale;
uniform sampler2D u_palette;

out vec3 v_color;
out float v_fade;

void main() {
  // reconciliation may have pushed the start point outside the world so the
  // seam is crossed the short way; wrapping here brings it back
  vec2 p = mod(mix(a_from, a_to, u_alpha), u_world);
  gl_Position = vec4(p.x / u_world.x * 2.0 - 1.0, 1.0 - p.y / u_world.y * 2.0, 0.0, 1.0);

  float vigour = clamp(mix(a_energy.x, a_energy.y, u_alpha) / 10.0, 0.0, 1.0);
  gl_PointSize = u_scale * mix(0.75, 1.2, vigour);
  v_color = texelFetch(u_palette, ivec2(int(a_species), 0), 0).rgb * mix(0.6, 1.0, vigour);
  v_fade = mix(a_fade.x, a_fade.y, u_alpha);
}`

const ORGANISM_FRAGMENT = `#version 300 es
precision highp float;
in vec3 v_color;
in float v_fade;
out vec4 color;

void main() {
  float r = length(gl_PointCoord - 0.5);
  float disc = 1.0 - smoothstep(0.36, 0.5, r);
  if (disc <= 0.0) discard;
  color = vec4(v_color, disc * v_fade);
}`

export class WorldRenderer {
  constructor(canvas) {
    const gl = canvas.getContext('webgl2', {
      alpha: false,
      antialias: false,
      depth: false,
      powerPreference: 'high-performance',
    })
    if (!gl) throw new Error('this browser has no WebGL2 context')

    this.canvas = canvas
    this.gl = gl
    this.lost = false
    this.raf = null

    // everything needed to rebuild the picture after a context loss
    this.world = null
    this.terrainBytes = null
    this.paletteBytes = null
    this.previous = null
    this.current = null
    this.batch = null

    this.arrivedAt = 0
    this.transition = 0

    this.viewport = [0, 0, 1, 1]
    this.pointScale = 3
    this.needsResize = true

    this.counters = { frames: 0, draws: 0, uploads: 0, organisms: 0 }
    this.frameTimes = new Float32Array(180)
    this.frameAt = 0
    this.lastFrame = 0

    this.onLost = (e) => {
      e.preventDefault()
      this.lost = true
    }
    this.onRestored = () => {
      this.build()
      this.restore()
      this.lost = false
    }
    canvas.addEventListener('webglcontextlost', this.onLost)
    canvas.addEventListener('webglcontextrestored', this.onRestored)

    this.observer = new ResizeObserver(() => {
      this.needsResize = true
    })
    this.observer.observe(canvas)

    this.build()
  }

  /** every GPU object, created here and nowhere else so restore can repeat it */
  build() {
    const gl = this.gl
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.disable(gl.DEPTH_TEST)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)

    this.worldProgram = program(gl, WORLD_VERTEX, WORLD_FRAGMENT)
    this.organismProgram = program(gl, ORGANISM_VERTEX, ORGANISM_FRAGMENT)
    this.worldUniforms = uniforms(gl, this.worldProgram, ['u_terrain', 'u_before', 'u_after', 'u_alpha'])
    this.organismUniforms = uniforms(gl, this.organismProgram, ['u_alpha', 'u_world', 'u_scale', 'u_palette'])

    this.terrain = texture(gl, gl.LINEAR, gl.REPEAT)
    this.before = texture(gl, gl.LINEAR, gl.REPEAT)
    this.after = texture(gl, gl.LINEAR, gl.REPEAT)
    this.palette = texture(gl, gl.NEAREST, gl.CLAMP_TO_EDGE)

    this.buffer = gl.createBuffer()
    this.vao = gl.createVertexArray()
    gl.bindVertexArray(this.vao)
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer)
    // [from.xy, to.xy, energy.01, fade.01, species] - the order `reconcile` writes
    for (const [location, size, offset] of [
      [0, 2, 0],
      [1, 2, 8],
      [2, 2, 16],
      [3, 2, 24],
      [4, 1, 32],
    ]) {
      gl.enableVertexAttribArray(location)
      gl.vertexAttribPointer(location, size, gl.FLOAT, false, VERTEX_BYTES, offset)
    }
    gl.bindVertexArray(null)

    this.maxPointSize = gl.getParameter(gl.ALIASED_POINT_SIZE_RANGE)[1]
  }

  /** the static half of a run. safe to call again for a new run. */
  setWorld(world, species) {
    const cells = world.width * world.height
    if (world.fertility.length !== cells) throw new Error('world field does not match its size')

    this.world = { width: world.width, height: world.height }
    this.terrainBytes = new Uint8Array(cells * 2)
    for (let i = 0; i < cells; i++) {
      this.terrainBytes[i * 2] = world.fertility[i]
      this.terrainBytes[i * 2 + 1] = world.temperature[i]
    }

    const count = Math.max(species.length, 1)
    this.paletteBytes = new Uint8Array(count * 4)
    for (let i = 0; i < count; i++) {
      const [r, g, b] = speciesColor(i)
      this.paletteBytes.set([r, g, b, 255], i * 4)
    }

    this.previous = null
    this.current = null
    this.batch = null
    this.transition = 0
    this.needsResize = true
    this.uploadWorld()
  }

  /** a decoded snapshot. everything else in here follows from this call. */
  setSnapshot(snapshot) {
    if (!this.world) throw new Error('a snapshot arrived before the world')
    if (snapshot.cells !== this.world.width * this.world.height) {
      throw new Error('snapshot does not match the world it belongs to')
    }

    const now = performance.now()
    if (this.current) {
      const observed = clamp(now - this.arrivedAt, MIN_TRANSITION, MAX_TRANSITION)
      // one gap of lag, smoothed: the next transition is paced by the last one
      this.transition = this.transition ? this.transition * 0.7 + observed * 0.3 : observed
    }
    this.arrivedAt = now

    this.previous = this.current
    this.current = snapshot
    this.batch = reconcile(this.previous, this.current, this.world.width, this.world.height)
    this.upload()
  }

  upload() {
    const gl = this.gl
    if (this.lost) return

    // the previous frame's texture becomes the "before" of this transition,
    // so only one of the two is ever written
    const spent = this.before
    this.before = this.after
    this.after = spent
    this.uploadResources(this.after, this.current.resources)
    if (!this.previous) this.uploadResources(this.before, this.current.resources)

    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer)
    gl.bufferData(gl.ARRAY_BUFFER, this.batch.data, gl.DYNAMIC_DRAW)

    this.counters.uploads++
    this.counters.organisms = this.current.count
  }

  uploadWorld() {
    const gl = this.gl
    if (this.lost || !this.world) return
    const { width, height } = this.world

    gl.bindTexture(gl.TEXTURE_2D, this.terrain)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG8, width, height, 0, gl.RG, gl.UNSIGNED_BYTE, this.terrainBytes)

    gl.bindTexture(gl.TEXTURE_2D, this.palette)
    const species = this.paletteBytes.length / 4
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, species, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, this.paletteBytes)
  }

  uploadResources(target, bytes) {
    const gl = this.gl
    const { width, height } = this.world
    gl.bindTexture(gl.TEXTURE_2D, target)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, bytes)
  }

  /** rebuild the picture from what is still retained, after a context loss */
  restore() {
    if (!this.world) return
    this.uploadWorld()
    if (this.current) {
      this.uploadResources(this.after, this.current.resources)
      this.uploadResources(this.before, (this.previous ?? this.current).resources)
      const gl = this.gl
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer)
      gl.bufferData(gl.ARRAY_BUFFER, this.batch.data, gl.DYNAMIC_DRAW)
    }
    this.needsResize = true
  }

  /** backing store and the centred world rectangle, on layout change only */
  resize() {
    const gl = this.gl
    const ratio = Math.min(window.devicePixelRatio || 1, MAX_PIXEL_RATIO)
    const width = Math.max(1, Math.round(this.canvas.clientWidth * ratio))
    const height = Math.max(1, Math.round(this.canvas.clientHeight * ratio))
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width
      this.canvas.height = height
    }

    const aspect = this.world ? this.world.width / this.world.height : width / height
    const fit = Math.min(width / aspect, height)
    const [w, h] = [Math.round(fit * aspect), Math.round(fit)]
    this.viewport = [Math.round((width - w) / 2), Math.round((height - h) / 2), w, h]
    this.pointScale = this.world
      ? clamp((w / this.world.width) * 0.85, 1.5, this.maxPointSize)
      : 3
    gl.viewport(0, 0, width, height)
    this.needsResize = false
  }

  start() {
    if (this.raf !== null) return
    this.lastFrame = performance.now()
    const tick = (now) => {
      this.raf = requestAnimationFrame(tick)
      this.draw(now)
    }
    this.raf = requestAnimationFrame(tick)
  }

  stop() {
    if (this.raf !== null) cancelAnimationFrame(this.raf)
    this.raf = null
  }

  draw(now) {
    const gl = this.gl
    if (this.lost) return
    if (this.needsResize) this.resize()

    this.frameTimes[this.frameAt] = now - this.lastFrame
    this.frameAt = (this.frameAt + 1) % this.frameTimes.length
    this.lastFrame = now
    this.counters.frames++

    gl.viewport(0, 0, this.canvas.width, this.canvas.height)
    gl.clearColor(0.04, 0.04, 0.045, 1)
    gl.clear(gl.COLOR_BUFFER_BIT)
    if (!this.world || !this.current) return

    // where this transition has got to. the first snapshot has no transition,
    // so it lands on its target immediately.
    const alpha = this.transition > 0 ? Math.min(1, (now - this.arrivedAt) / this.transition) : 1
    gl.viewport(...this.viewport)

    gl.useProgram(this.worldProgram)
    bind(gl, 0, this.terrain, this.worldUniforms.u_terrain)
    bind(gl, 1, this.before, this.worldUniforms.u_before)
    bind(gl, 2, this.after, this.worldUniforms.u_after)
    gl.uniform1f(this.worldUniforms.u_alpha, alpha)
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)

    gl.useProgram(this.organismProgram)
    bind(gl, 3, this.palette, this.organismUniforms.u_palette)
    gl.uniform1f(this.organismUniforms.u_alpha, alpha)
    gl.uniform2f(this.organismUniforms.u_world, this.world.width, this.world.height)
    gl.uniform1f(this.organismUniforms.u_scale, this.pointScale)
    gl.bindVertexArray(this.vao)
    gl.drawArrays(gl.POINTS, 0, this.batch.count)
    gl.bindVertexArray(null)

    this.counters.draws += 2
  }

  /** development counters. never React state, never read per frame. */
  stats() {
    const samples = [...this.frameTimes].filter((t) => t > 0).sort((a, b) => a - b)
    const at = (q) => samples[Math.min(samples.length - 1, Math.floor(samples.length * q))] ?? 0
    return {
      ...this.counters,
      drawn: this.batch ? this.batch.count : 0,
      frameP50: at(0.5),
      frameP95: at(0.95),
    }
  }

  dispose() {
    const gl = this.gl
    this.stop()
    this.observer.disconnect()
    this.canvas.removeEventListener('webglcontextlost', this.onLost)
    this.canvas.removeEventListener('webglcontextrestored', this.onRestored)

    for (const t of [this.terrain, this.before, this.after, this.palette]) gl.deleteTexture(t)
    gl.deleteBuffer(this.buffer)
    gl.deleteVertexArray(this.vao)
    gl.deleteProgram(this.worldProgram)
    gl.deleteProgram(this.organismProgram)
    this.previous = null
    this.current = null
    this.batch = null
  }
}

function program(gl, vertexSource, fragmentSource) {
  const p = gl.createProgram()
  gl.attachShader(p, shader(gl, gl.VERTEX_SHADER, vertexSource))
  gl.attachShader(p, shader(gl, gl.FRAGMENT_SHADER, fragmentSource))
  gl.linkProgram(p)
  if (!gl.getProgramParameter(p, gl.LINK_STATUS) && !gl.isContextLost()) {
    throw new Error(`shader link failed: ${gl.getProgramInfoLog(p)}`)
  }
  return p
}

function shader(gl, type, source) {
  const s = gl.createShader(type)
  gl.shaderSource(s, source)
  gl.compileShader(s)
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) {
    throw new Error(`shader compile failed: ${gl.getShaderInfoLog(s)}`)
  }
  return s
}

function uniforms(gl, program, names) {
  return Object.fromEntries(names.map((n) => [n, gl.getUniformLocation(program, n)]))
}

function texture(gl, filter, wrap) {
  const t = gl.createTexture()
  gl.bindTexture(gl.TEXTURE_2D, t)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrap)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrap)
  return t
}

function bind(gl, unit, texture, location) {
  gl.activeTexture(gl.TEXTURE0 + unit)
  gl.bindTexture(gl.TEXTURE_2D, texture)
  gl.uniform1i(location, unit)
}

function clamp(v, lo, hi) {
  return Math.min(hi, Math.max(lo, v))
}
