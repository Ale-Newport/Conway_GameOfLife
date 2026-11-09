# Hashlife (`life.hashlife`)

`crates/hashlife` runs two-state Life-like cellular automata on an unbounded
plane with Gosper's Hashlife. A hash-consed quadtree memoises the future of
every distinct square, so regular patterns can jump up to `2^61` generations
in one call. Guns, breeders, puffers, oscillator and spaceship ensembles, and
OTCA metapixel tilings are all regular in this sense. The engine
(`hashlife::Universe`) is used directly by `cellular-automata` (its Hashlife
adapter) and by `metapixel` (exact B3/S23 execution of metapixel tilings).
`hashlife::HashlifeSim` exposes it as a `simulation_core::Simulation` with
model id `life.hashlife`.

The algorithm, data structures, memory management and measured performance
are described in [`docs/HASHLIFE.md`](../HASHLIFE.md).

---

## 1. Model (rules actually executed)

* **State space.** Every cell of the square lattice ℤ² is dead (0) or alive
  (1). Coordinates are `i64`, with `x` to the right and `y` downward. The
  plane has no edges.
* **Neighbourhood.** Moore: the 8 cells at Chebyshev distance 1. Let `N(c, t)`
  be the number of live neighbours of cell `c` at generation `t`.
* **Transition** (synchronous, outer-totalistic rule `B{β}/S{σ}` with
  `β, σ ⊆ {0, …, 8}`):

  ```text
  s(c, t+1) = 1  if s(c, t) = 0 and N(c, t) ∈ β     (birth)
            = 1  if s(c, t) = 1 and N(c, t) ∈ σ     (survival)
            = 0  otherwise
  ```

  `0 ∉ β` is required (no `B0`), so the empty plane is a fixed point and the
  universe stays finite. `S0` is allowed (e.g. Life without Death,
  `B3/S012345678`).
* **Evaluation.** Exact Hashlife. A node of side `2^L` memoises its centre
  square of side `2^(L−1)` after `2^j` generations (`j ≤ L−2`). The result is
  bit-for-bit what the synchronous rule produces: there is **no
  approximation**.
