# Simulation models

This is an index of every model in the workspace. Each entry gives:

* the model id(s): the `ModelDescriptor::id`, which save files and the CLI
  use;
* what the model simulates;
* the rules or equations it actually executes;
* what can be reproduced exactly;
* its main limitations;
* a link to the detailed document.

The detailed documents are authoritative. This page only summarises them.

Contracts that every model follows (`docs/ARCHITECTURE.md` §3):

* one `step()` is one tick of fixed simulated duration;
* every external change goes through `apply(Command)`;
* all randomness comes from a seeded `SimRng` stored in the state;
* snapshots include the RNG, so a restored run replays identically.

| Model id | Crate | Laboratory | Detailed doc |
|---|---|---|---|
| `life.classic` | `cellular-automata` | Classic Conway | [models/cellular-automata.md](models/cellular-automata.md) |
| `ca.life_like`, `ca.generations`, `ca.wireworld`, `ca.langtons_ant`, `ca.wolfram` | `cellular-automata` | Cellular Automata Lab | [models/cellular-automata.md](models/cellular-automata.md) |
| `life.hashlife` | `hashlife` | (engine; CLI) | [models/hashlife.md](models/hashlife.md), [HASHLIFE.md](HASHLIFE.md) |
| `life.metapixel` | `metapixel` | Conway within Conway | [models/metapixel.md](models/metapixel.md), [OTCA_METAPIXEL.md](OTCA_METAPIXEL.md) |
| `continuous.lenia`, `continuous.smoothlife`, `continuous.flow_lenia` | `continuous` | Continuous Life | [models/continuous.md](models/continuous.md) |
| `voxel.life` | `voxel-life` | 3D Life | [models/voxel-life.md](models/voxel-life.md) |
| `environment.world` | `physics-world` | (used by the ecosystem; CLI) | [models/physics-world.md](models/physics-world.md) |
| `ecosystem` | `ecosystem` | Ecosystem Simulator | [models/ecosystem.md](models/ecosystem.md), [ECOSYSTEM.md](ECOSYSTEM.md) |
| `custom_life` | `rule-engine` | Custom Life Creator | [models/rule-engine.md](models/rule-engine.md), [RULE_ENGINE.md](RULE_ENGINE.md) |
| `evolution` | `evolution` | Evolution Lab | [models/evolution.md](models/evolution.md) |
| `civilization` | `civilization` | Emergent Civilizations | [models/civilization.md](models/civilization.md) |

Some crates are foundations rather than models and have no `Simulation`
implementation:

| Crate | Document |
|---|---|
| `patterns` | [models/patterns.md](models/patterns.md) |
| `organism-engine` | [models/organism-engine.md](models/organism-engine.md) |
| `analytics`, `persistence` | [models/analytics.md](models/analytics.md) |

They are summarised at the end of this page.

### A note on reproducibility

* The cellular automata (`life.*`, `ca.*`, `voxel.life`) use integer
  dynamics. The docs describe them as exact and bit-for-bit reproducible.
* Every floating-point model replays bit-exactly only on the same platform
  and build. Cross-platform bit-exactness is not promised.

---

## Cellular automata (`cellular-automata`)

### `life.classic` / `ca.life_like`: Life-like rules

**What it simulates.** Two-state, outer-totalistic Moore rules `B…/S…`.
`life.classic` defaults to Conway B3/S23. `ca.life_like` adds lab presets
(HighLife, Seeds, Day & Night, Life without Death, Maze, 2x2, Replicator,
Diamoeba, Morley, Anneal).

**Rule.** A dead cell with *n* live Moore neighbours is born if *n* ∈ B. A
live cell survives if *n* ∈ S. Every other cell is dead in the next
generation.

**Topologies:** bounded, torus, Klein bottle, cross-surface (Golly bounded-grid
semantics) and the infinite plane.

**Engines**

| Engine | How it works |
|---|---|
| Dense | One byte per cell. |
| Bit-packed SWAR | 64 cells per `u64`. |
| Sparse | 64×64 bit tiles. |
| Hashlife | Adapter over the `hashlife` crate. |
| Automatic | Chooses from world size, population, tile regularity and step size. |

Switching engines transfers the live cells exactly, or is refused.

**Determinism.**

* Integer dynamics.
* Rayon only computes pure functions into separate buffers.
* The sparse engine sorts tile keys before any order-dependent step.
* Replay after a snapshot is tested on every engine.

