`./benchmarks/bench.sh` times the CLI at three founder-population sizes and
reports the peak total population each run reached.

No criterion dependency yet. Add one when a micro-benchmark of the tick loop is
what is actually needed - that is also the point where `crates/gpu` stops being
empty.

## CPU baseline

The reference the first GPU backend has to beat, and the thing that says which
workload is worth moving to a device at all.

- host: AMD Ryzen 7 5800XT (8 cores), Linux 6.18 (WSL2), rustc 1.98.0
- build: `cargo run --release` (`lto = "thin"`, `codegen-units = 1`)
- engine: `cpu`, single-threaded
- recorded: 2026-08-29, with evolved neural behaviour, impassable sea, resource
  dispersal, wander charged as movement effort, the CSR cell index, and
  directional perception

Default scenario - two species, 500 founders each, 500 epochs of 20 ticks
(10,000 ticks total) on a 128x128 world:

| metric | directional perception | free gradient actuator |
| --- | --- | --- |
| wall clock | 67.7 s | 29.1 s |
| peak total population | 8,807 | 4,745 |
| mean population | 8,395 | 4,519 |
| organism-ticks | 84.0 M | 45.2 M |
| throughput | 7.4 epochs/s, **1.24 M organism-ticks/s** | 17.2 epochs/s, 1.55 M/s |
| replay digest | `cef4dae3963e179e` | `de08609476524f6c` |

Both columns are seed 1234 measured on this host; the right-hand one is
`310eba7`, checked out in a worktree so the two builds are the same compiler on
the same machine. Mean population is the mean of the 21 sampled epoch rows, so
organism-ticks are an estimate, not a count.

**The tick got 1.25x more expensive and the world got 1.9x more crowded.** Wall
clock is 2.3x because those multiply. Only the first number is a cost of this
change: the second is `experiments/2026-08-29-perception-costs-productivity`
doubling primary productivity so anything survives at all, and a world holding
twice as many organisms taking twice as long is the ecology, not the code.

Where the 1.25x went:

| | before | after |
| --- | ---: | ---: |
| stencil probes per organism-tick | 4 | 8 |
| cell-index lookups per probe | 1 | 2 (kin and rivals) |
| policy multiply-accumulates | 104 (`8 -> 8 -> 5`) | 128 (`12 -> 8 -> 4`) |
| `tanh` calls | 13 | 12 |

Doubling the stencil is what it cost, and it is what bought a *direction*: four
probes can rank tiles, eight can say which way. The network is the cheaper half
of the change and one `tanh` lighter than before.

The `--population-per-species` sweep now says something it did not:

| founders per species | seconds (200 epochs) | peak population |
| ---: | ---: | ---: |
| 125 | 41.6 s | 9,565 |
| 500 | 52.5 s | 9,473 |
| 2,000 | 65.1 s | 9,488 |

Peak population is flat across a 16x range of founders, where before it tracked
them (2,496 / 4,952 / 5,128). The world, not the founding stock, is what decides
how many organisms it holds - which is what carrying capacity binding looks like,
and it was not binding before.

### What the policy costs

Before organisms had brains the same run took **2.92 s** (digest
`96f429c0fdb790f2`). Adding a forward pass per organism per tick took it to
21.1 s, and resource dispersal added 3.4 s on top for a total of 8.4x. Roughly
half the whole bill is `tanh`:

| activation | wall clock | note |
| --- | --- | --- |
| `tanh` | 21.1 s | shipped. 13 libm calls per organism-tick |
| `x / (1 + \|x\|)` | 10.0 s | evolves just as well, bit-identical across platforms |

(both measured before dispersal landed, so both are 3.4 s light; the ratio is
the point.) Dispersal itself is one extra fullness pass and four neighbour reads
per tile per tick - cheap per tile, but it runs on all 16,384 of them whether
anything is standing there or not.

The softsign swap is one line in `crates/genetics/src/neural_genome.rs` and is
documented there. It is not taken because 21 s is fast enough and `tanh` is the
shape a reader expects; re-record this table if that changes.

Founder count vs wall clock, 200 epochs, with the `Occupancy` tile counts and
the CSR cell index that replaced them measured back to back on the same host:

| founders per species | occupancy | cell index | peak total population |
| --- | --- | --- | --- |
| 125 | 8.10 s | 8.15 s | 2,495 |
| 500 | 8.31 s | 8.47 s | 5,010 |
| 2000 | 8.59 s | 8.98 s | 5,138 |

