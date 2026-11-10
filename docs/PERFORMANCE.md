# Performance

Measured numbers only. Every figure below was produced on the machine and
build described here; rerun the scripts to get numbers for your hardware.

## 1. Environment

| | |
|---|---|
| Machine | Apple M2 Max, 12 cores (8 performance + 4 efficiency), 64 GiB RAM |
| OS | macOS 27.0.1 (Darwin 27.0.0, arm64) |
| Toolchain | rustc 1.98.1, `--release` (thin LTO, `debug-assertions` off) |
| Threads | rayon with 12 threads |
| Date | 2026-11-10 |
| Load | 1-minute load average 13–20 while measuring: Spotlight and the file provider were re-indexing after large build-folder deletions (about two cores busy). Treat the numbers as conservative. |

## 2. How to reproduce

```bash
scripts/bench_scenarios.sh    # emergence-cli bench over 23 scenarios -> target/bench-logs/scenarios/
scripts/run_benchmarks.sh     # Criterion benches of every crate with benches/ -> target/bench-logs/
cargo bench -p ecosystem --bench ecosystem -- agents
```

`emergence-cli bench <scenario> --ticks N --warmup W [--param k=v]` times each
tick of the model on its own (`advance(1)`), after `W` untimed warm-up ticks,
with a monotonic clock. It reports ticks per second, mean / median / p95 /
min / max time per tick and the model's own memory estimate
(`Simulation::memory_bytes`). Rendering is not included: these are headless
model costs.

## 3. Headless simulation throughput

One "tick" is one call of the model's step. Its meaning depends on the model
(the unit column); for Hashlife with `generations_per_tick = 1024` one tick
advances 1,024 generations.

| Benchmark | Scenario and overrides | Tick unit | Ticks/s | Mean ms | p95 ms | Model memory |
|---|---|---|---:|---:|---:|---:|
| Life 512², dense | `conway/random-soup-torus`, torus 512×512, `engine=dense` | generation | 7,718 | 0.130 | 0.172 | 2.0 MiB |
| Life 512², bit-packed | same, `engine=bit` | generation | 16,012 | 0.062 | 0.097 | 1.5 MiB |
| Life 512², sparse tiles | same soup on the infinite plane (512×512 home region), engine auto → sparse | generation | 6,733 | 0.149 | 0.212 | 1.5 MiB |
| Life 2048², dense | torus 2048×2048, `engine=dense` | generation | 758 | 1.320 | 1.401 | 9.5 MiB |
| Life 2048², bit-packed | torus 2048×2048, `engine=bit` | generation | 7,726 | 0.129 | 0.181 | 2.5 MiB |
| Life 2048², sparse tiles | infinite plane, 2048×2048 home region, engine auto → sparse | generation | 1,007 | 0.993 | 1.068 | 2.5 MiB |
| Life, infinite puffers | `conway/infinite-puffer-world` (auto → sparse) | generation | 5,479 | 0.183 | 0.424 | 24.0 MiB |
| Hashlife, breeder | `conway/breeder-hashlife`, `generations_per_tick=1024` | 1,024 generations | 141,610 | 0.007 | 0.013 | 1.5 MiB |
| Hashlife, acorn | `hashlife/acorn` | generation | 133,052 | 0.008 | 0.010 | 3.0 MiB |
| OTCA metapixel | `metapixel/metapixel.galaxy` (15×15 metacells, 7.4 M live cells) | model step | 110 | 9.103 | 16.002 | 48.0 MiB |
| Lenia | `continuous/lenia.orbium` | 1/T time unit | 6,365 | 0.157 | 0.170 | 774 KiB |
| Lenia soup | `continuous/lenia.soup` | 1/T time unit | 1,459 | 0.686 | 0.720 | 3.0 MiB |
| SmoothLife soup | `continuous/smoothlife.soup` | step | 596 | 1.679 | 1.739 | 3.5 MiB |
| Flow-Lenia | `continuous/flow_lenia.demo` | time unit | 1,357 | 0.737 | 0.793 | 1.8 MiB |
| 3D Life 5766 soup | `voxel/life-5766-soup` | generation | 3,531 | 0.283 | 0.343 | 1.3 MiB |
| 3D Life clouds | `voxel/clouds-1` | generation | 2,761 | 0.362 | 0.435 | 1.3 MiB |
| Ecosystem, NetLogo rules | `ecosystem/wolf_sheep_grass` | 6 simulated minutes | 2,128 | 0.470 | 0.580 | 850 KiB |
| Ecosystem, meadow | `ecosystem/balanced_meadow` (11 species, 4,330 trees) | 6 simulated minutes | 679 | 1.473 | 5.914 | 3.3 MiB |
| Ecosystem, savanna | `ecosystem/african_savanna` | 6 simulated minutes | 670 | 1.493 | 6.353 | 2.8 MiB |
| Ecosystem, rainforest | `ecosystem/tropical_rainforest` | 6 simulated minutes | 696 | 1.437 | 6.110 | 2.9 MiB |
| Evolution | `evolution/speed_selection` | tick | 1,731 | 0.578 | 0.709 | 192 KiB |
| Civilization | `civilization/trade_routes` | simulated hour | 1,880 | 0.532 | 0.728 | 149 KiB |
| Custom Life | `custom-life/producers-consumers` | tick | 2,302 | 0.434 | 0.509 | 260 KiB |

