# OTCA metapixel: Conway within Conway

The "Conway within Conway" mode runs **Brice Due's OTCA metapixel** (Outer
Totalistic Cellular Automata Meta-Pixel, autumn 2005 – spring 2006): a
2048×2048 pattern of Conway's Life (B3/S23) that behaves like *one cell* of
another Life-like automaton. Tiled at pitch 2048, ON and OFF metapixels
exchange gliders with their neighbours; every 35 328 generations each tile
displays the state the programmed rule gives it.

`crates/metapixel` builds such tilings from Brice Due's original tiles and
runs them **exactly** with `crates/hashlife`. This page explains how a
metacell is represented, the difference between the exact construction and
the accelerated macro model, the timing, what was verified and how long it
takes, and the licences. The parameter/command/view reference is
[`docs/models/metapixel.md`](models/metapixel.md); the sources and every
measured number are in
[`docs/research/otca-metapixel.md`](research/otca-metapixel.md) (validated
with Golly's `bgolly`).

---

## 1. How a metacell is represented

A macro cell is **not** a data structure: it is a region of 2048×2048 cells
of a single, real B3/S23 pattern.

| Item | Value (tile-nominal coordinates, tile = `[0, 2047]²`) | Status |
|---|---|---|
| Physical tile | 2058×2058 (`[−5, 2052]²`), 23 920 cells (OFF) / 66 203 (ON) unprogrammed | verified |
| ON vs OFF | ON − OFF = 42 422 cells of the 1720×1720 display, bounding box (166..1946, 101..1881); OFF − ON = the 5-cell state boat at (569..571, 1970..1972) and 134 idle emitter cells | verified |
| Rule table | 18 *broken* eaters `o$3o$3bo$2bo!` in the SW quadrant; adding one cell completes the eater of digit *d*: `B_d = (148, 1404 + o)`, `S_d = (162, 1406 + o)`, `o = [0,10,20,46,56,66,92,102,112][8 − d]` | verified (all 18 bits) |
| Neighbour channels | 8 fuse objects: corner LWSSs and edge blocks; removing an edge block makes every tile deaf to that edge, removing a corner LWSS makes it deaf to the **opposite** corner | verified (64 runs, research note §4.2) |
| Readout core | `[250, 1750]²`: 0 cells when OFF, 29 795 cells when ON at `t ≡ 0 (mod 46)` | verified |
| Period | P = 35 328 = 2⁹·3·23 generations per metageneration | verified |

### What happens inside (interpretation — UNVERIFIED in detail)

The black-box behaviour above is verified by simulation. The *mechanism*
below is inferred from Brice Due's slide captions (slides 21–31 of the 2006
"How does it work?" post) and the subpattern labels of Golly's `metafier`
script; it was **not** tested piece by piece.

1. **Proximity fuses** (slide 21). During the first cycle each fuse object
   reacts with the neighbouring tile's tub, block or eater and opens a glider
   channel. At the border of a finite grid there is no neighbour, so the
   channel stays closed (this consequence *is* verified: nothing escapes and
   outside acts as OFF).
2. **Linear clock** (slide 22). A glider stream up the west side, from a gun
   at (46, 1924) to a cap at (83, 401), presumably sequences each cycle.
3. **MWSS track** (slide 23). An MWSS circulating around the perimeter
   presumably broadcasts the cell state, held as an *inverted* bit by the
   state boat (present in OFF tiles only — that difference is verified).
4. **LWSS train** (slide 24). Incoming neighbour gliders are presumably
   converted into LWSSs, so the train carries the neighbour count.
5. **Decode and lookup** (slides 25–26). The count is decoded against the
   B and S tables; a complete eater at row *d* gives a "yes".
6. **Boat logic** (slide 27). The toggle is `T = (¬S ∧ C) ∨ (B ∧ ¬C)`: the
   cell flips iff it is ON and not surviving, or OFF and born.
7. **Display** (slides 29–31). Two HWSS guns poll the latches and switch 43
   nested L-shaped LWSS lanes, which fill the 1720×1720 display at c/2.

`TileInspection::annotations` labels these components for micro-level
overlays; everything in items 2–7 carries `verified: false` and an
"UNVERIFIED role" note.

## 2. Building a construction

1. **Load** `assets/patterns/otca/OTCAmetapixel-{OFF,ON}.rle` (unprogrammed)
   with `TileSet::from_dir`, shift by −5 to nominal coordinates and validate
   populations, bounding box, the 18 empty programming sites and the 8 fuse
   objects. Missing files give `MetapixelError::TileFileMissing` naming the
   path and `PROVENANCE.md`.
