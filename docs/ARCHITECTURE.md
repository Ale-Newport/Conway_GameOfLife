# Emergence Lab — Architecture

This document is the engineering contract for the workspace. It describes the
layers, the responsibilities of every crate, the shared conventions that keep
simulations deterministic and renderer-independent, and the minimum public API
each crate offers to its dependents. Detailed model documentation lives in
`docs/SIMULATION_MODELS.md`, `docs/HASHLIFE.md`, `docs/OTCA_METAPIXEL.md`,
`docs/ECOSYSTEM.md` and `docs/RULE_ENGINE.md`; research notes and sources live
in `docs/RESEARCH.md` and `docs/research/`.

## 1. Layers

```text
┌──────────────────────────────────────────────────────────────────────┐
│ apps/desktop (binary `emergence-lab`)      crates/simulation-cli      │  entry points
├──────────────────────────────────────────────────────────────────────┤
│ view-* crates (one per mode)  interface (egui shell)  rendering        │  presentation (Bevy)
├──────────────────────────────────────────────────────────────────────┤
│ model-registry (catalogue of models + scenarios, factories)           │  integration
├──────────────────────────────────────────────────────────────────────┤
│ cellular-automata  hashlife  metapixel  continuous  voxel-life         │
│ ecosystem  rule-engine  evolution  civilization                        │  models
├──────────────────────────────────────────────────────────────────────┤
│ patterns  physics-world  organism-engine  analytics  persistence       │  domain foundations
├──────────────────────────────────────────────────────────────────────┤
│ simulation-core                                                        │  shared contracts
└──────────────────────────────────────────────────────────────────────┘
```

**Rule:** nothing below the presentation layer depends on Bevy, egui or any
renderer. Every model can run in unit tests, Criterion benchmarks and the CLI
exactly as it runs in the desktop application.

