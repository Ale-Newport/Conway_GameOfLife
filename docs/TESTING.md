# Testing

The testing policy is in `docs/ARCHITECTURE.md` §3.6. In short:

* Every model has tests that check its **rules**, not just that the code runs.
  Examples: exact transitions, pattern periods and displacements,
  conservation laws, deterministic replay, error handling.
* Long exact verifications are `#[ignore]`d.
* Benchmarks use Criterion.

This page lists what exists and how to run it. It reports counts and
commands. It does not report results: no test or benchmark was run while
this page was written.

## 1. Everyday commands

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # all non-ignored tests
cargo test -p <crate>                        # one crate
cargo test -p <crate> --test <file>          # one integration-test file
```

## 2. Tests per crate

The counts below are the number of `#[test]` attributes in each crate (`src/`,
`tests/`, `examples/`). They were counted with `grep` on 2026-11-10 in the
working tree. Doctests are not counted.

| Crate | `#[test]` | of which `#[ignore]` | Integration-test files (`tests/`) |
|---|---:|---:|---|
| `simulation-core` | 40 | 0 | `live_workers.rs` |
| `patterns` | 111 | 4 | `asset_catalog.rs`, `builtin_dynamics.rs`, `golly_files.rs`, `properties.rs` |
| `hashlife` | 70 | 3 | `engine/` (`agreement.rs`, `fuzz.rs`, `golly.rs`, `io.rs`, `memory.rs`, `patterns.rs`, `simulation.rs`) |
| `cellular-automata` | 138 | 2 | `ant.rs`, `catalog.rs`, `engines_agree.rs`, `generations.rs`, `life_rules.rs`, `life_sim.rs`, `listing_limits.rs`, `scenarios.rs`, `wireworld.rs`, `wolfram.rs` |
| `metapixel` | 64 | 4 | `construction.rs`, `exact.rs`, `sim.rs`, `tiles.rs` |
| `continuous` | 96 | 5 | `lenia.rs`, `sim.rs`, `smoothlife_flow.rs`, `symmetry.rs`, `verification.rs` |
| `voxel-life` | 67 | 0 | `behaviour.rs`, `engine.rs`, `gliders.rs`, `presets.rs`, `vectors.rs`, `views.rs` |
| `physics-world` | 80 | 0 | `disasters.rs`, `fire.rs`, `generation.rs`, `hydrology.rs`, `simulation.rs` |
| `organism-engine` | 83 | 0 | `predator_prey.rs` |
| `ecosystem` | 129 | 6 | `aggregate.rs`, `api.rs`, `aquatic.rs`, `behaviour.rs`, `conservation.rs`, `determinism.rs`, `disease.rs`, `locomotion.rs`, `netlogo.rs`, `scenarios.rs`, `seasonal.rs`, `social.rs` |
| `rule-engine` | 95 | 0 | `presets.rs`, `sandbox.rs`, `simulation.rs`, `validation.rs` |
| `evolution` | 63 | 6 | `drift.rs`, `heredity.rs`, `presets.rs`, `replay.rs`, `selection.rs` |
| `civilization` | 65 | 0 | `behaviour.rs`, `interface.rs` |
| `analytics` | 90 | 0 | (unit tests only) |
| `persistence` | 33 | 0 | (unit tests only) |
| `model-registry` | 18 | 0 | `registry.rs` |
| `simulation-cli` | 18 | 0 | `cli.rs` |
| `rendering` | 105 | 0 | (unit tests only) |
| `organism-render` | 103 | 0 | `manifest_assets.rs` |
| `interface` | 84 | 0 | (unit tests only) |
| `view-conway` | 62 | 0 | — |
| `view-ecosystem` | 55 | 0 | — |
| `view-custom-life` | 77 | 0 | — |
| `view-metapixel` | 57 | 0 | — |
| `view-ca-lab` | 61 | 0 | — |
| `view-evolution` | 46 | 0 | — |
| `view-continuous` | 42 | 0 | — |
| `view-voxel` | 62 | 0 | — |
| `view-civilization` | 60 | 0 | — |
| `apps/desktop` (`emergence-lab`) | 6 | 0 | — (CLI parsing and asset-folder resolution) |

The root folders `tests/`, `benches/` and `examples/` are empty.

## 3. Long-running (ignored) suites

Each crate's documentation gives the command below. The `verify` profile
inherits `release` and adds `debug-assertions` and `overflow-checks`. The
generic form `cargo test --profile verify -p <crate> -- --ignored` should
also work for every suite. It has not been run here, and it is slower than
`--release` because of the extra assertions.

