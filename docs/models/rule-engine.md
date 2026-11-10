# rule-engine — the custom-life model

`rule-engine` implements the **Custom Life Creator** mode: a generic agent-based model whose entity types, behaviour
rules, relationships and world are all user data. This document describes the model the engine executes, its
equations, parameters, approximations, limitations and how it was verified. The rule language, script API and
editing policies are documented for users in [`docs/RULE_ENGINE.md`](../RULE_ENGINE.md).

Model id: `custom_life` · family: hybrid agent / field model · tick unit: `tick` (dt = 1) · deterministic: yes.

## 1. State

| Component | Representation |
|---|---|
| Entities | organism-engine `AgentStore<Entity>` (stable `AgentId`s, id order). Each entity: type, `Kinematics` (position, velocity, heading), energy, age, state, state entry tick, size, speed, user property values, carried resource stocks, per-rule cooldowns, reproduction cooldown, generation, parent, pending `on_birth` / `on_state_enter` flags. |
| Resource fields | One `Grid2<f32>` per resource, one value per cell (= one world unit). |
| Environment fields | Not stored: computed from position and tick. |
| Randomness | Decisions: streams keyed by (seed, tick, entity id, rule/script id). World events: one `SimRng` (xoshiro256++) in the state. Set-up: a stream derived from the seed. |
| Book-keeping | Per-rule firing totals, script status and errors, event log, last-tick counters, end-condition status. |

**Continuous positions** were chosen over cells: sizes, contact distances, sensing radii and speeds are then real
quantities, several entities may share a cell (bacteria on a nutrient patch), and organism-engine's kinematics,
steering and perception apply directly. Entities may overlap (no volume exclusion).

## 2. One tick

1. **Freeze**: build a spatial index (`SpatialGrid`, slot = store index) over the current positions; the decision
   phase reads only this frame.
2. **Decide** per entity, in id order (rayon above 256 entities; outputs are collected in id order):
   active events → DSL rules by (priority desc, declaration order) → Rhai scripts → default movement. A rule fires
   when it is eligible (event, cooldown, affordable costs, condition, probability, executable actions) and its
   movement / exclusive / transformation slots are free; it then claims them atomically.
3. **Resolve**: contested interactions become organism-engine `Claim`s on their target (priority, distance,
   claimant id) resolved by `resolve_matching`; winners are accepted in rank order with every entity in at most one
   interaction per tick. Refused firings and firings exceeding the entity cap are cancelled as a whole.
4. **Apply** (fixed order): costs and cooldowns → `set`/`add`/`set_state` → interactions (eat, attack, trade, mating)
   → asexual reproduction, `die` (skipped for entities already eaten or killed by an interaction) → `consume` → `emit`
   → metabolism → movement → transformations → births → deaths with `on_death` reactions → ageing.
5. **Fields**: decay, regrowth, diffusion.
6. **Random events** (catastrophe victims go through the same death path: `on_death` reactions, dropped stocks) →
   **end conditions**.

## 3. Equations

### 3.1 Movement

All movement goes through organism-engine `Kinematics::integrate` (semi-implicit Euler, dt = 1) with
`MotionLimits { max_speed: s, max_accel: ∞ }`, i.e. first-order kinematics: an entity reaches the desired velocity
within the tick.

* `move_towards(target)`: Reynolds *arrive* with slowing radius `s`: `v = ĝ · min(s, |g|)` where `g` is the offset to the
  target's nearest image. The entity lands on the target instead of overshooting.
* `move_away(target)`: Reynolds *flee*: `v = −ĝ · s`.
* `wander(turn)` / default movement: correlated random walk (Kareiva & Shigesada 1983). Heading
  `θ' = θ + U(−turn, turn)`, `v = s (cos θ', sin θ')`. `turn = π` gives an uncorrelated random walk (`RandomWalk`),
  smaller values directed motion (`Directed`).
* `stay()`: `v = 0`.

