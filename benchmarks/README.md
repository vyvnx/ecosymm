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
- recorded: 2026-08-28, with evolved neural behaviour, impassable sea and
  resource dispersal

Default scenario - two species, 500 founders each, 500 epochs of 20 ticks
(10,000 ticks total) on a 128x128 world:

| metric | value |
| --- | --- |
| wall clock | 24.5 s |
| peak total population | 5,100 |
| final population | 4,658 (A 2,220 / B 2,438) |
| throughput | ~20 epochs/s, ~2.0M organism-ticks/s |
| replay digest | `4d8a1fd0975a2c99` |

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

Founder count vs wall clock, 200 epochs:

| founders per species | seconds | peak total population |
| --- | --- | --- |
| 125 | 7.72 | 2,496 |
| 500 | 8.66 | 5,042 |
| 2000 | 9.71 | 5,137 |

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
