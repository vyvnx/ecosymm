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
npm run server   # rust ws server on :3001
npm run dev      # vite on :5173, proxies /api and /ws to the server
```

## Layout

| path | what |
| --- | --- |
| `apps/cli` | the `ecosym` binary (workspace `default-members`, so bare `cargo run` hits it) |
| `apps/server` | axum HTTP + WebSocket, streams one message per epoch |
| `apps/web` | React + Tailwind, live per-species chart off the WebSocket |
| `crates/core` | rng, `SimConfig`, hashing, named seed derivation |
| `crates/genetics` | `GenomeId`, `Genes`, `NeuralGenome`, immutable `Genome`, mutation, recombination |
| `crates/world` | terrain, passability, climate, and the shared resource field with local dispersal |
| `crates/ecology` | `Organism`, `Species`, `Population`, phenotype, `behavior/` (observations, neural policy, actions), interactions |
| `crates/simulation` | `SimulationState`, the `EpochEngine` contract, `CpuEngine`, the runner |
| `crates/replay` | run checksums |
| `crates/gpu` | empty until the CPU loop is measurably the bottleneck |

Dependencies run one way, left depending on right:

```text
genetics   -> core
world      -> core
ecology    -> core + genetics + world
simulation -> core + genetics + ecology + world
replay     -> simulation
gpu        -> simulation (future)
cli/server -> simulation + replay
```

`ecosym-simulation` must never depend on `ecosym-gpu`, `wgpu` or CUDA. The
backend seam is `EpochEngine`, defined in `crates/simulation`.

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
