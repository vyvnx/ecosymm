import { useState } from 'react'
import { api, ApiError } from './game/api.js'
import { formatCoins } from './game/coins.js'

/**
 * the account badge, top right, and the small panel behind it.
 *
 * it is one of only two things on the page that take pointer events. there is
 * no dashboard behind it: sign in, sign out, and what you are holding.
 *
 * whether it is open is App's call, not this component's - one state machine
 * owns every panel, so this one cannot open itself over a sheet or a result.
 */
export default function AccountPanel({ account, open, onToggle, onChanged }) {
  const [registering, setRegistering] = useState(false)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState(null)
  const [pending, setPending] = useState(false)

  async function submit(event) {
    event.preventDefault()
    setPending(true)
    setError(null)
    try {
      const next = await (registering
        ? api.register(username, password)
        : api.login(username, password))
      setPassword('')
      onChanged(next)
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'something went wrong')
    } finally {
      setPending(false)
    }
  }

  async function signOut() {
    setPending(true)
    try {
      await api.logout()
      onChanged(null)
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="pointer-events-auto absolute top-4 right-4 flex max-w-[calc(100vw-2rem)] flex-col items-end text-xs">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="min-h-11 rounded border border-neutral-800/80 sm:min-h-9 bg-neutral-950/70 px-3 py-1.5 tabular-nums text-neutral-300 backdrop-blur hover:border-neutral-700 hover:text-neutral-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
      >
        {account ? (
          <>
            <span className="text-neutral-400">{account.username}</span>
            <span className="mx-2 text-neutral-700">|</span>
            <span className="text-emerald-400">{formatCoins(account.balance)}</span>
          </>
        ) : (
          'sign in'
        )}
      </button>

      {open && (
        <div className="mt-2 w-64 rounded border border-neutral-800/80 bg-neutral-950/90 p-3 backdrop-blur">
          {account ? (
            <div className="space-y-2">
              <Row label="available" value={formatCoins(account.balance)} />
              <Row label="at stake" value={formatCoins(account.escrow)} />
              {account.recovery_available && (
                <p className="text-neutral-500">
                  a 100 DC recovery grant lands when the next market opens
                </p>
              )}
              <button
                type="button"
                onClick={signOut}
                disabled={pending}
                className="min-h-9 w-full rounded border border-neutral-800 px-2 py-1.5 text-neutral-400 hover:border-neutral-700 hover:text-neutral-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-50"
              >
                sign out
              </button>
            </div>
          ) : (
            <form onSubmit={submit} className="space-y-2">
              <Field
                id="account-username"
                label="username"
                value={username}
                onChange={setUsername}
                autoComplete="username"
              />
              <Field
                id="account-password"
                label="password"
                type="password"
                value={password}
                onChange={setPassword}
                autoComplete={registering ? 'new-password' : 'current-password'}
              />
              {error && <p className="text-amber-400">{error}</p>}
              <button
                type="submit"
                disabled={pending}
                className="min-h-9 w-full rounded border border-emerald-800/60 bg-emerald-950/40 px-2 py-1.5 text-emerald-300 hover:border-emerald-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500 disabled:opacity-50"
              >
                {pending ? '...' : registering ? 'create account' : 'sign in'}
              </button>
              <button
                type="button"
                onClick={() => {
                  setRegistering((r) => !r)
                  setError(null)
                }}
                className="w-full text-neutral-500 hover:text-neutral-300 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
              >
                {registering ? 'i already have an account' : 'create an account'}
              </button>
              {registering && (
                <p className="text-neutral-600">
                  1,000 Darwin Coins to start. play money - it cannot be bought, sold or cashed
                  out.
                </p>
              )}
            </form>
          )}
        </div>
      )}
    </div>
  )
}

function Field({ id, label, value, onChange, type = 'text', autoComplete }) {
  return (
    <label htmlFor={id} className="block">
      <span className="text-neutral-500">{label}</span>
      <input
        id={id}
        type={type}
        value={value}
        autoComplete={autoComplete}
        onChange={(e) => onChange(e.target.value)}
        className="mt-0.5 min-h-9 w-full rounded border border-neutral-800 bg-neutral-900/60 px-2 py-1 text-neutral-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
      />
    </label>
  )
}

function Row({ label, value }) {
  return (
    <div className="flex items-baseline gap-2 tabular-nums">
      <span className="text-neutral-600">{label}</span>
      <span className="ml-auto text-neutral-200">{value}</span>
    </div>
  )
}
