# Two-species crossover, and how much the seed decides

> **Superseded.** These numbers and digests come from the model *before*
> organisms had evolved neural policies, before the sea became impassable and
> before resources dispersed between tiles. They no longer reproduce - the
> commands still run, but they describe a different simulation. Kept because the
> question and the reasoning are still the useful part; see
> `experiments/2026-08-28-evolved-behavior-twins/` for the current model.

## Question

The default scenario is not tuned to pick a winner. Does the result actually
come out of the world and the genetic tradeoffs, or is one profile just better?

## Commands

```bash
cargo run --release -- --seed 1234 --epochs 120
cargo run --release -- --seed 1234              # 500 epochs
cargo run --release -- --seed 99     --epochs 120
cargo run --release -- --seed 20260828 --epochs 120
```

## Results

| seed | epochs | Species A | Species B | winner | digest |
| --- | --- | --- | --- | --- | --- |
| 1234 | 120 | 1,434 | 3,468 | B | `9bcc2f694e74c2ed` |
| 1234 | 500 | 2,464 | 2,281 | A | `96f429c0fdb790f2` |
| 99 | 120 | 0 | 3,782 | B | `5010d6be6315a519` |
| 20260828 | 120 | 1,543 | 2,961 | B | `f3cee2c4dc892538` |

Running seed 1234 twice gives the same digest both times.

## Conclusion

The winner is not a property of the profiles. It is a property of the world and
of *when you stop looking*:

- **Early, B wins.** The cheap, cool-adapted profile converts a full resource
  field into offspring faster, and peaks near 4,270 at epoch 50 while A is
  still under 700.
- **Late, A overtakes.** B's own bloom grazes the field down to ~660 standing
  biomass. In a grazed world, intake stops mattering and the expensive, fast,
  warm-adapted profile catches up. The two cross between epochs 300 and 325 and
  finish within 8% of each other.
- **The seed can end it outright.** Seed 99 generates a poorer world - 8,552
  habitable tiles against 1234's 11,297, and 3,216 initial biomass against
  4,084. A never gets a foothold there and is extinct inside 120 epochs, with
  39 births to its name. Mean temperature is the same 0.33-0.34 in all three
  worlds, so this is about how much habitable land there is, not about heat.

Note that every generated world averages ~0.33 temperature, so A's warm 0.62
profile starts mismatched almost everywhere while B's 0.38 starts near the mean.
A is not handicapped by tuning - it is handicapped by the world - and it still
wins the long run once B has eaten the easy calories.

Both species converge on metabolism ~0.24 (the floor documented in
`ecology/phenotype.rs`) but *diverge* on `heat_pref` - A settles near 0.45, B
near 0.12. That is niche partitioning falling out of a shared resource field,
not something the model was told to do.

Nothing here is evidence of health. At epoch 500 both species are running ~82k
and ~120k births against nearly as many deaths on a world holding a sixth of its
initial biomass. A won; neither thrived.
