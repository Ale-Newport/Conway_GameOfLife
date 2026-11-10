# Conway within Conway (`life.metapixel`)

`crates/metapixel` builds tilings of Brice Due's **OTCA metapixel** (2006)
and runs them **exactly**: the model state is a real Conway's Life (B3/S23)
pattern of up to hundreds of millions of cells, evolved generation by
generation (in Hashlife's memoised jumps) by `crates/hashlife`. Macro states
are *read* from that pattern every 35 328 generations. A separate,
clearly labelled **accelerated macro representation — not a physical
simulation** applies the programmed rule directly to the macro grid, only to
predict and flag disagreements.

How the construction works, the timing, the verification results and the
licences are in [`docs/OTCA_METAPIXEL.md`](../OTCA_METAPIXEL.md). The
validated geometry comes from
[`docs/research/otca-metapixel.md`](../research/otca-metapixel.md).

---

## 1. Model (rules actually executed)

* **Physical level (exact).** Each macro cell `(col, row)` of a `W × H` grid
  is the programmed ON or OFF tile OR-ed into the plane with its nominal
  origin at `(2048·col, 2048·row)` at generation 0 (tiles overlap their
  neighbours by 5 cells). Every cell of the plane then follows B3/S23,
  synchronously, with no approximation. `tick()` is the physical generation.
* **Programming.** The macro rule `B{β}/S{σ}` (any two-state
  outer-totalistic rule, `B0` included) is programmed by adding one cell per
  digit to every tile (`B_d = (148, 1404 + o)`, `S_d = (162, 1406 + o)`,
  `o = [0, 10, 20, 46, 56, 66, 92, 102, 112][8 − d]`); the macro
  neighbourhood (any subset of the 8 Moore directions) by removing the edge
  block of an unheard edge or the *opposite* corner LWSS of an unheard
  diagonal from every tile.
* **Readout.** Metageneration `n` is read at `t = (n + 1)·P`, `P = 35 328`,
  by counting live cells in each tile's core `[250, 1750]²` (tile-nominal):
  `0` = OFF, `29 795` = ON, anything else = **ambiguous** (reported, never
  rounded). The display shows `m(t) = max(0, ⌊(t − 7408)/P⌋)`; cores may
  switch while `t mod P ∈ [4320, 7407]` (`t ≥ P`).
* **Macro semantics observed** (and predicted by the accelerated model): an
  OFF macro cell turns ON iff the number of ON heard neighbours is in `β`; an
  ON one stays ON iff it is in `σ`; tiles outside the grid do not exist, so
  they are permanently OFF and never born.

The descriptor's `rules` text is generated from the running rule,
neighbourhood, size and step (`sim::params::rules_text`).

## 2. Parameters

| Key | Kind | Default | Policy | Effect |
|---|---|---|---|---|
| `rule` | text ≤ 32 | `B3/S23` | `RequiresReset` | macro rule (`B36/S23`, `23/3`, `B0/S`...) |
| `neighbourhood` | text ≤ 40 | `moore` | `RequiresReset` | `moore`, `von_neumann`, `none`, or listened directions `N,E,S,W,NE…` |
| `width`, `height` | int 1–256 | 3 | `RequiresReset` | grid size in tiles (pattern kept at the top-left) |
| `seed` | int ≥ 0 | 1 | `RequiresReset` | seed of the generator used by `Randomize` |
| `step` | choice | `35328` | `Immediate` | generations per tick: `1, 2, 4, …, 32768` or `35328` (one metagen); a tick that crosses a multiple of P pauses there to read the tiles, then completes its step (at `1024` the tick that reads at 35 328 ends at 35 840). Integers (`--param step=1024`) and `metagen` are accepted and normalised to the option text |
| `node_limit` | int 1024–2·10⁹ | 8 388 608 | `Immediate` | Hashlife soft node limit |

Changing the rule, neighbourhood or size changes the *physical* pattern, so
it only takes effect when the construction is rebuilt (`reset`). `params()`
reports the pending values.

## 3. Commands (`MetapixelCommand`)

Every command edits the **macro pattern** and immediately rebuilds the
construction at generation 0, clearing the readout history: inserting tiles
into a running construction is not meaningful (all tiles must share one
phase and their proximity fuses burn during the first cycle). On error
nothing changes.

| Command | Effect |
|---|---|
| `SetTile { col, row, on }`, `ToggleTile { col, row }` | one macro cell |
| `SetPattern(MacroGrid)` | whole pattern; its size becomes the grid size |
| `PlaceRle { rle, col, row }` | turns ON the live cells of an RLE pattern placed at `(col, row)` |
| `Randomize { density }` | random pattern from the model's `SimRng` (row-major) |
| `Clear` | all OFF |
| `RestartFromLatestReadout` | rebuild from the latest exact readout (refused if a tile was ambiguous) |

## 4. Metrics

`population` (physical cells), `generation`, `metagen` (= `m(t)`), `phase`
(`t mod P`), `readouts`, `exact_on_tiles`, `accelerated_on_tiles`,
`mismatches`, `ambiguous_tiles` (latest readout), `total_mismatches`,
`node_count`, `memory_bytes`. Exact values are 0 until the first readout at
`t = P` (`readouts` tells them apart).

## 5. Views (`MetapixelViewRequest` → `MetapixelView`)

* **Micro** (`micro: Option<GridViewRequest>`): an inclusive cell rectangle
  `[x0, x1] × [y0, y1]` and a texel budget. The model picks the smallest
  power-of-two `cells_per_texel` whose texel-aligned cover fits, snaps the
  origin to multiples of it, and fills the shared grid contract `GridView
  { origin_x, origin_y, width, height, cells_per_texel, rgba }`: R = 1 if
  the texel holds a live cell, G = 0 and B = 0 (age and activity are not
  tracked by Hashlife), A = 255 × live cells / requested cells of the texel
  (255/0 for exact texels). As in `cellular-automata`, every texel describes
  only the requested cells of its block, and no cell is iterated: texels
  whose whole block is requested come from `Universe::density_grid`
  (cached quadtree populations, O(visible nodes)); the border texels of an
  unaligned request count the requested part of their block exactly with
  `Universe::population_in_rect` (skipped for an empty block), one descent
  per border texel through the nodes crossing that part's border. Measured
  cost: +0.3 to +1.3 ms per view on 1024-texel requests of the galaxy and
  of a 64×64 construction (`docs/OTCA_METAPIXEL.md` §6). `ScaleInfo` gives
  cells per texel, texels per tile and the tiles visible.
* **Macro** (`macro_grid: true`): per tile the exact reading of the latest
  readout (`None` before `t = P`), the accelerated prediction for the same
  metagen, a mismatch flag and the tile's current physical population; with
  `live_readings` (grids ≤ 1024 tiles) also the threshold reading of every
  core *now* (`0` OFF, `≥ 21 677` ON, else transitioning). The view carries
  `EXACT_LABEL` and `ACCELERATED_LABEL`; a UI must show which one it draws.
* **Inspection** (`inspect: Some((col, row))`): the tile's plane origin,
  labelled annotations in tile-nominal coordinates (rule eaters `B0…B8`,
  `S0…S8` with their programmed state, the eight fuses with what they let the
  tile hear, the readout rectangle, the display area, and **unverified**
  circuit anchors from Golly's `metafier` and the slide captions), the
  current core count, and the eight neighbour channels.
* **Phase** (always): `t`, `P`, `t mod P`, `⌊t/P⌋`, `m(t)`, stage (first
  cycle / transition / stable), half of the cycle, latest readout, next
  readout generation.

## 6. Snapshots

Schema version 1: the physical pattern as Golly macrocell text (with its
generation), the macro pattern, active and pending configuration, step,
node limit, `SimRng` state, readout history, accelerated predictor and build
statistics. Restoring checks the model id, version, sizes, the generation and
that the physical rule is B3/S23; the memo is rebuilt on demand, so results
are identical (verified by `tests/sim.rs`).

## 7. Scenarios

RON files in `scenarios/metapixel/` (embedded with `include_str!`, loaded
with `scenario::{builtin, builtin_by_id, load_file, load_dir, parse}`):

```ron
(
    id: "metapixel.blinker", name: "Meta-blinker", description: "...",
    rule: "B3/S23",            // default B3/S23
    neighbourhood: "moore",    // default moore
    width: 3, height: 3,
    init: Rows(["...", "OOO", "..."]),   // or Random(density: 0.4) or Empty
    seed: 1,                   // default 1
    step: "35328",             // default one metagen per tick
    notes: ["..."],
)
```

| Id | Grid | Rule | Shows |
|---|---|---|---|
| `metapixel.blinker` | 3×3 | B3/S23 | golden-value meta-blinker |
| `metapixel.block` | 2×2 | B3/S23 | still life (exactly periodic from `t = P`) |
| `metapixel.glider` | 10×10 | B3/S23 | glider moving one metacell per 4 metagens |
| `metapixel.galaxy` | 15×15 | B3/S23 | Kok's galaxy; same macro pattern as Golly's demo |
| `metapixel.soup` | 6×6 | B3/S23 | seeded random soup |
| `metapixel.parity-von-neumann` | 9×9 | B135/S135, von Neumann | custom neighbourhood (XOR of 4 neighbours) |
| `metapixel.b0-single-tile` | 1×1 | B0/S | Brice Due's single blinking tile |

## 8. Approximations and limitations

* None at the physical level: every cell follows B3/S23 exactly.
* Macro states are only *defined* at readout times and by threshold during
  animation; between `4320` and `7407` generations into a cycle a core may be
  half-switched, which the live reading reports as "transitioning".
* The tiles are not embedded (GPL-2.0-or-later data): `TileSet::from_dir`
  must find `OTCAmetapixel-{OFF,ON}.rle`, otherwise construction fails with
  `TileFileMissing`.
* The simulation always builds its own tiling (OR-tiling, method A). Golly's
  own demo files (method B) are read by `read_golly_tiling` for verification
  only.
* Grids are full rectangles up to 256×256 tiles; sparse tilings (holes) are
  only supported when reading Golly files.
* The accelerated model is a prediction, never evidence; mismatches are
  counted rather than hidden.

## 9. How to test

```sh
cargo test -p metapixel                                   # 60 tests (incl. 7 exact runs), ~7 s
cargo test -p metapixel --profile verify -- --ignored     # 4 long verifications
cargo bench -p metapixel
```

What each test checks, and measured times, are listed in
[`docs/OTCA_METAPIXEL.md`](../OTCA_METAPIXEL.md#7-verification).
