# ecosystem — summary

`crates/ecosystem` is the biotic layer of the Ecosystem Simulator: animals,
tiny organisms, social insects, vegetation, trees, fungi, disease and the
nutrient and energy cycle on top of `physics-world` (terrain, climate,
water, fire, disasters) and `organism-engine` (agents, steering, utility
AI, claims, genomes). The full documentation — architecture, equations,
species catalogue, parameters, scenarios, measured dynamics and
performance, approximations — is in [`docs/ECOSYSTEM.md`](../ECOSYSTEM.md).

## At a glance

| | |
|---|---|
| Model | `EcosystemSim` (`simulation_core::Simulation`), id `ecosystem`, family `Hybrid` |
| Tick | 6 simulated minutes (`1440 / ticks_per_day`); 120-day compressed year |
| Rule sets | `ecological` (the full model) and `netlogo` (Wolf Sheep Predation rules) |
| Species data | 41 animal species, 7 trees, 7 vegetation layers and the fungi as RON files in `assets/species/`, embedded copies, validation with field-level messages; every species maps to a render visual (`model` id or `proc:<rig>`) |
| Representations | individual animals (33 species); density fields for insects, spiders, worms and forage fish (6; sticklebacks are an aquatic field); colony super-agents for ants and bees (2) |
| Scenarios | 14 in `scenarios/ecosystem/*.ron`, embedded, with pathogens and scheduled events: `wolf_sheep_grass`, `balanced_meadow`, `temperate_forest`, `african_savanna`, `freshwater_lake`, `ocean_ecosystem`, `tropical_rainforest`, `isolated_island`, `forest_fire`, `prolonged_drought`, `invasive_species`, `apex_predator_extinction`, `island_evolution`, `extreme_seasons` |
| Snapshot | postcard, `schema_version` 3 (older versions rejected with a clear message), includes the RNG, fields, colonies, pathogens, the schedule and every cache |
| Dependencies | simulation-core, physics-world, organism-engine, analytics, serde, ron, rayon, thiserror (no new third-party crates) |

## What is simulated

* **Animals** with individual state (mass, energy reserve, nutrient store,
  hydration, fatigue, stamina, health, injury, infection, thermal comfort,
  reproductive state, genome, lineage, group, altitude, action, target,
  history). Kleiber/MTE metabolism, Hirt sprint speeds, Gompertz ageing,
  West ontogenetic growth, energy-gated breeding, gestation or eggs and
  incubation, lactation, six heritable genes.
* **Locomotion and habitat**: walkers wade, swimmers cross water, fish stay
  in water of their kind (fresh or salt) and depth, amphibians move between
  both and spawn in water, fliers cruise at their species' altitude across
  water and land to feed (ground hunters cannot catch airborne prey),
  burrowers hide underground; flying and swimming cost a third and an
  eighth of running per metre.
* **Species-specific utility AI**: forage, hunt (stalk, ambush or coursing),
  scavenge, raid nests, flee (from fire: into burrows or through the front
  to burned ground), drink (water remembered over the home range, or search
  downhill), rest, sleep by activity rhythm, seek mate (water breeders
  gather at water), follow mother, incubate, defend territory, migrate,
  hibernate, wander — and search when hungry with nothing to eat in sight.
* **Social behaviour**: herds, flocks and schools with boids cohesion,
  alignment and separation; packs and prides with an alpha leader whose
  hunt target the pack shares; territorial fights between owners and
  intruders; alarm calls relayed through groups; parental care.
* **Seasonal behaviour**: migration towards the preferred climate,
  hibernation with reduced metabolism (also brumation of torpid
  ectotherms in a cold spring, periodic arousal to drink).
* **Disease and injuries**: SIR pathogens with per-contact transmission,
  recovery, immunity, mortality and fever; injuries from attacks and fights
  that slow animals down and heal.
* **Tiny organisms** as density fields (Holling II feeding, nutrient-limited
  births, density-dependent mortality, dispersal; cost `O(cells)` whatever
  their number) with representative markers for rendering; aquatic fields
  (sticklebacks) live in water of their kind and feed herons (in the
  shallows), perch, pike, sharks and dolphins.
* **Ant and bee colonies** as super-agents: workers, brood, stores,
  swarming; forager squads with pheromone trails (evaporation and
  diffusion) and recruitment; pollination trips.
* **Vegetation**, **trees**, **fungi**, **carcasses and eggs** as in part 1;
  flower seed production rises with pollination; water cells hold
  dissolved nutrients for algae, and aquatic detritus is remineralised by
  bacteria.
* **Nutrient and energy ledgers** booking every flux (fields and colonies
  included); closed budget conserved to 10⁻⁵.
* **Coupling** with fire, floods, drought, temperature and seasons.

## Interface

* Parameters with apply policies (world: reset; climate overrides, growth,
  decomposition, mutation, predation, fluxes, pheromone: next tick; caps:
  immediate). Scheduled parameter changes do not survive a reset.
* Commands: spawn (individuals, field organisms or colonies), kill, remove,
  plant, fell, ignite, every physics-world disaster, outbreak, season
  length, select, follow.
* Metrics: populations, births and deaths per species, colonies, biomass per
  trophic level, energy flows, nutrient pools and balance errors,
  diversity, fights, alarms, hibernating, migrating, airborne, injured,
  infections per pathogen, pollination, flower seed production, pheromone,
  burned area, water, climate.
* View: animals (with altitude, injury, infection, alarm, group), trees,
  carcasses, clutches, field markers, colonies, forager squads, vegetation,
  pheromone and pollination grids, the world view, the species legend with
  visual ids, the selected organism's full inspection and the followed
  organism's trail.

## Verification highlights

* NetLogo grass version: 8/8 seeds persist with sheep ≈ 160 and wolves ≈ 76
  (matching the research note's independent re-implementation); the
  no-grass version collapses in every seed.
* Behaviour tests for flight, swimming, amphibious movement, schooling
  alignment, pack hunts, territorial fights, alarm calls, migration,
  hibernation, epidemics, injuries, nest predation, field population
  dynamics, ant trails, pheromone decay and pollination; conservation with
  fields and colonies; deterministic replay of scenarios.
* Every scenario runs without violating invariants; long experiments show a
  fire burning and the herb layer regrowing, a drought reducing plants and
  herbivores, wolf removal raising herbivore numbers and grazing, invasive
  rats reducing native lizards, and the balanced meadow keeping its
  vertebrates for a year.
* Benchmarks: `docs/ECOSYSTEM.md` §21 (1,000 and 10,000 individual animals
  per tick; 10⁵ and 10⁷ field organisms per simulated hour at the same
  cost).
