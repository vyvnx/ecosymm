# ecosym

Deterministic ecosystem simulation. Rust workspace + React frontend.

Two reproductively isolated species compete for one shared, finite, regrowing
resource field on a torus of land and impassable sea. Every organism carries an identified, immutable genome; genetic
change happens only at birth, by recombining both parents' genes and mutating
the result.

That genome holds a body *and* a brain. Behaviour is not scripted: each organism
runs a small inherited neural policy (8 -> 8 -> 5, `tanh`) that decides where it
wants to go, whether to chase food, whether to rest and whether to try to breed.
Nothing trains it and nothing scores it - surviving and reproducing is the only
feedback there is, so the weights that stay in the pool are the ones that worked.

The world pushes back. The sea cannot be walked on - a step into it slides along
the shore or does not happen, and costs the effort either way - and a tile's
regrowth is boosted by seed rain from whichever neighbours are still standing,
so the edges of a grazed region recover before the middle does.

## Run

```bash
cargo run --release
cargo run --release -- --seed 1234 --population-per-species 500 --epochs 500
cargo run --release -- --twins --epochs 300   # identical bodies, different founder brains
```

Prints the generated world, both founder populations, a per-species epoch table,
the run result with a winner, and a replay digest. Same seed, same config and
same engine always produce the same digest.

The result reports genetic and behavioural drift side by side:

```text
Species A: initial 500, final 2514, change +2014 (+402.8%), births 97383, deaths 95369
  genes    speed 1.300 -> 1.087   metabolism 1.200 -> 0.240   heat_pref 0.620 -> 0.438
  behavior movement/tick 0.565 -> 1.011   food seeking 0.516 -> 0.985   rest tendency 0.492 -> 0.043
  brain drift 0.3798 per neural gene from the founder policy, mean energy 6.32
```

Those behavioural numbers are descriptive. Nothing reads them back, and there is
no AI fitness function anywhere in the model - see `experiments/` for what that
does and does not produce.

An **epoch** is a batch of simulation ticks - simulation time, not a biological
generation. Parents and descendants coexist, so genealogical depth has to be
derived from genome ancestry, never from the clock.

The winner is the species with the greatest final/initial population ratio.
**It is descriptive, not a health verdict**: a species that fell from 500 to 2
beats one that fell to 1, and neither thrived.

## Web UI

```bash
npm run server   # rust ws server on :3001, sqlite in ./ecosym.db
npm run dev      # vite on :5173, proxies /api and /ws to the server
```

The page *is* the world: a fullscreen canvas showing terrain and climate, the
shared resource field depleting and regrowing, and every sampled organism at its
simulated position, coloured by species. One small readout floats bottom left -
epoch, populations, frame rate, digest.

Two things are interactive, and nothing else on the page is: a `username |
1,000 DC` badge top right, and one betting panel bottom centre.

One copy of the world is drawn, centred, at whatever size the window allows.
The map is a torus, so it *could* tile to fill a wide screen; that was tried and
removed, because two identical landmasses side by side read as two worlds.

Movement, births and deaths are interpolated between samples at
`requestAnimationFrame` cadence, so the picture stays smooth however fast the
simulation and the socket happen to be running. That interpolated path is a
visual estimate across an epoch, not a record of the 20 ticks inside it.

The browser never starts anything - it cannot. There is no seed, population or
epoch count it can ask for. One coordinator task on the server owns the run and
market lifecycle and keeps going with nobody connected; a socket is a
subscriber to it. Opening a second tab, a second browser or a second device
joins the run already in progress rather than starting another world.

The server samples state at most 15 times a wall-clock second and always sends
the first and last. Aggregate epoch reports and the replay digest are unchanged
and still arrive as JSON; the sampled render state is a separate binary frame
(`ECSY`, version 1, little-endian, described in `apps/server/src/wire.rs`).
Golden hex vectors are duplicated in `apps/server/src/wire.rs` and
`apps/web/src/render/protocol.test.js` so a change to the format fails on both
sides at once.

Rendering is one-way by construction: extraction takes `&SimulationState`, is
not part of `EpochEngine`, and cannot reach the digest -
`render_extraction_does_not_reach_the_digest` in `crates/replay` is the proof.
WebGL2 is required; there is no Canvas 2D fallback.

```bash
npm test -w @ecosym/web   # protocol, reconciliation, coins and market state
```

## The game

Species compete; you bet **Darwin Coins** on which way it goes. Darwin Coin is
**play money only** - it cannot be bought, sold, transferred, redeemed or
cashed out, there are no payments and no prizes, and it represents no monetary
value of any kind. Registration grants 1,000 DC, and an account that reaches
zero with nothing at stake gets 100 DC back once a day.

Each cycle is one market and one run:

