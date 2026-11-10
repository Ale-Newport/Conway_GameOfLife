# Installation

This guide covers how to build and run Emergence Lab from source. There are no
prebuilt packages yet.

## 1. Prerequisites

* **Rust 1.95 or newer**, with edition 2024 support. The workspace sets
  `rust-version = "1.95"` and `edition = "2024"` in the root `Cargo.toml`.
  Install it with [rustup](https://rustup.rs). The research notes record
  development on Rust 1.98.1.
* **Git**, to clone the repository. Assets are plain files in the repository;
  no Git LFS configuration was found.
* **A GPU with a modern graphics API**, which Bevy needs: Metal on macOS,
  Vulkan or OpenGL ES on Linux, DirectX 12 or Vulkan on Windows.

### Platform notes

| Platform | Status in this repository | Notes |
|---|---|---|
| macOS (Apple silicon) | Built and run during development (Apple M2 Max, Metal). Smoke screenshots were taken there (`docs/research/bevy-0.20-api.md`). | Xcode Command Line Tools (`xcode-select --install`) provide the linker. |
| Linux | **UNVERIFIED**: not built or run as part of this project. | Bevy's default platform features in use include `x11`/`wayland` and `gilrs` (gamepads). Bevy's own Linux setup guide lists system packages such as `pkg-config`, `libudev-dev`, `libx11-dev`, `libxkbcommon-dev` and `libwayland-dev` (Debian/Ubuntu names). Audio is not enabled, so ALSA development headers should not be needed (UNVERIFIED). Follow Bevy's guide for your distribution. |
| Windows | **UNVERIFIED**: not built or run as part of this project. | Use the MSVC toolchain (Visual Studio Build Tools with the C++ workload), as recommended for Bevy. |

The Bevy features are set once in the workspace: `default-features = false`,
plus `2d`, `3d`, `ui` and `mesh_picking`.

## 2. Building

```text
git clone https://github.com/Ale-Newport/Conway_GameOfLife.git
cd Conway_GameOfLife
cargo build --release -p emergence-lab
```

## 3. Running the desktop application

```text
cargo run -p emergence-lab --release                         # dashboard
cargo run -p emergence-lab --release -- --mode <slug>        # open a laboratory directly
cargo run -p emergence-lab --release -- --help
```

`--mode` slugs come from `AppMode::slug` in `crates/interface/src/mode.rs`:

| Slug | Laboratory |
|---|---|
| `dashboard` | Dashboard (default) |
| `classic-conway` | Classic Conway |
| `ecosystem` | Ecosystem Simulator |
| `custom-life` | Custom Life Creator |
| `conway-within-conway` | Conway within Conway |
| `cellular-automata` | Cellular Automata Lab |
| `evolution` | Evolution Lab |
| `continuous-life` | Continuous Life |
| `3d-life` | 3D Life |
| `civilizations` | Emergent Civilizations |

How names are matched:

* `--mode` also accepts the variant name (`ClassicConway`) or the title
  (`"Classic Conway"`).
* Case, spaces, `-` and `_` are ignored.
* An unknown name is rejected and the valid slugs are listed.
* A known mode whose view is not registered opens the dashboard with a warning.

The window is titled "Emergence Lab" and is 1600 × 1000 logical pixels.

### Asset folder

From `apps/desktop/src/assets.rs`, the asset folder is the first of these
that exists:

1. `$EMERGENCE_LAB_ASSETS`. If it is set to something that is not a folder,
   startup fails with an error. An empty value is ignored.
2. `<directory of the executable>/assets`, for packaged builds.
3. The workspace `assets/` folder the binary was built from
   (`CARGO_MANIFEST_DIR/../../assets`, fixed at compile time).

The resolved path is made absolute and passed to Bevy's
`AssetPlugin::file_path`. The same binary therefore works from `cargo run`,
from `target/` and when packaged with an `assets/` folder next to it.

* The Conway within Conway mode reads the OTCA tiles from
  `<assets>/patterns/otca/` at run time.
* The Classic Conway pattern library reads `<assets>/patterns/catalog.toml`.

### User data folder

`persistence::UserDataDirs` (`crates/persistence/src/dirs.rs`) chooses the
data root:

| Condition | Root |
|---|---|
| `EMERGENCE_LAB_DATA` set and non-empty | that path |
| macOS | `~/Library/Application Support/EmergenceLab` |
| Linux / other Unix | `$XDG_DATA_HOME/emergence-lab` if `XDG_DATA_HOME` is absolute, else `~/.local/share/emergence-lab` |
| Windows | `%APPDATA%\EmergenceLab`, else `%USERPROFILE%\AppData\Roaming\EmergenceLab` |

Subfolders:

| Folder | Contents |
|---|---|
| `saves/` | Snapshots, as `saves/<mode>/<name>.elsnap` |
| `scenarios/` | User scenarios. The Custom Life editor library is in `scenarios/custom-life/` |
| `exports/` | Metric CSV/JSON files and pattern exports |
| `screenshots/` | Screenshots and PNG sequences |

The settings file is `settings.toml` at the root. The About window shows the
data folder in use.

## 4. Smoke screenshots (automated visual check)

```text
cargo run -p emergence-lab --release -- --smoke-screenshot out.png --smoke-frames 60
cargo run -p emergence-lab --release -- --mode 3d-life --smoke-screenshot shots/3d.png
```

* `--smoke-screenshot <PNG>` renders N frames, saves the window to the PNG
  and exits with status 0. Missing parent folders are created.
* `--smoke-frames <N>` sets N, from 1 to 100000. The default is 30. It
  requires `--smoke-screenshot`.
* The exit status is non-zero if the file could not be written, or if no
  screenshot arrived within 600 frames after the requested frame.

`scripts/capture_screenshots.sh` builds the release binary. It then captures
`docs/screenshots/<slug>.png` for the dashboard and for every mode slug.

## 5. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `EMERGENCE_LAB_ASSETS is set to ..., which is not a folder` | Point the variable at the `assets/` folder, or unset it. |
| `no asset folder found; tried: ...` | Run from a checkout, or put `assets/` next to the executable. |
| Conway within Conway shows an error about OTCA tiles | `assets/patterns/otca/OTCAmetapixel-{OFF,ON}.rle` are missing from the asset folder. |
| `unknown mode` | Use one of the slugs above (`--help` lists the options). |
