# Hashlife engine (`crates/hashlife`)

`crates/hashlife` is a standalone, safe-Rust implementation of Gosper's
Hashlife for every two-state outer-totalistic Moore-neighbourhood rule without
`B0` (Conway's `B3/S23`, HighLife `B36/S23`, Seeds `B2/S`, Day & Night
`B3678/S34678`, ...). It depends only on `simulation-core`, `serde` and
`thiserror`. No `unsafe` and no third-party hashing or collection crates are
used. This page explains the algorithm, the data structures, memory
management, complexity, when Hashlife is the right engine, and measured
performance. The model-level page (parameters, rules text, how to test) is
[`docs/models/hashlife.md`](models/hashlife.md).

| Module | Contents |
|---|---|
| `store` | 32-byte `Node`, `u32`-indexed arena with free list, open-addressing intern table, canonical empty nodes, non-moving mark-and-sweep collector |
| `leaf` | 8×8 leaves as `u64`, bit-parallel (SWAR) base case for any outer-totalistic rule |
| `step` | the memoised recursion (full speed and slow steps) with GC pins |
| `universe` | public `Universe`: editing, `step_pow2`, `advance`, root management, GC API |
| `query` | cell iterator, exact bounding box, `cells_in_rect`, level-of-detail `density_grid`, exact `u128` populations of the plane and of rectangles (`population_in_rect`) |
| `macrocell` | Golly `.mc` reader and writer, with Golly's exact coordinates |
| `rule` | `Rule` (masks, `B3/S23` / `23/3` parsing, canonical display) |
| `sim` | `HashlifeSim`, the `simulation_core::Simulation` wrapper (`life.hashlife`) |

---

## 1. The algorithm

### 1.1 Quadtree and hash-consing

The plane is a quadtree. A node of **level L** is a square of side `2^L`:

```text
level L node                    children are level L-1, grandchildren level L-2
┌───────────┬───────────┐
│    nw     │    ne     │       leaves are level 3: an 8×8 bitmap in one u64
│           │           │       (bit y*8 + x, one byte per row, y downward)
├───────────┼───────────┤
│    sw     │    se     │
│           │           │
└───────────┴───────────┘
```

Nodes are immutable and **hash-consed**: before a node is created, the intern
table is searched for an existing node with the same `(level, nw, ne, sw, se)`
(or, for a leaf, the same bitmap). Every distinct square therefore exists
exactly once, and **index equality is content equality**. A pattern made of
many copies of the same structure, such as a glider stream, a gun array or a
metapixel tiling, costs memory proportional to the number of *distinct*
squares, not to its area. The empty square of every level is a single
canonical node, created once for levels 3 to 64.

### 1.2 The memoised result

For a node `n` of level `L ≥ 4`, define

```text
result(n, j) = the level-(L-1) centre of n after 2^j generations,   0 ≤ j ≤ L-2
```

It is well defined because of the **light cone**: in a Moore-neighbourhood
rule, information travels at most one cell per generation. After `2^j ≤ 2^(L-2)`
generations, the centre square (margin `2^(L-2)`) depends only on cells inside
`n`. Without `B0`, `result(empty_L, j) = empty_{L-1}`, a shortcut taken before
any lookup.

**Full speed** (`j = L-2`) recurses twice, each time advancing `2^(L-3)`
generations:

```text
 n (level L): 4×4 grandchildren g[r][c]        step 1: nine overlapping level-(L-1) squares
 ┌────┬────┬────┬────┐                           n_ij = join(g[i][j],   g[i][j+1],
 │g00 │g01 │g02 │g03 │                                       g[i+1][j], g[i+1][j+1])
 ├────┼────┼────┼────┤                           m_ij = result(n_ij)  (level L-2, +2^(L-3) gens)
 │g10 │g11 │g12 │g13 │                         step 2: four level-(L-1) squares of the 3×3 m grid
 ├────┼────┼────┼────┤                           q_ab = join(m_ab, m_a,b+1, m_a+1,b, m_a+1,b+1)
 │g20 │g21 │g22 │g23 │                           r_ab = result(q_ab)  (level L-2, +2^(L-3) gens)
 ├────┼────┼────┼────┤                         result(n) = join(r_00, r_01, r_10, r_11)
 │g30 │g31 │g32 │g33 │                                     (level L-1, +2^(L-2) gens)
 └────┴────┴────┴────┘
```

