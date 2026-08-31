# What happens when you have to be somewhere to breed?

## Question

`Population::select_mate` drew uniformly across the entire species. Two
organisms on opposite sides of the torus bred as readily as two standing
together, so geography had no reproductive consequence at all: mate search,
local density and range fragmentation were absent from the model by
construction.

ADR 0008 makes mating spatial. The mate comes from a wrapped square around
where the parent stands *after* moving, off one coherent post-movement
snapshot, by count/rank/index over the shared cell index. The breeder is
subtracted before the draw, so a lone organism can no longer fertilise itself.

That forces a tick-contract change: conception leaves the organism visit and
becomes a pass of its own, so no breeder sees moved positions while another
sees the old ones.

## Picking the reach

ADR 0008 says the reach is a hypothesis to compare across fixed seeds, not a
knob to tune, and that the smallest rule keeping an isolated population viable
wins. Both halves matter, and they disagree.

An isolated population is viable at **reach 1**: Species A alone on the default
config reaches 8,081 through a trough of 308. But two species sharing one world
halve each other's local mate density, and that is where it breaks. Twins - two
identical populations, the strictest density test the repo has - at 60 epochs
of the default config:

| reach | cells | twins | Species A vs B |
| ---: | ---: | --- | --- |
| 1 | 9 | 0 / 0 | 0 / 9544 |
| 2 | 25 | 0 / 8895 | 0 / 9516 |
| 3 | 49 | 0 / 8772 | 760 / 8608 |
| **4** | **81** | **4884 / 3842** | **1226 / 7896** |

At reach 1 a run dies with **energy to spare**: mean energy climbs past 10
against a threshold of 8 while births go to zero. Organisms can afford to breed
and cannot find anyone. That is the model's own gap showing - nothing in it
searches for a mate. An organism holding surplus energy just idles until old
age; the kin-direction inputs are right there for a policy to evolve mate
search from, and the population is gone long before one does.

So the reach stands in for the walk that is not simulated. Four tiles is
roughly what a body covers in a few ticks, and it is still local by any measure
that matters: 81 cells of 16,384, against a draw that used to span the species.
It costs nothing measurable - only breeders pay it, and widening 1 to 4 moved a
100-epoch run from 16.74 s to 16.80 s.

## Design

```bash
for s in 1234 99 7 20260828 555 31337; do
  cargo run --release -p ecosym-cli --bin ecosym -- --seed $s
done
cargo run --release -p ecosym-cli --bin ecosym -- --seed 1234 --twins
```

One variable against the previous commit: mate selection. Perception, policy,
activation, crossover, mutation and the world are untouched.

## Results

| run | global mating | local mating |
| --- | --- | --- |
| seed 1234 | `4aa51da60fc894eb` | `c0fd53a79b897029` |
| seed 99 | `a816e56375a9f6db` | `bb6b6c4be7a5d712` |
| seed 7 | `1a26815391b1b972` | `c81a1e720211210f` |
| seed 20260828 | `f63668fb4813d8d5` | `a9148f0dd55a7b39` |
| seed 555 | `f765469b5d258a26` | `447c80428501d202` |
| seed 31337 | `5a9d815fd072962d` | `4344f5e163fcc3a4` |
| twins 1234 | `fc1961d58079b1ab` | `560d70e68ff58598` |

Populations at epoch 500, A / B:

| seed | global | local | winner |
| --- | --- | --- | --- |
| 1234 | 8467 / 171 | 9306 / 69 | A -> A |
| 99 | 6028 / 170 | **0** / 6005 | A -> **B** |
| 7 | 10207 / 1376 | 4075 / 8116 | A -> **B** |
| 20260828 | 7156 / 76 | 8616 / **0** | A -> A |
| 555 | **0** / 6905 | **0** / 6941 | B -> B |
| 31337 | 6918 / 775 | 9009 / 24 | A -> A |
| twins | 5268 / 3174 | **0** / 9516 | Twin 0 -> Twin 1 |

### Speed collapses

This is the result. Founder speed to survivor speed, every surviving species:

| seed | species | global mating | local mating |
| --- | --- | ---: | ---: |
| 1234 | A (1.300) | 1.025 | **0.544** |
| 20260828 | A (1.300) | - | **0.468** |
| 31337 | A (1.300) | - | **0.443** |
| 7 | A (1.300) | - | **0.497** |
| 99 | B (0.700) | - | **0.377** |
| 555 | B (0.700) | - | **0.392** |
| 7 | B (0.700) | - | **0.474** |

Movement per tick falls with it: Species A goes from 0.42 - 0.56 under global
mating to **0.15 - 0.25**. Under Phase 4 the winning strategy was persistent
ballistic sweeping - hold a heading, cover new ground linearly. Local mating
selects against exactly that. Moving away from your own kin costs you
reproduction, and that cost is not on the energy budget at all: it is paid in
offspring that never happen.

So a trait that was neutral-to-good is now expensive, and every lineage in
every seed walks it down. Dispersal has a reproductive price for the first
time.

### The loser becomes a plant

The species that loses no longer just dies. It contracts into a sessile
refuge:

```text
seed 1234, Species B      seed 31337, Species B
  speed      0.700 -> 0.334   0.700 -> 0.292
  movement   0.272 -> 0.013   0.277 -> 0.010
  rest       0.534 -> 0.927   0.526 -> 0.980
  size       1.000 -> 1.862   1.000 -> 1.525
  crowding   0.069 -> 0.557   0.082 -> 0.619
  climate fit 0.726 -> 0.864  0.693 -> 0.889
```

Large, immobile, packed together, and almost perfectly matched to the climate
where it is standing. It survives at 69 and 24 organisms by not going anywhere,
in a crowd dense enough to keep finding mates, on tiles it fits better than
anything else does. Nothing scripted that; it is what is left when the open
ground is gone and leaving is what kills you.

Realized `climate fit` rises in every survivor, 0.58 - 0.73 to 0.74 - 0.89.
Staying put lets a lineage match where it actually is.

### Cost

68 - 159 s against 79 - 153 s for the same seeds under global mating: unchanged
within the spread. Conception is a second pass and a second index rebuild per
tick, but only breeders search, and the search reads contiguous slices.

## Conclusion

**Keep it.** Geography now has reproductive consequence, and the consequence is
large enough to reverse the trait the previous phase selected for. That is the
strongest evidence available that the rule is load-bearing rather than
decorative.

Two things it cost, both recorded rather than tuned away:

- **A founding colony can now fail by being too thin.** Twins at 300 founders on
  64x64 go extinct in the trough at every reach up to 3, with energy to spare.
  The twin test moved to 500 founders on 96x96 for that reason, and says so.
- **The reach is doing work the model should eventually do itself.** Four tiles
  is standing in for a mate search that no organism performs. The honest fix is
  a policy that hunts for kin using the direction inputs it already has, and
  the honest test of that is whether the reach can then come back down.