**Limitations**

* No isotropic non-totalistic, hexagonal, von Neumann or MAP rules.
* B0 rules are allowed only on finite worlds, with literal semantics (no
  emulation).
* No Golly shifted tori, tubes or spheres.
* Births and deaths are not reported for Hashlife jumps without tracking.
* Some Hashlife view texels are estimated from block populations.
* Engine switches and tracking are refused above 2^26 live cells.
* The sparse plane ends at the `i64` limits.

### `ca.generations`

**What it simulates.** Multi-state Generations rules `B/S/C` with up to 255
states, for example Brian's Brain and Star Wars.

**Rule** (Golly `generationsalgo`):

* State 0 is dead, state 1 is alive, states 2…C−1 are dying.
* Only state 1 counts as a neighbour.
* A dead cell is born if *n* ∈ B.
* A live cell stays alive if *n* ∈ S; otherwise it becomes state 2 (or 0
  when C = 2).
* Dying states advance by one, and wrap to 0 at C.

**Limitations**

* Moore neighbourhood only.
* At most 255 states.
* No infinite-plane engine.

### `ca.wireworld`

**What it simulates.** Silverman's Wireworld.

**Rule.** There are four states: empty, head, tail and conductor.

* head → tail;
* tail → conductor;
* conductor → head if exactly 1 or 2 of its neighbours are heads.

**Data.** Circuits are in `assets/patterns/wireworld/`. The own-layout circuits
are embedded in the crate. The GPL McIntosh AND gate is read at run time.

**Limitations.** Finite worlds only.

### `ca.langtons_ant`: Langton's ant and turmites

**What it simulates.** Langton's ant and multi-colour turmites.

**Rule.** At each step, every ant in order:

1. turns according to the rule letter of the colour under it (L, R, N for
   none, U for U-turn);
2. advances that cell's colour by one, modulo the number of colours (2–16);
3. moves forward one cell.

**Limitations**

* Ants never annihilate or split.
* Trails keep at most the 2^20 most recent flips.

### `ca.wolfram`: elementary cellular automata

**What it simulates.** The 256 elementary rules, shown as a space-time
diagram.

**Rule.** The new cell is bit (4·l + 2·c + r) of the rule number, where l, c
and r are the left, centre and right cells (Wolfram numbering).

* Boundaries: fixed 0, fixed 1 or periodic.
* The first row is either a single cell or a seeded random row.

---

## `life.hashlife` (`hashlife`)

**What it simulates.** Two-state Life-like rules without B0 on an unbounded
`i64` plane, using Gosper's Hashlife.

**Algorithm.**

* A hash-consed quadtree memoises, for each node of side 2^L, its centre
  square after 2^j generations (j ≤ L−2).
* One tick advances 2^k generations, with `step_exponent` k from 0 to 61.
* `advance(n)` reaches any n exactly by binary decomposition.

**Exactness.** The result is bit-for-bit the synchronous rule. Snapshots
store macrocell text and the RNG. The memo is rebuilt on demand and never
changes results.

**Limitations**

* Only two-state outer-totalistic Moore rules, no B0, no bounded grids.
* All live cells must lie within ±2^61 of the origin. A tick that would break
  this fails with `CoordinateOverflow`, and the model halts with the reason.
* The `node_limit` is soft. Set too low, deep jumps become very slow.
* Chaotic patterns run tens of times slower per generation than the SWAR
  engine.
* Macrocell I/O handles two-state files only.

## `life.metapixel` (`metapixel`)

**What it simulates.** Tilings of Brice Due's OTCA metapixel. Each tile is a
2048 × 2048 region of one B3/S23 pattern and behaves as one cell of a
programmable Life-like macro rule. The model runs the real pattern exactly
through Hashlife.

**Rules and readout**

* Every physical cell follows B3/S23.
* The macro rule B{β}/S{σ} (B0 allowed) is programmed by adding one cell per
  rule digit.
* The neighbourhood is any subset of the 8 Moore directions. It is set by
  removing edge blocks or corner LWSSs.
* Macro states are read at t = (n+1)·35 328 by counting live cells in each
  tile's core:

  | Core count | Reading |
  |---|---|
  | 0 | OFF |
  | 29 795 | ON |
  | anything else | Ambiguous: reported, never rounded |

* A separate *accelerated macro model* applies the macro rule directly. It
  is only a labelled prediction, used to flag mismatches.

**Exactness.** The physical level is exact. Snapshots restore identically.

