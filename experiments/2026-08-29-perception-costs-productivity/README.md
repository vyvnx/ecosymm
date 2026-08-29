# What does it cost to stop handing every organism a food gradient?

## Question

Until now `actions::stride` blended the policy's own heading with a unit vector
that `observations::scan` had already pointed at the best of four neighbouring
tiles. The only choice a policy made about food was a scalar `seek` saying how
much of that answer to obey, and a few hundred random founders average `seek`
at 0.5 - so **every organism ever born was a half-strength food-follower before
selection did anything**.

ADR 0008 removes it. The resource direction becomes two signed inputs among
twelve, the policy grows to `12 -> 8 -> 4`, and nothing points an organism at
food except weights it inherited. The descriptive `food_seeking` statistic -
which reported an internal pressure - becomes measured **resource tracking**:
the normalized dot product of where the organism actually went against where
the food actually was, in `-1..1`.

So: does anything survive that, and does the behaviour it was supposed to buy
actually evolve?

## What happened first: everything died

Every species went extinct in every seed, including each profile run alone.
`each_profile_is_viable_on_its_own` failed for **both** profiles, not just the
expensive one.

The perception itself was fine. A hand-built follower population - the
resource-direction inputs wired straight to the two heading outputs - reached
4,302 organisms at a measured tracking of 0.831, so the observation is
actionable and the world is habitable by anything that uses it.

The founder generation was the problem, and two faults that the actuator had
been hiding were behind it. Both predate this change.

### The land was colder than the sea

`climate::temperature_field` subtracted `0.35 * elevation` from a latitude
band. Land is by definition above `SEA_LEVEL`, so every habitable tile paid an
altitude penalty measured from zero instead of from the water line.

```text
seed 1234, land tiles only
64x64    min 0.00  p25 0.02  med 0.22  p75 0.45  max 0.74  mean 0.25
128x128  min 0.00  p25 0.11  med 0.31  p75 0.48  max 0.79  mean 0.30
```

The scenario profiles are a symmetric warm/cool pair - 0.62 and 0.38 - about a
mean of 0.5 that the world never had. At the median tile Species A's
`climate_fit` was 0.49 against Species B's 0.85: a permanent ~40% tax on
everything the warm-adapted species ate, on top of its higher upkeep and its
shorter life.

The old runs had been saying so all along. `heat_pref` for Species B drifts to
**0.119, 0.122, 0.131, 0.165, 0.210** across the recorded baselines - straight
at the true land mean, nowhere near the authored one.

A trait grid isolated it. Holding the profile and moving only one gene, births
over 20 epochs alone:

| profile | births |
| --- | ---: |
| A: speed 1.3, metabolism 1.2, heat 0.62 | 5 |
| speed 1.3, metabolism **0.8**, heat 0.62 | 23 |
| speed **0.7**, metabolism 1.2, heat 0.62 | 4 |
| speed 1.3, metabolism 1.2, heat **0.38** | 20 |
| B: speed 0.7, metabolism 0.8, heat 0.38 | 21 |

Speed is nearly neutral. Metabolism and heat preference each cost about the
same, and Species A was paying both.

### Productivity was calibrated against the subsidy

Land capacity ran 0.20 - 0.70, mean 0.35, so a grazed tile regrows about 0.021
per tick. Basal cost alone is 0.06 for Species A and 0.04 for Species B: **a
stationary organism cannot pay its own upkeep from a tile.** The only viable
strategy is to keep reaching ungrazed ground, and a founder policy moves 0.2 -
0.4 tiles per tick, under the one tile that takes.

Under the actuator this never mattered - every organism was a follower from
tick one, so the world only ever had to feed foragers. Once foraging has to be
evolved, the founder generation is by definition the one that has not evolved
it, and it starves before selection has anything to select.

`intake` is a dead parameter besides. `0.9 * size * metabolism` is 1.08 for
Species A against a mean tile capacity of 0.35, so it never binds, and
metabolism is pure cost - which `phenotype.rs` already predicted in a comment.

