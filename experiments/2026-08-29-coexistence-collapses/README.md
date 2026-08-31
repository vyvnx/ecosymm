# The middle button stopped happening

## Question

`MarketRules::V1.coexistence_margin` is frozen at **0.15** on the strength of
`2026-08-28-bet-outcome-calibration`: 1,000 default-config runs, coexistence in
26.6% of them, chosen as the smallest simple margin landing inside a 20 - 35%
target band. The rule the margin implements is
`abs(ln(score_a / score_b)) <= margin`, where `score = final / initial`.

`2026-08-29-perception-costs-productivity` changed the ecology underneath it:
foraging became an evolved behaviour instead of a free actuator, the land
climate band was re-centred, and primary productivity doubled. Four of the six
seeds recorded there end with a species extinct.

So: does the frozen margin still describe this world?

## Design

The same tool, the same contiguous seed range, a smaller sample. The new
ecology is slower - populations roughly doubled - so 1,000 runs is no longer a
twenty-minute measurement; 80 seeds is enough to answer whether the
distribution moved by a lot or a little, and it moved by a lot.

```bash
cargo run --release -p ecosym-cli --bin calibrate -- --runs 80 --threads 8 \
  > experiments/2026-08-29-coexistence-collapses/table.txt
```

Nothing was changed to get these numbers. `table.txt` holds one row per seed -
initial and final populations for both species, both scores, and the replay
digest - so any row can be reproduced on its own with
`cargo run --release -p ecosym-cli --bin ecosym -- --seed N`.

## Results

| | 1,000 runs, free actuator | 80 runs, evolved foraging |
| --- | ---: | ---: |
| void (both extinct) | 0 (0.0%) | 0 (0.0%) |
| single survivor | 211 (21.1%) | **38 (47.5%)** |
| Species A wins | 196 (19.6%) | **62 (77.5%)** |
| Species B wins | 538 (53.8%) | 18 (22.5%) |
| coexistence at 0.15 | 266 (26.6%) | **0 (0.0%)** |

Coexistence share of non-void runs, by candidate margin:

| margin | old | new |
| ---: | ---: | ---: |
| 0.05 | 8.6% | 0.0% |
| 0.10 | 17.4% | 0.0% |
| **0.15 (frozen)** | **26.6%** | **0.0%** |
| 0.20 | 35.0% | 0.0% |
| 0.30 | 48.7% | 1.2% |
| 0.50 | 64.9% | 2.5% |

## What this says

**Two things broke, and only one of them is the margin.**

The first is that coexistence is not a narrow band any more, it is an empty
one. Widening the margin does not recover it: at 0.50 - a species growing 65%
faster than the other and still called a draw - it reaches 2.5%. There is no
value of this parameter that puts the middle button back in the 20 - 35% band,
because the runs are not landing near the middle at all. In the 42 runs where
both species survive, the survivor is typically an order of magnitude ahead.

The second is that the contest itself became lopsided. Species A now takes
77.5% of runs where it used to take 19.6%. Under the actuator both species
foraged optimally by construction, so the outcome turned on body economics and
climate, which were close. Now the outcome turns on **which species finds
resource tracking first**, and once one does it takes the open ground and the
other never gets it back. That is a winner-take-all dynamic on a single shared
resource pool, and it is what a real competitive-exclusion result looks like.

Both are honest ecology and neither is a bug in the simulation. Both are a
problem for the *game*: a three-outcome market where one outcome is impossible
and another wins three runs in four is a market a rational bettor solves once
and stops finding interesting.

## What is not being done here

**The margin is not moved.** Moving it cannot fix a distribution with nothing
in the middle, and re-freezing a game parameter against an 80-seed sample would
replace a measured constant with a guessed one.

The real question this raises is ecological, not economic: **this world has one
niche and two species competing for it.** Coexistence needs resource
partitioning that survives selection - the climate axis is the obvious
candidate now that the land band is centred and genuinely spans 0.0 to 0.93,
but under the actuator it was never load-bearing and under the new productivity
the winner grows fast enough to occupy both ends of it before the loser can
specialise into one.

That is a scenario and ecology hypothesis with its own experiment to run, and
it has to be settled before the market is recalibrated. Recalibrating first
would freeze a second margin against a world that is about to change again.
