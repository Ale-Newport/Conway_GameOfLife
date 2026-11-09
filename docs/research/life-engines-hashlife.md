# Life engines, Golly, Hashlife and pattern file formats

Research notes for the Emergence Lab Life engine (Conway's Life + Golly-like tooling).
Date: 2026-11-09. Scope: B3/S23 semantics and rule strings, how Golly's QuickLife/HashLife work, a
clean-room Hashlife design for Rust, existing Rust crates, dense engines, file formats, finite topologies.

Sources and how they were used:

- **Golly 5.1 source** (local shallow clone, commit `9a7079a8`, 2026-11-06, GPL-2.0-or-later). I read
  `gollybase/{hlifealgo,hlifedraw,ghashbase,qlifealgo,liferules,generationsalgo,lifealgo,readpattern,writepattern}.cpp`,
  `gui-common/control.cpp` and `gui-wx/wxcontrol.cpp`, plus `Help/{bounded,formats,control}.html` and
  `Help/Algorithms/*.html`. **No Golly code is reproduced here.** Everything below is a description of the
  algorithms in my own words, so the project stays clean-room (MIT OR Apache-2.0).
- **LifeWiki** (`conwaylife.com`) returns HTTP 403 (Cloudflare bot check) to WebFetch, and I did not try to get around it.
  Format pages were read from the older public mirror `conwaylife.njohnston.ca`, which may be **out of date**.
- **Validation done for this note.** All of these are my own code in the session scratchpad, not project code:
  - `proto/hl.py`, `hl2.py`, `hl3.py`: a Python Hashlife prototype that implements Section 3 directly. It was checked
    against a naive simulator and against published test vectors (Section 3.12).
  - `proto/swar.rs`: a Rust SWAR (SIMD within a register) benchmark (Section 5.1).
- Anything not verified is marked **UNVERIFIED**.

---

## 0. Summary (TL;DR)

1. **Semantics.** Life is the synchronous, Moore-neighbourhood, outer-totalistic rule `B3/S23` on Z².
   Use Golly's rule-string syntax as the project's external format. That covers B/S, legacy S/B, the `H`/`V`
   neighbourhood suffixes, Hensel isotropic letters, `MAP`, `Wn`, the Generations `S/B/C` form, and the
   bounded-grid suffix `:T/P/K/C/S`. Exact grammars are in Section 1.
2. **Hashlife.** Write our own engine (Section 3).
   - Every Rust Hashlife crate falls short on at least one count: licence (`gol_engines` is GPL-3.0), maintenance
     (`smeagol`: MPL-2.0, last release 2019, needs nightly `packed_simd`), or maturity (`golback`, `hashlife`). See Section 4.
   - The algorithm is about 1–2 kLOC for a solid version.
   - The pseudo-code below was validated with a prototype. It reproduces Rokicki's 2006 figure: breeder at
     generation 10^12 has population 1,302,083,334,180,208,337,404.
3. **Recommended Hashlife design.**
   - Arena `Vec<Node>` with `u32` ids.
   - Intern table: hashbrown or FxHash.
   - Level-3 leaves stored as `u64` 8×8 bitboards. The level-4 base case is a 16×16 SWAR simulation.
   - Full-speed result cached inline. Slow-step results cached in a map keyed by `(node, j)`, so changing the step size
     never needs a cache sweep.
   - Per-node saturating `u64` population. Exact population is computed with `u128`/BigUint on demand.
   - Mark-and-sweep GC at step boundaries. Optional in-step GC uses an explicit root stack, as Golly does.
4. **Dense engine for bounded grids and chaotic soups.** Use bit-packed SWAR, 64 cells per `u64`, with full-adder
   neighbour counting. Measured on this M2 Max with B3/S23 on a torus:
   - 1 thread: about 12–18 Gcell-updates/s.
   - 12 threads: about 106 Gcell-updates/s on 16384².
   - For unbounded interactive worlds, add 64×64-cell tiles in a hash map with an active set.
5. **Topologies.** Plane, torus (optionally shifted), Klein bottle, cross-surface and sphere belong in the dense engine
   as ghost-cell (halo) mappings (Section 7). Hashlife should only run on the unbounded plane, plus optionally a
   torus of side 2^k (validated 4-copy trick, Section 3.8).
6. **Formats.** Write our own readers and writers for:
   - RLE: Golly's extended RLE, multistate letters, `#CXRLE`.
   - Plaintext `.cells`.
   - Life 1.05 and 1.06.
   - Macrocell `.mc`, two-state and multistate.
   - gzip-compressed `.gz` versions of these.

   The grammars are in Section 6. `ca-formats` (MIT) can serve as a differential-test oracle.

---

## 1. Conway's Life: exact semantics and rule strings

### 1.1 B3/S23

- Lattice Z², states {0 = dead, 1 = live}. Time is discrete and every cell updates at once, from generation t to t+1.
- Moore neighbourhood N(x,y) = {(x+dx, y+dy) : dx,dy ∈ {−1,0,1}, (dx,dy) ≠ (0,0)}, so 8 cells.
  n = number of live cells in N.
- Transition: `s' = 1 iff (s = 0 ∧ n ∈ B) ∨ (s = 1 ∧ n ∈ S)`, with B = {3} and S = {2,3}.
- Equivalent "3×3 total" form, which is handy for SWAR: let T = n + s (the 3×3 sum including the centre).
  Then `s' = 1 iff T = 3 ∨ (T = 4 ∧ s = 1)`.
- An infinite universe has a dead background. Because B0 is false, empty stays empty. Hashlife and sparse engines
  depend on that, which is why B0 rules need special handling (Section 1.2).
- Information travels at most 1 cell per generation in Chebyshev distance ("speed of light" c). Hashlife's padding
  rules come from this bound.
- Coordinates: Golly's external coordinates have **y increasing downward**. RLE rows run top to bottom, and `#CXRLE Pos`
  is the top-left corner. Inside, `hlifealgo` flips y (`y = −y` in `setcell`).

### 1.2 Golly life-like rule strings (QuickLife / HashLife)

The grammar below is taken from `liferules::setrule` and `Help/Algorithms/QuickLife.html`. Parsing is
**case-insensitive** and spaces are ignored.

```
rule        := ( lifelike | map | wolfram ) [ suffix ]          ; suffix = bounded grid, Section 1.4
lifelike    := bs_form | sb_form
bs_form     := "B" part [ "/" | "_" ] "S" part [ nbhd ]          ; B before S, '/' optional (Golly 5.1 accepts "B3S23")
             | "S" part [ "/" | "_" ] "B" part [ nbhd ]          ; explicit S-first ("S23/B3") is accepted too
sb_form     := part "/" part [ nbhd ]                            ; legacy: first part = SURVIVAL, second = BIRTH ("23/3")
part        := { count }
count       := digit [ [ "-" ] letters ]                         ; Hensel isotropic non-totalistic letters (Moore only)
digit       := "0".."8"   (≤ 6 with H, ≤ 4 with V)
letters     := one or more of c e a i k n j q r y t w z (which letters are legal depends on the digit)
nbhd        := "H" | "V"                                         ; hexagonal (ignores NE and SW) / von Neumann; must be last
map         := "MAP" base64 [ "==" ]                             ; 86 chars (Moore, 512 bits), 22 (hex), 6 (vN)
wolfram     := "W" even_number_0_to_254                          ; 1-D elementary CA
```

- `_` may replace `/`, but only in non-totalistic rules. The canonical form always uses `/`.
- **Canonicalisation.** `getrule()` returns `B…/S…`. Digits are in ascending order. Letters are alphabetical and
  written in the shortest equivalent form (the version without the minus sign wins ties). A count whose letters
  cover every neighbourhood becomes plain totalistic. Examples from Golly's help:
  - `B2nic/S` becomes `B2cin/S`.
  - `B2ceikn/S` becomes `B2-a/S`.
  - `B2aceikn3/S` becomes `B23/S`.
- **MAP bit order.** Number the 3×3 cells 0..8 row-major, NW to SE, with cell 4 as the centre.
  - Row index i = Σ c_k · 2^(8−k).
  - The 512 output bits are listed for i = 0..511 and base64-encoded MSB-first.
  - Life is `MAPARYXfhZofugWaH7oaIDogBZofuhogOiAaIDogIAAgAAWaH7oaIDogGiA6ICAAIAAaIDogIAAgACAAIAAAAAAAA`.
    I checked by hand that bits 7 and 11 (B3) are set.
- **B0 rules.**
  - QuickLife emulates them. A rule with B0 and Smax becomes the complementary rule
    (e.g. `B0123478/S01234678` → `B3/S23`).
  - A rule with B0 but without Smax becomes a pair of rules that alternate on even and odd generations.
  - HashLife accepts B0 **only together with Smax** (`S8`, or `S6` for H, `S4` for V). Loading B0-not-Smax from `.mc`
    is an error.
  - Odd Wolfram rules are rejected for the same reason.
- `isRegularLife()` checks the rule bitmask `0x1808`, which is bit 3 (B3) plus bits 9+2 and 9+3 (S2, S3).

### 1.3 Generations rule strings (`generationsalgo`)

- Golly's canonical form is `S/B/C`. The digits before the first slash are **survival**, then birth, then the
  number of states C (2..256). Examples: `/2/3` (Brian's Brain), `23/3/2` (Life as Generations), `345/2/4` (Star Wars).
- Golly also accepts `B…/S…/C`-like orderings with the letters `B` and `S`. After the second slash it accepts **only
  digits**: by code reading, `B2/S/C3` is rejected with "Bad character found". **UNVERIFIED** at runtime.
  The neighbourhood suffix `H`/`V`, Hensel letters, `MAP…/C` and the bounded-grid suffix are allowed. B0 is rejected.
- Other ecosystems use `B2/S/C3` (LifeViewer/LifeWiki style) and Catagolue's `g3b2s` (per the `ca-rules` docs).
  Our parser should accept all three and emit Golly-canonical output.
- **Semantics** (`generationsalgo::slowcalc`): only state-1 cells count as neighbours.
  - A cell in state 0 with n ∈ B becomes 1.
  - A cell in state 1 with n ∈ S stays 1.
  - Any other cell in state ≥ 1 moves to state+1. Reaching C wraps to 0, so a cell dies after the C−1 state.

### 1.4 Bounded-grid suffix (`lifealgo::setgridsize`, `Help/bounded.html`)

```
suffix   := ":" [ topo [ wd [ twist ] [ shift ] [ "," ht [ twist ] [ shift ] ] ] ]
topo     := "P" | "T" | "K" | "C" | "S"        ; case-insensitive; canonical uppercase
wd, ht   := decimal (0 = infinite; values ≥ 2·10^8 clamp to 2,000,000,000)
twist    := "*"                                ; Klein bottle only; marks the twisted edge pair
shift    := ("+" | "-") decimal                ; stored as shift mod size
```

| Suffix | Meaning |
|---|---|
| `:P30,20` / `:P30` / `:P30,0` | Bounded plane. Width only means a square. A zero dimension is infinite. |
| `:T30,20`, `:T0,20` | Torus, or an infinite tube when one dimension is 0. |
| `:T30+5,20` / `:T30,20-2` | Torus with the horizontal edges shifted (crossing top/bottom shifts x) or the vertical edges shifted. **Only one** of the two. |
| `:K30*,20` / `:K30,20*` | Klein bottle with horizontal or vertical edges twisted (reversed). `:K30,20` defaults to a vertical twist. |
| `:K30*+1,20` | Twisted and shifted. Allowed only on the twisted pair, and only if that dimension is even. Any non-zero shift is treated as 1. |
| `:C30,20` | Cross-surface (real projective plane). Both pairs twisted. No shift. |
| `:S30` | Sphere. Width must equal height; `,ht` is not accepted. No shift. |

Other rules:

- `:` alone, or `:T` alone, means unbounded (same as `T0,0`).
- K, C and S cannot have an infinite dimension.
- Shifting is not allowed when either dimension is infinite.
- The grid's top-left cell is at `(−int(w/2), −int(h/2))`, using C truncating division.
- The canonical suffix is printed with both dimensions, e.g. `:T100,100`. The sphere is the exception (`:S30`).
- Semantics are in Section 7.

---

## 2. How Golly does it (read, not copied)

### 2.1 Inventory

| Algorithm | Kind | States | Notes |
|---|---|---|---|
| QuickLife (`qlifealgo`) | Dense/sparse conventional | 2 | Default base step 10 |
| HashLife (`hlifealgo`) | Hashlife, highly tuned | 2 | Default base step 8; default max memory 500 MB |
| Generations, JvN, Super, RuleLoader (plus the internal `ruletable_algo` and `ruletreealgo`), all subclasses of `ghashbase` | Generic multistate Hashlife | ≤ 256 | Hashlife with 2×2 leaves |
| Larger than Life (`ltlalgo`) | Dense array | ≤ 256 | Range-r rules |

### 2.2 QuickLife (from the header comments of `qlifealgo.h`)

- **Slice.** A 4 (wide) × 8 (high) block packed into a 32-bit word, MSB = top-left.
- **Rule application.** A 65,536-entry table (`liferules::rule0`) maps a 4×4 neighbourhood (16 bits) to the inner
  2×2 of the next generation.
- **Brick.** 8 slices for the even phase plus 8 for the odd phase: 32×8 cells × 2 generations = 64 bytes.
- **Stagger-step.** The odd-generation brick holds the area shifted by (+1, +1). Each phase then needs neighbours
  from only one diagonal side. The comment credits Alan Hensel.
- **Tile.** 4 bricks covering 32×32 cells (32 bytes).
  - Per-slice and edge "changed" flags let unchanged regions be skipped.
  - Empty space points to a shared `emptybrick`, so pointers are never null.
  - Each tile also stores two 12-bit population counts and dirty bits.
- **Supertile.** 8 children per level, stacked horizontally on odd levels and vertically on even levels:
  level 1 = 256×32, level 2 = 256×256, and so on. Change flags are aggregated per child and per edge.
  Population is cached per supertile and invalidated by dirty bits.
- **Growth.** A new root supertile is added when the pattern nears the bounds.
- **Takeaway for us.** Bit-packed blocks, change tracking at block granularity with edge flags, and a shared
  empty block. Our 64×64 tile design (Section 5.3) is a simpler modern version of the same idea.

### 2.3 HashLife (`hlifealgo.cpp`, `hlifedraw.cpp`)

**Data layout**

- **Node** = 6 machine words, 48 bytes on 64-bit: `next` (hash chain), `nw ne sw se`, `res` (cached result).
- **Leaf** = an 8×8 square.
  - Same size as a node. The `nw` slot is null, and that is how leaves are recognised.
  - Four 16-bit 4×4 quadrants, MSB-first in row-major order.
  - A `leafpop` population.
  - `res1` = the centre 4×4 after **1** generation and `res2` = the centre 4×4 after **2** generations.
    Both are computed eagerly when the leaf is first created: nine table lookups give a 6×6 result, then four more
    lookups give the second generation.
- **Depth.** Golly's "depth" d is defined so that a leaf has d = 2, and a node covers 2^(d+1) cells per side.
  In this note I use **level L = log2(side)**, so L = d + 1 and a leaf has L = 3.

**Canonicalisation (hash-consing)**

- `find_node(nw,ne,sw,se)` hashes the four child **pointers**: `65537·se + 257·sw + 17·ne + 5·nw`, plus `r += r>>11`.
  Leaves use the same polynomial over the four u16 values.
- The table is power-of-two sized with chaining. A hit is moved to the front of its chain.
- The table resizes ×2 when the load factor exceeds 0.7. A GC runs first to make the resize cheaper.
  If memory is short, resizing stops and the table runs at a higher load factor.
- Every node has exactly one canonical instance, so pointer equality means equal contents. Empty squares at every
  level are a single node (`zeronode(d)`).

**Memoised result: `getres`**

- `getres(n)` returns `n->res` if it is set.
- Otherwise it computes the centre square, half the side of `n`, advanced by 2^(L−2) generations at full speed,
  or by fewer generations in slow mode (see below).
- **Full speed.** Build 9 overlapping sub-squares from the grandchildren and get their results. Assemble 4 squares
  from the 9 results and get those results. Join the 4 results.
- **16×16 nodes** (`dorecurs_leaf`) do the same thing using the leaves' `res2`.
- `getres` is **the only place** that writes `res`. It never writes `res` if the computation was interrupted:
  the GUI poller is "sticky", and an interrupted computation returns empty placeholders.

**Step size**

- The user-facing increment is `base^exp`. Internally it is factored as `increment = nonpow2 × 2^ngens`, where
  `nonpow2` must fit in 31 bits.
- `step()` runs the root `nonpow2` times at step 2^ngens.
- `ngens` is a single global per universe.
- In `getres`, a node with `L−2 ≤ ngens` uses full speed (`dorecurs`). A larger node uses the half variant
  (`dorecurs_half`): the 9 results are taken at the reduced step and then **re-assembled from their inner quarters**,
  with no second round. A level-4 node with `ngens = 0` uses `res1` (1 generation). Slow nodes compose a 2^ngens
  step at every level.
- Because `res` holds a single value, changing `ngens` triggers `new_ngens`. It runs a GC, then walks the hash table and
  clears `res` on nodes whose level is above about the smaller of the old and new exponents, plus one.
  Nodes small enough to be full-speed under both settings keep their cache.
- The help notes that step changes are slow when memory is large.
- `halvesdone` counts cached half-step results. It has two uses:
  - `new_ngens` skips the cache walk entirely when the step is *increased* and no half results exist.
  - `runpattern` clears the root's `res` when exactly one half computation happened.

**Top-level step (`runpattern`)**

- Expand the root twice with `pushroot`, which centres the old root inside a node twice as big. Then keep expanding
  while `ngens + 2 > d`, i.e. while L < ngens + 3.
- Call `getres` once, then `popzeros`: while the outer ring of 12 grandchildren is empty, replace the node by its centre.
- Generation += 2^ngens.
- The universe stays centred on (0,0).

**Hyperspeed** (GUI, `wxcontrol.cpp`)

- While generating with a hashing algorithm, the step exponent increases by 1 every 64 steps (`hypdown = 64` → `GoFaster()`).
- The help says hyperspeed "quickly runs away" on regular patterns. For chaotic ones QuickLife may go further.

**Garbage collection**

- Nodes are carved from 1360-node blocks to suit allocator chunk sizes. Old hash arrays are reused as node blocks
  instead of being freed.
- GC triggers **during** recursion when the free list is empty and allocating another block would exceed `maxmem`.
- **Roots** are:
  - the root;
  - the largest zero node;
  - timeline frames;
  - a GC **stack**. Every node returned by `find_node` and `getres` is pushed with `save()`, and each `getres` frame
    pops back to its entry depth with `pop(sp)`. Intermediate values held in C locals therefore stay alive.
- Mark sets the low bit of `next`. Sweep walks all node blocks, re-threads marked nodes into a cleared hash table and
  moves the rest to the free list.
- `invalidate=1`, used after a rule change, clears all `res` fields and recomputes leaf results.
- Results of marked nodes are themselves marked (kept) unless invalidating. Golly therefore keeps its memo cache across GCs.

**Population caching**

- There is **no** per-node population field, to save memory.
- `calcpop` walks the DAG and temporarily unhashes each node, storing a `bigint` in its `next` field (marked via the
  low bits). Leaves keep `leafpop`. The table is restored afterwards.
- The result is cached as `population` with a `popValid` flag, invalidated each step.
- `getPopulation` returns −1 during GC or calculation.

**Rendering** (`hlifedraw.cpp`)

- Recursive `drawnode` with level-of-detail: empty nodes are skipped by pointer comparison with the zero node, and
  off-screen nodes are skipped.
- When a node covers exactly 1 screen pixel it is drawn as **one on-pixel if non-empty**. This is binary, not density.
- At 2×2 and 4×4 pixel sizes, 4×4 compression tables are used.
- Output goes into 256×256 bitmaps blitted by the renderer.

**Macrocell I/O.** Two-pass writer: first number the nodes, then write them in child-first order. Details in Section 6.6.

### 2.4 `ghashbase` (generic multistate Hashlife)

- Same node layout as `hlifealgo`. Leaves are **2×2 cells of `u8` states** (level 1).
- The base case `dorecurs_ghleaf` calls the virtual `slowcalc(nw,n,ne,w,c,e,sw,s,se)` four times to get the
  centre 2×2 of a 4×4 after 1 generation.
- Its half step (`dorecurs_half`, for nodes above level 2) works **the other way round** from hlife's:
  1. Take the **centres** of the 9 overlapping squares, with no time advance.
  2. Build 4 squares from them.
  3. Call `getres` on those 4 at the reduced step.

  Both variants are correct. I validated both (Section 3.12).

### 2.5 Bounded grids in Golly

- All of Golly's algorithms are unbounded. Bounded grids are emulated by the GUI (`NextGeneration` in `control.cpp`):
  the increment is forced to 1, and every generation does the following.
  1. `CreateBorderCells()`: copy edge cells into a 1-cell ring outside the grid, using the topology mapping. Only
     states > 0 are copied. The bounded plane copies nothing.
  2. `step()`.
  3. `DeleteBorderCells()`: clear every live cell outside the grid. The pattern usually grows 2 cells past the edge,
     more under B0-type rules.
- That is why the help warns that bounded grids are slow. It also means Hashlife's big steps are unused on bounded grids.
- When an RLE with known size and no `#CXRLE Pos` is loaded into a bounded grid, it is centred (`xoff = −wd/2`).
  Otherwise it goes to the top-left of the grid.

---

## 3. Hashlife for a fresh Rust implementation

References:

- R. Wm. Gosper, "Exploiting regularities in large cellular spaces", *Physica D* 10(1–2):75–80, 1984,
  doi:10.1016/0167-2789(84)90251-3 (verified via Crossref).
- T. G. Rokicki, "An Algorithm for Compressing Space and Time", *Dr. Dobb's Journal*, April 2006.

### 3.1 Definitions and invariants

- A node at **level L** is a canonical 2^L × 2^L square. Its children are `[nw, ne, sw, se]` at level L−1.
- Nodes are immutable and interned, so `id_a == id_b` exactly when the contents are equal.
- `result(n, j)`, for L ≥ 2 and 0 ≤ j ≤ L−2, is the level L−1 **centre** of n after 2^j generations.
  - It is well defined because the centre's future for 2^j ≤ 2^(L−2) generations depends only on cells inside n
    (light-cone argument).
  - j = L−2 is "full speed". j < L−2 is "slow step".
- `centre(m) = join(m.nw.se, m.ne.sw, m.sw.ne, m.se.nw)` is the level L−1 centre of m at the same time.
- `empty[L]` is the canonical empty node. `result(empty[L], j) = empty[L−1]`. This only holds without B0.

### 3.2 Data structures (sketch)

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash)] struct NodeId(u32);      // u32::MAX = NONE
struct Node {                     // ~40 B; leaves reuse `kids` storage for their bitmap
    kids: [NodeId; 4],            // level >= 4: nw, ne, sw, se
    leaf: u64,                    // level == 3: 8x8 bitmap, bit (y*8+x), y down   (union with kids in practice)
    level: u8,
    pop: u64,                     // saturating; exact for level <= 31 (4^31 < 2^64)
    res: NodeId,                  // full-speed result (2^(L-2) gens) or NONE
}
struct Hashlife {
    nodes: Vec<Node>,                                // arena; free list if GC is non-moving
    intern: hashbrown::HashTable<NodeId>,            // hash(kids) / hash(leaf); key not duplicated
    slow: FxHashMap<(NodeId, u8), NodeId>,           // result(n, j) for j < L-2
    empty: Vec<NodeId>,                              // empty[L]
    root: NodeId,                                    // centred on (0,0): covers [-2^(L-1), 2^(L-1))^2
    generation: BigUint,                             // or u128
}
```

- **Leaf granularity.** Use level-3 `u64` leaves, not 1-cell or 2×2 leaves. This cuts node count about 64× in dense
  areas, and the base case becomes word-level SWAR.
  `golback` uses cell-level leaves. The Python prototype also does, for clarity.
- **IDs.** `u32` ids are enough: 2^32 nodes × ~40 B ≈ 170 GB, which is more than this machine has.
- **Hashing.** Use a fast deterministic hasher, FxHash or foldhash. `rustc-hash 2.1.3`, `hashbrown 0.17.1` and
  `foldhash 0.2.0` are already in the local cargo registry, probably as transitive bevy dependencies.
  Their compatibility with the version pins is **UNVERIFIED**.
- **Coordinates.** The engine needs none. Only the root's level, and therefore its extent, matters. Use i64 at the API
  boundary for L ≤ 62. Above that, runaway hyperspeed patterns need i128 or BigInt; Golly uses bigint coordinates.

### 3.3 Interning

```
join(a, b, c, d):                         // all same level L-1 >= 3
    key = (a, b, c, d); h = hash(key)
    if let id = intern.find(h, |id| nodes[id].kids == key): return id
    pop = sat_add(a.pop, b.pop, c.pop, d.pop)
    id = alloc(Node { kids: key, level: L, pop, res: NONE })
    intern.insert(h, id); return id
