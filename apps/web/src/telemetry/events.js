/**
 * the event feed, as a pure reducer over what the server retains.
 *
 * the server publishes its whole bounded ring rather than a delta, so the
 * browser's only job is to merge by `event_id` and stop. that is what makes a
 * reconnect free: the same ring arrives again, every id is already held, and
 * nothing is duplicated or invented locally. the browser never assigns an id,
 * never orders by arrival, and never keeps a history the server has dropped.
 */

/** matches `EVENT_CAPACITY` in `apps/server/src/telemetry.rs` */
export const FEED_CAPACITY = 64

export const initialFeed = {
  /** oldest first, at most `FEED_CAPACITY` */
  events: [],
  /** the run these belong to, so a stale ring cannot be merged into a new run */
  runId: null,
  /** the highest `event_id` the viewer has actually seen at the bottom */
  seen: -1,
  /** true once a first ring has landed: a bootstrap is history, not news */
  bootstrapped: false,
}

/**
 * fold one `telemetry` message in. a ring from another run replaces rather
 * than merges - the server clears its own slot when a run's config lands, and
 * anything still in flight from the last one is not this run's history.
 */
export function reduceFeed(state, message) {
  if (message?.type !== 'telemetry' || !Array.isArray(message.events)) return state

  const fresh = state.runId !== null && message.run_id !== state.runId
  const held = fresh ? [] : state.events
  const known = new Set(held.map((e) => e.event_id))
  const added = message.events.filter((e) => !known.has(e.event_id))
  if (!fresh && added.length === 0) return state

  const events = [...held, ...added]
    .sort((a, b) => a.event_id - b.event_id)
    .slice(-FEED_CAPACITY)

  return {
    events,
    runId: message.run_id ?? state.runId,
    // a first ring is the backlog a late joiner is owed, so it is marked read
    // rather than announced. only what arrives after it is news.
    seen: state.bootstrapped && !fresh ? state.seen : lastId(events),
    bootstrapped: true,
  }
}

/** a new run wipes the feed, because the last run's events are not this one's */
export function resetFeed(runId) {
  return { ...initialFeed, runId: runId ?? null }
}

/** everything past the viewer's marker, in epoch order */
export const unread = (state) => state.events.filter((e) => e.event_id > state.seen)

/** the viewer reached the bottom, so everything held is now read */
export const markRead = (state) =>
  state.seen === lastId(state.events) ? state : { ...state, seen: lastId(state.events) }

const lastId = (events) => (events.length ? events[events.length - 1].event_id : -1)

/**
 * a severity marker that is not a colour. a screen reader gets the word, a
 * viewer who cannot separate the two species by hue gets the glyph, and the
 * colour is the third identifier rather than the only one.
 */
const SEVERITY = {
  major: { mark: '!!', label: 'major', tone: 'text-amber-400' },
  notable: { mark: '!', label: 'notable', tone: 'text-neutral-200' },
  info: { mark: '·', label: 'routine', tone: 'text-neutral-500' },
}

/** the plain words for what a detector said, keyed by the server's `kind` */
const KIND = {
  first_birth: 'first birth',
  extinction: 'extinction',
  near_extinction: 'near extinction',
  recovery: 'recovery',
  lead_change: 'lead change',
  population_trend: 'population',
  world_trend: 'world',
  strategy_shift: 'behaviour',
  trait_drift: 'evolution',
  result: 'result',
}

/**
 * everything an entry needs that is not already in the event. `species` is an
 * index into the server's species order, which is what every colour on the
 * page is keyed by; `null` is a world-level event and gets no marker.
 */
export function describe(event) {
  const severity = SEVERITY[event.severity] ?? SEVERITY.info
  const kind = KIND[event.kind] ?? event.kind
  return {
    ...severity,
    kind,
    species: event.species_id ?? null,
    // what a screen reader reads instead of "!! Species A is extinct"
    label: `${severity.label} ${kind}, epoch ${event.epoch}: ${event.title}`,
  }
}