Sweeps confirmed which fault was binding: raising `REGROWTH` alone to 3.3x
still left Species A extinct at 7 births, and lowering `intake` alone did
nothing.

## Design

Two world changes, both symmetric across species, neither handing anyone a
strategy. See `docs/adr/0010`.

```rust
// climate.rs: the lapse rate is measured from sea level
let altitude = (terrain.elevation[i] - SEA_LEVEL).max(0.0);
let t = lat * 0.8 + 0.2 * terrain.wetness[i] - 0.35 * altitude;

// terrain.rs: primary productivity doubles
(0.4 + 3.2 * w * (e - SEA_LEVEL)).min(2.0)
```

Land temperature moves to a mean of 0.38 - 0.44. Productivity x2 was chosen
against the whole sweep, run alone for 20 epochs on the small config:

| fertility | A final / births | B final / births |
| --- | --- | --- |
| x1 | 0 / 10 | 3 / 20 |
| **x2** | **65 / 232** | **91 / 219** |
| x3 | 519 / 1389 | 774 / 1376 |
| x4 | 772 / 2580 | 982 / 2176 |

x1 kills everything; x3 and x4 peg the allocation ceiling within twenty epochs
and flatline. x2 is the only setting that booms, busts and survives.

The allocation guard moved too. `safety_ceiling` was `founders * species * 10`,
and at the new productivity it started refusing births the world could still
feed - seed 7 printed *"the population safety ceiling refused births during
this run"*. It now also takes `width * height * 4`, because what bounds a
population eating one shared field is tiles, not how many founders were asked
for. That guard is allocation protection and must never be carrying capacity.

```bash
cargo run --release -p ecosym-cli --bin ecosym -- --seed 1234
cargo run --release -p ecosym-cli --bin ecosym -- --seed 99
cargo run --release -p ecosym-cli --bin ecosym -- --seed 7
cargo run --release -p ecosym-cli --bin ecosym -- --seed 20260828
cargo run --release -p ecosym-cli --bin ecosym -- --seed 1234 --twins
```

## Results

500 epochs, default config. Baseline is `310eba7`, the commit before this one.

| run | old digest | new digest |
| --- | --- | --- |
| seed 1234 | `de08609476524f6c` | `cef4dae3963e179e` |
| seed 99 | `c1869dd1ad5c8184` | `c44e0aa451e08ff1` |
| seed 7 | `57c70de486d17f55` | `cb00aadf19253841` |
| seed 20260828 | `bf17c03a2a107415` | `bb90fa361f0518ee` |
| seed 555 | - | `6b641a5263767227` |
| seed 31337 | - | `9d1361edd00f18d4` |
| twins 1234 | `b77d676f146c37b0` | `bf44f42a35fffa7f` |

Populations at epoch 500, A / B:

| seed | old | new | winner |
| --- | --- | --- | --- |
| 1234 | 2402 / 2343 | 8807 / 0 | A -> A |
| 99 | 0 / 3385 | 0 / 6417 | B -> B |
| 7 | 3410 / 2880 | 5336 / 6726 | A -> **B** |
| 20260828 | 1862 / 2354 | 7410 / 0 | B -> **A** |
| 555 | - | 7142 / 174 | A |
| 31337 | - | 7866 / 0 | A |

### The behaviour that was bought

Founder draw -> survivors, across the six seeds:

| measure | old founders | old survivors | new founders | new survivors |
| --- | --- | --- | --- | --- |
| food seeking / resource tracking | 0.48 - 0.52 | **0.965 - 0.987** | 0.11 - 0.19 | **0.02 - 0.49** |
| movement / tick | 0.25 - 0.51 | 0.78 - 0.87 | 0.24 - 0.50 | 0.04 - 0.48 |
| rest tendency | 0.47 - 0.52 | 0.03 - 0.06 | 0.50 - 0.57 | 0.18 - 0.84 |
| competitor exposure | 0.005 - 0.007 | 0.003 - 0.009 | 0.07 - 0.10 | 0.00 - 0.50 |
| climate fit | not measured | not measured | 0.58 - 0.71 | 0.68 - 0.81 |