* **Ticks.** One `step()` advances `2^k` generations, where `k` is the
  `step_exponent` parameter (Golly's "step size" `2^k`). `tick()` reports the
  generation count, so its unit is the generation: `ModelDescriptor::dt` is
  1 generation, and `Simulation::advance(n)` advances exactly `n`
  generations by binary decomposition, whatever the step exponent.

The descriptor's `rules` text is generated from the current rule and step
exponent (`sim::rules_text`). It states the same transition with the actual
birth and survival sets.

## 2. Parameters

| Key | Kind | Default | Policy | Effect |
|---|---|---|---|---|
| `rule` | text (≤ 32) | `B3/S23` | `NextTick` | Rule in `B3/S23`, `b3s23`, `S23/B3`, legacy `23/3` or `Life` form. Rejected: `B0`, non-totalistic letters (`B3-jknr…`), `MAP`, Generations, bounded-grid suffixes (`:T…`). Changing it clears every memoised result, since they depend on the rule. |
| `step_exponent` | int 0–61 | 0 | `Immediate` | Generations per tick = `2^k`. |
| `node_limit` | int 1024–2·10⁹ | 8,388,608 | `Immediate` | Soft limit on live quadtree nodes (about 48 B each, table included) at which garbage collection runs. |

## 3. Commands

| Command | Effect | Errors |
|---|---|---|
| `SetCell { x, y, alive }` | sets one cell | — |
| `LoadCells { cells, replace }` | sets cells alive (optionally clearing first); bulk-built quadtree | — |
| `LoadMacrocell(text)` | replaces pattern, rule and generation with a Golly `.mc` file (Golly's coordinates) | `SimError::Parse` with line number |
| `RandomSoup { x, y, width, height, density }` | replaces the rectangle's content with a soup drawn from the model's `SimRng`, row-major | `InvalidCommand`: density outside [0, 1] or NaN, empty or > 2^24-cell area, rectangle leaving `i64` |
| `ClearRect { x0, y0, x1, y1 }` | kills the cells of the inclusive rectangle | `InvalidCommand` if `x0 > x1` or `y0 > y1` |
| `Clear` | kills every cell (rule and generation kept) | — |

Every command clears a halt (see §6) and marks the pattern as edited. The
**reset point** is the state, including the RNG, captured when a run starts
after an edit, so `reset()` behaves like Golly's reset.

## 4. Metrics and view

| Metric | Meaning |
|---|---|
| `population` | live cells (saturating `u64`; `Universe::population_exact()` gives `u128`) |
| `generation` | generations since generation 0 |
| `node_count` | live quadtree nodes, memoised results included |
| `memory_bytes` | bytes allocated by the node arena and intern table |
| `root_level` | the root covers `2^level × 2^level` cells |

`HashlifeViewRequest { x0, y0, width_px, height_px, log2_scale }` →
`HashlifeView { populations, .. }`: the number of live cells in each
`2^log2_scale`-sided block, row-major, saturating at `u32::MAX`. Sides are
clamped to 1–4096 pixels and the scale to at most 64. When `x0` and `y0` are
multiples of `2^log2_scale`, the cost is proportional to the visible non-empty
nodes, not to the population.

## 5. Snapshot

`schema_version = 1`, model id `life.hashlife`. The payload is
`postcard`-encoded and holds:

* the pattern as macrocell text (rule and generation included);
* the step exponent and node limit;
* the seed and the `SimRng` state;
* the reset point (macrocell text and RNG);
* the edited flag and the halt reason.

Restoring validates the step exponent (0–61) and the node limit (the
parameter's range) and reads the pattern into the existing node store, so
memoised results for equal squares are reused. A restored run replays identically,
including later random soups (tested). The memo itself is not serialised; it
is rebuilt on demand and never changes results.

## 6. Approximations and limitations (explicit)

* **No approximation** in the dynamics. The limits below concern what can be
  run, not accuracy.
* **Rules.** Only two-state outer-totalistic Moore rules without `B0`. No
  isotropic non-totalistic (Hensel) or `MAP` rules, no Generations or other
  multi-state rules, no `B0` emulation, no bounded or toroidal grids. Use the
  dense engines in `cellular-automata` for those.
* **Coordinate range.** Cells may sit anywhere in `i64²`. A step needs all
  live cells within `±2^61` of the origin, so that the result (up to `2^61`
  cells further out) stays inside `i64`. Otherwise it returns
  `CoordinateOverflow` and the universe is unchanged. One call advances at
  most `2^61` generations; `advance` splits larger requests.
* **Generation counter** is `u64`. Overflow is refused with
  `GenerationOverflow`, checked before anything runs.
* **Population** is cached as a saturating `u64` per node. Patterns with more
  than `2^64 − 1` cells report `u64::MAX` from `population()`; use
  `population_exact()` (`u128`).
* **Halting.** If a tick fails (generation or coordinate overflow), the model
  halts: `is_finished()` becomes true and `halt_reason()` explains why. The
  failed tick changes nothing. Any command, reset or restore clears the halt.
* **Memory.** The node limit is soft. Collections keep the pattern and the
  memo of its nodes. A limit far below the working set makes deep jumps very
  slow, because useful memo is discarded. Automatic collections never move
  nodes (they may run inside a step), so the arena keeps its high-water
  capacity; `Universe::garbage_collect()` and `clear_memo()` compact it and
  return memory to the allocator. `HashlifeSim` exposes no command for this:
  its memory follows the `node_limit` policy. Hard capacity is `2^32 − 2`
  simultaneous nodes (over 128 GiB); exceeding it panics, like `Vec` capacity
  overflow.
* **Chaotic patterns** (soups, explosions) run tens of times slower per
  generation than a dense SWAR engine. Hashlife pays off only when structure
  repeats in space or time.
* **Macrocell I/O.** Two-state files only. `#R` and `#G` are honoured, and
  other `#` lines (comments, `#FRAMES`/`#FRAME` timelines) are ignored and not
  written back. Golly's exact coordinates are preserved.

## 7. References

* R. Wm. Gosper, "Exploiting regularities in large cellular spaces",
  *Physica D* 10 (1984) 75–80.
* T. G. Rokicki, "An Algorithm for Compressing Space and Time", *Dr. Dobb's
  Journal*, April 2006.
* Golly (GPL), `gollybase/hlifealgo.cpp`. It was read for the macrocell
  format, the origin convention and the GC policy. Clean-room: no code copied.
* Project research note: `docs/research/life-engines-hashlife.md`.

## 8. How to test

```text
cargo test -p hashlife                                   # 29 unit + 37 integration tests + 2 doctests
cargo clippy -p hashlife --all-targets -- -D warnings
GOLLY_PATTERNS=/path/to/golly/Patterns \
  cargo test -p hashlife --profile verify -- --ignored golly   # Golly .mc validation (3 tests)
cargo bench -p hashlife                                  # Criterion benchmarks
```

What the tests check is listed in `docs/HASHLIFE.md` §7. In short:

* agreement with an independent naive stepper (six rules, arbitrary `n`,
  every step size), plus randomised sequences of steps, edits, rule changes,
  collections and macrocell round trips under random rules;
* glider, blinker, Gosper-gun growth, R-pentomino and Acorn final populations;
* a `2^40`-generation glider jump;
* exactness of the queries;
* garbage-collection invariance;
* macrocell round trips;
* the `Simulation` contract (parameters, commands, snapshot replay, reset,
  halting, views);
* Golly's own files, including Gotts dots' documented populations and the
  metapixel galaxy meta-grid following B3/S23.
