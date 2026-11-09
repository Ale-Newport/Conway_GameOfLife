# physics-world — the abiotic world

`crates/physics-world` simulates everything in the Ecosystem Simulator that is
not alive: procedurally generated terrain with drainage, rivers and lakes;
soils, biomes and fertility; a compressed calendar with seasons and
day/night; weather and wind; hydrology (rain, snow, infiltration, evaporation,
groundwater, runoff, floods, drought); a probabilistic fire automaton; a soil
nutrient pool with mass accounting; and disasters as explicit interventions.
It is reused by the `evolution` and `civilization` crates and is runnable on
its own as the `environment.world` model.

Everything is pure Rust, renderer-independent and deterministic for a given
seed, platform and build (see [Determinism](#determinism)).

## Contents

1. [Module map and public API](#module-map-and-public-api)
2. [Time and calendar](#time-and-calendar)
3. [Terrain generation](#terrain-generation)
4. [Climate, biomes and soils](#climate-biomes-and-soils)
5. [Dynamics of one tick](#dynamics-of-one-tick)
6. [Disasters and interventions](#disasters-and-interventions)
7. [The `environment.world` model](#the-environmentworld-model)
8. [Approximations and limitations](#approximations-and-limitations)
9. [Determinism](#determinism)
10. [Performance](#performance)
11. [How to test](#how-to-test)
12. [References](#references)

## Module map and public API

| Module | Content |
|---|---|
| `config` | `WorldConfig` (all generation, climate and dynamics parameters with ranges), `BiomePreset` |
| `terrain` (private) | fBm + domain-warp heightmap, island mask, quantile sea level |
| `drainage` (private) | ocean mask, Priority-Flood + ε, D8 receivers, accumulation, lakes, rivers |
| `landscape` (private) | generation pipeline, slope, biome / soil / fertility classification |
| `climate` | `StaticClimate` (annual means, precipitation pattern), per-cell formulas |
| `calendar` | `Calendar`, `CalendarTime`, `Season`, daylight factor |
| `weather` | `Weather`, `WeatherKind`, `ForcedWeather`, rain-rate calibration, cloud field |
| `hydrology` | column physics, groundwater, routing, `WaterBudget`, `WaterStorage`, `NutrientBudget` |
| `fire` | `FireState`, Alexandridis spread probability, lightning |
| `disasters` (private, re-exported) | `Disaster`, `DisasterOutcome`, `fault_displacement_m`, landslides |
| `world`, `exchange`, `step` (private, re-exported) | `World`: construction, accessors, ecosystem hooks (`MAX_NUTRIENT_ADDITION_G_M2`), the per-tick update |
| `integrity` (private) | structural validation of deserialised worlds (used by `restore`) |
| `report` (private, re-exported) | `EnvironmentReport`, `FuelBurn`, `CellDamage` |
| `view` | `WorldView`, `WorldViewRequest`, `WorldLayer`, `TerrainView`, downsampling |
| `sim` | `EnvironmentSim` (`simulation_core::Simulation` for `environment.world`) |

Key entry points (the `docs/ARCHITECTURE.md` §4 contract):

```rust
World::generate(&WorldConfig) -> World                 // clamps out-of-range values
World::try_generate(&WorldConfig) -> Result<World, WorldError>
World::from_heightmap(&WorldConfig, Grid2<f32>) -> Result<World, WorldError>
World::step(&mut self) -> EnvironmentReport

// Per-cell layers (row-major Grid2, x right, y down)
elevation() slope() surface() biome() soil() water_depth() snow() soil_moisture()
temperature() humidity() light() precipitation() fertility() nutrients()
organic_matter() fuel() canopy() fire()

// Written by the ecosystem every tick
fuel_mut() / set_fuel(x, y, kg_m2)        canopy_mut() / set_canopy(x, y, cover)
add_nutrients(x, y, g_m2) -> Result<f32>  take_nutrients(x, y, g_m2) -> Result<f32>
add_organic_matter(x, y, g_m2) -> Result<f32>

// Time and weather
calendar() time() daylight() weather()

// Interventions (all validate their arguments and return typed errors)
trigger(Disaster) -> Result<DisasterOutcome, WorldError>
ignite(x, y) add_rain(mm) start_drought(ticks) heatwave(ticks) storm(ticks, intensity)
flood(x, y, radius, depth_m) earthquake(x, y, radius, magnitude) tornado(path, width)
landslide(x, y, radius) volcano(x, y, radius, height_m)

// Renderers
view(&WorldViewRequest) -> WorldView      terrain_version()
```

**Ecosystem coupling contract.** Before each `step` the ecosystem writes the
fuel load (kg/m² of burnable biomass) and canopy cover. The step reports
`EnvironmentReport::fuel_consumed` — one `FuelBurn { x, y, amount }` per cell,
already subtracted from `fuel` — so the ecosystem removes that biomass and may
return ash through `add_nutrients`. Nutrient exchange is explicit:
`take_nutrients` never takes more than is available (it returns the amount
actually taken) and every transfer is booked in the `NutrientBudget`, so mass
is conserved; a single addition is limited to `MAX_NUTRIENT_ADDITION_G_M2`
(10⁶ g/m²) so the pools stay finite. Tornado damage arrives as
`EnvironmentReport::wind_damage`, and `EnvironmentReport::lightning_strikes`
counts the strikes that hit non-ocean cells.

## Time and calendar

One tick lasts `1440 / ticks_per_day` minutes (default 240 ticks/day = 6
minutes). A year has four seasons of `days_per_season` days (default 30, a
compressed 120-day year; `docs/ARCHITECTURE.md` §3.3). Hourly processes
(weather transitions, landslide checks) run exactly once per simulated hour
whatever the tick length (`Calendar::hours_started`).

* Seasonal signal `S = sin 2π(f − 1/8)` with `f` the year fraction: 0 in
  mid-spring, +1 mid-summer, −1 mid-winter.
* Diurnal signal `D = cos 2π(h − 15)/24` (warmest at 15:00).
* Day length `L = 12 h + A·S` with `A = clamp(seasonality / 4, 0, 6)` h.
* Daylight (sun-elevation proxy) `= (0.75 + 0.25 S) sin(π (h − sunrise)/L)`
  between sunrise and sunset, 0 at night.

## Terrain generation

1. **Raw height.** `h = w_c C(p) + (1 − w_c) D(p + s·q)`, with `C` a 3-octave
   continent fBm (frequency 1.3 per map), `D` a 7-octave detail fBm
   (frequency 2.6, persistence `0.30 + 0.35 roughness`), domain warping
   `q = (fBm_x(p), fBm_y(p))` with strength `s = 0.12 + 0.25 roughness`
   (Quilez), `w_c = 0.15 + 0.6 continentality`. Coordinates are divided by
   `max(width, height)`, so a map keeps its shape at any resolution. With
   `lake_tendency > 0`, positive Perlin blobs are subtracted
   (`0.1 × lake_tendency × max(0, b − 0.05)`) to create closed basins.
   **Island mode** fades heights to −0.8 with `smoothstep(0.4, 0.95, r)` of
   the normalised radius.
2. **Sea level.** The threshold is the `sea_level` quantile of `h`, so
   `sea_level` is the fraction of cells below sea level. Island mode raises
   it above the highest border cell, guaranteeing an all-ocean border.
3. **Metres.** Land `0.5 m + (peak − 0.5 m) t^1.6` with
   `peak = 150 m + 3850 m × relief`; sea `−(0.5 m + 1499.5 m × t^1.2)`.
4. **Ocean** = cells below 0 m 8-connected (through below-sea cells) to a
   below-sea border cell. Enclosed below-sea basins are land depressions.
5. **Priority-Flood + ε** (Barnes et al. 2014): ocean cells are seeded at the
   sea surface (0 m), land border cells at their elevation; the min-heap is
   ordered by (level, index) so ties are deterministic; every newly reached
   cell is raised to at least `level + 5 mm`. The result has no depressions
   and no perfect flats.
6. **Lakes.** Depression cells (fill > 5 cm) form 8-connected basins.
   With `t = lake_tendency > 0`, basins of ≥ 2 cells whose depth reaches
   `0.5 m + 10 m (1 − t)²` become lakes, deepest first, while total lake
   area stays within `0.25 t²` of the non-ocean cells. All other land is
   raised to the filled surface (sediment fill). Lake water depth is
   `filled − bottom`.
7. **Flow.** D8 steepest descent on the filled surface (diagonals divided by
   √2; O'Callaghan & Mark 1984); border cells without a lower neighbour
   drain off-map. Flow accumulation sums the relative precipitation of every
   upstream cell, processing cells from highest to lowest filled level.
8. **Rivers.** `river_count` sources are drawn (seeded) with weight
   `(z/z_max)² × P × √A` among land cells above 25 % of the highest land, at
   least `max(3, max(w,h) / (1.5√(n+1)))` cells apart. Each river follows the
   receivers to the ocean, the map edge, or through a lake and out of its
   outlet. Beds are carved `0.6 + 0.35 ln(1 + A/50)` m (≤ 4 m) deep, the first
   cell after a lake (its spill rim) is not carved, and each bed is forced at
   least 1 cm below the previous one, so rivers strictly descend. The ocean
   mask is recomputed afterwards (a mouth carved below 0 m becomes estuary).
9. **Slope** by central differences (rise/run).

## Climate, biomes and soils

**Annual mean temperature** `T = T_base + G (y/(H−1) − ½) − Γ z/1000`
(`G` = `latitude_gradient`, cold at the top; `Γ` = `lapse_rate_c_per_km`,
default 6.5 °C/km; `z` = land or lake-surface height).

**Instantaneous temperature**
`T(t) = T_mean + c (A_s S + A_d D (1 − 0.6 k)) + ΔT_weather`, where `A_s` is
`seasonality`, `A_d` half the diurnal range, `k` the local cloudiness and
`c ∈ [0.3, 1]` the continental factor (`0.35 + 0.65 (1 − e^{−d/(0.25 L)})`
with `d` the distance to the ocean; ocean 0.3, lakes × 0.7) which damps
amplitudes near water.

**Precipitation pattern** (normalised to mean 1 over non-ocean cells):
`P = (1 + 0.35 n) × (0.55 + 0.45 e^{−d_w/(0.6 L)}) × (1 + 3 min(s_w, 0.5)) × e^{−max(0, z_b − z)/1200 m} × (1 + 0.4 min(z/2000 m, 1))`
— moisture fBm `n`, distance to open water `d_w`, windward rise along the
prevailing wind over two cells `s_w` (orographic lift), and the highest
terrain within `min(48, L/3)` cells upwind `z_b` (rain shadow). Annual
precipitation is `rainfall_mm × P`.

**Humidity** `RH = base + weather bonus (× cloudiness when positive) − 0.1 D + 0.15 wet − 0.01 max(0, T − 25)`,
clamped to `[0.05, 1]`, with `base = 0.35 + 0.35 min(P/2500, 1) + 0.2 e^{−d_w/8}`.

**Light** at ground level `= daylight × (1 − (1 − τ) k) × (1 − canopy)` with
the state's cloud transmission `τ` (clear 1, cloudy 0.6, rain 0.45, storm
0.25). `canopy` is the ecosystem's shading hook.

**Biomes** (`landscape::classify_biomes`), in priority order:

| Rule | Biome |
|---|---|
| ocean, depth ≤ 30 m and T ≥ 20 °C / otherwise | Reef / Ocean |
| lake, river surface | Lake, River |
| land ≤ 2 cells from ocean, z < 3 m + 0.002 peak, slope < 0.25 | Beach (coast) |
| z ≥ 55 % of the highest land and ≥ 600 m, or slope ≥ 1 | Mountain (alpine / rock) |
| T ≥ −1 °C and (slope < 0.03, P ≥ 450 mm, `ln((A+1)/(slope+0.002)) ≥ 9.5`) or (slope < 0.01, P ≥ 450, fresh water ≤ 1 cell) or (slope < 0.002, P ≥ 600) | Wetland |
| otherwise Whittaker table below | |

| T (°C) | P < 250 | 250–400 | 400–650 | 650–1700 | ≥ 1700 |
|---|---|---|---|---|---|
| < −1 | Tundra | Tundra | Tundra | Tundra | Tundra |
| −1 to 20 | Desert | Grassland | Grassland | Temperate forest | Temperate forest |
| ≥ 20 | Desert | Desert | Savanna | Savanna | Tropical rainforest |

**Presets** (`BiomePreset`) are plain bundles of field values (base
temperature, latitude gradient, rainfall, seasonality, sea level, relief,
roughness, continentality, lake tendency, river count, island) applied by
`WorldConfig::effective`; they add no hidden behaviour. Tests check that
savanna, rainforest, tundra, desert and temperate presets make the
respective biome dominant, that the ocean preset is > 60 % ocean, that the
lake preset has several times more lake area than the default, and that warm
shallow seas grow reefs.

**Soils** (`classify_soils`): ocean/beach → sand; lake and river beds →
silt; slope > 0.8 or mountain → rock (some mountain loam); desert → sand
(rock patches); wetland → peat below 16 °C, clay above; within 2 cells of
fresh water on slopes < 0.05 → silt (alluvium); wet flat tundra → peat;
elsewhere a soil-texture noise picks clay / sand / loam.

| Soil | Infiltration (m/h) | Capacity (m) | Field capacity | Percolation (/h) | Base fertility | Critical slope |
|---|---|---|---|---|---|---|
| Sand | 0.030 | 0.06 | 0.35 | 0.08 | 0.30 | 0.65 |
| Loam | 0.013 | 0.15 | 0.55 | 0.03 | 0.85 | 0.75 |
| Clay | 0.002 | 0.20 | 0.70 | 0.005 | 0.65 | 0.50 |
| Rock | 0.001 | 0.01 | 0.30 | 0.10 | 0.05 | 1.40 |
| Peat | 0.006 | 0.30 | 0.75 | 0.004 | 0.50 | 0.45 |
| Silt | 0.007 | 0.18 | 0.60 | 0.015 | 0.95 | 0.55 |

**Fertility** `= base(soil) × (0.3 + 0.7 f_T f_P) × (1 − 0.6 min(slope, 1))`
with `f_T = smoothstep(−8, 12, T)`, `f_P = 0.2 + 0.8 smoothstep(100, 900, P)`;
0 on water. Initial mineral nutrients are `50 g/m² × fertility` and organic
matter `150 g/m² × fertility`.

## Dynamics of one tick

`World::step` runs, in this order:

1. Effects of interventions since the last step (events, wind damage, lava
   fuel consumption, terrain changes) are moved into the report.
2. Ecosystem inputs are sanitised (fuel finite and ≥ 0, canopy in [0, 1]).
3. **Weather** (below).
4. **Atmosphere**: temperature, humidity, light and precipitation fields.
5. **Hydrology**: column pass, groundwater and baseflow, surface routing.
6. **Drought index** and **flood** count (event when flooding starts).
7. **Fire**: passive states, lightning, spread sub-steps.
8. **Landslides**, once per started hour; terrain refresh if anything moved.

### Weather

An hourly Markov chain over Clear, Cloudy, Rain, Storm, Heatwave and Drought
(`weather::transition_weights`): rain and storm weights scale with the
wetness `w = clamp(rainfall/900 mm, 0.05, 4)`, storms and heatwaves are more
likely in summer (`S > 0`), droughts start rarely (more in dry, warm
climates) and last ~10 days, heatwaves ~3 days. **Snow** is reported when it
rains or storms and the mean land temperature is below 0 °C; each cell
independently receives snow when its own temperature is below 0 °C.

The Rain-state rate is **calibrated** so that the long-run mean equals
`rainfall_mm / 365` per day: the stationary distribution of the chain is
computed by power iteration at eight seasonal phases and the expected
precipitation multiplier (Rain 1, Storm 3) is averaged. Interventions can
impose a state (`ForcedWeather`) for a duration; a forced storm multiplies
precipitation by its intensity.

**Wind** is an Ornstein–Uhlenbeck walk relaxing (12 h time scale) towards the
prevailing wind × a state factor (storm 2.5, heatwave 0.6...), with Gaussian
increments `σ = (0.5 + 0.1 V₀)√Δt` m/s, multiplied each tick by a half-normal
gust factor (σ 0.2, 0.5 in storms). The **cloud field** is a 3-octave 3-D fBm
(x, y, slow time) evaluated every 8 cells and sampled bilinearly; it is
advected with the mean wind and gives the local precipitation multiplier
`m = max(0, 1 + c n) / mean` (contrast 4, storms 6). Clipping at 0 would
raise the mean of `max(0, 1 + c n)` above 1 — by 3–15 % in the review's
measurements, which inflated the long-run rainfall by the same amount — so
the field is divided by its exact cell mean (computed from the bilinear
interpolation weights of the coarse nodes): the clouds redistribute the
regional rain over the map without changing its total. Local cloudiness is
`cover(state) × min(1, m)` and the precipitation rate
`rate(state) × P(x, y) × m` mm/h.

### Hydrology

Water is booked in m³ in `WaterBudget`; surface water is stored internally in
**f64** (the public `water_depth` layer is an f32 mirror) because deep lakes
receive increments far below the f32 resolution of their depth. Per non-ocean
cell (`hydrology::column_step`):

1. precipitation; snow if `T < 0 °C`;
2. degree-day snow melt, 4 mm °C⁻¹ day⁻¹;
3. open-water evaporation
   `PET = s × 3.3·10⁻⁵ m h⁻¹ °C⁻¹ × max(0, T + 5)(1 − RH)(0.3 + 0.7 daylight)(1 + 0.08 |wind|) × m_weather × Δt`
   (`s` = `evaporation_scale`; `m_weather` = 1.5 drought, 1.4 heatwave, 0.5
   rain);
4. on land only: infiltration (soil rate, 10 % on frozen ground, limited by
   free pore space), soil evaporation/transpiration `(PET − open) × factor ×
   saturation`, percolation of `percolation_rate × Δt` of the excess above
   field capacity into the groundwater store, mineralisation of organic
   matter `k = 5·10⁻⁴ h⁻¹ × 2^{(T−20)/10} × min(1, 2s)` and leaching of mineral
   nutrients `∝ 0.5 × percolated / capacity`.

Lakes and rivers are **gaining reaches**: their beds are treated as
connected to the groundwater, so they get precipitation, melt and open-water
evaporation but neither infiltrate nor percolate.

**Groundwater** is one lumped linear reservoir (time constant 40 days)
initialised at its steady state for 25 % recharge. It releases **baseflow**
`G (1 − e^{−Δt/τ})` evenly into river cells (lake cells if the map has no
rivers), which keeps rivers flowing through droughts.

**Surface routing (documented choice).** Instead of the momentum-carrying
virtual-pipe model of Mei et al. (2007), routing is a *momentum-free pipe
variant* — water-surface equalisation over the 8-neighbourhood, one sub-step
per started 6 simulated minutes, `clamp(⌈minutes per tick / 6⌉, 1, 16)`
(`Calendar::substeps_per_tick`, shared with the fire automaton). Each cell
with water releases
`0.8 × min(d, ½ Δmax)` where `Δmax` is the largest drop of water surface to a
neighbour, split in proportion to `Δ_k / L_k` among lower neighbours. The
ocean (surface 0 m) and the map edge (free outfall of drop `d`) are sinks.
Outflows are computed from the previous state into a flux buffer and
gathered afterwards (double buffering), so the update is parallel,
deterministic, non-negative and mass conserving. It fills depressions to
their spill level, carries river flow downstream, spreads floods and drains
them, and stays stable at 6-minute ticks without the CFL sub-stepping a
pipe model would need. Water moves at most one cell per sub-step.

**Floods**: dry-land cells deeper than `flood_threshold_m` (0.5 m). **Drought
index** `= 0.6 × clamp(1 − R̄/R_clim) + 0.4 × clamp(1 − s̄/s₀)` with `R̄` a
10-day moving average of land rainfall, `R_clim` the climatological rate,
`s̄` the mean land saturation and `s₀` its value at generation.

### Fire (Alexandridis et al. 2008)

States: `Unburnable`, `Fuel`, `Burning { intensity, steps }`,
`Burned { since }` and `Lava { steps_left }` (volcano extension). Passive
states are re-derived each tick: a cell is `Fuel` when it is land, has ≥ 0.05
kg/m² of fuel, < 5 mm of standing water and < 5 mm of snow; a burned cell
carries fire again after one day with ≥ 0.3 kg/m² of fuel.

Every sub-step (`Calendar::substeps_per_tick`: one per started 6 simulated
minutes, ≤ 16 per tick) each burning
cell ignites each `Fuel` neighbour (Moore neighbourhood) independently with

```
p = p_h (1 + p_veg)(1 + p_den) · e^{c1 V} e^{c2 V (cos θ − 1)} · e^{a θ_s} · f_m · f_I · k
p_h = 0.58, c1 = 0.045 s/m, c2 = 0.131 s/m, a = 0.078 /°
θ   = angle between the wind vector and the spread direction
θ_s = atan((z_target − z_source) / D), D = L (orthogonal) or √2 L (diagonal), degrees
p_veg: temperate forest +0.4; grassland/savanna 0; beach/desert/tundra/rainforest −0.3; mountain −0.2; wetland −0.4
p_den = clamp(0.3 ln(fuel / 1 kg m⁻²) / ln 3, −0.4, 0.3)
f_m   = max(0, 1 − m / 0.9),  m = 0.5 soil saturation + 0.35 RH + 0.15 min(1, water / 1 cm)
f_I   = clamp(0.6 + 0.4 I, 0.6, 1.8)   (I = source intensity; lava I = 2)
k     = fire_spread
```

Spread decisions are synchronous (computed from the state at the start of
the sub-step). A burning cell consumes 35 % of its remaining fuel per
sub-step (intensity `= consumed / (0.35 × 2 kg/m²)`), and becomes `Burned`
when its fuel drops below 0.05 kg/m², when it is under water or snow, after
40 sub-steps, or by rain with probability `1 − e^{−0.35 r}` (r in mm/h).
**Lightning**: during storms `Poisson(lightning_rate × map km² × Δt h)`
cloud-to-ground strikes (at most 10 000 per tick) hit uniformly random cells
of the whole map, so every cell receives the configured strike density;
strikes on the ocean have no effect, the others are counted in
`EnvironmentReport::lightning_strikes`, and a strike on fuel ignites with
probability `0.35 (1 − m)`. (The first implementation drew the strike count
for the land area but spread the strikes over the whole map, so the land
received only its area fraction of the configured density.)

### Landslides

Once per hour, land cells with a soil cover (not bare rock) steeper than
their soil's critical slope and above 95 % saturation fail with probability
`0.005 × (s − 0.95)/0.05`. A failure moves half of the excess height
`½ (g − g_c) × run` (≤ 5 m) to the steepest downhill neighbour, together with a
proportional share of nutrients and organic matter; the source loses
fertility and becomes rock when more than half a metre moved. Changing a
soil type conserves its water (saturation is rescaled to the new capacity and
the remainder released as surface water).

## Disasters and interventions

All interventions go through `World::trigger(Disaster)` (and through
`Simulation::apply` in `environment.world`), validate their arguments with
typed `WorldError`s and leave the world untouched on error.

| Disaster | Physical effect (approximation) |
|---|---|
| `Ignite { x, y }` | sets the cell burning if it currently holds dry fuel |
| `Drought { duration }` | forces the Drought state: no rain, ×1.5 evaporation, −0.25 RH, +2 °C |
| `Heatwave { duration }` | forces the Heatwave state: +4.2 °C, rising to +7 °C in mid-summer (`7 × (0.6 + 0.4 max(S, 0))`), ×1.4 evaporation, calm wind |
| `Storm { duration, intensity }` | forces a thunderstorm: rain ×3 × intensity, wind ×2.5, gusts, lightning |
| `AddRain { mm }` | adds rain (snow below 0 °C) uniformly on non-ocean cells; booked as input |
| `Flood { x, y, radius, depth }` | adds a dome of water `depth (1 − (d/r)²)`; routing spreads and drains it |
| `Earthquake { x, y, radius, M }` | straight fault at a seeded angle through the epicentre; displacement `±A(1 − (d/r)²)` with the Wells & Coppersmith (1994) average displacement `log₁₀ A = −4.80 + 0.69 M` (A in m, all slip types; ≈ 0.22 m at M6, 1.1 m at M7, 2.4 m at M7.5, extrapolated outside M 5.6–8.1) applied vertically; shaking `s = (1 − d/r)(M − 3)/6.5` triggers rockfall on slopes > 0.35 (debris `2 m × s × min(slope, 1)` moved downhill, fertility halved at the source, ×0.7 at the deposit — debris on a river cell raises its bed and can dam it) and forced slope failures above `(1 − 0.5 s_max)` × critical slope; land that sinks below sea level and connects to the ocean becomes ocean and hands its water to it |
| `Tornado { path, width }` | damage `1 − (d / (width/2))²` within half a width of the polyline; canopy reduced by the damage; damage map reported to the ecosystem |
| `Landslide { x, y, radius }` | forced slope failures where the slope exceeds half the critical slope |
| `Volcano { x, y, radius, height }` | cone `H (1 − d/r)^1.5` with a crater of depth `0.3 H` in the inner 12 %; non-ocean cells within `0.35 r` become lava for two days (fuel consumed and reported, water and snow boiled off, soil → rock, fertility 0.05, ignites neighbours); ash adds `300 g/m² × (1 − d/3r)` of organic matter within `3 r`, which mineralises into nutrients over the following weeks |

Every terrain change re-derives the ocean mask, slope, annual temperature and
biomes and bumps `terrain_version`, so renderers refetch static layers. Land
that sinks into the ocean hands its surface, snow and soil water to it; sea
floor lifted above 0 m becomes dry land; sea floor that stays below 0 m but
is cut off from the open ocean (a bay closed by uplift) keeps its sea water
as a lake whose surface is at 0 m — that water is booked in
`WaterBudget::added_m3`. (Before the review such a bay lost its water and
became a dry pit.)

## The `environment.world` model

`EnvironmentSim` implements `simulation_core::Simulation` with
`Command = Disaster`, `ViewRequest = WorldViewRequest`, `View = WorldView`,
`tick_unit = "minute"` and `dt = 1440 / ticks_per_day`. It adds a
**vegetation proxy** so fires have fuel without organisms:
`fuel += (capacity(biome) − fuel)(1 − e^{−Δt/τ})` on land that is not
burning (capacities in kg/m²: rainforest 4, temperate forest 3, savanna 1.2,
wetland 1, grassland 0.8, tundra 0.3, mountain 0.2, beach 0.1, desert 0.04);
it starts at 60 % of capacity and can be disabled.

| Parameter | Range (default) | Policy |
|---|---|---|
| `width`, `height` | 16–2048 (256) | requires reset |
| `seed` | 0–2⁶³−1 (1) | requires reset |
| `preset` | none / temperate / savanna / rainforest / island / lakes / ocean / tundra / desert / wetland / mountains | requires reset |
| `cell_size_m` | 1–10000 (50) | requires reset |
| `sea_level`, `relief`, `roughness`, `continentality`, `lake_tendency` | 0–1 (0.35, 0.45, 0.5, 0.5, 0.35; sea ≤ 0.95) | requires reset |
| `island` | bool (false) | requires reset |
| `river_count` | 0–200 (8) | requires reset |
| `ticks_per_day`, `days_per_season` | 1–14400 (240), 1–3650 (30) | requires reset |
| `base_temperature_c` | −40–45 (14) | next tick |
| `latitude_gradient` | −60–60 (12) | next tick |
| `rainfall_mm` | 0–8000 (900) | next tick (rain rate recalibrated) |
| `seasonality`, `diurnal_range_c` | 0–40 (10, 10) | next tick |
| `lapse_rate_c_per_km` | 0–15 (6.5) | next tick |
| `wind_speed`, `wind_from_deg` | 0–60 (4), 0–360 (270) | next tick |
| `fire_spread`, `lightning_rate`, `evaporation_scale`, `flood_threshold_m` | 0–5 (1), 0–10 (0.02), 0–10 (1), 0.01–10 (0.5) | next tick |
| `vegetation_proxy`, `fuel_regrowth_days` | bool (true), 1–3650 (60) | next tick |

`params()` reports float fields as the shortest decimal that round-trips to
the stored `f32` (`0.35`, not `0.3499999940395355`), so a reported set equals
what was set and always validates against the specs (with the raw `f32`
widening, a value at the lower end of a range such as `flood_threshold_m =
0.01` was reported below its minimum). `EnvironmentSim::new` and `restore`
reject seeds above 2⁶³ − 1, which the integer `seed` parameter cannot
represent.

Editing a field that the active preset controls first turns the preset into
explicit values (the preset parameter then reads `none`), so the edit is not
overwritten at the next reset. Runtime climate edits recompute the static
climate fields but keep the generated biomes (`World::reclassify_biomes`
re-derives them on demand).

Metrics: `mean_temperature` (°C), `rainfall` (mm/tick), `water_volume`,
`groundwater`, `snow_volume` (m³), `soil_moisture` (ratio), `burning_cells`,
`burned_area` (ratio of non-ocean cells), `drought_index` (ratio),
`flooded_cells`, `wind_speed` (m/s), `daylight` (ratio).

Snapshots (`schema_version` 1) serialise the whole state with postcard:
every grid, the weather, budgets, pending intervention effects, the last
report, the event log and all three random streams; a restored run replays
bit-identically (tested). Because snapshots are user input (files),
`restore` also checks the decoded state — valid configuration, calendar
matching it, every layer `width × height`, burning-cell list sorted and
inside the map — and returns `SimError::Serialization` instead of panicking
later on an out-of-range index; a failed restore leaves the model unchanged.

**Views.** `WorldViewRequest { layers, fire, block, known_terrain_version }`
returns block-mean scalar layers, the highest-priority fire state per block,
weather and calendar info, and the static terrain (block-mean elevation,
most frequent biome / soil / surface) only when the renderer's cached
version is stale.

## Approximations and limitations

* **Compressed time.** Default ticks are 6 minutes and years 120 days.
  Rainfall is specified per 365-day year but applied as a daily rate, so a
  compressed year receives `days_per_year / 365` of the annual total.
* **Spatial scale.** Defaults represent 50 m cells (a 256² map is 12.8 km
  wide); the noise is resolution-independent, so larger maps show the same
  features in more detail rather than more features.
* **No erosion during generation.** Valleys come from noise, depression
  filling and river carving, not from simulated hydraulic or thermal
  erosion; landslides are the only dynamic sediment transport.
* **Routing** has no momentum: no waves, hydraulic jumps or realistic flow
  velocities, and water moves at most one cell per sub-step (6 minutes
  for ticks up to 6 minutes, ≈ 0.14 m/s at 50 m cells; ticks longer than
  96 minutes are capped at 16 sub-steps, so flow is slower per simulated
  hour), and river depths are those of slow flow.
* **Groundwater** is a single lumped reservoir (no lateral groundwater
  flow, no water table); lake and river beds are always treated as gaining
  reaches; infiltration uses constant saturated conductivities (no Horton
  decay or SCS curve numbers).
* **Evaporation** is an empirical temperature/humidity/daylight index, not
  Penman–Monteith or Hargreaves; transpiration is folded into soil
  evaporation (the ecosystem may model plant water use separately).
* **Climate** has no seasonal lag, no ocean currents and no moisture
  advection; the precipitation pattern is static (computed for the
  prevailing wind at generation) and only modulated by the advected cloud
  field. The latitude gradient is linear across the map. The rain-rate
  calibration uses the chain's stationary distribution at eight fixed
  seasonal phases; long runs (12 compressed years at 1-hour ticks, 8 seeds,
  48² maps) measured 0.92–1.06 of the configured rainfall (mean 0.98), the
  remaining spread being weather randomness (droughts last ~10 days).
  Before the cloud field was normalised the same kind of runs gave
  1.03–1.15.
* **Biomes** use simplified Whittaker thresholds (no boreal forest, shrubland
  or temperate rainforest classes, because the required biome list does not
  include them). Biomes are classified at generation and after terrain
  changes, not when the climate parameters change at runtime.
* **Memory.** The routing flux buffer holds nine `f64` outflows per cell
  (72 B) next to the `f64` water store, 19 `f32` layers, the 16-byte fire
  state and four byte-sized layers, about 175 B per cell in all: ≈ 11.5 MB
  for 256², ≈ 740 MB for the largest 2048² model map.
* **Fire.** The fuel-moisture input is a composite index in `[0, 1]`
  (soil saturation, humidity, standing water), not fuel moisture content, so
  its extinction value 0.9 is on that index scale (Rothermel's 0.12–0.40
  moisture of extinction does not apply directly). `p_veg` values per biome
  are our mapping of Alexandridis' vegetation classes. Spread is at most one
  cell per sub-step (same sub-steps as routing), there is no
  spotting and no crown-fire distinction. Fire does not heat the
  atmosphere.
* **Disasters.** Earthquakes use one straight fault and a parabolic
  displacement profile (no elastic dislocation model); tornadoes do not
  move air or water; volcanoes do not erupt over time (the cone appears at
  once), lava does not flow, and ash does not dim light.
* **Nutrients** are one generic mineral pool (no N/P distinction) plus
  organic matter; leaching losses leave the system.
* **Floating point.** Results are reproducible on the same platform and
  build; cross-platform bit-exactness is not promised.

## Determinism

* All randomness comes from `SimRng` streams derived from the seed
  (`TERRAIN`, `CLIMATE`, `RIVERS`, `SOIL` at generation; `WEATHER`, `FIRE`,
  `DISASTERS` stored in the world state and snapshots). Noise generators
  are seeded from these streams.
* Parallel loops (rayon over rows) only compute pure per-cell functions into
  separate buffers; reductions are collected per row in row order and summed
  sequentially, so float sums do not depend on thread scheduling.
* Fire spread, lightning and landslides iterate cells in index order with a
  single stream; ignitions are applied after all decisions of a sub-step.
* No `HashMap` iteration drives any logic.

## Performance

Criterion (`cargo bench -p physics-world --bench world -- --warm-up-time 2
--measurement-time 5`), release profile, Apple M2 Max (12 cores), 256 × 256
default world (seed 7):

| Benchmark | Time (median, 95 % interval) |
|---|---|
| `world_step_256x256` (calm weather, evolving from tick 0) | 1.35 ms [1.29, 1.48] |
| `world_step_256x256_storm` (forced thunderstorm: rain everywhere, routing, lightning) | 2.35 ms [1.89, 3.01] |
| `world_step_256x256_fire_front` (one step of a fresh copy with a map-wide fire line, drought) | 2.01 ms [1.80, 2.25] |
| `generation/generate_256x256` | 17.9 ms [17.6, 18.3] |

Measured while other agents were compiling on the same machine (1-minute
load average 13–17 on 12 cores), so the intervals are wide; a scratch
harness taking the best of 1000 consecutive steps gave 1.7 ms (calm) and
1.5 ms (storm) with all threads and 2.3 / 2.6 ms on one thread. One step is
therefore ≈ 500–700 ticks/s (≈ 2–3 simulated days per second at 6-minute
ticks) for a 256² world before any organisms are added. (The first version
of the storm benchmark also lit fires, but the storm put them out within a
few ticks, so it measured a storm without fire; the fire front now has its
own benchmark that steps a fresh copy each iteration.)

## How to test

```sh
CARGO_TARGET_DIR=target/agents/physics-world cargo test -p physics-world
CARGO_TARGET_DIR=target/agents/physics-world cargo clippy -p physics-world --all-targets -- -D warnings
CARGO_TARGET_DIR=target/agents/physics-world cargo bench -p physics-world
```

Unit tests cover the calendar (including the shared sub-step count),
Priority-Flood/D8/lake selection, soil and biome tables, column
water/nutrient conservation, routing outflow bounds, the Alexandridis
formula, Poisson sampling, the unit cell mean of the cloud field, the Wells
& Coppersmith displacement, view downsampling and the rejection of
structurally inconsistent snapshots. Two **brute-force cross-checks** run
on 25 random grids each: Priority-Flood+ε against a relaxation of
`W_i = max(z_i, min_j W_j + ε)` from +∞, and D8 receivers plus flow
accumulation against an explicit steepest-descent search and a walk down
every cell's flow path. Integration tests (`crates/physics-world/tests/`):

* `generation.rs` — determinism per seed; every ocean cell connected to the
  border (island border all ocean); rivers continuous and strictly
  descending to ocean, lake or map edge; ≥ 8 biomes and ≥ 5 soils in default
  worlds; presets make their biome dominant; every cell's annual mean
  temperature equals the latitude/lapse-rate formula; validation of
  heightmaps and configurations.
* `hydrology.rs` — water balance (storage − initial = inputs − evaporation −
  outflow, relative error < 10⁻³) through storms, extra rain, floods, snow,
  earthquakes and volcanoes; runoff to the low edge; floods reported and
  spreading; drought lowers moisture and raises the index; rivers keep
  flowing on baseflow; nutrient conservation, non-negativity and the
  addition cap.
* `fire.rs` — faster spread downwind than upwind (12 seeds) and uphill than
  downhill (8 seeds on a 26.6° ramp); no spread without fuel and exact
  fuel-consumption reporting; extinction by rain and storms; lightning
  ignitions and a strike density on land within 15 % of
  `rate × land km² × hours` on a half-sea map; recovery of burned cells.
* `disasters.rs` — earthquake displacement and terrain version; landslides
  conserve soil volume; volcano cone, lava and ash; tornado damage swath;
  heatwave/storm effects; invalid interventions rejected without side
  effects; subsiding coast hands its water to the ocean; a bay cut off by
  uplift stays a lake at sea level with its water booked.
* `simulation.rs` — descriptor/params/metrics consistency (a default world
  reports exactly the spec defaults, and reported parameters always
  validate), snapshot round trip and bit-identical replay, determinism
  through `DynSimulation`, apply policies (a runtime base-temperature change
  shifts every cell's annual mean by exactly the difference), typed command
  and configuration errors, view downsampling and terrain caching,
  vegetation proxy, seasons warm and cool with the right sign, long-run
  rainfall within 20 % of the configured climate.

## References

* Alexandridis, A., Vakalis, D., Siettos, C. I., Bafas, G. V. (2008). A
  cellular automata model for forest fire spread prediction: the case of the
  wildfire that swept through Spetses Island in 1990. *Applied Mathematics
  and Computation* 204:191–201. doi:10.1016/j.amc.2008.06.046 (constants as
  reused by Freire & DaCamara 2019, *NHESS* 19:169).
* Barnes, R., Lehman, C., Mulla, D. (2014). Priority-Flood: An optimal
  depression-filling and watershed-labeling algorithm for digital elevation
  models. *Computers & Geosciences* 62:117–127.
  doi:10.1016/j.cageo.2013.04.024.
* O'Callaghan, J. F., Mark, D. M. (1984). The extraction of drainage
  networks from digital elevation data. *Computer Vision, Graphics, and
  Image Processing* 28:323–344.
* Mei, X., Decaudin, P., Hu, B.-G. (2007). Fast hydraulic erosion
  simulation and visualization on GPU. *Pacific Graphics*, 47–56
  (virtual-pipe model; our routing is a momentum-free simplification).
* Musgrave, F. K., Kolb, C. E., Mace, R. S. (1989). The synthesis and
  rendering of eroded fractal terrains. *SIGGRAPH* (fBm terrain);
  Quilez, I., *Domain warping*, https://iquilezles.org/articles/warp/.
* Whittaker, R. H. (1975). *Communities and Ecosystems* (temperature ×
  precipitation biome diagram; thresholds here are our simplification).
* Blackman, D., Vigna, S. (2018). Scrambled linear pseudorandom number
  generators (xoshiro256++, via `simulation_core::SimRng`).
* Project research notes: `docs/research/ecology-alife.md` §5 (fire) and §6
  (hydrology, terrain, climate, biomes).