After integration the position is mapped back into the world: reflection (`bounded`), wrapping (`torus`), or
wrapping with mirrored x when crossing the top/bottom edge (`klein_bottle`; on the `cross_surface` also mirrored y
when crossing the left/right edge), mirroring velocity and heading with it.

### 3.2 Sensing and contact

`nearby(k, r)` counts and `nearest(k, r)` returns entities of kind `k` within distance `r ≤ 32` measured to their
nearest image (organism-engine wrap-aware perception on bounded worlds and tori; on Klein bottles and
cross-surfaces the nine images `g(p) = (fx(p.x) + kx·W, fy(p.y) + ky·H)` of the fundamental square are queried at
`g⁻¹(origin)`, which is exact because each `g` is an isometry). Ties go to the lower id. Two entities touch when
`|p_a − p_b| ≤ (size_a + size_b)/2`; `on_collision` binds `other` to the nearest touching entity.

### 3.3 Energy

Per entity and tick:

```text
E ← E − Σ costs(fired rules)  (+ set/add effects)
E ← E + ε · E_prey                                    (eat, ε = efficiency)
E ← E − damage                                         (being attacked)
E ← min(E_max, E + granted_consumption · energy_value) (consume of a non-carried resource)
E ← E − metabolism
dies if E ≤ 0 (starvation on) or age + 1 ≥ max_age, or when eaten / killed / die()
```

Reproduction: each parent pays `cost` (requires `E > cost`, maturity and the reproduction cooldown); the `litter`
offspring share the paid energy equally (`Σ offspring energy = Σ paid`), so reproduction conserves energy. An entity
that is eaten in the same tick does not reproduce: its energy goes to the eater only (a test checks that eat +
reproduce in one tick conserves the total). Spawned
entities (`spawn`, products, immigration) start with their type's initial energy (an external source, by design).

### 3.4 Resource fields

Per cell, in order:

```text
R ← R (1 − decay)
R ← min(max, R + regen)               only where R < max
R ← R + D Σ_{n ∈ N4} (R_n − R)        explicit FTCS 5-point Laplacian, D ≤ 0.25
```

Neighbours follow the topology (`Topology::wrap`); across bounded edges the flux is zero. Every exchange is
antisymmetric, so diffusion conserves the total exactly in exact arithmetic (f32 rounding aside), and `D ≤ 1/4`
keeps the explicit scheme stable (coefficient of the centre cell `1 − 4D ≥ 0`).

**Consumption** requests on one cell are shared proportionally (organism-engine `resolve_shared`,
`ShareRule::Proportional`): if `Σ requests > R`, each request `q_i` receives `q_i · R / Σ q`; grants are subtracted
sequentially so the field never becomes negative. `emit` from a carried stock moves the stock to the cell; dying
entities drop their stocks; `trade` moves stocks atomically. Carried resources are therefore conserved exactly
(verified by a test).

### 3.5 Emergent carrying capacity

