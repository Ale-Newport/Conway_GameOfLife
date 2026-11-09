# Automata Catalog: Cellular Automata Laboratory and Three-Dimensional Life

Research notes for the **Cellular Automata Laboratory** and **Three-Dimensional Life** modes of Emergence Lab.
For each model the catalog gives the states, neighbourhood, exact update rule, rule-string notations,
canonical patterns (RLE for 2D) and behaviour that can be checked by a unit test.

Status: research document, written 2026-11-09. Nothing here is code; all pattern data are small facts with attribution.

## 0. How claims were verified, and the labels used

Every behavioural claim marked **[SIM]** was reproduced by an independent throw-away reference
simulator (Python + numpy, written from scratch, not derived from Golly code) kept outside the repository in
the session scratchpad (`.../scratchpad/verify/*.py`). These simulators are **not** part of the project. Treat their
numbers as golden test vectors only after the Rust implementation reproduces them as well. Two
implementations agreeing is the bar.

| Label | Meaning |
|---|---|
| **[SIM]** | Reproduced by our reference simulation (exact numbers given). |
| **[SRC]** | Read directly from a primary source (Golly 5.1 source or help, the original paper, OEIS, or the author's post). The source is cited. |
| **[WEB]** | Taken from a secondary web source, such as the LifeWiki mirror or Wikipedia, and not independently re-derived. |
| **UNVERIFIED** | Plausible but neither reproduced nor confirmed in a primary source. |

Local Golly source used: Golly **5.1** (`gui-wx/makefile-gtk: APP_VERSION = 5.1`), cloned in the scratchpad.
Golly is GPL-2.0-or-later (`docs/License.html`). We read it to learn notations and semantics. **No Golly code or
pattern file is copied into the project.** See §8 for licensing.

Coordinate convention used throughout: RLE coordinates, so **x grows to the right and y grows downward**, and
the first RLE row is y = 0. A displacement "(dx, dy)" is in these coordinates.

---

## 1. Shared conventions

### 1.1 Neighbourhoods (2D)

| Name | Cells | Golly suffix | Notes |
|---|---|---|---|
| Moore | 8 | none | The default for B/S rules. |
| von Neumann | 4 orthogonal | `V` | Counts run 0–4. Example: `B13/S012V`. [SRC Golly Help/Algorithms/QuickLife.html] |
| Hexagonal (emulated) | 6 | `H` | Golly ignores the **NE and SW** corners of the Moore neighbourhood: hex neighbours = NW, N, W, E, S, SE. Counts run 0–6. Example: `B245/S3H`. [SRC QuickLife.html] |

### 1.2 Life-like (outer-totalistic, 2-state) rule strings

* `B{digits}/S{digits}`: birth counts and survival counts, Moore neighbourhood. Example: `B3/S23`.
* An older S/B form without letters is still accepted: **`23/3` means S23/B3**. With no `B` or `S` letter,
  Golly treats the first field as survival. [SRC `gollybase/liferules.cpp` ~L1170, comment "no b or s so survival first"]
  This is a parsing trap: `3/23` is **not** Life.
* Golly's canonical output is `B.../S...`, with digits ascending. `_` is accepted for `/` but only in non-totalistic rules.
* Isotropic non-totalistic (Hensel) letters, such as `B2-a/S12`, and `MAP<base64>` rules exist. They are out of scope here
  but worth reserving in the grammar. [SRC QuickLife.html]
* **B0 rules.** Golly emulates them. If the rule contains B0 and Smax, it is converted to the equivalent rule
  `B(8−x for x∉S)/S(8−x for x∉B)`, for example `B0123478/S01234678` (AntiLife) becomes `B3/S23`. If it contains B0 but not Smax, it alternates
  two rules on odd and even generations. [SRC QuickLife.html "Emulating B0 rules"; liferules.cpp L661–680]
  Recommendation: the Rust engine should track a **background phase bit** rather than store infinite live areas.
* **Bounded-grid suffix** (Golly): `B3/S23:T30,20` is a 30×20 torus. `:P` = plane, `:T` = torus (with optional shifts
  `+n`/`-n`), `:K` = Klein bottle (`*` marks the twisted edge pair), `:C` = cross-surface, `:S` = sphere. A dimension of 0 means
  infinite, which is not allowed for K, C or S. The top-left cell of a w×h grid is at (−int(w/2), −int(h/2)). [SRC Help/bounded.html]
  Example from Golly: `Patterns/Life-Like/ice-nine.rle` uses `rule = B25678/S5678:T20,20`.

**Self-complementarity (inversion symmetry) test.** A 2-state Moore rule is invariant under swapping live and dead cells iff
`B = { 8 − k : k ∉ S }`. [SIM: formula checked against brute force on random 64×64 tori for Day & Night, Anneal, Vote and Life]

### 1.3 RLE cell letters

* 2-state: `b` = dead, `o` = live, `$` = end of row (a count before `$` skips rows), `!` = end.
* Multi-state (Golly): `.` = 0, `A`…`X` = states 1…24, `pA`…`pX` = 25…48, `qA`… and so on up to `yO` = 255.
  `o` is also read as state 1 and `b` as state 0. [SRC `gollybase/readpattern.cpp` L294–306, `writepattern.cpp` L68–79]
* The header line is `x = W, y = H, rule = R`. Lines starting with `#` are comments: `#N` name, `#O` author, `#C` comment.

---

## 2. Life-like family (2 states, Moore neighbourhood)

**Update rule** (all entries in this section): let `n` be the number of live Moore neighbours, excluding the cell itself.
`next = (cell == 0 && n ∈ B) || (cell == 1 && n ∈ S)`.

### 2.1 Summary table

| Name | Rule | Self-compl. | Character | Notable / test pattern |
|---|---|---|---|---|
| Life (reference) | B3/S23 | no | complex | glider, etc. |
| HighLife | B36/S23 | no | complex | replicator (§2.2) |
| Seeds | B2/S | no | explosive | every live cell dies each tick (§2.3) |
| Day & Night | B3678/S34678 | **yes** | complex | rocket c/2 p40 (§2.4) |
| Life without Death (Inkspot, Flakes) | B3/S012345678 | no | monotone growth | ladders (§2.5) |
| 2×2 | B36/S125 | no | complex | 2×2-block emulation, c/8 glider (§2.6) |
| Maze | B3/S12345 | no | maze-forming | c/3 wickstretcher (§2.7) |
| Mazectric | B3/S1234 | no | maze-forming, straighter corridors | [WEB] |
| Maze with Mice | B37/S12345 | no | maze plus moving "mice" | [WEB] |
| Replicator | B1357/S1357 | no | linear (XOR) | every pattern replicates (§2.8) |
| Fredkin | B1357/S02468 | yes | linear (XOR) | 9 copies (§2.8) |
| Diamoeba | B35678/S5678 | no | diamond blobs | c/7 ships (large) (§2.9) |
| Morley / Move | B368/S245 | no | complex | 6-cell c/7 diagonal "jellyfish" (§2.10) |
| Anneal (= Vichniac "twisted majority", called **Vote** in CelLab) | B4678/S35678 | **yes** | coarsening | (§2.11) |
| Majority (called "Vote" on LifeWiki, UNVERIFIED) | B5678/S45678 | yes | coarsening | name clash, see §2.11 |
| LongLife | B345/S5 | no | very long oscillators | White Whale p160,000,346 [SRC Golly `Patterns/Life-Like/white-whale.rle` header] |
| 3-4 Life | B34/S34 | no | explosive-ish, many oscillators | [SRC Golly `34-oscillators.rle`] |
| Coral | B3/S45678 | no | slow coral-like growth | [SRC Golly `coral.rle` header] |
| Serviettes / Persian rugs | B234/S | no | explosive symmetric | a 2×2 block produces Persian rugs [SRC QuickLife.html] |
| Live Free or Die | B2/S0 | no | explosive; light-speed ships | [SRC Golly `lfod-flying-rule110-simulator.rle` header] |
| B3578/S23 | B3578/S23 | no | | a clean 2D replicator (Giard 1999) [SRC Golly `b3578s23-2d-replicator.rle`] |

### 2.2 HighLife: B36/S23

* Origin: devised in 1994 by Nathan Thompson. The name comes from Conway. [WEB en.wikipedia.org/wiki/Highlife_(cellular_automaton); conwaylife.njohnston.ca/wiki/HighLife]
* **Replicator** (12 cells, symmetric under 180° rotation [SIM]):

```
#N Replicator (HighLife)
x = 5, y = 5, rule = B36/S23
2b3o$bo2bo$o3bo$o2bo$3o!
```

Verified behaviour [SIM]. Let `R` be the replicator's cell set at generation 0, and `R + (d, d)` the same set shifted by (d, d):

* **Gen 12:** the pattern is exactly `(R − (2,2)) ∪ (R + (2,2))`: two copies with 24 cells in total.
* **Gen 24:** exactly `(R − (4,4)) ∪ (R + (4,4))`. The two inner copies annihilate.
* **In general, at gen 12·n** the pattern is exactly `⋃_{k ∈ Row90(n)} (R + (2k, 2k))`, where `Row90(n)` is the set of live
  positions after n steps of Wolfram rule 90 started from the single cell {0}. Checked for n = 1…16. So the number of
  copies at gen 12·n is `2^popcount(n)`, giving **2^k copies at gen 12·(2^k − 1)** (12 → 2, 36 → 4, 84 → 8, 180 → 16).
  This matches the LifeWiki and Wikipedia statement that "a row of replicators behaves like Rule 90". [WEB]
* Population for gens 0…24 (unbounded grid) [SIM]:
  `12, 20, 17, 30, 20, 28, 34, 38, 26, 17, 16, 22, 24, 40, 34, 60, 31, 39, 43, 42, 34, 24, 24, 22, 24`.
* Other HighLife objects: the **bomber**, a c/6 diagonal p48 spaceship made of a replicator and a blinker (Nathan Thompson). [WEB]
  A c/98 orthogonal spaceship (Fomichev 2015) and its p1728 gun appear in Golly `Patterns/Life-Like/highlife-c98o-spaceship-p1728-gun.rle`. [SRC header]

### 2.3 Seeds: B2/S

* Initially investigated by Brian Silverman and named by Mirek Wójtowicz. [WEB LifeWiki mirror /wiki/Seeds]
* S is empty, so **no cell is ever live in two consecutive generations**. [SIM: random 64×64 tori, 20 trials] There are no still lifes.
* Most patterns explode. Light-speed ("photon") spaceships exist. [WEB]
* **Duoplet**, the smallest oscillator (2 cells, period 2) [SIM]:

```
x = 2, y = 2, rule = B2/S
o$bo!
```

  Gen 1 is the anti-diagonal pair, gen 2 the original pair.
* Collection of the five smallest oscillators (duoplet p2, shiplet p2, and three 4-cell p4 objects), from the LifeWiki mirror [WEB]. Each was re-checked [SIM]: periods 2, 2, 4, 4, 4.

```
x = 33, y = 5, rule = B2/S
22bo$6bo7bo8bo5bobo$bo6bo4bo14bo3bo$o6bo8bo4bo$15bo6bo!
```

* **Photon**, a light-speed spaceship: 4 cells, period 1, speed c orthogonal. [SIM] The pattern below moves (0, −1) every generation:

```
x = 4, y = 3, rule = B2/S
b2o2$o2bo!
```

* Test [SIM]: the horizontal domino `2o!` has populations 4, 6, 8, 14, 8, 8, 8, … for gens 1, 2, 3, …. From then on it is exactly **two photons**
  travelling in opposite directions (±y) at c.

### 2.4 Day & Night: B3678/S34678

* Explored by Nathan Thompson from April 1997, and discussed in detail by David Bell. [WEB LifeWiki mirror]
* **Self-complementary:** for any configuration X on a torus, `step(¬X) = ¬step(X)`. [SIM: 20 random 64×64 tori, all equal; formula in §1.2 holds.]
  A practical consequence: dead islands in a live sea behave exactly like live islands in a dead sea.
* **Rocket**, the first spaceship found in the rule: 170 cells, **period 40, displacement (−20, 0)**, so it travels at c/2 [SIM]:

```
x = 43, y = 13, rule = B3678/S34678
7b2o$8bo$4bo3bo3b2o$3b2ob8o9b2o$2b11obo3b2ob6o13b2o$b13o2b11o7bo4b3o$
26o10b5obo$b13o2b11o7bo4b3o$2b11obo3b2ob6o13b2o$3b2ob8o9b2o$4bo3bo3b2o$
8bo$7b2o!
```

* Golly ships `Patterns/Life-Like/Day-and-Night-gun-and-antigun.rle` (David Bell 2009): a p400 rocket gun crossing a day/night border. [SRC]

### 2.5 Life without Death: B3/S012345678

* Also called Inkspot (Toffoli & Margolus 1987) and Flakes. [WEB en.wikipedia.org/wiki/Life_without_Death]
* Cells never die, so **population is monotonically non-decreasing** and no oscillators or spaceships exist. [SIM: monotone over 50 generations on a random torus]
* Ladders: the most common ladder grows at c/3 (4 cells per 12 generations). Dean Hickerson found 4c/9, 4c/10 and 4c/13 ladders (2000). [WEB, UNVERIFIED in sim]
* P-complete (Griffeath & Moore 1996). [WEB]
* Test: every Life still life is also a still life here, since its live cells survive and no dead cell has exactly 3 live neighbours.

### 2.6 2×2: B36/S125

* Named for the property that patterns made of 2×2 blocks keep evolving as 2×2 blocks. [WEB LifeWiki mirror /wiki/2x2]
* **Precise verified form** [SIM]: if every 2×2 cell on the **even lattice** (block origins at even x and even y) is uniformly
  live or dead at gen 0, then at every **odd** generation the pattern is uniform on the **odd lattice** (origins (1,1) mod 2),
  and at every **even** generation it is uniform on the even lattice again. Checked over 50 random block patterns × 40 generations.
  The block lattice is **not** fixed: it alternates by (1,1) every tick.
* **c/8 diagonal glider**, the only spaceship known to arise from soup [WEB]. 6 cells, **period 8, displacement (−1, +1)** [SIM]:

```
x = 5, y = 4, rule = B36/S125
3bo$obo2$2b3o!
```

### 2.7 Maze: B3/S12345 (and Mazectric B3/S1234)

* Soups grow maze-like walls and corridors. [WEB] No spaceships are known in Maze. [WEB]
* **c/3 wickstretcher** (LifeWiki mirror) [WEB]. **Re-verified** [SIM]: its x-extent grows by exactly 4 cells and its population by
  16 every 12 generations (bounding box x ∈ [0, 9 + 4k] at gen 12(k + 1); population 37 + 16k). The height stays 8 rows.

```
x = 6, y = 8, rule = B3/S12345
2o$b3o$bob2o$3b3o$3b2o$bobo$b3o$2o!
```

### 2.8 Replicator: B1357/S1357. Fredkin: B1357/S02468

* For **Replicator**, birth and survival sets are equal and contain the odd counts, so `next(c) = XOR of the 8 Moore neighbours`,
  independent of c. The rule is linear over GF(2).
* For **Fredkin**, `next(c) = XOR of the full 3×3 block (9 cells, including c)`.
* Because the rule is linear and the field has characteristic 2, after **2^k** generations any finite pattern P becomes a superposition of shifted
  copies, and the copies are disjoint once 2^k exceeds P's diameter [SIM]:
  * **Replicator: 8 copies** at offsets (±2^k, 0), (0, ±2^k), (±2^k, ±2^k). A single cell after 2, 4, 8, 16 and 32 generations
    gives exactly 8 cells. An arbitrary 5-cell pattern inside a 3×3 box after 8 generations equals the union of its 8 shifted copies.
  * **Fredkin: 9 copies**, the same 8 plus the original position.
* Golly's `Patterns/Life-Like/replicator.rle` (rule B1357/S1357) says "All patterns in this rule replicate." [SRC]
* Name clash: some sources call B1357/S02468 "Replicator" or "Fredkin's replicator". Store both rules explicitly by rule string.

### 2.9 Diamoeba: B35678/S5678

* Large soups form big oscillating diamonds. Small soups often die out. There are no still lifes. [WEB]
* Known orthogonal spaceships: c/7 (first one by David Eppstein 1999, 144 cells; smallest known 112 cells, 2023), c/9, c/14 and c/16.
  All are large, so no small RLE is given. [WEB, UNVERIFIED in sim]
* Its black/white reversal is B45678/S4678. [WEB, consistent with §1.2 formula: S' = {8−k : k ∉ B} = {4,6,7,8}, B' = {8−k : k ∉ S} = {4,5,6,7,8}]

### 2.10 Morley / Move: B368/S245

* Named after Stephen Morley, who studied it extensively. MCell calls it "Move". [WEB LifeWiki mirror /wiki/Morley]
* **Jellyfish**, the smallest c/7 diagonal spaceship (apgcode `xq7_137`) [WEB]. Decoding `137` (one base-32 digit per column,
  bit k = row k) gives the 6-cell triangle below. **Period 7, displacement (−1, +1)** [SIM]:

```
x = 3, y = 3, rule = B368/S245
3o$b2o$2bo!
```

* **6-cell predecessor of a 13c/170 puffer** (Alan Hensel 1993) [WEB]. Verified [SIM]: its leftmost x is −13k and its population
  is 12 + 8k at gen 170k (k = 1…6), so it grows linearly forever.

```
x = 2, y = 6, rule = B368/S245
o$bo$bo$bo$bo$o!
```

* Golly ships Morley patterns by Stephen Morley: `Patterns/Life-Like/Morley/{breeder2,enterprise-gun,growing-ship}.rle`. [SRC]
  Note that `breeder2.rle` writes the rule as `S245/B368`. A parser must accept both letter orders.

### 2.11 Anneal / Vote / Majority (Vichniac)

* CelLab (Rucker & Walker) defines rules by a 10-bit totalistic code over the **NineSum** (8 neighbours plus self, 0…9). The leftmost
  bit is NineSum 9. **Vote = 1111010000b = 976**: next state 1 iff NineSum ∈ {4, 6, 7, 8, 9}. **Majority = 1111100000b = 992**:
  next state 1 iff NineSum ≥ 5. [SRC fourmilab.ch/cellab/manual/chap4.html]
* Converted to B/S: a dead cell has NineSum = n, so it is born for n ∈ {4, 6, 7, 8}. A live cell has NineSum = n + 1, so it survives for n ∈ {3, 5, 6, 7, 8}.
  So **CelLab "Vote" (twisted majority) = B4678/S35678**, which is the rule LifeWiki and MCell call **Anneal**. Plain Majority
  = B5678/S45678, which LifeWiki reportedly calls "Vote" (from memory, UNVERIFIED). **Use rule strings, not names, as identifiers.**
* Vichniac compared the dynamics to surface tension and to annealing: isolated regions shrink and sharp curvature
  is rounded off. [SRC CelLab chap. 4]
* Both rules are **self-complementary** [SIM].

---

## 3. Generations family (multi-state "decay" rules)

### 3.1 Semantics (exact, from Golly source)

States: `0` = dead, `1` = alive or firing, `2 … C−1` = dying or refractory. **Only state-1 cells count as neighbours.**

```
n = number of Moore (or V / H) neighbours in state 1
if   cell == 0:            next = (n ∈ B) ? 1 : 0
elif cell == 1:            next = (n ∈ S) ? 1 : (C > 2 ? 2 : 0)
else /* 2 ≤ cell ≤ C−1 */: next = (cell + 1 < C) ? cell + 1 : 0
```

[SRC `gollybase/generationsalgo.cpp` `slowcalc`, L25–52] A dying cell cannot be re-born or re-stimulated. It must decay to 0 first.
With C = 2 the rule is exactly the life-like rule. [SIM: `23/3/2` equals `B3/S23` for 30 generations on a random 20×20 soup]

### 3.2 Notations (important for the parser)

| Form | Example (Brian's Brain) | Used by | Order |
|---|---|---|---|
| Golly / MCell canonical | `/2/3` | Golly (all versions), MCell, Golly's `Patterns/Generations/*.rle` | **S / B / C** |
| B/S letters, digits-only count | `B2/S/3` | Golly accepts this (letters may be in either order); LifeWiki tables; Wikipedia | B and S by letter |
| LifeWiki "C" form | `B2/S/C3` | LifeWiki, LifeViewer | B and S by letter; C = number of states |
| LifeWiki "G" form | `B2/S/G3` | LifeWiki (alternative) | same as above |
| CelLab NLUK | "Brain 1 2 2" | CelLab | refractory count N, then firing-sum bounds L and U [SRC CelLab chap. 4] |

Facts from **Golly 5.1 source** (`generationsalgo.cpp` `setrule`, L751–1100) [SRC, by reading the code, not by running Golly]:

* The rule must contain **exactly two slashes**. The third field is a decimal state count from **2 to 256**.
* After the second slash **only digits** are allowed. A `C` or `G` prefix fails with "Bad character found." So Golly 5.1 itself
  **does not** accept `B2/S/C3`. Normalise C and G forms in our own parser.
* With no B or S letters the order is **S/B/C** (`345/2/4` is S345, B2, 4 states). With letters, B and S may come in either order.
* The neighbourhood suffix `H` or `V` must be the last character, for example `/2/3V`. The canonical output is `S/B/C` followed by H or V, and B0 is rejected
  ("Generations does not support B0").
* Isotropic non-totalistic letters and `MAP...` (for example `MAP<base64>/3`) are also supported.

LifeWiki mirror: "Bx/Sy/Cn and Bx/Sy/Gn are also sometimes used, with 'C' standing for 'Count' (of cell states), and 'G' for 'Generations'." [WEB conwaylife.njohnston.ca/wiki/Generations]

**Recommended canonical form for Emergence Lab:** `B{b}/S{s}/C{n}` with an optional neighbourhood suffix. The parser should also accept
`S/B/n` (no letters), `B/S/n`, `B/S/Cn` and `B/S/Gn`. Always print the canonical form.

**Golly RLE state letters:** `A` = state 1 (alive or firing), `B` = state 2, `C` = state 3, and so on (§1.3).

### 3.3 Named Generations rules

Sources: Golly `Help/Algorithms/Generations.html`, Golly `Patterns/Generations/*.rle` headers, and the LifeWiki mirror. [SRC/WEB]

| Name | Golly S/B/C | LifeWiki B/S/C | Author / character |
|---|---|---|---|
| Brian's Brain | `/2/3` | B2/S/C3 | Brian Silverman, chaotic |
| Star Wars | `345/2/4` | B2/S345/C4 | Mirek Wójtowicz, "exploding" |
| Banners | `2367/3457/5` | B3457/S2367/C5 | Wójtowicz |
| Bloomerang | `234/34678/24` | B34678/S234/C24 | John Elliott, expanding |
| Caterpillars | `124567/378/4` | B378/S124567/C4 | Wójtowicz |
| Cooties | `23/2/8` | B2/S23/C8 | Rudy Rucker |
| Fireworks | `2/13/21` | B13/S2/C21 | John Elliott |
| Frogs | `12/34/3` | B34/S12/C3 | Scott Robert Ladd |
| Lava | `12345/45678/8` | B45678/S12345/C8 | Wójtowicz |
| Lines | `012345/458/3` | B458/S012345/C3 | Anders Starmark, stable |
| Sticks | `3456/2/6` | B2/S3456/C6 | Rudy Rucker |
| Transers | `345/26/5` | B26/S345/C5 | John Elliott |
| Xtasy | `1456/2356/16` | B2356/S1456/C16 | John Elliott |
| 9:43 at Knight (pattern) | `237/34578/4` | B34578/S237/C4 | from Golly `9-43-at-knight-rake-gun.rle` |
| Brian 6 | `6/246/3` (derived) | B246/S6/C3 | [WEB] the LifeWiki row reads B246/S6/3. The S/B/C column was garbled in our fetch, so the Golly form is derived from B/S. **UNVERIFIED** |

### 3.4 Brian's Brain (B2/S/C3, Golly `/2/3`) tests

* States: 0 ready, 1 firing, 2 refractory. A cell fires iff it is ready and **exactly 2** neighbours are firing. A firing cell becomes
  refractory, and a refractory cell becomes ready. [SRC CelLab chap. 4; Golly help]
* [SIM] Over a random soup, **every** state-1 cell is state 2 next tick and **every** state-2 cell is state 0 next tick.
* **Domino spaceship**: 2 firing and 2 refractory cells. Period 1, speed c, moving toward the firing side. [SIM: identical shape displaced
  (+1, 0) every generation for 8 generations]

```
x = 2, y = 2, rule = /2/3
BA$BA!
```

* A 2×2 block of firing cells (no refractory cells) explodes outward. Total non-zero population at gens 1, 2, 3 = 12, 20, 28 [SIM].
* Golly ships `Patterns/Generations/what-a-mess.rle` (Brian's Brain puffer, Wójtowicz 1999) and `steeplechase.rle` (Star Wars p20). [SRC]

### 3.5 Star Wars (B2/S345/C4, Golly `345/2/4`)

States 0, 1, 2, 3 (two dying states). It follows the semantics in §3.1. Golly's pattern set includes `steeplechase.rle` (p20, Wójtowicz)
and the Golly rule-tree file `Rules/StarWars.rule`. No small spaceship has been verified here. **UNVERIFIED**: specific Star Wars ships.

---

## 4. Wireworld

* Created by **Brian Silverman** and included in his program *Phantom Fish Tank*. Publicised by A. K. Dewdney in "Computer Recreations",
  *Scientific American*, January 1990. [SRC Golly `Patterns/WireWorld/circuit.mcl` comments; `Rules/WireWorld.rule`] Wikipedia dates its creation to 1987. **UNVERIFIED** (not checked).
* **States (Golly numbering):** `0` empty, `1` electron head, `2` electron tail, `3` conductor (wire). Golly RLE letters are `.`, `A`, `B`, `C`.
  The default Golly colours are 0 dark grey (48,48,48), 1 light blue (0,128,255), 2 white (255,255,255) and 3 orange (255,128,0). [SRC `Rules/WireWorld.rule` @NAMES/@COLORS]
* **Rule** (Moore neighbourhood):
  * empty → empty
  * head → tail
  * tail → conductor
  * conductor → head iff **exactly 1 or 2** of its 8 neighbours are heads, otherwise conductor
* Golly also reads the MCell `.mcl` WireWorld files directly. `#GOLLY WireWorld` names the rule, and the `#L` lines are RLE-like data using
  `.`, `A`, `B`, `C` with the same numbering. [SRC `Patterns/WireWorld/*.mcl`]

### 4.1 Verified patterns (own layouts, written for this catalog)

All outputs were simulated with electrons injected at time 0 as shown. "Output at gen t" means the far-end output cell becomes a head at generation t. [SIM]

| Pattern | RLE (`rule = WireWorld`) | Verified behaviour |
|---|---|---|
| Straight wire | `BA8C!` | The head moves +1 cell per generation and reaches x = 9 at gen 8. |
| **Diode** (passes left → right) | `3.2C$BA2C.6C$3.2C!` | Forward: one clean electron reaches x = 10 at gen 9, with no extra delay. Reverse (electron injected at the right end as `...6CAB`): **nothing** reaches x = 0 within 40 generations. |
| **Clock**, 8-cell ring | `.BAC$C3.9C$.3C!` | Output pulses at gens 10, 18, 26, … with **period 8**. |
| **Clock**, 12-cell ring | `.CBA$C3.C$C3.7C$C3.C$.3C!` | Pulses at gens 7, 19, 31, … with **period 12**. Generalisation [SIM]: a corner-cut rectangular ring with one electron emits with period equal to the ring's cell count (6, 8, 12 and 20 were tested). |
| **OR** | `BA3C$5.C$4.8C$5.C$BA3C!` | Output (x = 11, y = 2): A only → 1 pulse at gen 10; B only → 1 pulse; A and B together → **1** pulse; neither → 0. No back-propagation into the other input. |
| **XOR** (ring XOR; same structure as the classic McIntosh/Dewdney gate) | `.B$.A$.C$4C$C.10C$4C$.C$.A$.B!` | Output (x = 11, y = 4): A only → 1 pulse (gen 11); B only → 1 pulse; **A and B together → 0**; neither → 0. Requires synchronous arrival. |

ASCII view of the diode and the XOR (`C` wire, `A` head, `B` tail):

```
Diode (L->R)        XOR (inputs top/bottom, output right)
...CC......         .B..........
BACC.CCCCCC         .A..........
...CC......         .C..........
                    CCCC........
                    C.CCCCCCCCCC
                    CCCC........
                    .C..........
                    .A..........
                    .B..........
```

* **AND**: Golly's `Patterns/WireWorld/gate-AND.mcl` (Harold V. McIntosh) shows five instances. In our simulation [SIM], only the instance whose
  two input electrons arrive **in phase** produces an output pulse (1 pulse). The four instances with one input, or with inputs offset by one
  cell, produce none. We did not design our own AND. **TODO**: build one, for example from XOR + OR via `A∧B = (A∨B) ⊕ (A⊕B)` with matched path lengths, and
  verify it the same way.
* NOT has to be **clocked** in Wireworld: a correctly timed input cancels a clock pulse (McIntosh, `gate-NOT.mcl`). [SRC]

### 4.2 Golly WireWorld files and their licences

`circuit.mcl`, `clocks.mcl`, `gate-*.mcl` (McIntosh), `flip-flop.mcl` (Ian Woollard 1990), `memory-cell.mcl` (Mike Frank 1990),
`NickGardner.mcl` (8-bit multiplier 2002), `NylesHeise.mcl` (32×32-bit multiplier), `unary-multiplier.mcl`, and **`primes.mc`, the Wireworld
computer by David Moore and Mark Owen. Its header says "Copyright 1992, 2008 David Moore and Mark Owen".** Do **not** bundle these in an
MIT/Apache distribution. At most, offer an "import from your Golly folder" feature. See §8.

---

## 5. Langton's Ant and turmites

### 5.1 Langton's Ant (rule string "RL")

* Invented by Chris Langton in 1986. [WEB Wikipedia] Grid cells are white (0) or black (1), and one ant has a position and a heading.
* Step: on a **white** cell turn 90° **right** (clockwise); on a **black** cell turn 90° **left**. Then **flip** the cell's colour and **move** forward one cell.
  The order is turn, flip, move.
* Verified numbers. The grid starts all white and the ant starts on (0,0). [SIM; cross-checked against OEIS]
  * The number of black cells after n moves is **OEIS A255938**: a(0…10) = `0 1 2 3 4 3 4 5 6 7 6`, **a(10000) = 720**, **a(11000) = 834**,
    a(9977) = 715, a(9978) = 714. [SIM equals the OEIS b-file]
  * **Highway:** the sequence of colours the ant reads is exactly periodic with **minimal period 104** from step index
    **9977** (0-based) onward. The ant's displacement is the same over every 104-step window for all t ≥ 9976. The displacement per period is
    **(±2, ±2)**, a diagonal whose sign depends on the initial heading and axis conventions. With heading north and y up, our run gave (−2, −2).
    Each period adds **12** black cells: OEIS gives `a(n+104) = a(n) + 12 for n > 9976`. [SRC OEIS A255938 formula, Zabolotskii 2016; SIM]
  * Wikipedia: "pseudo-random path until 9,977 steps" followed by the 104-step highway. [WEB]
* Theorem: the ant's trajectory is always unbounded (often called the Cohen–Kong theorem). Whether **every** finite initial configuration
  eventually builds a highway is **open**. [WEB Wikipedia]

### 5.2 Multi-colour ants (generalised RL strings)

* A string over {L, R} (and, in Golly's generator, also **U** for U-turn and **N** for no turn) of length n gives n colours. On colour i
  the ant turns `spec[i]` and sets the colour to `(i+1) mod n`. [SRC Golly `Scripts/Python/Rule-Generators/Langtons-Ant-gen.py`]
  Langton's ant is `RL`.
* Golly's generator suggests these examples: `RL` (Langton), `RLR` (chaos), `LLRR` (cardioid), `LRRL` (structure). [SRC same file] Golly includes
  `Patterns/Turmites/LangtonsAnt_LLRR.rle`, which grows a symmetric cardioid (Gale, Propp, Sutherland, Troubetzkoy, "Further Travels with My Ant",
  *Math. Intelligencer* 17 (1995) 48–56). [SRC header]
* Wikipedia examples: `RLR` grows chaotically and it is unknown whether it ever builds a highway; `LLRR` grows symmetrically; `LRRRRRLLR` fills a square;
  `LLRRRLRLRLLR` builds a convoluted highway; `RRLLLRLLLRRR` builds a triangle that starts moving after about 15,900 steps. If the name, read
  cyclically, is made of consecutive identical pairs, the pattern is repeatedly symmetric. [WEB]
* [SIM] `LLRR`: the coloured pattern is exactly mirror-symmetric (vertical or horizontal axis through the bounding-box centre) at 722 of
  the first 20,000 steps, still recurring after step 19,000 (for example at 19,812). `RL` is last symmetric at step 52. This makes a usable recurrence test.

### 5.3 Turmites (2D Turing machines). Golly / Ed Pegg Jr. notation

* Specification: `{{{c', t, s'}, … one triple per colour …}, … one row per internal state …}`. Reading colour c in state s gives
  `(new colour c', turn t, new state s')`. Turn codes form a bit mask: **1 = no turn, 2 = right, 4 = U-turn, 8 = left**. Combining bits, for example 10 = right + left,
  makes the turmite split. Pegg's variant annihilates turmites that meet. [SRC Golly `Scripts/Python/Rule-Generators/Turmite-gen.py`; `Patterns/Turmites/*.rle` headers]
* In this notation Langton's ant is `{{{1,2,0},{0,8,0}}}`, and LLRR is `{{{1,8,0},{2,8,0},{3,2,0},{0,2,0}}}`. [SRC Turmite-gen.py comment]
* **Absolute turmites** use compass moves `'N','E','S','W'` instead of relative turns, for example
  `{{{1,'N',1},{0,'S',1}},…}`. Golly includes `Highway2074575.rle`, a 4-state 2-colour machine by Wolfram that becomes periodic with period
  2,074,575 and displacement (3953, 1912), and `Perfectionist.rle`, which builds a highway after about 15.5 million steps. [SRC headers, UNVERIFIED in sim]
* Golly emulates ants as CA states. Its built-in `Langtons-Ant` rule uses states 0/1 for empty white/black and 2–9 for the ant ×
  {N, E, S, W} × {white, black}. Generated `LangtonsAnt_<spec>` rules encode an ant state as `n_colors + 4·(n_states·c + s) + d`. [SRC]
  **Recommendation:** implement ants and turmites as agents over a colour grid, not as a CA encoding, and only *import* Golly turmite RLE.
* Triangular and hexagonal variants exist (`TriTurmite`, `HexTurmite` generators). One example is a triangular-grid RL ant with a bounded period-92 orbit
  (`TriangularAnt_period92.rle`). [SRC]

---

## 6. Wolfram elementary cellular automata (ECA)

* 1D, 2 states, radius 1. The neighbourhood (l, c, r) is read as a 3-bit number `i = 4l + 2c + r`. **Rule number** `R = Σ_i f(i)·2^i`.
  Listed from neighbourhood 111 (bit 7) down to 000 (bit 0), the outputs are the binary digits of R (Wolfram 1983). [SIM: this convention reproduces OEIS A051023 for rule 30]
* Outputs per neighbourhood [SIM]:

| Rule | 111 | 110 | 101 | 100 | 011 | 010 | 001 | 000 | Closed form |
|---|---|---|---|---|---|---|---|---|---|
| 30 | 0 | 0 | 0 | 1 | 1 | 1 | 1 | 0 | `l XOR (c OR r)` [SIM] |
| 90 | 0 | 1 | 0 | 1 | 1 | 0 | 1 | 0 | `l XOR r` [SIM] |
| 110 | 0 | 1 | 1 | 0 | 1 | 1 | 1 | 0 | |
| 184 | 1 | 0 | 1 | 1 | 1 | 0 | 0 | 0 | traffic: a 1 moves right iff the cell to its right is 0 |

* **Equivalence classes** (mirror l↔r, complement 0↔1, both) [SIM]: 30 ~ 86, 135, 149. 110 ~ 124, 137, 193. 184 ~ 226 (184 is its own
  mirror-complement). 90 and 150 are self-mirror.
* **Behaviour tests** (single live cell, zero background, width ≥ 2T + 3) [SIM]:
  * Rule 30: centre column for t = 0… is `11011100110001011001001110101110…`, matching **OEIS A051023** for t = 0…2000. The pattern grows by exactly one cell per side per step.
    Mathematica used it as a random number generator (Wolfram 1986). [WEB]
  * Rule 90: live count at row n = **2^popcount(n)** (Gould's sequence, OEIS A001316), checked for n ≤ 256. This is the Sierpiński triangle.
  * Rule 110: the right edge stays fixed (001→1 but 100→0) and the left edge advances one cell per step. Rule 110 is Turing-complete (M. Cook,
    "Universality in Elementary Cellular Automata", *Complex Systems* 15 (2004) 1–40). [WEB]
  * Rule 184 on a ring: **the number of 1s is conserved** (checked over 1000 steps). At density 0.3, after 1000 steps there is no `11` pair (free flow). [SIM]
* **Boundary handling options** for the engine. Expose all of them as UI choices:
  1. **Periodic** (ring of width W).
  2. **Fixed 0** (null boundary).
  3. **Fixed 1**.
  4. **Reflective**.
  5. **"Infinite"**: grow the array by one cell per side per step, with an explicit background state. If `f(000) = 1` (odd rule numbers), an infinite 0
     background turns into 1 at the next step. If additionally `f(111) = 0`, the background **alternates**. Track the background bit the same way as for B0 Life (§1.2).
     NKS-style images use a single 1 on a 0 background.

---

## 7. Three-dimensional cellular automata

### 7.1 Neighbourhoods on the cubic lattice

| Name | Size | Offsets | Golly 3D.lua suffix | Softology field 4 | Bays 1987 |
|---|---|---|---|---|---|
| Moore | 26 | all (dx,dy,dz) ∈ {−1,0,1}³ \ 0 | none | `M` | `Life.111` (default) |
| Face (von Neumann) | 6 | exactly one non-zero coordinate | `F` (or `V`) | `N` | `Life.001` |
| Edge | 12 | exactly two non-zero | `E` | – | `Life.010` |
| Corner | 8 | all three non-zero | `C` | – | `Life.100` |
| Hexahedral / FCC ("dense-packed spheres") | 12 | `(0,0,±1), (0,±1,0), (±1,0,0), (0,+1,−1), (0,−1,+1), (+1,0,−1), (−1,0,+1), (+1,−1,0), (−1,+1,0)` | `H` | – | Bays' "universe of dense-packed spheres" |

Sources: Golly `Scripts/Lua/3D.lua` help ("Supported rules"). The H offsets are read from `gui-wx/wxoverlay.cpp`
`Do3DNextGenHexahedral` (comment: "using the top offsets given on page 872" of Bays, *Complex Systems* 1 (1987) 853–875). [SRC]
Bays' corner/edge/face flags are written `Life.xyz R`, with x = corners, y = edges, z = faces. So Life 4555 = `Life.111 4555`. [SRC Bays 1987, §3]
The H set is validated by the FCC oscillator test in §7.3 [SIM].

### 7.2 Notations

1. **Carter Bays (1987)**: `R = (E_l, E_u, F_l, F_u)`, written as four digits, for example "Life 4555". A live cell survives iff
   E_l ≤ n ≤ E_u; a dead cell is born iff F_l ≤ n ≤ F_u (n = live Moore neighbours, 0…26). Conway's Life is `2333` in this notation.
   Two-digit bounds are written with spaces, for example `(10 21 10 21)`. [SRC Bays, "Candidates for the Game of Life in Three Dimensions",
   *Complex Systems* 1 (1987) 373–400, §1.1]
2. **Golly 3D.lua**: `3D{S list}/{B list}{suffix}`. **Survival comes first.** Lists are comma-separated, with ranges `a..b`, for example
   `3D4..7,9/4,5,7`. In the canonical form a run of exactly two is written `a,b` and longer runs `a..b`. **Birth on 0 is not allowed.** The suffix is `F`/`V`,
   `C`, `E` or `H`, and its absence means Moore. Special rules are `BusyBoxes` (`BB`) and `BusyBoxesW` (`BBW`). The default rule is `3D5..7/6` = Bays 5766.
   [SRC `3D.lua` `ParseRule` L367–445 and help]
   * Bays to Golly: `4555 → 3D4,5/5`, `5766 → 3D5..7/6`, `5655 → 3D5,6/5`, `6855 → 3D6..8/5`, FCC `3333 → 3D3/3H`, FCC `4633 → 3D4..6/3H`.
   * There is **no `NN`/`NM` suffix** in Golly. `M`/`N` belong to the Softology notation below.
   * RLE3 file format: first line `3D version=1 size=N pos=x,y,z [gen=g]`, then `#` comments, then
     `x=W y=H z=D rule=...`, then RLE data where `$` = next row (y+1) and **`/` = next plane (z+1)**. [SRC 3D.lua help]
3. **Softology / Visions of Chaos (Jason Rampe)**: `S/B/C/N`. **Survival first**, then birth, then the number of states (including 0),
   then the neighbourhood (`M` = Moore 26; `N` = von Neumann 6, inferred from rule names such as "von Neumann Builder" that use N). Lists use
   `a-b` ranges, for example `9-26/5-7,12-13,15/5/M`. [SRC softologyblog.wordpress.com/2019/12/28/3d-cellular-automata-3/]
   * Semantics are the same as Generations (§3.1): "Only newly born cells are counted"; cells in the refractory period "continue fading out";
     "Only state 0 fully dead empty cells can be born". [SRC author's comment of 2020-04-22]
   * **State labelling conflict in the source:** the post body describes 445's 5 states with the newborn as state 4 counting down to 1,
     while the author's 2021-02-18 comment uses newborn = state 1 and dying states 2 → 3 → 4 → 0. The dynamics are identical. **Use the Golly
     convention (1 = alive, 2…C−1 dying)** and treat display colour as a separate mapping.
   * The author warns that the original YouTube video used **buggy** survival logic (cells in any state could survive), so the published
     rules "do all work as expected with the correct survival logic", not the video's. [SRC]
   * Initial condition recommended by the author: "Initialize the grid with a central small random cluster of state 1 cells." [SRC comment of 2021-02-18]

**Recommended internal representation (Rust):** `Rule3D { survive: u32 /* bit k = count k */, birth: u32, states: u8 (≥2), neighbourhood: enum { Moore26, Face6, Edge12, Corner8, Fcc12 } }`.
Parse all three notations into it and print in Golly or Softology style on request.

### 7.3 Rules with documented behaviour

**Bays' 3D "Game of Life" definition** [SRC Bays 1987]: a rule qualifies iff (1) a glider exists and appears "naturally" from primordial soup,
and (2) all primordial soups show bounded growth. Supporting results:
* Theorem 2: rules with F_l ≥ 10 cannot support a glider.
* Theorem 3: rules with F_l ≤ 4 lead to unbounded growth.
* Of all rules investigated, **only 4555 and 5766** satisfied the definition (1987). Life 5655 and Life 6855 were added later. [SRC Bays,
  "Further Notes on the Game of Three-Dimensional Life", *Complex Systems* 8 (1994) 67–73]
* Bays reports that random soups in a 70³ grid stabilise after about 30–70 generations under both 4555 and 5766. [SRC 1987 §2.3]

**Life 5766 (`3D5..7/6`)**
* Theorem 7 (Bays 1987): a Conway object has a 5766 *analogue*, namely its "expansion" copied into two adjacent z-planes, iff at every generation no
  dead cell near it has 6 live neighbours and no live cell has 5. Conway's glider qualifies. [SRC]
* **5766 glider = Conway glider doubled.** Cells (x,y,z): (1,0,z), (2,1,z), (0,2,z), (1,2,z), (2,2,z) for z ∈ {0,1}, 10 cells. **Period 4,
  displacement (1, 1, 0)**, with 10 cells in every phase [SIM]. It is also the glider that emerged naturally in our 5766 soup search
  (8³ random seed, 1 per about 3,158 soups) [SIM]. Bays found 2,206 of them in about 4 million 23³ experiments. [SRC Bays 1990]
* **Second 5766 glider** (Bays, "The Discovery of a New Glider for the Game of Three-Dimensional Life", *Complex Systems* 4 (1990) 599–602):
  period 8, moves 2 cells parallel to a coordinate axis, populations 20/16/16/22 in phases 0–3, and phases 4–7 mirror 0–3. It was found from a 7³
  centre at 20% density in a 23³ universe. **Cell coordinates UNVERIFIED** (the paper gives only figures).
* Two "time-space barrier" planes 4 apart confine life to two planes. In that configuration all of Conway's Life is embedded in 5766. [SRC 1987 §2.2.1–2.2.2]

**Life 4555 (`3D4,5/5`)**
* "Formed simply by adding 2 to Conway's rule". It has many small symmetric stable and oscillating forms and settles more slowly than 5766. [SRC 1987]
* **4555 glider:** 10 cells, period 4, moves √2 per period, perpendicular to one axis and at 45° to the other two (12 directions). [SRC 1987 §2.3, fig. 8]
  **Coordinates found and verified [SIM]**: we reproduced Bays' "soup stirring" procedure (40 random cells, stirred with rule 4512 for g ∈ [14,27]
  generations, then run under 4555). Across 888 stirred soups (8 parallel runs of 6 minutes each) it gave 17 distinct glider sightings (phases and orientations). All had period 4 and 10 cells, and all moved one cell along each of two axes per period. One phase:

```
(x,y,z): (0,0,1) (0,0,2) (0,1,1) (0,1,2) (1,0,0) (1,0,3) (1,1,0) (1,1,3) (2,0,1) (2,0,2)
-> under 3D4,5/5 (Moore): 10 cells in every phase, period 4, displacement (-1,-1,0)
```

  Direct 8³ random soups (3,189 of them) produced **no** 4555 glider. Stirring matters.
* Bays 1994 adds two more (rare) 4555 gliders, found with "symmetric primordial soup". Their coordinates are UNVERIFIED.

**Life 5655 and Life 6855**: these are life-supporting by Bays 1994, which includes a period-7 5655 glider that moves one diagonal cell per period.
**UNVERIFIED** coordinates.

**FCC / dense-packed spheres (12 neighbours, suffix H)**
* Bays 1987 §4: on that lattice, rules with F_l ≥ 4 cannot support gliders and rules with F_l ≤ 2 grow without limit, so a "Life" must have the form
  `E_l E_u 3 3`. **Life 3333** has a 7-cell two-phase glider (the "tadpole") and a 10-cell "frog". [SRC] Bays 1988 found a 12-phase glider for **(4,6,3,3)**
  that moves 3√3 per cycle. [SRC Bays, "A Note on the Discovery of a New Game of Three-dimensional Life", *Complex Systems* 2 (1988) 255–258]
* **Test pattern:** Golly `Patterns/Three-Dimensional/p126-christmas-tree.rle3` (Brian D. Eubanks, 2018), 364 cells, rule `3D3/3H`. [SIM with the H
  offsets above]: at **gen 63** it equals the original **point-reflected** (x,y,z → −x,−y,−z); at **gen 126** it is identical, with no drift.

**Rokicki's rule `3D4,7/5,8`** (Moore, 2 states)
* It has a 12-cell 1D replicator (Rokicki 2018). The (10,0,0)c/10 spaceship below appears in the 3D.lua help (Andrew Trevorrow, April 2018), 14 cells.
  [SIM]: period **10**, displacement **(+10, 0, 0)**, so it moves at light speed.

```
3D version=1 size=40 pos=19,18,18
x=2 y=4 z=4 rule=3D4,7/5,8
$bo$bo/bo$bo$bo$oo/oo$bo$bo$bo/$bo$bo!
```

**Busy Boxes** (Fredkin and Miller): a reversible 3D CA with 6 phases and a parity-dependent knight-move swap rule. It is described in 3D.lua and
arXiv nlin/0501022. Out of scope for the first version. [SRC]

### 7.4 Softology rules (Jason Rampe, Visions of Chaos)

Rules quoted exactly from the 2019-12-28 post. [SRC] "Credit" is as printed.

| Name | `S/B/C/N` | Credit |
|---|---|---|
| 3D Brain | `/4/2/M` | Rampe |
| 445 | `4/4/5/M` | Rampe |
| Amoeba | `9-26/5-7,12-13,15/5/M` | Rampe |
| Architecture | `4-6/3/2/M` | Rampe |
| Builder 1 | `2,6,9/4,6,8-9/10/M` | Rampe |
| Builder 2 | `5-7/1/2/M` | Rampe |
| Clouds 1 | `13-26/13-14,17-19/2/M` | Rampe |
| Clouds 2 | `12-26/13-14/2/M` | Rampe |
| Construction | `0-2,4,6-11,13-17,21-26/9-10,16,23-24/2/M` | Rampe |
| Coral | `5-8/6-7,9,12/4/M` | Rampe |
| Crystal Growth 1 | `0-6/1,3/2/N` | Rampe |
| Crystal Growth 2 | `1-2/1,3/5/N` | Rampe |
| Diamond Growth | `5-6/1-3/7/N` | Rampe |
| Expanding Shell | `6,7-9,11,13,15-16,18.6-10,13-14,16,18-19,22-25/5/M` (as printed; the `.` is almost certainly a typo for `/`, giving S = `6-9,11,13,15-16,18`, B = `6-10,13-14,16,18-19,22-25`). **UNVERIFIED** | Rampe |
| More Structures | `7-26/4/4/M` | Rampe |
| Pulse Waves | `3/1-3/10/M` | Rampe |
| Pyroclastic | `4-7/6-8/10/M` | Rampe |
| Sample 1 | `10-26/5,8-26/4/M` | Rampe |
| Shells | `3,5,7,9,11,15,17,19,21,23-24,26/3,6,8-9,11,14-17,19,24/7/M` | Rampe |
| Single Point Replication | `/1/2/M` | Rampe |
| Slow Decay 1 | `13-26/10-26/3/M` | Rampe |
| Slow Decay 2 | `1,4,8,11,13-26/13-26/5/M` | Rampe |
| Spiky Growth | `0-3,7-9,11-13,18,21-22,24,26/13,17,20-26/4/M` | Rampe |
| Stable Structures | `13-26/14-19/2/M` | Evan Wallace |
| Symmetry | `/2/10/M` | Rampe |
| von Neumann Builder | `1-3/1,4-5/5/N` | Rampe |

Explanation in the post: Clouds 1 means "alive cells with 13…26 neighbours survive; empty cells with 13, 14, 17, 18 or 19 neighbours are born; 2 states". [SRC]
Most rules "were found by trying multiple random rules until something interesting appeared". [SRC]

**Deterministic test vectors** (single state-1 seed cell at the centre of an empty, sufficiently large grid; Generations semantics; populations count
all non-zero cells unless noted) [SIM, our implementation only, so cross-check before freezing]:

| Rule | Gens 1…12 population |
|---|---|
| Crystal Growth 1 `0-6/1,3/2/N` | 7, 13, 43, 49, 79, 133, 259, 313, 343, 397, 523, 625. Monotone: S contains 0–6, so nothing ever dies. |
| Crystal Growth 2 `1-2/1,3/5/N` | 7, 13, 43, 48, 72, 120, 216, 264, 264, 264, 264, 312 (state-1 counts: 6, 6, 30, 6, 30, 54, 126, 54, 30, 54, 126, 102) |
| von Neumann Builder `1-3/1,4-5/5/N` | identical to Crystal Growth 2 for these 12 generations |
| Diamond Growth `5-6/1-3/7/N` | 7, 25, 63, 129, 231, 376, 568, 808, 1096, 1432, 1816, 2248 |
| Single Point Replication `/1/2/M` | 26, 8, 208, 8, 208, 64, 1664, 8, 208, 64, 1664, 64. **After 2^k generations, exactly 8 cells at (±2^k, ±2^k, ±2^k)**, checked for k = 1…4. This is the 3D B1/S analogue of §2.8. |
| Builder 2 `5-7/1/2/M` | 26, 16, 152, 88, 176, 400, 920, 544, 512, 1264, 1568, 2440 |
| Pulse Waves `3/1-3/10/M` | 27, 71, 235, 327, 827, 1087, 2643, 2879, 4674, 5940, 8828, 10596 |
| 445 `4/4/5/M` | 1, 1, 1, 0, … A lone cell decays through states 2, 3, 4 and then dies. |
| 3D Brain `/4/2/M`, 2×2×1 seed | 8, 16, 12, 24, 64, 32, 16, 32, 24, 48, 128, 64 |

**Observed qualitative behaviour** (our simulation, 64³ grid, dead boundary, numpy seed 42; a seed cube of side 6 or 12 at 50% density in the
centre, or a full-volume random fill). This is **implementation- and seed-specific**: use it to choose demo presets, not as golden tests.
* **Clouds 1 / Clouds 2 / Stable Structures** need a **dense, full-volume** random fill. A small central seed dies within 3–10 generations.
  The result depends sharply on density: at 30% fill everything dies by gen 4–5. At 50% fill the population falls to about 10–14% of the volume
  by gen 60 (Clouds 1 9.8%, Clouds 2 11.0%, Stable Structures 13.9%). At 60% fill, Clouds 1 and Stable Structures keep about 76–80% of the volume, while Clouds 2
  keeps about 26%. These behave like cave-generation smoothing rules. We did not inspect the morphology visually.
* **445**: a small central seed produces chaotic growth that reaches the boundary and then dies out (gen 42 with a side-6 seed, gen 99 with side 12). A full
  random fill at 50% or more dies within about 10 generations.
* **Amoeba, Coral, Pyroclastic, Pulse Waves, Symmetry, 3D Brain, Architecture, Builder 2, Crystal Growth 1/2, von Neumann Builder, Sample 1**:
  from a central seed they grow until they hit the boundary. The growth speed is light speed (about 1 cell per generation) for Crystal Growth, Pulse Waves, Symmetry and
  Single Point Replication, and slower for Amoeba, Coral and Pyroclastic (boundary at about gen 50–83).
* **Builder 1**: grows, then partially decays. With a side-6 seed it froze into a 12-cell still life by gen 84. With a side-12 seed, about 11k non-zero cells were still active at gen 150.
* **More Structures**: small, slowly changing structures (about 300 cells with the side-6 seed and about 1,000 with side 12, at gen 150).
* **Spiky Growth and Slow Decay 2** froze into small still lifes (14 or 99 cells, and 5 or 6 cells). **Slow Decay 1** died out from a central seed, but from a full fill
  at 50% or more it filled about 98% of the volume.
* **Diamond Growth**: grew to the boundary, then everything died (gen 85–95).
* **Bays 4555 and 5766**: small soups leave a few stable cells or nothing, consistent with Bays' "bounded growth".

---

## 8. Licensing notes (for patterns and data)

* **Golly** (source, help, `Patterns/`, `Rules/`, `Scripts/`): the distribution is under **GPL-2.0-or-later** (`docs/License.html`). Pattern
  files carry individual authors in `#O` and `#C` lines and have **no separate licence grant**. Do **not** copy Golly code into the MIT/Apache project.
  **Do not redistribute Golly pattern files.** Support importing `.rle`, `.mc`, `.mcl` and `.rle3` from a user-supplied Golly folder instead.
* **`Patterns/WireWorld/primes.mc`** carries an explicit "Copyright 1992, 2008 David Moore and Mark Owen". Do not bundle it.
* **Tiny canonical objects** (the replicator, gliders, the jellyfish, the domino ship, the 4555 and 5766 gliders): these are a handful of
  cell coordinates discovered by the named people. We restate them as facts, with attribution. The Wireworld diode, clocks, OR and XOR in §4.1
  are our own layouts, verified by simulation (the XOR uses the same well-known ring structure). Whether a few-cell configuration is copyrightable is a legal question. This is **not legal advice**, so get sign-off before shipping a bundled pattern library.
* **LifeWiki** text and pattern pages: the content licence was **not checked** (UNVERIFIED). We relied on the read-only mirror
  `conwaylife.njohnston.ca`, because conwaylife.com returns HTTP 403 (Cloudflare challenge) to automated fetches.
* **OEIS** sequence data (A255938, A051023, A001316) are used as numeric test vectors and cited. OEIS content is under CC BY-SA 4.0. This is from memory and was not re-checked here: **UNVERIFIED**.
* **Softology** rule strings and **Bays** rule numbers are short factual identifiers, quoted with citation.

---

## 9. Consolidated unit-test checklist (Rust)

Each line is self-contained. "Exact" means set equality of live cells, after translation where stated.

1. **Parser:** `"23/3"` → B{3}/S{2,3}. `"B3/S23"` = `"S23/B3"` = `"b3/s23"`. Morley `"S245/B368"` = `"B368/S245"`. Generations `"/2/3"` = `"B2/S/3"` =
   `"B2/S/C3"` = `"B2/S/G3"`, all printing canonically as `"B2/S/C3"`. 3D: `"3D5..7/6"` = Bays 5766 = Softology `"5-7/6/2/M"`. `"3D4,5/5"` = Bays 4555.
   Reject birth-on-0 in 3D (Golly rule) unless a background-phase engine exists.
2. **HighLife replicator** (§2.2): gen 12 = R±(2,2) exactly; gen 12n = the Rule-90 superposition (n = 1…16); copy counts 2, 2, 4, 2, 4, 4, 8, 2 for n = 1…8;
   population vector for gens 0…24.
3. **Seeds:** no cell live in two consecutive generations (random soup). Duoplet has period 2. Photon `b2o2$o2bo!` moves (0,−1) per generation. The domino `2o!` becomes two photons with populations 4, 6, 8, 14, 8, 8, ….
4. **Day & Night, Anneal (B4678/S35678), Majority (B5678/S45678), Fredkin:** `step(¬X) == ¬step(X)` on a torus. Life must fail this test.
5. **Life without Death:** population is non-decreasing. Life still lifes stay fixed.
6. **2×2:** the 2×2-block lattice alternates between offsets (0,0) and (1,1) every tick. The c/8 glider has period 8 and displacement (−1,+1).
7. **Replicator / Fredkin:** a single cell after 2^k generations has 8 / 9 cells at the offsets listed in §2.8. An arbitrary 3×3 pattern after 8 generations equals the union of its copies.
8. **Maze wickstretcher:** +4 width and +16 population per 12 generations.
9. **Morley:** the jellyfish `3o$b2o$2bo!` has period 7 and displacement (−1,+1). The puffer predecessor's leftmost x is −13k at gen 170k.
10. **Day & Night rocket:** period 40, displacement (−20,0), 170 cells.
11. **Generations semantics:** `23/3/2` equals `B3/S23`. Brian's Brain: 1→2 and 2→0 always. The domino ship `BA$BA!` moves +1 x per generation. A 2×2 firing block has populations 12, 20, 28.
12. **Wireworld:** wire speed 1 cell per generation; the diode passes forward and blocks reverse; clocks have periods 8 and 12; the OR truth table; the XOR truth table (§4.1).
13. **Langton's ant:** black-cell count after n moves, compared with OEIS A255938 (720 at n = 10000, 834 at n = 11000). The read-colour sequence is periodic with period 104 from index 9977. `a(n+104) − a(n) = 12` for n > 9976. The displacement per period has |dx| = |dy| = 2.
14. **LLRR ant:** mirror-symmetric at recurring step counts, including at least one step > 19,000.
15. **ECA:** rule 30 centre column = OEIS A051023 (first 2,001 terms). Rule 90 popcount = 2^popcount(n). Rule 110 has a fixed right edge and a left edge advancing at speed 1. Rule 184 conserves the count of 1s on a ring. Mirror and complement of 30 → 86, 135, 149.
16. **3D:** the 5766 doubled Conway glider has period 4 and displacement (1,1,0). The 4555 glider (§7.3) has period 4 and displacement (−1,−1,0), with 10 cells in every phase. `3D4,7/5,8` ship: period 10, displacement (10,0,0). The FCC `3D3/3H` p126 tree is point-reflected at gen 63 and identical at gen 126. Softology single-seed vectors (§7.4). B1/S in 3D: a single cell gives 8 cells at (±2^k)³.

---

## 10. Sources

Primary, local (Golly 5.1 clone, GPL-2.0-or-later; read only):
* `Help/Algorithms/QuickLife.html`, `Help/Algorithms/Generations.html`, `Help/bounded.html`, `docs/License.html`
* `gollybase/generationsalgo.cpp` (Generations semantics and parser), `gollybase/liferules.cpp` (S/B ordering, B0),
  `gollybase/readpattern.cpp` and `gollybase/writepattern.cpp` (multi-state RLE)
* `Rules/WireWorld.rule`, `Patterns/WireWorld/*.mcl`, `Patterns/WireWorld/primes.mc`
* `Patterns/Life-Like/*`, `Patterns/Generations/*`, `Patterns/Turmites/*`, `Patterns/Other-Rules/Langtons-Ant.rle`
* `Scripts/Python/Rule-Generators/{Langtons-Ant-gen.py, Turmite-gen.py}`
* `Scripts/Lua/3D.lua` (3D notation, RLE3, references), `gui-wx/wxoverlay.cpp` (3D neighbourhood implementations)
* `Patterns/Three-Dimensional/{p126-christmas-tree.rle3, rokicki-replicator-rule.rle3}`

Papers and web:
* C. Bays, "Candidates for the Game of Life in Three Dimensions", *Complex Systems* 1 (1987) 373–400. https://wpmedia.wolfram.com/sites/13/2018/02/01-3-1.pdf
* C. Bays, "A Note on the Discovery of a New Game of Three-dimensional Life", *Complex Systems* 2 (1988) 255–258. https://wpmedia.wolfram.com/sites/13/2018/02/02-3-1.pdf
* C. Bays, "The Discovery of a New Glider for the Game of Three-Dimensional Life", *Complex Systems* 4 (1990) 599–602. https://wpmedia.wolfram.com/sites/13/2018/02/04-6-2.pdf
* C. Bays, "Further Notes on the Game of Three-Dimensional Life", *Complex Systems* 8 (1994) 67–73. https://wpmedia.wolfram.com/sites/13/2018/02/08-1-4.pdf
* C. Bays, "Patterns for Simple Cellular Automata in a Universe of Dense-Packed Spheres", *Complex Systems* 1 (1987) 853–875 (cited by Golly for the FCC offsets; not downloaded here).
* J. Rampe (Softology), "3D Cellular Automata", 2019-12-28. https://softologyblog.wordpress.com/2019/12/28/3d-cellular-automata-3/
* Rudy Rucker and John Walker, *CelLab* manual, chapter 4 (Vote, Majority, Brian's Brain). https://www.fourmilab.ch/cellab/manual/chap4.html
* OEIS A255938 (Langton's ant black cells). https://oeis.org/A255938 , A051023 (rule 30 centre column). https://oeis.org/A051023 , A001316 (Gould's sequence). https://oeis.org/A001316
* Wikipedia: Langton's ant https://en.wikipedia.org/wiki/Langton%27s_ant ; Highlife https://en.wikipedia.org/wiki/Highlife_(cellular_automaton) ;
  Life without Death https://en.wikipedia.org/wiki/Life_without_Death
* LifeWiki read-only mirror (conwaylife.com itself returns 403 to automated fetches): https://conwaylife.njohnston.ca/wiki/HighLife ,
  /wiki/Seeds , /wiki/Day_%26_Night , /wiki/2x2 , /wiki/Maze , /wiki/Morley , /wiki/Diamoeba , /wiki/Generations
* M. Cook, "Universality in Elementary Cellular Automata", *Complex Systems* 15 (2004) 1–40 (rule 110). Cited, not re-read.
* S. Wolfram, "Statistical mechanics of cellular automata", *Rev. Mod. Phys.* 55 (1983) 601–644 (ECA numbering). Cited, not re-read. The numbering convention was confirmed via OEIS A051023 and simulation.
* D. Gale, J. Propp, S. Sutherland, S. Troubetzkoy, "Further Travels with My Ant", *Math. Intelligencer* 17 (1995) 48–56 (cited via Golly).
