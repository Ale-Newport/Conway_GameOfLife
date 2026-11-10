# Roadmap

This page records the status of each phase of the original plan, as found in
the repository on 2026-11-10 (commits, the working tree and the docs). It then
lists the known limitations and next steps, collected from the model and
system documents.

Status labels:

* **Done**: committed, and documented in `docs/`.
* **In progress**: present in the working tree but not committed, or
  incomplete.
* **Not started**: no trace in the repository.

## 1. Phases

| # | Phase | Status | Evidence |
|---|---|---|---|
| 1 | Research | Done | `docs/RESEARCH.md` and nine notes in `docs/research/`; verified 80-pattern library (`c97c2f6`); 3D model library with manifest (`bd2141b`). |
| 2 | Architecture and core | Done | `docs/ARCHITECTURE.md` (`617bb80`); `simulation-core` (`1f0a48f`, `e377d8c`); foundation crates `patterns`, `hashlife`, `organism-engine`, `physics-world`, `analytics`, `persistence`; presentation crates `rendering`, `organism-render`, `interface` and `apps/desktop` (`b0d4222`). |
| 3 | Classic Conway | Done | `cellular-automata` (`8d86271`); `view-conway` (`c545bb5`), registered in the desktop app (`c111bb7`). |
| 4 | Ecosystem Simulator | Done | `ecosystem` (`9dcd897`; `docs/ECOSYSTEM.md`); `view-ecosystem` (`1d48581`), registered in the desktop app in the same commit. |
| 5 | Custom Life Creator | Done | `rule-engine` (`d9144d2`); `view-custom-life` (`1d75764`), registered (`43573bf`). |
| 6 | Conway within Conway | Done | `metapixel` (`12904f1`); `view-metapixel` (`5a548bf`), registered (`c111bb7`). |
| 7 | Cellular Automata Lab | Done | Models in `cellular-automata`; `view-ca-lab` (`3a0efa2`), registered (`c111bb7`). |
| 8 | Evolution Lab | Done | `evolution` (`83f6c0e`); `view-evolution` (`dd89f14`), registered (`43573bf`). |
| 9 | Continuous Life | Done | `continuous` (`86b04bf`); `view-continuous` (`d12e747`), registered (`43573bf`). |
| 10 | 3D Life | Done | `voxel-life` (`4c39b20`); `view-voxel` (`f818f26`), registered (`43573bf`). |
| 11 | Emergent Civilizations | Done | `civilization` (`59369d5`); `view-civilization` (`0d70f5f`), registered (`43573bf`). |
| — | Model registry and CLI | Done | `model-registry` and `simulation-cli` (`4cbb771`): `emergence-cli` list, info, run, compare, validate, bench and convert; `validate` passes on the 17 default scenarios and on all 119 built-in scenarios in the registry tests. |
| — | Documentation | Done | README with screenshots of every laboratory (`docs/screenshots/`, `scripts/capture_screenshots.sh`), `CONTRIBUTING.md`, `CHANGELOG.md`, and `docs/INSTALLATION.md`, `USER_GUIDE.md`, `SIMULATION_MODELS.md`, `ASSET_LICENSES.md`, `TESTING.md`, `PERFORMANCE.md` and this roadmap, plus the architecture, research and model docs. |
| — | Performance | Done | `docs/PERFORMANCE.md`: headless throughput of 23 scenarios (`scripts/bench_scenarios.sh`) and ecosystem scaling to 100,000 animals, measured on an Apple M2 Max; Criterion benches in ten crates (`scripts/run_benchmarks.sh`). |

## 2. Known limitations and next steps

These are summarised from the "approximations and limitations" and "open
items" sections of the docs. The linked documents have the details.

### Application and release

* **Register the Ecosystem Simulator view** in `apps/desktop`, then commit
  `view-ecosystem`, `model-registry` and `simulation-cli`.
* **Platforms.** The app has only been built and run on macOS (Apple
  silicon, Metal). Linux and Windows builds are untested.
* **Licensing before a public release** (`docs/ASSET_LICENSES.md`):
  * decide how to ship the GPL-2.0-or-later pattern data: keep it with a
    notice, or import it from a user's Golly folder;
  * re-download the Poly Pizza models manually;
  * replace the two CC BY 3.0 Sherkiz models with CC0 ones if possible;
  * produce a third-party notice bundle;
  * add the proposed species `DATA_SOURCES.md`.
* **Shell** (`docs/UI_SHELL.md` §5):
  * Screenshots include the user interface.
  * PNG sequences save one image per frame, not per tick, and slow the frame
    rate while recording.
  * Plots keep 100 000 raw samples per metric, then a summary.
  * Loading a large save briefly pauses the window.
  * A timeline jump can land on the neighbouring snapshot when the history
    is full.
  * A very large *Step N* keeps the runner busy until it finishes.
