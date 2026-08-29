import { useEffect, useRef, useState } from 'react'
import { WorldRenderer } from './render/WorldRenderer.js'

/**
 * the only place React and WebGL meet: one canvas, one renderer, created on
 * mount and disposed on unmount.
 *
 * snapshots reach the renderer through the controller, never through props -
 * a decoded snapshot arriving 15 times a second must not become React state.
 */
export default function WorldView({ controller }) {
  const canvas = useRef(null)
  const [unsupported, setUnsupported] = useState(null)

  useEffect(() => {
    let renderer
    try {
      renderer = new WorldRenderer(canvas.current)
    } catch (e) {
      setUnsupported(e.message)
      return
    }
    setUnsupported(null)
    controller.attach(renderer)
    renderer.start()
    return () => {
      controller.attach(null)
      renderer.dispose()
    }
  }, [controller])

  // the canvas takes the whole page. the world keeps its own aspect ratio
  // inside it, so a wide window letterboxes rather than stretching the map.
  return (
    <>
      <canvas ref={canvas} className="absolute inset-0 block h-full w-full" />
      {unsupported && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 p-6 text-center">
          <p className="text-sm text-amber-400">this browser cannot show the world</p>
          <p className="text-xs text-neutral-500">{unsupported}</p>
        </div>
      )}
    </>
  )
}

/**
 * the seam between the socket and the renderer. it lives in a ref, outside
 * React state, so a snapshot can reach the GPU without a re-render.
 */
export function createController() {
  let renderer = null
  let world = null
  let species = []

  const controller = {
    attach(next) {
      renderer = next
      // a canvas remounting mid-run (a dev-mode double effect, a layout
      // change) gets the world it missed rather than an empty context
      if (renderer && world) renderer.setWorld(world, species)
    },
    setWorld(next, speciesMetadata) {
      world = next
      species = speciesMetadata
      renderer?.setWorld(world, species)
    },
    setSnapshot(snapshot) {
      renderer?.setSnapshot(snapshot)
    },
    reset() {
      world = null
      species = []
    },
    stats() {
      return renderer?.stats() ?? null
    },
  }

  // a console handle for the render counters: `__ecosym.stats()` reports frame
  // times, draw calls and uploads without a re-render and without React ever
  // holding per-frame state
  if (typeof window !== 'undefined') window.__ecosym = controller
  return controller
}