Observations:

* The bit-packed engine is the fastest for dense finite worlds: about 2× the
  dense engine at 512² and 10× at 2048², where the dense engine's byte grid no
  longer fits the caches. Automatic engine selection picks it for finite
  worlds with enough cells.
* Hashlife's per-tick cost is tiny once the memo is warm because repeated
  structure is reused; its advantage is the size of the jump per tick, not the
  per-generation speed on chaotic soups.
* Ecosystem p95 values are about 4× the mean: the hourly field update (every
  10th tick) and the daily tree update make some ticks much more expensive
  than the others.

## 4. Ecosystem scaling with the number of animals

Criterion, `cargo bench -p ecosystem --bench ecosystem -- agents`: one
ordinary tick (6 simulated minutes) of a 160 × 160 grassland (3.2 km side,
rivers and lakes) with individual animals of five species after a 50-tick
warm-up.

| Animals | Time per tick (median, [95 % interval]) |
|---:|---|
| 1,000 | 1.676 ms [1.648, 1.691] |
| 10,000 | 15.19 ms [14.91, 15.43] |
| 100,000 | 770.9 ms [762.9, 778.4] |

From 1,000 to 10,000 animals the cost grows linearly. From 10,000 to
100,000 it grows about 50×: at that density the perception and interaction
queries see many more neighbours per animal, so the per-animal cost rises.
100,000 animals run at about 1.3 ticks per second, which is usable for
headless experiments but not for real-time viewing. The default caps are
5,000 animals per species and 20,000 in total; both are parameters.

## 5. Other measurements

Each model document records its own Criterion results, several of them
measured while other builds were running (their load is noted there):
`docs/models/cellular-automata.md` (engines), `docs/HASHLIFE.md`,
`docs/OTCA_METAPIXEL.md` (16 OTCA periods of the galaxy demo in about
174 ms), `docs/ECOSYSTEM.md` §21 (fields with 100 k and 10 M cells),
`docs/models/continuous.md`, `docs/models/voxel-life.md`,
`docs/models/evolution.md` and `docs/models/civilization.md`.

## 6. Rendering

The desktop app renders at the display refresh rate (120 Hz on the test
machine) in all nine laboratories in their default scenarios. The views'
reviews recorded these exceptions: dense rainforest costs about 16 ms per
frame at 1 tick/s (shadow pass over far trees), Ecosystem frames spike to
about 50 ms at Max speed while trees are re-sent, X-ray mode in 3D Life on the
200 k-voxel Clouds world runs at about 30 FPS (alpha blending), and the
Civilizations debug build drops to 80–90 FPS in its busiest scenes.