The corner squares `n_00, n_02, n_20, n_22` are `n`'s own children, so only
five of the nine are joined.

**Slow steps** (`j < L-2`, Golly's step sizes smaller than the node allows)
take the *centres* of the nine squares without advancing time,
`m_ij = centre(n_ij) = join(g[i][j].se, g[i][j+1].sw, g[i+1][j].ne, g[i+1][j+1].nw)`.
They then advance only in step 2, by `2^j`. This is the scheme of Golly's
`dorecurs_half` (in both `hlifealgo` and `ghashbase`). At level 5 each centre
is computed from four leaves with shifts and masks, then interned.

### 1.3 Base case: level 4, bit-parallel

A level-4 node is four leaves, a 16×16 square. Its result after 1, 2 or 4
generations is computed directly:

```text
 16×16 square as four u64 words, four 16-bit rows ("lanes") per word
 word 0: rows 0-3     word 1: rows 4-7     word 2: rows 8-11     word 3: rows 12-15
 west/east neighbours: (w << 1) & 0xFFFE_FFFE_FFFE_FFFE, (w >> 1) & 0x7FFF_7FFF_7FFF_7FFF
 row above: (t << 16) | (t_prev >> 48)        row below: (t >> 16) | (t_next << 48)
```

For each word, the horizontal pair and triple sums are bit-sliced into two
planes. A small adder tree adds the three rows into four count planes
`s0..s3` (counts 0 to 8), for 64 cells at once. The rule is compiled into the
list of counts that give a live cell, each tagged *birth only*, *survival
only* or *both*. Then `next = OR over those counts (count == n) & mask`, where
the mask is `!alive`, `alive` or all ones. Cells outside the 16×16 are treated
as dead, so after `g` generations only the centre `(16 - 2g)²` cells are
exact. For `g ≤ 4` that always covers the returned centre 8×8. The last
generation computes only the two centre words. The kernel is checked against
a naive stepper for four rules (unit test `leaf::tests`).

**Why 8×8 leaves rather than 4×4?** One `u64` leaf stands for up to 21
distinct nodes of a quadtree with 2×2 leaves (16 at level 1, 4 at level 2 and
1 at level 3), so a dense region needs about 20× fewer nodes and hash lookups. The 16×16
base case is four machine words, which suits SWAR: 64 cells per operation and
no lookup tables. A table-driven 4×4 → 2×2 base case, as in Golly, needs a
65,536-entry table rebuilt for every rule. 8×8 is also the leaf size of Golly's
macrocell format, so reading and writing `.mc` maps lines to leaves one to one.

### 1.4 Memo per step exponent

Each node has **one result slot plus the exponent `e = min(k, L-2)` it was
computed for**. A lookup is a hit only if the stored exponent equals the one
needed now. Nodes with `L - 2 ≤ k` always use `e = L - 2`, whatever `k` is,
so their results survive any change of step size. Nothing is cleared when `k`
changes, unlike Golly's `new_ngens` sweep. Larger nodes simply recompute and
overwrite their slot. Changing the **rule** does invalidate everything:
`set_rule` clears every result.

### 1.5 Top-level step and padding

The root is kept centred on the origin. A root of level `L` covers
`[-2^(L-1), 2^(L-1))²`. Invariant: all live cells lie in the root's **central
half** `[-2^(L-2), 2^(L-2))²`, unless the pattern spans more than `±2^62`.
`step_pow2(k)` then does the following:

```text
1. expand the root (keeping it centred) until L ≥ k + 2         (no-op usually)
2. top = expand(root)                    level L+1, the old root is its centre
3. root = result(top, k)                 level L: the old root's square after 2^k gens
4. normalise: expand if cells left the central half, then shrink while the
   centre square holds every cell in *its* central half
```

Correctness: before step 3 every cell is within `2^(L-2)` of the origin. After
`2^k ≤ 2^(L-2)` generations it is within `2^(L-1)`, which is exactly the
square `result(top)` returns. `expand` wraps each child in a node of empties
so that the old root becomes the centre of a square twice as large.

`advance(n)` decomposes `n` into its binary digits and calls `step_pow2` once
per set bit, smallest first. Evolution is a group action, so any order is
correct. Bits above `2^61` become repeated `2^61` steps. A step is refused,
leaving the universe unchanged, if the generation counter would overflow `u64`
or if the pattern is not within `±2^61` of the origin. The result must stay
within the `i64` plane: a level-64 root covers all of it.

---

## 2. Data structures

### 2.1 Node (32 bytes)

| Field | Type | Meaning |
|---|---|---|
| `children` | `[u32; 4]` | `nw, ne, sw, se` indices; a leaf stores its `u64` bitmap in `children[0..2]` |
| `population` | `u64` | live cells, saturating at `u64::MAX` (exact up to level 31); `Universe::population_exact()` and `population_in_rect()` give `u128` counts, summing saturated nodes from their children (memoised) |
| `result` | `u32` | memoised result index or `NONE = u32::MAX` |
| `level` | `u8` | 3 (leaf) to 64; 0 marks a free slot |
| `result_exp` | `u8` | the `e` of the memoised result |

### 2.2 Arena and intern table

* `Vec<Node>` arena with `u32` indices. Index `u32::MAX` is reserved, so up to
  `2^32 − 2` live nodes are possible, which is more than 128 GiB. Running out
  panics, like `Vec` capacity overflow. Freed slots go on a free list that is
  reused lowest-index first.
* The intern table is **open addressing with linear probing**, a power-of-two
  size and a load factor of at most ½ (it doubles when needed). Each slot is a
  `u64`: the upper 32 bits of the key hash (a *tag*) and the node index. Most
  non-matching slots are rejected by comparing tags, without touching the
  arena.
* **Hash function** (our own, deterministic, no seed): split the key into
  `x = nw | ne << 32` and `y = sw | se << 32`, XOR each with a constant whose
  upper half is all ones, fold in the level, and compute the 64×64→128-bit
  product folded as `lo ^ hi` (the "mum" mix of wyhash). Child indices are
  never `u32::MAX`, so neither factor of an interior key can be zero. The low
  bits pick the slot and the high bits form the tag.
* Determinism: the table is only ever used for lookups. Nothing iterates it
  to drive evolution, so results never depend on table layout. Collections
  scan the arena in index order.

### 2.3 Coordinates

`i64`, `x` to the right, `y` downward. Node boundaries of level `j` fall on
multiples of `2^j`, which is what makes `density_grid` with aligned origins
O(visible nodes). Internally, queries use `i128` corners and `u64` offsets,
so the full `i64` plane, including `i64::MIN` and `i64::MAX`, works without
overflow.

**Macrocell origin.** Golly places a file's root of level `L` so that it
covers `y ∈ [-2^(L-1)+1, 2^(L-1)+1)`: the top-left cell of the root's SE child
is `(0, 1)`. To keep Golly's coordinates exactly, the reader shifts the tree
down one row and the writer shifts it up one row. A *window* function builds
the square starting at the last (or second) row of a vertical pair of nodes,
memoised by the pair, so the cost is proportional to the number of distinct
nodes. In the single case where a level-64 root has a cell on row `i64::MIN`,
the writer emits a level-65 Golly root, and the reader accepts it when its
content fits the `i64` plane. Validation: the tiles of Golly's
`metapixel-galaxy.mc` land exactly on `x ≡ 0, y ≡ 1 (mod 2048)` with uniform
per-tile populations (§6).

---

## 3. Queries and level of detail

| Query | Method | Cost |
|---|---|---|
| `population()` | root's cached count | O(1) |
| `get_cell` / `set_cell` | descend / rebuild the path | O(L) |
| `insert_cells(iter)` | partition the points recursively into a fresh tree, then OR-merge with memo `(a, b)` | O(n·L) |
| `clear_rect` | recursive: drop whole nodes inside, mask leaves on the border | O(border nodes) |
| `cells()` | DFS iterator skipping empty nodes, quadtree order | O(cells + visited nodes) |
| `bounding_box()` | per side: prefer the near child pair, memoised per node | O(distinct nodes on the frontier) |
| `cells_in_rect(.., limit)` | clipped DFS, stops at `limit`, sorted row-major | O(result + visited nodes) |
| `density_grid(x0, y0, w, h, k)` | a node inside one `2^k` pixel adds its cached population; otherwise recurse | O(visible non-empty nodes of level ≥ k) when `x0, y0` are multiples of `2^k` |
| `population_in_rect(x0, y0, x1, y1)` | a node inside the rectangle adds its cached population; a node crossing its border is split, memoised per (node, edges crossing it) | O(non-empty nodes crossing the border), and O(distinct nodes) |
| `population_exact()` | `population_in_rect` of the whole plane | O(saturated nodes) |

`density_grid` is the renderer's level of detail. Zoomed out to `2^k` cells
per pixel, a viewport of `w × h` pixels touches at most about `w·h` nodes of
level `k`, whatever the population. Unaligned origins are also exact, but
they descend to leaves where nodes straddle pixel borders. Counts saturate at
`u32::MAX`.

`population_in_rect` is the exact count of one rectangle, as a `u128`
(saturating only for a completely full plane), without listing a cell or
going through `u32` pixels. Nodes inside the rectangle add their cached
population (a saturated one, above `u64::MAX` cells, is summed from its
children); empty nodes and nodes outside add nothing; only non-empty nodes
crossing the border are split, down to 8×8 leaves that are masked and
popcounted. That is at most four nodes per level for nodes larger than the
rectangle, and about `2(w + h) / 2^j` nodes of each smaller level `j` along a
`w × h` border. Every node below the root lies at a multiple of its side, so
the edges of the rectangle cut every copy of a node at the same offsets: the
count of a split node depends only on the node and on which of the four
edges cross it, and is memoised under that key (for nodes no larger than the
rectangle; larger ones are at most four per level anyway). A border through
a periodic or hash-consed region therefore costs as much as its distinct
structure: a rectangle cutting a full 2^33 × 2^33 square (2^66 live cells,
2^30 leaves along each edge) is counted at once (the unit test doing four
such counts takes milliseconds in a debug build).

---

## 4. Memory management

* **Collector.** A non-moving mark-and-sweep, like Golly's. The roots are the
  current root, the *pins* and the canonical empty nodes. Marking follows
  children **and memoised results**, so the memo of every node still in use
  survives. Unmarked slots are freed, the free list is rebuilt, trailing free
  slots are truncated, and the intern table is rebuilt at its minimum size.
* **Compaction (explicit collections only).** `garbage_collect()` and
  `clear_memo()` run between steps, when no Rust local holds a node index,
  so after marking they also renumber the survivors densely (keeping their
  order), rewrite children, results, the canonical empties and the root, and
  return the unused capacity of the arena and the intern table to the
  allocator. `memory_bytes()` therefore drops after clearing a large pattern
  and collecting; a non-moving sweep alone would keep the arena at its
  high-water mark. Automatic collections stay non-moving, because they may
  run inside a step.
* **During a step.** Rust locals in the recursion hold raw indices, so the
  collector must not move nodes. Every node a frame holds across a recursive
  call is pushed on `store.pins`, and the frame truncates the pins on return.
  A collection may run at the entry of `result()` for an uncached interior
  node (a *safe point*), at the start of a step and before an edit. A test
  with the minimum node limit forces several collections inside single steps
  and checks the results against a reference and the naive stepper. Removing
  a single pin makes that test fail.
* **Policy.** `set_node_limit(n)` (default `2^23` nodes, minimum 1024) sets the
  soft limit. When the arena reaches the threshold, a collection runs. The
  next threshold is `max(limit, 2 × live)`, so a live set near or above the
  limit cannot cause thrashing; the amortised cost is O(1) per allocation. The
  limit is **soft**: a collection never drops the pattern or the memo of its
  nodes. `garbage_collect()` runs a compacting collection now. `clear_memo()`
  forgets every result first and keeps only the current pattern. That frees
  the most memory, but futures must be recomputed.
* **Working set.** Hashlife's speed comes from finding results of squares
  computed earlier, which may be momentarily unreachable. If the limit is far
  below the working set, collections discard that memo and deep jumps can
  slow down by orders of magnitude, as in Golly with too little hash memory.
  Measured: Gotts dots to `3.66·10^12` needs 4.9 M nodes. With the default
  limit it runs in 1.4 s with no collection. With a `2^20`-node limit it takes
  23 s (124 collections). With a 65,536-node limit it had not finished after
  6 minutes.
* **Memory per node.** 32 bytes in the arena, plus 16 to 32 bytes of table
  (8-byte slots at a load factor between ¼ and ½). `memory_bytes()` reports
  allocated capacity, which only an explicit (compacting) collection lowers.

---

## 5. Complexity, and when Hashlife wins or loses

* One `step_pow2(k)` call costs O(number of distinct `(node, e)` results it
  must compute). For periodic or regular patterns that number grows roughly
  linearly in `k`, so time per generation falls exponentially. A glider jumps
  `2^40` generations in microseconds, and Gotts dots reaches generation
  `3.66·10^12` in about a second.
* For **chaotic** patterns, such as random soups and the expanding front of an
  explosion, few squares repeat. Each generation then costs hash-table work
  for every active 16×16 square, roughly 100–190 ns per new node on an M2 Max.
  That is tens of times slower than a dense SWAR engine at the same area. The
  research note measured about 11.6 Gcell/s single-threaded for a flat
  bit-packed grid.

| Workload | Best engine |
|---|---|
| Guns, breeders, puffers, oscillator and spaceship ensembles, metapixels (OTCA), looking `10^6`–`10^15` generations ahead | **Hashlife** (large steps, memo reuse) |
| Huge, sparse, highly repetitive space (agar tilings, metapixel grids) | **Hashlife** (memory ∝ distinct structure) |
| Watching a soup evolve generation by generation, bounded or toroidal grids, dense chaos | Dense SWAR or sparse tiles (`cellular-automata`) |
| B0 rules, multi-state rules, non-totalistic rules | Not supported here: use the dense engines |

`cellular-automata`'s automatic engine choice should prefer Hashlife when the
requested step exponent is above a few units, or the pattern's bounding box
is large compared with its population.

---

## 6. Measured performance

Hardware: Apple M2 Max (8 performance + 4 efficiency cores), 64 GB RAM, macOS.
`rustc 1.98.1`, `cargo bench` profile (release, thin LTO, `codegen-units = 4`),
single-threaded. Criterion 0.8 with 10 samples (`cargo bench -p hashlife`).
Other agents were compiling on the same machine during the final run (load
average about 38 on 12 cores). An earlier run under lighter load, on the same
code except for the last-generation shortcut of §1.3, is shown for comparison.

| Benchmark (`benches/hashlife.rs`) | What it does | Final run: median [95% CI] | Earlier run |
|---|---|---|---|
| `gosper_gun_2^20` | Gosper gun, cold memo, one `step_pow2(20)` | 525 µs [515, 535] | 495 µs |
| `soup_256x256_step_2^10` | 256×256 soup (density 0.375), one `step_pow2(10)` | 37.6 ms [37.2, 38.2] | 34.0 ms |
| `soup_256x256_1024_single_generations` | same soup, 1024 × `step_pow2(0)` (0.26 ms per generation) | 269 ms [265, 273] | 284 ms |
| `metapixel_grid_empty_expansion` | 256 blocks on a 16×16 grid of 2048-cell tiles (32,768 cells wide, almost empty), `advance(35328)` | 29.6 µs [29.5, 29.7] | 28.8 µs |

The following runs used Golly's own files on the same machine, measured with a
temporary release-build program that is not kept in the repository. The load
average was about 45. Times are single runs.

| Run | Time | Nodes / memory |
|---|---|---|
| Read `metapixel-galaxy.mc.gz` (7,408,195 cells) | 28 ms | 10,732 nodes |
| …then one generation (`step_pow2(0)`) | 1.8 ms | |
| …then 16 OTCA periods (16 × 35,328 = 565,248 generations), cold memo | 174 ms | 689,947 nodes, 48 MB |
| …the next 16 periods (memo warm, the tiling is periodic) | 0.24 ms | |
| Gotts dots from generation 0 to 3,662,092,278,363 (default node limit) | 1.44 s | 4,906,159 nodes, 384 MB, no collection; population 5,742,718,768,151 |
| Same run with `set_node_limit(2^20)` | 23.4 s | 124 collections, 100 MB |
| Read `metapixel-p216-gun.mc.gz` (128,116,349 cells) | 16 ms | 12,670 nodes |
| …then 4 OTCA periods (141,312 generations) | 287 ms | 1,137,526 nodes, 96 MB |
| `density_grid` of that whole pattern (142 × 120 pixels at 2^11 cells per pixel) | 0.10 ms | sum = population |

Per created node, the cost is roughly 110–190 ns (hash-table probes, arena
writes and base cases). A `sample` profile of the soup benchmark puts about
55% of the time in the recursion with the inlined SWAR kernel, about 40% in
interning, and about 7% in growing the intern table from a cold start.

---

## 7. Validation

All tests run with `cargo test -p hashlife`. The Golly-file tests are
`#[ignore]`d, because Golly's pattern files are GPL data that is not
committed. Run them with
`GOLLY_PATTERNS=/path/to/golly/Patterns cargo test -p hashlife --profile verify -- --ignored golly`.

* **Naive agreement** (`tests/engine/agreement.rs`). An independent
  `BTreeMap`-based stepper checks random soups seeded from `SimRng`: every
  generation from 1 to 64 for Life; arbitrary `n ∈ {1, 2, 3, 5, 7, 13, 31, 37,
  63, 64}` for six rules (Life, HighLife, Seeds, Day & Night, Life without
  Death with S0, Replicator); `step_pow2(0..=6)`; interleaved step sizes;
  soups at `±2^40` and `±(2^61 − 40)`.
* **Randomised differential tests** (`tests/engine/fuzz.rs`). 48 trials, each
  with a random rule without `B0` (any of the 2^17), a soup placed across
  quadtree boundaries or within a few hundred cells of `±2^61`, half of them
  at the minimum node limit, then 12 random operations: `step_pow2(0..=5)`,
  `advance(1..=24)`, `set_cell`, `clear_rect`, a rule change, a compacting
  `garbage_collect()` or `clear_memo()`, and a macrocell round trip (`read`
  or `read_into` the same store). After every operation the cells must equal
  the naive stepper's. A second test parses the canonical text of all 2^17
  rules back to the same rule and steps 200 random rules one generation at a
  time.
* **Rectangle populations** (`tests/engine/patterns.rs`): 40 random
  universes — evolved soups, a random 8/16/32-cell tile repeated on an
  aligned 6×6 grid (identical nodes cut by different edges of one
  rectangle), plus soups across the origin, the 2^40 and 2^62 node
  boundaries and at three corners of the `i64` plane, sometimes with cells
  at the plane's corners — each queried with 60 rectangles whose corners lie
  near live cells, near the origin, at or near `i64::MIN`/`i64::MAX` or
  anywhere (one in ten reversed, counting nothing), against a brute-force
  count; the whole plane gives the population. Unit tests (`query::tests`)
  count exactly over a full 2^33 × 2^33 square (2^66 cells, saturated
  nodes), on an empty universe and at the `i64` extremes.
* **Patterns** (`tests/engine/patterns.rs`):
  * a glider moves `(1000, 1000)` after `advance(4000)`, and `(2^38, 2^38)`
    after `step_pow2(40)`;
  * the blinker has period 2;
  * the Gosper gun (36 cells) has population 36 at generation 0 and gains
    exactly 5 cells per 30 generations for every `t ∈ [120, 1170]`, and again
    at generation 3,000,000;
  * the R-pentomino reaches 116 cells at generation 1103, and Acorn 633 cells
    at 5206;
  * the bounding box, `cells_in_rect`, the cell iterator and `density_grid`
    match a naive bucketing (aligned and unaligned origins, `k = 0..6`), and
    density sums equal the population.
* **Memory** (`tests/engine/memory.rs`): results are identical with
  `garbage_collect()` between steps, with mid-step collections at the minimum
  limit, after `clear_memo()`, and across rule changes. A warm memo survives
  collection and is reused. Clearing a pattern and collecting returns the
  memory (`memory_bytes()` falls below a quarter of its peak).
* **Macrocell**: round trips (including `i64::MIN`/`MAX` rows), Golly's origin
  convention, multicell `1`/`2`/`3`-level nodes, and line-numbered errors.
* **Store**: hash-consing, table growth, collection with kept memo, and
  compaction (dense renumbering, content and memo preserved, capacity
  released) checked against a full invariant scan. `Rule` deserialisation
  goes through the parser, so a `B0` rule cannot be loaded from a file.
* **Golly files (ignored)**:
  * all 25 two-state files under Golly's `Patterns` load, round-trip
    byte-identically, and keep their exact population; the 11 files with
    non-totalistic or multistate rules are rejected with a clear error;
  * the 16 files with ≤ 20,000 cells match the naive stepper over 100
    generations;
  * **Gotts dots** reaches the populations documented in the file:
    316,387 at `t = 215,643` and 5,742,718,768,151 at `t = 3,662,092,278,363`;
  * **metapixel galaxy**: the 15×15 meta-grid, decoded from per-tile
    populations with `density_grid(.., 11)`, follows B3/S23 for 16
    meta-generations (565,248 generations of a 7.4 M-cell pattern). The tile
    display lags one period. The tiling becomes exactly periodic, with period
    16 × 35,328, after a one-period transient.
* Mutation checks run while developing: swapping one quarter in the slow-step
  centre makes 17 integration tests fail. Removing a GC pin makes the
  tiny-node-limit test fail. Dropping the count-8 term of the SWAR kernel
  fails the kernel test, the six-rule agreement test and both randomised
  tests. Skipping the remapping of children or results in compaction fails
  the store test (and, for children, the memory and randomised tests).
  Memoising rectangle counts per node alone, without the crossing edges,
  fails the rectangle property test and the 2^33-square unit test.

---

## 8. References

* R. Wm. Gosper, "Exploiting regularities in large cellular spaces",
  *Physica D* 10 (1984) 75–80, doi:10.1016/0167-2789(84)90251-3.
* T. G. Rokicki, "An Algorithm for Compressing Space and Time",
  *Dr. Dobb's Journal*, April 2006.
* Golly 5.1 `gollybase/hlifealgo.{h,cpp}` and `ghashbase.cpp` (GPL). They were
  read to understand the macrocell format, the origin convention and the GC
  policy. No code was copied. See `docs/research/life-engines-hashlife.md`.
* B. Due, OTCA metapixel (2006); Golly's `Patterns/HashLife/Metacell`.
