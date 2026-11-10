# Contributing to Emergence Lab

Emergence Lab is a Rust workspace (edition 2024, `rust-version = "1.95"`).
The desktop application uses Bevy 0.20.1 and bevy_egui 0.43.1. The workspace
contract is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), and the contract
between the shell and the laboratory views is in
[`docs/UI_SHELL.md`](docs/UI_SHELL.md). Read both before you change a crate
boundary or add a mode.

## 1. Build, test and lint

```text
cargo fmt --all                                          # rustfmt.toml: max_width = 120
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p emergence-lab                   # desktop binary `emergence-lab`
```

Workspace lints (root `Cargo.toml`): `unsafe_code = "deny"`,
`unused_must_use = "deny"`, and `clippy::all` at warn. With `-D warnings`,
every clippy warning is an error.

Build profiles (root `Cargo.toml`):

| Profile | Use |
|---|---|
| `dev` | `opt-level = 1` for workspace crates, `opt-level = 3` for all dependencies |
| `release` | `codegen-units = 4`, `lto = "thin"` |
| `verify` | inherits `release`, adds `debug-assertions` and `overflow-checks`; for long exact verification suites |

Long-running tests are `#[ignore]`d. Run them per crate:

```text
cargo test --profile verify -p <crate> -- --ignored
```

Some suites need extra arguments or environment variables (a Golly checkout,
a specific `--test` target). [`docs/TESTING.md`](docs/TESTING.md) lists each
of them.

Benchmarks use Criterion: `cargo bench -p <crate>`, or
`scripts/run_benchmarks.sh` for every crate that has a `benches/` folder. If
you quote a result in a document, it must be a real measurement, and you must
state the hardware and the machine load.

## 2. Crate layout and ownership

The layers, from bottom to top:

| Layer | Crates |
|---|---|
| Shared contracts | `simulation-core` |
| Domain foundations | `patterns`, `physics-world`, `organism-engine`, `analytics`, `persistence` |
| Models | `cellular-automata`, `hashlife`, `metapixel`, `continuous`, `voxel-life`, `ecosystem`, `rule-engine`, `evolution`, `civilization` |
| Integration | `model-registry` |
| Presentation (Bevy) | `rendering`, `organism-render`, `interface`, `view-*` (one crate per mode) |
| Entry points | `apps/desktop` (binary `emergence-lab`), `crates/simulation-cli` (binary `emergence-cli`) |

Rules from `docs/ARCHITECTURE.md`:

* **Below the presentation layer, nothing depends on Bevy, egui or any
  renderer.** A model must run the same way in unit tests, in Criterion
  benchmarks, in the CLI and in the desktop app.
* Every model implements `simulation_core::Simulation`:
  * `step()` advances exactly one tick.
  * All external changes go through `apply(Command)` and are validated.
  * Parameters are `ParamSpec`s with a correct `ApplyPolicy` (`Immediate`,
    `NextTick` or `RequiresReset`).
  * `view()` returns owned render data.
  * `snapshot()` and `restore()` include the RNG and carry a
    `schema_version`.
* `ModelDescriptor::rules` is the formal Markdown statement of the rules the
  code executes. If you change the dynamics, update this text in the same
  commit.
* Renaming a public item listed in `docs/ARCHITECTURE.md` §4 means updating
  every crate that depends on it.
* User-facing input (rule strings, RLE, scripts, scenarios) is validated with
  precise messages. Bad input must never cause a panic.
* Each view crate owns its mode. The `interface` crate owns the shell: top
  bar, side panels, bottom bar, dashboard, theme, tutorials and shortcuts.
  Mode code adds panel contents but does not reimplement shell services.

## 3. Determinism rules

* All randomness comes from `SimRng` (xoshiro256++), stored in the model state.
  Independent subsystems use `SimRng::derive(seed, stream)`. No other random
  source is allowed in simulation code.
* Never let `HashMap` or `HashSet` iteration order affect simulation logic.
  Iterate in id order, over sorted keys or in fixed grid order, or use
  `BTreeMap`.
* Parallel code (rayon) may only compute pure functions of read-only state into
  separate output buffers. If two agents could make conflicting decisions,
  each decision becomes an intent or claim, resolved afterwards in a total
  order: priority, then distance, then id (see `organism_engine::intent`).
* The `Runner` never drops or changes ticks to keep up with rendering, and no
  simulation runs inside Bevy's frame schedule.
* Floating-point models only need to be reproducible on the same platform and
  build. Write this down in the model's documentation when it applies.
* Every model needs a deterministic-replay test: same seed, same result, and a
  restored snapshot continues identically.

## 4. Adding a model

1. Put the model in an existing model crate, or create a new crate under
   `crates/` that depends only on `simulation-core` and the foundation crates
   it needs. New crates are added to the workspace automatically by the
   `members = ["crates/*", "apps/*"]` glob. Add a matching
   `[workspace.dependencies]` entry.