```text
open the market for 30s  ->  lock  ->  run 500 epochs over ~60s
  ->  settle  ->  hold the result for 8s  ->  open the next market
```

Three outcomes, exhaustive except for total extinction, which voids the market
and refunds every stake:

```text
[ Species A ]   [ Coexistence ]   [ Species B ]
```

Coexistence means both species survived *and* their final/initial population
ratios are within a symmetric margin: `abs(ln(score_a / score_b)) <= 0.20`.
That margin was calibrated over 1,000 recorded seeds rather than guessed - see
`experiments/2026-08-28-bet-outcome-calibration`.

Betting is pari-mutuel. Whole coins are escrowed while the market is open,
5% of the pool is burned at settlement, and the winners divide the rest in
proportion to their stakes. Nothing is paid from outside the pool, so a
projected return moves with every later bet and **can be below the stake when
nearly everyone was right**. The panel says so before you confirm.

The seed is the reason the market locks before the run starts. Before the
first bet the server publishes `sha256(tag || run_id || config || seed ||
nonce)`; seed and nonce are revealed only once the lock transaction has
committed. So the server cannot pick a seed after seeing the pool, and a
bettor cannot run the world ahead of the market. Both halves are checkable
after the fact from the reveal and the retained digest.

The game layer never reaches into the sim. A finished `RunOutcome` crosses into
`ecosym-game` as two species ids and four head counts, and nothing crosses
back - `publishing_and_pacing_cannot_reach_the_digest` runs the same seed with
and without viewers, and with and without pacing, and compares digests.

### Running it

Accounts, coins, markets, bets and an append-only ledger live in one SQLite
file, `./ecosym.db` by default and `$ECOSYM_DB` if set. Migrations run at
startup. Exactly one server may own a database: the second one to try takes an
exclusive lock on `ecosym.lock`, fails, and says so, because two coordinators
would run two worlds into one set of markets.

A restart cannot resume a simulation it never checkpointed, so any market that
was still open or running is voided and refunded exactly once before a new run
starts.

In production, set `ECOSYM_SECURE_COOKIES=1` and terminate TLS in front of the
server - the session cookie is `HttpOnly; SameSite=Lax` either way, but without
`Secure` it will travel over plain http. Back up the SQLite file: it is the
only record of every account and every coin.

## Layout

| path | what |
| --- | --- |
| `apps/cli` | the `ecosym` binary (workspace `default-members`, so bare `cargo run` hits it) |
| `apps/server` | axum HTTP + WebSocket; a blocking producer streams epoch reports as JSON and sampled render state as binary |
| `apps/web` | React + Tailwind dashboard around a WebGL2 world view (`src/render/`) |
| `crates/core` | rng, `SimConfig`, hashing, named seed derivation |
| `crates/genetics` | `GenomeId`, `Genes`, `NeuralGenome`, immutable `Genome`, mutation, recombination |
| `crates/world` | terrain, passability, climate, and the shared resource field with local dispersal |
| `crates/ecology` | `Organism`, `Species`, `Population`, phenotype, `behavior/` (observations, neural policy, actions), interactions |
| `crates/simulation` | `SimulationState`, the `EpochEngine` contract, `CpuEngine`, the runner |
| `crates/replay` | run checksums |
| `crates/game` | darwin coins, market rules, three-outcome scoring, pari-mutuel settlement |
| `crates/gpu` | empty until the CPU loop is measurably the bottleneck |

Dependencies run one way, left depending on right:

```text
genetics   -> core
world      -> core
ecology    -> core + genetics + world
simulation -> core + genetics + ecology + world
replay     -> simulation
game       -> serde only
gpu        -> simulation (future)
cli        -> simulation + replay
server     -> simulation + replay + game
```

`ecosym-simulation` must never depend on `ecosym-gpu`, `wgpu` or CUDA. The
backend seam is `EpochEngine`, defined in `crates/simulation`.

`ecosym-game` depends on nothing but `serde` - not the simulation, not a
database, not a clock, not a random number generator. That is what makes it
structurally impossible for a balance or a wager to reach the ecology, and
`the_game_crate_cannot_reach_the_simulation` fails if it ever gains one.

## Test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Determinism is covered by `crates/simulation` (same seed replays identically,
down to every weight in every surviving brain, and the same visit order for a
fixed seed) and `crates/replay` (the digest is sensitive to species order, to
per-species values, and to the evolving neural weights and behaviour).

Any `EpochEngine` implementation must pass
`ecosym_simulation::conformance::verify_engine`, which checks accounting,
reproductive isolation, genome immutability, id stability, world bounds, neural
weight bounds, that every organism carried through an epoch ran its policy on
every tick, and determinism for 0, 1, 2 and 3 species on the same execution
path.

`benchmarks/` holds the recorded CPU baseline. `experiments/` holds runs worth
re-checking, each with the command and digest that produced it.