leaf(bits: u64): same, keyed by bits, pop = bits.count_ones()
```

### 3.4 Base case (level 4 → level 3, j ∈ {0, 1, 2})

1. Unpack the four 8×8 leaves into a 16×16 board stored as 16 × `u32` rows.
2. Run the rule 2^j times (1, 2 or 4 generations) with the SWAR row step from Section 5.1. After k generations the
   valid area is the centre (16−2k)², so 4 generations leave exactly the centre 8×8.
3. Extract the centre 8×8 into a `u64` and return `leaf(bits)`.
4. Memoise: j = 2 goes in `res`, j < 2 goes in `slow`.

For any outer-totalistic, Hensel or MAP rule, replace the SWAR kernel with a rule-specific kernel or a 65,536-entry
4×4→2×2 lookup table, as Golly and QuickLife do.

### 3.5 Recursion

```
result(n, j):                                   // L = n.level >= 4, 0 <= j <= L-2
    if n.pop == 0: return empty[L-1]
    if j == L-2 { if n.res != NONE { return n.res } }
    else if let r = slow.get((n, j)) { return r }
    if L == 4: r = base_case(n, j)
    else:
        let (a, b, c, d) = n.kids              // level L-1
        // 9 overlapping level-(L-1) squares, built from grandchildren
        n00 = a;                         n01 = join(a.ne, b.nw, a.se, b.sw);  n02 = b
        n10 = join(a.sw, a.se, c.nw, c.ne);  n11 = join(a.se, b.sw, c.ne, d.nw);  n12 = join(b.sw, b.se, d.nw, d.ne)
        n20 = c;                         n21 = join(c.ne, d.nw, c.se, d.sw);  n22 = d
        if j == L-2:                             // FULL SPEED: two half-steps of 2^(L-3)
            s_ij = result(n_ij, L-3)             // 9 x level L-2
            q00 = result(join(s00, s01, s10, s11), L-3)
            q01 = result(join(s01, s02, s11, s12), L-3)
            q10 = result(join(s10, s11, s20, s21), L-3)
            q11 = result(join(s11, s12, s21, s22), L-3)
        else:                                    // SLOW: one step of 2^j, j <= L-3
            // variant A (ghashbase-style): centres first, then 4 results
            c_ij = centre(n_ij)                  // 9 x level L-2, time unchanged
            q00 = result(join(c00, c01, c10, c11), j)   ... (q01, q10, q11 likewise)
            // variant B (hlifealgo-style): s_ij = result(n_ij, j); then
            //   q00 = join(s00.se, s01.sw, s10.ne, s11.nw) ... (inner quarters, no second result)
        r = join(q00, q01, q10, q11)
    memoise(n, j, r); return r
