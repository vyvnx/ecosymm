# Does charging for the wander change what gets selected?

## Question

Every organism's step was the stride its policy asked for **plus** a random
offset of up to `MOVE_NOISE` (0.3) on each axis, added after `stride` had
already calculated `effort`. The offset moved the organism and cost it nothing.

That is free displacement, and it broke three things at once:

- **Rest was not rest.** A policy resting at 1.0 still drifted up to 0.42 tiles
  per tick, grazing whatever it drifted onto, for the basal bill alone.
- **The movement bill understated the movement.** `movement/tick` counted the
  distance travelled; `effort` did not, so the energy accounting and the
  reported behaviour disagreed with each other.
- **Selection was reading a subsidy.** The cheapest way to cover ground was to
  ask for nothing and let the noise carry you, which is not a strategy anything
  in the world should be able to choose.

So: fold the wander into the intended displacement before `effort` is
calculated, let rest suppress it like anything else, and charge it. Does the
world still work, and does anything about selection change?

## Design

`actions::stride` now takes the tick's wander as a fourth argument and adds it
to the blended heading, before the length cap, the rest multiplier and the
effort calculation. `live_one_tick` no longer adds anything to the returned
stride. A blocked shoreline step is unchanged: `effort` is charged on the
intention, so walking into the sea still costs what walking anywhere else would.

```bash
cargo run --release -- --seed 1234     --epochs 200
cargo run --release -- --seed 99       --epochs 200
cargo run --release -- --seed 7        --epochs 200
cargo run --release -- --seed 20260828 --epochs 200
```

## Results

| seed | old digest | new digest | old wall | new wall |
| --- | --- | --- | --- | --- |
| 1234 | `4391ce6e3a9a3ccc` | `5e983c8092ca50d6` | 15.42s | 15.07s |
| 99 | `85311fffa21499c1` | `cec9f5dac4dd5f27` | 9.65s | 9.65s |
| 7 | `7627d99581db7ac0` | `963ffc9f27f90e82` | 22.74s | 21.57s |
| 20260828 | `e7b01fe85151f09d` | `2a6d0599bf0c9508` | 13.31s | 13.45s |

Populations at epoch 200, A / B:

| seed | old | new | winner (old -> new) |
| --- | --- | --- | --- |
| 1234 | 2236 / 2703 | 2214 / 2675 | B -> B |
| 99 | 855 / 2773 | 0 / 3527 | B -> B |
| 7 | 3806 / 2619 | 3327 / 3256 | A -> A |
| 20260828 | 1984 / 2350 | 2070 / 2326 | B -> B |

Behaviour, founders -> survivors, averaged over the four seeds:

| measure | old founders | old survivors | new founders | new survivors |
| --- | --- | --- | --- | --- |
| movement / tick (A) | 0.55 | 0.92 - 1.00 | 0.49 | 0.75 - 0.87 |
| movement / tick (B) | 0.36 | 0.77 - 0.82 | 0.25 | 0.61 - 0.69 |
| rest tendency (B) | 0.51 | 0.13 - 0.17 | 0.51 | 0.13 - 0.21 |
| food seeking | 0.49 | 0.89 - 0.96 | 0.49 | 0.87 - 0.95 |
| reproduction intent | 0.50 | 0.64 - 0.89 | 0.50 | 0.62 - 0.77 |

Reported founder movement drops immediately (0.55 -> 0.49 for A, 0.36 -> 0.25
for B) because the free 0.2-tile average drift is gone from tick one: a founder
that asks for nothing now goes almost nowhere. Survivor movement drops by the
same order and stays well above the founders, so movement is still what
selection buys - it just costs what it weighs now.

Nothing else moved much. Food seeking still converges to ~0.9, rest still
collapses, reproduction intent still climbs, and the winner is unchanged in all
four seeds.

Seed 99 is the one visible outcome change: Species A held on at 855 with the
subsidy and goes extinct without it. That is the same seed that already killed a
twin in `2026-08-28-evolved-behavior-twins` - it is the poorest world in the set
(8,552 habitable tiles), Species A is the expensive body, and removing a
movement subsidy is exactly the pressure that finishes a marginal population.

## Conclusion

**Charging for the wander is ecologically required and costs nothing else.**
Energy is now conserved across every displacement in the model: there is no path
by which an organism moves without paying, and rest genuinely means standing
still. Runtime is unchanged within noise.

The digests change on every seed, which is expected: the wander enters the
heading before normalisation, so both the direction and the length of every step
differ from the first tick onward.

Selection outcomes are qualitatively identical. The one flipped survival (seed
99, Species A) is a marginal population in the poorest world losing a subsidy,
not a change in what the world rewards.