2. **Program** both tiles identically (`TileSet::program`): add the rule-bit
   cells, remove the fuse objects of unheard directions. B3/S23 reproduces
   the shipped `*-B3S23.rle` tiles cell for cell.
3. **Compose** (`build::construction_macrocell`). Tile `(col, row)` is OR-ed
   with its nominal origin at `(2048·col, 2048·row)` (method A of the research
   note). Because tiles overlap by 5 cells, the content of every 2048×2048
   square is a function of the 3×3 block of macro cells around it, so each
   **distinct neighbourhood class is turned into a quadtree once** (from a
   dense 256×256-leaf bitmap), equal squares and sub-squares are hash-consed,
   and the grid becomes a small DAG written as a Golly macrocell document,
   which `hashlife::macrocell::read` interns directly. No cell is inserted
   individually. A 64×64 random grid (4 096 tiles, 183 605 330 live cells) needs
   30 distinct squares, 6 174 document nodes and a 133 KB document; it is
   built and loaded in about 10 ms (benchmark below).
4. **Frame.** Golly's macrocell origin places the root's south-east child at
   `(0, 1)`; the document is therefore written one row up, so that after
   reading, every tile sits on its nominal origin and on exactly one level-11
   node of the engine's quadtree (what makes Hashlife's memo so effective).
5. **Edits** rebuild at generation 0 (interning into the existing store, so
   memoised futures of unchanged squares are reused).

Golly's demos use method B (trimmed 2048×2048 tiles plus tubs for existing
diagonal neighbours). The test suite checks that our construction of the
galaxy pattern equals Golly's file tile for tile, except for the four corner
cells of every tile that method B trims.

## 3. Exact construction vs accelerated macro model

| | Exact construction | Accelerated macro representation |
|---|---|---|
| What runs | every cell of the B3/S23 pattern, via Hashlife | the macro rule applied to the macro grid |
| Unit | one physical generation | one metageneration |
| Role | the simulation | a **prediction**, never evidence |
| Where shown | `MacroTile::exact`, `GridView` | `MacroTile::accelerated`, labelled `ACCELERATED_LABEL` |

At every readout the exact states are compared with the prediction for the
same metageneration; differences (and ambiguous tiles) are listed in
`ReadoutRecord::mismatches` and counted in the `mismatches` and
`total_mismatches` metrics. Both labels (`EXACT_LABEL`,
`ACCELERATED_LABEL`) travel with every `MacroView` so the interface always
says which one it displays.

## 4. Timing

* **First cycle** `[0, P)`: fuses burn and first signals are exchanged; no
  display changes. Metagen 0 stays displayed until `t ≈ P + 4320`.
* **Transitions** (cycles `k ≥ 1`, `t mod P`): an ON→OFF core leaves its
  steady state at **4320** and is empty from **7362**; an OFF→ON core gets
  its first cell at **4366** and is steady from **7408**. Steady ON cores
  repeat with period 46 (21 677 … 29 795 cells).
* **Displayed metagen** `m(t) = max(0, ⌊(t − 7408)/P⌋)` outside transitions;
  `PhaseInfo` reports it with `t mod P`, the stage and the half of the cycle.
* **Readout**: metagen `n` at `t = (n + 1)·P`. Whatever the step size,
  evolution pauses exactly there (`MetapixelSim::run`), reads all tiles,
  records the result and then completes the tick, so a tick ends on a
  multiple of P only when its step lands there (with 1024 generations per
  tick, the tick that reads at 35 328 ends at 35 840).
* **Finite grids**: outside the grid is permanently OFF (never sends, never
  born); no debris escapes; the bounding box is constant.

## 5. Gzip and hashes

Golly ships its demos as `.mc.gz`. `metapixel::gzip` is a small DEFLATE
decoder written from RFC 1951/1952 (stored, fixed and dynamic Huffman
blocks, CRC-32 and length checked, output capped), so no compression crate
is needed. The golden SHA-256 hashes of the research note are checked with
a FIPS 180-4 implementation in `tests/common`.

## 6. Performance

Hardware: Apple M2 Max (8 performance + 4 efficiency cores), 64 GB,
macOS; rustc 1.98.1, `bench` profile (release, thin LTO, 4 codegen units);
Criterion 0.8, 10 samples (`cargo bench -p metapixel`). Other agents were
working on the same machine during both runs: the implementation run
(load average 4–7) and the adversarial review run of 2026-11-10 (load
average 16–35), which also added `micro_view_galaxy_1024`.