**Limitations**

* Macro states are only defined at readout times.
* For generations 4320–7407 of each cycle a core may be half-switched.
* The tiles are GPL data read from `assets/patterns/otca/`, and are required.
* Grids are full rectangles of up to 256 × 256 tiles.
* Edits rebuild the construction from generation 0.
* The internal-mechanism narrative in `OTCA_METAPIXEL.md` is marked
  UNVERIFIED.
* Hashlife memory growth over long chaotic runs has not been measured.

---

## Continuous models (`continuous`)

All three models run on a periodic W × H grid.

### `continuous.lenia`: Lenia and Expanded Lenia

**What it simulates.** C channels of continuous state A ∈ [0, 1], following
Chan 2019 and 2020.

**Update**

```text
U_k = K_k * A_src(k)                       (periodic convolution)
D_j = Σ_{k: dst(k)=j} h_k · G_k(U_k)
A_j ← clip(A_j + dt · D_j, 0, 1),          dt = 1/T
```

**Kernels.** Rings β over a radial core. The core is exponential, polynomial,
rectangular, staircase or gaussian_bump. Kernels are normalised so that
Σ K = 1.

**Growth functions**

| Name | G(u) |
|---|---|
| Gaussian | 2·exp(−(u−μ)²/(2σ²)) − 1 |
| Polynomial | 2·1[\|u−μ\| ≤ 3σ]·(1 − (u−μ)²/(9σ²))⁴ − 1 |
| Rectangular | +1 if \|u−μ\| ≤ σ, else −1 |

**Size limits.** Up to 8 channels and 64 kernels.

### `continuous.smoothlife`

**What it simulates.** Rafler's SmoothLife (2011).

**Update**

* m is the inner filling, measured over a disc. n is the outer filling,
  measured over an annulus.
* The transition is s(n, m) = σ2(n, σm(b1, d1, m), σm(b2, d2, m)), built
  from logistic σ1 functions.
* A second "indicators" mixing variant is available.
* Stepping modes:

  | Mode | Update |
  |---|---|
  | Discrete | f ← s |
  | Smooth | f ← f + dt·(2s − 1) |
  | Relax | f ← f + dt·(s − f) |

  The result is then clipped to [0, 1].

### `continuous.flow_lenia`

**What it simulates.** Flow-Lenia (Plantec et al. 2023), with
mass-conserving, non-negative, unbounded channels.

**Update**

1. Affinity: U_j = Σ h_k·G_k(K_k * A_src).
2. Flow: F = (1 − α)∇U − α∇A_Σ, with α = clip((A_Σ/θ_A)^n, 0, 1).
3. Mass moves by reintegration tracking. The displacement is clipped to
   w − s per axis. The square-overlap weights are renormalised to sum
   exactly 1.

**Numerics (all three models)**

* Convolution uses `rustfft` with precomputed kernel spectra, or a direct
  sum over non-zero taps. An automatic mode picks between them with a
  calibrated cost model.
* Fields are `f32`. Kernel construction and sums use `f64`.

**Determinism.**

* Deterministic for a given seed, platform and build.
* Bitwise independent of the thread count (tested).
* Not bit-identical across CPU architectures: the FFT SIMD paths differ.

**Limitations**

* Periodic boundaries only.
* `f32` chaotic dynamics diverge from `f64` after about 1000 steps.
* Lenia features not implemented: quantisation, soft clip, alternative
  integrators, the asymptotic update, and 3-D/4-D worlds.
* SmoothLife: modes 3–4 are not implemented.
* Flow-Lenia: no parameter localisation and no food or decay.
* Rotated or scaled stamps are resampled bilinearly.
* Centroid metrics describe the whole world, not individual creatures.

---

## `voxel.life` (`voxel-life`)

**What it simulates.** Three-dimensional outer-totalistic cellular automata
with Generations-style decay states: Bays' 4555 and 5766, and the Softology
multi-state rules.

**Rule**

* State 0 is empty, state 1 is alive, states 2…C−1 are dying.
* N is the number of neighbours in state 1.
* An empty cell is born if N ∈ B.
* A live cell survives if N ∈ S. Otherwise it becomes 2 (or 0 if C = 2).
* Dying states advance, then return to 0.
* Neighbourhoods: Moore-26 or von Neumann-6.
* Notations accepted: Softology S/B/C/N, letters, Bays E_l E_u F_l F_u, and
  Golly `3D…`.