With uptake limited by the field, a consumer population in steady state satisfies
`N · metabolism = supply`, so `K = regen · cells / metabolism` whenever the field is depleted below `max`
(Verhulst's logistic curve emerges from this resource limitation rather than being imposed). For two consumers of
one resource the one that persists at the lower resource level (lower R*, Tilman 1982) excludes the other.

## 4. Parameters

Model parameters are the definition itself (see [`RULE_ENGINE.md` §2](../RULE_ENGINE.md#2-definitions)); the
`ParamSpec`s exposed to generic UIs are generated from it: `seed`, `width`, `height`, `topology` (`RequiresReset`),
`max_entities` (`NextTick`), `pause_on_end` (`Immediate`), `resource.<R>.{regen,max,diffusion,decay}`,
`entity.<T>.{metabolism,reproduction_rate}`, `event.<E>.probability` (`NextTick`), `entity.<T>.initial_count`
(`RequiresReset`). Validation bounds: world side 4–4096, diffusion ≤ 0.25, sensing radius ≤ 32, litter ≤ 100,
spawn ≤ 100 per action, entity cap ≤ 10⁶, script limits ≤ documented ceilings.

Parameter defaults are the values of the system as last loaded or applied from the editor (the design; the seed's
default is 0); a parameter change alters the simulated system (reset-only ones that of the next reset), not that
reference. Reset-only parameters show their pending values (those of the next reset), with ranges that accept them.

## 5. Approximations

* **First-order kinematics**: no inertia; desired velocities are reached within one tick.
* **No volume exclusion**: entities overlap freely; crowding acts only through shared resources and rules.
* **Contact is evaluated on the frozen frame**: an `eat` decided at tick *t* succeeds even if the prey moved away
  during *t* (interactions are applied before movement).
* **One interaction per entity per tick**: an entity is the claimant or target of at most one `eat`, `attack`,
  mating or `trade` per tick (pack hunting needs several ticks). An entity eaten or killed by an interaction does
  nothing else that tick; its effects (`emit`, `set`...) decided simultaneously still apply.
* **Trades are checked at decision time** (both parties carry both resources, hold the amounts, have room): stocks
  cannot change between decision and application, so an accepted trade always completes.
* **Default energy semantics**: a single energy store serves as food reserve, health (attacks) and reproductive
  budget.
* **`on_death` rules** see the end-of-tick state and can only `emit` and `spawn`.
* **Environment fields** are analytic functions of position and time (no dynamics of their own); the radial pattern
  uses plain Euclidean distance, not the topology.
* **Field storage in f32**: totals drift by f32 rounding (≈10⁻⁷ relative per operation); carried stocks and entity
  counts are exact.
* **Cross-surface corners**: the square model of the projective plane has two cone points at its corners; distances
  there are taken as the minimum over the nine images.
* **Default script output**: `print` is discarded.
* **Environment fields in views** are sampled at the centre of each output block (they are analytic), resource
  fields are block-averaged.

## 6. Limitations

* Rhai scripts cost about 1 µs per call (research measurement, release); they suit small populations or rare events.
* Rhai 1.27 reports operation-budget errors without a source position.
* A cancelled script (host flag) is disabled for the run; this is the only non-deterministic path and it never
  changes the state silently.
* Floating-point results are bit-identical for the same platform and build; cross-platform equality is not promised.
* Mating does not require reciprocal intents: the partner only has to be eligible (mature, off cooldown, with energy).
* Sensing radii must be literals (they bound sensor cost); there is no line of sight.
* Infinite worlds are not supported.

## 7. Presets and their dynamics

| Preset | Mechanism | Verified behaviour (default seed) |
|---|---|---|
| `bacterial-growth` | consume + divide on a regrowing, diffusing field | 20 → plateau 285 (mean of ticks 600–800) vs K = 0.01·2304/0.08 = 288; peak 335; per-capita growth falls with density. |
| `virus-and-cells` | transform on contact (p = 0.5), latency 12, lysis burst 6, virion decay 30, rare influx | cells cycle between 245 and 55 (ticks 100–1500); the first epidemic crashes them below half of their peak, the virus peaks after its hosts, hosts recover, and at least two epidemics occur within 1500 ticks (troughs of about 70, 60, 40, 60 over 3000 ticks). |
| `competitive-colonies` | shared nutrient, `competes_with` interference | amber (maintenance 0.06) 311 vs violet (0.10) 29 at tick 1300; violet extinct at tick 2127; amber settles at 382 (mean of ticks 2700–3000) vs K = 384. |
| `alien-ecosystem` | producer (light-gradient photosynthesis) / grazer / predator, avoidance, events | all levels persist (moss and spikemaws in > 90 % of samples on every seed tried). With ≈ 30 predators the dynamics are bimodal across seeds: medusae either persist food-limited (≈ 100) or, after a predator overshoot (default seed 2024: medusae present in 80 % of samples), survive for a while as a few drifting newcomers. Removing predators raises mean medusae 52 → 126 and lowers moss 929 → 176 at the default seed; over seeds 2024 and 1–7 medusae rise ×1.2–2.9 and moss falls ×0.14–0.45 (trophic cascade, Hairston, Smith & Slobodkin 1960). |
| `producers-consumers` | plants on sunlight, grazers and hunters with 30 % / 50 % assimilation and handling times | mean standing energy plants 5839 > grazers 1070 > hunters 366 and numbers 474 > 87 > 25 (Lindeman's pyramid). |
| `artificial-chemistry` | A + B → C (eat + transform), C → A + B (transform + spawn), catalyst ×90 both ways | #A + #C = #B + #C = 300 every tick; equilibrium constant [C]/([A][B]) = 1.90·10⁻³ (300/300), 1.89·10⁻³ (400/200), 1.56·10⁻³ (500/100, where B is nearly used up and the estimate is noisiest) — mass action; with the catalyst C reaches half its equilibrium after 64 ticks vs 158 without, at the same equilibrium (88 vs 79). |
| `robot-swarm` | stop on contact, Rhai script sets waiting time 10 + 25 × neighbours (BEECLUST) | Clark–Evans index 1.06 → 0.58 after 400 ticks; pure walkers stay at 0.94. |

Numbers re-measured on 2026-11-10 after the review fixes (per-offspring headings and an independent random stream
for carrying out script intents changed individual trajectories, not the qualitative behaviour).

## 8. How it was verified

`cargo test -p rule-engine` (all pass; ≈ 15 s on the loaded development machine):

* **DSL** (unit tests): tokens and positions, all constructs, precedence and parenthesisation, `parse(print(x)) == x`
  and idempotent printing, error recovery with positions (no depth leaking between rules, the next `rule` never
  skipped), depth limit including operator chains (a 1 MB flat chain used to abort the process; now a 256 KiB stack
  suffices), compact round-tripping of extreme numbers, JSON/RON round trip of the AST.
* **Definitions**: RON/JSON round trip, rules stored as text, future formats rejected, defaults.
* **Validation** (`tests/validation.rs`): every diagnostic family (unknown names with suggestions, arity, types,
  read-only, `other`, sessile movement, reproduction mode, slots, events, products, ranges, duplicates, definitions),
  line/column positions, warnings, relationship compilation (exact generated text, evaluation order, graph),
  item anchors (`ItemRef`) of every family, `validate`/`check_rule`/`check_rule_text`, editor-built rules without a
  faithful text form or with non-finite numbers or 5000-deep trees (would overflow the validator without the guard),
  `on_death` restrictions, priority bounds, initial populations vs the cap, `trades_with` carry requirements, unique
  names for repeated relationships, W102/W103, trophic levels, and that `compile::vocab` signatures are exactly the
  ones accepted; every preset round-trips through RON and JSON unchanged.
* **Sandbox** (`tests/sandbox.rs`): endless loops (budget), recursion, huge strings and arrays, unknown and forbidden
  functions (`sleep`, `timestamp`, file access, `parse_json`), `eval`/`import` rejected, raw-engine checks, runtime
  API errors with line numbers, intent recording, determinism with 300 entities (parallel path), re-enabling,
  host cancellation, `on_death` restrictions.
* **Semantics** (`tests/simulation.rs`): slot claiming by priority, cooldown/once/probability/cost, contested prey
  (one winner, loser pays nothing), proportional consumption with conservation, energy-conserving reproduction (asexual
  and sexual), birth/state/transform/death events, collisions binding `other`, carried resources conserved through
  consume/trade/death, commands and their errors, next-tick vs reset edits, parameter policies, end conditions,
  views, all four finite topologies, identical results with 1 and 4 rayon threads; an eaten entity does not also
  reproduce (energy conserved), impossible trades are never charged or counted, script `random()` is independent of
  the action rolls (birth rate 0.25, not 0.5), catastrophe victims run `on_death`, `reset_pending` in the view, and
  inconsistent snapshots (field sizes, state indices, positions) are rejected instead of panicking later.
* **Fields** (unit tests): diffusion conserves mass on every topology and is symmetric; regrowth saturates; decay;
  disc brushes match a brute-force reference on all four topologies (cross-surface corners included) and stay
  bounded for any radius (previously quadratic in the disc area and unbounded in the radius).
* **Space** (unit tests): Klein-bottle and cross-surface wrapping agree with `Topology::wrap` on cells; queries see
  across twisted edges exactly once.
* **Presets** (`tests/presets.rs`): the behaviours in §7 (the alien cascade on three seeds covering both regimes; the
  virus test follows the first epidemic and counts repeated ones instead of using the global minimum of a fixed
  window), embedded copies equal the files, snapshot/restore replays identically.

## 9. Performance

Criterion (`cargo bench -p rule-engine --bench custom_life`), Apple M2 Max, release profile, macOS, measured
2026-11-09 while the machine was heavily loaded by parallel builds (load average 14–36), so absolute numbers are
pessimistic:

| Benchmark | Time (median) |
|---|---|
| `step/bacterial-growth` (332 entities, 48×48 field with diffusion) | 234 µs / tick |
| `step/producers-consumers` (1264 entities, 3 types, 48×48 field) | 281 µs / tick |
| `step/artificial-chemistry` (531 entities, collisions every tick) | 217 µs / tick |
| `step/robot-swarm` (150 entities, one Rhai call per waiting robot) | 269 µs / tick |
| `compile/all_presets` (parse, validate, generate, lower, compile scripts) | 761 µs |
| `dsl/parse_print` (all preset rules) | 57 µs |

The decision phase is parallel above 256 entities; field diffusion is parallel above 64 rows. These figures predate
the 2026-11-10 review fixes, none of which touches the per-tick hot paths (they add a dead-entity check per exclusive
action and a stock check per decided trade); not re-measured because the machine was again heavily loaded.

## 10. References

* C. W. Reynolds, "Steering Behaviors For Autonomous Characters", GDC 1999 (seek, flee, arrive).
* P. M. Kareiva, N. Shigesada, "Analyzing insect movement as a correlated random walk", *Oecologia* 56 (1983) 234–238.
* C. S. Holling, "Some characteristics of simple types of predation and parasitism", *Can. Entomol.* 91 (1959) 385–398 (handling time).
* P.-F. Verhulst, "Notice sur la loi que la population suit dans son accroissement" (1838) (logistic growth).
* D. Tilman, *Resource Competition and Community Structure*, Princeton University Press (1982) (R* rule).
* R. L. Lindeman, "The trophic-dynamic aspect of ecology", *Ecology* 23 (1942) 399–417.
* N. G. Hairston, F. E. Smith, L. B. Slobodkin, "Community structure, population control, and competition", *Am. Nat.* 94 (1960) 421–425.
* A. Campbell, "Conditions for the existence of bacteriophage", *Evolution* 15 (1961) 153–165; B. R. Levin, F. M. Stewart, L. Chao, "Resource-limited growth, competition, and predation: a model and experimental studies with bacteria and bacteriophage", *Am. Nat.* 111 (1977) 3–24.
* C. M. Guldberg, P. Waage, *Études sur les affinités chimiques* (1867) (law of mass action).
* T. Schmickl, R. Thenius, C. Möslinger, G. Radspieler, S. Kernbach, M. Szymanski, K. Crailsheim, "Get in touch: cooperative decision making based on robot-to-robot collisions", *Auton. Agents Multi-Agent Syst.* 18 (2009) 133–155 (BEECLUST).
* P. J. Clark, F. C. Evans, "Distance to nearest neighbor as a measure of spatial relationships in populations", *Ecology* 35 (1954) 445–453.
* U. Wilensky, *NetLogo Wolf Sheep Predation model* (1997), Center for Connected Learning, Northwestern University (design reference).
* Rhai 1.27.0 source and book, <https://rhai.rs/book>; research note [`docs/research/rules-scripting.md`](../research/rules-scripting.md).
