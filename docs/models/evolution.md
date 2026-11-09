# evolution — the Evolution Laboratory

`crates/evolution` (model id `evolution`, mode "Evolution Laboratory") is an
individual-based model of evolution by natural selection. Prey, and
optionally predators, carry heritable genomes and live in a self-contained 2D
world: food that regrows logistically on land, an analytic temperature field
with a spatial gradient, a directional shift and optional seasons, and
switchable barriers (water, mountains, land bridges) that cut the land into
islands. There is **no fitness function anywhere in the model**. Gene
frequencies change only because some organisms survive and breed more than
others, through drift in small populations, and through mutation. The crate
measures what happens: gene statistics, allele-like class frequencies,
diversity, divergence between islands, F_ST, a pruned genealogy, and an
explicitly defined speciation criterion.

The crate depends on `simulation-core` (contracts, `SimRng`, `Grid2`,
`SpatialGrid`) and `organism-engine` (agent store, kinematics, steering,
perception, intent resolution, genomes, genealogy), plus `serde`, `rayon`,
`thiserror` and `ron`. It has no renderer and no `physics-world` dependency.

## Contents

1. [Module map and public API](#module-map-and-public-api)
2. [World](#world)
3. [Genome](#genome)
4. [Physiology and trade-offs](#physiology-and-trade-offs)
5. [Behaviour](#behaviour)
6. [One tick](#one-tick)
7. [Reproduction and inheritance](#reproduction-and-inheritance)
8. [Mortality](#mortality)
9. [Measurements](#measurements)
10. [The speciation criterion](#the-speciation-criterion)
11. [Parameters, commands, metrics, view](#parameters-commands-metrics-view)
12. [Scenarios](#scenarios)
13. [Approximations and limitations](#approximations-and-limitations)
14. [Determinism](#determinism)
15. [How it was verified](#how-it-was-verified)
16. [Performance](#performance)
17. [References](#references)

## Module map and public API

| Module | Contents |
|---|---|
| `genes` | `GENES` table (13 heritable genes + 2 acquired traits), gene indices, `schema(mutation_size)`, `Phenotype::express`, `TraitValue`, `founder_genome` |
| `config` | `EvolutionConfig` and its parts (`WorldConfig`, `FoodConfig`, `ClimateConfig`, `Barrier`, `Shape`, `Terrain`, `RegionMode`, `PopulationConfig`, `PredatorConfig`, `PhysiologyConfig`, `ReproductionConfig`, `MutationConfig`, `StatisticsConfig`, `SpeciationConfig`), `validate()` |
| `params` | `specs()`, `get`, `set`, `field_path`: the 41 parameters and their mapping onto config fields |
| `environment` | `Environment`: terrain, barriers, regions, food, temperature |
| `organism` | `Organism`, `Species`, `Activity`; cost functions (`basal_cost`, `movement_cost`, `speed_upkeep`, `perception_cost`, `intake_capacity`, `storage`, `senescence_hazard`, `kill_probability`, `capture_radius`) |
| `behaviour` | the parallel decision phase (crate-internal), `INTERFERENCE_RADIUS` |
| `reproduction` | `is_ready`, `clone_heritable`, `offspring_genome`, `recombine` (crossover over heritable genes only), `clutch`, `provision` |
| `step` | `EvolutionWorld::step` |
| `stats` | `PopulationStats`, `SpeciesStatistics`, `GeneStatistics`, `RegionStatistics`, `RegionDivergence`, `LineageCount` |
| `speciation` | `SpeciationTracker`, `SpeciationStatus`, `ClusterSplit`, `SpeciationEvent` |
| `commands` | `EvolutionCommand` and `EvolutionWorld::apply` |
| `view` | `EvolutionViewRequest`, `EvolutionView`, `OrganismView`, `FieldLayer`, `GenealogyExport`, `EvolutionWorld::genealogy_export` |
| `world` | `EvolutionWorld` (state, accessors, snapshot form), `TickReport`, `DeathCause`, `MAX_ORGANISMS` |
| `sim` | `EvolutionSim` (`Simulation` impl), `MODEL_ID`, `SCHEMA_VERSION`, `metric_specs()` |
| `scenarios` | `Scenario`, `builtin()`, `find_builtin(id)`, `load_dir(path)`, `load_file(path)`, `BUILTIN_FILES` |

Typical use:

```rust
let scenario = evolution::scenarios::find_builtin("island_evolution")?;
let mut sim = scenario.build()?;                    // EvolutionSim
sim.advance(1000);                                  // simulation_core::Simulation
let stats = sim.world().stats();                    // PopulationStats
let tree = sim.world().genealogy_export();          // ancestry edges + lineage counts
sim.apply(EvolutionCommand::ToggleBarrier { index: 1 })?;
```

## World

* **Grid.** `W × H` cells, one world unit each, `y` downward. The edges are
  walls (positions are clamped). Organisms move continuously.
* **Terrain and barriers.** The world starts as land. `Barrier`s (name,
  terrain, shapes = union of rectangles and discs, `outside` flag, initial
  `active` flag) are painted in list order; a cell belongs to a shape when its
  centre does. Water and mountains are impassable; a `land` barrier opens a
  corridor through an earlier one (a land bridge). `outside: true` paints
  everything outside the shapes (an ocean around islands). Barriers are
  switched on and off by command; terrain, capacities and regions are then
  repainted and food on newly impassable cells is lost. An organism standing
  on a cell that becomes impassable can still walk off it (only destinations
  are tested).
* **Regions.** `islands`: connected components of passable cells
  (4-neighbourhood), numbered in row-major order of their first cell.
  `sectors(cols, rows)`: a grid of rectangles (passable cells only), e.g.
  bands along a temperature gradient.
* **Food.** Each land cell has capacity `K = max(background, richest patch) ×
  capacity_scale`. Per tick, with seed bank `s` and rate `r`:

  ```text
  F₀ = max(F, s·K)
  F₁ = K F₀ / (F₀ + (K − F₀) e^{−r})        exact one-tick solution of dF/dt = r F (1 − F/K)
  F ← F + (F₁ − F₀)
  ```

  The closed form is stable for any `F`, including food above `K` added by a
  command (it decays back towards `K`). Cells with `K = 0` decay as
  `F e^{−r}`. The update is a pure per-cell function run with rayon.
* **Temperature.**

  ```text
  T(x, y, t) = T₀ + G_x (x/W − ½) + G_y (y/H − ½) + Δ(t) + A sin(2π t / P)
  Δ(t+1) = Δ(t) + shift_rate / 1000          (plus jumps from ChangeTemperature)
  ```

  Because the shift accumulates in `Δ`, changing the rate never makes the
  field jump.

## Genome

Genes are `organism_engine::Genome` values normalised to `[0, 1]` and
expressed linearly into the ranges below (`GENES` in `genes.rs`).

| Gene | Range | Default | Heritable | Role |
|---|---|---|---|---|
| `speed` | 0.2–1.6 units/tick | 0.7 | yes | top speed (flee, hunt); upkeep and movement costs ∝ M·v² |
| `perception` | 2–14 units | 6 | yes | detection radius for food, mates, prey, predators; cost ∝ r |
| `size` | 0.5–3 | 1 | yes | mass M: storage ∝ M, basal and intake ∝ M^0.75, movement ∝ M, size refuge |
| `efficiency` | 0.5–1.5 | 1 | yes | divides basal cost **and** maximum intake (rate–yield trade-off) |
| `temp_optimum` | −5–40 °C | 20 | yes | thermal optimum |
| `temp_tolerance` | 1–12 °C | 5 | yes | niche breadth; generalist cost on basal metabolism |
| `clutch` | 1–8 | 2 | yes | offspring per breeding event (rounded) |
| `effort` | 0.2–0.8 | 0.5 | yes | fraction of the reserve spent per breeding event |
| `maturity` | 30–300 ticks | 100 | yes | first breeding age; senescence time scale |
| `boldness` | 0–1 | 0.5 | yes | prey: flee distance; predators: hunger threshold |
| `choosiness` | 0–1 | 0.2 | yes | minimum condition demanded from a mate |
| `marker_a`, `marker_b` | 0–1 | 0.5 | yes | neutral markers (no phenotypic effect) |
| `acclimation` | −4–4 °C | 0 | **no** | acquired thermal shift |
| `experience` | 0–1 | 0 | **no** | acquired predator wariness |

The two acquired traits are declared non-heritable in the
`GenomeSchema`, so the organism engine never mutates them, never passes
them on (offspring start at the default), excludes them from
`genetic_distance`, and they change only during life. All other individual
state — energy, health, age, refractory and handling counters, position,
offspring count, activity — lives outside the genome and is never copied to
offspring.

Founders: every heritable gene starts at its configured mean (or default)
plus `N(0, spread)` in normalised units, reflected into `[0, 1]`; founder
ages are uniform in `[0, maturity)`, energy is half of storage, and each
founder starts its own lineage.

## Physiology and trade-offs

Per tick, for phenotype size `M`, efficiency `η`, tolerance `σ`, top speed
`v_max`, perception `r`, speed actually used `v` and temperature `T`:

```text
mismatch     m = (T − (T_opt + acclimation)) / σ
basal        b₀ · M^0.75 / η · (1 + c_σ (σ − 1)) · (1 + c_T · min(m², 9))
movement     c_m · M · v²
speed upkeep c_u · M · v_max²
perception   c_p · r
storage      E_max = s · M                     (energy above E_max is not stored)
intake       ≤ i₀ · M^0.75 / η food per tick   (× food_energy)
health       −= d (|m| − 1) if |m| > 1, else += recovery (max 1); death at 0
acclimation  += a (T − T_opt − acclimation), limited to ±4 °C by the gene range
experience   += λ (1 − e) on a tick spent escaping a predator, else −= φ e
```

Defaults: `b₀ = 0.2`, `c_m = 0.15`, `c_u = 0.05`, `c_p = 0.006`,
`c_σ = 0.03`, `c_T = 0.5`, `d = 0.05`, recovery `0.01`, `s = 60`, `i₀ = 1.5`,
`a = 0.01`, `λ = 0.3`, `φ = 0.005`, food energy `1`.

The trade-offs that keep selection from being trivially monotonic:

* **Size**: basal metabolism and intake scale as `M^0.75` (Kleiber 1932;
  Brown et al. 2004), storage as `M` (fasting endurance ∝ `M^0.25`, Lindstedt
  & Boyce 1985), movement and speed upkeep as `M`; large prey have a refuge
  (lower kill probability), small organisms are cheap.
* **Speed**: costs every tick (upkeep of locomotor capacity) and when used
  (`∝ M v²`); it helps only to flee or to hunt, because searching, grazing and
  courting happen at `min(v_max, cruise_speed)` (default 0.4). Acceleration
  is `agility × v_max`, so fast genotypes are also nimbler.
* **Efficiency**: lowers basal cost but also the maximum intake rate — the
  rate–yield trade-off of microbial and animal energetics (Pfeiffer, Schuster
  & Bonhoeffer 2001).
* **Thermal tolerance**: broad niches cost basal energy ("a jack of all
  temperatures is a master of none", Huey & Hertz 1984).
* **Perception**: earlier detection of food, mates and predators against a
  linear energy cost.
* **Life history**: `clutch × per-offspring energy = effort × reserve ×
  birth_efficiency` makes the number/size trade-off explicit (Smith & Fretwell
  1974); late maturity means a longer life (senescence time scale ∝ maturity,
  in the spirit of Charnov's life-history invariants) but a later start.
* **Behaviour**: bold prey keep feeding longer but flee later; choosy mates
  obtain well-provisioned partners but find fewer.

## Behaviour

Decisions are pure functions of the read-only world and of a per-organism
random stream, computed in parallel. Sight requires a passable straight line
(sampled every cell) whenever the world has obstacles, and sampled feelers
(`organism_engine::steering::avoid_obstacles`) then steer away from walls with
priority over everything else (`combine_prioritized`).

**Prey**, in priority order:

1. *Flee*: the nearest predator **in sight** within
   `min(r, r (1 − boldness + ½ experience))` → `evade` at full speed
   (prediction horizon 2 ticks). Fleeing prey do not eat. Predators behind a
   wall are not seen, exactly as prey behind a wall are not seen by
   predators.
2. *Court* (sexual mode, ready): nearest ready, mutually acceptable mate in
   sight → `seek` it, brake when within `mate_radius`.
3. *Graze*: the current cell holds at least half a full bite → brake and eat.
4. *Forage*: march 8 rays (random rotation per tick) cell by cell up to `r`,
   stopping at the first impassable cell, and `seek` the point maximising
   `food / (1 + 0.15 d)` if it beats the current cell.
5. *Wander* (Reynolds wander) at cruising speed.

**Predators**: hunt — `pursue` the nearest prey in sight at full speed — while
energy is below `(0.3 + 0.6 boldness) E_max` and no kill is being handled. A
hungry predator with `n` rivals within 3 units loses the tick to a contest
with probability `1 − e^{−c n}` (interference, Beddington 1975; DeAngelis et
al. 1975; `c` = `predator_interference`, default 0). Hungry predators without
prey in sight wander at cruising speed; satiated ones court (sexual, ready)
or rest at 60 % of cruising speed.

## One tick

```text
1 environment   Δ += shift/1000; food regrows (parallel per cell)
2 index         phenotypes and readiness (parallel); prey and predator SpatialGrids (slot = store index)
3 decide        parallel; organism i uses SimRng::derive(tick_key, id_i), tick_key drawn from the world RNG
4 live          parallel, each organism writes only itself: integrate kinematics (passability test),
                costs, thermal health, acquired traits, age, refractory and handling counters
5 predation     hunting predators within 0.5 + 0.25 M of their target strike; one predator per prey
                (organism_engine::resolve_exclusive: closest, then lowest id); kill with probability
                p₀ · min(1, 2 M_pred / (M_pred + M_prey)); gain α (ε_body M_prey + max(E_prey, 0)), capped at
                E_max; the predator then handles its kill for handling_ticks ticks
6 feeding       non-fleeing prey claim min(intake, room) on their cell; resolve_shared(Proportional)
7 reproduction  see below (sexual pairs: closest pair first)
8 mortality     starvation, thermal, background, senescence (id order, world RNG)
9 commit        offspring appended, the dead removed; genealogy births/deaths recorded
10 measure      statistics every tick; speciation criterion every check_interval ticks
```

## Reproduction and inheritance

* **Ready**: `age ≥ maturity`, refractory period over, `health ≥ ½`,
  `E ≥ threshold · E_max` (default 0.6). Predators under the constant regime
  never breed.
* **Asexual**: every ready organism breeds.
* **Sexual** (simultaneous hermaphrodites): a courting organism in contact
  (`≤ mate_radius`, default 1.5) with a ready partner forms a pair if both
  accept each other — the partner's condition `E/E_max ≥ θ + choosiness (1 −
  θ)` and their genetic distance `≤ mate_distance_limit` (1 = no
  restriction; below 1 = assortative mating). Pairs are formed one-to-one:
  `organism_engine::resolve_matching` gives every suitor one partner and
  every partner one suitor, and because a pair is unordered (A may court B
  while B courts C) the matched claims are then taken closest pair first
  (then lowest ids), skipping anyone already paired (`step::mating_pairs`).
* **Investment**: each parent spends `effort × E`; the clutch — the parent's
  clutch gene, or the rounded mean of both — shares `birth_efficiency × Σ
  spent` (default 0.85) equally. A newborn stores at most its own capacity
  `s · M` (`reproduction::provision`); provisioning beyond it is lost, like
  any energy above `E_max` (this matters for two-parent clutches of one with
  high effort, which would otherwise create newborns above 100 % condition).
  Offspring appear within 0.8 units of the first parent and enter a fresh
  state (age 0, health 1, no refractory period); the parents enter their
  refractory period (default 20 ticks).
* **Genome**: asexual offspring copy the heritable genes; sexual offspring
  recombine both parents with `Genome::crossover` (uniform or one-point)
  applied to the **heritable genes alone** (`reproduction::recombine`). The
  engine draws a one-point cut uniformly over all schema slots; with the two
  acquired slots at the end of the genome, cuts among them would copy the
  first parent unchanged (about one sexual offspring in seven), so the
  genome is projected onto its 13 heritable genes first and every cut falls
  in one of the 12 gaps between them (tested: all 12 equally likely, never a
  non-recombinant child). Uniform crossover draws the same numbers either
  way. Then
  `Genome::mutate_with`: every heritable gene mutates with probability
  `rate` by `N(0, size)` (normalised units), reflected (default) or clamped at
  the bounds. Acquired traits start at their defaults. Predators with
  `predators_evolve = false` clone their first parent exactly (no
  recombination, no mutation, no random draws).
* **Genealogy**: every birth and death is recorded in an
  `organism_engine::Genealogy` with pruning (dead nodes without living
  descendants are removed) and a node cap (`genealogy_max_nodes`, the oldest
  dead ancestors are truncated first). The `founder` of an organism is the
  founder at the root of its line of first parents (a "matriline").

## Mortality

In id order, for organisms not killed by a predator this tick: starvation
(`E ≤ 0`), else thermal (`health ≤ 0`), else background (probability
`background_mortality`), else senescence with Gompertz hazard

```text
h(age) = h₀ · exp(ln 1000 · age / (L · maturity))      h₀ = 1e-4, L = longevity_ratio = 4
```

so the hazard has grown 1000-fold at `L` maturity ages. The **constant
predation** regime (`predators_constant`) is an explicit experimental
treatment: predators neither breed nor die (energy and health are floored at
0, so hunger still drives hunting) and their number changes only by command.

## Measurements

Recomputed after every tick and command (`stats.rs`). Sampling uses a
stream derived from `(seed, tick)`, independent of the simulation's
generator, so measuring never changes a run.

* **Per gene and species**: mean, population standard deviation, range,
  histogram over the gene's range (`histogram_bins`), allele-like class
  frequencies (`allele_classes` equal classes of the normalised value) and
  their expected heterozygosity `H = 1 − Σ pᵢ²`.
* **Genetic diversity**: mean pairwise genetic distance among prey — the
  organism engine's RMS distance over normalised heritable genes, exact when
  there are at most `sample_pairs` pairs, otherwise estimated from that many
  random pairs (an analogue of nucleotide diversity π).
* **Heterozygosity**: mean class heterozygosity over heritable prey genes.
* **Regions**: prey and predators per region, mean genome per region.
  For the six most populous prey regions: mean between-region distance
  `d_AB` and net divergence `d_AB − (d_A + d_B)/2` (Nei's D_A analogue);
  metric `region_divergence` = the largest net divergence.
* **F_ST** = `(H̄_T − H̄_S) / H̄_T` over heritable genes, with class
  frequencies, regions with ≥ 2 prey weighted by size (Nei's G_ST form).
* **Generations**: mean and maximum generation number of living prey.
* **Lineages**: living organisms per founder.
* **Genealogy export** (`genealogy_export()` or the view with
  `genealogy: true`): recorded nodes (living organisms and ancestors with
  living descendants), parent → child edges, lineage counts and counters.

## The speciation criterion

Speciation is never inferred from anything but this rule (`speciation.rs`),
and the status reads "not applicable" (with the reason) when its
preconditions do not hold:

1. Sexual reproduction with assortative mating (`mate_distance_limit` θ < 1)
   and the criterion enabled.
2. At least 20 prey, split by deterministic 2-means in normalised heritable
   gene space (seeds: the individual farthest from the centroid, then the one
   farthest from it, ties to the lower index; ≤ 25 Lloyd iterations); each
   cluster holds ≥ `max(5, min_cluster_fraction · N)` (default 10 %).
3. The mean genetic distance between members of different clusters exceeds
   θ, i.e. the average cross-cluster pair could not mate.
4. Conditions 2–3 hold at every evaluation (every `check_interval` = 20 ticks)
   while the mean prey generation advances by `speciation_generations`
   (default 5). Then a `SpeciationEvent` is declared and logged as a major
   event; the metric `speciation` becomes 1. When the separation is lost the
   status returns to monitoring; declared events stay in the history.

## Parameters, commands, metrics, view

### Parameters

| Key | Group | Range | Default | Applies |
|---|---|---|---|---|
| `width`, `height` | World | 16–512 | 96, 64 | requires reset |
| `initial_prey` / `initial_predators` | World | 0–5000 / 0–2000 | 200 / 0 | requires reset |
| `genealogy_max_nodes` | World | 100–1 000 000 | 20 000 | requires reset |
| `max_ticks` | World | 0–1e9 (0 = until extinction) | 0 | immediately |
| `food_regrowth_rate` | Food | 0–0.5 | 0.05 | next tick |
| `food_capacity_scale` | Food | 0.05–5 | 1 | next tick |
| `food_energy` | Food | 0.1–10 | 1 | next tick |
| `base_temperature` | Climate | −20–50 °C | 20 | next tick |
| `temperature_gradient_x` / `_y` | Climate | −60–60 °C | 0 | next tick |
| `temperature_shift_rate` | Climate | −20–20 °C/1000 ticks | 0 | immediately |
| `seasonal_amplitude` / `seasonal_period` | Climate | 0–30 °C / 10–100 000 | 0 / 1000 | next tick |
| `reproduction_mode` | Reproduction | asexual / sexual | asexual | next tick |
| `crossover` | Reproduction | uniform / one_point | uniform | next tick |
| `breeding_threshold` | Reproduction | 0.1–1 | 0.6 | next tick |
| `refractory_ticks` | Reproduction | 0–1000 | 20 | next tick |
| `mate_distance_limit` | Reproduction | 0–1 | 1 | next tick |
| `mutation_rate` / `mutation_size` | Mutation | 0–1 / 0–0.5 | 0.1 / 0.05 | next tick |
| `mutation_bounds` | Mutation | reflect / clamp | reflect | next tick |
| `predators_evolve` / `predators_constant` | Predators | bool | false / false | next tick |
| `capture_probability` / `predator_assimilation` | Predators | 0–1 | 0.5 / 0.5 | next tick |
| `handling_ticks` | Predators | 0–1000 | 20 | next tick |
| `predator_interference` | Predators | 0–5 | 0 | next tick |
| `basal_cost`, `movement_cost` | Physiology | 0–2 | 0.2, 0.15 | next tick |
| `speed_upkeep` | Physiology | 0–1 | 0.05 | next tick |
| `perception_cost` | Physiology | 0–0.2 | 0.006 | next tick |
| `tolerance_cost`, `thermal_stress_cost` | Physiology | 0–0.5, 0–5 | 0.03, 0.5 | next tick |
| `background_mortality`, `longevity_ratio` | Physiology | 0–0.05, 1–20 | 0.0005, 4 | next tick |
| `histogram_bins`, `allele_classes` | Statistics | 2–64, 2–16 | 12, 4 | immediately |
| `speciation_enabled`, `speciation_generations` | Statistics | bool, 1–1000 | true, 5 | next tick |

Parameters that require a reset are stored and used by the next `reset()`;
the running world keeps its geometry until then (so snapshots stay
consistent). Config fields without a parameter (patches, barriers, founder
traits, storage, intake rate, cruise speed, agility, seed bank, body energy,
statistics sampling, speciation interval...) are set in scenario files and
validated with field-path error messages. Changing `speciation_enabled`,
`reproduction_mode` or `mate_distance_limit` updates the speciation status
at once (not applicable / monitoring); the population is evaluated at the
next check.

The run is **finished** (`is_finished`, the runner pauses) at `max_ticks`, or
when nothing can change any more: no organism is left, or the prey are
extinct and only constant-regime predators (which never breed or die)
remain.

### Commands (`EvolutionCommand`)

| Command | Effect |
|---|---|
| `AddFood { area, amount }` | adds food to land cells in the area (decays back towards K) |
| `ChangeTemperature { delta }` | shifts the whole temperature field at once (±50 °C) |
| `SetTemperatureShift { per_1000_ticks }` | sets the climate trend |
| `SetBarrier { index, active }`, `ToggleBarrier { index }` | raise or remove a scenario barrier |
| `IntroducePredators { count, area, traits }` | new predator founders |
| `Cull { fraction, species, area }` | kills `round(fraction · n)` uniformly chosen matching organisms |
| `Spawn { species, count, area, traits, spread }` | organisms with a custom genome (new lineages) |

Every command is validated first (unknown genes, acquired traits, values out
of range, areas without land, the 100 000-organism limit) and changes
nothing on error. Command randomness uses its own stream keyed by a command
counter stored in snapshots, so commands never perturb the simulation's
generator. Births and deaths caused by commands appear in the next tick's
counters.

### Metrics

`prey`, `predators`, `births`, `deaths`, `deaths_{starvation, predation,
thermal, senescence, background, culled}`, `matings`, `mutations`,
`mean_generation`, `max_generation`, `mean_<gene>` and `sd_<gene>` for the
13 heritable prey genes, `mean_acclimation`, `mean_experience`,
`predator_mean_{speed, perception, size}`, `genetic_diversity`,
`heterozygosity`, `fst`, `region_divergence`, `regions_populated`,
`lineages`, `genealogy_nodes`, `food_total`, `mean_temperature`,
`mean_condition`, `cluster_distance`, `speciation` (57 in all; the order is
fixed and tested against the descriptor).

### View

`EvolutionViewRequest { field: none | food | temperature | region,
color_gene: Option<gene index>, genealogy: bool }` →
`EvolutionView { tick, width, height, terrain, field (values, min, max),
organisms (id, species, position, heading, size, condition, health,
normalised gene value, generation, founder, activity), stats, speciation
status, latest cluster split, genealogy export, barrier states, last tick
report, temperature offset, up to 20 recent events (speciation, extinctions,
interventions) }`.

## Scenarios

Six presets live in `scenarios/evolution/*.ron` and are embedded with
`include_str!` (`scenarios::builtin()`); `scenarios::load_dir(path)` reads a
folder, rejecting duplicate ids, and RON errors name the file, line and
column. A file is a header `(id, name, description, seed, config: (...))`
where the config lists only values that differ from the defaults. The
observations below come from calibration runs of the shipped files (release
build, the seed in each file unless stated); every row is asserted
qualitatively by `tests/presets.rs` (ignored by default, see below).

| Id | Set-up | Observed |
|---|---|---|
| `speed_selection` | 64 × 48, 300 asexual prey (speed 0.6 ± spread), 40 identical non-evolving predators of top speed 0.85 (the upper edge of the founders' range) under the **constant** regime | mean prey speed over ticks 1700–2000 with vs without predators, seeds 11/13/17/19/23/29: 0.640/0.439, 0.628/0.536, 0.744/0.661, 0.844/0.614, 0.790/0.658, 0.704/0.601 (founders ≈ 0.60–0.61); perception falls to ≈ 2.5–2.7 in both |
| `thermal_adaptation` | gradient 10 °C, warming 6 °C/1000 ticks, 3 sector regions | over 4000 ticks the mean temperature goes 18 → 42 °C and the mean optimum 18.4 → 36.0 °C (lag ≈ 6 °C); acclimation absorbs 2.6 °C; tolerance widens 4.0 → 5.5 °C; 988 thermal deaths. At 15 °C/1000 ticks the prey are extinct at tick 2454 |
| `arms_race` | 80 × 60, co-evolving dynamic predators with handling time 25 and interference 0.6 | both persist 4000 ticks in seeds 37–39 (813–960 prey, 79–103 predators at the end); prey speed 0.59–0.62 → 0.95–1.11, predator speed 0.77–0.89 → 0.90–1.11 (seed 38 barely: 0.886 → 0.901); prey shrink to 0.54–0.57 |
| `food_scarcity` | capacity scale 0.3 | vs scale 2 (seeds 41–43, mean over ticks 2700–3000): perception 3.02 vs 2.47, efficiency 1.02 vs 0.98, tolerance 3.7 vs 4.1 °C, maturity 114 vs 101 — scarcity favours search and efficiency; the effects are modest and vary between seeds |
| `island_evolution` | 3 islands centred at 10 / 18 / 26 °C (base 18 °C, gradient 24 °C over the width), sexual, `mate_distance_limit` 0.18, land bridge (barrier 1, off) | F_ST 0.29–0.36 at the checkpoints every 1500 ticks (mean 0.33 over ticks 1500–9000, sampled every 100); region divergence grows 0.05 → 0.11 by tick 9000; the speciation criterion is met at tick 6200 (mean generation 47.4) |
| `r_vs_k` | r founders (clutch 6, maturity 50, size 0.8, marker 0.1) vs K founders (clutch 1, maturity 150, size 1.6, marker 0.9), food in 4 separated patches | after 5000 ticks K wins (marker 0.90, clutch 1.3); with `background_mortality` 0.008 r wins (marker 0.12, clutch 6.0); with the same total food spread uniformly (capacity 2.4) r also wins (marker 0.11, clutch 6.3) |

## Approximations and limitations

* **Time and units are abstract**: one tick, one world unit, energy and
  food units are model units, not calibrated to a species. Allometric
  exponents follow the literature; coefficients are designer defaults.
* **No growth**: offspring are born at adult size, so parental investment acts
  only through the newborn's energy reserve (enough to make the r/K trade-off
  real — see `r_vs_k` — but not a full dynamic-energy-budget model).
* **Simultaneous hermaphrodites** in sexual mode; the lineage (`founder`)
  follows the first parent. No sex ratio, no diploidy or dominance: genes are
  haploid real values, and "alleles" are equal-width value classes.
* **Food perception** marches 8 rays; a patch narrower than the gap between
  rays at its distance can be missed. Sight is sampled every cell.
* **Predation** is contact-based with a fixed kill probability and size
  refuge; there is no group defence. Predators eat only prey. Interference
  is a probabilistic lost tick (Beddington–DeAngelis in spirit, not in
  closed form).
* **Constant predation regime** is an experimental treatment (fixed
  predator density, as in a lab selection experiment), not a natural
  population; use the dynamic regime for ecological dynamics.
* **Small worlds go extinct**: the model is individual-based and stochastic;
  boom–bust cycles and demographic extinctions happen, as in real small
  populations. The presets were calibrated to persist, not guaranteed to.
* **Genetic distance** is the RMS over all 13 normalised heritable genes,
  neutral markers included; F_ST and heterozygosity use value classes.
  Regional divergence covers the six most populous regions; pairwise
  quantities above `sample_pairs` are sampled estimates.
* **Speciation** is only the two-cluster criterion above; more than two
  incipient species are not resolved, and the generation clock is the mean
  generation of the living prey.
* **Limits**: at most 100 000 living organisms; births beyond are suppressed
  and counted (`births_blocked`), and the parents still pay the full clutch.
* The world edges are walls (no torus).
* **Gene ranges bound adaptation**: the thermal optimum cannot evolve above
  40 °C (plus at most 4 °C of acclimation and a 12 °C tolerance). Under the
  `thermal_adaptation` warming (6 °C per 1000 ticks) the mean temperature
  passes 40 °C after about 4000 ticks, after which the population can only
  decline.
* **Selection on speed by predation is weak** unless predators are about as
  fast as the prey: a predator faster than every prey catches whichever prey
  it targets, and speed then only prolongs the chase. The `speed_selection`
  preset therefore uses predators of top speed 0.85, at the upper edge of the
  founders' range. Even so the selection differential is small (about
  −0.05 speed units per kill, `tests/selection.rs`) and replicate runs vary
  a lot, so conclusions need several seeds.
* **Small sexual populations go extinct** by chance and mate scarcity (an
  Allee effect): about one in five isolated islands of ≈ 20–30 prey was lost
  within 2500 ticks in the drift set-up.

## Determinism

* All randomness comes from `SimRng`: the world stream (in snapshots), a
  per-tick key from it that seeds per-organism decision streams
  (`derive(tick_key, id)`), a founder stream, a command stream keyed by a
  counter in the snapshot, and measurement streams derived from
  `(seed, tick)` that never feed back.
* Parallel phases (food regrowth, phenotypes, decisions, movement and
  physiology) are pure per-element functions; every contested decision
  (attacks, feeding, mating) is resolved by the organism engine's intent
  resolution in a total order; serial phases run in id order.
* Ordered containers only (`BTreeMap`); no `HashMap` iteration drives logic.
* Snapshots (`schema_version` 1) hold the configuration, world RNG,
  environment state (food, barrier states, temperature offset), the organism
  store with its id counter, the genealogy, the speciation tracker, counters
  and the event log; derived data (terrain, capacities, regions, schema,
  statistics) is rebuilt and validated on restore (dimensions, genome
  lengths, finite values, store/genealogy agreement).
* Float results are reproducible on the same platform and build; the
  selection test produced bit-identical numbers in the dev and release
  profiles on the development machine.

## How it was verified

```sh
CARGO_TARGET_DIR=target/agents/evolution cargo test -p evolution
CARGO_TARGET_DIR=target/agents/evolution cargo test --release -p evolution --test presets -- --ignored
CARGO_TARGET_DIR=target/agents/evolution cargo clippy -p evolution --all-targets -- -D warnings
CARGO_TARGET_DIR=target/agents/evolution cargo bench -p evolution
```

57 tests (50 unit, 7 integration) pass in the dev profile in ≈ 30 s on the
loaded development machine (the selection and drift tests take most of it);
the 6 preset calibration checks run in ≈ 25 s in release.

* **Rules** (unit tests): Kleiber scaling of basal cost, `M v²` movement
  cost, 4× speed upkeep for 2× speed, thermal mismatch with acclimation,
  Gompertz hazard (`1e-4` at birth, `0.1` at `L` maturity ages), size refuge;
  exact logistic food solution against the closed form, decay above `K`,
  seed bank; barrier painting, islands, land bridges, sectors; temperature
  gradient, shift and season, and the cached mean land temperature against a
  cell-by-cell average on an irregular coastline; flee/hunt/rest/graze/forage
  decisions, walls blocking the sight of food, prey **and predators**,
  courtship requiring mutual acceptance and assortative limits; one-to-one,
  closest-first mating pairs (chains, mutual courtship, rival suitors);
  readiness rules, clutch energetics, newborn provisioning capped at storage;
  one-point crossover always recombining with all 12 cuts equally likely, and
  uniform crossover drawing exactly what the engine draws; 2-means
  recovering planted clusters, the persistence requirement of the speciation
  criterion and its status following its preconditions at once; parameter
  round trips, policies and validation; the finish condition (constant
  predators alone, `max_ticks`); snapshot validation (dead generator state,
  store/genealogy disagreement); scenario parsing errors with file names and
  line numbers; metric keys matching the descriptor.
* **Heritable vs acquired** (`acquired_traits_are_never_inherited`,
  `tests/heredity.rs`): offspring of acclimated, experienced parents always
  start at 0; with mutation rate 0, ≥ 100 checked parent/offspring pairs in a
  running population carry identical heritable genes; newborns have fresh
  individual state.
* **Mutation statistics**: with rate 0.25 and size 0.04, 8000 offspring show a
  mutated fraction of 0.25 ± 0.01 and unbiased steps with sd 0.04 ± 0.002.
* **Deterministic replay** (`tests/replay.rs`): equal seeds and commands give
  bit-identical snapshots (`arms_race`, `island_evolution`, 150 ticks with
  spawn, cull and temperature commands); a snapshot restored into a
  different scenario, also through bytes, continues bit-identically for 120
  ticks; `reset` returns to the initial snapshot.
* **Selection** (`tests/selection.rs`): the `speed_selection` set-up scaled
  down (48 × 40 world, 250 prey, 30 constant-regime predators of top speed
  0.75), 1500 ticks, seeds 1–4, three paired arms with identical founders:
  predators; no predators; and **random deaths** — prey culled at random at
  the per-capita rate the predators killed in the first arm. The third arm
  matters: predation also lowers density and relaxes food competition, which
  by itself weakens selection against costly speed (the earlier version of
  this test compared only with the predator-free control and could not tell
  the two apart). Mean prey speed over ticks 1200–1500:

  | Seed | Founders | Predators | No predators | Random deaths |
  |---|---|---|---|---|
  | 1 | 0.596 | 0.761 | 0.692 | 0.669 |
  | 2 | 0.611 | 0.679 | 0.639 | 0.662 |
  | 3 | 0.618 | 0.649 | 0.540 | 0.465 |
  | 4 | 0.609 | 0.680 | 0.520 | 0.506 |

  Mechanism: on ticks where predation was the only cause of death, killed
  prey were slower than the population by `S = −0.046` speed units
  (n = 514 kills, z = −7.1), while the random culls were unselective
  (`S = −0.004`, n = 3348, z = −1.5). Response: predators > no predators for
  every seed; averaged over the seeds +0.094 vs no predators, +0.117 vs
  random deaths and +0.084 vs the founders (all required > 0.05). Replicate
  runs vary a lot, so the random-deaths comparison is asserted on the
  average, not per seed (seed 2: +0.017). In 8 further exploratory seeds
  (5–12, not part of the test) predators beat no predators in 8 of 8 and
  random deaths in 7 of 8.
* **Drift** (`tests/drift.rs`): four isolated islands (r = 7, ≈ 20–30 prey
  each) versus the same world without the ocean (≈ 550 prey), sexual with
  free recombination, mutation off, seeds 1–4, 2500 ticks (≈ 16 generations).
  Islands that died out (2 of 16) are excluded. On the 14 persisting islands
  the marker kept on average 17 % of its founders' variance (10 islands
  near fixation), the mainland 87 %; the differentiation
  `among / (among + within)` rose from 0.03 to 0.56. Thresholds follow
  Wright–Fisher expectations (`(1 − 1/N_e)^t` ≤ ≈ 0.5 here vs ≈ 0.97 on the
  mainland).
* **Presets** (`tests/presets.rs`, `#[ignore]`, release): each row of the
  scenario table above is asserted qualitatively.

## Performance

Criterion (`cargo bench -p evolution`), release profile, Apple M2 Max
(12 cores), measured while other agents were building (load average 17–22):

| Benchmark | Time (median, 95 % interval) |
|---|---|
| `speed_selection_100_ticks` (463 → 544 organisms incl. 40 predators, 64 × 48) | 48.7 ms [47.3, 50.7] |
| `island_evolution_100_ticks` (149 → 182 organisms, obstacles, mate search) | 36.3 ms [34.4, 40.4] |
| `view_with_genealogy` (463 organisms, 584 genealogy nodes) | 19.4 µs [18.6, 20.6] |

So one tick of the speed-selection preset costs ≈ 0.5 ms (≈ 2000 ticks/s);
the island preset is costlier per organism because of line-of-sight checks,
obstacle feelers and mate search. Statistics (gene histograms, sampled
pairwise distances, regional divergence, F_ST) are recomputed every tick and
are part of these timings. The `mean_temperature` metric is `O(1)`: the field
is affine in position, so its mean over land is its value at the land
centroid, cached when the terrain is painted.

## References

* Beddington, J. R. (1975). Mutual interference between parasites or
  predators and its effect on searching efficiency. *J. Anim. Ecol.* 44:331.
* Brown, J. H., Gillooly, J. F., Allen, A. P., Savage, V. M. & West, G. B.
  (2004). Toward a metabolic theory of ecology. *Ecology* 85:1771.
* Charnov, E. L. (1993). *Life History Invariants.* Oxford University Press.
* DeAngelis, D. L., Goldstein, R. A. & O'Neill, R. V. (1975). A model for
  trophic interaction. *Ecology* 56:881.
* Gompertz, B. (1825). On the nature of the function expressive of the law of
  human mortality. *Phil. Trans. R. Soc.* 115:513.
* Holling, C. S. (1959). Some characteristics of simple types of predation
  and parasitism. *Can. Entomol.* 91:385.
* Huey, R. B. & Hertz, P. E. (1984). Is a jack-of-all-temperatures a master
  of none? *Evolution* 38:441.
* Kleiber, M. (1932). Body size and metabolism. *Hilgardia* 6:315.
* Lindstedt, S. L. & Boyce, M. S. (1985). Seasonality, fasting endurance, and
  body size in mammals. *Am. Nat.* 125:873.
* Nei, M. (1973). Analysis of gene diversity in subdivided populations.
  *PNAS* 70:3321 (G_ST); Nei, M. (1987). *Molecular Evolutionary Genetics*
  (net divergence D_A).
* Pfeiffer, T., Schuster, S. & Bonhoeffer, S. (2001). Cooperation and
  competition in the evolution of ATP-producing pathways. *Science* 292:504.
* Reynolds, C. W. (1999). Steering behaviors for autonomous characters. GDC.
* Smith, C. C. & Fretwell, S. D. (1974). The optimal balance between size and
  number of offspring. *Am. Nat.* 108:499.
* Stanley, K. O. & Miikkulainen, R. (2002) and Gras, R. et al. (2009, EcoSim)
  for distance-based species criteria; Dolson, Rodriguez-Papa & Moreno (2024,
  Phylotrack) for pruned genealogies — via `docs/research/ecology-alife.md` §8.
* Wright, S. (1931). Evolution in Mendelian populations. *Genetics* 16:97.
