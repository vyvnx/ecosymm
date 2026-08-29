-- accounts, sessions, and the run/market/bet/ledger chain behind darwin coin.
--
-- every amount is whole darwin coins in a signed 64-bit integer, and
-- `accounts.balance` is *available* balance: an escrowed stake has already
-- left it. the ledger is append-only and its per-account sum is that balance,
-- which is what `the_ledger_always_explains_the_cached_balance` checks.

CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    -- the spelling the player chose, shown back to them
    username      TEXT    NOT NULL,
    -- ascii-lowercased, and the only thing uniqueness is decided on
    username_key  TEXT    NOT NULL UNIQUE,
    password_hash TEXT    NOT NULL,
    balance       INTEGER NOT NULL CHECK (balance >= 0),
    -- bumped by every transaction that touches balance, escrow or a bet, so a
    -- device cannot overwrite newer account state with an older fetch
    revision      INTEGER NOT NULL DEFAULT 1,
    last_recovery_at INTEGER,
    created_at    INTEGER NOT NULL
);

-- only the sha-256 of the opaque token is ever stored; the raw token exists in
-- the player's cookie and nowhere else
CREATE TABLE sessions (
    token_hash TEXT    PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX sessions_by_account ON sessions(account_id);
CREATE INDEX sessions_by_expiry ON sessions(expires_at);

-- seed and nonce are written when the market opens but only revealed after it
-- locks. the commitment is published first, so the server cannot pick a seed
-- once it has seen the pool.
CREATE TABLE runs (
    id           INTEGER PRIMARY KEY,
    status       TEXT    NOT NULL CHECK (status IN ('pending', 'running', 'complete', 'void')),
    config       TEXT    NOT NULL,
    seed         INTEGER NOT NULL,
    nonce_hex    TEXT    NOT NULL,
    commitment   TEXT    NOT NULL,
    engine       TEXT    NOT NULL,
    digest       TEXT,
    outcome      TEXT,
    created_at   INTEGER NOT NULL,
    completed_at INTEGER
);

-- one market per run. `revision` increments inside every transaction that
-- changes anything public about it, including a bet moving the pools.
CREATE TABLE markets (
    id                 INTEGER PRIMARY KEY,
    run_id             INTEGER NOT NULL UNIQUE REFERENCES runs(id),
    revision           INTEGER NOT NULL DEFAULT 1,
    status             TEXT    NOT NULL CHECK (status IN ('open', 'locked', 'settled', 'void')),
    rule_version       INTEGER NOT NULL,
    fee_bps            INTEGER NOT NULL,
    coexistence_margin REAL    NOT NULL,
    min_stake          INTEGER NOT NULL CHECK (min_stake >= 0),
    max_stake          INTEGER NOT NULL CHECK (max_stake >= min_stake),
    opened_at          INTEGER NOT NULL,
    locks_at           INTEGER NOT NULL,
    locked_at          INTEGER,
    winning_outcome    TEXT,
    gross_pool         INTEGER,
    burn               INTEGER,
    settled_at         INTEGER
);
CREATE INDEX markets_by_status ON markets(status);

CREATE TABLE bets (
    id         INTEGER PRIMARY KEY,
    market_id  INTEGER NOT NULL REFERENCES markets(id),
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    outcome    TEXT    NOT NULL CHECK (outcome IN ('species_a', 'coexistence', 'species_b')),
    stake      INTEGER NOT NULL CHECK (stake > 0),
    payout     INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- one bet per account per market; replacing one updates this row
    UNIQUE (market_id, account_id)
);

-- append-only. `amount` is the signed change to available balance, so
-- escrowing is negative and a payout is positive.
CREATE TABLE ledger_entries (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts(id),
    market_id       INTEGER REFERENCES markets(id),
    bet_id          INTEGER REFERENCES bets(id),
    kind            TEXT    NOT NULL CHECK (kind IN (
                        'initial_grant', 'recovery_grant', 'escrow',
                        'escrow_release', 'payout', 'refund')),
    amount          INTEGER NOT NULL,
    -- set on entries a retry could duplicate (settlement, refund). escrow
    -- moves are guarded by their own transaction and leave it null, and
    -- sqlite lets a unique index hold any number of nulls.
    idempotency_key TEXT UNIQUE,
    created_at      INTEGER NOT NULL
);
CREATE INDEX ledger_by_account ON ledger_entries(account_id);
