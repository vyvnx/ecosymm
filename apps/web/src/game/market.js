/**
 * the market half of what arrives on the socket, as a pure reducer.
 *
 * the browser is a subscriber to one server-owned run. it never decides what
 * is current: it accepts what the coordinator says, rejects anything from an
 * older market, ignores a revision it already has, and asks for a fresh
 * bootstrap whenever the stream stops making sense - a revision that moved
 * backwards, a `sync_end` that does not match its `sync_begin`, or an epoch
 * belonging to a run it has never been told about.
 */

export const initialMarket = {
  market: null,
  /** the run id of a bootstrap in progress, or null */
  syncing: null,
  synced: false,
  /** bumped whenever the browser needs the server to start again */
  resync: 0,
  /** server clock minus this device's clock, in seconds */
  offset: 0,
}

const serverTime = (message) => message.server_time ?? message.market?.server_time ?? null

export function reduceMarket(state, message, now = Date.now() / 1000) {
  const at = serverTime(message)
  const offset = at === null ? state.offset : at - now

  switch (message.type) {
    case 'sync_begin':
      return { ...state, syncing: message.run_id, synced: false, offset }

    case 'sync_end':
      // a bootstrap that did not close the one it opened tells us nothing
      // about what we now hold, so start again rather than trust it
      if (state.syncing !== message.run_id) {
        return { ...state, syncing: null, resync: state.resync + 1, offset }
      }
      return { ...state, syncing: null, synced: true, offset }

    // a point sample rather than a stream: an http response that arrives
    // behind the socket is ordinary, and never a reason to resynchronise
    case 'market_fetched': {
      const next = message.market
      if (!next) return state
      const current = state.market
      const stale =
        current &&
        (next.market_id < current.market_id ||
          (next.market_id === current.market_id && next.revision <= current.revision))
      return stale ? { ...state, offset } : { ...state, market: next, offset }
    }

    case 'market_open':
    case 'market_pool':
    case 'market_locked':
    case 'market_settled': {
      const next = message.market
      if (!next) return state
      const current = state.market
      if (current && next.market_id < current.market_id) return state
      if (current && next.market_id === current.market_id) {
        if (next.revision === current.revision) return { ...state, offset }
        if (next.revision < current.revision) {
          return { ...state, resync: state.resync + 1, offset }
        }
      }
      return { ...state, market: next, offset }
    }

    default:
      return state
  }
}

/** an epoch or a run message that belongs to a run this client is not watching */
export function fromAnotherRun(message, runId) {
  return (
    runId !== null &&
    message.run_id !== undefined &&
    message.run_id !== null &&
    message.run_id !== runId
  )
}

/** whether betting controls may do anything at all right now */
export function canBet({ market, account, synced, connected, submitting }, now) {
  if (!market || !account || !synced || !connected || submitting) return false
  if (market.phase !== 'open') return false
  return market.locks_at > now
}
