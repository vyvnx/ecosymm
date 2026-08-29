import { useEffect, useRef, useState } from 'react'
import { speciesCss } from './render/WorldRenderer.js'
import { describe, unread } from './telemetry/events.js'

/**
 * the run's own event log. read-only, and structurally so: there is no
 * composer, no reply, no author and no client-to-server call anywhere in this
 * file. every line came from a detector on the server reading a finished
 * epoch, which is why every line can show its working.
 *
 * it borrows the scanning rhythm of a chat window and none of its affordances.
 * what makes that safe is the evidence line: an entry that cannot say what it
 * measured would be narration, and narration is what a spectator cannot check.
 *
 * `role="log"` already announces politely, and the bootstrap is rendered
 * before the region is live, so a late joiner's backlog is history rather than
 * fifty announcements.
 */
export default function LiveLog({ feed, onSeen, label = 'run events' }) {
  const scroller = useRef(null)
  const [following, setFollowing] = useState(true)
  const behind = unread(feed).length

  // auto-follow only while the viewer is already at the bottom. reading back
  // through the run must not be interrupted by the run continuing.
  useEffect(() => {
    const box = scroller.current
    if (!box || !following) return
    box.scrollTop = box.scrollHeight
    if (behind > 0) onSeen()
  }, [feed.events, following, behind, onSeen])

  function onScroll() {
    const box = scroller.current
    if (!box) return
    // a couple of pixels of slack, because a fractional scroll height is
    // ordinary and "nearly at the bottom" is what the viewer meant
    setFollowing(box.scrollHeight - box.scrollTop - box.clientHeight < 4)
  }

  function toBottom() {
    const box = scroller.current
    if (!box) return
    box.scrollTop = box.scrollHeight
    setFollowing(true)
    onSeen()
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scroller}
        onScroll={onScroll}
        role="log"
        aria-label={label}
        tabIndex={0}
        className="min-h-0 flex-1 space-y-2 overflow-y-auto overflow-x-hidden pr-1 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
      >
        {feed.events.length === 0 ? (
          <p className="text-neutral-600">nothing yet - the detectors are watching</p>
        ) : (
          feed.events.map((event) => <Entry key={event.event_id} event={event} />)
        )}
      </div>

      {behind > 0 && !following && (
        <button
          type="button"
          onClick={toBottom}
          className="absolute inset-x-0 bottom-0 mx-auto min-h-9 w-fit rounded-full border border-neutral-700 bg-neutral-900/95 px-3 text-neutral-200 backdrop-blur hover:border-neutral-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
        >
          {behind} new {behind === 1 ? 'event' : 'events'} &darr;
        </button>
      )}
    </div>
  )
}

/**
 * one entry: who it is about, when, what happened, and what says so.
 *
 * the species marker is a dot *and* a name, and the severity is a glyph *and*
 * a spoken word - colour is the third identifier here, never the only one.
 */
function Entry({ event }) {
  const seen = describe(event)
  return (
    <article
      className="animate-[outcome-in_260ms_ease-out] border-l-2 pl-2 motion-reduce:animate-none"
      style={{ borderColor: seen.species === null ? '#404040' : speciesCss(seen.species) }}
    >
      <div className="flex items-baseline gap-1.5">
        <span aria-hidden className={`w-4 shrink-0 ${seen.tone}`}>
          {seen.mark}
        </span>
        <span className="sr-only">{seen.label.split(':')[0]}. </span>
        <span className="tabular-nums text-neutral-600">{event.epoch}</span>
        <span aria-hidden className="truncate text-neutral-600">
          {seen.kind}
        </span>
      </div>
      <p className="text-neutral-200">{event.title}</p>
      <p className="text-neutral-500">{event.evidence}</p>
    </article>
  )
}

/** the one-line summary the mobile default state gets instead of the feed */
export function LatestEvent({ feed, onOpen, behind }) {
  const latest = feed.events.at(-1)
  const seen = latest ? describe(latest) : null
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex min-h-11 w-full items-center gap-2 rounded border border-neutral-800/80 bg-neutral-950/80 px-2 text-left text-xs backdrop-blur hover:border-neutral-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
    >
      {/* a status summary, not the log region: the sequential feed behind it
          is the one thing marked role="log" */}
      <span role="status" className="flex min-w-0 flex-1 items-center gap-2">
        {seen ? (
          <>
            <span
              aria-hidden
              className="h-2 w-2 shrink-0 rounded-full"
              style={{ background: seen.species === null ? '#525252' : speciesCss(seen.species) }}
            />
            <span aria-hidden className={`shrink-0 ${seen.tone}`}>
              {seen.mark}
            </span>
            <span className="truncate text-neutral-300">{latest.title}</span>
          </>
        ) : (
          <span className="text-neutral-600">nothing yet</span>
        )}
      </span>
      {behind > 0 && (
        <span className="shrink-0 rounded-full bg-emerald-950/60 px-1.5 tabular-nums text-emerald-300">
          {behind}
        </span>
      )}
    </button>
  )
}
