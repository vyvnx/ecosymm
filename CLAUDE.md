# ecosym

## What we are building

A **survival spectator game**. Species compete in a persistent ecosystem;
players bet **fake money** (play-money only, never real currency, no payments,
no cash-out) on which species will thrive. The simulation runs **forever on a
server** — players tune in to watch evolution, competition and collapse happen
on their own.

The sim is the game. Nobody controls an organism. The only player verbs are
*watch* and *bet*.

## Why it exists

This is a learning vehicle, and that is a first-class goal. Each area gets built
for real, not stubbed:

- **Rust** — workspace design, ownership, traits as seams, no-`unsafe` by default.
- **Algorithms** — spatial queries, scheduling, deterministic RNG, data layout.
- **Neural networks** — evolved policies, no trainer, no loss function.
- **GPU programming** — the `EpochEngine` backend on `wgpu`/compute.
- **GPU rendering** — drawing a large live world at frame rate.

When there is a choice between a shortcut and the version that teaches the
thing, take the one that teaches — *but* keep it minimal. Learning is the reason
to write the code, not a licence to over-build it.

## Design rules

**Game changes follow game theory. Simulation changes follow ecology.**
Anything touching the *game* — betting, odds, payouts, scoring, seasons — is
judged as a mechanism: what does a rational bettor do with it, is it exploitable,
does it stay interesting when players learn it, does it dodge dominant strategies
and degenerate equilibria. Anything touching the *world* — organisms, genomes,
resources, terrain — must stay ecologically coherent: energy is conserved and
paid for, carrying capacity binds, selection is the only optimiser.

**The game layer never reaches into the sim.** Betting must not bias outcomes.
Odds read the simulation; the simulation never reads the odds. If a game feature
needs the sim to change, that change has to be defensible on ecological grounds
alone.

**Fun to watch beats fair on paper.** Aim for legible drama: visible boom/bust,
comebacks, niches, arms races, real extinction risk. A world that flatlines to a
stable equilibrium is a bug even when the model is correct. Prefer emergence over
scripted events — no hand-placed "and then a meteor" unless it is a rule of the
world that anyone could have predicted.

**Determinism is non-negotiable.** Same seed + config + engine = same digest.
It is what makes a bet auditable and a bug reproducible. Never trade it for
speed. See `docs/adr/0002`.

**Forever-running means it has to survive itself.** No unbounded growth in
memory or history, no drift that only shows up on day 30. Long-run behaviour is
a correctness property, not an ops concern.

## Working agreements

- Read `README.md` first — layout, crate dependency direction, run and test
  commands live there and are not repeated here.
- Every non-obvious decision gets an ADR in `docs/adr/`. `docs/` is gitignored,
  so ADRs are local reference only — anything the repo must carry belongs in
  `README.md` or here. Runs worth re-checking get a folder in `experiments/`
  with the exact command and digest.
- `ecosym-simulation` must never depend on `ecosym-gpu`, `wgpu` or CUDA. The
  seam is `EpochEngine` (`docs/adr/0003`).
- Any new engine passes `ecosym_simulation::conformance::verify_engine`.
- Before pushing: `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`.
- Optimise only against `benchmarks/`. The GPU engine lands when the CPU loop is
  a measured bottleneck, not before.

## Commits

`type(scope): subject` — lowercase, imperative, no trailing period, subject
under ~72 chars.

**Types:** `feat` (new behaviour) · `fix` (bug) · `perf` (measured speedup,
cite the benchmark) · `refactor` (no behaviour change) · `test` · `docs` ·
`chore` (deps, tooling, config) · `bench` (benchmark changes) · `exp`
(experiment runs).

**Scopes** are the crate or app directory name, so the scope points at where the
change lives:

| scope | path |
| --- | --- |
| `core` `genetics` `world` `ecology` `simulation` `replay` `game` `gpu` | `crates/*` |
| `cli` `server` `web` | `apps/*` |
| `bench` | `benchmarks/` |
| `exp` | `experiments/` |
| `workspace` | root `Cargo.toml`, CI, `.gitignore`, rustfmt |

Multiple scopes: `feat(ecology,simulation): ...`. No sensible scope: omit it.

```text
feat(ecology): let organisms read neighbour density
fix(world): stop shore slides from refunding movement cost
perf(simulation): halve tick allocations, 1.8x on the 500-pop bench
chore(workspace): bump wgpu to 0.20
```

Anything that changes a run digest says so in the body — old and new digest, and
why the change is ecologically justified.