| Benchmark | What it does | Review run: median [95 % CI] | Implementation run |
|---|---|---|---|
| `program_tiles_b3s23` | program both tiles | 1.27 ms [1.26, 1.27] | 1.28 ms |
| `build_64x64_random` | build a 64×64 construction and read it into Hashlife | 10.8 ms [10.7, 10.8] | 10.2 ms |
| `blinker_3x3_one_metagen_cold` | build a 3×3 blinker, advance 35 328 generations with an empty memo, read 9 tiles | 79.0 ms [77.0, 83.2] | 76.3 ms |
| `readout_galaxy_15x15` | count the 225 readout cores of the galaxy at `t = P` | 0.977 ms [0.959, 1.008] (≈ 4.3 µs per tile) | 1.01 ms |
| `micro_view_galaxy_1024` | cell texture of the whole galaxy construction (30 730² cells) in a 1024² budget: 32 cells per texel, 962×962 texels | 6.21 ms [4.97, 7.05] | — |
| `accelerated_step_256x256` | one accelerated step of a 256×256 grid | 1.16 ms [1.12, 1.21] | 0.683 ms |

For comparison, the research note measured Golly's HashLife (`bgolly`) at
0.10 s for the first metagen of a 3×3 grid with a cold cache; our cold
3×3 metagen including construction and readout takes 76–79 ms. With a warm
memo the tiling is periodic and later metagens cost far less: the 9-metagen
galaxy test (225 tiles, Golly's file *and* our construction side by side)
takes 0.34–0.46 s, and the 4 512-tile p216 demo runs 4 metagens in
0.53–0.64 s (two runs under different load).

**Scaling to the largest grids** (single runs during the review, `verify`
profile, a throw-away probe test that is not kept; random soups of density
0.4 under B3/S23, load average ≈ 14; times per metagen include the readout
of every tile and the comparison with the accelerated model):

| Grid | Tiles / live cells at t = 0 | Build | Metagen 0, 1 (, 2) | Hashlife arena after the run | Mismatches |
|---|---|---|---|---|---|
| 64×64 | 4 096 | 17 ms | 178, 69, 62 ms | 0.99 M nodes, 48 MB | 0 |
| 128×128 | 16 384 | not timed | 263, 206 ms | 1.23 M nodes, 96 MB | 0 |
| 256×256 | 65 536 / 2 667 659 838 | 18 ms (30 distinct squares, 11 103 document nodes, 248 KB document) | 635, 686 ms (of which ≈ 380 ms is the readout of 65 536 cores) | 2.19 M nodes, 192 MB, no garbage collection at the default node limit; peak RSS of the whole test process 390 MB | 0 |

For the 64×64 soup, a full-grid micro view (budget 1920×1080 texels →
128 cells per texel, 1026×1026 texels) took 9.7 ms, a snapshot 2.8 ms
(150 KB payload) and a restore 2.5 ms. A restore starts from a fresh
Hashlife store (the memo is not part of a snapshot), so the next metagen
took 210 ms instead of ≈ 65 ms; the result was again mismatch-free.

## 7. Verification

`cargo test -p metapixel` (normal suite, debug profile: 31 unit tests, 28
integration tests including 7 exact runs, 1 doctest; 6.7 s wall time
including the incremental build) and
`cargo test -p metapixel --profile verify -- --ignored` (4 long runs; the
test binary finishes in 4.1 s because the runs execute in parallel). Times
are wall-clock times printed by the tests (`--nocapture`) during the
adversarial review run of 2026-11-10 on the machine above while other agents
were working (load average 14–60); tests of one binary run concurrently, so
each time includes contention from its neighbours.

| Test | Checks | Time |
|---|---|---|
| `tiles::*` | populations 23 920 / 66 203, bounding box, 28 overlap cells; B3/S23 programming equals the shipped `*-B3S23.rle` tiles (23 923 / 66 206) and adds exactly (148,1470), (162,1472), (162,1498); HighLife adds B3, B6, S2, S3; all rule-bit and fuse coordinates equal `otca-spec.json`; von Neumann removes 36 cells; clear errors for missing/programmed/invalid tiles | 0.05 s |
| `construction::generation_zero_matches_the_golden_or_tilings` | 3×3 blinker 342 076 cells, 1×1 66 206, 2×2 264 796, with the **bgolly SHA-256 hashes**; bounding boxes | 0.16 s (suite) |
| `construction::every_tile_is_the_programmed_tile_or_ed_at_its_nominal_origin` | a 3×4 grid with HighLife and a custom neighbourhood equals the OR of the shifted tiles, cell for cell | |
| `construction::golly_galaxy_demo_is_our_construction_plus_method_b_trimming` | Golly's galaxy decompressed by our gzip; geometry, programming (B3/S23, Moore), population 7 408 195; its generation-0 macro pattern is our `galaxy` scenario; per-tile populations differ by exactly the 4 trimmed corner cells; 8 tiles compared cell for cell | |
| `construction::large_grids_share_squares_instead_of_inserting_cells` | 64×64 random grid: ≤ 34 distinct squares | |
| `exact::golden_table_of_the_research_note_is_reproduced` | blinker, single tile and block at `t = kP`, k = 1…8: **every population and SHA-256 of the bgolly golden table**; blinker readouts alternate horizontal/vertical (period 4P full state); lone tile dies; block cores 29 795 | 1.27 s |
| `exact::life_rules_hold_for_metacells` | no birth with 1 or 2 ON neighbours, birth with exactly 3, survival with 2 and 3, death by overpopulation (4), over 2 metagens each | 0.71 s |
| `exact::metacells_change_only_through_heard_neighbours` | rule B1/S: one ON tile, Moore / deaf to NW / von Neumann / deaf to N and W — births exactly where the tile is heard | 0.57 s |
| `exact::macro_glider_moves_one_metacell_diagonally_every_four_metagens` | 10×10 glider, 12 metagens: translated by (k, k) at metagen 4k | 0.21 s |
| `exact::random_soups_match_the_accelerated_model` | 4 seeded 6×6 soups × 5 metagens: zero mismatches, no ambiguity | 0.78 s |
| `exact::other_rules_b0_and_finite_borders` | B0/S single tile blinks; 1×3 `OOO` → `.O.` → `...` with constant bounding box; HighLife B6 birth | 0.40 s |
| `exact::golly_galaxy_demo_follows_life_and_matches_our_construction` | Golly's file **and** our construction: 9 metagens, all readouts equal Life on the 15×15 dead-boundary grid; metagen 8 = metagen 0 | 0.46 s |
| `exact::every_rule_bit_is_programmed_where_the_spec_says` (ignored) | 18 digits × {in rule, not in rule}: centre of a 3×3 grid with *d* ON neighbours | 4.06 s |
| `exact::golly_parity_demo_follows_the_von_neumann_parity_rule` (ignored) | 64×64 B135/S135 von Neumann demo (detected from the tiles), 5 metagens | 0.34 s |
| `exact::golly_p216_demo_follows_its_rule_with_missing_tiles_never_born` (ignored) | sparse 4 512-tile B35/S236 demo (128 116 349 cells), 4 metagens; missing tiles never ON | 0.64 s |
| `exact::transition_window_timing_matches_the_research_note` (ignored) | per-generation core counts for `t mod P ∈ [4250, 7500]`: dying cores leave their period-46 sequence at exactly **4320** and are empty from **7362**; born cores get their first cell at **4366** and are steady from **7408**; 29 795 at `t ≡ 0 (mod 46)` | 1.70 s |
| `sim::*` | descriptor and metrics, apply policies and pending values (integer `step` overrides from the command line accepted; the physical rule stays B3/S23 whatever the macro rule), readouts at multiples of P, commands rebuilding at generation 0 (errors change nothing), grid-view contract (budget, power-of-two LOD, snapped origins, densities equal to Hashlife's block counts, exact texels), macro view labels, inspection, snapshot/restore replay including the RNG, scenarios (files equal embedded copies, RON round trip, reset restores the generator) | 0.23 s |

Every exact readout so far had cores of exactly 0 or 29 795 cells; no tile
was ever ambiguous and no prediction ever mismatched.

## 8. Licences

* **Pattern data** (`assets/patterns/otca/`): Brice Due's tiles and Golly's
  three metapixel demos are third-party data redistributed under
  GPL-2.0-or-later; see `assets/patterns/otca/PROVENANCE.md`. They are
  **loaded at run time and never embedded** (`include_str!` is used only for
  our own scenario files), so binaries built from this crate do not carry
  them.
* **Code**: written from the specification in the research note under the
  project licence (MIT OR Apache-2.0). No GPL code (Golly's C++,
  `metafier.lua/py`, `tile-otcamp.pl`) was copied or translated. The gzip
  decoder and the test SHA-256 are our own implementations of the RFCs and
  FIPS 180-4.
* `otca-spec.json` (numbers and coordinates) is project data.

## 9. Open items

* The internal mechanism narrative of §1 is an interpretation (UNVERIFIED).
* The start of the visible transition outside the readout core is not
  isolated (roughly ≤ 3000 generations into a cycle).
* Tiles inserted into a running construction are untested; edits rebuild
  from generation 0 instead.
* Hashlife memory for long chaotic runs of the largest grids. The 256×256
  soup above used 2.2 M nodes after two metagens, well below the default
  limit of 8.4 M; how the memo grows over hundreds of metagens of such a
  grid was not measured. If collections become frequent (`node_count`
  near `node_limit`), raise `node_limit`.