* **CLI** (its README): it can set parameters but cannot send model commands.
  `validate` checks default scenarios without overrides.

### Life engines, Hashlife and patterns

* **Rules not supported:**
  * isotropic non-totalistic (Hensel), hexagonal, von Neumann and MAP rules;
  * Generations is Moore-only, with at most 255 states;
  * B0 only on finite worlds, with no emulation;
  * no Golly shifted tori, tubes or spheres.
* No infinite-plane engine for Generations or Wireworld. Ants never
  annihilate or split.
* **Hashlife:**
  * live cells must stay within ±2^61;
  * the node limit is soft;
  * chaotic patterns are tens of times slower than SWAR;
  * macrocell I/O handles two-state files only.
* Engine switches and tracking are refused above 2^26 live cells. Some
  Hashlife view texels are estimates.
* **Pattern I/O:**
  * at most 2^25 live cells per file;
  * no macrocell (outside `hashlife`), MCell, compressed files or `.rle3`;
  * bounded-grid patterns keep cells that lie outside the grid.

### Conway within Conway

* Macro states are defined only at readouts. Cores can be half-switched in
  part of each cycle.
* The tiles must be present as GPL asset files.
* Grids are full rectangles of at most 256 × 256 tiles. Edits rebuild from
  generation 0.
* Open items (`docs/OTCA_METAPIXEL.md` §9):
  * the internal-mechanism narrative is unverified;
  * the exact start of the visible transition outside the core has not been
    isolated;
  * inserting tiles into a running construction is untested;
  * long-run Hashlife memory growth on large chaotic grids has not been
    measured.

### Continuous Life

* Periodic boundaries only.
* `f32` fields; not bit-identical across CPU architectures.
* Lenia options missing: quantisation, soft clip, alternative integrators,
  the asymptotic update, and 3-D/4-D worlds.
* SmoothLife modes 3–4 are missing.
* Flow-Lenia has no parameter localisation and no food or decay.
* The automatic choice between FFT and direct convolution uses a single
  calibrated constant.

### 3D Life

* Only Moore-26 and von Neumann-6 neighbourhoods. No B0, Busy Boxes or RLE3.
* Dense worlds up to 256³. The unbounded engine halts above 32 768 chunks.
* Small tori count a neighbour once per offset.

### Environment and ecosystem

* **Compressed calendar.** A 120-day year with 6-minute ticks. Reproduction
  per day is about three times the real rate.
* **Physics-world simplifications:**
  * momentum-free water routing;
  * groundwater as one lumped reservoir;
  * empirical evaporation;
  * static precipitation pattern;
  * simplified biomes;
  * fire without spotting or crown fire;
  * geometric disasters;
  * one nutrient pool.
* **Ecosystem simplifications:**
  * insects, spiders and worms are density fields;
  * colonies are super-agents;
  * disease is SIR only;
  * packs only grow;
  * migration is driven by temperature only;
  * simplified water memory, cost of transport, fire escape and pollination;
  * weak trophic cascades in grass-dominated scenarios;
  * one nutrient stands for N and P.
* Nutrient balance drift of about 3·10⁻⁷ per day comes from `f32` soil grids.

### Custom Life Creator

* First-order kinematics, no volume exclusion.
* One interaction per entity per tick. Contact is tested on the frozen frame.
* Rhai is slow, about 1 µs per call. Budget errors have no source position.
* Sensing radii must be literals, and there is no line of sight.
* No infinite worlds.

### Evolution Lab

* Abstract units.
* No ontogenetic growth.
* Hermaphrodites without diploidy.
* Simple predation.
* Walls only.
* At most 100 000 organisms.
* Speciation uses a two-cluster criterion only.
* Selection on speed is weak unless predators are matched to the prey.
* Small sexual populations suffer an Allee effect.

### Emergent Civilizations

* Near-perfect information.
* Uncalibrated utility weights.
* No value of leisure: food piles up.
* Bilateral barter only: no money or credit.
* Euclidean territory.
* No pregnancy, disease, seasons or weather.
* Single-contest fights.

### Foundations

* **`organism-engine`:**
  * fixed-step Euler integration;
  * passability is checked at the destination only;
  * reactive, sampled obstacle avoidance;
  * greedy (not maximum) matching;
  * real-valued, linear genomes without diploidy.
* **`analytics` and `persistence`:**
  * power-of-two display buckets;
  * capping raw samples drops data permanently;
  * the period estimator assumes stationary data;
  * binary snapshots cannot be migrated;
  * CSV has no formula-injection protection;
  * atomic writes are atomic within one file system only.
* **Floating point.** Every floating-point model is reproducible only on the
  same platform and build.
