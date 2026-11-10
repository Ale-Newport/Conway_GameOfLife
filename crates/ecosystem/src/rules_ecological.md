### Ecosystem (ecological rules)

One tick lasts `1440 / ticks_per_day` minutes (default 6). Physiological rates are per simulated day; life-history
durations (lifespan, maturity, gestation, incubation, tree growth per year) are compressed by `days_per_year / 365`.
Masses: plants, litter and fungi in kg dry matter per m², animals in kg wet mass; energy in kJ (plants, litter and fungi
18 kJ/g, animal tissue 7 kJ/g); nutrients in g of the world's generic mineral nutrient (animal tissue 30 g/kg).

**Coupling with the world.** Before the world steps, fuel = Σ flammability·vegetation + litter + 0.3·dead wood + tree
foliage × flammability, and canopy = `1 − e^{−0.5·LAI_trees}` are written into it. After the step, the fuel consumed in
each cell removes the same fraction of every fuel component; burned energy is combustion, `volatilisation_fraction` of
its nutrients is lost and the rest returns as ash. Nutrients leached by the world's percolating water leave the
ecosystem; with `leaching_loss` or `external_fluxes` off they are returned to the soil of the cells that lost them
(weights: each cell's decrease of mineral + organic nutrients during the world step). Trees in burning cells die with probability
`1 − exp(−2 I (1 − bark) / (1 + H/10))` (I = fuel consumed, kg/m²); resprouters lose their stem instead. Tornado damage
`d` flattens vegetation (× layer height ≤ 1 m) into litter and throws trees with probability `d (0.3 + 0.7 H/H_max)`.

**Vegetation (hourly, per cell and layer, tallest layer first).** Light under the layers above:
`I = I_ground e^{−0.5 Σ LAI}`; growth `G = r B (1 − B/K_biome) · min(1, I/L_sat)/0.4 · f_T · min(f_W, f_N) · Δt`,
`f_T` a beta function, `f_W` a ramp from wilting to optimal soil moisture, `f_N = N/(K_N + N)`, for mycorrhizal layers
times the boost `1 + b·M/(M + K_M)` (capped at 1); water cells start with `soil.water_nutrients_g_m2` of dissolved
nutrients; growth takes `G · n_layer` from the soil and is scaled down when the soil holds
less. While `B < K/2`, recruitment from the seed bank adds `seedling · germination · bank · f_T · f_W · Δt`. Losses
`B (turnover + cold + drought + flood dieback) Δt` go to litter (at least 0.2/day outside the layer's habitat).
Daily, seed banks gain `B/K` (flowers × (0.3 + 0.7 pollination)), decay 2 %/day and spread to the 8 neighbours.

**Trees (daily).** Height growth `ΔH = g/days_per_year · (1 − H/H_max) · min(1, I/(0.1 + 0.6(1 − shade_tol))) · f_T ·
min(f_W, f_N)` with `f_N = min(1, N/(2 + N) · boost)`, wood `B_max (H/H_max)^{2.5}`, leaves `LAI·πr²/SLA` × phenology (deciduous: 0 in winter); the light `I`
at a crown is Beer–Lambert through taller crowns sharing its footprint. Shade, drought and frost reduce health; trees
die by Gompertz ageing or exhausted health, stand as snags for 60 days and fall into the dead-wood pool. Mature,
healthy trees in their seeding season release Poisson(seeds/day) seeds at kernel distances (exponential or 2Dt); a
seed establishes on dry land of a listed biome below the per-cell tree cap with probability
`0.05 · f_light · f_W · (1 − 0.5 min(grass/0.5 kg m⁻², 1)) · max(f_T, 0.2)` (ground light under the canopy), if the
soil holds the nutrients of the seedling's tissue.

**Fungi and decomposition (hourly).** `f = Q10^{(T−20)/10} · hump(moisture)`, `sat = F/(F + K_F)`; litter decays at
`k_L (m + (1 − m) sat) f` (in water: bacterial decay `k_aq · Q10`), dead wood at `k_W sat f`. Fungi grow by `yield ×`
their share; a fraction `h` is humified into soil organic matter (none at sea: marine detritus is remineralised); free nutrients are mineralised, or soil nutrients immobilised when fungi need more. Mycelium dies
into litter, fruits mushrooms when moist and mild, and spreads to its 4 neighbours (conservative exchange). Mycorrhiza
grow logistically on host foliage with nutrients from the soil. Carcasses decay at 0.15/day × Q10 into litter.

**Animals (every tick).** Perception: animals within the vision radius and field of view (× (1 − 0.5 cover) of the
target cell; × (0.5 + 0.5 vigilance) while foraging), heard within the hearing radius when moving, or smelled within
the smell radius (moving or not); burrowed animals are invisible. Forage counts as plentiful at 0.15 kg/m² above the
grazing residual (aquatic layers: `min(0.15, ½ K_max)`; density-field prey: 1 g/m²); aquatic field prey (sticklebacks)
is reachable by swimmers anywhere and by others only in water ≤ 2 × their wading depth (≥ 0.3 m). Carcasses count
only when the straight way to them is passable. Utility AI (`organism-engine` compensated products × species weights; momentum 1.15; 5 % noise):
forage, hunt, scavenge, flee (predators, fire, deep water), drink (water remembered within `max(3 vision, 12 cells, 3 h of cruising)`; beyond
it, an animal with thirst > 0.3 searches: towards the lowest passable of 8 points clamp(vision, 3, 10) cells away
if it is lower, else straight ahead), rest, sleep (circadian by activity pattern; acute thirst `u = thirst_curve(thirst)` multiplies it by `1 − u²`),
seek mate (without a mate in sight, species spawning in water head for water), wander (with nothing to eat in sight,
at least `½ max(hunt, forage, scavenge)` × hunger: searching at cruise speed), follow mother, guard eggs, defend the
territory, migrate, hibernate. From fire, burrowers hide in a burrow within two sprints; others walk away at cruise
speed and, within 3 cells, sprint to the nearest burned-out or unburnable passable cell within `clamp(vision, 2, 12)`,
through the front if need be. A forager eats in the cell it heads for once that cell's centre is within 1 cell. Founders of social species
start in groups (sizes from `group_size`, or a scenario's `groups`); packs and pairs are founded by an adult female and
an adult male. Herds, flocks and schools steer with boids cohesion
and alignment towards their 8 nearest perceived conspecifics (wandering, foraging at ½ weight, migrating); separation
keeps three body lengths. Pack leaders (adults first, then mass × health) lead travel (members farther than ¼ of
their vision regroup: wander score up to `cohesion` at one vision, at cruise speed), their hunting target is joined
by members that hear them (prey value ≥ 0.8, hunger at ½ weight) and they eat first at carcasses; only the dominant
adult female and male of a pack breed (reproductive suppression). A territorial adult
chases an adult conspecific of another group (or, without groups, of its sex) inside its territory (`defend`:
aggression × closeness to the centre × health × (1 − injury)); on contact
`P(owner wins) = σ(2 ln(S_o/S_i) + 0.5)`, `S = (aggression + 0.2) m^{1/3} health (1 − ½ injury)`; the loser gains
0.15–0.3 injury and loses 0.05 health; a chased intruder flees. Calling species that flee a predator call with
strength 1; a call of strength `s` heard at `d` is a threat of level `0.7 s (1 − ½ d/hearing)`, relayed while fleeing
if ≥ 0.2. Migrants whose daily mean temperature is > 1 °C outside their preferred range head for the best of 8
points `migration_range` away (≥ 0.5 °C better; score `weight` × compensated product of `discomfort/10` and
`1 − hunger` at ½ weight). Hibernators
enter dormancy outside summer (autumn, winter, a still-cold spring) below `below_c` with reserves ≥ the threshold and
hydration ≥ ½ (score 2.5; torpid ectotherms meeting these conditions go dormant) and stay until the daily mean exceeds
`wake_above_c`, fire or flood threatens them, or hydration falls below ¼ (arousal to drink). Nest predators raid other species' clutches they find
within `max(vision, hearing, smell) · (1 − ½ cover)` (`max(forage, scavenge) · (½ + ½ egg preference)`). Territorial adults keep a territory centre (burrow, founding place, or where they
matured); beyond `territory_radius` their wandering is pulled back towards it with a weight ramping from 0 at `R` to
1 at `2R`. Steering: obstacle avoidance, then the action,
then separation; movement in sub-steps of ≤ ½ cell with species passability (water depth, slope; aquatic species
only in water of their kind — fresh or salt — and minimum depth; fliers anywhere). Fliers climb towards their cruising
altitude while travelling (wander, migrate, flee, defend, search) by `max(5 m, ½ cruise)` per tick, land for every
other action and stoop when they sprint at prey; ground hunters cannot catch prey more than 1 m up, non-swimmers
cannot reach fish deeper than their wading depth (at least 0.3 m). Aquatic animals and swimming ectotherms feel at
least 4 °C in water; animals in shelter (hidden, dormant, torpid) feel the daily mean, at least 0 °C.
Speeds: sprint (Hirt et al. 2017 unless given) × `motion_seconds / cell_size` cells per tick; stamina drains
`1/endurance` per sprint tick.

**Interactions.** Attacks within half a tick of sprinting: one per prey (best claim; a failed attack adds its injury
to the prey's `injury` and the predator's), success with
`logit p = 2.5 (v_p − v_q)/max(v_p, v_q) + 1.2 ln(A/D) − d/strike − 1.5 cover + (1 − health_q) + ln(efficiency)`.
Carcass meat: killer first, then by dominance `class + aggression · m^{1/3}/(1 + m^{1/3})` (class 1 for hunters of
animals, 0 otherwise; + ½ for a pack leader; ordered sharing); eggs: ordered by hunger; plants, litter, nectar (1 %
of flower biomass), mushrooms, browse and density-field prey: proportional sharing per cell. Mating: one male per
receptive female (in season, energy ≥ threshold). Territorial fights: one owner per intruder.

**Physiology.** `B = 0.78 c_th M^{3/4} e^{−0.63 eV/k (1/T − 1/293 K)}` W; cost = B × activity (+ thermoregulation,
+ `10.7 M^{0.68}` J/m of locomotion × speed gene², × 0.35 airborne, × 0.12 swimming); reserve capacity = fasting days × field metabolism. Meals:
assimilated `a·E` to reserves, `a·N` to the store (surplus excreted), `(1 − a)` egested as dung. Hydration falls by
`1/dehydration_days` per day (× `1 + 0.03 (T − 25 °C)⁺`), drinking restores 0.25 per tick and food water `0.1–1` of the
daily need per day of food × the species' `food_water` (reptiles 2); empty reserves, thirst, cold/heat beyond tolerance, deep water and fire reduce health;
death at health 0 or by the Gompertz hazard (1 % reach the lifespan). Dormant animals pay `metabolic_fraction` of
basal metabolism, no thermoregulation, 5 % of the water loss and no exposure. Injuries slow an animal by
`1 − 0.6 injury` and heal by 0.2/day (0.4 at rest) while it is fed and watered. Juveniles grow `a m^{3/4} (1 − (m/M)^{1/4})`.
Pregnancy accrues `litter × (birth tissue + ½ reserve)` from the mother; births (or a clutch of eggs that hatches after
incubation) when due; a bird that loses its clutch may lay a replacement clutch; mothers feed dependent young. Offspring genomes: uniform crossover + Gaussian mutation
(`mutation_rate`, reflection). Dead animals become carcasses.

**Disease (SIR).** A susceptible host with `k` infectious hosts of a pathogen within its contact distance is infected
with `P = 1 − (1 − β)^{k Δt_h}`; infected hosts recover with `1 − e^{−Δt_d/infectious_days}` (immune for
`immunity_days`, 0 = life), die with `1 − e^{−μ Δt_d}`, run at speed `× (1 − ½ v)` and metabolism `× (1 + ½ v)`.

**Density fields (insects, spiders, worms, sticklebacks; hourly).** Terrestrial fields live on dry land, aquatic
fields in water of their kind and minimum depth (at ≥ 4 °C). Per cell, `n` individuals/m² of mass `m` respire
`n B(m, T)` (× field multiplier while active: `T ≥ T_pref,low − 5 °C`), eat with a Holling II response
`B · fm · 24 · intake_per_hour · F/(F + K_h)` split over their foods (≤ ½ of each), assimilate `a`, egest the rest;
a positive balance makes young (≤ `max_birth` per day, paid at 1.5 × tissue energy, nutrient-limited), a negative one
burns tissue; deaths `n μ (1 + n/K) Δt` (half while dormant) go to the litter; a dispersal fraction moves to the 8 habitat neighbours.
Fires kill 80 % (fliers 30 %, aquatic 0) per burning tick. Pollinators record flower visits.

**Colonies (ants, bees; every tick).** Workers respire at the nest temperature (hives 33 °C, nests the soil-buffered
air) from the store (deficit: workers starve), die after their lifespan, the queen lays while foraging is possible
and the store holds two days of metabolism (eggs cost 1.5 × tissue), brood emerges after the development time, full
colonies above `swarm_workers` split in the breeding season. Forager squads (`workers_per_forager` each) leave when it
is warm (and light for diurnal species): fliers fly to the recruited patch (p = 0.7) or a random point of the range,
walkers search; a squad harvests its cell's food (or carrion within one cell) and carries it home; searchers smell
carrion and follow trail pheromone outwards; loaded ants mark every cell of their way home; pheromone decays with
`pheromone_half_life_h` and diffuses by `pheromone_diffusion`. Nectar trips pollinate (`flower_visits_per_trip` per
worker): the pollination index rises by visits / flowers (5000 flowers per kg) and halves daily; flower seed
production is `B/K · seed_production · (0.3 + 0.7 pollination)`.

**Schedule.** A scenario's events run at the start of tick `round(day · ticks_per_day)`: ignitions, world
interventions, introductions, removals, outbreaks and parameter changes (not kept by `reset`).

**Ledgers.** Every transfer is booked; with external fluxes disabled the nutrient stock is constant (disasters such as
volcanic ash remain explicit inputs); the energy
stock always equals initial + inputs (production, interventions) − outputs (respiration, combustion, humus).