```

- Variant A makes 4 recursive `result` calls per node. Variant B makes 9 plus 4 cheap joins.
- Both were validated against a naive simulator (Section 3.12).
- Which one is faster with memoisation depends on the workload. Benchmark both; **UNVERIFIED**.
- Use an explicit stack or keep recursion. Depth is at most about the root level (≤ a few hundred), so the native
  stack is fine.

### 3.6 Top-level stepping, padding and shrinking

```
expand(n):  z = empty[L-1]
            join(join(z,z,z,n.nw), join(z,z,n.ne,z), join(z,n.sw,z,z), join(n.se,z,z,z))   // keeps (0,0) centred
shrink(n):  while L > 4 and the 12 outer grandchildren of n are all empty[L-2]: n = centre(n)
step_pow2(j):
    r = expand(expand(root)); while r.level < j + 3: r = expand(r)
    root = shrink(result(r, j)); generation += 2^j
step(N):    for each set bit j of N: step_pow2(j)        // evolution commutes; any order is correct
```

Why this padding is enough (light cone):

- After two expansions the old root, of side S, sits in the central S×S of a 4S square.
- The result is the central 2S, so the margin around the old root is S/2.
- The node level is L ≥ j+3, so the margin is also ≥ 2^L/8 ≥ 2^j.
- No live cell can reach past the margin in 2^j generations, so nothing is lost.

This matches Golly's `runpattern` rule (two `pushroot`, then `while ngens+2 > depth`).

Step-size policy:

- **Fixed exponent.** For animation, use `base^exp` with a fixed j, as Golly does, and step repeatedly so `slow`
  entries are reused.
- **Arbitrary N.** Use the binary decomposition above. It is correct, but uses a different j per bit.
- **Hyperspeed.** Increment j every K steps; Golly uses K = 64. Because the slow cache is keyed by `(node, j)`,
  **no cache sweep is needed** when j changes, unlike Golly's `new_ngens`. Old `(n, j)` entries simply age out at GC.

### 3.7 Editing, loading, extracting

- **`set_cell(x, y, v)`.** Expand the root until (x, y) is inside, then copy the path from the root down to the leaf,
  re-joining each level. That is O(L) interning operations.
- **Bulk load.** Bucket cells into a `HashMap<(lx, ly), u64>` of 8×8 leaves, then build bottom-up by recursive
  quadrant split. Empty quadrants use `empty[L]`.
- **Iterate live cells.** Do a DFS that skips nodes with pop = 0, then iterate the leaf bits. Cost is
  O(#distinct visited nodes + #cells).
- **Bounding box** (`findedges`). Find the extreme non-empty paths with a DFS that prunes on pop = 0.

### 3.8 Torus of side 2^k inside Hashlife (validated)

Let T be the level-k torus contents and j ≤ k−1. Then:

```
T' = roll(result(join(T, T, T, T), j)),   roll(X) = join(X.se, X.sw, X.ne, X.nw)
```

Why it works:

- The centre of the 2×2 tiling is T rolled by (N/2, N/2).
- A periodic pattern's tiling evolves periodically.
- Rolling by half twice is the identity.

The prototype's results matched a naive torus simulation. Non-power-of-two and twisted topologies should use the
dense engine. `gol_engines` documents the same restriction: square tori with power-of-two sides.

### 3.9 Population

- Each node stores `pop: u64` (saturating), computed at intern time. Leaves use `count_ones`.
- This is exact up to level 31. **u64 is not enough at the root**: breeder at 10^12 generations has 1.3×10^21 cells.
- `exact_population(root)` should recurse with a memo `HashMap<NodeId, u128 or BigUint>` only for nodes at level ≥ 32.
  There are few of them, because high-level nodes are rare.
- Golly computes bigint population lazily instead (Section 2.3).

### 3.10 Level-of-detail rendering

```
draw(n, x0, y0, L):                               // world-space origin of n
    if n.pop == 0 or square(x0, y0, 2^L) ∩ view == ∅: return
    px = 2^L * scale                               // on-screen size
    if px <= 1:  plot(x0, y0, density = pop / 4^L)   // or "any live" like Golly
    elif L == 3: plot leaf bits (each bit is >= 1 px)
    else: recurse into 4 kids