**Engines and edges**

| Engine | Limits and edges |
|---|---|
| Dense | Up to 2^24 voxels; bounded or torus. |
| Sparse | 16³ chunks; bounded, or unbounded within ±2^30 per axis. |

**Determinism.** No floating point in the dynamics. Bit-for-bit
reproducible. Hash-map order is never used.

**Limitations**

* Edge, corner and FCC neighbourhoods are not supported.
* No B0, Busy Boxes or RLE3.
* Small tori count a neighbour once per offset.
* An unbounded world halts above 32 768 chunks.
* Dense worlds are limited to 256³.
* Age saturates at 65 535.

---

## `environment.world` (`physics-world`)

**What it simulates.** The abiotic world that the ecosystem builds on:

* procedural terrain with drainage, rivers and lakes;
* soils and biomes;
* a compressed calendar with seasons and day/night;
* weather (a Markov chain calibrated to annual rainfall);
* Ornstein–Uhlenbeck wind;
* hydrology: snow, infiltration, groundwater as one linear reservoir, and
  mass-conserving surface routing;
* drought and floods;
* fire spread following Alexandridis et al. 2008;
* landslides and scripted disasters.

Standalone, a fuel proxy stands in for the vegetation.

**Fire spread.** The probability of spreading to a neighbour is

```text
p = p_h (1 + p_veg)(1 + p_den) · e^{c1 V} e^{c2 V (cos θ − 1)} · e^{a θ_s} · f_m · f_I · k
```

Spread decisions are synchronous.

**Determinism**

* Each subsystem has its own `SimRng` stream.
* Rows are processed in parallel as pure functions and reduced in order.
* Fire is processed in index order.
* Reproducible on the same platform and build.

**Limitations**

* Compressed time.
* No erosion during generation.
* Surface routing has no momentum: at most 1 cell per sub-step, and a cap of
  16 sub-steps.
* Groundwater is a single lumped reservoir.
* Evaporation is an empirical index.
* Static precipitation pattern, simplified biomes and a composite fire
  moisture index.
* Fire has no spotting or crown fire.
* Disasters are geometric, not physical.
* One generic nutrient pool.
* About 175 B of memory per cell.

## `ecosystem` (`ecosystem`)

**What it simulates.** The biotic layer:

* 41 animal species with individual state and species-specific utility AI;
* insects, spiders, worms and forage fish as density fields;
* ants and bees as colony super-agents;
* seven vegetation layers, individual trees, fungi, carcasses and eggs;
* SIR disease;
* a fully booked energy and nutrient cycle;
* 14 scenarios;
* a separate NetLogo Wolf Sheep Predation rule set.

One tick is 6 minutes, and the default year is 120 days.

**Equations** (examples)

| Process | Rule |
|---|---|
| Metabolism (MTE / Kleiber) | B(T) = 0.78·c_th·M^{3/4}·exp(−E/k (1/T − 1/293.15 K)) |
| Sprint speed (Hirt) | v = a·M^b·(1 − e^{−h·M^i}) |
| Ageing | Gompertz |
| Growth | West et al. 2001 |
| Kill probability | Logistic in speed ratio, attack/defence, distance and cover |
| Density fields | Holling type II feeding |
| Accounting | Every flux is booked in kJ and g ledgers; stock = start + inputs − outputs |

Contested actions (one attack per prey, carcass sharing) are claims resolved
in total order.

**Determinism**

* Random streams are derived from (seed, subsystem, tick, entity id).
* Parallel phases are pure, and commits happen in id order.
* The same seed gives bit-identical snapshots on the same platform and build.

**Limitations**

* The compressed calendar triples reproduction per day.
* Insects and worms are fields without individual behaviour.
* Colonies are super-agents.
* Disease is SIR only.
* Packs only grow.
* Migration is driven by temperature only.
* Simplified water, cost of transport, fire escape and pollination.
* The trophic cascade is weak in grass-dominated scenarios.
* One nutrient stands for N and P together.
* Small `f32` drift in the soil-nutrient balance.

## `custom_life` (`rule-engine`)

**What it simulates.** A user-defined agent model. Entity types, rules,
relationships, resource fields and environment fields are all user data.
Positions are continuous on bounded, torus, Klein-bottle or cross-surface
worlds.

**Rules.** Rules are written as

```text
rule NAME for TYPE: [EVENT] [when CONDITION] then ACTION, … [MODIFIERS]
```

