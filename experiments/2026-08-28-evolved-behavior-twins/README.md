# Can behaviour itself be selected on?

## Question

Every organism now carries a small feed-forward policy (8 -> 8 -> 5, `tanh`)
in its genome, inherited by crossover and mutated at birth. Nothing scores a
policy. There is no fitness function, no reward, no training step - an organism
either survives and breeds or it does not.

So: does that alone move behaviour? And is any movement actually *behavioural*,
or just morphology wearing a new label?

## Design

The controlled scenario, `--twins`: two reproductively isolated species with the
**same physical blueprint** (Species A's: speed 1.3, size 1.0, metabolism 1.2,
heat_pref 0.62). Their founder brains are drawn from separate derived seeds, so
the founding policy is the only thing that differs between them. World and
climate are the usual limited, regrowing, fixed-climate field, with impassable
sea and local seed-rain dispersal.

```bash
cargo run --release -- --twins --seed 1234     --epochs 300
cargo run --release -- --twins --seed 99       --epochs 300
cargo run --release -- --twins --seed 7        --epochs 300
cargo run --release -- --twins --seed 20260828 --epochs 300
cargo run --release --                --seed 1234 --epochs 500   # default two-species scenario
```

## Results

| seed | habitable tiles | Twin 0 | Twin 1 | winner | digest |
| --- | --- | --- | --- | --- | --- |
| 1234 | 11,297 | 2,212 | 2,693 | Twin 1 | `41ac8c1b2583113b` |
| 99 | 8,552 | 3,376 | 0 | Twin 0 | `7d245d5588103eb5` |
| 7 | 13,011 | 3,416 | 2,953 | Twin 0 | `345841c9342fb593` |
| 20260828 | 10,326 | 1,107 | 3,038 | Twin 1 | `b758e1a433aac6ff` |

Same seed, same digest, both times (`--epochs 60`, `208e42e693260561` twice).

Every surviving population, in every seed, moved the same way:

| measure | founders | survivors |
| --- | --- | --- |
| movement / tick | 0.52 - 0.56 | 0.91 - 1.05 |
| food seeking | 0.47 - 0.52 | 0.94 - 0.99 |
| rest tendency | 0.48 - 0.50 | 0.04 - 0.11 |
| reproduction intent | 0.46 - 0.51 | 0.74 - 0.88 |
| brain drift / gene | 0 | 0.22 - 0.50 |
| mean energy | - | 5.5 - 6.5 |

Founders start at ~0.5 on every tendency because a few hundred random policies
average out to no opinion. Nothing told them which way to go from there.

### The crossover, in both directions

Identical bodies, and the early leader loses. Seed 1234:

```text
 epoch     total     biomass       Twin 0       Twin 1
    15       283      3716.9          194           89
    45      4082       956.7         3272          810
    75      4940       806.9         2939         2001
   120      5106       789.4         2572         2534
   285      4726       809.0         2189         2537
```

Seed 7, with the twins the other way round:

```text
 epoch     total     biomass       Twin 0       Twin 1
    30      2965      1914.9         1134         1831
    60      6253       876.3         2031         4222
   120      6460       819.6         2719         3741
   195      6523       806.0         3193         3330
   285      6325       811.0         3295         3030
```

One twin converts a full resource field into offspring faster and peaks near
epoch 45-60; the other overtakes once the field is grazed down. Which twin plays
which role flips between seeds, so it is not the slot - it is the policy each
one's founders happened to hold.

### Niche partitioning out of identical bodies

In all three seeds where both twins survive, they split the thermal niche, from
a shared founding `heat_pref` of 0.620:

| seed | Twin 0 | Twin 1 |
| --- | --- | --- |
| 1234 | 0.474 | 0.145 |
| 7 | 0.495 | 0.168 |
| 20260828 | 0.594 | 0.200 |

Nothing in the model asks them to divide the map. Two species with the same body
competing for one field find it cheaper to stop overlapping, and the policies
that take them to different latitudes are the ones that leave descendants.

## Conclusion

**Behaviour is under selection, and selection is doing the work.** Nothing
rewards moving, foraging or breeding; upkeep, intake, lifespan and the shared
field do. Policies that spend the movement half of their metabolic bill and get
to food outbreed policies that sit still, so their weights spread. Rest tendency
collapsing from 0.50 to 0.06 is the clearest reading: *energy conservation* was
available - resting is genuinely cheaper, and a resting organism still grazes
the tile under it - and this world rejected it.

**The early crash is the selection event.** Every run loses most of its founders
in the first ~15 epochs. Random policies are mostly lethal; the survivors are
not. Seed 1234 falls from 1,037 to 283 and comes back to 5,100.

**One founding draw can still end a species.** Seed 99 is the poorest world here
at 8,552 habitable tiles, and Twin 1 never got a foothold - 30 births before it
was gone. Physically identical species, no tuning, no handicap.

Nothing here is evidence of health, as usual. At epoch 300 seed 7 is running
155,000 births against 150,000 deaths on a world holding a sixth of its initial
biomass.

## What this does not show

No sophisticated strategy emerged. The policies converged on "move, seek food,
breed when you can, and get away from the other species" - the obvious answer to
this world, discovered rather than programmed, but still the obvious answer.
Competitor exposure stayed near 0.01 throughout, so nothing resembling resource
guarding or deliberate interference had a chance to appear: with 16,384 tiles
and ~6,000 organisms the two species barely meet, and the niche split means they
meet even less. A denser world, or one with somewhere worth defending, is where
to look for that next.