```

- Cost is O(visible pixels × depth), independent of population.
- Snap zoom to powers of two, as Golly does with `mag`, so node boundaries line up with pixels.
- Write to an `R8`/`R16` density image that the Bevy layer uploads as a texture. Colour-map it with a gamma or log
  curve so sparse ash stays visible.
- **Big win from canonical nodes.** Cache rasterised tiles per `(NodeId, lod)` in an LRU, e.g. 32×32-pixel tiles for
  nodes whose on-screen size is 32 px. Repeated structures such as OTCA metapixels, guns and agar then render once.
  This is a design proposal, **UNVERIFIED** for performance.

### 3.11 Memory management

Options:

1. **Clear-and-rebuild**, as in Rokicki 2006. Create a fresh table and re-intern from the live roots, dropping all memo
   results. Simplest, but after each GC the cache must warm up again.
2. **Mark-and-sweep with a free list** (Golly). Mark from the roots, mark `res` and live `slow` values (keeping the
   cache), rebuild the intern table from marked nodes, and free the rest.
   - **Recommended v1:** run GC **only between top-level steps**. Roots are the root, `empty[*]`, timeline and undo
     snapshots, and pinned library nodes (e.g. OTCA metacells).
   - Policy: GC when `nodes.len()` passes about 90% of the budget. If less than about 30% is freed, GC again and drop
     all `res`/`slow` results. Both thresholds are **UNVERIFIED** heuristics.
   - If one step overruns a hard limit, abort the step (results computed so far stay valid) and retry with smaller j.
     `gol_engines` documents the same idea: "may reduce the step if an update does not fit in memory".
3. **In-step GC (v2)**, needed for huge single steps. Keep an explicit `Vec<NodeId>` root stack. Push every node
   produced inside `result`, and truncate to the frame's entry length on return, as Golly's `save`/`pop` does.
   Then a non-moving GC can run at allocation time.
   - **Do not combine with compaction.** Rust locals hold raw ids.
4. **Compaction** (optional, between steps). Remap ids in DFS order for locality, then rewrite `kids`, `res`, `slow`
   and the external roots.

Budget: about 40 B per node plus table overhead. A hashbrown `HashTable<u32>` adds about 6–12 B per entry: 4 B for
the value plus 1 control byte, divided by a load factor between 0.44 and 0.875.
Golly's default is 500 MB and its help suggests ~50% of RAM. On a 64 GB machine, a default of 4–8 GB with a user
setting is reasonable.

### 3.12 Validation vectors

These were reproduced with the prototype (`proto/hl*.py`). Use them as Rust integration tests:

| Test | Expected | Status |
|---|---|---|
| Random 16×16 soups for N ∈ {1,2,3,5,8,13,37,64,100}, binary decomposition | = naive simulator | ✔ both slow-step variants |
| R-pentomino at generation 1103 | population 116 | ✔ |
| Acorn at generation 5206 (and 6206) | population 633 | ✔ |
| Glider after 2^20 generations | displaced by (2^18, 2^18), population 5 | ✔ |
| Breeder (Golly `Patterns/Life/Breeders/breeder.lif`) at generation 10^12 | **1,302,083,334,180,208,337,404** (matches Rokicki 2006) | ✔ 0.3 s in Python, 345,700 nodes |
| 2^4 torus, 4-copy trick, mixed j | = naive torus | ✔ |

The breeder file is Golly data (GPL distribution). Re-enter the pattern ourselves or confirm its licence before
committing it as a fixture (Section 8).

### 3.13 Rules beyond B3/S23, determinism, parallelism

- **Any radius-1 rule** works unchanged:
  - Outer-totalistic, Hensel isotropic and MAP rules, via the base-case kernel.
  - Multistate rules (Generations, Wireworld, Brian's Brain), via `u8` leaves or 2-bit/8-bit plane leaves, as in
    `ghashbase`.
- **B0 without Smax** breaks the empty-maps-to-empty invariant. Either emulate it as Golly does (complement or
  alternate rules) or refuse the rule in Hashlife.
- **Radius r > 1** (Larger than Life): result(n) can only advance 2^(L−2)/r generations, so generalise the time-step
  bookkeeping. Golly runs LtL with a dense algorithm.
- **Determinism.** `result` is a pure function. Output never depends on hash seeds, GC timing, table layout or thread
  scheduling, as long as outputs are produced by tree traversal and never by iterating a hash map.
- **Parallelism.** The 9-then-4 structure exposes parallel subproblems. `gol_engines` (GPL-3.0, so idea only, no code)
  reports HashLife speedups with a concurrent memo table and work stealing, e.g. 29.3× from 1 to 32 cores on the
  0E0P metaglider, and little or none on the Pi calculator. Start single-threaded.
- **StreamLife** (Goucher, in lifelib, **MIT**) is an advanced follow-up for patterns made of non-interacting streams.
  Port it from lifelib if needed, not from `gol_engines`.

---

## 4. Existing Rust crates

Status was checked with the crates.io API and the GitHub/GitLab APIs on 2026-11-09.

| Crate | Version / last release | Licence | What it is | Assessment |
|---|---|---|---|---|
| `smeagol` | 0.1.2 / 2019-02-27 (repo pushed 2022; master `0.2.0-alpha`, unpublished) | MPL-2.0 | Hashlife + SIMD | Depends on `packed_simd 0.3`, which is nightly-only; repo toolchain `nightly-2022-03-17`. Unmaintained. **Do not use.** |
| `gol_engines` | 0.3.1 / 2026-11-08 (active) | **GPL-3.0** | Parallel HashLife + StreamLife for B3/S23 only; `.rle`/`.mc`/`.mc.gz`; power-of-two torus; OTCA "metafy" | Best-performing Rust engine with published benchmarks (beats Golly 5.0 and lifelib 2.5.9 on most patterns with many cores). **GPL-incompatible with our MIT/Apache distribution.** Benchmark reference only; do not read its code into ours. |
| `golback` | 0.3.4 / 2026-04-15 | MIT | Hashlife, cell-level leaves, B/S rules, `advance(n)` for any n | Young (2 stars), cell-level leaves (slow and memory-heavy). Useful as an MIT cross-check. |
| `hashlife` | 0.0.0 / 2020 | crates.io says MIT OR Apache-2.0; repo has no LICENSE | Placeholder | Ignore. |
| `lari` | 0.1.3 / 2026-10-14 | GPL-2.0-or-later | Life library | Licence-incompatible. |
| `life-backend` | 0.0.2 / 2023 | MIT OR Apache-2.0 (GitHub detects Apache-2.0) | Simple backend | **Archived.** |
| `ca-formats` | 0.3.5 / 2023-03-18 (repo pushed 2025-01) | MIT | Parsers for RLE (incl. Golly extended), Plaintext, apgcode, Macrocell | No Life 1.05/1.06 and no writers; regex-based. Good **dev-dependency oracle**. |
| `ca-rules` | 0.3.5 / 2023-03-18 | MIT | Rule-string parser: B/S, S/B, Hensel, MAP, hex, von Neumann, Generations (`B357/S3457/C5`, `3457/357/5`, `g5b357s3457`) | Good reference and oracle for our rule parser. Low activity. |
| `bevy_life` | 0.13.0 / 2026-02-08 | MIT (crates.io metadata says "non-standard") | Generic CA plugin, one entity per cell | Targets **bevy 0.18**, not our 0.20.1. Entity-per-cell does not scale. Not suitable. |
| `rlifesrc-lib` | 0.6.1 / 2023 | MIT | Pattern *search* (SAT-like) | Out of scope for engines; possibly useful for the evolution lab later. |
| lifelib (C++/Python, GitLab `apgoucher/lifelib`) | active (2026-10-18) | **MIT** | Hashlife, StreamLife, upattern SIMD, many rules | Best reference for porting ideas, with attribution. |
| Golly (C++) | 5.1, 2026-11 | GPL-2.0-or-later | Reference implementation | Read only; clean-room. |

**Recommendation: implement our own** Hashlife, dense engines and format I/O. Reasons:

- licence (the only performant engines are GPL);
- maintenance;
- feature fit: multistate, our rule DSL, Bevy-independent determinism, LOD rendering hooks, bounded topologies;
- size: the core is small.

Use `ca-formats` and `ca-rules` (MIT) as optional dev-dependency oracles, and lifelib (MIT) as the algorithmic reference.

---

## 5. Dense engines

### 5.1 Bit-packed SWAR, 64 cells per u64

Layout: row-major. Bit i of word k is column 64k+i, and rows are `W/64` words.

Neighbour words for row r:

- West-shifted: `(w << 1) | (w_prev >> 63)`.
- East-shifted: `(w >> 1) | (w_next << 63)`.
- The same for rows r−1 and r+1.

Kernel, validated against a naive simulator for 100 generations on a 192×70 torus:

```
// above row (3 inputs) and below row (3 inputs) -> 2-bit sums; middle row has 2 inputs (no centre)
a1 = aw ^ a ^ ae;  a2 = (aw & a) | (ae & (aw ^ a))
b1 = bw ^ b ^ be;  b2 = (bw & b) | (be & (bw ^ b))
c1 = cw ^ ce;      c2 = cw & ce
ones  = a1 ^ b1 ^ c1                       // weight-1 bit of the count
carry = (a1 & b1) | (c1 & (a1 ^ b1))       // weight-2 carry
// count ∈ {2,3}  <=>  exactly one of {a2, b2, c2, carry} is set
x = a2 ^ b2;  y = c2 ^ carry
exactly_one = (x ^ y) & !((a2 & b2) | (c2 & carry))
next = (ones | centre) & exactly_one       // B3/S23
```

The kernel is 30 bitwise operations per 64 cells. Building the west- and east-shifted words adds about 6 more per
word when each row's shifted words are computed once and reused for the 3 output rows that need them.

Measured on the Apple M2 Max (8 performance + 4 efficiency cores), `rustc -C opt-level=3 -C target-cpu=native`,
B3/S23 on a torus, no explicit SIMD intrinsics:

| Grid | Threads | Throughput | Time per generation |
|---|---|---|---|
| 4096² | 1 | 11.6 Gcell-updates/s | 1.45 ms |
| 16384² | 1 | 18.2 Gcell-updates/s | 14.7 ms |
| 16384² | 12 (`std::thread::scope`, spawned every generation) | 106 Gcell-updates/s | 2.53 ms |
| 4096² | 12 (same) | 10.2 Gcell-updates/s | spawn overhead dominates |

- Use a persistent pool (rayon) and parallelise only above a size threshold.
- These are micro-benchmarks: they include `%`-based wrap indexing and use no SIMD. NEON `u64x2` or wider
  explicit SIMD could add more (**UNVERIFIED**).

**Generic outer-totalistic rules**:

- Extend the adder tree to 4 bit-planes c0..c3 (counts 0..8).
- `next = OR_{k∈B} (¬s ∧ eq_k) ∨ OR_{k∈S} (s ∧ eq_k)`, where `eq_k` is the AND of the plane literals.
  Precompute a minimised boolean form per rule, or use a 512-entry 3×3 table on the scalar path.
- **Hensel and MAP rules** need neighbour *configuration*, not counts. Either evaluate a 512-bit truth table
  bit-serially (slow), or use QuickLife's 65,536-entry 4×4→2×2 table on 4×4 blocks. The table is the usual choice.
- **Generations**: store states as bit-planes (⌈log2 C⌉ planes) plus an "alive = state==1" plane. Neighbour counting
  uses the alive plane. Decay is a bit-sliced increment, masked with a wrap to 0 at C.

### 5.2 Chunked sparse storage with an active set

- **Tile.** 64×64 cells = `[u64; 64]` (512 B), with one row per word.
- **Index.** `FxHashMap<IVec2, TileId>` over a tile arena with double buffers, or two maps swapped each generation.
- **Halo per tile.** Row 63 of the tile above, row 0 of the tile below, bit 63 of each row of the west tile, bit 0
  of each row of the east tile, and 4 corner bits. Then run the same row kernel with carries taken from the
  neighbour tiles.
- **Active set.**
  - A tile is recomputed at generation t+1 only if it or one of its 8 neighbours **changed** between t−1 and t.
    If nothing in its neighbourhood changed, its next state equals its current state.
  - Candidates are the dirty tiles plus their neighbours.
  - Allocate a missing neighbour tile only when an adjacent tile has live cells on the shared edge or corner.
  - Free tiles that stay empty, with some hysteresis.
  - Still lifes (ash) then cost nothing. Oscillators keep their tiles active. This is QuickLife's change-flag idea at
    a coarser grain.
- **Parallelism.** Each candidate tile is a pure function of 9 old tiles, so `rayon::par_iter` over the candidates
  gives deterministic results. Sort tile keys when deterministic iteration order matters (export, statistics).
- **Population.** Keep a per-tile `count_ones` sum, updated when a tile is rewritten.

### 5.3 When each wins

| Workload | Best engine | Why |
|---|---|---|
| Bounded worlds (any topology), dense chaotic soups, per-generation animation, GPU display | **Flat SWAR** | Constant cost of W·H/64 words per generation; simplest halo handling; easy to parallelise and to upload as a texture |
| Unbounded, mostly empty, localised activity (soups settling to ash, small spaceships) | **Tiles + active set** | Cost ∝ active area; unbounded growth |
| Highly regular structure in space or time: guns, breeders, metapixels / OTCA "Life in Life", long-run fast-forward | **Hashlife** | Sub-linear in generations; memory ∝ distinct structure; the only practical option for OTCA (2048×2048 metacells, period 35,328, **UNVERIFIED here**: see the OTCA research note) |
| High-entropy chaos over huge areas | Tiles or SWAR; **not** Hashlife | Golly's help says HashLife "performs very poorly on highly chaotic patterns" |

A good default for the app:

- Edit and run in the tile engine.
- Switch to Hashlife (manually, or automatically when step exponent > ~4 or the pattern exceeds a size threshold).
- Convert between the two via cell lists or leaf bitmaps.

Both engines must be bit-for-bit equivalent. Add a property test that compares them on random soups.

### 5.4 GPU compute

- **Approach.** A wgpu compute shader. One invocation handles one or two packed `u32` words, ping-pong storage buffers,
  and a workgroup tile in shared memory with a 1-word/1-row halo. The fragment shader renders straight from the bit
  buffer, so there is no CPU readback.
- **Pros.**
  - Very high throughput for large dense bounded grids.
  - Integer/bitwise operations are exactly deterministic across GPUs.
  - Rendering is local, with no upload.
  - Upper bound from 400 GB/s unified memory on M2 Max and ~2 bits/cell/generation of traffic: on the order of
    10^12 cell-updates/s. **UNVERIFIED estimate.**
- **Cons.**
  - Readback latency for analytics (population, bounding box, events) needs reduction passes.
  - Poor fit for unbounded, sparse or Hashlife workloads.
  - Determinism tests need a CPU reference anyway.
  - **Architecture conflict.** Simulation crates must not depend on Bevy. A GPU engine would depend on `wgpu`, and to
    share Bevy's device it must live in the rendering crate.
- **Recommendation.** Defer. The CPU SWAR engine is the source of truth. Later, add an optional GPU "display-rate
  accelerator" for huge bounded worlds in the render crate, with cross-check tests against the CPU engine.

---

## 6. File formats (exact grammars)

### 6.1 Golly's format detection (`loadpattern`)

Detection runs in this order, after skipping leading whitespace-only lines:

1. `#Li…` → Life 1.05/1.06 (`readpclife`).
2. `#P ` → Life 1.05 body without a header (WinLifeSearch clipboard).
3. `#MCell` → MCell.
4. First non-whitespace `#` or `x` → RLE.
5. `!` → dblife/Plaintext (`readdblife`).
6. `[` → Macrocell.
7. The line contains only digits, `b`, `o`, `$`, whitespace and an optional final `!`, with some digit or the `!`
   present → headerless RLE.