| Crate | Ignored tests | Command (from the crate docs) | Needs |
|---|---|---|---|
| `metapixel` | `exact::every_rule_bit_is_programmed_where_the_spec_says` (rule-bit matrix: 36 exact runs, one per rule digit and table), `exact::golly_parity_demo_follows_the_von_neumann_parity_rule`, `exact::golly_p216_demo_follows_its_rule_with_missing_tiles_never_born`, `exact::transition_window_timing_matches_the_research_note` | `cargo test -p metapixel --profile verify -- --ignored` | The OTCA tiles and Golly demos in `assets/patterns/otca/` |
| `hashlife` | `golly_files_parse_round_trip_and_evolve_like_the_naive_stepper`, `golly_gotts_dots_matches_its_documented_populations`, `golly_metapixel_galaxy_meta_grid_follows_life` | `GOLLY_PATTERNS=/path/to/golly/Patterns cargo test -p hashlife --profile verify -- --ignored golly` | A local Golly `Patterns/` folder (GPL, not committed) |
| `cellular-automata` | `breeder_population_at_generation_10_to_the_12`, `sawtooth_returns_to_177_cells_at_its_second_low` (catalog long-horizon checks, `tests/catalog.rs`) | `cargo test -p cellular-automata -- --ignored` | The asset catalog |
| `patterns` | 4 tests in `tests/golly_files.rs` (parse and round-trip every Golly pattern, Life folder rules, bounded grids, Generations rules) | `GOLLY_DIR=/path/to/golly cargo test -p patterns --test golly_files -- --ignored` | A local Golly checkout |
| `continuous` | `orbium_lives_5000_steps`, `animals_json_presets_long_windows`, `multi_kernel_presets_long_windows`, `smoothlife_glider_1000_steps`, `flow_lenia_conserves_mass_over_1000_steps` | `cargo test -p continuous --release --test verification -- --ignored --nocapture` | — |
| `ecosystem` | `forest_fire_burns_then_vegetation_recovers`, `drought_reduces_plants_and_herbivores`, `removing_the_apex_predator_releases_herbivores_then_vegetation_declines`, `an_invasive_species_changes_the_native_populations`, `the_balanced_meadow_keeps_its_vertebrates_for_a_year`, `no_scenario_loses_a_founding_species_within_ten_days` | `cargo test --profile verify -p ecosystem --test scenarios -- --ignored` | — |
| `evolution` | Six preset calibration runs in `tests/presets.rs` (speed selection, thermal adaptation, arms race, food scarcity, island evolution with the speciation criterion, r vs K) | `cargo test --release -p evolution --test presets -- --ignored` | — |

The results recorded so far are in each crate's documentation. Durations are
quoted from the docs, not re-measured:

| Suite | Source of the recorded results | Recorded duration |
|---|---|---|
| `metapixel` | `docs/OTCA_METAPIXEL.md` §7 and `docs/models/metapixel.md` §9 | Ignored binary finishes in about 4.1 s (run in parallel) |
| `hashlife` | `docs/HASHLIFE.md` §7 and `docs/models/hashlife.md` §8 | — |
| `cellular-automata` | `docs/models/cellular-automata.md` §10 | — |
| `continuous` | `docs/models/continuous.md` | — |
| `ecosystem` | `docs/ECOSYSTEM.md` §19.3 and §22 | — |
| `evolution` | `docs/models/evolution.md` | Preset checks about 25 s in release |

## 4. Determinism and snapshot tests

Every model has replay tests. They check that the same seed gives the same
state, and that a restored snapshot (which includes the RNG) continues
identically. Examples:

| Area | Where |
|---|---|
| Life engines | `crates/cellular-automata/tests/engines_agree.rs`: dense, bit, sparse and Hashlife against a naive reference; snapshot tests in `life_sim.rs`, `generations.rs`, `wireworld.rs`, `ant.rs`, `wolfram.rs` |
| Hashlife | `crates/hashlife/tests/engine/simulation.rs` (snapshots); GC invariance in `memory.rs` |
| Metapixel | `crates/metapixel/tests/sim.rs`: snapshot restore |
| Continuous | `crates/continuous/tests/sim.rs` (`deterministic_replay_and_thread_count_independence`); `symmetry.rs` (rotations, mirrors, translations) |
| Voxel | `crates/voxel-life/tests/behaviour.rs` (snapshot replay, including seedless Randomize per `docs/models/voxel-life.md` §8) |
| Ecosystem | `crates/ecosystem/tests/determinism.rs`, plus the conservation tests in `conservation.rs` |
| Evolution | `crates/evolution/tests/replay.rs` |
| Civilization | `crates/civilization/tests/behaviour.rs` (determinism and snapshot replay) |
| Agent infrastructure | `crates/organism-engine/tests/predator_prey.rs`: bit-identical replay after a postcard round trip |
| Runner | `crates/simulation-core/tests/live_workers.rs`: worker threads end when the runner is dropped |
| Persistence | `persistence` unit tests: envelope and version checks, CRC, migrations, atomic writes |

## 5. View smoke tests

Each laboratory view has a standalone example. The example opens the shell
with only that mode registered and can capture the window:

