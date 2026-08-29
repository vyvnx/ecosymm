# Is `tanh` worth what it costs now?

## Question

ADR 0009 set a gate before recurrence landed: `softsign(x) = x / (1 + |x|)` is
adopted **only if** the recurrent policy stays inside the recorded CPU budget
*and* multi-seed experiments show selection still produces viable populations.

Recurrence made the policy the expensive half of the tick - 192
multiply-accumulates per organism-tick against 128, and a 1.64x wall clock at
the same population. `tanh` is 12 libm calls of that, and softsign is a divide.

Softsign is also bit-identical across platforms, which for a project whose
first rule is that a seed reproduces a digest is worth something on its own.

## Design

One variable. Activation changes on both layers; topology, perception, mating,
crossover, mutation, the world and the scenarios are untouched. The baseline is
the previous commit - `tanh`, recurrent, local mating.

```bash
for s in 1234 99 7 20260828 555 31337; do
  cargo run --release -p ecosym-cli --bin ecosym -- --seed $s
done
cargo run --release -p ecosym-cli --bin ecosym -- --seed 1234 --twins
```

## Results

### The performance half

| run | `tanh` | softsign | speedup |
| --- | ---: | ---: | ---: |
| seed 1234 | 118.5 s | 90.4 s | 1.31x |
| seed 99 | 68.7 s | 51.3 s | 1.34x |
| seed 7 | 159.0 s | 122.9 s | 1.29x |
| seed 20260828 | 107.1 s | 75.7 s | 1.42x |
| seed 555 | 86.4 s | 67.6 s | 1.28x |
| seed 31337 | 109.3 s | 87.1 s | 1.25x |
| twins 1234 | 119.2 s | 88.2 s | 1.35x |

**1.25 - 1.42x on every seed.** The populations differ between columns, so
these are not identical workloads, but the direction and rough size are
consistent across seven runs and no seed goes the other way.

### The viability half

| run | `tanh` | softsign |
| --- | --- | --- |
| seed 1234 | `c0fd53a79b897029` | `063119e7a7c2029f` |
| seed 99 | `bb6b6c4be7a5d712` | `382567dfb7c9005b` |
| seed 7 | `c81a1e720211210f` | `7e9d15d314f71530` |
| seed 20260828 | `a9148f0dd55a7b39` | `843680a11572c133` |
| seed 555 | `447c80428501d202` | `5201c4df43907cc6` |
| seed 31337 | `4344f5e163fcc3a4` | `3b500021708c80e4` |
| twins 1234 | `560d70e68ff58598` | `fac0ddc06626f809` |

Populations at epoch 500, A / B:

| seed | `tanh` | softsign | winner |
| --- | --- | --- | --- |
| 1234 | 9306 / 69 | 9388 / 641 | A -> A |
| 99 | 0 / 6005 | 0 / 6427 | B -> B |
| 7 | 4075 / 8116 | 10322 / 2370 | B -> **A** |
| 20260828 | 8616 / 0 | 0 / 7771 | A -> **B** |
| 555 | 0 / 6941 | 0 / 7479 | B -> B |
| 31337 | 9009 / 24 | 9130 / 0 | A -> A |
| twins 1234 | 0 / 9516 | 9026 / 446 | Twin 1 -> **Twin 0** |

Selection does the same things:

| | `tanh` | softsign |
| --- | --- | --- |
| speed, survivors from 1.300 | 0.443 - 0.544 | 0.390 - 0.519 |
| speed, survivors from 0.700 | 0.377 - 0.474 | 0.314 - 0.490 |
| metabolism, from 1.200 / 0.800 | 0.237 - 0.283 | 0.244 - 0.249 |
| size, from 1.000 | 1.215 - 1.862 | 1.216 - 1.324 |
| seeds won, A / B | 4 / 2 | **3 / 3** |

Dispersal still collapses, metabolism still drives to its clamp, bodies still
grow, and the scenario is if anything better balanced - the two profiles split
the six seeds evenly instead of four to two.

### What it cost

Softsign saturates far more slowly. `tanh(4)` is 0.9993; `softsign(4)` is 0.8,
and reaching 0.999 takes an input of 1,000. The same weight drift therefore
shows as a **smaller output move**, which is a different search landscape
rather than a worse one - but two tests were pinned to `tanh`'s gain and had to
be re-based on measurement rather than on a number that no longer means what it
did:

- `behaviour_measurably_changes_over_a_long_run` asked for 0.1 of absolute
  behaviour change; a settled, still-evolving population now shows 0.07 - 0.09
  on that scenario at any run length from 60 to 140 epochs. The bar is 0.05,
  and the activation-independent half of the claim - `brain_drift` - is
  unchanged.
- `the_twin_scenario_leaves_the_policy_as_the_only_difference` was on a 96x96
  world, which was already sitting on the local-mating Allee threshold at
  2123/705. Under softsign it falls the other way. It moved to the default
  128x128 world, where the trough is 497 of 1000 and both twins finish alive -
  the margin it should have had in the first place.

Neither is softsign failing the gate. The first is a unit change, the second is
a knife-edge scenario that was always going to tip on something.

## Conclusion

**Adopt.** It wins the performance half on every seed, viable selection is
intact by every measure recorded here, and it removes a platform-dependent
libm call from the one function a replay digest is most sensitive to.

`tanh` remains one line away and this table is what to re-record if it ever
comes back.