8. Otherwise "text pattern": `.`, `,` and characters ≤ space are dead, `$` means **10** dead cells, anything else is live.

gzip input is read transparently when Golly is built with zlib (`.rle.gz`, `.mc.gz`). Golly also loads `.zip`
archives (GUI). `@RULE` data appended to an `.rle` (not `.rle.gz`) is extracted as a temporary `.rule` file.

### 6.2 RLE / Extended RLE (`.rle`)

```
file        := { blankline | commentline } [ cxrle_lines ] { commentline } header data [ trailer ]
cxrle_lines := { "#CXRLE" { ws key "=" value } EOL }        ; Golly: only recognised at the very top
               key: "Pos" value = int "," int  (absolute top-left of the bounding box, y down)
                    "Gen" value = digits        (generation count, arbitrary precision)
commentline := "#" tag text EOL
               tag: C | c  comment (C preferred)   N  pattern name   O  author/date (XLife)
                    P | R  top-left coordinates "x y" (Life32 / XLife; R often negative = centred)
                    r      rule in S/B form, e.g. "#r 23/3" (ignored by Golly if a rule is already set)
header      := ws* "x" ws* "=" ws* uint ws* "," ws* "y" ws* "=" ws* uint
               [ ws* "," ws* "rule" ws* "=" ws* rulestring ] ws* EOL      ; rule defaults to B3/S23
data        := { item | ws | EOL } "!"                    ; "!" ends the pattern; '!' with no count
item        := [ count ] token
count       := nonzero_digit { digit }                    ; Golly: leading zero = error; no ws inside item
token       := "b" | "." | "o" | "$" | state_letter
state_letter:= "A".."X"                     ; states 1..24
             | ("p".."y") ("A".."X")        ; states 24·(prefix−'p'+1) + (letter−'A'+1): pA=25 … pX=48, qA=49 …, yO=255
trailer     := anything after "!" (historically comments; ignored)
```

