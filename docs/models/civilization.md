# civilization — Emergent Civilizations

`crates/civilization` is the model behind the **Emergent Civilizations** mode.
People with needs, energy, skills, inventories, preferences and relationships
live on a self-contained, regenerating tile world. A utility AI chooses what
each of them does; they walk along A* routes, gather, farm, craft tools, build
huts, group stores and farm plots, barter at marginal-rate-of-substitution
prices, share food, steal and fight. Nothing is scripted: roads emerge from
footfall, groups from mutual affinity, territories from group buildings,
specialisation from learning by doing and trade, and population change from
births and deaths.

The model implements `simulation_core::Simulation` as `CivilizationSim`
(model id `civilization`, family *agent-based*, one tick = one hour) and uses
`organism-engine` for its agent store, kinematics, steering, perception,
utility AI, drives and intent resolution, and `analytics` for the Gini
coefficient. It does not depend on `physics-world`.

## Contents

1. [Module map and public API](#module-map-and-public-api)
2. [Time scales](#time-scales)
3. [World](#world)
4. [Agents](#agents)
5. [Decisions](#decisions)
6. [Production](#production)
7. [Economy: welfare, barter and markets](#economy-welfare-barter-and-markets)
8. [Society: affinity, groups and sharing](#society-affinity-groups-and-sharing)
9. [Conflict](#conflict)
10. [Territory](#territory)
11. [Births, deaths and inheritance](#births-deaths-and-inheritance)
12. [One tick and determinism](#one-tick-and-determinism)
13. [Parameters](#parameters)
14. [Metrics](#metrics)
15. [Commands and views](#commands-and-views)
16. [Scenarios](#scenarios)
17. [Approximations and limitations](#approximations-and-limitations)
18. [How it was verified](#how-it-was-verified)
19. [Performance](#performance)
20. [References](#references)

## Module map and public API

| Module | Content |
|---|---|
| `sim` | `CivilizationSim` (`new(Scenario)`, accessors `world()`, `agents()`, `buildings()`, `groups()`, `events()`, `last_tick_stats()`, `reseed(seed)`), `descriptor()`, `RULES` (rule-panel text), `MODEL_ID`, `SCHEMA_VERSION` |
| `config` | `Scenario` (file header, world, populations, parameter overrides), `WorldSpec`, `PopulationSpec`, `PreferenceDist`, `Dist`, `Region`, runtime `Rules` with their `ParamSpec`s |
| `scenarios` | `parse(text, origin)`, `builtin()`, `builtin_by_id(id)`, `load_file(path)`, `load_dir(path)`, `BUILTIN_FILES` (embedded copies) |
| `terrain`, `noise` | `Terrain`, `Resource`, `TerrainSpec` (`Generated(GenParams)` or `Map { rows }`), `ResourceZones`, `generate`, `parse_map`; seeded fractal `ValueNoise` |
| `world`, `path` | `World` (terrain, stocks, trails, danger, buildings, territory, connected land regions), `Claim`; `PathFinder` (A*) |
| `agent`, `skills` | `Agent`, `Needs`, `Preferences`, `Relations`, `Memory`, `Activity`, `Goal`, `Target`, `DeathCause`, demographic constants; `Skills`/`Skill` and `Work` (kinds of production), re-exported from `agent` |
| `economy` | `Good`, `Bundle`, `welfare`, `ln_welfare`, `mrs`, `marginal_need`, `barter`, `trade_potential`, `expected_gain`, `meeting_steps` |
| `buildings`, `groups`, `territory` | `Building`, `BuildingKind`, `Crop`, `Buildings`; `Group`, `GroupId`, `Groups`; the territory update |
| `command`, `view`, `metrics` | `CivCommand`; `CivViewRequest`/`CivView` and its parts; `metric_specs`, `concentration`, `specialization` |
| private | `decide` (utility AI), `navigate` (standing tiles, reach, A* routes, exploration targets), `act` (movement, individual actions), `social` (interactions), `life` (physiology, deaths, upkeep, daily bookkeeping), `step` (tick pipeline), `state` (`CivState`, `TickStats`) |

## Time scales

* One tick is one hour (`ModelDescriptor::time("hour", 1.0)`), a day is 24
  ticks.
* **Compressed demography** (approximation): a "year" of life lasts 240
  ticks (ten days) so that generations turn over during a session. Founders
  are 16–35 years old, adulthood starts at 14, fertility ends at 45 and old
  age kills at a lifespan drawn uniformly from 55–75 years. Births are at
  least 2 years apart per parent.
* Physiological rates (hunger, thirst, fatigue, food spoilage) are in real
  hours.

## World

### Terrain

| Terrain | Walk cost | Resource (capacity, growback/tick) | Farm fertility | Buildable |
|---|---|---|---|---|
| Grassland | 1.0 | – | 0.6 | yes |
| Forest | 1.6 | wood (20, 0.01) | – | no |
| Hills | 2.0 | stone (30, 0.003) | – | yes |
| Water | impassable | fish (6, 0.012), caught from the shore | – | no |
| Riverbank | 1.0 | – | 1.0 | yes |
| Berry bushes | 1.2 | berries (8, 0.02) | – | no |
| Mountain | impassable | – | – | no |

Capacities and growback rates are multiplied by the world's `richness`;
growback is additionally multiplied by the per-resource regeneration
parameters. Growback is Sugarscape's rule G_α:
`stock ← min(capacity, stock + α)` every tick.

### Generation

`TerrainSpec::Generated` builds the map from three independent layers of
seeded fractal value noise (4 octaves; lattice values from a SplitMix64 hash,
smoothstep-bilinear interpolation, normalised fBm). Class fractions are exact
quantiles:

1. the lowest `water` share of elevation becomes water, the highest
   `mountains` share mountain;
2. among the rest, the highest `hills` share of `elevation + 0.5·b_m·x̂`
   becomes hills;
3. remaining land touching water (8-neighbourhood) becomes riverbank;
4. the highest `forest` share of `woods + b_m·x̂` becomes forest and the
   highest `berries` share of `bushes + b_f·x̂` berry bushes; the rest is
   grassland.

`x̂ = x/(w − 1) − 0.5`. With `ResourceZones::Mixed` the biases are 0; with
`Split { strength: s }`, `b_m = −s` and `b_f = +s`, concentrating materials
in the west and food in the east. `TerrainSpec::Map` reads ASCII rows
(`.` grassland, `T` forest, `h` hills, `~` water, `r` riverbank, `b` berries,
`^` mountain) and reports the row and column of bad symbols.

### Trails and roads (stigmergy)

Roads follow a tile version of the active-walker model (Helbing, Keltsch &
Molnár 1997). Every time an agent enters a tile it deposits
`ΔG = 1 − G/G_max` (`G_max = 100`, saturating footfall); every tick trails
fade, `G ← G·(1 − 1/T)` (`T` = `road_decay_ticks`). The intensity
`I = G/G_max ∈ [0, 1)` lowers the walking cost,
`c = c_terrain·(1 − k·I)` (`k` = `road_discount`), which A* route planning
then prefers, so frequently used routes reinforce themselves. With a visit
rate `f` per tick the steady state is `G* = fT/(1 + fT/G_max)`. A tile whose
intensity reaches `road_threshold` is counted and drawn as a road.

### Danger

Fights add 1 danger to the tile (0.5 to its neighbours), thefts 0.5 and
disasters 0.5 over their area; danger decays by 3 % per tick and feeds the
agents' safety need and their choice of work places.

## Agents

| State | Content |
|---|---|
| kinematics | position (tile units), velocity, heading (`organism_engine::Kinematics`) |
| needs | energy `E` (hunger = `1 − E`); thirst, fatigue and loneliness as `organism_engine::Drive`s; perceived danger |
| health, age | health in `[0, 1]`, age and lifespan in ticks, last cause of damage |
| skills | gathering, farming, crafting, building, trading in `[0, 1]`; activity-share moving average |
| inventory | food, wood, stone, tools (`Bundle`), carry limit 24 units of food+wood+stone |
| preferences | risk tolerance, cooperation tendency, trade propensity, time preference (impatience) |
| relationships | affinity in `[−1, 1]` towards at most `max_relationships` agents (the weakest is forgotten first) |
| memory | last shore, rich food/wood/stone patches (≥ 2 units when seen), last market place |
| learned demand | per good, the stock kept for selling (raised by sales, fading 3 %/day) |
| social | group, home hut, farm plot, parents, generation, last birth, founding population |
| goal | activity, target (tile, building or agent), building plan, route, next decision tick |

### Needs and health

Per tick (`effort` = 0.6 when working, 0.3 when walking, −0.3 when resting):

```text
E        ← max(0, E − metabolism·(1 + effort))          eating: +0.25 per food unit, ≤ 2 units/tick
thirst   ← thirst + thirst_rate·(1.3 if working)        drinking at a shore (land touching water): −0.25/tick
fatigue  ← fatigue + fatigue_rate·(1.5 working, 1.2 walking)   resting: −0.04/tick, −0.07 at home
lonely   ← lonely + 0.004                               company: −0.05/tick socialising, −0.006 near a friend
danger_s ← danger_s + 0.1·(threat − danger_s)
threat   = (min(1, 0.5·Σ danger over 5×5 tiles) + 0.15 in foreign territory)·(1 − 0.6·risk)
health   −= 0.012 if E = 0;  −= 0.015 if thirst = 1;  += 0.003 if E > 0.4 and thirst < 0.6
```

An agent dies at zero health (cause: starvation, thirst or conflict, from the
last damage) or at its lifespan. Carried food spoils at 0.1 % per tick and
stored food at 0.05 % per tick.

## Decisions

Decision making follows the layering of `docs/research/ecology-alife.md` §7.1:
needs every tick, utility decisions occasionally, steering every tick.

* **When.** An agent chooses a new goal when its goal ended or failed, when
  it cannot reach its target, when thirst or hunger reach 0.8 and the goal
  does not address them (a search trip for a water or food source the agent
  does not know is left to finish), and otherwise `8 + route length` ticks
  after its last decision (it commits to a trip until the expected arrival).
* **Reachability.** The world labels the 4-connected regions of passable
  land (A* reachability equals 4-connectivity because diagonal moves need
  both orthogonal tiles free). Agents only consider tiles, buildings and
  agents in their own region, remembered places they can no longer reach are
  forgotten, and routes to other regions are rejected in O(1) instead of
  exhausting A*.
* **Perception.** One scan of the tiles within `vision` finds the best food,
  wood and stone tile (score `min(stock, 4)/4 · place · 1/(1 + 0.12·d)`,
  where `place` penalises foreign territory unless the agent is bold and
  recent danger unless it is risk-tolerant), the nearest shore, the agent's
  or its group's farm plots, stores and construction sites, and a site for
  the agent's building plan. Agents within vision come from
  `organism_engine::perception::within` on the spatial grid. Remembered
  places stand in for goods not in sight (scored as uncertain,
  `0.25/(1 + 0.05·d)`); a remembered patch seen exhausted, a remembered
  shore that no longer touches water and an unreachable place are forgotten.
* **Scoring.** Each feasible action is an `organism_engine::utility::ActionScore`:
  the product of response-curve considerations with Dave Mark's
  compensation factor, times an action weight. The current goal gets a 15 %
  momentum bonus; `choose_best` adds ±5 % multiplicative noise (one draw per
  option). A route to the chosen target is planned with A*; if none exists
  the next best option is tried (three attempts).

| Action | Weight | Main considerations |
|---|---|---|
| eat (inventory, else the nearest stocked group store in sight or household store) | 3.0 | logistic hunger (k 12, x₀ 0.45); distance to the store |
| drink | 3.0 / 3.2 | logistic thirst (k 12, x₀ 0.55) and distance; or critical thirst (k 25, x₀ 0.85) regardless of distance |
| rest (at home when within 2·vision) | 2.0 / 1.6 | logistic fatigue (k 12, x₀ 0.6) or fear (k 10, x₀ 0.55) |
| forage / log / quarry | 1.6 / 1.3 / 1.3 | need, tile score (weight 0.5), gathering skill (weight 0.25 + 0.5·trade propensity) |
| forage or harvest without food (survival) | 2.6 | logistic hunger, tile score (0.3), distance |
| harvest a ripe crop | 2.0 | `0.5 + 0.5·relative food need`, distance |
| plant | 1.2 | patience `1 − time preference`, `0.2 + 0.8·food need`, farming skill (0.5), distance |
| craft tools (wood and stone ≥ 1, tools < 1) | 1.2 | relative tool need, patience (0.6), crafting skill (0.4) |
| build (continue own/group site, or lay out a planned one) | 1.4 | ownership or cooperation (or urgency: 1 for a first hut, patience otherwise), building skill (0.3), distance |
| trade with a perceived agent | 1.4 | meeting value `1 − e^{−Δln W/0.05}` (≥ 0.25), trade propensity, trading skill (0.3), distance |
| carry goods to the market place | 1.2 | trade propensity, goods for sale `x/(x + 4)` (≥ 2 units), distance |
| socialise with the best friend in sight | 1.0 | logistic loneliness (k 10, x₀ 0.5), cooperation (0.6), distance |
| share food with a hungry group member or relative | 1.6 | cooperation, their hunger, own surplus (0.5), distance |
| store food in the group store | 1.0 | cooperation, food above what is kept `/8` (≥ 2 units) |
| store food in the household store (own hut) | 1.0 | food above what is kept `/8` (≥ 2 units), load `1 − free/24`, distance |
| steal (adult, hunger > 0.55, food < 0.5) | 2.6 | logistic hunger (k 10, x₀ 0.65), `1 − cooperation`, risk (0.7), victim's food × (1 − affinity⁺) (0.5), convenience `d_food/(d_food + d_victim)` (1 without known food) |
| court a partner | 1.2 | mutual affinity, energy, distance |
| explore | 0.1, 2.8, 2.4 | default wandering; survival search when no shore (thirst) or, without food, no food is known (hunger) |

The **need** for a good is the largest of (a) its absolute marginal utility
`need_g = (m_g/(h_g + 1))/max_k m_k` — the marginal welfare
`∂ln W/∂h_g` divided by its largest possible value — where the holdings `h`
include the food in the agent's household store, (b) for food, the reserve
shortfall `1 − food/reserve` (reserve `2 + 6·(1 − time preference)`) and
hunger, (c) for wood and stone, the missing share of the current building
plan's cost, times a commitment of 1 for a first hut and `0.4 + 0.6·patience`
otherwise, and (d) the refill of the stock for sale,
`trade propensity · (target_g − h_g)/target_g`. Because (a) falls as
holdings grow (diminishing marginal utility), gathering saturates once an
agent holds enough; a measure relative to the currently most-wanted good
would not (some good would always score 1), which is what made agents fill
their carrying capacity in an earlier version of the model.

**Building plans** (adults): a hut while homeless; a farm plot when the agent
has none and its time preference is below 0.6; a group store when its group
has none and its cooperation exceeds 0.55.

**Exploration.** Wandering picks a random passable tile within vision. A
survival search is a correlated random walk: a tile `vision·U[1, 2]` away
within ±90° of the current heading, which covers new ground quickly.

**Movement.** Each tick is split into `⌈1/(1 − k)⌉` sub-steps so that no
sub-step covers more than one tile; in each the agent steers with
`organism_engine::steering::arrive` towards its next waypoint at speed
`1/c(current tile)` and is integrated with `Kinematics::integrate_with`
(impassable destinations are refused). Agents chasing a moving partner walk
straight at it once their route ends and it is within 4 tiles.

## Production

```text
productivity = (0.5 + skill)·(1 + tool_bonus if tools are carried)·(0.5 for children)·(0.6 + 0.4·health)
gather   1.0·productivity per tick from a tile within 1.6 tiles (fish from the shore)
harvest  3.0·productivity per tick from a ripe crop
craft    0.08·productivity tools per tick, each from 1 wood + 1 stone
build    0.06·productivity/work per tick of progress; work = 2 (hut), 3 (storage), 0.8 (farm plot)
wear     0.01 tool units per tick of gathering, farming, crafting or building
```

Several gatherers on one tile claim `request = productivity` (bounded by
their free carrying capacity) and the stock is split with
`organism_engine::intent::resolve_shared` (proportional). A farm plot and a
free building site go to one claimant (`resolve_exclusive`: priority, then
distance, then id); several group members may work on the same site.

**Farming.** Planting costs 0.5 food and stores the crop quality
`0.5 + farming skill`. Growth per tick is `fertility · crop multiplier / 96`;
at maturity the yield is `10 · fertility · quality`. A ripe crop rots after
120 ticks.

**Stores.** An agent keeps on itself its food reserve or, if larger, the
food it aims to carry for sale, and can put any surplus above that in its
group's finished store (pooling, counted as shared food) or in its own
finished hut (the household store, shared with everyone living there, e.g.
children who inherited the home). Group members eat from group stores and
residents from their hut when they carry no food. Stored food spoils at
0.05 % per tick. Household stores keep harvests and surplus from blocking the
carrying capacity that building plans need for materials.

**Buildings.** Materials are consumed when the site is laid out: hut 6 wood +
3 stone, storage 8 wood + 6 stone, farm plot 2 wood. Huts are placed on
grassland, riverbank or hills in own or unclaimed territory (1.6× within own
territory, 1.5× within 6 tiles of the remembered shore), farm plots by
fertility, stores preferably inside the group's territory. Buildings do not
block movement. A building whose owner died without children and whose group
dissolved decays over 1000 ticks.

**Skills** learn by doing: each tick the practised skill becomes
`s + λ(1 − s)` and the others `s(1 − φ)` (λ = `learning_rate`,
φ = `forgetting_rate`). With a share `f` of time on one skill the
equilibrium is `s* = fλ/(fλ + (1 − f)φ)`, so practice concentrates skills.
Founders start with uniform random skills in `[0, 0.15]`.

## Economy: welfare, barter and markets

Agents value their holdings `h` of food, wood, stone and tools with a
Cobb–Douglas welfare function (Epstein & Axtell 1996, generalised to four
goods), shifted by `ε = 1` so that empty holdings stay finite:

```text
W(h) = Π_g (h_g + 1)^{m_g/m_T},     m_T = Σ m_g
MRS_ab = (m_a/(h_a + 1)) / (m_b/(h_b + 1))      units of b the agent gives for one unit of a
```

Demand weights `m`: food 1; wood 0.15, stone 0.1, tools 0.1; +0.3 tools and
+0.1 wood and stone when tools are below 0.5; plus `0.08·cost` (hut) or
`0.05·cost` (other plans) of the current building plan.

**Barter** (Sugarscape rule T, Kehoe 2016; Mesa G1MT). For every pair of
goods in a fixed order, the agent with the higher MRS buys good `a` at
`p = √(MRS_A · MRS_B)` units of `b`: one step exchanges 0.5 units of `a` for
`0.5·p` of `b` when `p ≥ 1`, else `0.5/p` of `a` for 0.5 of `b` (scaled down
to what the parties hold). A step is executed only if **both welfares rise
and the MRSs do not cross**; steps repeat until no pair allows one or the
meeting's budget `4 + 8·mean trading skill` is used. Goods are conserved.

Welfare comparisons are made on `ln W` using only the two goods a step
changes (`Δ ln W = Σ (m_g/m_T)·ln_1p(Δh_g/(h_g + 1))`), which is exactly
equivalent to comparing `W` and better conditioned.

**Partners.** An adult considers perceived adults (affinity ≥ −0.3) whose
**trade potential** `1 − e^{−max |ln MRS_A − ln MRS_B|}` over the pairs that
admit a feasible step is at least 0.25. The MRS gap says nothing about how
much a meeting is worth (two agents holding crumbs can have a large gap), so
for the four best candidates (gap discounted by distance) the meeting is
simulated on copies of both holdings and valued by the initiator's **expected
welfare gain** `Δ ln W`, as `1 − e^{−Δ ln W/0.05}`; the agent walks to the
partner with the best value (discounted by distance) if it is at least 0.25
(a gain of about 1.4 %). When the initiator reaches its partner (≤ 2 tiles)
and the partner is willing (trade propensity ≥ 0.1, affinity ≥ −0.3), pairs
are matched one-to-one by a greedy maximal matching over potential (ties by
ids). Partners also exchange knowledge of places.

**Learned demand and markets.** After a trade, each net seller of a good
raises its stock for sale of that good by the amount sold (at most 12, fading
3 % per day) and both remember the place as their market. The targets for
the bulk goods (food, wood, stone) are scaled down together to fill at most
half the carrying capacity, so frequent traders, whose learned demand
saturates for every good, still have room to work. Agents then gather
to refill that stock (specialising in what others buy) and, with goods for
sale and nobody to trade with in sight, carry them to their market place.
Markets are therefore emergent meeting points, and the trips to and from them
are what turns into trade routes.

## Society: affinity, groups and sharing

Affinity changes (each agent remembers at most `max_relationships` others):

| Event | Change |
|---|---|
| within 2 tiles of someone, per tick | +0.002·own cooperation |
| socialising, per tick (both) | +0.04·cᵢ·cⱼ |
| successful trade | +0.04·own cooperation |
| gift of food | receiver +0.15, giver +0.05 |
| theft | victim −0.5, thief −0.1, witnessing members of the victim's group −0.2 |
| fight | both −0.3 |
| birth | parents ↔ child +0.6 |
| every day | ×0.98; dead agents and near-zero entries are forgotten |

**Groups.** Every day the bond graph links two agents whose mutual affinity
`min(aᵢⱼ, aⱼᵢ)` reaches `bond_threshold` (half of it if they already share a
group, a hysteresis against flicker). Connected components (union–find) with
at least 3 members are groups. Identity is carried by greedy maximum overlap
with the previous groups (ties to the lower id), so a group keeps its id
when it grows, the larger part keeps it when it splits, and merging groups
keep the best-overlapping id. New groups get the next id and the colour
`hsv(frac(id·0.618…), 0.65, 0.92)`. Formation, splits and dissolutions are
logged as events.

**Sharing.** A cooperative agent with more than half its reserve gives
`min(3, food − reserve/2)` to a hungry (hunger > 0.5) group member or relative
holding less than one unit; agents deposit surplus food in their group's
store (see [Stores](#production)), and group members eat from it (claims
split proportionally).

## Conflict

* **Theft.** A hungry adult (hunger > 0.55, food < 0.5) may target a non-kin
  agent in sight holding at least 1.5 food (a member of its own group only if
  its cooperation is below 0.5); stealing competes with foraging and wins
  mostly for uncooperative, risk-tolerant agents when a victim is closer than
  any known food. Within reach, thieves claim victims
  (`resolve_exclusive`, priority = hunger). Adult victims resist with
  probability `0.2 + 0.6·risk`; an unresisted or won theft takes up to 3 food.
* **Territorial defence.** Each adult member of the owning group within 5
  tiles of someone gathering in its territory confronts the trespasser with
  probability `0.15·(1 − cooperation)·(0.3 + 0.7·risk)` per tick (×1.5 on
  contested tiles); one confrontation per trespasser and tick. A beaten
  trespasser drops what it gathered there (back into the tile's stock).
* **Fights** use a Tullock contest success function:
  `P(a wins) = s_a/(s_a + s_b)` with
  `s = (0.5 + health)(0.5 + energy)(1.3 if carrying tools)(0.4 for children)`.
  The loser loses 0.3 health and gains fear, the winner loses 0.08.

## Territory

Every day, each finished building of a group is a centre of strength
`S = 4 · w · (1 + 0.25·ln members)` (w: hut 1, storage 1.6, farm plot 0.6)
with influence `S − falloff·d` on tiles at Euclidean distance `d` — an
XTENT-style rule (Renfrew & Level 1979). A group's influence on a tile is the
maximum over its buildings; the tile belongs to the group with the largest
positive influence and is **contested** when a second group's influence is
positive and within 1. The online top-2 computation is exact because maxima
only grow. Territory affects work-place choice (foreign tiles deter
risk-averse agents), fear, building placement and territorial defence.

## Births, deaths and inheritance

An adult courting a partner of the other sex within 2 tiles has a child with
it when both are fertile, have energy ≥ 0.6, carry at least 3 food, have mutual
affinity ≥ 0.2, are not parent and child, and had no child for 2 years, and
the population is below `max_population`. Pairs are matched one-to-one by
mutual affinity. Each parent gives 3 food (Sugarscape rule S: parents endow
the child). The child (sex 50/50, lifespan 55–75 years) inherits the parents'
mean preferences plus N(0, 0.05) noise per trait, its mother's group, home,
population and memory of places, and starts with affinity 0.6 towards both
parents. Children work at half productivity and do not build, trade, court,
steal, resist thieves or defend territory until they are 14 (they can still
be confronted as trespassers, with strength ×0.4).

When an agent dies its goods are split equally among its living children and
its buildings pass to the eldest living child (Sugarscape rule I); without
heirs they stay with its group.

## One tick and determinism

```text
daily     (every 24 ticks) relationship decay, groups, building groups, territory
index     spatial grid of agent positions (slot = store index)
decide    agents due for a decision choose a goal (utility AI, A*)
move      sub-stepped steering along routes; footfall builds trails
act       eat, drink, rest, craft, build, store; claims on tiles, plots, sites, stores resolved
interact  barter, sharing, socialising, familiarity, theft, territorial defence, births
life      needs, energy, health, skills, ageing; deaths and births committed; inheritance
world     crop growth, store spoilage, building decay, growback, trail and danger decay
```

* Agents are processed in id order (`organism_engine::AgentStore`); every
  random number comes from one `SimRng` stream in the state (terrain uses
  derived streams), drawn in that fixed order.
* Contested resources are resolved by `resolve_shared` / `resolve_exclusive`,
  whose orders are total; mutual interactions by a greedy matching sorted by
  score and ids. No hash maps are used.
* Snapshots (`schema_version` 2; version 1 snapshots are rejected because
  the agent layout changed) contain the scenario, the configured rules,
  the seed and the full state including the generator, the event log and the
  learned memories; a restored run replays bit-identically. Restored data is
  validated (scenario, seed range, layer sizes, id orderings, tile
  references, both rule sets' ranges, finiteness).
* Results are reproducible on the same platform and build; float
  transcendental functions (`ln`, `exp`, `sin`, `cos`) come from the
  platform's libm, so cross-platform bit-exactness is not promised.

## Parameters

All rule parameters apply at the next tick; they are kept across `reset()`.

| Key | Range | Default | Meaning |
|---|---|---|---|
| `regen_berries`, `regen_fish`, `regen_wood`, `regen_stone` | 0–5 | 1 | growback multiplier per resource |
| `crop_growth` | 0–5 | 1 | crop growth multiplier |
| `metabolism` | 0.001–0.05 | 0.006 | energy burned per tick at rest |
| `thirst_rate` | 0.001–0.05 | 0.012 | thirst per tick |
| `fatigue_rate` | 0.001–0.05 | 0.008 | fatigue per tick of activity |
| `vision` | 2–32 | 10 | perception radius (tiles) |
| `trade_enabled` | bool | true | barter on/off |
| `tool_bonus` | 0–3 | 0.8 | productivity bonus of tools |
| `learning_rate` | 0–0.1 | 0.01 | λ |
| `forgetting_rate` | 0–0.01 | 0.0005 | φ |
| `road_decay_ticks` | 10–20000 | 600 | trail decay time `T` |
| `road_discount` | 0–0.9 | 0.6 | maximum cost reduction `k` |
| `road_threshold` | 0.01–1 | 0.2 | intensity counted as a road |
| `conflict_enabled` | bool | true | theft and fights on/off |
| `births_enabled` | bool | true | births on/off |
| `bond_threshold` | 0.05–1 | 0.3 | mutual affinity for group bonds |
| `max_relationships` | 2–64 | 12 | relationship memory |
| `territory_falloff` | 0.1–5 | 1 | influence lost per tile |
| `max_population` | 1–5000 | 600 | no births at or above |
| `seed` | 0–2⁶³−1 | scenario | terrain, founders and behaviour; **requires reset** |

Scenario files may override any rule parameter in `params`.

## Metrics

| Key | Kind | Meaning |
|---|---|---|
| `population`, `groups` | gauge | living agents; groups |
| `grouped` | ratio | share of agents in a group |
| `gini_wealth` | ratio | Gini of wealth = carried food + wood + stone + 2·tools + food in the huts the agent owns (`analytics::gini`) |
| `trade_volume`, `trades` | counter | units exchanged; meetings with ≥ 1 exchange |
| `specialization` | ratio | mean over adults that produced something of `½[(1 − H(p)/ln 6) + (1 − H(q)/ln 5)]`, `p` = shares of production ticks spent foraging, logging, quarrying, farming, crafting and building (moving average), `q` = normalised skills |
| `territory_per_group`, `contested_tiles` | gauge | mean claimed tiles per group; contested claimed tiles |
| `road_length` | gauge | tiles with trail intensity ≥ road threshold |
| `food_stock`, `wild_food` | gauge | food carried + in group and household stores; berries + fish on the map |
| `conflicts`, `thefts`, `shared_food` | counter | fights; successful thefts; food given or deposited |
| `births`, `deaths`, `deaths_starvation`, `deaths_thirst`, `deaths_conflict`, `deaths_age` | counter | demography by cause |
| `buildings`, `farms` | gauge | finished buildings; finished farm plots |
| `mean_energy`, `mean_skill` | ratio | averages over agents |

## Commands and views

`CivCommand` (all validated, errors as `SimError::InvalidCommand`):

| Command | Effect |
|---|---|
| `SpawnAgents { x, y, count, population }` | 1–200 adults of a scenario population on passable tiles within 3 tiles (they know the nearest shore) |
| `AddResources { x, y, radius, resource }` | tiles of the disc without buildings become the resource's terrain with a full stock (no water under agents); one batch edit (shores refreshed, land regions relabelled once) |
| `RemoveResources { x, y, radius, resource }` | empties the stock of one or all resources in the disc |
| `Disaster { x, y, radius }` | destroys buildings (stores, crops) in the disc, adds danger and recomputes territory at once |
| `SetRegeneration { resource, multiplier }` | growback multiplier 0–5 (same as the `regen_*` / `crop_growth` parameters) |

`CivViewRequest { region, tiles, roads, territory }` selects a clipped tile
rectangle and the grids to include. `CivView` holds row-major grids over the
region — terrain, resource level `stock/capacity`, trail intensity (with the
road threshold), territory owner id (0 = none) and contested flags — plus
agents (id, position, heading, group, activity, carried goods, energy,
health, age, sex, population, adulthood), buildings (kind, tile, group,
progress, crop growth, stored food) and all groups (id, colour, members,
territory, contested tiles).

## Scenarios

Scenario files live in `scenarios/civilization/*.ron` and are embedded in the
crate (`scenarios::builtin()`); `scenarios::load_dir(path)` reads a folder
(sorted by file name). A file is a `Scenario`:

```ron
(
    id: "abundant", name: "Abundant resources", description: "...", seed: 11,
    world: (
        terrain: Generated(width: 64, height: 64, water: 0.10, mountains: 0.03, hills: 0.07,
                           forest: 0.18, berries: 0.14, feature_size: 14.0),   // zones: Split(strength: 0.9)
        richness: 1.5,
    ),
    populations: [(label: "Settlers", count: 40,
                   preferences: (risk: (mean: 0.4, sd: 0.15), cooperation: (mean: 0.6, sd: 0.15),
                                 trade: (mean: 0.5, sd: 0.2), time: (mean: 0.4, sd: 0.15)),
                   region: (x0: 0.0, y0: 0.0, x1: 0.4, y1: 1.0),   // optional
                   kin_affinity: 0.5, starting_food: 6.0)],         // optional
    params: { "regen_berries": 0.6 },                                // optional overrides
)
```

| Preset | Set-up |
|---|---|
| `abundant` | rich mixed 64×64 map (richness 1.5, 14 % berries, 10 % water), 40 settlers with varied preferences (cooperation 0.55 ± 0.25, risk 0.45 ± 0.2) |
| `scarcity` | the same settlers, seed and shares of water, forest and hills, but richness 0.5, 2.5 % berries and berry, fish and crop growth ×0.6 |
| `cooperation_vs_competition` | 30 cooperators (cooperation 0.85, risk 0.3) and 30 competitors (0.15, 0.75) mixed in one valley |
| `two_strategies` | patient farmers (time preference 0.15) in the west, impatient foragers-traders (trade 0.85) in the east |
| `territorial_expansion` | three founding bands (kin affinity 0.5) in separate corners of a 96×96 map |
| `trade_routes` | timber and stone strictly in the west, berries in the east (`Split(strength: 2.0)`), few lakes (3 % water, so fish cannot replace berries everywhere), 50 eager traders |

Outcomes after 3000 ticks (125 days; release build, single seeded runs,
measured with the code in this repository):

| Preset | Pop. (start → end) | Groups | Grouped | Gini | Specialisation | Roads | Births | Deaths: starvation / thirst / conflict | Thefts | Fights | Trade vol./tick | Food stock |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| abundant | 40 → 132 | 4 | 0.98 | 0.72 | 0.38 | 265 | 92 | 0 / 0 / 0 | 0 | 43 | 12.0 | 12 098 |
| scarcity | 40 → 70 | 2 | 0.97 | 0.39 | 0.25 | 243 | 46 | 11 / 4 / 1 | 0 | 53 | 4.0 | 533 |
| cooperation_vs_competition | 60 → 119 | 5 | 0.93 | 0.53 | 0.34 | 351 | 59 | 0 / 0 / 0 | 0 | 135 | 8.6 | 3 571 |
| two_strategies | 50 → 108 | 3 | 0.99 | 0.62 | 0.41 | 261 | 58 | 0 / 0 / 0 | 0 | 114 | 8.0 | 5 141 |
| territorial_expansion | 45 → 136 | 4 | 0.99 | 0.61 | 0.33 | 297 | 91 | 0 / 0 / 0 | 0 | 75 | 10.3 | 6 942 |
| trade_routes | 50 → 69 | 1 | 0.97 | 0.54 | 0.37 | 114 | 25 | 3 / 2 / 1 | 0 | 78 | 5.9 | 1 288 |

("Fights" include territorial confrontations, which occur wherever group
territories meet. The large food stocks of the rich presets are farm surplus
in household and group stores; see the limitations.)

Controlled comparisons (same scenario and seed, one switch changed):

* **Food scarcity.** `scarcity` against `abundant` after 2500 ticks: fewer
  people and births, more starvation deaths (starvation, not thirst, is the
  main cause), much smaller food stocks (behaviour test).
* **Gains from trade.** `trade_routes` with barter on and off for 1500
  ticks, seeds 67–71 (67 is the preset): deaths 5/13, 8/16, 0/7, 8/13, 12/24
  (with/without barter) and specialisation 0.47/0.28, 0.40/0.33, 0.42/0.29,
  0.41/0.43, 0.44/0.21 — barter saves lives in every seed and raises the
  division of labour in four of five. Without barter roads are *longer* in
  four of five seeds (175–230 vs 95–138 road tiles; seed 67: 114 vs 124):
  everyone then fetches both kinds of goods personally, and trade
  substitutes for travel.
* **Strategies** (3000 ticks): in `cooperation_vs_competition` all 71
  cooperators belong to groups but only 40 of 48 competitors do, while the
  competitors, who pool less, hold slightly more wealth per head (34 vs 32);
  in `two_strategies` the patient farmers grow from 25 to 77 and hold 53
  units of wealth per head, the impatient foragers-traders reach 31 with 14.
  Theft is rare in all presets (none in these runs): agents seldom become
  hungry with nothing to eat once they keep reserves and household stores.
* **Trade elsewhere** (2000 ticks): in `abundant` barter raises the
  population (101 vs 67); in `two_strategies` it slightly lowers it (90 vs
  103), because time spent travelling to partners is time not spent
  socialising and courting.

## Approximations and limitations

* **Compressed demography** (240-tick years) while physiology runs in hours.
* **Information.** Agents see all tiles and agents within `vision` (no line of
  sight) and know the inventories and demand weights of agents in sight when
  estimating trade potential. Memory holds one place per resource kind.
  Agents know which land region they are in (reachability is perfect
  knowledge).
* **Utility weights and curves** are design values chosen so that the
  presets show plausible behaviour; they are not calibrated to data.
* **No value of leisure.** Gathering saturates through diminishing marginal
  utility, but harvesting a ripe crop always scores at least half (it would
  rot) and planting keeps a floor of 0.2 on its food-need factor, so farmers
  keep farming and rich presets accumulate large food stocks in stores,
  bounded only by spoilage (0.05 %/tick). Production is fast relative to
  consumption (about one hour of gathering covers a day's food), so agents
  spend much of their time resting, trading and socialising.
* **Trade decisions** value a meeting by a simulated barter for at most four
  candidates (preselected by MRS gap and distance); travel time is only a
  distance discount, so traders can still spend time that would have gone to
  socialising (see the `two_strategies` comparison).
* **Welfare** uses a subsistence shift `ε = 1` (a Stone–Geary-like form) and
  continuous goods in steps of 0.5 units; Mesa's G1MT truncates to integers.
  Goods are traded at bilateral prices only; there are no money, credit
  (Sugarscape L), culture tags (K) or disease (E).
* **Territory** uses Euclidean distance instead of path cost and is
  recomputed once a day; groups too.
* **Movement.** Buildings do not block movement; agents may overlap; A*
  gives up after 4000 expansions (far or unreachable targets fall back to the
  next option); straight-line chasing only within 4 tiles.
* **Biology.** No pregnancy, child care or diseases; sex ratio 50/50; children
  inherit preferences as a parental average plus noise (cultural and genetic
  transmission lumped together).
* **Fights** are single contests with fixed damage; there is no group
  warfare, alliances or weapons beyond tools.
* **Specialisation index** is a descriptive statistic of activity shares and
  skills; it is not a measure of a division of labour across a market.
* **Seasons and weather** are not modelled (the world is self-contained and
  does not use `physics-world`).
* **Carrying capacity** bounds gathering and harvesting, but barter, gifts,
  theft and inheritance can leave an agent above 24 units until it eats,
  stores or sells.
* Results are deterministic on the same platform and build; see
  [determinism](#one-tick-and-determinism).

## How it was verified

`cargo test -p civilization` (dev profile, which the workspace builds with
`opt-level = 1`):

* **Unit tests (44)**:
  * world: value-noise range, continuity and seeding; exact terrain
    quantiles, riverbanks touching water, split zones, map errors with
    row/column; growback, trail saturation and decay, shores, region labels
    following 4-connectivity and updating when terrain changes, batch terrain
    edits equal to one-by-one edits;
  * routing: valid routes, **A* route costs equal to a reference Dijkstra**
    over many start/goal pairs on mixed terrain with worn roads, roads
    attracting routes, expansion budget;
  * economy: Sugarscape MRS formula; **barter raises both welfares at every
    step, conserves goods, never crosses MRSs and stops when MRSs converge**;
    `ln W` and its two-good increments equal `ln` of the welfare; the
    expected gain of a meeting is small for crumbs and large for real
    surpluses; marginal need is absolute and saturates;
  * agents and society: skill learning/decay closed form; bounded
    relationships; inheritance of preferences; demand from plans; group
    components, hysteresis, identity on split; territory top-2; matching
    order independence;
  * actions: household stores accept surplus only from residents and keep
    food for sale in the pocket, sale targets fit half the capacity; residents
    eat from their hut, strangers cannot; drinking only works at a shore and a
    dry "shore" is forgotten; theft from a child moves 3 food and the
    affinity penalties; fights are won with probability `s_a/(s_a + s_b)`;
  * configuration: parameter specs round trip and validation; scenario
    parsing errors with position; folder files equal the embedded copies;
    metric definitions.
* **Behaviour tests (9)**: roads form on a forced corridor between water and
  food (corridor intensity > 0.3, > 3× elsewhere, median above the 90th
  percentile elsewhere, a continuous road); scarcity lowers population,
  births and food stocks relative to abundance, with more starvation deaths
  that outnumber thirst deaths; groups emerge and cooperators are grouped
  more than competitors; the conflict switch removes all thefts and fights;
  agents without water die of thirst (and only of thirst); identical seeds
  give identical snapshots and different seeds differ; snapshot round trip
  into another instance replays bit-identically (and foreign, truncated or
  newer snapshots are rejected); **barter between separated regions saves
  lives and raises specialisation** (same seed, barter on vs off); surplus
  food goes home so that group stores get built and at most a quarter of the
  adults have full loads (regression test for the capacity deadlock below).
* **Interface tests (12)**: spawning and its validation, resource commands,
  large area edits matching a fresh region labelling, regeneration
  command/parameter agreement and persistence across reset, the seed
  requiring a reset, `reseed` keeping the seed representable, older snapshot
  schemas rejected, disasters removing buildings and homes and updating
  territory at once, every claimed tile explained by a building of its owner
  in range, development of huts, farms, tools, skills and children, view
  regions and clipping, descriptor parameters and metrics matching the
  published values.

`cargo clippy -p civilization --all-targets -- -D warnings`, `cargo fmt -p
civilization -- --check` and `RUSTDOCFLAGS='-D warnings' cargo doc -p
civilization --no-deps` are clean.

### Review findings fixed

An adversarial review (probing every preset for 3000 ticks with temporary
measurement harnesses) found and fixed:

* **Capacity deadlock.** Harvests and food gathered for sale filled the
  24-unit load; with no place to put food, agents could not gather the
  materials of their plans. In `abundant` no group store was ever built
  (16–18 of 40 adults kept an unfulfilled storage plan, 32–37 had full
  loads) and in `two_strategies` 21 adults stayed homeless for 3000 ticks.
  Fixed with household stores in huts.
* **Insatiable gathering.** The gathering need was the marginal-utility
  share relative to the most-wanted good, which never falls; once capacity
  was freed this produced unbounded hoarding. Replaced by the absolute,
  diminishing marginal utility (counting the household store), and planting
  now weighs the food need.
* **Store/sell loop.** Food gathered for sale was immediately deposited;
  agents now keep their food for sale, and bulk sale targets fit half the
  carrying capacity (learned demand had saturated at 12 units of every good).
* **Trade crowding out production.** Partners were chosen by MRS gap alone,
  so agents walked to trade crumbs: in `trade_routes`, a third of all time
  was spent chasing partners and the population ended 25 % lower with barter
  than without. Meetings are now valued by their simulated welfare gain.
* **Specialisation index blind to the division of labour.** It counted one
  "gathering" activity for food, timber and stone and was dominated by
  one-tick barter meetings; it now uses six kinds of production.
* **Confounded scarcity preset and metric.** `scarcity` also halved the water,
  so nearly half its "starvation" deaths (which included thirst) were
  thirst. Water, forest and hill shares now match `abundant`, and thirst has
  its own metric.
* **Trade preset without separated food.** Fish and farms fed everyone
  everywhere, so barter brought no gains; the preset now separates food from
  materials (strength 2, 3 % water) and the gains from trade are tested.
* **Correctness and robustness.** Drinking was possible at a remembered
  "shore" that no longer touched water after a terrain edit; area edits
  relabelled land regions once per tile (`O(r²·n)`, seconds on large maps);
  disasters left territory stale until the next day; `reseed` accepted seeds
  the `seed` parameter cannot represent; restored snapshots were not checked
  for a valid scenario, seed and configured rules; schema-1 snapshots would
  have been decoded with the new layout.

## Performance

Criterion (`cargo bench -p civilization`), Apple M2 Max (12 cores), measured
while the machine was loaded by parallel builds of other crates (load average
27–41 during the run), so these are upper bounds:

| Benchmark | Time (95 % interval) |
|---|---|
| `step/abundant` (one tick, 52 agents after 500 warm-up ticks, 64×64) | 346 µs (308–403 µs) |
| `step/trade_routes` (one tick, 52 agents after 500 ticks, 80×48) | 577 µs (574–581 µs) |
| `command/add_resources_r30_96x96` (radius-30 area edit, 2821 tiles) | 335 µs (334–336 µs) for the command alone (batch terrain change, shores, one region relabelling) |
| `build/territorial_expansion` (generate a 96×96 world and 45 founders) | 1.62 ms (1.61–1.62 ms) |

Over 3000-tick runs of the presets (release build, same machine) the mean
cost was 0.19–0.39 ms per tick with up to 136 agents. Most of it is decision
making: the tile scan of `vision²` tiles, A* per decision and, since the
review, simulating the barter of up to four candidate partners (which made
a tick of the 52-agent states about twice as expensive as before, 118 → 346 µs
and 329 → 577 µs, measured under different loads). Connected-region labels
make routes to unreachable places fail in O(1). Area edits relabel land
regions once per command: the radius-30 edit above took 185 ms when every
tile was relabelled separately and 0.57 ms as one batch (`World::set_terrains`,
measured directly on the world), and the old cost grows as `O(area · tiles)`
(seconds on a 512×512 map).

## References

* Epstein, J. M. & Axtell, R. (1996). *Growing Artificial Societies: Social
  Science from the Bottom Up*. Brookings / MIT Press.
* Kehoe, J. (2016). *The Specification of Sugarscape*. arXiv:1505.06012.
* Mesa examples, *Sugarscape G1MT* (constant growback, MRS trade):
  https://mesa.readthedocs.io/stable/examples/advanced/sugarscape_g1mt.html
* Helbing, D., Keltsch, J. & Molnár, P. (1997). Modelling the evolution of
  human trail systems. *Nature* 388, 47–50.
* Grassé, P.-P. (1959). La reconstruction du nid et les coordinations
  interindividuelles chez *Bellicositermes*. *Insectes Sociaux* 6, 41–80
  (stigmergy).
* Renfrew, C. & Level, E. V. (1979). Exploring dominance: predicting
  polities from centers. In *Transformations: Mathematical Approaches to
  Culture Change*, Academic Press (XTENT).
* Tullock, G. (1980). Efficient rent seeking. In *Toward a Theory of the
  Rent-Seeking Society*, Texas A&M University Press (contest success function).
* Arrow, K. J. (1962). The economic implications of learning by doing.
  *Review of Economic Studies* 29, 155–173.
* Mark, D. (2009). *Behavioral Mathematics for Game AI*. Course Technology;
  Infinite Axis Utility System, GDC AI Summit talks (2013, 2015).
* Reynolds, C. W. (1999). Steering behaviors for autonomous characters. GDC.
* Hart, P. E., Nilsson, N. J. & Raphael, B. (1968). A formal basis for the
  heuristic determination of minimum cost paths. *IEEE Trans. SSC* 4, 100–107.
* Research notes: `docs/research/ecology-alife.md` §7 (agent AI, deterministic
  conflict resolution) and §9 (Sugarscape, cooperation, trade routes,
  territory).
