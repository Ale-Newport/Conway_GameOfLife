# Ecosystem — the biotic layer of the Ecosystem Simulator

`crates/ecosystem` is the living part of the Ecosystem Simulator. It runs
41 animal species — from elephants to earthworms — with individual
physiology and species-specific decision making, flight, swimming and
burrowing, herds, flocks, schools, packs with leaders, territories, alarm
calls, migration, hibernation, infectious disease and injuries; tiny
organisms as density fields and eusocial insects as colonies with forager
squads, pheromone trails and pollination; seven per-cell vegetation layers,
individual trees, decomposer and mycorrhizal fungi, carcasses and eggs, and a
nutrient and energy cycle in which every transfer is booked. It sits on top
of two foundation crates:

* `physics-world` — terrain, biomes, climate, calendar, weather, hydrology,
  fire, wind and disasters (the abiotic world, `docs/models/physics-world.md`);
* `organism-engine` — agent stores, kinematics and steering (boids
  included), perception queries, utility AI, deterministic claim
  resolution, genomes and genealogy (`docs/models/organism-engine.md`).

The model is `ecosystem::EcosystemSim`, a `simulation_core::Simulation` with
model id `ecosystem`. One tick is 6 simulated minutes (`ARCHITECTURE.md`
§3.3). It also contains a faithful re-implementation of the rules of
NetLogo's *Wolf Sheep Predation* as a separate rule set, and 14 scenarios.

## Contents