Semantics:

- **Rows.** Cells run left to right within a row. `n$` ends the current row and skips n−1 blank rows. Trailing dead
  cells in a row, and the dead remainder of the last row, may be omitted.
- **Two-state files** use `b`/`o`. **Multistate files** (more than 2 states) use `.` for 0 and the letters for 1–255.
- **Reader leniency** (Golly):
  - `b` and `.` are interchangeable, and so are `o` and `A`.
  - A `p`..`y` prefix that is not followed by `A`..`X` is read as state 1.
  - Unknown letters are ignored.
  - Whitespace between items is allowed. Whitespace between a count and its token is an **error**, as is a leading
    zero. Both checks were added for LifeViewer consistency.
  - Leading whitespace on lines is ignored.
- **Header** dimensions are informational on load. Golly uses them when pasting, and when centring a pattern in a
  bounded grid. Without `#CXRLE Pos`, the top-left goes to (0,0) on unbounded grids.
- **Writer conventions** (Golly `writerle`):
  - `#CXRLE Pos=x,y [Gen=n]` first, when extended RLE is enabled.
  - Then `#…` comment lines.
  - Then `x = W, y = H, rule = R`.
  - Data lines are kept to **≤ 70 characters** by breaking only between items.
  - Runs of 1 have no count. Trailing dead cells in a row are dropped. Blank rows are folded into `n$`. The pattern
    ends with `!`.
  - An empty pattern is written as `x = 0, y = 0, rule = R` followed by `!`.
  - Comment text that does not start with `#` is written after `!`.