The index costs 2-5% and buys contiguous per-cell membership. Its own cost,
measured directly at populations the default world cannot sustain
(`cargo test --release -p ecosym-ecology -- --ignored --nocapture`):

| organisms | spread over the map | all on one tile |
| --- | --- | --- |
| 5,000 | 51 us/rebuild | 78 us/rebuild |
| 10,000 | 114 us/rebuild | 106 us/rebuild |

Linear in population and flat against clustering, which is the property a
counting sort is chosen for: the clustered worst case is a longer run of writes
into one bucket, not a scan. A tick at 5,000 organisms costs about 2.1 ms, so
the index is ~2.4% of it.

**Carrying capacity, not the ceiling, is why 10,000 organisms do not appear in
the CLI rows.** The 128x128 world saturates near 5,100 whatever it is founded
with, so the 10,000-organism target has to be measured on the index directly
until a world-size option exists.

At 500 and 2,000 founders the population saturates near 5,000: carrying capacity
binds and the extra founders buy nothing but a slower start. The 125-founder row
is a different story and always was - its safety ceiling is
`(125 * 2).max(100) * 10 = 2,500`, so that run is ceiling-bound, not
capacity-bound, and the CLI says so. It plateaus at ~2,495 and holds there
through 400 epochs.

The useful reading for GPU work has not changed in kind, only in size. The hot
loop is now ~5,000 organisms x 10,000 ticks of an 8->8->5 forward pass on top of
the tile scoring, harvesting and upkeep that were already there: a great deal
more arithmetic per organism-tick, all of it the same arithmetic for every
organism, which is a much better fit for a device than the pre-brain loop was.

## Browser renderer baseline

The WebGL2 renderer from `docs/browser-render-engine-plan.md`, measured through
the counters on `WorldRenderer` (`__ecosym.stats()` in the console). Frame times
come from the `requestAnimationFrame` loop; the decode figure is one wall-clock
span around `decode` plus `setSnapshot`, so it covers protocol decoding, id
reconciliation and both GPU uploads together.

- host: as above, Chromium 148 headless via Playwright, DPR 1
- GPU: **discrete** - ANGLE / D3D12 on an NVIDIA RTX 4060 Ti. The plan's gate
  names an integrated GPU, so that gate is *not* demonstrated by this row.
- recorded: 2026-08-28

At the 10,000-organism target (synthetic stream, 128x128 world, 15 Hz, a tenth
of the ids replaced per snapshot so 12,000 points are drawn during every
transition):

| metric | value | gate |
| --- | --- | --- |
| frame time p50 | 16.7 ms | - |
| frame time p95 | 16.8 ms | 16.7 ms, vsync-locked at 60 fps |
| decode + reconcile + upload p50 | 2.0 ms | - |
| decode + reconcile + upload p95 | 3.6 ms | under 8 ms |
| worst single snapshot | 5.7 ms | - |
| draw calls per ordinary frame | 2 | 2 |
| buffer uploads per ordinary frame | 0 | 0 |
| snapshot size | 256,416 B | ~256 KiB predicted |

p95 sits one 60 Hz frame above p50 because the loop is vsync-bound: the renderer
is not the thing deciding the frame rate at this size.

The default run (two species, 500 founders, 128x128), live off the server:

| metric | value |
| --- | --- |
| organisms at steady state | ~4,500 - 5,000 |
| snapshots per second | 10.3 (ceiling is 15) |
| bandwidth | ~1.3 MB/s |
| frame time p50 / p95 | 16.7 / 16.8 ms |
| epochs per second, visualised | 15.4 |
| epochs per second, CLI (no visualisation) | 16.9 |
| retained JS heap over 70 s | 13 - 25 MB, sawtooth, no trend |

Sampling costs the simulation about **9%** of its epoch rate. That is the
extraction copy plus encoding on the producer thread; it is well inside the
budget, and the first lever if it ever is not is `SAMPLE_INTERVAL`, not a more
complicated readback.

Verified end to end at 80 epochs: the final rendered organism count equals the
final `EpochReport.population` (5,042), and the browser's replay digest
`5f0f5263d91a1074` is byte-identical to the CLI's for the same seed and config.
Watching a run does not change it.

Not yet measured: an integrated GPU, a real mobile device (the 390x844 layout
was checked on desktop Chromium, where it also holds 60 fps), DPR above 2, and
a browser with no WebGL2 at all.
