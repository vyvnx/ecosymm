# What does an organism do with a memory?

## Question

The feed-forward policy is a pure function: the same observations produce the
same intent on tick 1 and on tick 500. Nothing can be *heading somewhere*
across ticks, nothing can persist through a bad patch, and nothing can respond
to a change in conditions rather than the conditions themselves.

ADR 0009 replaces the hidden layer with an Elman recurrent one - each hidden
neuron also sees all eight hidden activations from the previous tick. The
weights are inherited; the eight activations are the organism's own, start at
zero at birth, and are never passed on.

The ADR is explicit that a failed experiment is an acceptable outcome: if
multi-seed runs show no temporal behaviour change beyond noise, recurrence
costs 50% more multiply-accumulates for nothing and should be reverted.

So: does anything use it, and does anything pay for it?

## Design

One variable. Topology goes `12 -> 8 -> 4` to `12 -> 8 recurrent -> 4`;
perception, crossover, mutation, the world and the scenario are untouched, and
the activation stays `tanh`. The baseline is the previous commit, the
feed-forward `tanh` policy at the same productivity.

Genome grows from 140 numbers to 204: the 64 recurrent weights are inherited,
recombined and mutated like any other. There is no lifetime weight update
anywhere - this is memory, not learning.

```bash
for s in 1234 99 7 20260828 555 31337; do
  cargo run --release -p ecosym-cli --bin ecosym -- --seed $s
done
cargo run --release -p ecosym-cli --bin ecosym -- --seed 1234 --twins
```

## Results

Populations at epoch 500, A / B, and the winner:

| seed | feed-forward | recurrent | winner |
| --- | --- | --- | --- |
| 1234 | 8807 / **0** | 8467 / 171 | A -> A |
| 99 | **0** / 6417 | 6028 / 170 | B -> **A** |
| 7 | 5336 / 6726 | 10207 / 1376 | B -> **A** |
| 20260828 | 7410 / **0** | 7156 / 76 | A -> A |
| 555 | 7142 / 174 | **0** / 6905 | A -> **B** |
| 31337 | 7866 / **0** | 6918 / 775 | A -> A |
| twins 1234 | 1039 / 7368 | 5268 / 3174 | Twin 1 -> **Twin 0** |

| run | feed-forward digest | recurrent digest |
| --- | --- | --- |
| seed 1234 | `cef4dae3963e179e` | `4aa51da60fc894eb` |
| seed 99 | `c44e0aa451e08ff1` | `a816e56375a9f6db` |
| seed 7 | `cb00aadf19253841` | `1a26815391b1b972` |
| seed 20260828 | `bb90fa361f0518ee` | `f63668fb4813d8d5` |
| seed 555 | `6b641a5263767227` | `f765469b5d258a26` |
| seed 31337 | `9d1361edd00f18d4` | `5a9d815fd072962d` |
| twins 1234 | `bf44f42a35fffa7f` | `fc1961d58079b1ab` |

**Both species survive in five of six seeds, against two of six.** That was not
the hypothesis and it is the largest single change in the table.

### It did not learn to track. It learned to keep going.

Founder draw -> survivors, over the six seeds:

| measure | feed-forward | recurrent |
| --- | --- | --- |
| resource tracking | 0.11 - 0.19 -> **0.02 - 0.49** | 0.09 - 0.17 -> **0.02 - 0.16** |
| movement / tick, founders | 0.24 - 0.50 | **0.26 - 0.53** |
| movement / tick, winners | 0.30 - 0.41 | **0.42 - 0.56** |
| rest tendency, winners | 0.18 - 0.31 | 0.26 - 0.36 |

Resource tracking stops being selected: no survivor exceeds 0.163, where the
feed-forward winners reached 0.451 and 0.488. Movement goes the other way -
winners now travel further per tick than their own founders did, where before
they travelled less.

That is one strategy replacing another, not a strategy failing to appear. A
memoryless policy standing on a grazed tile re-derives its heading every tick
from inputs that barely changed, so its path is diffusive and the only way to
find food is to read the gradient. A policy with eight `tanh` values carried
forward can hold a heading: its path is ballistic, it covers new ground
linearly instead of as the square root of time, and it does not need to know
where the food is to keep finding some.

Ballistic search is cheaper than gradient search here, so selection took it.
The recurrent state is being used as *persistence*, which is exactly the class
of strategy ADR 0009 said was unreachable before, and the world picked it over
the one the previous phase built.

### Why coexistence came back

The feed-forward winner monopolised because tracking compounds: the species
that finds the gradient first reaches the good tiles first and keeps them. A
ballistic forager does not concentrate on the good tiles, it sweeps. Two
sweeping species interleave where two tracking species exclude, and the loser
survives as a crowded remnant instead of dying - competitor exposure for the
losing species is 0.44 - 0.61 in every seed, against 0.00 - 0.50 before.

That is a better world for the game as well as a more interesting one, but it
is a side effect of a behaviour change and not a thing that was tuned for. The
market's coexistence margin still needs its own measurement
(`2026-08-29-coexistence-collapses`), against whatever ecology finally ships.

### Cost

| | feed-forward | recurrent |
| --- | ---: | ---: |
| multiply-accumulates per organism-tick | 128 | **192** |
| `tanh` calls | 12 | 12 |
| genome | 140 numbers | **204** |
| organism state | - | **+8 f32** |
| seed 1234 wall clock | 67.7 s | **111.1 s** |

1.64x wall clock for 1.5x the arithmetic, at essentially the same population
(8,638 against 8,807). The extra beyond the MAC count is the recurrent block
being a second strided read per neuron.

## Conclusion

**Keep it.** The recurrent layer is not decoration: it changed which strategy
selection finds, from gradient following to persistent sweeping, and it changed
the shape of the outcome distribution with it. That is a behaviour class the
feed-forward policy could not reach at any weight.

It is also the more expensive half of the tick now, which is what the softsign
gate in ADR 0009 exists for. Measured on seed 1234, softsign runs the same
scenario in **77.3 s against 111.1 s**, a 1.44x speedup at the same
population, and both species still survive. That is its own experiment with its
own multi-seed viability check, and it is not adopted here: one variable per
experiment, and this one was topology.