- **LifeWiki** (mirror) adds:
  - Lines should not exceed 70 characters, but readers must accept longer ones.
  - DOS, Unix and Mac newlines are all valid.
  - Two-state readers should treat any letter other than `b` as live.
  - Mark Niemiec's `x`, `y`, `z` tags exist; Golly reads them as `o`.
  - Golly's `3D.lua` uses an RLE-like `.rle3` format.
- **Brian's Brain example:** `x = 3, y = 2, rule = /2/3` with data like `.A$AB!`.

### 6.3 Plaintext (`.cells`)

```
file     := { "!" text EOL } { row EOL }          ; first line conventionally "!Name: <name>"
row      := { "." | "O" }                           ; 'O' (capital letter O) = live, '.' = dead
```

- The `!` lines are comments. A bare `!` line usually comes before the pattern.
- No maximum line length.
- Omitting trailing dead cells, or writing empty rows as blank lines, is described as non-standard but common.
  **Readers must accept both.** Writers should pad rows to the full width for strict compliance.
- `*` is documented as an erroneous substitute.
- Golly reads `.cells` with its dblife reader:
  - Every non-`!` line is a row.
  - Digits are run counts (dblife heritage).
  - `O` is live and `.` is dead.
  - **Other characters, including `*`, are ignored.** A blank line is an empty row.
- Multistate is not supported.

### 6.4 Life 1.05 (`.lif`, `.life`)

```
file    := "#Life 1.05" EOL { "#D" text EOL } [ "#N" EOL | "#R" ws survival "/" birth EOL ] { block }
block   := "#P" ws int ws int EOL { row EOL }       ; top-left of block relative to centre (0 0)
row     := ( "." | "*" ) { "." | "*" }               ; '*' live; an empty row is written "."
```

- `#N` means normal Conway rules. The `#R` digits are **S/B order**, e.g. `#R 125/36`.
- Description lines are ≤ 78 characters (at most 22 lines), and every line is ≤ 80 characters.
- Zero-length lines are ignored. CRLF line endings.
- Golly also accepts `#R B3/S23` (it calls the generic `setrule`). Lines starting with anything other than `.`, `*`,
  `#`, `-` or a digit are ignored.
- Golly's sample `persian-rugs.lif` uses `#R B234`.

### 6.5 Life 1.06 (`.lif`, `.life`)

```
file := "#Life 1.06" EOL { int ws int EOL }      ; one live cell per line, "x y"
```

- Two-state only. Order and global offset are irrelevant.
- Golly parses any line starting with `-` or a digit as coordinates, using the same reader as 1.05. It also honours
  `#P`, `#N`, `#R` lines.
- The y direction is not specified on LifeWiki. Golly uses its y-down coordinates.

### 6.6 Macrocell (`.mc`, `.mc.gz`)

```
file      := header { comment } { nodeline }
header    := "[M2]" text EOL                         ; Golly writes "[M2] (golly <version>)"
comment   := "#R" ws rule EOL                        ; rule (Golly always writes it)
           | "#G" ws digits EOL                      ; generation (written if non-zero)
           | "#FRAMES" ws count ws start ws base "^" expo EOL   ; timeline (optional)
           | "#FRAME" ws index ws nodeindex EOL      ; timeline frame root
           | "#" text EOL                            ; other comments; writer prefixes "#C "
nodeline  := leaf8 | multistate_leaf | inner
leaf8     := { "." | "*" | "$" }+  EOL               ; two-state 8x8 leaf (level 3)
multistate_leaf := "1" sp state sp state sp state sp state EOL   ; 2x2 leaf: nw ne sw se, states 0..255
inner     := level sp nw sp ne sp sw sp se EOL       ; level = log2(side): >= 4 (two-state), >= 2 (multistate)
```

- **Two-state leaf.** Rows are written top to bottom. `.` is dead, `*` is live, `$` ends a row. Trailing dead cells
  in a row and trailing empty rows are omitted, and no whitespace is allowed.
  Example (glider leaf): `$$..*$...*$.***$$$$`.
  Golly writes the leaf's last row with its `$` and stops at the last non-empty row.
- **Node numbering.**
  - Every node line, leaf or inner, gets the next number, starting at **1**.
  - Index **0** is the empty square of the required size.
  - A line may only refer to earlier lines, so the order is children first.
  - Each child must be exactly one level lower.
  - The **last line is the root**, and it is the single largest node.
- **Origin.** By Golly convention, the top-left cell of the root's SE child is at (x = 0, y = 1), in Golly's y-down
  coordinates. hlife's internal y-flip explains the off-by-one.
- **Compatibility.** Two-state and multistate files are mutually incompatible: leaf types and minimum levels differ.
- **Writing from our Hashlife.** Post-order DFS with a `HashMap<NodeId, u64>` of assigned numbers. Skip empty nodes
  (they are written as 0). Write leaves as 8×8 rows. Golly's level number is `depth + 1`, which is our L.

### 6.7 Other formats (lower priority)

- **MCell** (`#MCell` header; Mirek Wójtowicz): read by Golly.
- **dblife**: `!` comments; `O`/`.` with counts.
- **apgcode / Extended Wechsler**: Catagolue object codes. Supported by `ca-formats`, useful for soup census.
- **LifeViewer `#C [[ … ]]` script comments**: preserve them as plain comments.
- **Golly `.rule`**: `@RULE`/`@TABLE`/`@TREE`/`@COLORS`. That is the CA-lab topic; see `formats.html`.

