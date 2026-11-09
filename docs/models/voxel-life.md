# voxel-life — Three-Dimensional Life

`crates/voxel-life` implements the **Three-Dimensional Life** mode:
outer-totalistic cellular automata on the cubic lattice, with Generations-style
decay states. It covers Carter Bays' 3D Life rules (4555, 5766) and the
multi-state rules popularised by Softology's *Visions of Chaos* (`4/4/5/M`,
Clouds, Amoeba, Builder…). The crate depends on `simulation-core`, `serde`,
`rayon`, `thiserror` and `ron`, all of them workspace dependencies. It adds no
new third-party crate.

## Contents

1. [Public API](#1-public-api)
2. [The model](#2-the-model)
3. [Rule notations](#3-rule-notations)
4. [Engines](#4-engines)
5. [Initial states](#5-initial-states)
6. [Parameters, commands and metrics](#6-parameters-commands-and-metrics)
7. [Views](#7-views)
8. [Snapshots and determinism](#8-snapshots-and-determinism)
9. [Scenarios](#9-scenarios)
10. [Approximations and limitations](#10-approximations-and-limitations)
11. [How it was verified](#11-how-it-was-verified)
12. [Performance](#12-performance)
13. [References](#13-references)

---

## 1. Public API

| Item | Purpose |
|---|---|
| `Rule3D` | Canonical rule: survival set, birth set (`CountSet`, bit *n* = count *n*), number of states `C` (2–256), `Neighbourhood` (`Moore`, `VonNeumann`). `parse`, `to_softology` (also `Display`), `to_letters`, `to_bays`, `to_golly`, `next_state`, `bays_4555()`, `bays_5766()`. Serialises as its canonical Softology string. |
| `RuleError` | Typed parse errors (`Empty`, `UnknownNotation`, `Field`, `CountOutOfRange`, `BirthOnZero`, `States`, `Neighbourhood`). |
| `VoxelLife` | The `simulation_core::Simulation` implementation (model id `voxel.life`, `SCHEMA_VERSION` 1). Built with `VoxelLife::new(Scenario)`. Accessors: `rule`, `config` (current world), `pending_config` (used by the next reset), `bounds`, `stats`, `state_at`, `voxel`, `voxels` (sorted z, y, x), `halt_reason`. |
| `VoxelCommand` | `SetVoxel`, `ClearVoxel`, `SetVoxels`, `FillBox`, `Clear`, `Randomize { seed: Option<u64> }`. |
| `VoxelViewRequest` / `VoxelView` | `Surface { range: SliceRange, max_voxels }` → `SurfaceView`; `Slice { axis, layer, window }` → `SliceView` (a `Grid2<u8>`); `Counts` → `CountsView`. `SurfaceVoxel { pos, state, age, faces }` with face bits in `faces::{NEG_X, POS_X, …}`. |
| `WorldKind` | `DenseBounded`, `DenseTorus`, `SparseBounded`, `SparseUnbounded`. |
| `WorldConfig`, `InitSpec`, `InitKind`, `Mirror`, `PatternVoxel` | World size and kind, initial state, seed. |
| `Stats` | Counts per state, births, deaths, bounding box, centroid. |
| `scenarios` | `parse(text, name)`, `load_file(path)`, `load_dir(dir)`, `builtin()`, `builtin_by_id(id)`; `Scenario`, `Evidence`. |
| Limits | `MAX_SIDE` = 65 536, `MAX_DENSE_VOXELS` = 2²⁴, `MAX_RANDOM_VOXELS` = 2²⁴, `MAX_SPARSE_CHUNKS` = 32 768, `COORD_LIMIT` = 2³⁰, `CHUNK_SIDE` = 16, `MAX_SLICE_SIDE` = 4096, `DEFAULT_MAX_SURFACE_VOXELS` = 2 000 000. |

A desktop runner can wire the generic session controls like this:

```rust
Runner::spawn(life, RunnerConfig::default())
    .with_clear(|s| { let _ = s.apply(VoxelCommand::Clear); })
    .with_randomize(|s, seed| { let _ = s.apply(VoxelCommand::Randomize { seed: Some(seed) }); });
```

`Clear` cannot fail. `Randomize` fails only when the random cube would cover
more than 2²⁴ voxels or, in a sparse world, more than `MAX_SPARSE_CHUNKS`
chunks.

## 2. The model

**Lattice and states.** Voxels sit at integer positions `(x, y, z)`. A voxel
has a state in `0 … C−1`. State 0 is empty, state 1 is alive, and states
`2 … C−1` are dying (refractory). With `C = 2` the rule is an ordinary
two-state rule.

**Neighbourhoods.**

| Name | Size | Offsets |
|---|---|---|
| Moore (`M`) | 26 | all `(dx, dy, dz) ∈ {−1, 0, 1}³ \ {0}` |
| von Neumann (`N`) | 6 | exactly one non-zero coordinate, ±1 |

**Transition.** Let `N(v, t)` be the number of neighbours of voxel `v` that
are in state 1 at generation `t`. Dying voxels are **not** counted. Every
voxel updates simultaneously:

```text
s(v, t+1) = 1                       if s = 0 and N ∈ B          (birth)
          = 0                       if s = 0 and N ∉ B
          = 1                       if s = 1 and N ∈ S          (survival)
          = 2 if C > 2, else 0      if s = 1 and N ∉ S          (death)
          = s + 1                   if 2 ≤ s and s + 1 < C      (decay)
          = 0                       if s = C − 1 ≥ 2
```

This is Golly's Generations algorithm (`generationsalgo.cpp`, `slowcalc`) and
also what Softology describes: only newly born cells count, dying cells keep
fading, and only fully empty cells can be born. A dying voxel cannot be reborn
before it reaches 0. Birth on 0 neighbours (B0) is rejected, so empty space
stays empty. Golly's 3D rules reject B0 too.

**Edges.**

* *Bounded* (dense or sparse): the world is the box `[0, W) × [0, H) × [0, D)`.
  Voxels outside it are permanently empty.
* *Torus* (dense): neighbour offsets are added modulo the size of each axis.
  This is the standard CA on `Z_W × Z_H × Z_D`. On an axis shorter than 3,
  the same voxel is reached through several offsets and is counted once per
  offset. For example, on a torus 1 voxel deep, the offsets `dz = −1` and
  `dz = +1` both reach the voxel's own plane.
* *Unbounded* (sparse): coordinates may lie anywhere in `[−2³⁰, 2³⁰)` on each
  axis. Voxels beyond that are never born. The world size only places the
  initial state.

**Age** (display only). Age is the number of generations since a voxel was
born or placed by a command. It is 0 at birth and increases by 1 for every
generation the voxel stays non-empty, including while it decays. It
saturates at 65 535 and returns to 0 when the voxel becomes empty. Age has no
effect on the dynamics.

## 3. Rule notations

`Rule3D::parse` accepts four notations, chosen by the shape of the text:

| Notation | Form | Example | Notes |
|---|---|---|---|
| Softology | `S/B/C/N`, exactly four fields | `13-26/13-14,17-19/2/M` | Lists are comma separated with `a-b` ranges. `C` is the number of states. `N` is `M` or `N`. |
| Letters | `B…/S…` in either order, plus optional `C<n>`, `G<n>` or bare `<n>` (number of states, as in Golly's `B2/S/3`) and `M`/`N` fields | `B5/S45`, `B4/S4/C5`, `B4/S4/5`, `B13/S12/C5/N` | A list without `,`, `-` or `.` is read digit by digit (`S45` = {4, 5}). Otherwise it is itemised (`S13-26`, `B5,10`). |
| Bays | `E_l E_u F_l F_u` | `5766`, `(10 21 10 21)` | Two states, Moore. Survive iff `E_l ≤ N ≤ E_u`, born iff `F_l ≤ N ≤ F_u`. |
| Golly 3D.lua | `3D<S>/<B>[F|V]` | `3D5..7/6`, `3D4,5/5`, `3D0..6/1,3F` | Survival first, `a..b` ranges, `F`/`V` = face (von Neumann). Two states. |

Itemised lists accept both `a-b` and `a..b`. Whitespace around fields is
ignored.

**Printing.** `Display` and `to_softology` give the canonical Softology form:
singletons as `a`, runs as `a-b`, for example `5-7/6/2/M`. `to_letters` gives
`B6/S567`. A lone count of 10 or more is written `a-a`, so it cannot be read
back as separate digits. `to_bays` exists only for two-state Moore rules
whose sets are single intervals (`5766`). `to_golly` exists only for
two-state rules; runs of exactly two are printed `a,b`, as Golly does
(`3D4,5/5`).

**Errors** name the faulty field. Examples:

* `birth list: count 27 is impossible in the 26-voxel Moore neighbourhood`
* `survival list: range `7-5` is reversed`
* `birth on 0 neighbours is not supported…`
* `the number of states must be an integer from 2 to 256, got `1``
* `Golly 3D rule: suffix `H` (corner, edge or hexahedral neighbourhood) is not supported…`

Unrecognised text such as the 2D rule `23/3` lists the accepted forms.

## 4. Engines

### 4.1 Dense grid (`DenseBounded`, `DenseTorus`)

The dense grid stores `u8` states in layout `index = (z·H + y)·W + x`, with a
second `u8` back buffer, a `u16` age buffer and a `u8` buffer of plane sums.
A dense world holds at most 2²⁴ voxels (256³), about 80 MiB. One generation
takes two parallel passes. Each pass is a pure function of read-only buffers,
run with rayon over z-planes:

1. **Moore only.** For every plane, compute the 3×3 box sum of alive voxels
   separably: first `x−1 + x + x+1`, then the same sum along y. Edges are
   zero-padded (bounded) or wrapped (torus).
2. For every output plane `z`:
   * the Moore count is `P(z−1) + P(z) + P(z+1) − alive(v)`, where `P` is
     the plane sum from pass 1 (empty planes are used outside a bounded world);
   * the von Neumann count is the sum of the six face neighbours.

   Pass 2 then applies precomputed lookup tables (`B`/`S` for 27 counts) into
   the back buffer. It updates the age in place, because an age depends only
   on the voxel's own old and new state. It also accumulates per-plane
   statistics.

The buffers are then swapped, and the per-plane statistics are summed in plane
order.

### 4.2 Sparse chunks (`SparseBounded`, `SparseUnbounded`)

The sparse engine keeps a `HashMap` of 16³ chunks. Each chunk holds `u8`
states and `u16` ages. Only chunks with at least one non-empty voxel are
stored, and a sorted key list (by z, then y, then x) is kept beside the map.
One generation:

1. **Candidates.** Every stored chunk is a candidate. Chunks holding an alive
   voxel also add their 26 neighbour chunks that intersect the world box.
   Births need an alive neighbour (B0 is rejected), so no other chunk can
   change. The candidate list is sorted and deduplicated.
2. **Limit.** If there are more than `MAX_SPARSE_CHUNKS` (32 768) candidates,
   the world is left unchanged and the model halts. `is_finished()` becomes
   true and `halt_reason()` explains why. Any successful command or a reset
   clears the halt. Restoring a snapshot restores the halt state it was
   taken in.
3. **Parallel evaluation.** Candidates are processed with rayon, each as a
   pure function of the read-only map:
   * gather an 18³ block of alive bits from the chunk and its neighbours;
   * count separably (Moore) or directly (von Neumann);
   * apply the rule, leaving voxels outside the world box empty.
4. **Rebuild.** The new map and key list are built from the results in
   candidate order. Empty chunks are dropped.

The same chunk limit applies to everything else that stores chunks: a
command (`SetVoxel`, `SetVoxels`, `FillBox`, `Randomize`) that would make the
world store more than `MAX_SPARSE_CHUNKS` chunks is rejected before anything
changes, and a sparse world's random cube or pattern must fit in that many
chunks. Without this check a single `SetVoxels` of scattered voxels could
allocate 12 KiB per voxel.

Hash-map iteration order is never used. Lookups do not depend on it, and all
iteration goes through the sorted key list.

## 5. Initial states

The world centre is `c = ⌊size / 2⌋` on each axis.

| `InitKind` | Contents |
|---|---|
| `random_cube` | A cube of side `s = min(cube_side, size)` per axis, starting at `c − ⌊s/2⌋`, so it always fits a finite world. Each voxel is alive with probability `density`, using one `chance(density)` draw per voxel in z, y, x order. |
| `single_voxel` | One alive voxel at `c`. |
| `pattern` | The scenario's `(dx, dy, dz, state)` list, placed at `c + d`. Voxels outside the world, or in states the current rule lacks, are skipped. |
| `empty` | Nothing. |

**Mirror symmetry** (`none`, `x`, `y`, `z`, `xy`, `xz`, `yz`, `xyz`). Along each
mirrored axis, only the first `⌈s/2⌉` layers of the cube are drawn. Each drawn
voxel is also placed at its reflection `s − 1 − l` in cube coordinates. The
result is symmetric about the cube's central plane(s).

**Random streams.** The initial state uses `SimRng::derive(seed, 1)`.
Seedless `Randomize` commands use the model's generator, which starts as
`SimRng::derive(seed, 2)`, is stored in the state and is included in
snapshots. `Randomize { seed: Some(k) }` uses a fresh `SimRng::seed_from_u64(k)`.

## 6. Parameters, commands and metrics

### 6.1 Parameters

Defaults are the scenario's values. World and initial-state parameters are
stored in a **pending** configuration and only take effect at the next reset.
Each change is validated against the whole pending configuration, which is
therefore always valid. `params()` reports the pending values.

| Key | Kind | Policy | Meaning |
|---|---|---|---|
| `rule` | text (≤ 256) | NextTick | Any notation from §3. Voxels in states the new rule lacks are emptied. |
| `world` | choice | RequiresReset | `dense_bounded`, `dense_torus`, `sparse_bounded`, `sparse_unbounded` |
| `size_x`, `size_y`, `size_z` | int 1–65 536 | RequiresReset | World size. A dense world holds at most 2²⁴ voxels. |
| `init` | choice | RequiresReset | `random_cube`, `single_voxel`, `pattern` (offered only when the scenario has one), `empty` |
| `cube_side` | int 1–65 536 | RequiresReset | Random-cube side. Also used by `Randomize`. The cube may cover at most 2²⁴ voxels. |
| `density` | float 0–1 | RequiresReset | Probability that a random-cube voxel is alive |
| `mirror` | choice | RequiresReset | Symmetry of the random cube |
| `seed` | int 0–2⁶³−1 | RequiresReset | Seed of the initial state and of seedless `Randomize` |

`reset()` rebuilds the world from the pending configuration and the current
rule. The scenario's initial state therefore follows any parameter changes.

### 6.2 Commands

Every command is fully validated before anything changes. Edited voxels get
age 0. Statistics are recounted, and the births and deaths of the last
generation are kept.

| Command | Effect | Errors |
|---|---|---|
| `SetVoxel { pos, state }` | Set one voxel (state 0 empties it) | Position outside the world; state ≥ C; sparse chunk limit |
| `ClearVoxel { pos }` | Empty one voxel | Position outside the world |
| `SetVoxels(list)` | Set many voxels, all or nothing | As above, for any entry |
| `FillBox { min, max, state }` | Fill the inclusive box (corners in any order), clipped to the world | No overlap with the world; more than 2²⁴ voxels; sparse chunk limit |
| `Clear` | Empty the world | — |
| `Randomize { seed }` | Empty the world, then draw a random cube (pending `cube_side`, `density`, `mirror`, in the current world) | Cube larger than 2²⁴ voxels; sparse chunk limit |

The sparse chunk limit is `MAX_SPARSE_CHUNKS` stored chunks (§4.2). Emptying
voxels never needs a new chunk, so it is never refused.

### 6.3 Metrics

| Key | Kind | Meaning |
|---|---|---|
| `population` | gauge | Non-empty voxels |
| `state_1` … `state_{C−1}` | gauge | Voxels per state (`state_1` = alive) |
| `births` | counter | Empty voxels that became alive in the last generation |
| `deaths` | counter | Alive voxels that did not survive the last generation (they became empty, or entered state 2) |
| `bbox_min_x`, `bbox_max_x`, … `bbox_max_z` | gauge | Bounding box of the non-empty voxels |
| `centroid_x`, `centroid_y`, `centroid_z` | gauge | Mean coordinate of the non-empty voxels |

The bounding-box and centroid metrics are absent when the world is empty. The
centroid is the arithmetic mean of the coordinates, even on a torus.

## 7. Views

**Surface** (`VoxelViewRequest::Surface { range, max_voxels }`, the default).
`range` is a `SliceRange { axis, min, max }` with inclusive bounds;
`SliceRange::all()` shows everything. A non-empty voxel whose coordinate on
the range axis lies in `[min, max]` is emitted iff at least one of its six
faces is exposed. A face is exposed when the voxel across it:

* is empty;
* lies outside the world box (on a torus too, because the renderer draws the
  box and not its periodic images); or
* is sliced away, meaning its coordinate on the range axis is outside
  `[min, max]`.

Fully enclosed voxels are omitted. A solid 5³ cube shows its 98 shell voxels,
for example. Each `SurfaceVoxel` carries `pos`, `state`, `age` and a `faces`
bit mask of the exposed faces, so a renderer can skip hidden faces. The list
is capped at `max_voxels`; `visible` gives the total and `truncated` says
whether the cap applied. The order is deterministic: z, y, x for dense
worlds, chunk by chunk for sparse ones.

The runner builds a view up to 60 times per second on the simulation thread,
so the surface is computed without per-voxel hash lookups. Dense worlds read
neighbours by index arithmetic. Sparse worlds read them inside the chunk,
and look up the six face-adjacent chunks once per chunk. Planes or chunks
are processed in parallel. A test compares the result with the definition
above, evaluated voxel by voxel, on all four world kinds and on random slice
ranges along every axis.

**Slice** (`Slice { axis, layer, window }`). One layer as a `Grid2<u8>` of
states. The in-plane axes `(u, v)` are `(y, z)` for x, `(x, z)` for y and
`(x, y)` for z. Cell `(i, j)` is at `u = u0 + i`, `v = v0 + j`. When `window`
is `None`, the region is chosen as follows:

* dense worlds: the whole world face;
* sparse worlds: the initialisation box joined with the population's bounding
  box, clipped to the world.

Sides are capped at 4096 and the view reports `truncated`. Sparse slices copy
only the stored chunks that cross the layer.

**Counts** (`Counts`). `counts[s]` is the number of voxels in state `s`.
`counts[0]` is the number of empty voxels of a finite world. `empty` is
`None` for the unbounded world.

## 8. Snapshots and determinism

A snapshot is a `postcard` payload (schema version 1) containing:

* the scenario;
* the rule, as its canonical string;
* the current and pending configurations;
* the voxels, run-length encoded: `(value, run)` pairs over the dense layout,
  or per stored chunk in sorted order;
* the tick, the RNG state, the last births and deaths, and any halt reason.

`restore` validates every invariant before changing anything: configurations,
buffer lengths, states below `C`, chunk order, chunk keys inside the world's
chunk range (checked before any coordinate arithmetic, so extreme keys cannot
overflow), voxels inside the world and a non-zero generator state. A corrupt
or hostile payload gives an error, never a panic.
Restoring a snapshot replays the run identically, including later seedless
`Randomize` commands.

All randomness comes from `SimRng`. Parallel passes only read the current
state and write disjoint outputs. Statistics are integer sums combined in a
fixed order. Runs are therefore bit-for-bit reproducible, with no floating
point in the dynamics.

## 9. Scenarios

Scenario files live in `scenarios/voxel/*.ron` and have the header fields
`id`, `name`, `description`, `seed`, `rule`, `world`, `size`, `init`,
`evidence` and `reference`. The same files are embedded with `include_str!`
(`scenarios::builtin()`), and a test checks that the embedded copies equal
`load_dir("scenarios/voxel")`. `evidence` labels how well the described
behaviour is established:

* `tested` — asserted by this crate's tests;
* `documented` — stated in a primary source and not reproduced here;
* `observed` — qualitative and seed-specific, from the research simulation.

| id | Rule | World | Initial state | Evidence |
|---|---|---|---|---|
| `life-5766-glider` | Bays 5766 | torus 24×24×8 | 10-voxel glider | tested: period 4, (1, 1, 0), 10 voxels per phase, back home after 96 generations |
| `life-4555-glider` | Bays 4555 | torus 24×24×12 | 10-voxel glider (research §7.3) | tested: period 4, (−1, −1, 0), 10 voxels per phase |
| `rokicki-ship` | `3D4,7/5,8` | torus 60×16×16 | 14-voxel ship | tested: period 10, (+10, 0, 0); phase populations 14, 16, 18, 20, 20, 14, 16, 18, 20, 20 (measured here) |
| `life-in-5766-two-planes` | Bays 5766 | bounded 96×96×2 | z-mirrored soup | tested: equals 2D B3/S23 in both planes (see below) |
| `life-5766-soup` | Bays 5766 | bounded 64³ | 16³ soup, 30%, seed 38 | documented: Bays reports bounded growth, settling after about 30–70 generations; this seed settles at generation 29 into 35 still-life voxels |
| `crystal-growth-1` | `0-6/1,3/2/N` | sparse unbounded | single voxel | tested: population vector |
| `single-point-replication` | `/1/2/M` | sparse unbounded | single voxel | tested: 8 voxels at (±2ᵏ)³ after 2ᵏ generations, k = 1…5 |
| `softology-445` | `4/4/5/M` | bounded 64³ | 6³ soup, 50% | observed |
| `clouds-1` | `13-26/13-14,17-19/2/M` | bounded 64³ | full volume, 60% | observed |
| `softology-amoeba` | `9-26/5-7,12-13,15/5/M` | bounded 64³ | 12³ soup, 50% | observed |
| `builder-1` | `2,6,9/4,6,8-9/10/M` | bounded 64³ | 12³ soup, 50% | observed |
| `softology-pyroclastic` | `4-7/6-8/10/M` | bounded 64³ | 12³ soup, 50% | observed |

The "observed" descriptions quote the research runs. Those runs used a
different random generator (numpy, seed 42), so our soups differ and the
quoted generation numbers are not expected to repeat. Each description also
states what the preset does with its own seed in this build, and
`tests/presets.rs` asserts those statements, so a preset cannot silently stop
showing what its description promises. Populations at generations 0, 10, 30,
60, 100 and 150:

| Scenario | Populations | Asserted by `tests/presets.rs` |
|---|---|---|
| `clouds-1` | 157 172, 200 280, 199 841, 199 385, 199 592, 199 593 | 75–77% of the volume at generation 60 (76% in the research runs too) |
| `builder-1` | 826, 7 587, 17 489, 23 761, 20 150, 14 471 | peak above 23 000 between generations 50 and 70, then 14 471 at 150 |
| `softology-amoeba` | 841, 4 121, 14 465, 66 850, 193 358, 218 591 | reaches the boundary at generation 47 (research: 50–83), more than 200 000 at 150 |
| `softology-pyroclastic` | 846, 3 322, 8 126, 30 920, 73 720, 78 823 | reaches the boundary at generation 54 (research: 50–83) |
| `softology-445` | 108, 134, 147, 0, 0, 0 | reaches the boundary at generation 29, extinct at 47 (research: 42 for a side-6 seed) |
| `life-5766-soup` | 1 174, 400, 35, 35, 35, 35 | stays inside the central 32³ box; still life of 35 voxels from generation 29 |

The 5766 soup originally used seed 5766, which dies out by generation 25.
Seeds 0–39 were then surveyed: 29 ended with a constant population of 7 to
39 voxels and 11 died out. Seed 38 was chosen because it shows the typical
outcome, a soup that settles into still lifes.

**Why the two-plane world is exactly Conway's Life.** Take a bounded world
two voxels deep whose planes are identical. A voxel with `n` live neighbours
in its own plane has `n + n + 1` live 3D neighbours if it is alive (its twin
counts) and `2n` if it is empty. Then:

* 5766 survival, `5 ≤ 2n + 1 ≤ 7`, holds iff `n ∈ {2, 3}`;
* 5766 birth, `2n = 6`, holds iff `n = 3`.

This is exactly B3/S23, and the planes stay identical. Bays' Theorem 7 states
the general condition for 5766 analogues of Conway objects.

## 10. Approximations and limitations

* **Neighbourhoods.** Only Moore-26 and von Neumann-6 are implemented, as
  `docs/ARCHITECTURE.md` specifies. The research note also describes edge-12,
  corner-8 and FCC-12 (Golly `E`, `C`, `H`). The parser rejects those suffixes
  with a clear message. The FCC p126 Christmas-tree test therefore does not
  apply.
* **B0 is rejected**, as in Golly's 3D rules. A background-phase engine would
  be needed for unbounded worlds.
* **Golly Busy Boxes** (`BB`, `BBW`) and the RLE3 file format are not
  supported.
* **The Softology "Expanding Shell" rule** is printed with a typo in the
  source (`…18.6-10…`). It is not shipped.
* **Small tori.** On an axis shorter than 3, a neighbour reached by several
  offsets is counted once per offset (§2). This is the standard
  `Z_n`-arithmetic definition, but it differs from the infinite lattice.
* **Unbounded is finite.** The unbounded world is limited to `[−2³⁰, 2³⁰)`
  per axis. More importantly, it halts when a generation would evaluate more
  than 32 768 chunks, about 134 M voxels of chunk capacity and about 400 MiB.
  The old and new chunk maps coexist during a step, so the peak is about
  twice that. A light-speed von Neumann crystal reaches the limit after a few
  hundred generations. Edits and initial states that would store more chunks
  are refused (§4.2).
* **Dense size.** Dense worlds are limited to 2²⁴ voxels (256³). Random cubes
  and `FillBox` commands are limited to 2²⁴ voxels per operation.
* **Display conventions.** Age saturates at 65 535. The centroid is an
  arithmetic mean, so it is not meaningful for patterns that wrap around a
  torus.
* **State labelling.** States follow Golly's convention: 1 = alive (newborn),
  2 … C−1 = dying. Softology's original post labels the 445 states in reverse
  (newborn = 4, counting down); the dynamics are identical. Display colours
  are the renderer's choice.

## 11. How it was verified

There are 68 tests: 26 unit tests, 41 integration tests and 1 doc test. Run
them with `cargo test -p voxel-life`.

* **Rule parsing** (`src/rule/tests.rs`):
  * all 25 Softology rules of the research catalogue round-trip exactly;
  * 20 000 random rules round-trip through every notation that can express
    them: Softology, letters, Golly and Bays. The rules cover every count set,
    2–256 states and both neighbourhoods;
  * equivalences between notations: `5766` = `(5 7 6 6)` = `3D5..7/6` =
    `5-7/6/2/M` = `B6/S567` = `S567/B6`, and likewise for 4555, 5655, 6855 and
    the von Neumann forms;
  * canonical printing in every notation;
  * 26 specific error cases.
* **Neighbour counts vs brute force** (`tests/engine.rs`). For every k, the
  rule `k/k/5/{M,N}` turns exactly the empty or alive voxels with k alive
  neighbours into state 1. The resulting sets are compared with a reference
  that counts neighbour offsets voxel by voxel. Soups include dying states,
  which must not be counted. The configurations are:
  * dense bounded and dense torus;
  * tori with axes of size 1, 2 and 3;
  * a sparse bounded world crossing chunk borders;
  * a sparse unbounded world at negative coordinates.
* **Whole generations vs brute force.** 9 rules × 7 configurations ×
  6 generations, compared voxel for voxel. The rules include multi-state
  rules, von Neumann rules and the 7-state "Shells".
* **Hand-built rule checks:**
  * a centre voxel with exactly k alive neighbours (k = 0…26), empty or alive,
    under `4,7/5,8,13,26/2/M`;
  * von Neumann ignores diagonal neighbours;
  * the decay chain 1 → 2 → 3 → 4 → 0;
  * dying neighbours are not counted;
  * a dying voxel is not reborn;
  * wrap-around across all three faces of a torus.
* **Engine agreement.** Dense and sparse engines give identical voxels, ages
  and statistics for 25 generations of a 10-state soup. Ages and the
  births/deaths/per-state counts match the reference.
* **Documented objects** (`tests/gliders.rs`):
  * the 5766 glider has period 4, moves (1, 1, 0), has 10 voxels in every
    phase and returns after 96 generations on the 24-torus;
  * the 4555 glider has period 4, moves (−1, −1, 0) and has 10 voxels in every
    phase;
  * the Rokicki ship has period 10 and moves (+10, 0, 0), including over
    500 generations on the sparse engine;
  * a glider crosses chunk and sign boundaries intact.

  The period checks also assert that no earlier phase is a translate of the
  start.
* **Softology test vectors** (`tests/vectors.rs`), on both the dense and the
  sparse engine. The research catalogue computed these with an independent
  numpy simulator, and our engines reproduce every number. Two independent
  implementations now agree, which is the catalogue's bar for golden vectors.
  * Crystal Growth 1 and 2, including state-1 counts;
  * von Neumann Builder;
  * Diamond Growth;
  * Single Point Replication, including 8 voxels at (±2ᵏ)³ for k = 1…5;
  * Builder 2;
  * Pulse Waves;
  * the lone 445 voxel;
  * 3D Brain from a 2×2×1 block.
* **Life inside 5766** (`tests/behaviour.rs`). Both planes equal an
  independent 2D B3/S23 implementation for 60 generations of a 48×48
  soup at 35%.
* **Determinism and snapshots:**
  * identical runs from identical scenarios;
  * a snapshot replay that includes a seedless `Randomize`;
  * sparse snapshots round-trip;
  * wrong model ids, newer schema versions and truncated payloads are rejected
    without changing the model;
  * crafted payloads are rejected with an error, never a panic: chunk keys at
    `i32::MAX` and `i32::MIN` or just outside the unbounded world, an all-zero
    generator state, and dense voxels in a sparse world.
* **Limits** (`tests/engine.rs`):
  * a sparse world halts at the chunk limit and leaves the world unchanged;
  * `SetVoxels`, `SetVoxel`, `FillBox` and `Randomize` are refused exactly
    when they would store more than `MAX_SPARSE_CHUNKS` chunks;
  * emptying voxels is never refused.
* **Surface view** (`tests/views.rs`). The view matches the surface
  definition, evaluated voxel by voxel, on dense bounded, dense torus, sparse
  bounded and sparse unbounded worlds. The unbounded case crosses chunk
  borders at negative coordinates. Slice ranges are random, along every axis,
  and include half-open ones.
* **Presets** (`tests/presets.rs`). Each "observed" preset, and the
  documented 5766 soup, shows the behaviour its description states (§9).
* **Parameters, commands and views:**
  * apply policies, pending-until-reset behaviour and dense and random-cube
    limits;
  * the conditional `pattern` option;
  * atomic `SetVoxels`, `FillBox` clipping and errors, and `Randomize`
    determinism;
  * surface extraction: interior voxels omitted, cut layers exposed, face
    masks, caps;
  * slices on every axis, with dense and sparse slices identical;
  * counts and the metric keys matching the descriptor.
* **Scenarios.** All 12 load, run, and match the folder. Invalid RON reports
  the file name and the faulty field: a bad rule, an unknown field, a pattern
  voxel outside the world or in a missing state.

Clippy runs with `-D warnings` on all targets, and `cargo fmt` is applied.

## 12. Performance

Criterion benchmark `benches/voxel.rs`; run it with `cargo bench -p voxel-life`.
In `voxel_step`, each iteration is one generation after 10 warm-up
generations. In `voxel_view`, each iteration builds the default surface view
(every visible voxel) of the Clouds 1 world after 10 generations.

The numbers below are real runs on an Apple M2 Max (macOS, release profile
with thin LTO), shown as the median with the 95% confidence interval. Many
other builds were running on the machine at the same time, so the numbers
vary between runs; treat them as upper bounds. The run on 2026-11-09 had load
averages between 17 and 58.

| Benchmark | World | First run | Run on 2026-11-09 |
|---|---|---|---|
| `voxel_step/clouds1_128_dense` | Clouds 1, 128³ bounded dense, 60% fill (≈ 1.74 M voxels, 83%, stable) | 3.13 ms (2.95–3.57) | 3.95 ms (3.40–4.57) |
| `voxel_step/clouds1_128_sparse` | the same world in bounded sparse chunks (all 512 chunks occupied) | 2.04 ms (2.01–2.06) | 5.66 ms (2.73–8.33) |
| `voxel_step/crystal2_128_dense_von_neumann` | Crystal Growth 2 (5 states, von Neumann), 128³ dense torus, 32³ soup at 30%, growing from 73 k to ≈ 1.5 M voxels during the run | 3.50 ms (3.18–4.08) | 4.52 ms (3.83–5.27) |
| `voxel_view/surface_clouds1_128_dense` | surface view of the dense Clouds 1 world (≈ 0.95 M visible voxels) | — | 7.96 ms (5.90–10.7) |
| `voxel_view/surface_clouds1_128_sparse` | the same view of the sparse world | — | 4.76 ms (4.26–5.51) |

The surface view used to look up every face neighbour through the hash map.
A release-mode timing of that version, on the same sparse world, took
24.6 ms per view, against a 2.2 ms step. Since the runner builds views up to
60 times per second on the simulation thread, the view then cost more time
than stepping. The current version reads neighbours by index (§7).

That is roughly 0.67 G voxel updates per second for the dense Moore engine
(first run).
In the first, steadier run, the sparse engine was faster on this fully
occupied world. Each 16³ chunk and its 18³ alive block stay in cache, while
the dense engine streams a full plane-sum buffer. The second run's sparse
interval is too wide, at 2.7–8.3 ms, to confirm or refute this. On mostly empty worlds the sparse engine's advantage
is larger, since empty chunks are never visited.

## 13. References

* C. Bays, "Candidates for the Game of Life in Three Dimensions", *Complex
  Systems* 1 (1987) 373–400 — 4555 and 5766, the definition of a 3D Life,
  Theorem 7 (5766 analogues of Conway objects), soups and gliders.
* C. Bays, "The Discovery of a New Glider for the Game of Three-Dimensional
  Life", *Complex Systems* 4 (1990) 599–602.
* C. Bays, "Further Notes on the Game of Three-Dimensional Life", *Complex
  Systems* 8 (1994) 67–73 — 5655 and 6855.
* J. Rampe, "3D Cellular Automata", Softology blog, 2019-12-28 — `S/B/C/N`
  notation, Generations semantics, rule list.
* Golly 5.1 `gollybase/generationsalgo.cpp` (Generations semantics) and
  `Scripts/Lua/3D.lua` (3D notation, the `3D4,7/5,8` ship by A. Trevorrow /
  T. Rokicki, 2018). Read for semantics only; no code was copied.
* `docs/research/automata-catalog.md` §3.1, §7 and §9 — semantics, notations,
  4555 glider coordinates and the Softology test vectors.