They can have priority, probability, cooldown and cost. Sandboxed Rhai
scripts are also allowed: they have operation, call-depth, size and memory
limits, and no file, network or process access.

**One tick**

1. Freeze the frame.
2. Decide per entity, in id order: rules by priority, then scripts, then
   default movement.
3. Resolve interactions with total-order matching. Each entity takes part in
   at most one interaction per tick.
4. Apply the effects in a fixed order.
5. Update fields: decay, regrowth, FTCS diffusion.
6. Run events.

Energy is conserved through reproduction. The carrying capacity that emerges
is K = regen·cells/metabolism.

**Determinism**

* Random streams are keyed by (seed, tick, entity, rule).
* The only non-deterministic path is host cancellation of a script. It
  disables the script and never changes state silently.

**Limitations**

* First-order kinematics, no volume exclusion.
* Contact is tested on the frozen frame.
* One interaction per entity per tick.
* Rhai costs about 1 µs per call.
* Sensing radii must be literals, and there is no line of sight.
* No infinite worlds.

## `evolution` (`evolution`)

**What it simulates.** Prey, and optionally predators, with 13 heritable
genes, in a world with logistic food, an analytic temperature field and
barriers. There is no fitness function: selection emerges from survival and
reproduction.

**Model**

* Food grows by the exact logistic one-tick solution.
* Cost is Kleiber basal cost × a thermal mismatch penalty, plus movement and
  perception costs.
* Mating requires mutual acceptance and uses greedy closest-pair matching.
* Genomes undergo crossover and reflected Gaussian mutation.
* Senescence is Gompertz.
* Measurements: F_ST, heterozygosity, divergence and a two-cluster speciation
  criterion.

**Determinism**

* Per-organism streams derived from the tick key and the id.
* Total-order resolution; BTreeMap only.
* Same platform and build.

**Limitations**

* Abstract units.
* No growth: offspring are born at adult size.
* Hermaphrodites without diploidy.
* Food perception by 8 rays.
* Simple predation.
* Walls instead of a torus.
* At most 100 000 organisms.
* Weak selection on speed unless predators are matched to the prey.
* An Allee effect in small populations.

## `civilization` (`civilization`)

**What it simulates.** People with needs, skills, inventories, preferences
and relationships on a regenerating tile world. One tick is one hour; one
demographic year is 240 ticks.

**Model**

* A utility AI chooses actions.
* People move along A* routes.
* Resources grow back as in Sugarscape.
* Trails follow Helbing's active-walker model and become roads.
* Welfare is Cobb–Douglas.
* Barter follows Sugarscape rule T: the price is the geometric mean of the
  MRSs, and a trade happens only if both welfares rise.
* Groups are union–find components of mutual affinity.
* Fights are Tullock contests.
* Territory follows the XTENT model.

**Determinism**

* Agents act in id order from one `SimRng` stream.
* No hash maps.
* Same platform and build.

**Limitations**

* Compressed demography.
* Agents have near-perfect information.
* Utility weights are not calibrated.
* There is no value of leisure.
* Bilateral prices only: no money.
* Euclidean territory.
* No pregnancy or disease.
* Single-contest fights.
* No seasons or weather.

---

## Foundation crates

| Crate | Provides | Doc |
|---|---|---|
| `patterns` | Life-like and Generations rule notation, Golly bounded-grid suffixes, the `Pattern` model, readers and writers for RLE, plaintext and Life 1.05 / 1.06, format detection, the catalog loader, and 18 embedded classic patterns. Limits: outer-totalistic Moore rules only; at most 2^25 live cells per file; no macrocell (that is in `hashlife`), MCell or compressed files. | [models/patterns.md](models/patterns.md) |
| `organism-engine` | Agent store with stable ids, semi-implicit Euler kinematics, Reynolds steering and boids, perception, an IAUS-style utility AI, a total-order intent resolver (priority, distance, id), genomes, lineage and drives. | [models/organism-engine.md](models/organism-engine.md) |
| `analytics` | Time series with M4 / LTTB downsampling, Welford statistics, histograms, diversity and Gini indices, Kaplan–Meier, period estimation, run records, and CSV/JSON export. | [models/analytics.md](models/analytics.md) |
| `persistence` | Versioned envelopes (RON, JSON, TOML), the `.elsnap` binary container with CRC-32, migrations, atomic writes and user data folders. | [models/analytics.md](models/analytics.md) |
