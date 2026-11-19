# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The workspace version lives in the root `Cargo.toml`.

## [Unreleased]

## [0.1.0] - 2026-11-19

First public release. The entries below summarise the commit history from
2026-11-09 to 2026-11-19 (`git log --oneline`).

### Added

- **Workspace and core** (`simulation-core`):
  - `Simulation` and `DynSimulation` traits;
  - seeded `SimRng`;
  - parameter specs with apply policies;
  - metrics, snapshots, history and undo;
  - the threaded `Runner`;
  - grid, topology and spatial helpers;
  - the type-erased `SessionControl` for generic UI controls.
- **Research and data:**
  - research notes (`docs/RESEARCH.md`, `docs/research/`);
  - the verified 80-pattern B3/S23 library (`assets/patterns/`);
  - a CC0 / CC-BY 3D model library with a manifest (`assets/models/`).
- **Architecture document** with cross-crate contracts (`docs/ARCHITECTURE.md`).
- **`patterns`:**
  - Life-like, Generations and bounded-grid rule notation;
  - the pattern model;
  - RLE, plaintext and Life 1.05 / 1.06 readers and writers;
  - the catalog loader.
- **`hashlife`:**
  - Hashlife engine for two-state rules without B0;
  - macrocell I/O;
  - level-of-detail queries;
  - garbage collection.
- **`organism-engine`:**
  - agent store;
  - kinematics and steering;
  - perception;
  - utility AI;
  - deterministic intent resolution;
  - genomes and lineage.
- **`physics-world`:**
  - terrain, climate and biomes;
  - weather and hydrology;
  - fire spread and disasters.
- **`analytics` and `persistence`:**
  - time series, statistics, diversity indices, survival curves and run
    comparison;
  - versioned files, `.elsnap` binary snapshots, migrations and user-data
    folders.
- **`continuous`:** Lenia, Expanded Lenia, SmoothLife and Flow-Lenia with FFT
  and direct convolution.
- **`voxel-life`:** three-dimensional outer-totalistic cellular automata (Moore-26 /
  von Neumann-6), with dense and sparse engines.
- **`evolution`:** heritable genomes with selection that emerges from survival
  and reproduction, drift, isolation and a speciation criterion.
- **`civilization`:** agents with needs, production, barter, building, roads,
  groups and territory.
- **`rendering`:**
  - styled grid renderer;
  - 2D and 3D cameras;
  - voxels, terrain, water and particles.
- **`organism-render`:** animated glTF and procedural organisms.
- **`interface` and `apps/desktop`:**
  - the application shell (dashboard, top bar, panels, time controls,
    save/load, exports, screenshots, themes, tutorials, shortcuts);
  - the `emergence-lab` binary with `--mode` and `--smoke-screenshot`.
- **`rule-engine`:**
  - custom-life DSL;
  - sandboxed Rhai scripts;
  - presets.
- **`cellular-automata`:**
  - Life engines (dense, bit-packed, sparse, Hashlife adapter, automatic
    selection);
  - Generations, Wireworld, Langton's ant and turmites, and Wolfram
    elementary CA models.
- **`metapixel`:** OTCA metapixel tilings run exactly under B3/S23 through
  Hashlife, with an accelerated macro model kept separately.
- **View modes** registered in the desktop app:
  - Continuous Life;
  - 3D Life;
  - Evolution Lab;
  - Emergent Civilizations;
  - Custom Life Creator;
  - Classic Conway;
  - Cellular Automata Lab;
  - Conway within Conway.
- **`ecosystem`:**
  - individual-based ecosystem simulator: 41 animal species, vegetation,
    trees and fungi;
  - nutrient and energy accounting;
  - disease;
  - 14 scenarios.
- **Runner** publishes live model descriptors and fresh metric samples after
  state-changing controls. It refreshes them after `NextTick` parameter
  changes, and adds read-only `Runner::query`.

### Changed

- Custom-life parameter sliders use logarithmic scales for wide ranges and
  keep the designed values as defaults.
- Selections and selected text use the accent colour in every theme, with at
  least 4.5:1 contrast for text.
- Camera keys ignore Ctrl / Cmd / Alt chords so they do not clash with shell
  shortcuts. Age colouring is logarithmic, and the age legends match the
  shader.
- Metapixel tick descriptions now state accurately what a tick does: it pauses
  at each multiple of the 35,328-generation period to read the tiles, then
  completes the step.

### Fixed

- CSV exports (`analytics`) neutralise spreadsheet formula injection: text
  fields starting with `=`, `+`, `-`, `@`, a tab or a carriage return (and not
  a number) get a leading `'`.
- Parameter sliders no longer send values that the user did not edit.
- The grid material is rebuilt when its texture is reallocated.
- Every Life engine counts the live cells of a rectangle exactly before
  listing them. Region tools, the tracker and engine switches now refuse
  oversized work before allocating memory. Engine switches above 2^26 live
  cells are refused with a clear error.

### Added later in this release

- `model-registry` (catalogue of models and scenarios with factories).
- `simulation-cli` (`emergence-cli`: list, info, run, compare, validate, bench,
  convert).
- `view-ecosystem`: the Ecosystem Simulator view, registered in the desktop
  app; all nine laboratories are now available.
- Project documentation: this changelog, `CONTRIBUTING.md`, the guides in
  `docs/` (including `PERFORMANCE.md` with measured benchmarks) and README
  screenshots of every laboratory.
- **Web demo** (`apps/web`, crate `emergence-web`): the Classic Conway engines
  and the embedded pattern library compiled to WebAssembly, with a canvas
  front end (rules, patterns, drawing, live engine readout), deployed to
  GitHub Pages. `scripts/build_web.sh` builds it locally.
- `emergence-lab --play` opens laboratories with their simulation running.
- `emergence-lab --smoke-sequence <N> --smoke-every <K>` saves a numbered
  series of window screenshots; `scripts/record_demo.sh` uses it to record the
  README animation (`docs/demo.gif`) from the real app.
- **Continuous integration**: rustfmt, clippy (`-D warnings`), tests on
  Linux, macOS and Windows, a headless `emergence-cli validate` run and the
  WebAssembly build. Release archives for the three platforms are built from
  version tags.
- **Security and maintenance**: CodeQL (Rust, JavaScript, Actions),
  `cargo-deny` (advisories, licences, bans, sources), gitleaks secret
  scanning, dependency review, OpenSSF Scorecard and Dependabot for Cargo and
  GitHub Actions. Actions are pinned to commit SHAs and run with read-only
  tokens by default.
- **Community files**: `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CITATION.cff`,
  `.github/CODEOWNERS`, issue forms (bug, feature, improvement), a pull
  request template, `.gitattributes` and `.env.example`.

[Unreleased]: https://github.com/Ale-Newport/Conway_GameOfLife/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Ale-Newport/Conway_GameOfLife/releases/tag/v0.1.0