The old column is the same number in every seed because it was not a result:
`seek` was free to have, so it went to ~0.97 whatever the world was doing. The
new one splits into **two regimes**, and which one a seed lands in is decided by
whether the world stays patchy.

**Trackers** - seeds 1234, 20260828, 555, 31337, and both twins. One species
takes the open ground, competitor exposure falls to ~0, and tracking climbs from
a ~0.14 founder draw to 0.28 - 0.49. Food is worth walking to, so walking to it
is selected.

```text
seed 20260828, Species A
  resource tracking   0.142 -> 0.488    rest         0.517 -> 0.177
  competitor exposure 0.081 -> 0.000    climate fit  0.656 -> 0.694
```

**Grazers** - seed 7, where both species saturate together. Tracking *falls*,
0.192 -> 0.139 and 0.192 -> 0.069, while competitor exposure goes to 0.45 -
0.48. At saturation the world is uniformly grazed, the resource direction
carries no information, and moving is pure cost.

Seed 555 has one of each in the same run. Species A tracks its way to 7,142;
Species B is squeezed into a crowded remnant of 174 and stops moving at all -
rest 0.566 -> **0.837**, movement 0.238 -> **0.041**, exposure 0.442. Nothing
scripted that. It is what is left when the other species has taken the ground.

The competitor-exposure jump at founding, 0.005 -> 0.08, is not ecology: the old
channel counted rivals on the same tile, the new one is density over the
eight-cell stencil. Different instrument, not a different world.

### Twins: the policy alone still decides it

Identical bodies, separate brain seeds:

| | old | new |
| --- | --- | --- |
| Twin 0 | 2258, seeking 0.977, exposure 0.005 | **1039**, tracking 0.282, exposure **0.501** |
| Twin 1 | 2576, seeking 0.987, exposure 0.005 | **7368**, tracking 0.358, exposure **0.141** |

Old twins finished within 14% of each other with the same behaviour, because the
free actuator had flattened the thing that separated them. New twins differ by
7x, and the surviving one is the one that tracked better and stayed out of the
crowd. This is what the twin scenario was built to detect and could not.

### heat_pref means something again

Old survivors converged on 0.119 - 0.210 for the cool profile: the floor of a
mis-centred band. New survivors sit inside the land band and move with the
seed's own geography, and the realized `climate fit` **rises** in every
surviving species - 0.578 -> 0.703, 0.606 -> 0.699, 0.656 -> 0.694. A species is
now measurably evolving into where it lives, which is what that channel was
added to be able to say. The climate axis on a species card is reporting a niche
rather than an artifact.

## Conclusion

**Removing the actuator was correct and the world could not pay for it.** The
change exposed two pre-existing faults - a lapse rate measured from the wrong
datum, and a productivity constant calibrated against a subsidy - and neither
was visible while every organism was a forced forager.

Fixed, the phase does what it was for: resource tracking is now an evolved
behaviour with a founder baseline of ~0.14, it reaches 0.5 where tracking pays,
and it *decays* where it does not. Two selection regimes exist where there was
one. The twin scenario has teeth.

Every digest changes, on every seed, which is expected: the observation vector,
the policy shape, the movement decode and the world's two constants all differ
from tick one.

### What this leaves owing

`MarketRules::V1.coexistence_margin` is frozen at 0.15 on the strength of 1,000
runs in `2026-08-28-bet-outcome-calibration`, whose outcome distribution no
longer holds - four of the six seeds here end with a species extinct. Measured
again, coexistence has gone from 26.6% of runs to **0%**, and no candidate
margin recovers it. The margin is not moved, because moving it cannot fix a
distribution with nothing in the middle:
`experiments/2026-08-29-coexistence-collapses` has the numbers and what they
actually ask for.

`FOOD_HERE` had to be renormalised in the same commit. It was
`resource_at(tile).clamp(0.0, 1.0)`, harmless while tile capacity topped out at
0.70 and silently flattening the richest quarter of the world into one number
once it did not. It now reads against this body's own `intake`, so 0.5 means
"exactly one tick's mouthful is standing here" and no world constant can clip
it again.