1. [Architecture](#1-architecture)
2. [Units, time and space](#2-units-time-and-space)
3. [Species data and the catalogue](#3-species-data-and-the-catalogue)
4. [Animals](#4-animals)
5. [Locomotion and habitat](#5-locomotion-and-habitat)
6. [Social behaviour](#6-social-behaviour-animalsocial)
7. [Seasonal behaviour](#7-seasonal-behaviour-animalseasonal)
8. [Disease and injuries](#8-disease-and-injuries)
9. [Tiny organisms: density fields](#9-tiny-organisms-density-fields-fieldsrs)
10. [Colonies: ants and bees](#10-colonies-ants-and-bees-colonyrs)
11. [Vegetation layers](#11-vegetation-layers)
12. [Trees](#12-trees)
13. [Fungi, litter and decomposition](#13-fungi-litter-and-decomposition)
14. [Energy and nutrient accounting](#14-energy-and-nutrient-accounting)
15. [Coupling with the physical world](#15-coupling-with-the-physical-world)
16. [The NetLogo rule set](#16-the-netlogo-rule-set)
17. [The `ecosystem` model: parameters, commands, metrics, views](#17-the-ecosystem-model)
18. [Scenarios](#18-scenarios)
19. [Observed dynamics](#19-observed-dynamics)
20. [Determinism and snapshots](#20-determinism-and-snapshots)
21. [Performance](#21-performance)
22. [Testing](#22-testing)
23. [Approximations and limitations](#23-approximations-and-limitations)
24. [Extending the model](#24-extending-the-model)
25. [References](#25-references)

## 1. Architecture

### 1.1 Module map

| Module | Content |
|---|---|
| `species` | `Species` (animal records: `behaviour.rs` for social structure, rhythm, hunting, weights, combat and climate; `lifestyle.rs` for seasonal behaviour, water kinds and the representation), `PlantLayer`/`LayerParams`, `TreeSpecies`, `FungiParams`; validation with precise messages; `SpeciesCatalog` (RON loader, embedded built-in set of 41 animals, 7 trees, 7 layers and the fungi; cross references, predator × prey matrix, resolved prey of every diet entry); `AnimalTraits` (Kleiber/MTE metabolism, Hirt speeds, durations in ticks, Gompertz constants, flight altitude, migration range) |
| `animal` | `Animal` state; `perceive` (senses), `hazards` (fire with escape to safe ground, deep water, water search), `decide` (utility AI + steering), `interact` (claims for prey, carcasses, eggs, food, mates and fights; attack outcomes), `physiology` (meals, metabolism, hibernation, drives, growth, injuries, stress, ageing, pregnancy, deaths), `reproduction` (conception, births, eggs, lactation, caps), `social` (boids, pack leaders and shared targets, territorial defence and fights, alarm calls), `seasonal` (migration, hibernation), `genetics` (shared gene schema, phenotype) |
| `fields` | density fields of tiny organisms (insects, spiders, worms) and aquatic forage fish: habitat (land or water of the right kind), feeding, respiration, births, deaths, dispersal, representative markers |
| `colony` | ant and bee colonies: workers, brood, store, swarming, forager squads, pheromone trails, pollination trips |
| `food` (private) | per-cell food shared by fields and colonies (layers, nectar, litter, mushrooms, other fields) |
| `disease` | pathogens, infection records, SIR transmission, recovery and death |
| `schedule` | scheduled scenario events (`ScheduledEvent`, `EventAction`, `WorldEvent`) |
| `introduce` (private) | adding organisms of any representation (founders in groups, field individuals, colonies) |
| `vegetation` | per-cell biomass of 7 layers, seed banks, pollination index and flower seed production; growth, recruitment, losses, dispersal |
| `trees` | individual `Tree`s, `Canopy` (light, LAI, fuel, browse per cell), daily growth, phenology, mortality, seeding |
| `fungi` | `Detritus` (litter and dead wood with nutrients), `Fungi` (mycelium, mycorrhiza, mushrooms); decomposition and spread |
| `carcass` | `Carcass`, `Clutch` (with nest predation), `Remains` (decay into litter), `ClutchIndex` (clutches by position and mother for perception) |
| `ledger` | `Flux` catalogue, `Ledger`/`Ledgers` (cumulative and per-tick bookings), `Stocks` (with fields and colonies) |
| `environment` | fuel and canopy written to the world, fire and wind damage read back, caches (nearest water, fire blocks, cover, daily mean temperature), passability by species (water kinds, depths), felt temperature |
| `netlogo` | the Wolf Sheep Predation rule set |
| `config`, `params`, `scenario` | `EcosystemConfig` (world, terrain — generated, flat, basin, coast — rules, vegetation, trees, soil, animals, parameters, pathogens, events), parameter specs and apply policies, scenario files and loaders |
| `command`, `metrics`, `view`, `sim` | `EcosystemCommand`, metric specs and measurement, render views (`view::inspect`: the organism inspector), `EcosystemSim` |
| `rng`, `units` | deterministic random streams; constants, allometries and response functions |

Private: `state` (the serialisable `EcoState` and its construction) and
`step` (the tick pipeline).

### 1.2 One tick (ecological rules)

```text
1  apply queued NextTick parameter changes; run the scheduled events due at this tick
2  canopy (if trees changed) → write fuel and canopy into the world → World::step
   → book the world's nutrient budget (leaching, volcanic ash) → fire and wind effects
   → field organisms in burning cells die
3  hourly (once per started simulated hour):
     water map, cover and daily-temperature caches; vegetation growth;
     decomposition, fungi, fruiting, mycorrhiza; density fields (feeding, births, deaths,
     dispersal); carcass decay; deposition
4  daily: seed banks (flower seed set from pollination), fungal spread, trees
5  colonies: workers, brood, laying, swarming, forager squads (harvest, trails,
   pollination), pheromone evaporation and diffusion
6  animals:
     index (spatial grid, slot = store index); pack leaders
     perceive + decide          rayon, pure, one random stream per (tick, animal)
     move (+ altitude)          rayon, each animal mutates only itself
     re-index
     interact                   serial: claims → resolve_exclusive / shared / matching
     attacks → fights → meals → matings
     disease draws              rayon, pure (infectious neighbours on the spatial grid)
     physiology → disease outcomes → parental care → births → clutches
     deaths → carcasses → commit (spawns, despawns)
```

Contested resources follow `ARCHITECTURE.md` §3.2: every decision that could
conflict is a `Claim` resolved in a total order (priority, then distance,
then id). Parallel phases only read shared state and write into their own
output slot. No hash map drives any logic.

### 1.3 Dependencies

`simulation-core`, `physics-world`, `organism-engine`, `analytics`
(diversity indices), `serde`, `ron`, `rayon`, `thiserror`. Development only:
`criterion` (benchmark) and `toml` (one test checks every visual model id
against `assets/models/manifest.toml`). No new third-party crates.

## 2. Units, time and space

| Quantity | Unit |
|---|---|
| plant, litter and fungal pools | kg dry matter per m² |
| animal bodies, carcasses | kg wet mass |
| energy | kJ: plant, litter and fungal dry matter 18 kJ/g; animal tissue 7 kJ/g |
| nutrients | g of the world's generic mineral nutrient (one pool standing for N and P); animal tissue 30 g/kg |
| space | one world unit = one terrain cell of `cell_size_m` metres; `x` right, `y` down |
| time | one tick = `1440 / ticks_per_day` minutes (default 6) |

**Compressed calendar.** The world's year has four seasons of
`days_per_season` days (default 30, so 120 days). Physiological rates
(metabolism, thirst, intake, decay, growth rates) are per simulated day.
Life-history durations (lifespans, maturity, gestation, incubation, the
interval between births, parental care, tree growth per year, tree ages)
are given in real days or years and compressed by `days_per_year / 365`
(`units::TimeScale`). See [§23](#23-approximations-and-limitations) for the
consequences.

**Movement time.** A 6-minute tick is far longer than any chase. Movement
represents `motion_seconds` (default 5 s) of continuous locomotion per tick:
a speed of `v` m/s moves `v · motion_seconds / cell_size` cells per tick.
Interaction distances, stamina and the hunting sequence are expressed in
the same units, so chases are resolved over a few ticks while energy and
water budgets follow the full 6 minutes.

## 3. Species data and the catalogue

### 3.1 Files

```text
assets/species/animals/<id>.ron    one `Species` per file (41)
assets/species/trees/<id>.ron      one `TreeSpecies` per file (7)
assets/species/vegetation.ron      the seven `LayerParams`
assets/species/fungi.ron           `FungiParams`
```

`SpeciesCatalog::builtin()` uses copies embedded with `include_str!` (the
application works without the folder); `SpeciesCatalog::load_dir(path)`
reads a directory with the same layout. A test checks that both are equal.
Files are read with a 1 MiB limit; unknown fields are rejected
(`deny_unknown_fields`) and parse errors report file, line and column.

### 3.2 Animal species (`Species`)

| Section | Fields |
|---|---|
| identity | `id` (lowercase), `name`, `scientific_name`, `class` (`TaxonClass`: mammal, bird, reptile, amphibian, fish, insect, arachnid, crustacean, annelid, mollusc) |
| `roles` | any of herbivore, carnivore, omnivore, predator, prey, apex, scavenger, detritivore, pollinator, decomposer, social, solitary |
| `diet` | entries `(food, preference, assimilation?)`; `Food` = `plant(layer)`, `browse` (tree foliage within 2.5 m), `prey("species")` (an individual species is hunted, a density-field species gleaned like forage), `carrion`, `nectar`, `detritus`, `fungi`, `eggs` (other species' clutches) |
| `locomotion` | walk, run, fly, swim, burrow, jump, climb |
| `habitat` | preferred `biomes`, `aquatic`, `water` (`any`, `fresh`, `salt`), `min_depth_m`, `breeds_in_water`, `max_slope`, `max_wade_depth_m`, `refuges` (burrow, dense vegetation, trees, water) |
| `body` | `mass_kg`, `length_m`, `thermoregulation` (endotherm/ectotherm), `body_temperature_c` |
| `metabolism` | `field_multiplier` (FMR/BMR), `fasting_days` (reserve; default Lindstedt & Boyce `5 M^{1/4}/c_th`), `dehydration_days`, `intake_per_hour`, `food_water` (water economy: multiplier of the water gained from food; default 1, reptiles 2) |
| `movement` | `sprint_speed_m_s` (default Hirt et al. 2017), `cruise_fraction`, `endurance_s`, `agility`, `flight_altitude_m` |
| `perception` | `vision_m`, `fov_deg`, `hearing_m` (moving animals), `smell_m` (chemoreception, moving or not; default 0) |
| `life` | `lifespan_years` (≈ 1 % reach it), `maturity_days` |
| `reproduction` | `mode` (sexual/asexual), `development` (`live(gestation_days)` or `eggs(laying_days, incubation_days)`), `litter` range, `breeding_seasons`, `interval_days`, `parental_care_days`, `birth_mass_fraction`, `min_energy_fraction` |
| `social` | `structure` (solitary, pair, herd, pack, flock, school, colony), `group_size`, `cohesion`, `alignment`, `territorial`, `territory_radius_m`, `alarm_calls` |
| `behaviour` | `activity` (diurnal, nocturnal, crepuscular, cathemeral), action `weights` (forage, hunt, scavenge, flee, drink, rest, sleep, mate, wander, defend, migrate), response curves (hunger, thirst, fear, fatigue), `boldness`, `vigilance`, `hunting` (none, ambush, stalk, coursing) |
| `combat` | `attack`, `defence`, `aggression` |
| `climate` | `preferred_c` and `tolerance_c` ranges |
| `seasonal` | `migrates`, `migration_range_m`, `hibernation` (`below_c`, `wake_above_c`, `metabolic_fraction`, `min_reserve_fraction`) |
| `representation` | `individual` (default), `field(...)` (density field, §9) or `colony(...)` (super-agent, §10) |
| `visual` | `model` (id in `assets/models/manifest.toml`), `procedural` (rig id of `organism-render`'s species table), `body_plan`, `color`, `scale`; `Visual::visual_id()` is the model id or `proc:<rig>` |

**Validation** (`Species::validate`, `SpeciesCatalog::new`) checks ranges of
every number, ordered pairs, roles against the diet (a predator must list a
prey, a pollinator nectar, a carnivore prey, carrion or eggs...), social
structure against roles, refuges against locomotion, aquatic species can
swim, spawning in water needs swimming, territorial species have a positive
radius, maturity precedes the lifespan, hibernation wakes above the
temperature at which it starts, model and rig ids are well formed and at
least one exists, no species preys on itself, prey ids exist, ids are
unique; density fields eat only plants, nectar, litter, mushrooms and other
fields, colonies also carrion; field and colony species may only prey on
fields; colony species are not prey; species that hunt individual animals
have a hunting style (insectivores that only glean fields need none).
Errors read like ``species `rabbit`: `body.mass_kg` -1 is outside
[0.000001, 20000]``.

### 3.3 Derived traits (`AnimalTraits`)

For a species of adult mass `M` (kg) and the run's `TimeScale`:

```text
B(T)       = 0.78 · c_th · M^{3/4} · exp(−E/k · (1/T − 1/293.15 K))   W   (MTE; c_th = 1 endotherm, 0.16 ectotherm)
                                                                   E = 0.63 eV, k = 8.617·10⁻⁵ eV/K
field      = B(T_body) · field_multiplier                              kJ/day
reserve    = fasting_days · field                                      kJ   (default fasting_days = 5 M^{1/4} / c_th)
intake     = field · intake_per_hour / ticks_per_hour                  kJ per tick (gross)
thirst     = Δt_days / dehydration_days                                fraction of full hydration per tick
v_sprint   = Hirt(mode, M) = a M^b (1 − e^{−h M^i}) km/h              (running 25.5, 0.26, 22, −0.6; flying 142.8, 0.24, 2.4,
                                                                   −0.72; swimming 11.2, 0.36, 19.5, −0.56); walkers that cannot run: 0.4×
sprint     = v_sprint · motion_seconds / cell_size                     cells per tick; cruise = cruise_fraction · sprint
strike     = max(½ sprint, 0.3)                                        cells (attack distance)
COT        = 10.7 · M^{0.68}                                           J per metre (Taylor et al. 1982), per cell × cell_size;
                                                                   × 0.35 airborne, × 0.12 swimming (§5)
water mem. = max(3 · max(vision, 4), 3 h · cruise)                     cells within which water is remembered
durations  = real days · days_per_year/365 · ticks_per_day             ticks; lifespan = years · days_per_year · ticks_per_day
Gompertz   γ = ln 2 / (L/6),  h₀ = γ ln 100 / (e^{γL} − 1)            S(L) = 1 %; hazard per tick 1 − exp(−h₀ e^{γ·age})
birth mass = M · birth_mass_fraction  (default 0.1 M^{−0.08} / mean litter)
```

For endotherms at 37 °C the MTE form gives 3.06 W·M^{3/4}, close to Kleiber's
3.39 W·M^{3/4} (tested). With the default scale (20 m cells, 240 ticks per
day, 120-day year, `motion_seconds` 5) the built-in species derive to
(`cargo run --release -p ecosystem --example ecosystem_report -- species`;
ectotherm rates at 20 °C; colony and field species per individual):

| Species | Mass (kg) | BMR (kJ/day) | Field (kJ/day) | Reserve (kJ) | Sprint (m/s) | Sprint (cells/tick) | Vision (cells) | Lifespan (days) | Maturity (days) | Gestation or egg formation (days) |
|---|---|---|---|---|---|---|---|---|---|---|
| alpaca | 65.00 | 6,530 | 13,059 | 156,708 | 13.0 | 3.25 | 15.0 | 2,160 | 164.4 | 113.4 |
| ant | 1.0e-6 | 3.4e-4 | 6.8e-4 | 6.7e-4 | 0.1 | 0.01 | 0.1 | 1,800 | 16.4 | 0.3 |
| bat | 9.0e-3 | 7.7 | 23.2 | 70 | 9.0 | 2.25 | 1.0 | 1,200 | 88.8 | 18.1 |
| bear | 250.00 | 16,624 | 33,249 | 1,329,959 | 13.0 | 3.25 | 7.5 | 3,000 | 427.4 | 33.9 |
| bee | 1.0e-4 | 1.1e-2 | 3.2e-2 | 1.0e-1 | 7.5 | 1.88 | 2.5 | 480 | 6.9 | 0.3 |
| beetle | 1.0e-4 | 1.1e-2 | 1.7e-2 | 5.4e-2 | 0.3 | 0.08 | 0.0 | 240 | 39.5 | 3.3 |
| bison | 630.00 | 37,245 | 67,042 | 2,011,253 | 15.0 | 3.75 | 12.5 | 2,400 | 295.9 | 92.1 |
| boar | 100.00 | 9,366 | 18,733 | 280,989 | 13.0 | 3.25 | 5.0 | 1,800 | 118.4 | 37.8 |
| bream | 1.50 | 14.6 | 26.3 | 910 | 1.5 | 0.38 | 0.4 | 2,040 | 480.0 | 1.0 |
| butterfly | 3.0e-4 | 2.5e-2 | 4.9e-2 | 2.0e-1 | 3.0 | 0.75 | 0.2 | 120 | 19.7 | 0.3 |
| deer | 120.00 | 10,739 | 21,478 | 300,685 | 20.0 | 5.00 | 15.0 | 2,400 | 240.0 | 78.9 |
| dolphin | 200.00 | 13,850 | 34,626 | 207,755 | 9.0 | 2.25 | 5.0 | 4,800 | 1084.9 | 120.0 |
| duck | 1.10 | 383.4 | 958.6 | 3,834 | 29.0 | 7.25 | 15.0 | 1,200 | 120.0 | 3.9 |
| eagle | 4.50 | 1,024 | 2,560 | 20,479 | 36.0 | 9.00 | 75.0 | 3,000 | 480.0 | 1.6 |
| elephant | 4000.00 | 123,232 | 209,494 | 8,379,767 | 10.0 | 2.50 | 10.0 | 7,200 | 1315.1 | 217.0 |
| fox | 5.00 | 990.4 | 2,179 | 13,073 | 13.3 | 3.33 | 11.0 | 1,440 | 98.6 | 17.1 |
| frog | 0.03 | 0.7 | 1.4 | 17 | 1.0 | 0.25 | 0.5 | 960 | 360.0 | 9.9 |
| heron | 1.50 | 466.2 | 1,026 | 5,129 | 14.0 | 3.50 | 15.0 | 1,800 | 240.0 | 2.6 |
| horse | 300.00 | 20,252 | 40,504 | 607,567 | 19.0 | 4.75 | 17.5 | 2,400 | 295.9 | 110.1 |
| lion | 170.00 | 13,530 | 27,061 | 270,608 | 22.0 | 5.50 | 25.0 | 1,800 | 328.8 | 36.2 |
| lizard | 0.01 | 0.4 | 0.8 | 8 | 1.7 | 0.43 | 0.5 | 1,200 | 240.0 | 6.6 |
| manta | 700.00 | 1,467 | 2,348 | 46,957 | 6.0 | 1.50 | 2.0 | 4,800 | 1200.0 | 120.0 |
| perch | 0.30 | 4.4 | 8.7 | 202 | 2.0 | 0.50 | 0.5 | 1,440 | 295.9 | 0.7 |
| pike | 3.00 | 24.6 | 44.2 | 885 | 3.0 | 0.75 | 0.8 | 1,800 | 295.9 | 1.0 |
| rabbit | 1.70 | 441.0 | 881.9 | 3,528 | 15.0 | 3.75 | 7.5 | 1,080 | 49.3 | 9.9 |
| rat | 0.30 | 111.3 | 278.3 | 835 | 3.5 | 0.88 | 2.0 | 300 | 24.7 | 7.2 |
| raven | 1.20 | 380.0 | 949.9 | 2,850 | 36.4 | 9.10 | 40.0 | 1,800 | 360.0 | 1.6 |
| reef_fish | 0.05 | 1.1 | 2.3 | 34 | 1.0 | 0.25 | 0.4 | 960 | 120.0 | 4.9 |
| shark | 30.00 | 138.2 | 248.8 | 7,464 | 8.0 | 2.00 | 1.2 | 3,000 | 723.3 | 98.6 |
| sheep | 45.00 | 5,343 | 10,686 | 106,863 | 17.0 | 4.26 | 12.5 | 1,440 | 78.9 | 49.3 |
| small_fish | 3.0e-3 | 0.1 | 0.3 | 2 | 0.8 | 0.20 | 0.2 | 360 | 98.6 | 1.0 |
| snake | 0.10 | 1.9 | 3.5 | 207 | 1.5 | 0.38 | 0.8 | 1,800 | 480.0 | 3.3 |
| songbird | 1.0e-2 | 11.3 | 33.9 | 34 | 12.0 | 3.00 | 5.0 | 600 | 98.6 | 2.0 |
| spider | 5.0e-5 | 6.4e-3 | 1.0e-2 | 2.7e-2 | 0.5 | 0.12 | 0.0 | 240 | 65.8 | 3.3 |
| stag | 300.00 | 21,350 | 42,701 | 768,619 | 20.0 | 5.00 | 17.5 | 2,160 | 249.9 | 81.2 |
| turtle | 0.50 | 6.4 | 10.3 | 270 | 0.4 | 0.10 | 0.8 | 4,800 | 986.3 | 2.3 |
| vulture | 6.00 | 1,270 | 2,541 | 35,575 | 20.0 | 5.00 | 125.0 | 3,600 | 591.8 | 1.6 |
| wasp | 1.0e-4 | 1.1e-2 | 2.7e-2 | 8.4e-2 | 3.0 | 0.75 | 0.1 | 120 | 9.9 | 0.3 |
| wolf | 32.00 | 3,985 | 9,962 | 119,550 | 17.5 | 4.38 | 20.0 | 1,560 | 217.0 | 20.7 |
| worm | 4.0e-3 | 0.2 | 0.3 | 2 | 0.0 | 0.00 | 0.0 | 720 | 120.0 | 3.3 |
| zebra | 250.00 | 17,267 | 34,535 | 518,021 | 18.0 | 4.50 | 20.0 | 2,400 | 295.9 | 118.4 |

Durations are in simulated days of the 120-day year.

### 3.4 Plants and fungi

* `LayerParams` (one per `PlantLayer`: grass, shrubs, flowers, moss, algae,
  aquatic, desert): height, aquatic flag, growth rate `r`, carrying capacity
  `K` per biome, light saturation, beta temperatures, moisture (wilting,
  optimum), nutrient content, Monod half-saturation, turnover and cold /
  drought / flood dieback, specific leaf area, flammability, germination,
  seedling biomass, dispersal, `seed_production_per_day` (flowers 0.02:
  insect-pollinated), mycorrhizal flag, grazing residual, model ids.
* `TreeSpecies`: style (broadleaf, conifer, palm, acacia), deciduous, maximum
  height, mature woody biomass, crown ratio, LAI, specific leaf area, height
  growth per year, maturity, lifespan, shade and drought tolerance, beta
  temperatures, establishment biomes, seeds per year, seeding seasons,
  dispersal kernel (`exponential(mean_m)` or `two_dt(scale_m2, shape)`), wood
  and leaf nutrient contents, bark resistance, flammability, resprouting,
  model ids per stage (young, mature, old, dead). Built in: pedunculate oak,
  silver birch, Scots pine, Norway spruce, kapok (tropical emergent),
  palm and umbrella-thorn acacia.
* `FungiParams`: saprotrophs (litter and wood decay, microbial share,
  half-saturation, yield, mortality, Q10, moisture optimum, humification,
  spread, nutrient content, bacterial decay of detritus in water), fruiting (rate, moisture, temperature window,
  decay, minimum mycelium) and mycorrhiza (growth, capacity per kg of host
  foliage, uptake boost, half-saturation, mortality). Fungi are not plants:
  they take no light.

### 3.5 The built-in catalogue

41 animal species: 17 mammals (including the dolphin), 6 birds, 7 fish, one
amphibian, 3 reptiles and 7 invertebrates; 6 density fields (5
invertebrates and the stickleback, an aquatic field of forage fish) and 2
colony species. Life-history
values come from `docs/research/ecology-alife.md` §10 (AnAge Build 15,
PanTHERIA 1.0, Hirt et al. 2017) where the research table has the species
and are literature orders of magnitude otherwise; behavioural weights,
perception and colony and field parameters are designer values. Every file
starts with a comment that says which is which. Every `model` id exists in
`assets/models/manifest.toml` and every `procedural` rig id in
`organism-render`'s species table (both tested); species with an authored
stand-in model (bison → `bull`, boar → `pig`, eagle → `hawk`) keep a
specific rig for distant level of detail.

| Id | Name | Class | Mass (kg) | Representation | Diet (by preference) | Locomotion | Social and seasonal | Sprint (m/s) | Lifespan (sim. days) | Visual id |
|---|---|---|---|---|---|---|---|---|---|---|
| `alpaca` | Alpaca | mammal | 65 | individual | grass, moss, shrubs, desert | walk, run | herd, alarm calls | 13.0 | 2160 | `alpaca` |
| `ant` | Black garden ant | insect | 1e-06 | colony | carrion, nectar, beetle, grass | walk, climb | colony | 0.1 | 1800 | `proc:ant` |
| `bat` | Little brown bat | mammal | 0.009 | individual | butterfly, beetle, wasp | fly, walk, climb | colony, hibernates | 9.0 | 1200 | `bat` |
| `bear` | Brown bear | mammal | 250 | individual | shrubs, carrion, fungi, bream, grass, perch, deer, beetle, stag, boar, worm | walk, run, swim, climb | solitary, hibernates | 13.0 | 3000 | `proc:bear` |
| `bee` | Honey bee | insect | 0.0001 | colony | nectar | walk, fly | colony | 7.5 | 480 | `proc:bee` |
| `beetle` | Ground and dung beetles | insect | 0.0001 | field | detritus, fungi, grass | walk, fly, burrow | solitary | 0.3 | 240 | `proc:beetle` |
| `bison` | American bison | mammal | 630 | individual | grass, flowers, shrubs, moss | walk, run, swim | herd, migrates | 15.0 | 2400 | `bull` |
| `boar` | Wild boar | mammal | 100 | individual | fungi, worm, shrubs, beetle, carrion, grass, eggs | walk, run, swim | herd, alarm calls | 13.0 | 1800 | `pig` |
| `bream` | Common bream | fish | 1.5 | individual | aquatic, algae, detritus | swim | school | 1.5 | 2040 | `fish_bream` |
| `butterfly` | Meadow brown | insect | 0.0003 | field | nectar, flowers, grass, shrubs | walk, fly | solitary | 3.0 | 120 | `proc:butterfly` |
| `deer` | Red deer | mammal | 120 | individual | grass, shrubs, browse, flowers, moss, fungi | walk, run, jump, swim | herd, alarm calls | 20.0 | 2400 | `deer` |
| `dolphin` | Bottlenose dolphin | mammal | 200 | individual | small_fish, reef_fish | swim | pack, alarm calls | 9.0 | 4800 | `dolphin` |
| `duck` | Mallard | bird | 1.1 | individual | aquatic, algae, grass, worm, beetle | walk, fly, swim | flock, alarm calls, migrates | 29.0 | 1200 | `proc:duck` |
| `eagle` | Golden eagle | bird | 4.5 | individual | rabbit, rat, carrion, duck, snake, lizard | walk, fly | pair, territorial | 36.0 | 3000 | `hawk` |
| `elephant` | African bush elephant | mammal | 4000 | individual | grass, browse, shrubs, flowers | walk, run, swim | herd, alarm calls | 10.0 | 7200 | `proc:elephant` |
| `fox` | Red fox | mammal | 5 | individual | rabbit, rat, carrion, duck, eggs, beetle, worm, frog, shrubs, fungi | walk, run, jump, swim, burrow | solitary, territorial | 13.3 | 1440 | `fox` |
| `frog` | Common frog | amphibian | 0.025 | individual | beetle, spider, butterfly, worm | walk, jump, swim | solitary, hibernates | 1.0 | 960 | `frog` |
| `heron` | Grey heron | bird | 1.5 | individual | small_fish, frog, perch, rat, bream, snake | walk, fly | colony | 14.0 | 1800 | `proc:heron` |
| `horse` | Wild horse | mammal | 300 | individual | grass, shrubs, flowers | walk, run, swim | herd, alarm calls | 19.0 | 2400 | `horse` |
| `lion` | Lion | mammal | 170 | individual | zebra, horse, boar, carrion, elephant | walk, run, swim, climb | pack, territorial | 22.0 | 1800 | `proc:lion` |
| `lizard` | Sand lizard | reptile | 0.012 | individual | beetle, spider, butterfly, wasp | walk, run, climb, burrow | solitary, hibernates | 1.7 | 1200 | `proc:lizard` |
| `manta` | Reef manta ray | fish | 700 | individual | algae | swim | solitary | 6.0 | 4800 | `manta_ray` |
| `perch` | European perch | fish | 0.3 | individual | small_fish, frog | swim | school | 2.0 | 1440 | `fish_perch` |
| `pike` | Northern pike | fish | 3 | individual | perch, small_fish, bream, frog, duck | swim | solitary, territorial | 3.0 | 1800 | `proc:pike` |
| `rabbit` | European rabbit | mammal | 1.7 | individual | grass, flowers, shrubs | walk, run, jump, burrow | colony, alarm calls | 15.0 | 1080 | `rabbit` |
| `rat` | Brown rat | mammal | 0.3 | individual | beetle, eggs, carrion, grass, lizard, flowers, worm, detritus, frog | walk, run, swim, climb, burrow | colony | 3.5 | 300 | `rat` |
| `raven` | Common raven | bird | 1.2 | individual | carrion, shrubs, detritus, fungi, beetle, eggs | walk, fly | pair, territorial | 36.4 | 1800 | `proc:crow` |
| `reef_fish` | Damselfish | fish | 0.05 | individual | algae, aquatic | swim | school, alarm calls | 1.0 | 960 | `fish_clown` |
| `shark` | Blacktip reef shark | fish | 30 | individual | reef_fish, small_fish, carrion | swim | solitary | 8.0 | 3000 | `shark` |
| `sheep` | Sheep | mammal | 45 | individual | grass, flowers, shrubs, moss | walk, run, jump | herd | 17.0 | 1440 | `sheep` |
| `small_fish` | Three-spined stickleback | fish | 0.003 | field (aquatic) | algae, aquatic, detritus | swim | school, alarm calls | 0.8 | 360 | `fish_perch` |
| `snake` | Grass snake | reptile | 0.1 | individual | frog, small_fish, rat, lizard, eggs | walk, swim, climb | solitary, hibernates | 1.5 | 1800 | `snake` |
| `songbird` | Willow warbler | bird | 0.01 | individual | beetle, butterfly, spider, wasp, shrubs | walk, fly | flock, territorial, alarm calls, migrates | 12.0 | 600 | `proc:songbird` |
| `spider` | Wolf spider | arachnid | 5e-05 | field | beetle, butterfly, wasp | walk, climb | solitary | 0.5 | 240 | `spider` |
| `stag` | Wapiti (elk) | mammal | 300 | individual | grass, browse, shrubs, flowers, moss | walk, run, jump, swim | herd, alarm calls, migrates | 20.0 | 2160 | `stag` |
| `turtle` | European pond turtle | reptile | 0.5 | individual | aquatic, carrion, small_fish, algae, worm, beetle | walk, swim | solitary, hibernates | 0.4 | 4800 | `proc:turtle` |
| `vulture` | White-backed vulture | bird | 6 | individual | carrion | walk, fly | flock | 20.0 | 3600 | `proc:vulture` |
| `wasp` | Common wasp | insect | 0.0001 | field | butterfly, nectar, beetle | walk, fly | solitary | 3.0 | 120 | `wasp` |
| `wolf` | Gray wolf | mammal | 32 | individual | deer, stag, sheep, alpaca, boar, horse, bison, rabbit, carrion | walk, run, swim | pack, territorial | 17.5 | 1560 | `wolf` |
| `worm` | Common earthworm | annelid | 0.004 | field | detritus | burrow | solitary | 0.0 | 720 | `proc:worm` |
| `zebra` | Plains zebra | mammal | 250 | individual | grass, flowers, shrubs | walk, run, swim | herd, alarm calls | 18.0 | 2400 | `zebra` |

Fish `litter` values count recruits that survive the larval stage (a pike
spawns 10⁴–10⁵ eggs); frogs' count metamorphs; reptiles' surviving
hatchlings. Fish feel the air temperature of their cell (at least 4 °C in
water) as a proxy for water temperature.

## 4. Animals

### 4.1 State (`Animal`)

Species index, sex, kinematics (position, velocity, heading), wander state,
birth tick, body mass, energy reserve (kJ), nutrient store (g), hydration,
fatigue, stamina, health, thermal comfort, reproductive state (`Juvenile`,
`Ready`, `Gestating(Pregnancy)` with litter, father, embryo energy and
nutrients, `Recovering{until}`), genome (`organism-engine` `Genome`), lineage
(parents, generation), current action, target, action start, home (burrow
or territory centre), hidden flag, drowning and stuck counters, the utility
scores of the last decision and life counters (kills, offspring, distance,
energy eaten, escapes), and since part 2 its altitude (m; negative under
water, §5), injury (0–1, §8), infection record (`Infection`: pathogen,
stage and its end, §8), group id (herd, pack, flock or school, §6) and
the alarm it is calling (origin and strength, §6). Events about each animal (birth, matings, attacks,
death...) are in the global `EventLog`; the followed animal also gets a
trace-level log of its decisions.

### 4.2 Perception (`animal::perceive`, pure)

* Other animals within the vision radius **and** field of view, with the
  radius reduced to `(1 − ½ cover)` in the target's cell and to
  `0.5 + 0.5 vigilance` while foraging; or within the hearing radius if they
  move faster than 30 % of their cruise speed; or within the smell radius
  (`perception.smell_m`, chemoreception: grass snakes 40 m) whether they
  move or not. Burrowed animals are
  invisible; sleeping animals do not see and hear at half range.
* **Threat**: the most dangerous adult or hunting animal of a species that
  eats mine (`1` if it hunts me, `0.8` if it hunts, `0.5` otherwise, times
  proximity); an alarm call heard from a conspecific (§6); a territory
  owner chasing it (`Rival`, §6); fire (burning-cell blocks within vision);
  water deeper than the animal can wade.
* **Prey**: the best `preference · min(1, 2 m_self (1 + ½ pack)/m_prey) ·
  vulnerability / (1 + d/r)` where vulnerability favours injured and young
  prey and the prey must stand on passable ground and be **reachable**:
  ground hunters do not see airborne prey (> 1 m up) as catchable, nor
  non-swimmers prey deeper than they wade (§5).
* **Food**: the best plant-like food on a sampled lattice within
  `min(vision, 6)` cells, valued `Σ preference · min(1, available / R)`
  over the diet with `R` = 0.15 kg/m² for terrestrial forage, browse,
  litter and mushrooms and `min(0.15, ½ K_max)` for aquatic layers (algae
  hold a few dozen g/m² at most; with the terrestrial reference manta rays
  and reef fish took rich algae for scarce food and starved);
  density-field prey (insects, worms, spiders, small fish, §9) counts as
  plentiful from 1 g/m². An **aquatic** field (sticklebacks) is within reach
  of swimmers anywhere and of other animals only where the water is at most
  twice their wading depth (at least 0.3 m): herons glean fish in the
  shallows (`perceive::aquatic_prey_within_reach`). Nest predators (`eggs`
  in the diet) also perceive the clutches of other species within their
  vision, hearing or smell range × `(1 − ½ cover)` of the nest's cell
  (nests in dense cover are harder to find), through a spatial index of the
  clutches (`carcass::ClutchIndex`, rebuilt with the animal index).
* Social and seasonal senses: the perceived conspecifics (boids), the pack
  leader and its hunt target, intruders in the territory (§6), the
  migration target and the urge to hibernate (§7).
* Carcasses it eats (scavengers: any; hunters: their own kill or a prey
  species), conspecifics (herd centroid, velocity, alarm direction of
  fleeing neighbours, crowding), a ready mate of the other sex, the mother
  (dependent young), its clutch, the nearest refuge, the nearest drinkable
  water (remembered within `water_memory_cells = max(3 · max(vision, 4),
  three hours of cruising)` cells, the home range around water: rats 12,
  rabbits 28, bats 34, warblers 36, deer 45, wolves 60 cells) and, for an animal
  with thirst > 0.3 that knows no water, a **search point**: the lowest
  passable of 8 points `clamp(vision, 3, 10)` cells away if it lies lower
  (water collects in valleys), else straight ahead. Within 3 cells of a
  fire, it also looks for the nearest **safe ground** within
  `clamp(vision, 2, 12)` cells: a passable cell that is burned out or
  unburnable (bare, wet, water it can stand in) (`animal::hazards`).

### 4.3 Decisions (`animal::decide`)

Each candidate action is scored with `organism-engine`'s compensated product
of considerations times the species' action weight:

| Action | Considerations | Available when |
|---|---|---|
| forage (graze, browse, forage) | hunger (species curve), food value | plant-like or field food perceived |
| hunt | hunger, prey value, stamina (½ weight) | prey perceived; pack members: the leader's target (value ≥ 0.8, hunger ½ weight) |
| scavenge | hunger, proximity (½ weight); weight ×2 at its own kill | carcass perceived, or another species' clutch (nest predators: weight `max(forage, scavenge) · (½ + ½ egg preference)`) |
| flee | threat − ½ boldness (species fear curve); fire and deep water use weight ≥ 3 | threat (fire: see *Flee* below) |
| drink | thirst (species curve), water distance (0.3 weight), or 0.5 when searching | water known or searching |
| rest | 1 − hunger, 1 − thirst, 1 − stamina (½) | always |
| sleep | fatigue (species curve), circadian window, safety, `1 − u²` with `u` the thirst-curve response (acute thirst keeps animals awake) | always |
| seek mate | condition (½), mate proximity | ready, in season, energy ≥ threshold (without a mate in sight, species spawning in water head for the nearest water, where they meet) |
| wander | the species weight; with nothing to eat in sight at least `½ max(hunt, forage, scavenge)` × the hunger curve (**searching**, at cruise speed); pack members regroup (§6) | always |
| follow parent | distance to the mother | dependent young |
| incubate | 1 − hunger | own clutch |
| defend | aggression, intruder closeness to the territory centre, health × (1 − injury) | a territorial adult with an intruder (§6) |
| migrate | discomfort / 10 °C, 1 − hunger (½ weight) | a migrant outside its preferred climate (§7) |
| hibernate | constant 2.5 | not summer, cold day, enough reserves and water, or already dormant (§7) |

The circadian window is 1 in the species' resting time (diurnal: night;
nocturnal: daylight; crepuscular: 10–15 h and 22–3 h) and 0.15 otherwise.
The current action gets a 15 % momentum bonus; near-ties are broken by 5 %
multiplicative noise from the animal's own stream for the tick. Ectotherms
below their tolerance are torpid, or dormant (brumation) when the
conditions of their hibernation hold. Hungry animals that perceive nothing
to eat search: before this rule a hungry wolf pack with no prey in sight
rested for ten days and never hunted. Different species therefore behave
differently from the same data model: weights, curves, boldness, vigilance,
activity rhythm, hunting style and refuges all differ.

**Execution** (Reynolds steering from `organism-engine`):

* *Hunt*: detect → evaluate (prey value includes distance, size ratio and
  accessibility) → approach by style (ambushers wait until 3 strike
  distances, stalkers creep at ½ cruise until 4 strike distances, coursers
  cruise until 0.6 vision) → pursue at sprint speed → attack within the
  strike distance → kill → eat (the carcass is now its own) → rest when
  satiated.
* *Flee*: to a refuge closer than twice the threat's distance and not
  behind the threat (burrowers disappear into their burrow when they reach
  it), otherwise evade
  the predicted position of the threat; herd animals add a pull towards the
  herd centroid (selfish herd). From **fire**: burrowers dive into a
  burrow within two sprints (the fire passes over); others walk away at
  cruising speed while the fire is more than 3 cells off (saving stamina),
  and closer sprint to the nearest safe ground — burned ground behind a
  wind-driven front is usually nearest, so they dash through the flames
  (one tick in a burning cell costs ½ health) instead of being cornered
  ahead of it. In `animals_escape_a_fire_front_to_burned_ground` 7 of 10
  deer survive a fire that burns 99 % of a plain (before: 3 of 10).
* *Drink*: arrive at the remembered water, or seek the search point.
* *Wander*: Reynolds wander, plus boids cohesion and alignment for social
  species (§6), plus a pull back to the territory centre for territorial
  adults beyond `territory_radius` (weight ramping from 0 at `R` to 1 at
  `2R`); pack members travel with their leader.
* *Defend*, *migrate*, *hibernate*: pursue the intruder, head for the
  migration target, stay hidden in the den (§6, §7).
* Always: obstacle avoidance (feelers over the species' passability) first,
  then the action, then separation; a random escape after 3 blocked ticks.

Movement integrates in sub-steps of at most ½ cell; a species stands only
where it is passable (land with slope ≤ `max_slope` and water ≤
`max_wade_depth_m`; swimmers also in water; aquatic species only in water;
fliers anywhere). The world is clamped (the NetLogo rule set wraps).

### 4.4 Interactions (`animal::interact`, serial and deterministic)

* **Attacks**: a hunter within its strike distance of its prey claims it
  (priority `attack · m^{1/3} · (½ + hunger)`); `resolve_exclusive` keeps at
  most one attack per prey. The kill probability is

  ```text
  logit p = 2.5 (v_p − v_q)/max(v_p, v_q) + 1.2 ln(A/D) − d/strike − 1.5 cover_q + (1 − health_q) + ln(predation_efficiency)
  A = attack · m_p^{1/3} · (1 + 0.3 pack hunters)     D = defence · m_q^{1/3} · (1 + 0.1 herd mates within 2 cells)
  ```

  with sprint speeds reduced by health, exhaustion, injury
  (`× max(1 − 0.6 injury, 0.2)`) and sickness (`× (1 − ½ virulence)`); prey that is not
  fleeing reacts late (× 0.6, × 0.45 against stalkers, × 0.3 against
  ambushers). The outcome is drawn from the predator's attack stream. A
  failed attack may injure the prey (p = 0.3 p, −0.25 health and +0.25 injury)
  and the predator (p = ½ D/(A + D), −0.15 D/(A + D) health and as much
  injury) and always costs the predator
  0.3 stamina. Nothing is scripted: who wins depends on positions, speeds,
  sizes, health, cover and the draw.
* **Carcasses**: `resolve_shared` with ordered sharing: the killer first,
  then by dominance `class + aggression · m^{1/3}/(1 + m^{1/3})` (class 1 for
  predators of animals, 0 for pure scavengers), with +½ for the pack
  leader.
* **Clutches** (nest predation): nest predators within 1 cell of another
  species' clutch claim the eggs they want; `resolve_shared` (ordered by
  hunger, then distance) shares the clutch (egg `assimilation` 0.8); an
  emptied clutch disappears.
* **Fights**: a territory owner in contact with an intruder claims a fight
  (one owner per intruder, `resolve_exclusive`); the outcome and the
  injuries are drawn from the owner's attack stream (§6).
* **Plant food, litter, nectar, mushrooms, browse and density-field prey**:
  proportional sharing per (cell, food) among the claimants, so the last
  biomass of a cell is split fairly and never over-eaten. A forager eats in
  the cell it heads for once that cell's centre is within 1 cell (before,
  only its own cell counted, and dolphin pods hovering at the edge of a
  fished-out cell starved next to plenty).
* **Mating**: `resolve_matching` pairs each receptive female with at most
  one male of her species within 1 cell (priority health × mass); in a
  pack only the dominant adult female and male may mate (§6).
* **Drinking**: an animal whose action is `drink` and that stands next to a
  drinkable cell (lake, river, or ≥ 2 cm of standing water) drinks.

### 4.5 Physiology (`animal::physiology`)

Per tick and animal, in id order:

```text
cost      = B(T) · activity + thermoregulation + locomotion
            activity: sleep 0.85, rest/incubate 1, torpor 0.3, otherwise field_multiplier
            B(T): body temperature for endotherms; for ectotherms the air, the water (≥ 4 °C) when swimming;
            × (1 + 0.1 (vision gene − 1))
            T in shelter (hidden in a burrow or den, dormant, torpid in the litter): the daily mean, ≥ 0 °C
            thermoregulation (endotherms): B · (0.05 (T_low − T)⁺ + 0.03 (T − T_high)⁺)
            locomotion: COT · mode · (m/M)^{0.68} · distance · speed_gene²; mode = 0.35 airborne (> 1 m),
            0.12 swimming (aquatic, or a swimmer in water deeper than it wades), else 1
energy   −= min(cost, energy)              unpaid cost → health −Δt/2 days (starvation)
meal      gross E, N → a·E to the reserve (surplus above capacity respired), a·N to the store
            (surplus above 10 % of body content excreted to the soil), (1 − a) egested as dung (litter)
water     hydration −= thirst · (1 + 0.03 (T − 25)⁺);  drinking +0.25;
            food water: f · food_water · (gross kJ / own daily field need) · daily need,
            f = plants 0.3 + 0.5 ramp(soil moisture, 0.1, 0.4), nectar 1, mushrooms 0.8, meat and browse 0.6, litter 0.1;
            food_water = the species' water economy (1; reptiles 2, pond turtle 1.5)
            hydration 0 → health −Δt/1 day (dehydration)
fatigue   +1.5/day awake, −3/day asleep;  stamina −1/endurance per sprint tick, recovery 1/(3 endurance) (×2 resting)
fever     metabolism × (1 + ½ virulence) while infectious (§8)
dormancy  hibernating: metabolic_fraction · B, no thermoregulation, 5 % of the water loss, no exposure (§7);
            hydration < 0.25 rouses the animal to drink
climate   outside the tolerance range: health −0.5 Δt max(excess/5, 0.2) (exposure)
hazards   deep water for non-swimmers −0.25 health per tick (drowning); burning cell −0.5 (fliers −0.15) (fire)
recovery  health +0.5/day when fed (hunger < ½), hydrated (> ½) and undamaged;
            injury −0.2/day (−0.4 at rest) under the same conditions
ageing    death with the Gompertz hazard of the species
growth    dm = a m^{3/4} (1 − (m/M')^{1/4}) Δt  (West et al. 2001; M' = M × size gene; a reaches 90 % of M' at
            maturity), paid (1 + 0.5) · 7 kJ/g and 30 g/kg from reserve and store
maturity  age ≥ maturity and mass ≥ 0.7 M' → Ready (territorial animals settle a territory centre)
```

Hunger is `1 − reserve/capacity` with the capacity scaled by `(m/M)^{3/4}`.
Water from food and milk is measured against the eater's **own** daily need
(`field · (m/M)^{3/4}`), so small young fed on fresh forage stay hydrated.

### 4.6 Reproduction (`animal::reproduction`)

* **Conception**: granted matings (sexual species) or ready females in
  season with enough reserve (asexual species). Litter = uniform integer in
  the species range × fecundity gene, rounded.
* **Pregnancy** accrues `litter × (birth tissue + ½ reserve at birth mass)`
  kJ and `litter × birth mass × 30 g/kg` from the mother over the gestation
  (or egg formation), paying 50 % synthesis overhead on the tissue part and
  keeping 20 % of her reserve.
* **Birth** (or laying): the young the accrued energy and nutrients can pay
  for, limited by the population caps (suppressed births are counted).
  Live young start with tissue, half a reserve and their nutrient share;
  eggs form a `Clutch` that hatches after the incubation unless it burns or
  floods (its matter then goes to the litter). A bird whose clutch is
  eaten or destroyed is ready to breed again at once and lays a
  replacement clutch if it is still in season (reptiles, amphibians and
  fish wait for their usual interval). Unused investment returns to
  the mother (energy above her capacity is respired, nutrients above her
  store are excreted). The mother then waits `interval_days`. A nest
  predator's bite removes eggs rounded **down**, so every remaining egg
  keeps at least the energy and nutrients it was laid with (rounding to
  the nearest let hatchlings carry more than their eggs: a leak, tested in
  `carcass::tests`). The catalogue test
  `every_species_can_pay_for_its_mean_litter` checks that each species can
  gather the cost of its mean litter (reserve above 20 % plus the net
  intake of its gestation or egg formation): frogs, reef fish and sand
  lizards could not (5 % metamorphs formed in a day) and never bred; their
  `birth_mass_fraction` and `laying_days` (oogenesis) were corrected.
* **Parental care**: during `parental_care_days`, a mother within 2 cells
  feeds each dependent young `min(room, intake cap, her surplus above 30 %)`
  kJ with 2 mg of nutrients per kJ (milk), which also covers its water.
* **Inheritance** (`animal::genetics`): one schema for all species with six
  genes — size, speed, vision (multipliers 0.8–1.2), boldness (±0.3),
  thermal-niche shift (±4 °C) and fecundity (0.8–1.2). Offspring take a
  uniform crossover of both parents (asexual: a clone) followed by Gaussian
  mutation with reflection at the bounds (`mutation_rate` per gene,
  σ = 0.05 of the range). Founders get one full mutation pass of standing
  variation. Trade-offs come from the physiology: larger bodies burn more
  (Kleiber), sprinting costs `speed²`, keener vision costs basal metabolism,
  more offspring split a fixed budget.

### 4.7 Death

Predation, starvation, dehydration, old age, fire, drowning, exposure,
injury, disease, or a kill command. The body (tissue + reserve + embryos) becomes a
`Carcass` at the death position, eaten by its killer first, then by
predators and scavengers in dominance order, and decaying into the litter at
`0.15/day · Q10^{(T−20)/10}` (Q10 = 2; remains below 50 g go at once).
Deaths are counted per species and per cause and recorded in the genealogy.

## 5. Locomotion and habitat

| Mode | Where an animal can stand (`environment::passable`) | Notes |
|---|---|---|
| walkers and runners | land with slope ≤ `max_slope`, water ≤ `max_wade_depth_m` | deeper water drowns them (−0.25 health per tick) |
| swimmers (`swim`) | land and any water | wolves, deer, bears, frogs, turtles, ducks cross rivers and lakes; swimming costs 0.12 of running per metre |
| aquatic species (`habitat.aquatic`) | water of their kind (`habitat.water`: `fresh` = lakes, rivers and floods; `salt` = the ocean; `any`) at least `min_depth_m` deep | a fish stranded on land or in the wrong water is in hostile water (drowning damage, `in_hostile_water`); aquatic density fields (sticklebacks) live in the same water (§9) |
| fliers (`fly`) | anywhere | fliers cross water and burning cells (fire damage × 0.3); airborne flight costs 0.35 of running per metre |
| burrowers (`burrow` + `burrow` refuge) | as walkers | they rest, sleep and hide in their burrow, invisible to predators |

**Altitude** (`Animal::altitude_m`, `AnimalView::altitude_m`). Fliers climb
towards their species' cruising altitude (`movement.flight_altitude_m`:
eagle 120 m, vulture 200 m, duck 50 m, heron 40 m, songbird 10 m, bat 8 m,
butterfly 1.5 m) by `max(5 m, ½ cruise)` per tick while they travel
(wander, migrate, flee, defend, search for prey or water) and descend at the
same rate to land for every other action; a hunter that sprints at its prey
stoops to the ground at once. Fliers that cannot swim stay at least 1 m
above open water. Aquatic animals report a negative altitude: half the water
depth, at most 20 m below the surface. Altitude has consequences, not only
looks: **ground hunters cannot catch prey more than 1 m up** and
**non-swimmers cannot reach prey deeper than their wading depth** (at least
0.3 m) — a fox does not catch a flying duck, a heron only fishes the
shallows, a bear or a pike catches fish anywhere (`perceive::reachable`,
used by perception and by attack claims).

**Water and temperature.** Aquatic animals, and swimming ectotherms
(frogs, turtles, snakes) in water, feel at least 4 °C (deep water does not
freeze); everything else feels the air temperature of its cell. Animals in
shelter — hidden in a burrow or den, dormant, or torpid in the litter —
feel the running daily mean, at least 0 °C (soil and cover damp the daily
cycle and keep frost out). Founders of species that drink are placed within
the range at which they remember water (`water_memory_cells`) when such
places exist.

**Cost of transport.** Taylor et al.'s (1982) running COT
`10.7 M^{−0.32}` J kg⁻¹ m⁻¹ applied to every species made swimming fish and
flying birds pay a runner's price: flight now costs 0.35 of it (≈ Tucker's
1975 `3.6 M^{−0.31}` for birds) and swimming 0.12 (an order of magnitude
below running, Schmidt-Nielsen 1972), by the mode of each tick
(`physiology::transport_cost_factor`, unit-tested).

## 6. Social behaviour (`animal::social`)

* **Groups.** Founders of social species (any structure but `solitary`)
  start in groups — `groups` groups of near-equal size if the scenario says
  so, otherwise sizes drawn from the species' `group_size` — and share a
  group id (`Animal::group`); young inherit their mother's. Packs and pairs
  are founded by a breeding pair (an adult female and an adult male; the
  other founders have random sex and age).
* **Boids** for herds, flocks and schools (`organism-engine`
  `cohesion` and `alignment`): the 8 nearest perceived conspecifics steer an
  animal with weights `social.cohesion` and `social.alignment` (default: the
  cohesion) while it wanders, forages (½ weight) and migrates; its own
  random wander is damped by `1 − ½ alignment`; separation keeps three body
  lengths. In the test `schools_align_their_headings` the mean cosine
  between a stickleback's heading and its nearest neighbour's is 0.29 in a
  school against 0.01 without cohesion and alignment.
* **Packs and prides** (`structure: pack`: wolves, lions, dolphins). The
  leader (alpha) of each group is its highest-ranking member: adults before
  juveniles, then mass × health, then the lower id
  (`social::pack_leaders`, recomputed every tick). Members travel with the
  leader (seek it beyond 2 cells) and **regroup** when they have strayed
  (the wander score rises from 0 at a quarter of their vision to
  `social.cohesion` at their vision, and they travel at cruise speed), so
  the pack moves and rests together; they join its hunt when they can hear it
  (within twice their hearing range; the leader's target becomes their
  prey with value ≥ 0.8 and the hunger consideration at half weight, so
  fed members still help), and let it eat first at a carcass (+½ rank).
  **Reproductive suppression**: only the pack's dominant adult female and
  adult male (highest mass × health of each sex) may mate; when one dies
  the next-ranking adult takes over
  (`packs_are_founded_by_a_breeding_pair_and_only_the_dominant_pair_breeds`).
  Without it, every adult female of the balanced meadow's pack bred in the
  same winter (15 cubs from a pack of five).
  Coursing and stalking packs add `0.3` attack per pack mate within 3 cells
  of the prey (§4.4).
* **Territories.** A territorial adult keeps a territory centre (`home`)
  and a radius; beyond the radius its exploration is pulled back (§4.3).
  It now notices **intruders** — adult conspecifics of another group (or,
  for species without groups, of its own sex) inside its territory — and
  scores `defend` with `aggression × closeness to the centre × health ×
  (1 − injury)`. A defender pursues the intruder at 0.8 × sprint; on contact
  (`max(1 cell, strike distance)`) a fight is claimed (one owner per
  intruder) and resolved with
  `P(owner wins) = σ(2 ln(S_o/S_i) + 0.5)`,
  `S = (aggression + 0.2) · m^{1/3} · health · (1 − ½ injury)`; the loser
  gains 0.15–0.3 injury and loses 0.05 health (it can die of its injuries),
  the winner is scratched (+0.05) with probability 0.3. An intruder
  perceives a conspecific chasing it as a threat (`Rival`) and flees. Foxes,
  wolves, lions, eagles, ravens, pike and warblers are territorial.
* **Alarm calls** (`social.alarm_calls`: rabbits, deer, wapiti, zebras,
  elephants, horses, alpacas, boar, ducks, warblers, sticklebacks, reef
  fish, dolphins). An animal that flees from a predator calls with strength 1
  (`Animal::alarm`); a conspecific that hears a call of strength `s` at
  distance `d` perceives the caller's threat with level
  `0.7 s (1 − ½ d/hearing)` and, while it flees from it, relays the call
  with that level as strength if it is at least 0.2. The alarm spreads a few
  hops through the group and stops when nobody sees the predator any more
  (a relayed call is always weaker than the call it relays, so a herd cannot
  keep itself in permanent panic). Fish alarm substance (Schreckstoff) is
  modelled the same way.
* **Parental care** (§4.6): dependent young follow their mother (score
  `2 · min(1, d/3)`) and are fed (milk) within 2 cells; mothers guard their
  clutch (`incubate`).

## 7. Seasonal behaviour (`animal::seasonal`)

* **Migration** (`seasonal.migrates`: wapiti, bison, ducks and
  warblers). When the running daily mean temperature where a migrant stands
  is more than 1 °C outside its preferred range (shifted by its thermal
  gene), it samples the daily mean at 8 passable points
  `migration_range_m` away and heads for the one closest to its range if it
  is at least 0.5 °C better. The `migrate` action scores
  `weight` times the compensated product of `discomfort / 10 °C` and
  `1 − hunger` (half weight); flocks and herds migrate
  together through boids. In `migrants_travel_towards_their_preferred_climate`
  ducks released at the cold edge of a map with a 40 °C gradient move from
  row 8 to row 54 in a day; non-migrant controls stay at row 8.
* **Hibernation** (`seasonal.hibernation`: brown bear, bats, frogs, grass
  snakes, pond turtles, sand lizards). Outside summer (autumn, winter, or a
  spring that is still cold), an animal whose daily mean temperature is
  below `below_c`, whose reserves exceed `min_reserve_fraction` of capacity
  and whose hydration is at least ½ enters dormancy (score 2.5, above
  every other drive except fleeing fire or floods); it stays until the daily
  mean exceeds `wake_above_c` (a gap prevents flickering), a fire or flood
  threatens it, or its hydration falls below ¼ (**periodic arousal** to
  drink, as hibernating bats do). Scenarios start in early spring, which
  is still cold in continental climates: allowing dormancy only in autumn
  and winter left bats, frogs and snakes of `extreme_seasons` to die of
  exposure in their first week, and dormant bats without arousal died of
  thirst after 20 days. A dormant animal hides in its den (invisible), does not move,
  pays `metabolic_fraction` of its basal rate (bear 0.25, bat 0.03,
  reptiles and frogs 0.1–0.2), no thermoregulation, 5 % of its water loss
  and no exposure damage. In `bears_hibernate_in_the_cold_season_and_save_energy`
  a dormant bear spends 0.12 days of field metabolism per winter day against
  0.47 for an identical bear without hibernation.
* Ectotherms below their tolerance are torpid (§4.3); a torpid ectotherm
  that meets the conditions of its hibernation goes dormant instead
  (brumation; before, torpor pre-empted the choice and frogs sat torpid in
  the frost). Hibernation usually starts earlier, at a daily mean still
  above the tolerance. Tests: `torpid_ectotherms_brumate_in_a_cold_spring`,
  `hibernators_rouse_to_drink_when_their_water_runs_low`.

## 8. Disease and injuries

**Pathogens** are part of a scenario (`config.pathogens`):

```ron
pathogens: [(
    id: "rhd", name: "Rabbit haemorrhagic disease", hosts: ["rabbit"],
    transmission_per_hour: 0.02, contact_m: 30.0, infectious_days: 2.0,
    mortality_per_day: 0.4, immunity_days: 0.0, virulence: 0.6,
)],
```

Outbreaks are started by a scheduled `outbreak` event or the `Outbreak`
command (random susceptible hosts of a species). SIR dynamics per tick
(`disease.rs`):

```text
transmission  susceptible host with k infectious hosts of pathogen p within contact_m:
              P = 1 − (1 − β)^{k Δt_h}                                 β = transmission_per_hour
recovery      P = 1 − e^{−Δt_d / infectious_days}  → immune for immunity_days (0 = for life)
death         P = 1 − e^{−μ Δt_d}                  → DeathCause::Disease (carcass as usual)
sickness      speed × (1 − ½ v), metabolism × (1 + ½ v)   v = virulence
waning        immunity ends → susceptible again
```

Counting infectious neighbours is a pure parallel phase on the spatial grid;
draws come from one stream per `(tick, animal)`; outcomes are applied in id
order after physiology. Metrics: `infected.<id>`, `recovered.<id>`,
`new_infections.<id>`, `disease_deaths.<id>`. Tests: an epidemic in a flock
of 60 sheep peaks at 38 infected and burns out (55 recovered, lifelong
immunity: `total = recovered + dead + still infected`); weak transmission
(`R₀ < 1`) fizzles; virulent strains kill.

**Injuries** (`Animal::injury` in `[0, 1]`) come from failed attacks (the
prey's and the predator's injury) and territorial fights. They slow the
animal (`speed × (1 − 0.6 injury)`, so injured prey are caught more easily)
and lower its defence in fights; they heal by 0.2 per day (0.4 at rest)
while the animal is fed and watered. Health damage of the same events can
kill (`DeathCause::Injury`).

## 9. Tiny organisms: density fields (`fields.rs`)

Insects, spiders, worms and forage fish live in numbers no agent model can
afford (the temperate-forest scenario holds 40 million beetles and 60
million earthworms; the ocean scenario 3 million small fish). Species with
`representation: field(...)` (butterflies, spiders, ground and dung
beetles, earthworms, common wasps, and three-spined sticklebacks) are
therefore an **aggregate representation**: a density (individuals per m²)
per cell, updated hourly at a cost of `O(cells)` whatever the number of
organisms. Terrestrial fields live on dry land; **aquatic** fields
(`habitat.aquatic`: sticklebacks) live in water of their kind (fresh or
salt) at least their minimum depth, feel the water temperature (the air,
at least 4 °C), do not burn, and their markers swim below the surface.
Sticklebacks became a field because their predators could not live on
them as individuals: a heron needs ≈ 50 sticklebacks a day and a dolphin
≈ 1,600, so 200–300 individual fish were eaten out within days and herons
and dolphins starved (a heron or a dolphin catches one individual per
attack):

```text
active     T ≥ T_pref,low − 5 °C and T ≤ T_tol,high (otherwise dormant: no feeding, basal metabolism)
B(T)       = 0.78 · 0.16 · m^{3/4} · e^{−E/k (1/T − 1/293 K)}       W per individual (MTE, ectotherm)
R          = n B(T) (field multiplier if active) Δt                respiration
F          = Σ_k pref_k F_k                                       food (kg/m²): plants above residuals,
                                                                  nectar, litter, mushrooms, other fields
I          = B(T) · fm · 24 · intake_per_hour · F/(F + K_h)       gross intake per individual and day (Holling II)
eaten_k    = min(n I Δt pref_k F_k / F / ε_k, ½ F_k)               ε_k: energy density
A          = Σ a_k eaten_k ε_k                                    assimilated; (1 − a_k) egested to the litter
A ≥ R:     births = min(max_birth n Δt, (A − R)/(1.5 e), assimilated N / (m · 30 g/kg)); the rest is respired,
           surplus nutrients excreted
A < R:     tissue of (R − A)/e individuals burned (starvation), its nutrients excreted
deaths     n μ (1 + n/K) Δt → litter (½ μ while dormant)            K = max_density (preferred biomes; 0.1 × elsewhere; 0 outside
                                                                  the habitat: water for terrestrial, land for aquatic fields)
dispersal  dispersal_per_day Δt of every cell to its 8 habitat neighbours (conservative)
fire       80 % (fliers 30 %, aquatic 0) of the organisms of a burning cell die per tick → litter
```

`e = m · 7 kJ/g`. All transfers are booked (herbivory, nectarivory,
detritivory, fungivory, predation on other fields, egestion, excretion,
respiration, mortality) and the fields are a stock of their own, so both
ledgers still close (test `fields_close_the_energy_and_nutrient_budgets`:
nutrient stock constant to 10⁻⁵ over three days with five field species and
lizards eating them). Fields are **food**: insectivores (warblers, bats,
lizards, frogs, rats, foxes, boar, ducks, turtles, ravens, bears) and
piscivores (herons, perch, pike, sharks, dolphins, grass snakes, turtles)
forage them like plants (a gram per m² counts as plentiful) through
ordinary proportional food claims — non-swimmers only where the water is
at most twice their wading depth (tests in `tests/aquatic.rs`); spiders and wasps eat other fields in the same
update. Litter is poor in nutrients, so detritivore growth on it is
nutrient-limited (beetles on 1 kg/m² of litter grow ≈ 1 %/day and starve
without food: `field_populations_follow_their_food`).

**Display.** `EcosystemView::field_markers` carries up to three
*representative markers* per cell and species (one per
`individuals_per_marker`, at most `max_markers` per view) with positions
derived from a hash of the cell, species, marker and tick (they drift
slowly so they can be animated) and the species' flight altitude. They are
not simulated individually; this is documented in the view and here.

## 10. Colonies: ants and bees (`colony.rs`)

Eusocial insects are **colony super-agents** (research note §10.4):
`representation: colony(...)` (black garden ant, honey bee). A `Colony`
holds its nest, workers, cohorts of brood and a food store (kJ and g); a few
**forager squads** (`Forager`, each standing for `workers_per_forager`
workers) move on the map. Per tick:

```text
T_nest       33 °C in hives (nest_temperature_c); the air clamped to 4–30 °C in soil nests
respiration  workers · B(m, T_nest) · (field multiplier while foraging) · Δt from the store;
             a deficit is paid with worker tissue (starvation; nutrients stay in the store)
mortality    workers · Δt / worker lifespan (compressed like every life history) → litter at the nest
laying       while foraging is possible and the store keeps two days of metabolism:
             min(max_eggs_per_day · Δt (per real day), room below max_workers, affordable), each egg costs
             1.5 × worker tissue energy and its nutrients; brood emerges after development_days
swarming     breeding season, workers ≥ swarm_workers, store > worker tissue: half the workers and store
             found a colony at a dry place within the foraging range (≤ 500 colonies per species)
squads       min(max_foragers, workers · forager_fraction / workers_per_forager) while it is warm enough
             (air ≥ preferred minimum) and, for diurnal species, light; otherwise they return and retire
departure    fliers: to the patch reported by returning foragers (recruitment, p = 0.7; bees' dance) or a
             scouting flight to a random point of the foraging range; walkers: search from the nest
searching    harvest the cell's food (the first diet item with ≥ ½ load; carrion within one cell first) or move:
             towards carrion smelt within max(1.5, vision) cells, outwards along the strongest trail pheromone
             (ants, full speed), towards the best food sensed, or a correlated random walk (¼ of the travel
             distance); beyond the foraging range: home
returning    straight home; loaded ants mark every cell they cross with pheromone; at the nest the load is
             assimilated into the store (the rest egested) and the patch quality reported
pheromone    p ← p (1 − ln 2 Δt / half-life), then a fraction D exchanged with the 4 neighbours
```

Squads move `cruise speed × tick seconds × travel_fraction` per tick (at
most 8 cells): ants ≈ 11 m, bees ≈ 160 m (a trip of a bee to a patch 1 km
away takes about an hour with flower handling). Ant trails resolve at the
cell size (use small cells to see them; the tests use 1 m cells). In
`ant_foragers_lay_a_trail_between_nest_and_food` the mean pheromone on the
cells between the nest and a carcass is 4.7 against 0.13 at the same
distance elsewhere; `pheromone_evaporates_with_its_half_life` checks the
decay (half in one half-life).

**Pollination.** Every nectar meal records flower visits: bee squads
`workers × flower_visits_per_trip` (75), butterflies and wasps
`flower_visits_per_hour` per individual, individual nectar feeders one visit
per 0.5 mg of nectar. The **pollination index** of a cell rises by
`visits / flowers` (5,000 flowers per kg of flower biomass) and halves every
day. Flower **seed production** is `B/K · seed_production_per_day ·
(0.3 + 0.7 pollination)` (30 % from wind and self pollination); the flower
seed bank receives it (seed production 0.02 per day for flowers, so the
bank no longer saturates and recruitment after disturbance depends on
pollinators) and `flowers.seed_production` reports
`biomass · 2,000 seeds/kg/day · seed set`. Tests: with two hives on a 16 ha
meadow the pollination index reaches 0.06 and seed production rises by
11 % in four days (`pollinators_raise_flower_seed_production`); with
butterflies the gain stays below the bound `1/0.3`
(`pollination_index_raises_seed_set_linearly`).

## 11. Vegetation layers

Seven per-cell biomass fields (kg/m²): grass, shrubs, flowers (nectar), moss
on land; algae and rooted aquatic plants in water (aquatic layers also grow
in deeply flooded wetland cells); desert vegetation. Hourly, per cell, layers
from the tallest down:

```text
I_i   = I_ground · exp(−0.5 Σ_{taller j} LAI_j),       LAI_j = B_j · SLA_j       (Beer–Lambert, Monsi & Saeki)
G     = r B (1 − B/K_biome) · min(1, I_i/L_sat)/0.4 · f_T · min(f_W, f_N) · Δt · plant_growth
f_T   = beta function (Yan & Hunt 1999);  f_W = ramp(soil moisture, wilting, optimum) (aquatic: 1)
f_N   = min(1, N/(K_N + N) · (1 + b·M/(M + K_M)))      (mycorrhizal layers; M = mycorrhizal biomass)
R     = seedling · germination · seed bank · f_T · f_W · Δt                     (while B < K/2)
loss  = B (turnover + cold + drought + flood dieback) Δt   → litter            (≥ 0.2/day outside the habitat)
```

The light sum `I_ground` is the world's light field below the tree canopy.
The factor `1/0.4` is the mean of the light-saturation factor over a clear
day, so a clear, light-saturated day integrates to the daily rate. Growth
takes `G · n_layer` grams of nutrients from the soil (aquatic layers also
from the 8 banks); when the soil holds less, growth is scaled down, so mass
is conserved (tested). The physical world gives water no soil fertility,
so water cells start with `soil.water_nutrients_g_m2` (default 2 g/m²) of
dissolved nutrients; without them algae could only shrink by their
turnover (−10 %/day), and every aquatic food chain starved within weeks
(test `dissolved_nutrients_keep_algae_growing`). Daily, each seed bank gains
`B/K · seed_production_per_day` (1 for wind-dispersed layers; flowers 0.02
× `(0.3 + 0.7 pollination)`, which pollinators raise, §10), decays 2 %/day
and receives `dispersal × mean(neighbour banks)`. Herbivory removes biomass down
to the layer's grazing residual.

## 12. Trees

Individual trees with stable ids:

```text
wood(H)    = B_max (H/H_max)^{2.5}            crown radius r = c·H        full leaves = LAI · πr² / SLA
ΔH / day   = g/days_per_year · (1 − H/H_max) · f_light · f_T · min(f_W, f_N) · tree_growth
f_light    = min(1, I / (0.1 + 0.6 (1 − shade_tolerance)))      I = light at the crown
f_W        = ramp(moisture, w, w + 0.25),  w = 0.25 (1 − drought_tolerance)
f_N        = min(1, N/(2 + N) · mycorrhizal boost)
```

* **Light competition**: each crown covers the cells of its disk (sampled on
  a 7 × 7 lattice); the light at a crown is the mean over its footprint of
  `exp(−0.5 LAI_above/area)` through the strictly taller crowns (ties by
  store order). The canopy also gives each cell its tree LAI, cover
  `1 − e^{−0.5 LAI}`, crown fuel and browse within 2.5 m.
* **Phenology**: deciduous leaf mass × (spring: fraction of the season,
  summer 1, autumn 1 − fraction, winter 0); dropped leaves go to the litter.
* **Stages**: seedling (< 1.3 m), sapling, mature (≥ maturity age and 30 %
  of H_max), old (≥ 70 % of the lifespan), dead (standing snag).
* **Stress and death**: deep shade in the growing season, soil below the
  wilting point and temperatures 10 °C below the minimum or above the
  maximum each cost 0.02 health per day (+0.01 per day otherwise). Death by
  exhausted health or the Gompertz hazard (1 % reach the lifespan); leaves
  fall at once; the snag stands 60 days and then falls into the dead-wood
  pool.
* **Seeding**: mature healthy trees in their seeding seasons release
  Poisson(`seeds_per_year / season days × health`) seeds per day at kernel
  distances (exponential with mean `2α`, sampled exactly as Gamma(2, α); or
  Clark's 2Dt, inverted CDF). A seed establishes on dry land of a listed
  biome below the per-cell trunk cap with probability
  `0.05 · f_light(ground) · f_W · (1 − 0.5 min(grass/0.5, 1)) · max(f_T, 0.2)`
  if the soil holds the seedling's nutrients.

## 13. Fungi, litter and decomposition

`Detritus` keeps litter (leaf fall, dieback, dung, carcass remains, dead
fungi; as dry-matter equivalents with exact energy) and dead wood per cell,
each with its nutrients. `Fungi` keeps saprotrophic mycelium, mycorrhizal
biomass and mushrooms per cell. Hourly:

```text
sat    = F / (F + K_F)                                    f = Q10^{(T−20)/10} · hump(moisture) · decomposition
dL     = L · k_L (m + (1 − m) sat) f Δt                    litter (m = bacterial share); in water: L · k_aq · Q10 Δt
dW     = W · k_W sat f Δt                                  dead wood: fungi only (white rot)
ΔF     = yield · (fungal share of dL + dW)                 mycelium growth
humus  = h (dL + dW)                                       → the world's soil organic matter (h = 0 at sea)
N_free = nutrients(dL + dW)(1 − h) − ΔF n_F                > 0 mineralised to the soil, < 0 immobilised from it
```

The rest of the decomposed energy is respired. `hump` peaks at the optimal
relative moisture (Linn & Doran 1984); decomposition stops below −2 °C.
In water, bacteria decompose labile detritus (dead algae, dung, remains)
at `aquatic_decay_per_day` (0.1/day at 20 °C; phytoplankton detritus
0.05–0.3/day, Enríquez, Duarte & Sand-Jensen 1993) without fungi; at sea
nothing is humified, because the world does not mineralise organic matter
in ocean cells — marine detritus is remineralised in the water column.
With the terrestrial litter rate (≈ 0.005/day without fungi) dead algae
locked the dissolved nutrients of a lake or the sea into litter within
three weeks.
Mycelium and mycorrhiza die back into the litter, mushrooms fruit from the
mycelium when the soil is moist and the temperature mild and rot within
days; mycelium spreads to its 4 land neighbours by a conservative exchange
(daily). Mycorrhiza grow logistically on host foliage (shrubs and tree
leaves) with nutrients from the soil and raise the nutrient uptake of
mycorrhizal plants and trees by `b · M/(M + K_M)`. Fungi are not plants:
they take no light and live on dead matter or plant carbon.

## 14. Energy and nutrient accounting

Every transfer is booked as a `Flux` in two ledgers (kJ and g). Each flux
is an *input*, an *output* or *internal* in each ledger:

| Flux | Energy | Nutrients |
|---|---|---|
| primary production, mycorrhizal carbon | input | internal (soil → plant / fungus) |
| deposition, volcanic deposition | — | input |
| intervention in / out (spawn, remove, plant) | input / output | input / output |
| herbivory, predation, scavenging, nectarivory, detritivory, fungivory | internal | internal |
| egestion (dung), carcass decay, litterfall, tree mortality, animal mortality, fungal growth, fungal mortality, fruiting, reproductive investment | internal | internal |
| excretion, mineralisation, immobilisation, ash, leaching retained | — | internal |
| animal respiration, decomposer respiration, reproductive loss | output | — |
| humification | output (to soil organic matter) | internal |
| combustion | output | — |
| volatilisation, leaching | — | output |

**Stocks** (`Stocks`): vegetation, trees, animals (tissue, reserves,
embryos), clutches, carcasses, density fields (§9), colonies (workers,
brood, stores and loads in transit, §10), litter, dead wood, fungi, and
the world's mineral and organic soil nutrients. The books must satisfy

```text
stock(now) = stock(start) + Σ inputs − Σ outputs          for energy and for nutrients
```

and the metrics `energy_balance_error` and `nutrient_balance_error` report
`|stock − books| / stock` every tick.

**External nutrient fluxes are declared per scenario** through parameters:
`deposition_g_m2_year` (atmospheric input onto land), `leaching_loss`
(nutrients carried below the root zone by percolating water leave the
ecosystem; when off they are returned to the soil of the cells that lost
them, in proportion to each cell's loss during the world step),
`volatilisation_fraction` (share of burned nutrients lost to the air, the
rest returns as ash) and the master switch `external_fluxes` — off closes
the budget (no deposition, no volatilisation, leaching retained). Disasters
(volcanic ash) and commands (spawning, removing, planting) remain explicit,
booked inputs and outputs.

**Measured closure.** With `external_fluxes` off the nutrient stock stays
constant to 10⁻⁵ relative over 4–10 days, also under heavy rain that leaches
(tests `nutrients_are_conserved_without_external_fluxes`,
`a_closed_budget_retains_leached_nutrients`). With all fluxes on, fire,
interventions and leaching, both ledgers close to 10⁻⁵
(`ledgers_close_with_external_fluxes_fire_and_interventions`,
`open_budget_loses_leached_nutrients_and_still_balances`). In the
balanced-meadow run the nutrient error grows by ≈ 3·10⁻⁷ per simulated day
(§19.2). We traced it: after 40 days the ledger
discrepancy is 2,417 g, and physics-world's own `f32` soil grids differ from
its own `NutrientBudget` (same grids, independent bookkeeping) by 2,424 g;
the ecosystem's pools differ from their books by only −7 g out of 4·10⁸ g
(2·10⁻⁸). The energy error stays at about 10⁻⁶ over two years. With five
density-field species, colonies, eggs and nest predators the closed
budget still holds to 10⁻⁵ (`fields_close_the_energy_and_nutrient_budgets`).

## 15. Coupling with the physical world

* **Before each world step** the ecosystem writes per cell
  `fuel = Σ flammability · vegetation + litter + 0.3 dead wood + Σ tree foliage
  × flammability` (kg/m²) and `canopy = 1 − e^{−0.5 LAI_trees}`.
* **After the step** it books the world's nutrient budget (leaching and
  volcanic ash, see §14), then reads `fuel_consumed`: each burned cell loses
  the same fraction of every fuel component; the energy is combustion, the
  nutrients split into volatilisation and ash (`add_nutrients`). Trees in
  burning cells lose foliage and die with
  `p = 1 − exp(−2 I (1 − bark) / (1 + H/10))` (I = fuel consumed, kg/m²);
  resprouters lose the stem above 0.5 m instead and regrow; survivors are
  scarred and lose health. Tornado damage `d` flattens vegetation (× layer
  height ≤ 1 m) into litter and throws trees with probability
  `d (0.3 + 0.7 H/H_max)` into the dead-wood pool.
* **Animals**: burning cells injure (−0.5 health per tick) and kill; fire is
  perceived and fled (flee weight ≥ 3 whatever the boldness); water deeper
  than the wading depth drowns non-swimmers, who flee it (floods); clutches
  in burning or flooded cells are lost; 80 % of the field organisms of a
  burning cell (fliers 30 %) die per tick; air temperature drives metabolism,
  thermoregulation costs, exposure and ectotherm torpor; drought dries the
  forage (less preformed water) and the soil.
* **Plants**: soil moisture (drought dieback, wilting), temperature (beta
  functions, cold dieback, winter phenology), light and day length, standing
  water (flood dieback of land layers, habitat of aquatic layers), terrain
  changes (outside-habitat dieback). Seasons act through temperature,
  daylight and phenology.
* **Caches**: nearest drinkable cell per cell (multi-source BFS, 8
  neighbours, up to 256 cells; the ocean is salt), burning cells per 4 × 4
  block, shelter cover `1 − (1 − shrubs/0.6)(1 − canopy cover)`, 24-h mean
  temperature. They are refreshed on a schedule and part of the snapshot.

## 16. The NetLogo rule set

`rules: netlogo((...))` replaces the ecological rules by a re-implementation
of NetLogo's *Wolf Sheep Predation* (Wilensky 1997; rules re-implemented
from `docs/research/ecology-alife.md` §1, no NetLogo code or text copied).
The world is a torus of patches; the physics world steps but does not affect
the animals. Setup: each patch green with p = ½ (green: countdown = regrowth
time; brown: `random(regrowth)`), animals at uniform random positions with
`energy = random(2 × gain)`. Each tick, in a fresh seeded random order:
sheep wiggle (`rt random 50`, `lt random 50`) and step 1; in the grass
version lose 1, eat a green patch (+gain, patch turns brown) and die below
0; reproduce with `sheep_reproduce %` (halve energy, hatch one lamb that
turns randomly and steps 1). Wolves move, lose 1, eat one random sheep on
their patch (+gain), die below 0, reproduce like sheep (not gated by
energy). Brown patches turn green when their countdown is ≤ 0, otherwise
count down. The run finishes when no animal is left, or wolves are extinct
and sheep exceed `max_sheep`. The descriptor's rule text switches to this
description and the NetLogo sliders appear as parameters.

## 17. The `ecosystem` model

`EcosystemSim` implements `Simulation` with `Command = EcosystemCommand`,
`ViewRequest = EcosystemViewRequest`, `View = EcosystemView`,
`tick_unit = "minute"`, `dt = 1440 / ticks_per_day` (6), family `Hybrid`,
deterministic. `descriptor().rules` is the Markdown of the active rule set
(`src/rules_ecological.md` or `src/rules_netlogo.md`), kept in sync with the
code above.

### 17.1 Parameters

| Key | Range (default) | Policy |
|---|---|---|
| `width`, `height` | 16–1024 (96) | requires reset |
| `cell_size_m` | 1–100 (20) | requires reset |
| `days_per_season` | 1–3650 (30) | requires reset |
| `seed` | 0–2⁶³−1 (1) | requires reset |
| `rainfall_mm`, `base_temperature_c`, `seasonality`, `wind_speed`, `fire_spread`, `lightning_rate` | 0–8000 (650), −40–45 (13), 0–40 (9), 0–60 (4), 0–5 (1), 0–10 (0.02) | next tick (written into the world's runtime configuration: weather overrides) |
| `plant_growth`, `tree_growth`, `decomposition`, `mycorrhizal_boost` | 0–5 (1) | next tick |
| `max_trees_per_cell`, `max_trees` | 1–64 (4), 0–10⁶ (100,000) | immediate |
| `motion_seconds` | 1–120 (5) | next tick (derived traits rebuilt) |
| `mutation_rate` | 0–1 (0.1) | next tick |
| `predation_efficiency` | 0.05–20 (1, logarithmic) | next tick |
| `max_population_per_species`, `max_population_total` | 1–10⁶ (5,000, 20,000) | immediate (population caps) |
| `deposition_g_m2_year`, `volatilisation_fraction`, `leaching_loss`, `external_fluxes` | 0–100 (0.5), 0–1 (0.5), bool (true), bool (true) | next tick |
| `pheromone_half_life_h`, `pheromone_diffusion` | 0.05–100 h (1, logarithmic), 0–0.25 (0.05) | next tick (ant trails, §10) |
| NetLogo only: `sheep_gain_from_food`, `wolf_gain_from_food`, `sheep_reproduce_percent`, `wolf_reproduce_percent`, `grass_regrowth_time` | 0–1000 (4, 20), 0–100 (4, 5), 0–100,000 (30) | next tick |
| NetLogo only: `max_sheep` | 0–10⁷ (30,000) | immediate |
| NetLogo only: `initial_sheep`, `initial_wolves`, `grass` | 0–100,000 (100, 50), bool (true) | requires reset |

`NextTick` changes are queued and applied at the start of the next step
(`params()` already reports them); `RequiresReset` changes are stored in the
configuration and used by `reset()`. Values are validated against the specs
(out-of-range values are rejected, not clamped).

### 17.2 Commands (`EcosystemCommand`)

| Command | Effect |
|---|---|
| `SpawnAnimals { species, x, y, radius, count }` | 1–1000 animals with random age and condition near a point; for a field species `count` individuals added to the cell density, for a colony species `count` colonies (booked as intervention input) |
| `KillAnimal { id }` | the animal dies (carcass) |
| `RemoveAnimal { id }` | the animal leaves the world (intervention output) |
| `PlantTree { species, x, y }` | a seedling on land (nutrients from the soil) |
| `FellTree { id }` | the tree falls into the dead-wood pool |
| `Ignite { x, y }` | sets a cell with dry fuel burning |
| `Disaster(Disaster)` | any physics-world intervention: rain, drought, heatwave, storm, flood, earthquake, tornado, landslide, volcano |
| `Outbreak { pathogen, species, count }` | infects up to `count` random susceptible hosts of a species with a scenario pathogen (§8) |
| `SetSeasonLength { days_per_season }` | season speed; the world calendar is fixed during a run, so it applies at the next reset (same policy as the parameter) |
| `Select(Option<OrganismRef>)` | the view then carries the organism's full inspection |
| `Follow(Option<OrganismRef>)` | the view carries its position and trail; its decisions are traced |

Every command validates its arguments and returns `EcosystemError` (mapped
to `SimError`) without side effects on error. `EcosystemSim::inspect`
returns an `OrganismInspection` for any animal or tree id at any time.

### 17.3 Metrics

Every tick: `animals`, `trees`, `population.<species>`, `births.<species>`,
`deaths.<species>`, `attacks`, `kills`, `mean_age_days`, `richness`,
`shannon`, `simpson` (Gini–Simpson, via `analytics::Diversity`),
`burned_area`, `burning_cells`, `rain`, `water_volume`, `soil_moisture`,
`mean_temperature`, `daylight`, `drought_index`. Ecological rules add:
biomass per trophic level (`biomass_producers`, `biomass_primary_consumers`,
`biomass_secondary_consumers`, `biomass_apex`, `biomass_decomposers`),
energy flows per tick (`energy.primary_production`, `energy.herbivory`,
`energy.predation`, `energy.scavenging`, `energy.animal_respiration`,
`energy.decomposer_respiration`, `energy.combustion`), nutrient pools in kg
(`nutrients.soil_mineral`, `soil_organic`, `plants`, `animals`, `carcasses`,
`detritus`, `fungi`), the two balance errors and `burned_biomass`; part 2 adds `fights`,
`alarms`, `hibernating`, `migrating`, `airborne`, `injured`,
`flowers.pollination`, `flowers.seed_production`, `pheromone`,
`colonies.<species>` (colony species; their `population.<species>` counts
workers and field species report individuals = density × area) and, per
pathogen, `infected.<id>`, `recovered.<id>`, `new_infections.<id>` and
`disease_deaths.<id>`. Diversity indices count individual animals;
`richness` also counts the aggregate species present. The NetLogo rule
set adds `green_patches`. Trophic levels come from roles
(apex; carnivore/omnivore/predator/scavenger; otherwise primary).

### 17.4 Views and inspection

`EcosystemViewRequest { world, region, animals, trees, remains, vegetation,
mushrooms, litter, block, field_markers, max_markers, colonies,
pheromone, pollination }` returns an owned `EcosystemView`:

* tick, calendar time, season, hour, rule set, cell size;
* `world`: the physics-world `WorldView` (requested abiotic layers, fire
  grid, weather, daylight, and the static terrain whenever the renderer's
  `known_terrain_version` is stale);
* `species`: per species id, name, model id, `visual_id` (the model id or
  `proc:<rig>`), representation, flight altitude, body plan, colour,
  length, class, locomotion modes, activity rhythm, social structure,
  population;
* `animals` in the region: id, species index, position, heading, velocity,
  action (animation clip), age fraction, size (length × visual scale ×
  `(m/M)^{1/3}`), health, energy fraction, sex, pregnant, juvenile, hidden,
  altitude, injury, infected, calling an alarm, group;
* density-field markers (§9), colonies (nest, workers, brood, store),
  forager squads (position, heading, altitude, loaded) and the pheromone
  and pollination grids as block means;
* `trees` in the region: id, species, style, position, height, crown radius,
  stage, health, burnt, foliage fraction, model id for the stage and variant;
* carcasses (remaining fraction) and clutches;
* requested vegetation layers, mushrooms and litter as block-mean grids;
* the selected organism's `OrganismInspection` and the followed organism's
  position and trail.

`AnimalInspection` contains the full state: species, sex, age, mass, energy,
nutrient store, drives, condition, temperature, kinematics, action/target
and duration, home, reproductive state, expressed genes, lineage and
children, the utility score of every action and a perception summary
(recomputed for the current state), life counters, the derived species
traits and its history (global events about it plus the follow log), and
its altitude, injury, infection, group and leader, and alarm.
`TreeInspection` gives species, style, stage, age, size, biomass, health,
light, fire scar and death.

## 18. Scenarios

Scenario files live in `scenarios/ecosystem/*.ron` and are embedded with
`include_str!` (`scenario::BUILTIN_FILES`). Loaders: `scenario::builtin()`,
`find_builtin(id)`, `load_file(path, catalog)`, `load_dir(dir, catalog)`
(sorted, duplicate ids rejected), `Scenario::parse(text, source)`. A
scenario is `(id, name, description, seed, config: EcosystemConfig)`; the
config holds the world (size, cell size, climate, relief, rivers, lakes,
lightning...), the terrain (`generated`, `flat(elevation_m)`,
`basin(elevation_m, depth_m, radius_cells)` — a plain with a round lake —
or `coast(elevation_m, sea_depth_m, sea_fraction)`), the rule set, initial
vegetation, trees, soil (litter, dead wood, mycelium, mycorrhiza and the
dissolved nutrients of water cells, `water_nutrients_g_m2`, default 2 g/m²),
animals (`(species, count, region?, groups?)`;
`count` is individuals, field individuals or colonies), parameter
overrides, pathogens and scheduled events, all validated with the file
name in the message.

**Scheduled events** are scenario inputs; only the intervention is
scripted, its consequences emerge. An event of day `d` runs at the start of
tick `round(d · ticks_per_day)`, in file order:

```ron
events: [
    (day: 8.0, action: world(drought(days: 14.0))),
    (day: 22.0, action: ignite(x: 12, y: 20)),
    (day: 3.0, action: introduce(species: "rat", count: 24, region: Some((2.0, 54.0, 10.0, 62.0)))),
    (day: 15.0, action: remove(species: "wolf", fraction: 1.0)),
    (day: 20.0, action: outbreak(pathogen: "rhd", species: "rabbit", count: 4)),
    (day: 10.0, action: set_param(key: "rainfall_mm", value: 160.0)),
]
```

`world(...)` takes drought, heatwave and storm spells in days, rain, flood,
earthquake, tornado, landslide and volcano. Introductions and removals are
booked as intervention inputs and outputs. Parameter changes made by events
use the parameter's apply policy and are **not** kept by `reset()` (which
rebuilds the scenario plus the user's own parameter changes).

| Id | Content | What to watch |
|---|---|---|
| `wolf_sheep_grass` | NetLogo rules with the slider defaults on a 51 × 51 flat torus (grass version) | the classic oscillation (§19.1) |
| `balanced_meadow` | 100 × 100 cells of 20 m (4 km²): grassland and oak, birch and pine woodland with a river; rabbits, deer, foxes, a wolf pack, ravens, warblers, 3 bee hives, 6 ant colonies, butterflies, beetles, earthworms; all layers, fungi and soil pools; deposition, leaching and volatilisation declared | populations, trophic biomass, energy flows, nutrient balance (§19.2) |
| `temperate_forest` | 80 × 80, five trophic levels: deer, wapiti, boar, rabbits, rats, beetles, worms, butterflies, bees; foxes, warblers, bats, frogs, lizards, snakes, spiders, wasps, ants; a wolf pack, a bear, golden eagles, ravens; oak, birch, pine, spruce; rabbit haemorrhagic disease on day 20 | `biomass_*`, `infected.rhd`, `fights`, `alarms`, `hibernating` |
| `african_savanna` | 96 × 96 hot savanna with acacia, palm and kapok: zebra herds, elephant families, boar, rats, a lion pride, white-backed vultures, eagles, warblers, lizards, snakes, insects, ants, bees | pack hunts, `energy.scavenging`, `flowers.pollination` |
| `freshwater_lake` | 80 × 80 plain with a round lake (radius 18 cells, 6 m deep): 400,000 sticklebacks (a field), bream, perch, pike, mallards, herons, frogs, pond turtles, grass snakes, rabbits, foxes, warblers, bats, insects | fish confined to the lake, herons fishing the shallows, frogs gathering and spawning in water |
| `ocean_ecosystem` | 96 × 96 warm coast, 70 % reef 25 m deep: 600 damselfish, 3 million sticklebacks standing for coastal forage fish (a field), 4 manta rays, 4 blacktip reef sharks, a pod of 6 dolphins, lizards on the beach (no fresh water: reptiles live on the water in their prey) | marine food web in salt water only |
| `tropical_rainforest` | 80 × 80 kapok and palm forest, 3,200 mm of rain: forest elephants, boar, rats, bats, warblers, an eagle pair, snakes, frogs, lizards, 12 ant colonies, 3 hives, butterflies, beetles, spiders, wasps, worms | fast decay, insect fields, pheromone trails |
| `isolated_island` | 80 × 80 warm oceanic island with a steep volcanic coast (no shallows for waders): lizards, warblers, 40 mallards, frogs, an eagle pair that hunts mallards and lizards, beetles, butterflies, spiders, one hive | small closed populations |
| `forest_fire` | 80 × 80 dry pine, birch and oak woodland; a 14-day drought from day 8, three ignitions on day 22 with a 7 m/s westerly wind, no lightning | `burned_area`, `burned_biomass`, recovery of the herb layer (§19.3) |
| `prolonged_drought` | 80 × 80 grazed grassland: sheep, wild horses, deer, rabbits, a wolf pack, foxes; on day 10 a 50-day drought and the annual rainfall from 750 to 160 mm | `drought_index`, `soil_moisture`, plants and herbivores against the control (§19.3) |
| `invasive_species` | 64 × 64 grassland with a pond and a native insectivore community (lizards, frogs, warblers); 24 brown rats arrive on day 3 | rats, natives against the control (§19.3) |
| `apex_predator_extinction` | 64 × 64 dry grassland and woodland: red deer, wapiti, feral sheep, a pack of 12 wolves, ravens; all wolves removed on day 15 | herbivores, kills, starvation and herbs against the control (§19.3) |
| `island_evolution` | 64 × 64 cool wet island with a closed population of 120 sand lizards (and their insect prey) at the cold edge of their niche; mutation rate 0.3 | genes (`thermal_shift_c`, size, speed...) over years in the inspector and genealogy |
| `extreme_seasons` | 96 × 96 continental landscape: ±25 °C seasonality around 4 °C and a 30 °C north–south gradient; hibernating bears, bats, frogs and snakes; migrating ducks, warblers and wapiti; resident deer, rabbits, wolves, foxes; beetles, worms, butterflies | `hibernating`, `migrating`, exposure deaths |

Every scenario runs without lightning only when the experiment needs it
(fire, drought, invasive, apex); elsewhere storms can start fires. Relief is
kept low (peaks of 150–270 m on 1.3–2 km maps) so that slopes stay walkable at
20 m cells.

## 19. Observed dynamics

### 19.1 Wolves, sheep and grass

`cargo run --release -p ecosystem --example ecosystem_report -- netlogo 8 3000`
(3,000 ticks, statistics after a 500-tick burn-in):

| Seed | Persisted | Sheep mean (sd) | Wolves mean (sd) | Green patches (mean) |
|---|---|---|---|---|
| 1 | yes | 160.7 (30.9) | 75.1 (15.8) | 973.7 |
| 2 | yes | 162.1 (20.4) | 76.6 (12.3) | 962.4 |
| 3 | yes | 159.2 (23.5) | 76.0 (14.0) | 975.1 |
| 4 | yes | 159.0 (23.6) | 75.8 (12.9) | 976.0 |
| 5 | yes | 159.5 (23.8) | 75.7 (13.6) | 974.4 |
| 6 | yes | 158.5 (25.9) | 75.9 (14.2) | 981.0 |
| 7 | yes | 158.3 (26.5) | 75.4 (15.3) | 981.3 |
| 8 | yes | 160.8 (25.2) | 74.9 (13.6) | 972.3 |

The no-grass version (run until no animal is left, or wolves are extinct
and sheep exceed 30,000) collapses in every seed:

| Seed | Wolves extinct at tick | Sheep extinct at tick | Run ends at tick |
|---|---|---|---|
| 1 | 270 | — | 417 (sheep > 30,000) |
| 2 | 231 | 146 | 231 |
| 3 | 219 | 175 | 219 |
| 4 | 245 | 148 | 245 |
| 5 | 219 | 142 | 219 |
| 6 | 236 | — | 381 (sheep > 30,000) |
| 7 | 205 | — | 374 (sheep > 30,000) |
| 8 | 229 | 143 | 229 |

The research note's independent re-implementation found sheep 156–162 (sd
22–31), wolves 75–77 (sd 13–15) and 969–987 green patches in 8/8 persisting
runs, and 10/10 no-grass collapses within 194–605 ticks (in 6 of them the
sheep overran the map, in 4 they died first): our figures agree.
The test `grass_version_persists_with_fluctuating_populations` requires at
least 3 of seeds 1–4 to keep both species for 1,500 ticks with fluctuating
sheep numbers; `no_grass_version_collapses` requires the collapse.

### 19.2 Balanced meadow

`cargo run --release -p ecosystem --example ecosystem_report -- meadow 240 7`
(two simulated years, seed 7; the report prints every 10 days, shown here every 30):

| Day | Season | Rabbits | Deer | Foxes | Wolves | Ravens | Warblers | Bees (workers) | Ants (workers) | Beetles | Trees | Producers (t) | Herbs (t) | Soil moisture | Nutrient error | Energy error |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | spring | 150 | 40 | 8 | 6 | 6 | 30 | 30,000 | 12,000 | 50.0 M | 4,330 | 6,152 | 2,134 | 0.39 | 0 | 0 |
| 30 | summer | 317 | 37 | 8 | 6 | 19 | 58 | 40,274 | 9,312 | 49.9 M | 5,628 | 6,218 | 2,204 | 0.31 | 3·10⁻⁶ | 2·10⁻⁷ |
| 60 | autumn | 867 | 35 | 7 | 5 | 18 | 52 | 77,210 | 18,052 | 72.6 M | 6,093 | 6,226 | 2,236 | 0.43 | 1·10⁻⁵ | 1·10⁻⁷ |
| 90 | winter | 801 | 34 | 7 | 5 | 18 | 49 | 61,271 | 23,544 | 70.8 M | 7,258 | 6,119 | 2,221 | 0.43 | 2·10⁻⁵ | 1·10⁻⁶ |
| 120 | spring | 689 | 30 | 30 | 11 | 18 | 47 | 7,186 | 16,656 | 56.3 M | 7,247 | 5,751 | 1,870 | 0.40 | 3·10⁻⁵ | 1·10⁻⁶ |
| 150 | summer | 1,927 | 41 | 30 | 7 | 25 | 51 | 51,949 | 13,158 | 57.3 M | 8,456 | 6,042 | 2,082 | 0.38 | 3·10⁻⁵ | 6·10⁻⁷ |
| 180 | autumn | 4,238 | 35 | 29 | 7 | 26 | 49 | 77,184 | 20,304 | 111.6 M | 8,645 | 5,573 | 1,648 | 0.12 | 4·10⁻⁵ | 7·10⁻⁷ |
| 210 | winter | 3,281 | 31 | 25 | 7 | 26 | 44 | 69,767 | 25,052 | 109.1 M | 8,709 | 4,770 | 978 | 0.14 | 5·10⁻⁵ | 1·10⁻⁶ |
| 240 | spring | 2,787 | 26 | 59 | 7 | 23 | 38 | 7,699 | 18,141 | 81.3 M | 8,448 | 4,576 | 820 | 0.29 | 6·10⁻⁵ | 1·10⁻⁶ |

Over the two years: births — rabbits 5,989, foxes 69, warblers 36, ravens
23, deer 16, wolves 6; deaths by cause — predation 2,300, dehydration 893
(all rabbits), old age 188, starvation 40, injury 18; 4,481 trees
germinated and 363 died. Butterflies range from 1.1 to 8.1 million and
earthworms from 59 to 68 million.

What happens:

* **Rabbits** breed in spring and summer and decline over winter; with a
  dry second summer and autumn (land soil moisture 0.08–0.16) they reach
  ≈ 4,200 and then lose condition: 893 die of thirst and 36 of hunger,
  mostly rabbits far from water on dry forage, before numbers fall back.
* **Foxes** give birth in late winter (7 → 30 in the first year, → 59 in
  the second) and are the main rabbit predators.
* **Wolves**: the pack of six (founded by a breeding pair) loses one member
  in the first summer; only the dominant pair breeds, so the first winter
  brings one litter of six cubs (11 wolves), of which seven remain,
  travelling and resting together.
* **Deer** decline slowly (one calf per hind, wolf predation): 40 → 26.
* **Warblers** nest in spring; ravens, foxes and hungry searching
  predators raid most clutches. Since a partly eaten clutch loses its eggs
  (rounded down: before, rounding to the nearest kept eggs alive with less
  energy than they were laid with and hatchlings appeared with energy
  that came from nowhere), few clutches hatch: 36 hatchlings in two years,
  and the warblers decline from 58 to 38.
* **Bees** rise from 30,000 to ≈ 77,000 workers in summer (laying while
  nectar flows and the store keeps two days of metabolism) and fall to
  ≈ 8,000 by the end of winter, when workers die and nothing is laid; **ant**
  workers follow the same cycle with a lag. **Beetles** follow the litter
  (leaf fall in autumn, dieback in the dry season) and earthworms are
  nearly constant.
* **Trees** colonise open grassland by seed dispersal (4,330 → 8,700); the
  herb layer (grass, shrubs, flowers) falls from 2,134 to 820 t over the
  dry second year under heavy grazing.
* Both balance errors stay small: the nutrient error grows by ≈ 3·10⁻⁷
  per simulated day (the world's `f32` soil grids, §14) and the energy
  error stays below 2·10⁻⁶.

### 19.3 Experiments

Each experiment is compared with the same scenario without its scheduled
events (same seed; `ecosystem_report scenario <id> <days> <every> control`).
The tests in `tests/scenarios.rs` assert the qualitative outcome
(`cargo test --profile verify -p ecosystem --test scenarios -- --ignored`).

**Forest fire** (`forest_fire`). The 14-day drought dries the fuel; the
three ignitions on day 22 spread with the westerly wind through the pine,
birch and oak woodland and its herb layer:

| | Herbs and shrubs (kg) |
|---|---|
| before the ignitions (day 22) | 1,371,570 |
| minimum, 20 days after ignition | 35,050 |
| 70 days after ignition | 183,515 |

4,334 of 6,400 cells burned and 7.3·10¹⁰ kJ of fuel were combusted
(booked as `Combustion`, the nutrients split into volatilisation and ash).
Recovery comes from the seed banks, resprouting shrubs and the ash
nutrients; it is slower than the destruction because germination needs
the seed bank and moist soil. Animals flee the fire (weight ≥ 3), fliers
escape over it, rabbits dive into their burrows, and clutches and field
organisms in burning cells die. The crown fire is several cells deep:
12 of 30 deer survive it by dashing through to burned ground (before the
fire-escape rule 1 of 30 did), while the rabbits, safe in their burrows
from the fire itself, starve afterwards on the burned ground (no rabbit is
left 30 days after the fire).

**Prolonged drought** (`prolonged_drought`, day 100, drought against
control):

| | Drought | Control |
|---|---|---|
| producers (kg) | 1,946,724 | 2,567,743 |
| soil moisture | 0.100 | 0.253 |
| sheep, horses, deer, rabbits | 43, 18, 18, 512 | 54, 20, 18, 649 |
| herbivores (number; body mass) | 591; 9,943 kg | 741; 11,006 kg |
| herbivores starved | 125 | 0 |

The drought acts through the plants: soil moisture collapses, growth stops
and drought dieback turns forage into litter (herbs 355 t against 932 t on
day 100); the forage also carries less water. Herbivores starve (125
against none) and are 20 % fewer by day 100, rabbits first; the effect
grows with time (by day 120: 180 starved, sheep 62 against 87). The large
grazers suffer less than in part 2's first version, where 17 of 20 horses
died by day 120: grazers then could only eat in the cell they stood in and
starved next to forage at the edge of a grazed-out patch (§4.4); with
forage within reach they live on the standing biomass, which the drought
halves but does not exhaust within its 50 days. The test therefore runs to
day 100 and requires fewer producers (< 0.9 ×), more starvation and fewer
herbivores (< 0.9 ×).

**Apex predator removal** (`apex_predator_extinction`, red deer, wapiti and
feral sheep with and without the wolf pack removed on day 15):

| Day | Herbivores, no wolves | Herbivores, wolves | Herbs, no wolves (kg) | Herbs, wolves (kg) |
|---|---|---|---|---|
| 30 | 201 | 198 | 829,502 | 829,534 |
| 60 | 198 | 188 | 486,099 | 486,335 |
| 90 | 171 | 178 | 462,913 | 463,111 |
| 120 | 221 | 219 | 374,892 | 375,372 |

The pack of 12 (16 by day 120) kills 44 herbivores in 120 days, which is
what twelve wolves need (≈ 10 MJ a day each, ≈ 2 t of prey): about a
tenth of the herbivore biomass. Without wolves the herbivores are up to
6 % more numerous in the first three months (198 against 188 on day 60)
and graze 1.4 % more (6.99·10⁸ against 6.90·10⁸ kJ), and the herb layer is
lower by 0.1 %; from day 90 on, births in the two runs differ by chance
more than predation separates them. The cascade is weak because it is
bounded by the predators' energy budget and grass regrowth compensates
extra grazing (logistic growth is fastest at intermediate biomass). Part
2's first version showed a much larger effect (241 against 112 herbivores
on day 120): its wolves fixated on carcasses they could not reach and
killed again, and its grazers starved at the edges of grazed-out patches
(both fixed, §4.2, §4.4).

**Invasive species** (`invasive_species`, 180 days, with and without the 24
rats introduced on day 3):

| | With rats | Without |
|---|---|---|
| rats | 5,000 (the per-species cap) | — |
| sand lizards | 256 | 312 |
| frogs | 123 | 195 |
| warblers | 251 | 304 |

The rats multiply from 24 to the population cap of 5,000 (≈ 30 per ha)
within six months. They eat the ground beetles that are the staple of the
lizards, frogs and warblers, and also lizards, frogs and eggs: lizards are
18 %, frogs 37 % and warblers 17 % fewer than without rats. Frogs now
breed (their clutches can be paid for and they gather at the pond to
mate, §4.6), so both runs grow, and the rats' predation on frogs and their
spawn shows. The population cap (`max_population_per_species`, births
above it are suppressed and counted) bounds the invasion; a scenario that
wants food limitation alone raises it.

**Balanced meadow for a year**: rabbits, deer, foxes, wolves, ravens and
warblers are all alive after 120 days (the test requires at least three
vertebrate species).

**Every scenario for a month** (`no_scenario_loses_a_founding_species_within_ten_days`):
no founding species dies out within ten days and the invariants hold for
30 days in all 13 ecological scenarios. Before the review fixes of §19.4
the ocean lost its manta rays, dolphins and every fish within 30 days, the
lake and the island their herons within ten, `extreme_seasons` its bats
and frogs within five (exposure), `island_evolution` 90 % of its lizards
to thirst, and the savanna and rainforest their snakes.

### 19.4 Adversarial review of part 2

A review ran every scenario for 40 days with deaths tallied by species
and cause (`ecosystem_report scenario <id> <days> <every>` now prints
them) and fixed what it found:

| Finding | Evidence | Fix |
|---|---|---|
| Aquatic primary production was zero | the world gives water no fertility: algae shrank 10 %/day in every lake and sea | dissolved nutrients for water cells (`soil.water_nutrients_g_m2`), bacterial decay of aquatic detritus, no humus at sea (§11, §13) |
| Algae looked like scarce food | the 0.15 kg/m² reference exceeds what algae ever hold: mantas and reef fish rarely foraged and starved | aquatic layers use `min(0.15, ½ K_max)` (§4.2) |
| Herons and dolphins could not live on fish | one individual stickleback per attack: 6 herons emptied a 200-fish lake in 10 days and starved; dolphins starved in 10 | sticklebacks became an aquatic density field gleaned by piscivores, waders only in the shallows (§9) |
| Foragers starved next to food | the claim used the cell the animal stood in; pods and herds hovering at the edge of a grazed-out cell ate nothing | a forager eats in the cell it heads for within 1 cell (§4.4) |
| Hungry predators never searched | a hungry wolf pack rested for ten days with no prey in sight, never hunting | hunger-driven searching (§4.3) |
| Wolves starved beside an unreachable carcass | a pack fixated on a dead pack mate across a ridge for ten days | carcasses need a passable straight path (§4.2) |
| Frogs, reef fish and lizards could never breed | a mean clutch cost 2.4–5 × what the mother could gather | catalogue test and corrected birth masses and egg formation (§4.6) |
| Clutch rounding created energy and nutrients | a partly eaten clutch rounded its egg count to the nearest | round down (unit test) |
| Hibernators died in a cold spring; dormant bats died of thirst | dormancy only in autumn/winter, torpor pre-empted it, no arousal | dormancy outside summer, brumation, arousal (§7) |
| Lizards died of thirst on islands | prey water covered 60 % of a reptile's need | species water economy `food_water` (§4.5) |
| Snakes starved | ambush with 15 m vision and 1 kJ/tick intake: they killed frogs and abandoned them | smell, stalking, swallowing whole, 60-day reserve (catalogue) |
| Every species paid a runner's cost of transport | fish and birds paid 3–8 × too much per metre | mode factors (§5) |
| Deer burned against the map edge | 29 of 30 deer died in the forest fire | fire escape to burned ground, burrows (§4.3) |
| Rounding made reserves negative | `body out of range` in the drought and ocean runs | growth step clamped (physiology) |
| Per-animal scans of every clutch | `O(animals × clutches)` per tick in perception and population caps | clutch spatial index, census once per phase |


## 20. Determinism and snapshots

* All randomness comes from `SimRng` streams derived from
  `(seed, subsystem, tick, entity id)` (`rng::stream`: decisions, attacks
  and fights, life events, trees, disturbances, NetLogo, colonies, disease),
  so draws do not depend on iteration order, thread scheduling or how many
  other entities drew; a stateful `SimRng` in the state serves setup,
  commands and scheduled events.
* Parallel phases (perception and decisions; movement; disease draws) are
  pure functions of read-only state writing into per-animal slots;
  everything contested is a claim resolved in a total order; pack leaders,
  fields and colonies are updated serially in a fixed order; births,
  deaths and ids are applied at commit in id order.
* Same seed and inputs → bit-identical snapshots (tested, including the
  scenarios with fields, colonies, pathogens and schedules, through the byte
  encoding and the object-safe `DynSimulation`).
* `snapshot()` serialises the whole `EcoState` with postcard (configuration
  with its pathogens and schedule, the reset point, catalogue, parameters,
  queued changes, seed, tick, RNG, world, animals, trees, remains, fields,
  colonies with their squads and pheromone, vegetation, detritus, fungi,
  canopy, caches, ledgers, genealogy, event logs, selection, counters,
  disease counters, NetLogo grass) as model `ecosystem`, **`schema_version`
  3** (2 added part 2's life styles, fields, colonies, diseases, schedules
  and social state; 3 the species' water economy, smell and aquatic
  detritus decay and the dissolved water nutrients of the soil setup).
  Older snapshots predate this layout; `restore()` rejects them with a
  message that says so (no migration; tested for every older version). `restore()` also rejects
  other models, newer versions and inconsistent states (grid sizes, ids,
  species indices, genome lengths, ledger shapes, field and colony data,
  disease counters, the configuration...) with `SimError::Serialization`,
  leaving the model unchanged (tested). Scheduled world events are stored as
  `WorldEvent` (externally tagged) because `physics_world::Disaster` is
  internally tagged and cannot be decoded by binary formats.
* Floating point is IEEE `f32`/`f64` with transcendental functions from the
  platform: replay is bit-exact on the same platform and build.

## 21. Performance

Criterion benchmark `benches/ecosystem.rs` (`cargo bench -p ecosystem`;
each iteration restores the same snapshot first, untimed, so every sample
measures the same work):

| Benchmark | What one iteration does | Time [95 % CI] |
|---|---|---|
| `meadow_tick` | one ordinary tick of `balanced_meadow` one day in (≈ 240 individual animals of six species, 3 hives and 6 ant nests with 42,000 workers, ≈ 1.1·10⁸ field organisms, ≈ 4,400 trees): world step, colonies and animals | 722 µs [711, 738] |
| `meadow/meadow_day` | 240 ticks (one simulated day) from the same state, with the 24 hourly vegetation / fungi / field phases and the daily tree update | 292.0 ms [291.1, 293.1] → **1.22 ms per tick** |
| `wolf_sheep_grass_tick` | one tick of the NetLogo rules after 200 ticks (51 × 51, 200–250 animals, world step included) | 423 µs [421, 426] |
| `agents/1000` | one ordinary tick of a 160 × 160 grassland (3.2 km side, rivers, lakes, oak and birch) with 1,000 individual animals (60 % rabbits, 15 % deer, 15 % sheep, 5 % foxes, 5 % wolves) after 50 ticks | 1.52 ms [1.51, 1.53] |
| `agents/10000` | the same with 10,000 animals | 14.3 ms [14.1, 14.4] |
| `fields/100k` | one simulated hour (10 ticks, one field update) of a 100 × 100 grassland with 10⁵ beetles, earthworms, spiders and butterflies (density fields) | 9.07 ms [9.02, 9.12] |
| `fields/10M` | the same with 10⁷ field organisms | 9.14 ms [9.08, 9.19] |

Measured on 2026-11-10, after the review fixes of §19.4, with `cargo bench -p ecosystem -- --warm-up-time 2 --measurement-time 6` (release profile, Criterion 0.8, 30 samples; 10 for the day, agent and field groups) on an Apple M2 Max (12 cores, 64 GB, macOS) while other agents' builds shared the machine (load average 8.9–12.6 during the run): an idle machine will be somewhat faster. The part-2 run before the review (load 12–26) gave 711 µs, 291 ms, 423 µs, 1.55 ms, 13.6 ms, 9.16 ms and 9.19 ms: the fixes (clutch index, census, fire refuges, smell, carcass paths) cost nothing measurable, and the figures are stable to about 5–10 %.

What the numbers say:

* **Individual animals scale linearly**: 10 × the animals cost
  9.4 × the time (≈ 1.4 µs per animal and tick, including
  perception through the spatial grid, utility decisions, steering,
  claims and physiology; perception and decisions run on all cores). At
  10,000 animals a simulated day takes ≈ 3.4 s.
* **The aggregate path is flat**: 10⁷ field organisms cost the same as 10⁵
  (9.14 against 9.07 ms), because a density field is updated per cell, not per
  organism; the hour consists of 10 world steps and one field update over
  10⁴ cells. 100,000 tiny
  organisms — or a hundred million, as in the meadow — therefore cost about
  as much as one field update of the map, far less than 100,000 agents
  would (≈ 0.14 s per tick extrapolated linearly).
* Colonies cost `O(squads)`: a hive or nest of 10,000 workers moves at most
  `max_foragers` squads.
* Of the meadow's 1.22 ms per tick, 0.72 ms is the ordinary tick (world
  step, colonies and ≈ 240 animals); the rest is the 24 hourly phases
  (vegetation, fungi and fields over 10⁴ cells × 7 layers, ≈ 5 ms each) and
  the daily tree update. A 120-day year takes ≈ 35 s.

## 22. Testing

`cargo test -p ecosystem` runs 129 tests (6 long ones are
`#[ignore]`d and run with `--profile verify -- --ignored`; all pass):

* **species** — the 41 built-in records are valid and cover every group;
  field errors name species and field; roles against diet and locomotion;
  representations restrict diets and prey (fields eat no carrion, colonies
  are not prey); hunters of individual prey need a hunting style,
  insectivores do not; hibernation and visual sections are validated;
  cross references; RON positions; Kleiber ratio; Gompertz survival;
  compressed durations; dispersal kernels; disk catalogue = embedded
  catalogue; model ids exist in the manifest; rig ids exist in
  `organism-render`.
* **conservation** — closed nutrient budget constant to 10⁻⁵ (dry and wet);
  open budget with deposition, fire, volatilisation, ash, spawns and
  removals closes both ledgers; leaching losses booked; growth and
  decomposition conserve nutrients; carcass decay; with five density fields
  and lizards eating them both ledgers still close (`aggregate`).
* **behaviour** — grazing, predation energy transfer, fleeing against a
  control, mating and inheritance, hydration of weaned young, starvation and
  old age, fire and regrowth, deer escaping a wind-driven fire front to
  burned ground (7 of 10 survive a fire over 99 % of the plain), floods,
  downhill water search, light competition, carcass dominance; nest
  predation (a rat raids a lizard clutch); a songbird that loses its
  clutch lays again, a lizard does not; injuries slow animals and heal;
  every species can pay for its mean litter (catalogue); partly eaten
  clutches never enrich the remaining eggs (unit).
* **locomotion** — eagles climb to their cruising altitude while wandering,
  cross the lake airborne and never drown; perch and bream stay in water of
  their depth below the surface, a perch cannot be put into the sea, reef
  fish stay at sea; frogs move between land and water; ground hunters
  cannot reach airborne or deep prey, waders reach aquatic field prey only
  in the shallows (unit tests); the cost of transport depends on the mode
  (unit test).
* **aquatic** — the stickleback field lives only in water and keeps feeding
  on algae; an euryhaline field lives at sea, a freshwater one has no
  habitat there (clear error); a hungry heron fishes the shallows of a lake
  (predation booked); dissolved nutrients keep a lake's algae growing
  (0.6+ of the start after 30 days against a decline without them);
  reptiles keep hydrated on insects without water (mean hydration 0.90
  against 0.67 with the ordinary water economy).
* **social** — herd founders start together, solitary ones apart;
  territorial ravens stay near their founding place (5.3 cells after two
  days against 34.6 without territoriality); schools align
  (nearest-neighbour heading cosine 0.29 against 0.01); pack members join
  their leader's hunt; a territory owner fights and drives off an intruder
  (8.1 cells from home against 1.5 without territoriality); an alarm call
  makes a rabbit that neither sees nor hears the fox flee; a calf closes in
  on its mother; packs are founded by a breeding pair and only the dominant
  female conceives until she is gone.
* **seasonal** — migrants fly towards their preferred climate (row 8 → 54
  in a day; residents stay); a bear hibernates in winter, hidden, using
  0.12 field-metabolism days per day against 0.47 for a bear that cannot;
  it wakes when the daily mean rises, and rouses to drink when its water
  runs low; torpid frogs brumate in a cold spring instead of staying
  exposed.
* **disease** — an epidemic rises, peaks (38 of 60) and burns out with
  lifelong immunity (`total = recovered + dead + infected`); weak
  transmission fizzles; virulent strains kill; outbreaks and pathogens are
  validated.
* **aggregate** — fields close both budgets; beetles multiply on litter and
  starve without food; butterflies disperse into empty habitat and are drawn
  as flying markers; lizards eat field prey; ant foragers mark the trail
  between nest and carcass (4.7 against 0.13 off the trail); pheromone halves
  in one half-life; bee hives pollinate (index 0.06) and raise flower seed
  production (+11 %); seed production rises with pollination within its
  bound.
* **NetLogo** — setup, regrowth, energy and eating rules, persistence with
  grass and collapse without.
* **API** — scenarios on disk = embedded; every metric declared; reported
  parameters validate; apply policies; a new seed applies at reset only;
  scheduled parameter changes do not survive a reset, user changes do;
  command validation; selection, following, regions, block grids and
  inspection; views carry altitudes, field markers, colonies, squads,
  pheromone and pollination grids and visual ids.
* **determinism** — same seed → identical snapshots; snapshot round trips
  through bytes and `DynSimulation`; NetLogo, meadow, temperate-forest and
  forest-fire scenarios replay exactly; foreign, newer, older and corrupt
  snapshots rejected.
* **scenarios** — the 14 scenarios exist; each runs a simulated day without
  violating invariants (finite metrics, balance errors below 10⁻⁴/10⁻³,
  animals inside the map with valid condition, non-negative colony pools, no
  internal errors) and keeps every configured species; scheduled events run
  on their tick; and, ignored (long): the forest fire burns then the herb
  layer regrows; the drought reduces plants and herbivores; removing the
  apex predator increases herbivores and lowers the herb layer; rats change
  the native populations; the balanced meadow keeps at least three
  vertebrate species for a year; no scenario loses a founding species (four
  or more individuals, or a field or colony, not removed on purpose)
  within ten days, and every scenario keeps its invariants (now including
  non-negative nutrient stores) for 30 days.

## 23. Approximations and limitations

* **Compressed calendar.** Life-history durations shrink by
  `days_per_year/365` (≈ 1/3) while daily energetics do not; reproduction
  per simulated day is ≈ 3× real. Fecund prey (rabbits) therefore boom more
  than in nature. `days_per_season = 91` removes the compression at three
  times the run length. Colony laying and worker lifespans and field birth
  and death rates follow the same convention.
* **Movement time** (`motion_seconds`) represents a few seconds of
  locomotion per 6-minute tick for individual animals; forager squads move
  for the whole tick (× `travel_fraction`).
* **Aggregates.** Insects, spiders and worms are density fields: no
  individual behaviour (no flight paths, no mating, no larval stages — the
  densities are stage-averaged); their display markers are not simulated.
  Colonies are super-agents: one squad stands for 50–500 workers; queens,
  males and nuptial flights are summarised by laying and swarming; colonies
  are not prey (no bears raiding hives); honeydew is represented by nectar.
* **Disease**: one infection record per animal (infection by another
  pathogen replaces an old immunity); no latent (SEIR) stage, no vertical
  or environmental transmission, no age-dependent susceptibility.
* **Social behaviour**: pack hierarchy is a rank (adults, then mass ×
  health), not a learned dominance network, and reproductive suppression
  is absolute (only the dominant pair breeds); dispersal of young adults
  from their pack and pack fusion are not modelled, so packs only grow;
  territory defence is between conspecifics only; scenarios give packs and prides one group each because
  several packs on a few km² fight constantly (realistic, but not what the
  scenarios study).
* **Migration** follows temperature only (not rainfall or food), within the
  map; animals do not leave the world.
* **Hibernation** dens are not physical places (dormancy happens where the
  animal is, hidden); shelter (burrow, den, torpor in the litter) is a
  temperature rule — the daily mean, at least 0 °C — not a microclimate
  model; ectotherms below their tolerance are torpid or brumate (§4.3).
* **Water**: animals remember water within their home range
  (`max(3 vision radii, 3 h of cruising)`, straight line, ignoring
  obstacles) and otherwise search downhill; steep generated terrain can
  make remembered water unreachable — scenarios keep relief low. Water
  temperature is the air temperature, at least 4 °C. Water cells start with
  a uniform dissolved nutrient stock; there is no stratification, inflow or
  upwelling. The reptiles' water economy is one multiplier (`food_water`),
  not a model of evaporative and urinary losses.
* **Aquatic forage fish** are a density field (sticklebacks standing for
  forage fish at sea too): predators glean them like insects, so there is
  no individual chase of small fish, no school shape and no spawning
  migration; larger fish (perch, bream, pike, reef fish) stay individuals.
* **Cost of transport** by mode uses two fractions of the running COT
  (flight 0.35, swimming 0.12) instead of mode-specific allometries.
* **Fire escape**: animals look for safe ground only within 12 cells and
  dash through the front when it is within 3 cells; a front deeper than
  their sprint can still kill them (a wind-driven crown fire in the
  forest-fire scenario kills about 60 % of the deer).
* **Pollination** is an index (fraction of flowers visited recently) that
  raises the flower seed set; herb seeds are a presence index, not counts;
  pollination does not affect trees.
* **Trophic cascade strength**: in grass-dominated scenarios grass regrowth
  compensates most extra grazing, so removing wolves lowers the herb layer
  by only ≈ 2 % within 120 days although twice as many herbivores survive
  and they graze 29 % more (§19.3).
* **Replacement clutches** are laid by birds only; birds and reptiles do
  not raise second broods after a successful one (one breeding per
  `interval_days`).
* **One nutrient** stands for N and P together; litter is poor in it, so
  detritivore growth is nutrient-limited.
* **Carcasses** are eaten and decay but do not attract scavengers from
  beyond their perception (vultures see 2.5 km).
* **Fire** behaviour (spread, intensity) belongs to physics-world; the
  ecosystem maps the consumed fuel back onto its pools proportionally.
* **Balance precision**: the soil nutrient grids of physics-world are `f32`;
  their rounding makes the nutrient balance error grow by ≈ 3·10⁻⁷ per
  simulated day in a 4 km² meadow (§14). The ecosystem's own pools close to
  ≈ 10⁻⁸.
* **Floating point** results are reproducible on the same platform and
  build only.

## 24. Extending the model

* **Species** are data: add a RON file in `assets/species/animals/` (and its
  `include_str!` entry in `species::catalog::BUILTIN_ANIMALS` to embed it).
  Behaviour differences come from weights, curves, rhythm, hunting style,
  refuges, social structure (cohesion, alignment, packs, territories,
  alarm calls), seasonal behaviour and combat values; insects in large
  numbers become a `field(...)`, eusocial insects a `colony(...)`; new
  diets, locomotion modes and refuges plug into the `Food`, `Locomotion` and
  `Refuge` enums.
* **Actions**: `animal::decide::ScoredAction` lists the scored candidates;
  new behaviours add a sense in `perceive`/`social`/`seasonal`, a score and
  a steering branch in `decide`, and claims in `interact`.
* **Scenarios**: add a RON file in `scenarios/ecosystem/` and an entry in
  `scenario::BUILTIN_FILES`; pathogens and scheduled events are part of the
  file.
* **Fluxes**: a new transfer adds a `Flux` variant with its role in both
  ledgers; the conservation tests then check it automatically.

## 25. References

* Wilensky, U. (1997). *NetLogo Wolf Sheep Predation model*. CCL,
  Northwestern University (rules re-implemented, no code copied).
* Kleiber, M. (1932). Body size and metabolism. *Hilgardia* 6:315.
* Brown, J. H., Gillooly, J. F., Allen, A. P., Savage, V. M. & West, G. B.
  (2004). Toward a metabolic theory of ecology. *Ecology* 85:1771.
* Hirt, M. R. et al. (2017). A general scaling law reveals why the largest
  animals are not the fastest. *Nature Ecology & Evolution* 1:1116.
* Taylor, C. R., Heglund, N. C. & Maloiy, G. M. O. (1982). Energetics and
  mechanics of terrestrial locomotion. *J. Exp. Biol.* 97:1.
* Lindstedt, S. L. & Boyce, M. S. (1985). Seasonality, fasting endurance and
  body size in mammals. *Am. Nat.* 125:873.
* West, G. B., Brown, J. H. & Enquist, B. J. (2001). A general model for
  ontogenetic growth. *Nature* 413:628.
* Lindeman, R. L. (1942). The trophic-dynamic aspect of ecology. *Ecology*
  23:399.
* Monsi, M. & Saeki, T. (1953). Light factor in plant communities.
* Yan, W. & Hunt, L. A. (1999). An equation for modelling the temperature
  response of plants. *Ann. Bot.* 84:607.
* Clark, J. S. et al. (1999). Seed dispersal near and far. *Ecology* 80:1475.
* Olson, J. S. (1963). Energy storage and the balance of producers and
  decomposers. *Ecology* 44:322.
* Linn, D. M. & Doran, J. W. (1984). *SSSAJ* 48:1267.
* Reynolds, C. W. (1987). Flocks, herds and schools: a distributed
  behavioral model. *SIGGRAPH* 21:25; (1999) Steering behaviors for
  autonomous characters. GDC 1999.
* Holling, C. S. (1959). Some characteristics of simple types of predation
  and parasitism. *Can. Entomol.* 91:385.
* Kermack, W. O. & McKendrick, A. G. (1927). A contribution to the
  mathematical theory of epidemics. *Proc. R. Soc. A* 115:700.
* Deneubourg, J.-L., Aron, S., Goss, S. & Pasteels, J. M. (1990). The
  self-organizing exploratory pattern of the Argentine ant. *J. Insect
  Behav.* 3:159.
* Seeley, T. D. (1995). *The Wisdom of the Hive*. Harvard University Press.
* Ripple, W. J. & Beschta, R. L. (2012). Trophic cascades in Yellowstone:
  the first 15 years after wolf reintroduction. *Biol. Conserv.* 145:205.
* Towns, D. R., Atkinson, I. A. E. & Daugherty, C. H. (2006). Have the
  harmful effects of introduced rats on islands been exaggerated? *Biol.
  Invasions* 8:863.
* Mark, D. (2009). *Behavioral Mathematics for Game AI*.
* Hamilton, W. D. (1971). Geometry for the selfish herd. *J. Theor. Biol.*
  31:295.
* Damuth, J.; Jetz, W. et al. (2004) and PanTHERIA (Jones et al. 2009) via
  `docs/research/ecology-alife.md` §3.3 and §10; AnAge (de Magalhães et al.)
  via the same note.
