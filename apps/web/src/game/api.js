/**
 * the authenticated half of the server, over ordinary fetch.
 *
 * the session lives in an `HttpOnly` cookie, so nothing here reads or stores
 * a token - `credentials: same-origin` is the whole of it. every failure
 * arrives as an `ApiError` carrying the server's stable code.
 */

export class ApiError extends Error {
  constructor(code, message, status) {
    super(message || 'something went wrong')
    this.code = code || 'server_error'
    this.status = status
  }
}

async function call(path, options = {}) {
  let response
  try {
    response = await fetch(path, {
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      ...options,
    })
  } catch {
    throw new ApiError('offline', 'no connection to the server', 0)
  }
  const body = await response.json().catch(() => null)
  if (!response.ok) {
    throw new ApiError(body?.error?.code, body?.error?.message, response.status)
  }
  return body
}

const post = (path, body) => call(path, { method: 'POST', body: JSON.stringify(body) })

export const api = {
  me: () => call('/api/me'),
  register: (username, password) => post('/api/auth/register', { username, password }),
  login: (username, password) => post('/api/auth/login', { username, password }),
  logout: () => post('/api/auth/logout', {}),
  market: () => call('/api/market/current'),
  bet: (marketId, outcome, stake) =>
    call('/api/market/current/bet', {
      method: 'PUT',
      body: JSON.stringify({ market_id: marketId, outcome, stake }),
    }),
}