## 2. Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `simulation-core` | `Simulation`/`DynSimulation` traits, `SimRng`, parameter schemas + apply policies, metrics, snapshots, history/undo, threaded `Runner`, `Grid2`, `Topology`, `SpatialGrid`, `EventLog` | serde, postcard, glam |
| `patterns` | Life-like / Generations rule notation, `Pattern` model, RLE / plaintext / Life 1.06 readers & writers, transforms, catalogue loader for `assets/patterns/catalog.toml` | core |
| `hashlife` | Hash-consed quadtree with memoised results for any two-state outer-totalistic rule without B0; arbitrary and power-of-two steps; level-of-detail queries; macrocell I/O; GC | core |
| `cellular-automata` | Engines: dense `u8`, bit-packed SWAR, sparse chunked, Hashlife adapter, automatic engine selection; models: Classic Life / Life-like, Generations (incl. Brian's Brain), Wireworld, Langton's ant & turmites, Wolfram elementary CA; editing tools; random/symmetric initialisation | core, patterns, hashlife |
| `metapixel` | OTCA metapixel tiling, rule programming, exact B3/S23 execution through Hashlife, macro-state readout, and a separately labelled accelerated macro model | core, patterns, hashlife, cellular-automata |
| `continuous` | Lenia (kernels, growth functions, multi-kernel), SmoothLife, Flow-Lenia; FFT convolution | core, rustfft, rayon |
| `voxel-life` | 3D cellular automata (Moore-26 / von Neumann-6, survival/birth/states notation), dense & sparse storage, slicing and surface extraction | core, rayon |
| `physics-world` | Terrain/heightmap and biome generation, scalar fields (temperature, moisture, fertility, nutrients, light), calendar (day/night, seasons), weather, hydrology (rain, runoff, infiltration, evaporation, floods), fire spread, wind, disasters | core, noise, rayon |
| `organism-engine` | Generic agent store with stable ids, kinematics and steering (seek/flee/arrive/wander/boids/pursuit/evasion), perception queries, utility-AI framework, FSM helpers, deterministic intent resolution, genomes (genes, mutation, recombination, distance) and lineage | core |
| `ecosystem` | Species catalogue (data driven), animals with individual state and species-specific behaviour, vegetation layers and individual trees, fungi, nutrient/detritus cycle with conservation accounting, disease, social behaviour, interaction with fire/water/climate, 14 scenarios | core, physics-world, organism-engine |
| `rule-engine` | No-code rule DSL (conditions → actions with probability, priority, cooldown, costs), sandboxed Rhai scripts with operation/memory/depth limits, entity-type and relationship definitions, the custom-life simulation, presets | core, organism-engine, rhai |
| `evolution` | Population model with heritable genomes, mutation, recombination, selection emerging from survival, drift, geographic isolation, genealogy, gene-frequency statistics, experiments; its own lightweight environment (food patches, gradients, barriers) | core, organism-engine |
| `civilization` | Agents with needs, energy, skills, inventories, preferences, relationships and goals; gathering, production, barter trade, building, roads, territory, groups; experiments; its own resource-tile world | core, organism-engine |
| `analytics` | Time series with downsampling, summary statistics, histograms, diversity indices, Gini, survival curves, run records & comparison, CSV/JSON export | core |
| `persistence` | Versioned file envelopes, scenario/config files (RON/JSON/TOML), binary snapshot files, schema migrations, user-data locations | core |
| `model-registry` | List of all models, scenario discovery under `scenarios/`, factories building `Box<dyn DynSimulation>` from (model, scenario, overrides, seed) | all model crates |
| `simulation-cli` | `emergence-cli`: list, info, run, compare, validate, bench, convert patterns | registry, analytics, persistence |
| `rendering` (Bevy) | Grid texture renderer with styles, 2D/3D cameras, voxel instancing, procedural organism and plant meshes + glTF animation with LOD, particles | bevy |
| `interface` (Bevy + egui) | Theme, application shell, dashboard cards, panels, time controls, parameter editors generated from `ParamSpec`, plots, tutorials | bevy, bevy_egui, egui_plot |
| `view-*` (Bevy) | One crate per mode connecting a `Runner<S>` to rendering and panels | rendering, interface, model crate |
| `apps/desktop` | Binary wiring all plugins, application states, fullscreen, screenshots | all view crates |

## 3. Shared conventions

### 3.1 The `Simulation` contract (`simulation-core`)

* `step()` advances exactly one tick of fixed simulated duration
  (`ModelDescriptor::dt` in `tick_unit`s). Speed is chosen by the caller; the
  `Runner` runs ticks on a worker thread and never drops or alters ticks to
  keep up with rendering. If a model is slower than requested, the measured
  rate is shown instead.
* `tick()` reports generations. Engines that jump several generations per call
  (Hashlife) still report the true generation count.
* All external mutations go through `apply(Command)`. Commands are validated and
  return `SimError` with a user-readable message.
* `params()` / `set_param()` use the model's `ParamSpec`s. Every spec states its
  `ApplyPolicy`: `Immediate`, `NextTick` (default for rules) or
  `RequiresReset` (world size, topology...). `set_param` returns the policy so
  the UI can tell the user when the change takes effect.
* `view(&ViewRequest) -> View` returns owned render data for the requested
  region / level of detail. Renderers never borrow model internals.
* `snapshot()` / `restore()` capture the complete state **including the RNG**,
  so a restored run replays identically. Snapshots are versioned
  (`schema_version`) and carry the model id.
* `ModelDescriptor::rules` is a formal Markdown statement of the rules that are
  actually executed. It is displayed in the rule panel and must stay in sync
  with the code.

### 3.2 Determinism

* All randomness comes from `SimRng` (xoshiro256++) stored in the model state.
  Independent subsystems use `SimRng::derive(seed, stream)` so changing one
  subsystem does not perturb another.
* Never use `HashMap` iteration order to drive simulation logic. Iterate in id
  order, sorted keys or fixed grid order.
* Parallel code (rayon) only computes pure functions of read-only state into
  separate output buffers (double buffering). Any decision that could conflict
  (two predators attacking one prey, two agents eating the last biomass) is
  collected as an *intent* and resolved afterwards in a deterministic order
  (priority, then distance, then id).
* Floating point: f32/f64 with standard IEEE operations; results are
  reproducible on the same platform and build. Cross-platform bit-exactness is
  not promised for float-heavy models and this is documented.

### 3.3 Coordinates and units

* 2D cell grids: `x` grows to the right, `y` grows downward (same as RLE).
  Infinite worlds use `i64` coordinates.
* Continuous agent worlds: `glam::Vec2` positions in world units where one unit
  is one terrain cell; the renderer maps `(x, y)` to Bevy's `(x, 0, y)`.
* Ecosystem calendar (compressed, configurable): one tick = 6 simulated
  minutes, one day = 240 ticks, four seasons per year. Biological durations
  (lifespans, gestation) are expressed in days and converted by the model.
  This compression is an explicit modelling approximation.

### 3.4 Data locations

| Path | Content |
|---|---|
| `assets/patterns/` | Pattern library (`catalog.toml` + RLE files), OTCA tiles in `assets/patterns/otca/` |
| `assets/species/` | Species definitions (RON) for the ecosystem |
| `assets/models/` | 3D models with `manifest.toml` and per-source license files |
| `scenarios/<mode>/` | Scenario files (RON) per mode |
| `docs/` | Documentation; `docs/research/` holds detailed research notes |

### 3.5 Errors, validation and limits

* User-facing input (rule strings, RLE, scripts, scenarios) is validated with
  precise messages (line/column where possible). Nothing panics on bad input.
* Scripts run in a sandbox with operation, call-depth, expression-depth, string,
  array and map limits; a failing script disables the offending rule and
  reports the error instead of stopping the application.

### 3.6 Testing policy

* Every model has tests that check the *rules* (not just that code runs):
  exact Life transitions, pattern periods and displacements, conservation laws,
  energy transfer, deterministic replay, error handling.
* Long-running exact verifications (OTCA metacell transitions, large Hashlife
  runs) live in `#[ignore]` tests or dedicated integration suites run with
  `cargo test --profile verify -- --ignored`, and their results are recorded in
  `docs/TESTING.md`.
* Benchmarks use Criterion (`cargo bench -p <crate>`); results quoted in docs
  are real measurements with hardware specifications.

## 4. Minimum cross-crate APIs

The following public items are relied upon by other crates. Crates may expose
more; renaming any of these requires updating dependents.

### `patterns`
* `rule::LifeRule { birth: u16, survive: u16 }` (bit *n* = neighbour count *n*),
  `LifeRule::parse(&str)` accepting `B3/S23`, `b3s23`, `23/3`; `LifeRule::conway()`;
  `Display` in canonical `B3/S23` form; `has_b0()`.
* `rule::GenerationsRule { birth, survive, states }` parsing `B2/S/C3`
  (Golly) and `/2/3` (MCell `S/B/C`) forms.
* `rule::BoundedGrid` parsing Golly suffixes (`:T100,100`, `:P64,64`, `:K..`, `:C..`).
* `Pattern { name, description, author, rule: Option<String>, comments, cells: Vec<Cell> }`
  with `Cell { x: i64, y: i64, state: u8 }`, `bounding_box()`, `population()`,
  `normalized()`, `rotate_cw()`, `rotate_ccw()`, `flip_horizontal()`, `flip_vertical()`,
  `translated(dx, dy)`.
* `rle::parse(&str) -> Result<Pattern, PatternError>`, `rle::write(&Pattern) -> String`;
  `plaintext::{parse, write}`, `life106::{parse, write}`, `parse_any(text, file_name)`.
* `catalog::Catalog::load(path)`, `Catalog::entries()`, `CatalogEntry` metadata,
  `Catalog::load_pattern(&entry)`.

### `hashlife`
* `Universe::new(birth_mask: u16, survive_mask: u16) -> Result<Universe, HashlifeError>` (rejects B0).
* `set_cell(x: i64, y: i64, alive: bool)`, `get_cell`, `insert_cells(iter)`, `clear()`.
* `step_pow2(k: u32)` (advance 2^k generations), `advance(n: u64)` (arbitrary n),
  `generation() -> u64`, `population() -> u64` (cached).
* `bounding_box() -> Option<(i64, i64, i64, i64)>`, `cells_in_rect(x0, y0, x1, y1)`,
  `density_grid(x0, y0, x1, y1, cell_px_log2) -> Grid of populations` for LOD rendering.
* `macrocell::read(&str) -> Result<Universe, _>`, `macrocell::write(&Universe) -> String`.
* `node_count()`, `memory_bytes()`, `garbage_collect()`.

### `organism-engine`
* `AgentId(u64)`, `AgentStore<T>` (stable ids, id-ordered iteration, deferred spawn/despawn).
* `Kinematics { position: Vec2, velocity: Vec2, heading: f32 }`, steering helpers.
* `utility::{Consideration, Curve, ActionScore, choose_best}`.
* `intent::{Claim, resolve_claims}` deterministic conflict resolution.
* `genome::{GeneSpec, Genome, mutate, crossover, genetic_distance}`, `Lineage`.

### `physics-world`
* `WorldConfig`, `World::generate(&WorldConfig) -> World`, `World::step(&mut self) -> EnvironmentReport`.
* Per-cell `Grid2<f32>` layers: `elevation`, `water_depth`, `soil_moisture`,
  `temperature`, `fertility`, `nutrients`, `light`; `biome: Grid2<Biome>`; `fuel` (written by
  the ecosystem each tick) and `burning`/`burned` fire state.
* `Calendar` (tick → hour/day/season/year, daylight factor), `Weather` (rain, wind vector, storms).
* Interventions: `ignite`, `add_rain`, `start_drought`, `trigger(Disaster, ...)`.

## 5. Presentation architecture (Bevy)

* Each mode is a Bevy `Plugin` living in its own `view-*` crate. It owns a
  `Runner<S>` resource, sends `Control` messages from UI input, and reads the
  latest published view each frame. Simulation never runs inside Bevy's frame
  schedule, so slow frames cannot change results.
* The `interface` crate owns the application shell (top bar, bottom time
  controls and metrics, collapsible/resizable side panels, dashboard, theme,
  tutorials, keyboard shortcuts); mode plugins contribute panel contents.
* The `rendering` crate maps views to GPU resources: one dynamic texture for 2D
  grids (only the visible region is requested, with level-of-detail density for
  zoomed-out Hashlife worlds), instanced meshes for voxels and organisms, LOD
  switching by camera distance that never feeds back into the simulation.

## 6. Performance strategy

* Dense Life: bit-packed SWAR rows (64 cells per `u64`), rayon over row bands.
* Sparse Life: 64×64 bit-packed tiles in a hash map, only active tiles stepped.
* Hashlife for highly regular or huge-time-scale patterns (guns, metapixels);
  automatic engine choice uses population, bounding box and requested step size.
* Agents: spatial grid rebuilt per tick, intents resolved serially in id order,
  perception and scoring parallelised where pure.
* Continuous fields: FFT convolution with precomputed kernel spectra.
* Rendering: draw only visible cells, instancing, LOD, decoupled ticks/frames.