---

## 7. Topologies for finite worlds

Internal coordinates are 0 ≤ x < w and 0 ≤ y < h, with y down. To match Golly, the external coordinate is
`x_golly = x − int(w/2)` and likewise for y.

Each step behaves as if neighbour lookups at (x, y) outside the grid are mapped by the rules below. Golly gets the
same effect with a 1-cell ghost ring that it refills every generation (Section 2.5). Corner rules fall out of applying
the x-rule and the y-rule in sequence; I checked this against every Golly diagram.

| Topology | Out-of-range mapping (apply y-crossing, then x wrap, or the reverse as noted) |
|---|---|
| Plane `:P` | Every outside cell is permanently dead. Births outside are discarded. |
| Torus `:T` | `(x mod w, y mod h)` |
| Torus with horizontal-edge shift `:Tw+s,h` | Crossing the top (y<0): `y += h, x −= s`. Crossing the bottom (y≥h): `y −= h, x += s`. Then `x mod w`. Checked against `:T4+1,3`. |
| Torus with vertical-edge shift `:Tw,h+s` | Crossing the left (x<0): `x += w, y −= s`. Crossing the right: `x −= w, y += s`. Then `y mod h`. |
| Tube `:T0,h` / `:Tw,0` | Wrap only the finite dimension; the other is unbounded. |
| Klein bottle, horizontal twist `:Kw*,h` | Crossing top or bottom: `y = y mod h, x = w−1−x`. Left/right: plain wrap. Checked against `:K4*,3`. |
| Klein bottle, vertical twist `:Kw,h*` (also the default for `:Kw,h`) | Crossing left or right: `x = x mod w, y = h−1−y`. Top/bottom: plain wrap. |
| Klein bottle, twist + shift `:Kw*+1,h` (w even) | Golly code: crossing top or bottom maps to `x = (w−2−x) mod w`. **Caution:** the help and code-comment diagram show `x = (w−x) mod w`, which is the same surface translated by 2 columns, not the same cell mapping. Follow the code if exact Golly compatibility matters (**UNVERIFIED at runtime**). |
| Cross-surface `:C` | Both pairs twisted: `(x, −1) → (w−1−x, h−1)`, `(−1, y) → (w−1, h−1−y)`, and similarly at the far edges. Corner cells are their own neighbours. |
| Sphere `:Sn` (w = h = n) | Top edge joined to left edge, right edge joined to bottom edge: `(−1, y) → (y, 0)`, `(x, −1) → (0, x)`, `(n, y) → (y, n−1)`, `(x, n) → (n−1, x)`. Each diagonal corner maps to the same corner, e.g. `(−1,−1) → (0,0)`. The two "poles" (top-left and bottom-right) have different neighbourhoods from the other two corners. |

Implementation notes for the dense SWAR engine:

- **Ghost ring.** Keep 1 ghost row above and below, and a ghost column at each side as edge bits. Rebuild them each
  generation from the mapping.
  - **Twisted edges**: the ghost row is the opposite edge row **bit-reversed**: `u64::reverse_bits`, then shift right
    by `64·words − w` and stitch across words.
  - **Sphere**: ghost rows and columns are a **transpose** of an edge column or row. Gather 1 bit per row.
  - **Shifted torus**: the ghost row is the opposite edge row rotated by s bits (multi-word funnel shift).
  - **Plane**: ghosts are 0, and bits past w in the last word must be masked to 0 after every step.
- **Widths not divisible by 64**: mask the last word, and take wrap neighbours from bit w−1, not bit 63.
- **Sizes.** Golly allows up to 2·10^9 per side. A dense engine is RAM-bound: 2^32 cells = 512 MiB per buffer.
  Larger bounded worlds, or an infinite tube, need the tile engine with per-tile wrap mapping.
- **Hashlife** supports these topologies only for a 2^k torus (Section 3.8). Run everything else in the dense or tile
  engine, or do what Golly does and step one generation at a time with ghost cells, which loses Hashlife's advantage.
- **B0-type rules on bounded planes.** Outside cells stay dead (Golly deletes them), so the plane is not
  complement-symmetric. Document this for B0 emulation.

---

## 8. Risks, open questions, UNVERIFIED items

- **Clean-room discipline.** Golly (GPL-2.0+), `gol_engines` (GPL-3.0) and `lari` (GPL-2.0+) must not be copied or
  closely paraphrased in code. lifelib (MIT) can be ported with attribution.
- **Pattern files.** Golly's `Patterns/` folder ships inside a GPL distribution, and individual files credit their
  authors. Licensing of the data needs review before we bundle any of it. The cell configurations themselves are
  probably not copyrightable, but the comments are text; **legal opinion UNVERIFIED**.
- **Hashlife memory blow-up** on chaotic input. Needs a memory budget, a step-reduction fallback, and an automatic
  suggestion to switch to the tile engine.
- **Population overflow.** u64 is not enough at the root (Section 3.9).
- **Golly-compatibility quirks.**
  - Shifted-Klein cell mapping (code vs diagram).
  - Macrocell origin (0,1).
  - RLE errors for whitespace between a count and its token, and for leading zeros.
  - Generations `C` prefix rejected by Golly.
- **Unmeasured performance choices.** Slow-step variant A vs B, explicit SIMD, the GPU estimate, GC thresholds and the
  LOD tile cache are all **UNVERIFIED** until benchmarked in Rust.
- **Source freshness.** LifeWiki content came from an older mirror (`conwaylife.njohnston.ca`). The live wiki
  could not be read without a browser.

---

## 9. References

- Golly 5.1 source: <https://sourceforge.net/projects/golly/> (git: `https://git.code.sf.net/p/golly/code`), files
  listed at the top of this note.
- Golly help:
  - <https://golly.sourceforge.io/Help/formats.html>
  - <https://golly.sourceforge.io/Help/bounded.html>
  - <https://golly.sourceforge.io/Help/control.html>
  - <https://golly.sourceforge.io/Help/Algorithms/QuickLife.html>
  - <https://golly.sourceforge.io/Help/Algorithms/HashLife.html>
  - <https://golly.sourceforge.io/Help/Algorithms/Generations.html>
- Gosper, R. W. (1984). *Exploiting regularities in large cellular spaces.* Physica D 10:75–80.
  doi:[10.1016/0167-2789(84)90251-3](https://doi.org/10.1016/0167-2789(84)90251-3). Scan:
  <https://gwern.net/doc/cs/cellular-automaton/1984-gosper.pdf>
- Rokicki, T. G. (2006). *An Algorithm for Compressing Space and Time.* Dr. Dobb's Journal, April 2006.
  - <https://www.drdobbs.com/cpp/an-algorithm-for-compressing-space-and-t/184406478>
  - Archive copy that was read: <https://jacobfilipp.com/DrDobbs/articles/DDJ/2006/0604/0604b/0604b.html>
- LifeWiki pages, read via the mirror:
  - RLE: <https://conwaylife.njohnston.ca/wiki/RLE>
  - Plaintext: <https://conwaylife.njohnston.ca/wiki/Plaintext>
  - Life 1.05: <https://conwaylife.njohnston.ca/wiki/Life_1.05>
  - Life 1.06: <https://conwaylife.njohnston.ca/wiki/Life_1.06>
  - Canonical URLs: `https://conwaylife.com/wiki/...`
- Crates:
  - <https://crates.io/crates/smeagol>
  - <https://crates.io/crates/gol_engines> (<https://github.com/das67333/gol_engines>)
  - <https://crates.io/crates/golback>
  - <https://crates.io/crates/ca-formats> (<https://docs.rs/ca-formats/0.3.5/ca_formats/>)
  - <https://crates.io/crates/ca-rules> (<https://docs.rs/ca-rules/0.3.5/ca_rules/>)
  - <https://crates.io/crates/bevy_life>
  - <https://crates.io/crates/life-backend>
  - <https://crates.io/crates/lari>
  - <https://crates.io/crates/hashlife>
- lifelib (MIT): <https://gitlab.com/apgoucher/lifelib>
