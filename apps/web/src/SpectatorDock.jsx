import { useEffect, useRef } from 'react'
import LiveLog, { LatestEvent } from './LiveLog.jsx'
import { unread } from './telemetry/events.js'

/**
 * the mobile chrome: one latest-event ticker, one dock button, and the sheet
 * it opens.
 *
 * the page is a world viewer, so the default state gives the world everything
 * it can: a one-line ticker and a button, and nothing else stacked over the
 * map.
 *
 * whether the sheet is open is App's decision, not this component's - see the
 * overlay state machine there. this file draws what it is told to draw.
 */
export default function SpectatorDock({ feed, sheet, onOpen, onClose, onSeen }) {
  const behind = unread(feed).length
  const trigger = useRef(null)

  return (
    // above the betting line rather than over it: betting stays the highest
    // priority control on the page and never gets covered by telemetry
    <div className="pointer-events-none absolute inset-x-0 bottom-[4.25rem] flex flex-col gap-1 px-2 sm:hidden">
      {sheet && (
        <Sheet
          title="run events"
          onClose={() => {
            onClose()
            trigger.current?.focus()
          }}
        >
          <LiveLog feed={feed} onSeen={onSeen} label="run events" />
        </Sheet>
      )}

      <div className="pointer-events-auto">
        <LatestEvent feed={feed} behind={behind} onOpen={() => onOpen('events')} />
      </div>

      <div className="pointer-events-auto flex gap-1">
        <Tab
          ref={trigger}
          open={sheet === 'events'}
          onClick={() => (sheet === 'events' ? onClose() : onOpen('events'))}
        >
          Live
          {behind > 0 && (
            <span className="ml-1.5 rounded-full bg-emerald-950/60 px-1.5 tabular-nums text-emerald-300">
              {behind}
            </span>
          )}
        </Tab>
      </div>
    </div>
  )
}

function Tab({ ref, open, onClick, children }) {
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      aria-expanded={open}
      className={`min-h-11 flex-1 rounded border px-3 text-xs backdrop-blur focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 ${
        open
          ? 'border-neutral-600 bg-neutral-900/90 text-neutral-100'
          : 'border-neutral-800/80 bg-neutral-950/80 text-neutral-400'
      }`}
    >
      {children}
    </button>
  )
}

/**
 * the one detail slot. capped at 45dvh during a run so the world keeps the
 * majority of the screen, one vertically scrolling column so nothing ever
 * scrolls sideways, and focus stays inside it until it closes.
 */
function Sheet({ title, onClose, children }) {
  const box = useRef(null)

  useEffect(() => {
    const el = box.current
    el?.focus()

    function onKey(e) {
      if (e.key === 'Escape') {
        e.stopPropagation()
        return onClose()
      }
      if (e.key !== 'Tab' || !el) return
      const focusable = [
        ...el.querySelectorAll(
          'a[href],button:not([disabled]),input,select,textarea,summary,[tabindex]:not([tabindex="-1"])',
        ),
      ]
      if (focusable.length === 0) return
      const [first, last] = [focusable[0], focusable[focusable.length - 1]]
      const active = document.activeElement
      if (e.shiftKey && (active === first || active === el)) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && active === last) {
        e.preventDefault()
        first.focus()
      }
    }

    el?.addEventListener('keydown', onKey)
    return () => el?.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <section
      ref={box}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="pointer-events-auto flex max-h-[45dvh] animate-[outcome-in_240ms_ease-out] flex-col overflow-hidden rounded border border-neutral-800/80 bg-neutral-950/95 p-3 text-xs backdrop-blur focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 motion-reduce:animate-none"
    >
      <div className="flex items-baseline gap-2 pb-2">
        <span className="text-neutral-300">{title}</span>
        <button
          type="button"
          onClick={onClose}
          className="ml-auto min-h-11 px-2 text-neutral-500 hover:text-neutral-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
        >
          close
        </button>
      </div>
      {/* one column, and the child owns the scrolling in it - a scroller here
          as well is what put two bars on the feed. nothing scrolls sideways
          either way, because a sideways swipe on a phone is how you lose half
          a card without noticing it is there. */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {children}
      </div>
    </section>
  )
}