2. Implement `Simulation`, following §3 of `docs/ARCHITECTURE.md`:
   * a stable descriptor `id`;
   * `ParamSpec`s with `description`, `group` and `ApplyPolicy`;
   * a description for every metric;
   * a `population` metric if the model has a natural population.
3. Put built-in scenarios in `scenarios/<mode>/*.ron`. Existing crates embed
   them with `include_str!` so the model works without the folder. **Never
   embed GPL data** (see §8).
4. Write tests that check the rules themselves, not just that the code runs.
   Examples: exact transitions, periods, conservation laws, error messages,
   deterministic replay.
5. Document the model in `docs/models/<crate>.md`. Cover:
   * the rules actually executed;
   * parameters, commands, metrics and views;
   * determinism;
   * approximations and limitations;
   * how it was verified;
   * measured performance;
   * references.

   Then add an entry to [`docs/SIMULATION_MODELS.md`](docs/SIMULATION_MODELS.md).
6. If the model should be usable from the CLI, register it in `model-registry`.
   Add its factory and scenario discovery next to the existing models.

## 5. Adding a view mode

The full contract is in `docs/UI_SHELL.md` §2. In short:

1. Create a `view-<name>` crate with a Bevy `Plugin`, and add it inside
   `register_modes` in `apps/desktop/src/main.rs`, between the
   `BEGIN/END MODE REGISTRATION` markers. Also add the dependency to
   `apps/desktop/Cargo.toml`.
2. Call `app.register_mode(ModeCard { .. })`. A mode can only be opened once it
   is registered.
3. On `OnEnter(mode)`, insert `ActiveSession(Arc<dyn SessionControl>)` around a
   `Runner<S>`. Remove it on `OnExit(mode)`.
4. Add panels with `add_mode_panel(mode, PanelSlot::{Left, Right, Toolbar,
   Overlay}, system)`. Each panel is a one-shot system whose first parameter is
   `InMut<egui::Ui>`.
5. Spawn cameras with `rendering::WorldCamera` and `DespawnOnExit(mode)`. Never
   set `Camera::viewport` yourself. Gate all world input on
   `Res<rendering::WorldInput>`, not on `EguiWantsInput`.
6. Keep the shell's keys free:
   * Space, N, R, Z, Y, `=`, `-`, Home, Tab, F1, F11, F12;
   * any key while Ctrl, Cmd or Alt is held (`rendering::shortcut_modifier_held`).
7. Fill the scenario picker through `ModeScenarios`, register a tutorial, and
   report fatal loading errors through `ModePlaceholders`.
8. Add a `<name>_standalone` example with a `VIEW_SMOKE_SCREENSHOT` capture
   (see `docs/TESTING.md`). Add the mode slug to
   `scripts/capture_screenshots.sh` if it is new.

The shell is the only consumer of `drain_metrics()` and `take_errors()`. Use
`Runner::query` for read-only access to the model; it never blocks a frame.

## 6. Documentation

* Every claim in `docs/` must be checkable in the repository. Mark anything
  not confirmed as **UNVERIFIED**.
* Model documents follow the section order of the existing
  `docs/models/*.md`.
* Do not copy text from GPL, NC or unlicensed sources (see
  `docs/RESEARCH.md` §8).

## 7. Commit style

Look at `git log --oneline` for examples:

* Keep commits small and focused. One logical change per commit.
* Write the subject as a descriptive sentence in the imperative, with no
  prefix tags and no trailing period. For example: "Count live cells in a
  rectangle before listing them", "Add metapixel crate: OTCA metapixel running
  under B3/S23".
* Do not add `Co-authored-by` lines.
* Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and the tests of
  the crates you touched before committing.

## 8. Asset and data licence rules

Details are in `docs/ASSET_LICENSES.md` and `docs/RESEARCH.md` §8.2.

* Project code is `MIT OR Apache-2.0`. Link only permissive dependencies.
* Read GPL, NC and unlicensed code for understanding only. Implement
  clean-room from papers and specifications.
* Third-party **data** keeps its own licence, provenance header and
  attribution next to the files (`PROVENANCE.md`, `LICENSE.txt`,
  `ATTRIBUTION.txt`, `license` / `license_note` in
  `assets/patterns/catalog.toml`).
* GPL-2.0-or-later data must stay in separate files under `assets/`, read at
  run time. It must never be embedded with `include_str!` or `include_bytes!`.
  This covers the OTCA tiles and Golly demos, the 9 Golly-derived catalog
  patterns and `wireworld/and-gate-mcintosh.rle`.
* 3D models: prefer CC0. CC-BY models are accepted only in their own folder,
  with an `ATTRIBUTION.txt` that the About window shows. NC, ND and unclear
  licences are excluded. Before adding a Quaternius pack, check that its own
  page says CC0, not the Quaternius Asset License.
* Record every new 3D model in `assets/models/manifest.toml`, with its source,
  licence, author and retrieval date.
