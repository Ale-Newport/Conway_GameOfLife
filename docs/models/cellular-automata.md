# cellular-automata — Classic Conway and the Cellular Automata Laboratory

`crates/cellular-automata` holds the Life engines and every model of the
**Classic Conway** mode and the **CA Laboratory**:

| Model id | Type | What it runs |
|---|---|---|
| `life.classic` | `LifeSim` (`LifeMode::Classic`) | Conway's Life and any Life-like `B/S` rule; default preset "Conway (B3/S23, unaltered)" |
| `ca.life_like` | `LifeSim` (`LifeMode::Lab`) | the same model for the laboratory presets (HighLife, Seeds, Day & Night, …) |
| `ca.generations` | `GenerationsSim` | Generations rules `B/S/C` with up to 255 states (Brian's Brain, Star Wars, …) |
| `ca.wireworld` | `WireworldSim` | Wireworld (4 states) |
| `ca.langtons_ant` | `LangtonsAntSim` | Langton's ant and multi-colour turmites (`RL`, `LLRR`, `RLR`, …), several ants |
| `ca.wolfram` | `WolframSim` | the 256 elementary 1D rules as a space-time diagram |

Dependencies: `simulation-core`, `patterns`, `hashlife`, `serde`, `ron`,
`rayon`, `thiserror` (all workspace dependencies); `criterion` for the
benchmark. No new third-party crate.

## Contents

1. Public API
2. Engines
3. Engine selection
4. `LifeSim`
5. The CA Laboratory models
6. The view contract
7. Snapshots, reset and determinism
8. Scenarios and pattern data
9. Approximations and limitations
10. Verification
11. Performance (measured)
12. References

---

## 1. Public API

| Item | Purpose |
|---|---|
| `engine::{LifeEngine, EngineKind, EngineChoice, GenStats}` | engine trait, the four implementations' kinds, automatic/fixed choice, births/deaths of a generation |
| `DenseEngine`, `BitEngine`, `SparseEngine`, `HashlifeEngine` | the engines |
| `create_engine`, `switch_engine`, `select_engine`, `SelectionInput`, `tile_regularity`, `engine::AUTO_REGULARITY_MAX_POPULATION`, `engine::MAX_SWITCH_CELLS` | construction, exact state transfer (refused, never lossy, when the target cannot hold every cell or the pattern has more than 2^26 live cells), automatic choice |
| `LifeEngine::reserve`, `DenseEngine::window_fits`, `engine::BORDER_CELL_BUDGET`, `LifeEngine::count_in_rect` | capacity checks before writes; bounded view cost on Hashlife; exact live-cell counts of rectangles (up to a limit) without listing a cell |
| `engine::reference::reference_step` | naive reference stepper (used by the property tests) |
| `LifeSim`, `LifeConfig`, `LifeMode`, `LifeCommand`, `PasteMode`, `MirrorAxis`, `RulePreset`, `InitItem`, `PatternSource`, `PatternLibrary`, `place_pattern` | the Life model, its configuration, commands and pattern resolution; `LifeSim::copy_region` is the copy query and `LifeSim::cut_region` the cut (copy + clear), both refused above `MAX_EDIT_CELLS` live cells |
| `GenerationsSim`/`GenerationsConfig`, `WireworldSim`/`WireworldConfig`, `PaintCommand`, `StateInit` | multi-state models and their shared painting commands (`halt_reason` reports a reset that could not be carried out) |
| `LangtonsAntSim`, `AntConfig`, `AntCommand`, `Ant`, `Heading`, `TurmiteRule`, `AntView` | ants; `LangtonsAntSim::world` is the running world (the parameters describe the next reset) |
| `WolframSim`, `WolframConfig`, `WolframCommand`, `Boundary`, `EcaStart`, `wolfram::eca_step` | elementary CA; `WolframSim::{width, boundary, first_row}` describe the running diagram (rows `first_row()..=tick()` are kept) |
| `GridView`, `GridViewRequest`, `view::TexelGrid` | renderer contract (§6) |
| `geometry::{CellRect, World, Rotation, Symmetry, line, disc, circle_outline, rectangle, stroke, check_fill, random_fill, random_fill_each, transform_in_rect}` | shapes, symmetric random fills (streamed up to the largest world, `MAX_FILL_CELLS` = 2^32 cells) and `World::images` (a rectangle as the topology sees it) used by every editing command |
| `scenario::{Scenario, ScenarioModel, BuiltSimulation, parse, load_file, load_dir, builtin_scenarios, builtin_scenario, to_ron}` | scenario files |
| `library::EMBEDDED_PATTERNS` | the permissively licensed Wireworld circuits embedded with `include_str!` |
| `CaError` | typed errors (converted into `SimError`) |

Coordinates: `x` to the right, `y` downward, `i64`. Finite worlds hold the
cells `0 ≤ x < width`, `0 ≤ y < height`.

---

## 2. Engines

All four implement `LifeEngine`: set/get cell, set many cells, clear, clear a
rectangle, load/export cells (export is sorted row-major), step one
generation (returning births and deaths), `advance(n)`, population
(maintained incrementally, constant time), bounding box, cells in a
rectangle and their exact number (`count_in_rect`, see below),
level-of-detail counting for views (`count_live`), rule change,
`reserve` (capacity check before writing), `memory_bytes`, `name`. Every
engine runs every `B/S` rule (any birth and survival masks; `B0` only on
finite worlds, see below).

| Engine | Storage and algorithm | Topologies |
|---|---|---|
| `DenseEngine` | one byte per cell in a `(W+2)×(H+2)` grid with a ghost ring, stepped through a 2×9 lookup table into a second buffer; rayon over row bands for grids of 2^16 cells or more | all five; the infinite plane is an auto-growing window |
| `BitEngine` | 64 cells per `u64`; full-adder neighbour counting (SWAR) with a dedicated 30-operation Conway kernel and a generic count-plane kernel for other rules; rayon over row bands (global persistent pool) from 4096 words | bounded, torus, Klein bottle, cross-surface |
| `SparseEngine` | 64×64 bit tiles (`[u64; 64]`) in a `HashMap` keyed by tile coordinates; active set = tiles changed in the last generation and their 8 neighbours; candidates stepped in parallel as pure functions of the old tiles and applied in sorted key order; empty tiles freed | infinite plane |
| `HashlifeEngine` | adapter over `hashlife::Universe`; `advance(2^k)` is one memoised `step_pow2(k)`, other counts are binary-decomposed | infinite plane |

**Topologies.** Neighbour positions outside a finite world are mapped by
`Topology::wrap` (core semantics, following Golly's bounded grids): torus =
plain wrap, Klein bottle = crossing the top/bottom edge mirrors x, cross-surface
= both pairs mirrored, bounded plane = dead outside. The dense engine refills
its ghost ring from this mapping before every step. The bit engine builds a
*ghost border* per step: rows `y = −1` and `y = H` (the opposite edge row,
bit-reversed for twisted edges, all zero for the bounded plane) and the single
cells at `x = −1` and `x = W` of every row. Both are exactly Golly's ghost-cell
emulation, so on worlds narrower than three cells the same cell can be counted
twice as a neighbour — the reference stepper does the same.

**B0 rules** (birth on 0 neighbours) are run literally on finite worlds by
the dense and bit engines: every cell of the world follows the rule and the
outside of a bounded plane stays dead (bits beyond the row width are masked
after every step). On the infinite plane B0 would light up all of space; the
sparse and Hashlife engines and the infinite dense window reject such rules
with `CaError::UnsupportedRule`. There is no Golly-style background-phase
emulation.

**The infinite dense window** keeps every live cell at least one cell away
from its border: before a step it grows by a quarter of its size (at least 16
cells) when a live cell touches the border, and every 64 steps it shrinks when
the live cells occupy less than a quarter of it. Outside the window everything
is dead, so the result is the exact unbounded evolution. It is limited to 2^26
cells (`MAX_DENSE_WINDOW_CELLS`); beyond that `step` fails with a message
suggesting the sparse or Hashlife engine. Writes of live cells the window
cannot hold are refused up front: `LifeEngine::reserve` (called by every
`LifeSim` command, by engine switches, snapshot loading and resets) fails
with that message and nothing is written. `set_param` also refuses a fixed
dense engine or a world change on the infinite plane when the scenario's
initial content (home region of a random fill, patterns) would not fit.

**Sparse active set.** A tile is recomputed only if it or one of its 8
neighbours changed in the previous generation (or was edited): otherwise its
3×3 tile neighbourhood equals the one of the previous generation, so its next
state equals its current state. Still lifes therefore cost nothing. The plane
is the whole `i64` range; cells that would be born beyond it are discarded.

**Rectangle counts.** `count_in_rect(rect, limit)` returns the number of
live cells of a rectangle, exact up to `limit` (the smaller of the two;
`u64::MAX` gives the exact count), without listing any: the dense engine
scans the clipped bytes, the bit engine popcounts masked words, the sparse
engine adds the cached population of every tile inside the rectangle and
popcounts the tiles its border cuts, and the Hashlife adapter adds cached
quadtree populations (read through `Universe::density_grid`), descending
only into non-empty squares that cross the border and listing the overlap of
squares of at most 64 cells. The universe reports node populations through
`u32` density pixels, so a square inside the rectangle that holds more than
`u32::MAX` cells is split further — only while the count is still below
`limit`; callers pass their maximum plus one, which keeps the cost
proportional to the squares crossing the border for any population. Both
users of `cells_in_rect` in `LifeSim` count first: the region tools (§4.3)
and the tracker after an edit (§4.4).

**Engine switching** (`switch_engine`, the `engine` parameter) exports the
live cells and loads them into the new engine: the state is transferred
exactly, and the future is identical (tested). A switch the target cannot
hold (the dense window on the infinite plane) is refused and the running
engine is kept; no cell is ever dropped. The exported list takes 16 bytes per
live cell, so a switch of more than `MAX_SWITCH_CELLS` = 2^26 live cells (at
most 1 GiB of list) is refused before anything is listed, with a message
giving the population and the limit (shown by `set_param` for the `engine`
parameter); a Hashlife pattern of billions of cells therefore stays on
Hashlife instead of exhausting memory.

**Hashlife `advance(n)`**: `n = 2^k` with `k ≤ 61` is one atomic
`step_pow2(k)`; other counts (including `2^62`, `2^63`) use the universe's
decomposition. `LifeSim` itself splits a tick's generations into
power-of-two jumps and counts every completed jump, so its generation
counter stays exact when a later jump fails (coordinate overflow near
±2^61).

## 3. Engine selection

`EngineChoice::Auto` uses `select_engine`:

| World | Condition | Engine |
|---|---|---|
| finite | at most 128 × 128 cells | Dense |
| finite | larger | Bit |
| infinite | power-of-two tick of at least 16 generations ("large jumps") | Hashlife |
| infinite | at least 65,536 live cells and tile regularity ≥ 0.5 (power-of-two tick) | Hashlife |
| infinite | otherwise (active, chaotic or small patterns) | Sparse |

*Tile regularity* = `1 − distinct / non-empty` over the 64×64 tiles of the
pattern (tile contents compared through a 64-bit FNV-1a hash): 0 for soups, close to 1 for glider
streams, agar or metapixel arrays — the structures Hashlife compresses. The
choice is re-evaluated on reset, when a pattern is loaded and when the
generations per tick change; a fixed choice is never overridden. Measuring
lists every live cell, so `LifeSim` measures patterns of at most
`AUTO_REGULARITY_MAX_POPULATION` = 2^20 live cells: a larger pattern keeps
Hashlife if it runs (leaving it would list every cell too; a breeder after a
big jump has billions), whatever the tick, and is treated as irregular by the
other engines. The choice therefore costs at most a few tens of megabytes
for any population. A pattern of more than `MAX_SWITCH_CELLS` = 2^26 live
cells keeps the running engine whatever the choice would be (switching would
copy every cell; every engine runs every tick, Hashlife by decomposing it
into powers of two): a large sparse soup stays on the sparse engine when the
tick becomes a large power of two. Rationale
(see `docs/research/life-engines-hashlife.md` §5.3): the bit engine has a
constant cost of W·H/64 words per generation and wins on finite and dense
worlds; tiles cost in proportion to the active area and win on unbounded,
localised activity; Hashlife is sub-linear in time on regular patterns but
performs poorly on chaos, so it is only chosen for big time steps or large
regular patterns. The dense engine is the simplest (and the only one for the
tiniest worlds where SWAR rows would be mostly padding).

## 4. `LifeSim`

### 4.1 Rule presets

`RulePreset`: Conway (B3/S23, unaltered), HighLife B36/S23, Seeds B2/S,
Day & Night B3678/S34678, Life without Death B3/S012345678, Maze B3/S12345,
2×2 B36/S125, Replicator B1357/S1357, Diamoeba B35678/S5678, Morley B368/S245,
Anneal B4678/S35678, Custom. Presets are constants. Editing the `rule`
parameter never modifies a preset: it selects the preset whose rule was typed
or **Custom**, so the Conway preset cannot be altered by accident; selecting a
preset again restores its exact rule.

### 4.2 Parameters

| Key | Kind | Policy | Meaning |
|---|---|---|---|
| `preset` | choice | NextTick | rule preset |
| `rule` | text | NextTick | `B3/S23`, `b3s23`, `S23/B3`, `23/3` (S/B), names; B0 only on finite worlds |
| `topology` | choice | RequiresReset | bounded, torus, klein_bottle, cross_surface, infinite |
| `width`, `height` | int 1..=65536 | RequiresReset | world size (the home region on the infinite plane) |
| `engine` | choice | Immediate | auto, dense, bit, sparse, hashlife (exact transfer; refused above 2^26 live cells) |
| `generations_per_tick` | int 1..=2^30 | NextTick | must be a power of two while Hashlife runs |
| `track_ages`, `track_activity` | bool | Immediate | age (G) and activity (B) channels |

Invalid combinations are rejected by `set_param` with a message (B0 on the
infinite plane, the bit engine on the infinite plane, Hashlife with a
non-power-of-two step, a world too large for a fixed engine, an engine
switch of more than 2^26 live cells, a world on which the scenario cannot be
rebuilt — e.g. a non-square world for an 8-fold symmetric soup, or a home
region too large for a fixed dense engine on the infinite plane), so a
NextTick or RequiresReset change can never fail later. `reset` rebuilds
atomically: should it still fail, the state is kept and the reason is
reported by `halt_reason` / `is_finished`.

### 4.3 Commands (`LifeCommand`)

`SetCell`, `Stroke` (brush/eraser: discs of a radius along the segments
between points), `Line` (Bresenham), `Rectangle` and `Circle` (filled or
outline; discs are `dx² + dy² ≤ r(r+1)`), `Paste` (pattern at a position with
clockwise rotation and left-right mirror, modes Or / Copy / Xor), `Clear`,
`ClearRegion`, `Randomize` (density, optional seed, region, symmetry none /
horizontal / vertical / both / 4-fold rotational / 8-fold), `MoveRegion`,
`RotateRegion`, `MirrorRegion`, `LoadPattern` (replace everything, restart at
generation 0, optionally adopt the pattern's rule), `ImportText` (any format of
the `patterns` crate). Copy is the query `LifeSim::copy_region`, cut is
`LifeSim::cut_region` (copy + `ClearRegion`); the paste preview lives in the
user interface. Single cells map through the topology (wrap on tori, ignored
outside a bounded plane); `ClearRegion`, `Randomize` and the source of the
region tools are clipped to the world. The *destination* of `MoveRegion`,
`RotateRegion` and copy-mode `Paste` is cleared as the topology sees it
(`World::images`: wrapped and mirrored across the edges of tori, Klein
bottles and cross-surfaces), so moved or pasted cells that wrap around an
edge replace what was there instead of merging with it. Drawing shapes larger
than 2^26 cells (`MAX_EDIT_CELLS`) are refused, and so are moves, rotations,
mirrors, copies and cuts of a region holding more than 2^26 live cells (they
list its cells one by one; the region is counted first with
`count_in_rect`, so nothing is listed and nothing changes); random fills are
streamed into the engine and cover any region up to 2^32 cells (the largest
world). Shapes, pastes and rotations that would leave the 64-bit plane are
refused (never clamped).

Symmetric fills visit the region row-major and draw one random number per
symmetry orbit (at the orbit's first cell), so a seed always gives the same
soup; rotational symmetries need a square region.

### 4.4 Ticks, metrics, tracking

One tick advances `generations_per_tick` generations; `tick()` is the
generation count. Without tracking, Hashlife advances the whole tick in one
memoised jump; every other engine (and Hashlife with tracking on) steps
generation by generation.

Metrics: `population`, `births` and `deaths` (summed over the generations
of the last tick, also after `advance(n)`; omitted when Hashlife jumps
without tracking), `density`
(population / world area, or / bounding-box area on the infinite plane),
`bbox_width`, `bbox_height`, `generation`, `engine` (0 dense, 1 bit, 2 sparse,
3 Hashlife).

**Age and activity are computed by the model once per generation, never per
frame.** The tracker keeps the sorted live cells with their ages and a sorted
trail; after every generation a linear merge with the new live set finds the
births and deaths. Age = generations a cell has been alive (its birth
generation counts as 1, saturating at 255). Activity = 255 when a cell was
born or died during the last tick (in any of its generations), minus 16 per
tick afterwards (the trail decays once at the first generation of every
tick), matching the view contract "255 when the cell changed this tick". Edits
are merged for the edited region without ageing anything: the region's live
cells are counted first and listed only when there are at most 2^26
(`MAX_EDIT_CELLS`); an edit that leaves more inside its region (a fill of a
large world, for example) is applied and stops tracking — `track_ages` and
`track_activity` turn off — instead of listing them. Cost O(live cells +
trail) per generation on every engine; enabling tracking starts every live
cell at age 1. Views read the tracker only inside the request (rows found by
binary search, then the requested columns).

## 5. The CA Laboratory models

* **Life-like presets** reuse `LifeSim` (`ca.life_like`) with the presets and
  scenarios of §8.
* **`GenerationsSim`** follows Golly's `generationsalgo`: states 0 dead, 1
  alive, 2..C−1 dying; only state-1 cells count as neighbours; a dead cell
  with *n* ∈ B becomes 1; a live cell stays 1 with *n* ∈ S, otherwise starts
  dying (state 2, or 0 when C = 2); a dying cell advances by one and wraps to
  0 at C. Rules by name or notation (`/2/3`, `B2/S/C3`, `345/2/4`, `g3b2s`),
  presets for every named rule of the `patterns` crate. Metrics `alive`,
  `dying`, `births` (0 → 1), `deaths` (cells leaving state 1), `density`,
  `generation`. After a NextTick rule change, cells in states that no longer
  exist decay to 0 in one generation.
* **`WireworldSim`**: 0 empty, 1 head, 2 tail, 3 conductor; head → tail →
  conductor; a conductor becomes a head with exactly 1 or 2 head neighbours.
  Metrics `heads`, `tails`, `conductors`, `generation`.
* Both run on finite worlds (bounded, torus, Klein bottle, cross-surface;
  at most 2^28 cells) through the shared byte grid of the dense engine (ghost
  ring, lookup table counting state-1 neighbours, parallel row bands), with
  optional activity trails (255 on change, −16 per generation; one tick is
  one generation) and the painting commands `PaintCommand` (cell, stroke,
  line, rectangle, circle, paste keeping states, load, import, clear, clear
  region, random fill of a state, streamed for any world size). States are
  validated against the rule. World changes are validated so that the next
  reset can rebuild the scenario; a reset that fails anyway keeps the state
  and halts the model with a reason (`halt_reason`, `is_finished`) instead of
  failing silently. A reset point may hold states of a rule replaced since;
  they decay to 0 in one generation, as after any rule change.
* **`LangtonsAntSim`**: ants walk on colours. Each step every ant, in order,
  turns by the rule letter of the colour under it (`L`, `R`, `N` none, `U`
  U-turn), advances that colour by one (mod the number of colours, 2..16), and
  moves forward. Ants do not collide; two ants on one cell act one after the
  other. Infinite plane (64×64 colour tiles in a `BTreeMap`), torus (wrap) or
  bounded plane (an ant leaving it is removed; the model finishes when no ant
  is left). Steps per tick 1..10^6; `tick()` counts steps. The view is
  `AntView { grid: GridView, ants }`: the colour grid (B = 255 for the cells
  flipped during the last tick, 16 less per tick afterwards, as in the view
  contract; at most the 2^20 most recent flips are kept, which only matters
  with very many steps per tick) plus the ants in the requested rectangle.
  Commands: paint colours, random colours (streamed on finite worlds; at most
  2^26 cells per fill on the infinite plane, where every touched tile is
  allocated), clear colours, add ants, remove the ants in a rectangle.
  Snapshots store the running rule separately from the rule of the next
  reset.
* **`WolframSim`**: rule numbers 0..=255 with Wolfram's numbering (the new
  cell is bit `4l + 2c + r` of the rule), bit-packed rows, boundaries fixed 0,
  fixed 1 or periodic, first row a single cell at `width / 2` or random (seeded
  density). The last `history` rows are kept (rows × width ≤ 2^30). The view is
  the space-time diagram: cell `(x, t)` is cell `x` of row `t` (rows = time),
  B = 255 where the cell differs from the one above it. Commands edit the
  current row.

## 6. The view contract

All models answer a `GridViewRequest { x0, y0, x1, y1, max_width,
max_height }` — an **inclusive** cell rectangle and a texel budget (each side
clamped to 1..=8192) — with a `GridView { origin_x, origin_y, width, height,
cells_per_texel, rgba }`:

* `cells_per_texel` is the smallest power of two for which the texture fits
  the budget (at most 2^31; a request too large even then is shrunk around its
  centre).
* Texel boundaries are aligned to multiples of `cells_per_texel` in absolute
  cell coordinates (stable while panning); `origin_*` is the cell of the
  top-left corner of texel (0, 0); the view covers every requested cell.
* Every texel aggregates only the cells of its block that lie inside the
  request (and inside a finite world). **No model reads cells outside the
  request**: dense grids and bit rows scan only the clipped rectangle, the
  sparse engine visits only tiles intersecting it, the age/activity tracker
  visits only its entries inside the rectangle, and the Hashlife adapter
  uses cached quadtree populations (`density_grid`) for texels fully inside
  the request and enumerates only the live cells of the partly covered border
  strips — at most `BORDER_CELL_BUDGET` = 2^21 per view. Beyond that budget
  (only for requests not aligned to the texel grid that cut through
  gigantic populations, e.g. a zoomed-out breeder after 10^7 generations) a
  border texel is *estimated* as its whole aligned block's cached population
  times the covered fraction of the block, so a view never enumerates more
  cells (see §9).
* Channels: R = cell state (most frequent non-zero state of the block for
  level-of-detail texels, ties to the lowest state; for Life 1 if any cell is
  alive); G = age (mean age of the block's live cells; 0 when not tracked);
  B = activity (maximum of the block; 0 when not tracked); A = 255 / 0 for an
  exact live / dead cell, `round(255 × non-zero / covered cells)` for
  level-of-detail texels. Texels covering no existing cell are all zero.

## 7. Snapshots, reset and determinism

Snapshots (schema version 1, model id checked) hold the complete state:
cells (packed bit rows for finite Life worlds, a macrocell file for Hashlife,
a sorted list otherwise; raw bytes for multi-state worlds), the random
generator, pending NextTick / RequiresReset changes, tracker, reset point and
halt reason (and, for ants, the running rule next to the rule of the next
reset). Restoring replays identically (tested on every engine and model).
Restored data is validated before anything is allocated or committed (world
sizes, cell-data sizes, tracker consistency, colours, ant positions, row
lengths), so a corrupted snapshot is an error, never a panic (tested by
flipping bytes of snapshots of every model).

`reset()` returns to generation 0 the way Golly does: the scenario's initial
content plus any edit made at generation 0 (the state is captured at the
first step). After a RequiresReset change, reset rebuilds the initial content
from the scenario and the seed.

Determinism: randomness comes from the `SimRng` stored in each model; rayon
only computes pure functions of read-only buffers into separate output
buffers (row bands, candidate tiles) whose integer counts are summed; the
sparse engine sorts tile keys before any iteration that matters; the ant
colours use a `BTreeMap`.

## 8. Scenarios and pattern data

RON files, also embedded with `include_str!` (`scenario::builtin_scenarios`):

| `scenarios/conway/` | |
|---|---|
| `glider-gun-showcase` | two Gosper guns (one mirrored) with diverging streams |
| `methuselah-gallery` | R-pentomino, acorn, diehard, pi- and B-heptomino, thunderbird; ages tracked |
| `random-soup-torus` | 512² torus, 35 % soup, ages and trails |
| `infinite-puffer-world` | four puffers, 16 generations per tick → Hashlife |
| `pattern-collisions` | 3-glider LWSS synthesis, eater 1, Snark, buckaroo |
| `klein-bottle-symmetric-soup` | 8-fold symmetric soup on a Klein bottle |
| `bounded-r-pentomino` | the R-pentomino in a 128² box |
| `large-bit-soup` | 2048² torus → bit engine |
| `breeder-hashlife` | Breeder 1 at 64 generations per tick |

| `scenarios/ca-lab/` | |
|---|---|
| Life-like | `highlife-replicator`, `seeds`, `day-and-night-rocket`, `life-without-death`, `maze`, `two-by-two`, `replicator-rule`, `diamoeba`, `morley-jellyfish`, `anneal-coarsening` |
| Generations | `brians-brain`, `star-wars` |
| Wireworld | `wireworld-circuits` |
| Ants | `langtons-ant-highway`, `turmite-llrr`, `turmite-four-ants` |
| Wolfram | `wolfram-rule-30`, `wolfram-rule-90`, `wolfram-rule-110`, `wolfram-rule-184` |

`PatternLibrary` resolves `Library("id")` sources: the Wireworld circuits
embedded in this crate first, then the 18 classics embedded in `patterns`,
then the asset catalog when one is loaded (`PatternLibrary::load`), and
finally a file `<catalog directory>/<id>.rle` (plain relative ids only).
Scenarios that need catalog-only patterns report "load the asset catalog"
when built without it. Small CA-lab objects (replicator, rocket, gliders of other rules)
are inline RLE in the scenario files, with their source in `reference`.

**Wireworld circuits** (`assets/patterns/wireworld/`, see its
`PROVENANCE.md`): diode, period-8 and period-12 clocks, OR and XOR gates are
own layouts (MIT OR Apache-2.0); the AND gate is Harold V. McIntosh's from
Golly's `gate-AND.mcl` (GPL-2.0-or-later data). That file is never
compiled into the binary: like the OTCA tiles it is read from the asset
directory at run time, so `wireworld-circuits` needs the asset catalog.
An own AND was not designed:
`(A ∨ B) ⊕ (A ⊕ B)` needs a wire crossing (its connection graph contains
K3,3 once inputs and output are on the outside).

## 9. Approximations and limitations

* Only two-state outer-totalistic Moore rules run on the Life engines
  (isotropic non-totalistic, hexagonal, von Neumann and MAP rules are rejected
  by the `patterns` parser). Generations rules are Moore-only and limited to
  255 states.
* B0 rules: finite worlds only, literal semantics (no complement emulation).
* Finite worlds use the four topologies of `simulation_core::Topology`;
  Golly's shifted tori, tubes, spheres and vertically twisted Klein bottles
  are not offered. Views show the fundamental domain only (no tiling).
* Births/deaths are not reported for Hashlife jumps without tracking.
* Hashlife views: border texels of unaligned requests whose partly covered
  strips hold more than 2^21 live cells are estimated from block populations
  (proportional to the covered area), not counted exactly.
* Ant trails keep at most the 2^20 most recent flips.
* Tracking costs O(live cells) per generation and stores 17 bytes per live
  cell; Hashlife with tracking steps one generation at a time. An edit whose
  region holds more than 2^26 live cells stops tracking (§4.4).
* Engine switches copy at most 2^26 live cells; above that the running
  engine is kept (refused for a fixed choice, kept by the automatic one).
* Hashlife texel counts saturate at `u32::MAX` per texel (only for blocks of
  more than 4·10^9 live cells).
* The sparse plane ends at the `i64` limits (births beyond are dropped);
  Hashlife reports a coordinate overflow there and the model halts.
* Multi-state models and ants have no infinite *multi-state* engine (ants do
  have an infinite plane).
* Ants never annihilate (Pegg's variant) and do not split.

## 10. Verification

```sh
CARGO_TARGET_DIR=target/agents/cellular-automata cargo test -p cellular-automata
CARGO_TARGET_DIR=target/agents/cellular-automata cargo test -p cellular-automata -- --ignored
CARGO_TARGET_DIR=target/agents/cellular-automata cargo clippy -p cellular-automata --all-targets -- -D warnings
```

* **`tests/engines_agree.rs`** (property test): Dense and Bit against the
  naive reference stepper on 48 random soups with random rules (B0 included)
  on all four finite topologies, sizes 1×1 to 130×11 (word boundaries), 40
  generations each, comparing cells, population, births and deaths and
  bounding boxes every generation; Dense, Sparse and Hashlife against the
  reference on 36 random soups and rules on the infinite plane (offsets up to
  ±2^50, 15–45 generations); 300-generation runs on a Klein bottle; 2000
  generations of an unbounded soup on three engines; Hashlife jumps against
  sparse stepping; engine switching; level-of-detail counts of every engine
  against direct counting; region listings and `count_in_rect` (exact and
  capped at several limits) of every engine on random rectangles, a whole
  tile and the whole plane.
* **`tests/catalog.rs`**: every one of the 80 catalog patterns against its
  declared data — initial population and size; still lifes; oscillator periods
  (minimal) and population ranges; spaceship periods and displacements; gun
  core periods and emitted gliders/MWSS counted as free spaceships; puffer and
  rake heads moving by (dx, dy) every period; eaters restored after eating the
  demonstration glider; reflectors restored with one glider leaving in a new
  direction; methuselah stabilisation generation and final population (free
  spaceships ignored, not periodic one generation earlier); diehard's death
  at 130; linear growth from `linear_from`; quadratic growth populations;
  sawtooth and LWSS synthesis. Plus: the Gosper gun adds one glider (5 cells)
  every 30 generations; the R-pentomino has 116 cells at 1103 on the sparse,
  Hashlife and dense engines. `#[ignore]`d long-horizon checks: Breeder 1 has
  1,302,083,334,180,208,337,404 cells at generation 10^12 (Rokicki 2006), and
  the sawtooth is back to 177 cells at generation 702,720.
* **`tests/life_rules.rs`**: HighLife replicator = rule-90 superposition for
  n = 1..16 and its population sequence for generations 0..24; Seeds keeps no
  cell alive two generations, duoplet, photon, domino; Day & Night, Anneal,
  Majority and Fredkin commute with inversion, Life does not; Day & Night
  rocket (170 cells, period 40, (−20, 0)); Life without Death monotone; 2×2
  glider and block lattice; Replicator 8 / Fredkin 9 copies; Maze
  wickstretcher; Morley jellyfish and puffer; `23/3/2` equals B3/S23.
* **`tests/life_sim.rs`**: parameters and policies, the protected Conway
  preset, B0 handling, engine switching mid-run, Hashlife ticks, metrics,
  every command, reset semantics, snapshots on every engine, per-generation
  tracking, view budget and density; region tools and copy-mode paste across
  the edges of a torus replace (not merge with) the wrapped destination; the
  fixed dense engine refuses cells it cannot hold (commands, switches, world
  changes) instead of dropping them; random fills of a 16,384 × 8,200 world;
  activity = 255 for every change of the tick, −16 per tick; births/deaths of
  the last tick after `advance(n)`; Hashlife jumps keep the generation exact
  when a later jump fails at the ±2^61 limit; cut; the automatic choice
  keeps Hashlife for a soup too large to measure and for a Gosper gun after
  2^40 generations (about 1.8 × 10^11 cells) without listing them, and keeps
  Sparse for a full block one row above 2^20 cells (one of exactly 2^20 is
  measured and moves to Hashlife).
* **`tests/listing_limits.rs`**: a counting global allocator (the crate's
  only `unsafe` code: it forwards to the system allocator and fails single
  allocations above 4 GiB, so a regression aborts instead of exhausting the
  machine) measures what refusals allocate, the tests running one at a time.
  Moves, rotations, mirrors and cuts of a full 8,192 × 8,193 torus (2^26 +
  8,192 live cells), and a move of a Gosper gun's 1.8 × 10^11 cells on
  Hashlife, are refused unchanged allocating less than 16 MiB; switching that
  gun to the sparse or dense engine through `set_param` is refused with the
  limit in the message and the state kept, and `switch_engine` refuses it
  too, likewise without listing; at 64
  generations per tick the automatic choice keeps a sparse pattern of more
  than 2^26 cells instead of moving it to Hashlife; filling the whole torus
  with tracking on is applied and stops tracking without listing the cells,
  while a smaller region keeps it. Peak memory of the refusal test
  (`/usr/bin/time -l`, maximum resident set size): 3.24 GB when the refusal
  listed 2^26 + 1 cells first, 13 MB with the count.
* **`tests/generations.rs`**: Brian's Brain 1 → 2 → 0 and no surviving cells,
  the domino spaceship, the exploding 2×2 block (12, 20, 28), Star Wars decay,
  streamed fills of worlds above 2^26 cells, world changes that would make the
  scenario unbuildable refused by `set_param`.
* **`tests/wireworld.rs`**: electron speed, diode forward (arrival at 9) and
  blocked backward, clock periods 8 and 12, OR / XOR / AND truth tables.
* **`tests/ant.rs`**: OEIS A255938 (a(0..10), a(9976..9978), a(10000) = 720,
  a(11000) = 834, a(12000), a(15000)); the colour sequence is periodic with
  minimal period 104 exactly from step 9977 (not 9976); highway displacement
  (±2, ±2) per period; +12 black cells per period; LLRR mirror-symmetric at
  exactly 722 of the first 20,000 steps, including step 19,812; RL last
  symmetric at step 52 — all matching `docs/research/automata-catalog.md` §5;
  snapshots taken while a rule change waits for the reset replay with the
  running rule; the trail marks every flip of the last tick; `world()` is the
  running world while a world change waits for the reset.
* **`tests/wolfram.rs`**: rule 90 = Pascal's triangle mod 2 exactly for 300
  rows; rule 30 centre column = OEIS A051023 (257 terms); rule 184 conserves
  cars and reaches free flow; rule 110 edges; 30 ≅ 86 (mirror) ≅ 135
  (complement); fixed-1 boundaries; history and view; `width`, `boundary` and
  `first_row` describe the running diagram while changes are pending.
* **`tests/scenarios.rs`**: files equal their embedded copies; all 29 build
  with the asset catalog, run and replay from a snapshot; embedded-only
  scenarios build without it; the gun showcase's streams never collide;
  automatic engine choices of the scenarios; corrupted snapshots (bytes
  flipped across the payloads of seven models, truncation) restore with an
  error or a valid state, never a panic.
* Unit tests (`src/`): SWAR kernel against scalar counts for many rules,
  shifts and bit reversal, the dense window and its refusal of far-apart
  cells, Hashlife border-texel budget (exact up to the budget, proportional
  estimate beyond, exact for uniform blocks) and `advance(2^62)`, Hashlife
  rectangle counts against listings (cells at the `i64` extremes, the whole
  plane) and on a full 2^17 × 2^17 square whose blocks saturate the `u32`
  density pixels (exact without a limit, immediate with one), the
  topology images of rectangles (`World::images`) against cell-by-cell
  mapping on all finite topologies, streamed fills equal to listed fills,
  shapes refused at the edge of the 64-bit plane, tracker per-tick decay and
  clipped iteration.

## 11. Performance (measured)

Criterion benchmark `benches/engines.rs` (`cargo bench -p cellular-automata`).
Measured on 2026-11-10 on an Apple M2 Max (12 cores: 8 performance + 4
efficiency, 64 GB), macOS, Rust 1.98.1, workspace release profile
(`codegen-units = 4`, thin LTO), rayon's default global pool. **The machine
was heavily loaded by other build jobs (load average 8–12 on 12 cores)**, so
these are conservative numbers; the median of each Criterion estimate is shown.

| Benchmark | Workload | Time per iteration | Throughput | Review re-run (load 50–75) |
|---|---|---|---|---|
| `finite_soup_generation/dense/512` | one generation, 512² torus, 30 % soup (after 16 generations) | 138.7 µs | 1.89 Gcell-updates/s | 299.9 µs |
| `finite_soup_generation/bit/512` | same, bit engine | 57.9 µs | 4.53 Gcell-updates/s | 114.5 µs |
| `finite_soup_generation/dense/2048` | one generation, 2048² torus | 1.400 ms | 3.00 Gcell-updates/s | 2.436 ms |
| `finite_soup_generation/bit/2048` | same, bit engine | 196.3 µs | 21.4 Gcell-updates/s | 264.8 µs |
| `sparse_far_apart/generation` | one generation of 48 gliders and 48 pulsars in six groups around (±2^40, ±2^40) and the origin (2,544 cells at the start) | 211.5 µs | 12.0 M live cells/s | 203.3 µs |
| `hashlife_gosper_gun/advance_2^20_cold` | Gosper gun from an empty memo to generation 2^20 (one `step_pow2(20)`, engine created per iteration) | 544.1 µs | — | 552.7 µs |

The last column was measured again on the same machine during the code
review (2026-11-10, same command, after the review's changes; the measured
step, sparse and Hashlife paths were not modified) while other jobs kept the
load average between 50 and 75 on 12 cores: the single-threaded sparse and
Hashlife figures reproduce within 4 %, the rayon-parallel dense and bit
figures are 1.3–2.2× slower because their worker threads compete with the
other jobs. A first re-run at load average ≈ 170 was 10–30× slower for the
parallel engines, so these numbers should always be read with the load.

Reading: the bit engine is 2.4× faster than the byte engine at 512² (where
row-band parallelism barely pays) and 7× faster at 2048²; at 21 Gcell/s it is
in the range of the research prototype's single-machine figures (11–18
Gcell/s single-threaded, 106 Gcell/s with 12 idle threads at 16384²). The
sparse engine's cost follows the active tiles, not the coordinate range: the
patterns 2^41 cells apart cost the same as if they were adjacent. Hashlife
reaches generation 2^20 of a gun in about half a millisecond because the
glider stream is made of a few repeated quadtree nodes, which is why the
automatic choice uses it for large power-of-two steps.

## 12. References

* `docs/research/life-engines-hashlife.md` (SWAR kernel §5.1, tiles §5.2,
  engine choice §5.3, topologies §7), `docs/research/automata-catalog.md`
  (rules and verified behaviours), `docs/research/pattern-library.md`.
* Golly 5.1 sources and help (bounded grids, Generations), read for
  behaviour only.
* R. Wm. Gosper (1984), T. Rokicki (2006) for Hashlife; S. Wolfram (1983) for
  the elementary rule numbering; C. Langton (1986); A. K. Dewdney, Scientific
  American, January 1990 (Wireworld); OEIS A255938, A051023, A001316.
