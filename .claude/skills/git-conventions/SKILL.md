---
name: git-conventions
description: Ecosym's commit message format - type(scope) subject, the allowed types, the scope-to-path table, and the run-digest rule. Use whenever there is git work involved - writing a commit, staging changes, amending, squashing, drafting a PR title or body, or reviewing someone else's commit messages.
---

# Commits

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
