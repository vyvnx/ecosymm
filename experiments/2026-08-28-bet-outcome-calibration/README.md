# what the three outcomes are actually worth

Coexistence has to be a real result, not a default. Too wide a margin and
"both are alive" swallows the market; too narrow and the middle button is
decoration. So the margin was measured before any settlement code was written,
and it is frozen at what the measurement said.

## The run

```bash
cargo run --release -p ecosym-cli --bin calibrate -- --runs 1000 --threads 8 \
  > experiments/2026-08-28-bet-outcome-calibration/table.txt
```

1,000 default-config runs on seeds 1..=1000, 500 epochs each, on the CPU
engine at `f5bdf80`. `table.txt` holds one row per seed - both initial and
final populations, both scores, and the replay digest - so any single run here
can be reproduced on its own with `cargo run --release -- --seed N` and
checked against its digest. Rows are sorted by seed, so the output does not
depend on how many threads produced it.

Nothing about the simulation was changed to get these numbers, and nothing
should be. This measures the world; it does not tune it.

## What 1,000 seeds look like

```text
void (both extinct)      0 (0.0%)
single survivor          211 (21.1%)
```

Total extinction never happened in 1,000 runs. It is still a rule of the
market - it voids and refunds - because a rule that has not fired yet is not
the same as a rule that cannot.

## Candidate margins

Coexistence is `abs(ln(score_a / score_b)) <= margin`, where
`score = final / initial`. The log ratio is what makes the band symmetric:
outrunning the other species by a factor and being outrun by its reciprocal
are the same distance from the middle.

| margin | Species A | Coexistence | Species B | coexistence share |
| ---: | ---: | ---: | ---: | ---: |
| 0.05 | 283 | 86 | 631 | 8.6% |
| 0.10 | 238 | 174 | 588 | 17.4% |
| **0.15** | **196** | **266** | **538** | **26.6%** |
| 0.20 | 147 | 350 | 503 | 35.0% |
| 0.25 | 110 | 418 | 472 | 41.8% |
| 0.30 | 76 | 487 | 437 | 48.7% |
| 0.40 | 38 | 580 | 382 | 58.0% |
| 0.50 | 16 | 649 | 335 | 64.9% |

## The choice: 0.15

The target was the smallest simple margin putting Coexistence in 20-35% of
non-void runs. 0.10 falls short at 17.4%; 0.20 reaches 35.0% but sits on the
ceiling, which leaves no room for the distribution to drift as the ecology
changes. 0.15 lands at 26.6%, in the middle of the band, and reads plainly:
**both species survived and neither grew more than about 16% faster than the
other.**

It is fixed as `ecosym_game::COEXISTENCE_MARGIN_V1` under rule version 1, and
every market stores the margin it settled under - so changing it later cannot
rewrite a market that already paid.

## The asymmetry is the world, not the game

At 0.15 the sides are not even: Species B takes 538 runs to Species A's 196.
Species B is the slow, cool-adapted, thrifty profile and this world rewards it.

That is left exactly as it is. Pari-mutuel odds already price an outcome
nobody expects to lose, so an uneven prior is information a bettor can use
rather than a fault to correct. Changing a species parameter to even up a
betting market would be tuning the ecology to suit the game, which is the one
thing the game layer may never do.