```text
cargo run -p view-conway       --example conway_standalone
cargo run -p view-ecosystem    --example ecosystem_standalone
cargo run -p view-custom-life  --example custom_life_standalone
cargo run -p view-metapixel    --example metapixel_standalone
cargo run -p view-ca-lab       --example ca_lab_standalone
cargo run -p view-evolution    --example evolution_standalone
cargo run -p view-continuous   --example continuous_standalone
cargo run -p view-voxel        --example voxel_standalone
cargo run -p view-civilization --example civilization_standalone
```

Captures are controlled by environment variables. Each example's header
comment documents the full set.

| Variable | Purpose |
|---|---|
| `VIEW_SMOKE_SCREENSHOT=<png>` | Enables the capture. The program exits after saving. |
| `VIEW_SMOKE_FRAMES=<n>` | Frames before the capture. The default varies (60–120) by example. |
| `VIEW_SMOKE_SCENARIO=<id>` | Scenario to open. |
| `VIEW_SMOKE_PLAY`, `VIEW_SMOKE_STEPS`, `VIEW_SMOKE_SPEED` | Run the simulation first. |
| `VIEW_SMOKE_TAB=<name>` | Right-panel tab, or a shell window. |
| `VIEW_SMOKE_THEME=<light\|contrast>` | Theme. |
| `VIEW_SMOKE_TOOL`, `VIEW_SMOKE_CURSOR`, `VIEW_SMOKE_DRAG`, `VIEW_SMOKE_CLICK`, `VIEW_SMOKE_EDIT` | Exercise tools and picking. |
| `VIEW_SMOKE_ACTION` / `VIEW_SMOKE_ACTIONS` | Run tutorial actions. |
| `VIEW_SMOKE_REENTER` | Leave to the dashboard and re-enter (lifecycle check). |

Some examples check more than the screenshot. For example,
`voxel_standalone` fails if more than one simulation thread is alive
(`simulation_core::live_workers`).

The shell has its own reference mode:

```text
SHELL_DEMO_VIEW=metrics SHELL_DEMO_SCREENSHOT=out.png cargo run -p interface --example shell_demo
```

### Desktop smoke screenshots

The desktop binary renders N frames, saves the window and exits with status
0:

```text
cargo run -p emergence-lab --release -- --mode <slug> --smoke-screenshot out.png --smoke-frames 180
```

It exits non-zero if the file was not written, or after 600 extra frames.
`scripts/capture_screenshots.sh` runs this for the dashboard and every mode
slug, writing `docs/screenshots/<slug>.png`.

`ecosystem` currently shows the dashboard, because `view-ecosystem` is not
registered in `apps/desktop`.

## 6. Benchmarks

Criterion benchmarks exist in these crates:

| Crate | Bench file | Benchmarks named in the docs |
|---|---|---|
| `cellular-automata` | `benches/engines.rs` | `finite_soup_generation/{dense,bit}/{512,2048}`, `sparse_far_apart/generation`, `hashlife_gosper_gun/advance_2^20_cold` |
| `hashlife` | `benches/hashlife.rs` | `gosper_gun_2^20`, `soup_256x256_step_2^10`, `soup_256x256_1024_single_generations`, `metapixel_grid_empty_expansion` |
| `metapixel` | `benches/metapixel.rs` | `program_tiles_b3s23`, `build_64x64_random`, `blinker_3x3_one_metagen_cold`, `readout_galaxy_15x15`, `micro_view_galaxy_1024`, `accelerated_step_256x256` |
| `continuous` | `benches/continuous.rs` | `lenia_step` (128²–1024²), `tessellatium_step`, `smoothlife_step`, `flow_lenia_step`, FFT vs direct |
| `voxel-life` | `benches/voxel.rs` | `voxel_step/*`, `voxel_view/*` |
| `physics-world` | `benches/world.rs` | `world_step_256x256`, `world_step_256x256_storm`, `world_step_256x256_fire_front`, `generation/generate_256x256` |
| `ecosystem` | `benches/ecosystem.rs` | `meadow_tick`, `meadow/meadow_day`, `wolf_sheep_grass_tick`, `agents/{1000,10000,100000}`, `fields/{100k,10M}` |
| `rule-engine` | `benches/custom_life.rs` | `step/*` presets, `compile/all_presets`, `dsl/parse_print` |
| `evolution` | `benches/evolution.rs` | `speed_selection_100_ticks`, `island_evolution_100_ticks`, `view_with_genealogy` |
| `civilization` | `benches/civilization.rs` | `step/abundant`, `step/trade_routes`, `command/add_resources_r30_96x96`, `build/territorial_expansion` |

Run one crate with `cargo bench -p <crate>`. To run all of them, use:

```text
scripts/run_benchmarks.sh
```

The script runs `cargo bench` for every crate that has a `benches/` folder.
It writes each crate's output to `target/bench-logs/<crate>.log`.

* Measured results, with the hardware and machine load, are in each model's
  documentation.
* Several of those measurements were taken on a heavily loaded machine. The
  docs say so.
* The CLI's `bench` command (`emergence-cli bench`) times single `step()`
  calls through the registry; `scripts/bench_scenarios.sh` runs it over 23
  scenarios and `docs/PERFORMANCE.md` reports the results.
