# Rule engine — Custom Life Creator

The **Custom Life Creator** lets users invent their own artificial life without writing Rust: entity types,
relationships between them, rules written in a small no-code language, optional sandboxed Rhai scripts, and a world
with resource and environment fields. Everything lives in the `rule-engine` crate:

| Module | Contents |
|---|---|
| `def` | Serialisable, versioned definitions (`LifeSystem`, `EntityType`, `Relationship`, `WorldDef`, `ScriptRule`, ...). RON and JSON. |
| `dsl` | Lexer, recursive-descent parser with line/column spans, AST, pretty-printer that round-trips. |
| `compile` | Definition validation, relationship compilation, rule validation and lowering to a typed runtime IR, script compilation. |
| `script` | The Rhai sandbox and the simulation API visible to scripts. |
| `sim` | `CustomLifeSim` (implements `simulation_core::Simulation`), commands, views, parameters. |
| `scenarios` | The seven presets in `scenarios/custom-life/` (`builtin()`, `load_dir()`). |

The model itself (representation, equations, approximations, verification, benchmarks) is described in
[`docs/models/rule-engine.md`](models/rule-engine.md). Background research: [`docs/research/rules-scripting.md`](research/rules-scripting.md).

Contents: [1. Quick start](#1-quick-start) · [2. Definitions](#2-definitions) · [3. The rule language](#3-the-rule-language) ·
[4. Relationships](#4-relationships) · [5. Rhai scripts](#5-rhai-scripts) · [6. Evaluation order](#6-evaluation-order) ·
[7. Commands, edits and parameters](#7-commands-edits-and-parameters) · [8. Views and metrics](#8-views-and-metrics) ·
[9. Diagnostics](#9-diagnostics) · [10. Developer notes](#10-developer-notes) ·
[11. Editor integration](#11-editor-integration)

---

## 1. Quick start

```rust
use rule_engine::{CustomLifeSim, scenarios};
use simulation_core::Simulation;

let preset = scenarios::builtin_by_id("bacterial-growth")?;   // or scenarios::load_dir(path)
let mut sim = CustomLifeSim::from_scenario(&preset)?;
sim.advance(500);
println!("{} bacteria", sim.population("bacterium"));
```

From the command line, `cargo run -p rule-engine --release --example run_preset -- <preset-id | file.ron> [ticks] [every] [seed]`
prints population, energy and resource time series as CSV.

A minimal system written by hand (RON):

```ron
(
    name: "Grazers",
    entity_types: [
        (name: "grass", color: (0.2, 0.8, 0.3, 1.0), energy: (initial: 5.0, metabolism: 0.0, starvation: false),
         initial: (count: 200, distribution: Uniform)),
        (name: "sheep", movement: RandomWalk(speed: 0.5), energy: (initial: 20.0, max: 40.0, metabolism: 0.1),
         reproduction: (mode: Asexual, cost: 10.0, cooldown: 20, rate: 0.05),
         initial: (count: 20, distribution: Uniform)),
    ],
    relationships: [
        (subject: "sheep", kind: Consumes(efficiency: 0.5, hunger: Some(30.0), handling: 10), object: "grass"),
    ],
    rules: [
        "rule regrow for grass: when random() < 0.01 then spawn(grass, 1) cooldown 50",
    ],
    world: (width: 48, height: 48, topology: torus),
    end_conditions: [Extinction("sheep")],
)
```

---

## 2. Definitions

A `LifeSystem` (format version 1, field `format`) contains:

| Field | Meaning |
|---|---|
| `name`, `description` | Display text. |
| `entity_types` | The kinds of entities (order = type index used by views). |
| `relationships` | Typed edges compiled into rules (§4). |
| `rules` | DSL rules, **stored as text** (one string per rule, canonical form). |
| `scripts` | Rhai script rules (§5). |
| `world` | Size, topology, resource fields, environment fields, random events, entity cap. |
| `end_conditions` | `TickLimit(n)`, `Extinction("type")`, `AllExtinct`, `PopulationAbove(entity: Some("t"), count: n)`, `PopulationBelow(...)`. |
| `script_limits` | Sandbox limits (§5.3). |

`LifeSystem::from_ron`, `to_ron`, `from_json`, `to_json` read and write documents; a document with a newer `format`
is rejected (`UnsupportedFormat`). Parsing only checks syntax; `compile(&system)` validates everything else.

### 2.1 Entity types

| Field | Default | Meaning |
|---|---|---|
| `name` | — | Unique; types, resources and environment fields share one namespace. |
| `description`, `color` (RGBA 0..1), `icon` (glyph or emoji), `sprite` (sprite/model id) | | Presentation. |
| `size` | 1.0 | Diameter in world units. Two entities **touch** when their centres are closer than the mean of their diameters. |
| `properties` | `[]` | `(name, default, min, max)`; numeric, readable as `self.name`, writable with `set`/`add` (clamped). |
| `states` | `[]` → `["default"]` | Named states; the first is the initial state. |
| `carries` | `[]` | `(resource, initial, capacity)`: resource stocks the entity holds (see `consume`, `emit`, `trade`). |
| `movement` | `None` | `None` (sessile), `RandomWalk(speed)` (uniformly random direction each tick) or `Directed(speed, turn)` (correlated random walk turning by U(−turn, turn) per tick). Used when no rule or script moves the entity. |
| `energy` | `(initial: 10, max: 100, metabolism: 0, starvation: true)` | Energy budget; `metabolism` is subtracted every tick; with `starvation` the entity dies at energy ≤ 0. |
| `reproduction` | `(mode: None)` | `mode` `None`/`Asexual`/`Sexual`, `cost` (energy each parent pays), `cooldown`, `litter`, `rate` (per-tick chance of a generated reproduce rule; 0 = only explicit `reproduce`). Offspring share the paid energy equally, so reproduction never creates energy. |
| `lifecycle` | | `max_age` (death of old age), `maturity` (minimum age to reproduce). |
| `perception` | 5.0 | Default radius of `nearby`/`nearest` (≤ 32). |
| `initial` | `(count: 0)` | `count` and `distribution`: `Uniform`, `Cluster(x, y, radius)`, `Rect(x, y, width, height)`, `Grid`. |

### 2.2 World

| Field | Meaning |
|---|---|
| `width`, `height` | 4..4096 cells; one cell = one world unit. |
| `topology` | `bounded` (reflecting walls), `torus`, `klein_bottle`, `cross_surface` (same edge identifications as `simulation_core::Topology`). `infinite` is rejected: fields need a size. |
| `resources` | Consumable fields: `(name, color, initial, regen, max, diffusion, decay, energy_value)`. `initial` is `Uniform(v)`, `Random(min, max)`, `Patches(count, radius, amount)` or `Gradient(from, to, axis)`. Per tick: decay, regrowth up to `max`, diffusion (≤ 0.25). |
| `environment` | Read-only fields: `pattern` `Constant(v)`, `Gradient(from, to, axis)`, `Radial(x, y, radius, peak, edge)`, optionally `seasonal: Some((amplitude, period))` → value × (1 + a·sin(2πt/period)). |
| `random_events` | `(name, probability, effect)` with effect `ResourcePulse(resource, amount, radius)`, `ScaleResource(resource, factor)`, `Catastrophe(radius, kill_probability, entity)`, `Immigration(entity, count)`; placed at a uniformly random location. |
| `max_entities` | Cap (default 20 000); births and spawns beyond it are refused and counted (`births_refused`). The initial populations must fit under it. |

### 2.3 Scenario files

Presets are RON files in `scenarios/custom-life/` with a header:
`(id, name, description, seed, expected, tags, system: (...))`. `scenarios::builtin()` returns the seven presets
embedded at compile time; `scenarios::load_dir(path)` and `load_file(path)` read files (compiling each to validate it
and naming the file in errors).

| Id | Shows |
|---|---|
| `bacterial-growth` | Logistic growth emerging from a regrowing nutrient field; plateau at K = supply / maintenance. |
| `virus-and-cells` | Infection (transform on contact), latency, lysis (`on_death` → `spawn`); host–virus cycles. |
| `competitive-colonies` | Two colonies on one resource plus interference competition; the lower-R* colony excludes the other. |
| `alien-ecosystem` | Invented producer / grazer / predator chain under a day-side light gradient; trophic cascade. |
| `producers-consumers` | Energy flow through three trophic levels; pyramid of standing energy. |
| `artificial-chemistry` | A + B ⇌ C with a catalyst; exact atom conservation, mass-action equilibrium. |
| `robot-swarm` | BEECLUST-style aggregation with a Rhai script. |

---

## 3. The rule language

### 3.1 Shape of a rule

```text
rule NAME for TYPE:
    [EVENT]
    [when CONDITION]
    then ACTION, ACTION, ...
    [MODIFIERS]            # priority, probability, cooldown, cost, produces, once
```

Keywords are case-insensitive; `#` and `//` start comments; a rule may end with `;`. Names that are not plain
identifiers (or are keywords) are written as strings: `rule "flee predator" for rabbit: ...`.

Example:

```text
rule forage for rabbit:
    when energy < 20 and nearby(grass, radius=5) > 0
    then move_towards(nearest(grass, radius=5))
    priority 500

rule graze for rabbit:
    on_collision with grass
    when energy < 40
    then eat(other, efficiency=0.6)
    priority 520 cooldown 5

rule breed for rabbit:
    when age > 10 and energy > 30 and nearby(same_species, radius=2) >= 1
    then reproduce(probability=0.15)
    priority 300 cost energy 5

rule panic for rabbit:
    when nearby(fox, radius=6) > 0
    then move_away(nearest(fox), speed=self.speed * 1.5), set_state(scared)
    priority 900
```

### 3.2 Grammar (EBNF)

```ebnf
program   = { rule } ;
rule      = "rule" , name , "for" , name , ":" , [ event ] , [ "when" , expr ] ,
            "then" , action , { "," , action } , { modifier } , [ ";" ] ;
name      = ident | string ;
event     = "on_tick" | "on_birth" | "on_death"
          | ( "on_collision" | "on_contact" ) , [ "with" , name ]
          | "on_state_enter" , "(" , name , ")" ;
modifier  = "priority" , [ "-" ] , int | "probability" , number | "cooldown" , int
          | "cost" , path , number | "produces" , name , [ number ] | "once" ;
action    = "set" , path , "=" , expr
          | "add" , path , ( "+=" | "-=" ) , expr
          | ident , "(" , [ arg , { "," , arg } ] , ")" ;
expr      = and_expr , { ( "or" | "||" ) , and_expr } ;
and_expr  = not_expr , { ( "and" | "&&" ) , not_expr } ;
not_expr  = ( "not" | "!" ) , not_expr | cmp_expr ;
cmp_expr  = sum , [ ( "<" | "<=" | ">" | ">=" | "==" | "!=" ) , sum ] ;   (* no chaining *)
sum       = term , { ( "+" | "-" ) , term } ;
term      = unary , { ( "*" | "/" ) , unary } ;
unary     = "-" , unary | primary ;
primary   = number | string | "true" | "false" | call | path | "(" , expr , ")" ;
call      = ident , "(" , [ arg , { "," , arg } ] , ")" ;
arg       = [ ident , "=" ] , expr ;        (* positional arguments before named ones *)
path      = ident , { "." , ident } ;
```

`and` binds tighter than `or`; `not` binds tighter than both; `a < b < c` is an error ("comparisons cannot be
chained"). `-` followed directly by a number is a negative literal. Expressions nest at most 64 levels; every
parenthesis, `not`, unary `-` and every extra operand of an operator chain counts (`a + b + c` is the tree
`(a + b) + c`, two levels), so no input can build a tree deep enough to overflow the stack. Number literals must be
finite; very large or small ones may use an exponent (`1e300`, `2.5e-9`).

### 3.3 Values and names

| Written as | Type | Meaning |
|---|---|---|
| `energy`, `self.energy`, `organism.energy` | number | Property of the acting entity. |
| `other.energy` | number | Property of the touched entity (only in `on_collision` rules). |
| `tick`, `world.tick` | number | Current tick. |
| `state == scared`, `other.state != "calm"` | true/false | State test (names checked against the type's states). |
| `type == fox`, `other.type == fox` | true/false | Type test. |
| `self`, `other` | entity | The acting entity / the touched entity. |

Built-in properties: `energy` (rw), `age` (ro), `state_age` (ro, ticks since entering the current type/state),
`size` (rw), `speed` (rw), `x`, `y` (ro), `generation` (ro), `id` (ro); plus the type's declared properties (rw,
clamped to their range) and carried resources (ro; changed by `consume`, `emit`, `trade`).
User properties of `other` need `on_collision with TYPE`.

Arithmetic follows IEEE rules except that **division by zero yields 0**; comparisons involving NaN are false.

### 3.4 Sensors (functions)

| Function | Type | Meaning |
|---|---|---|
| `nearby(kind, radius=r)` | number | Entities of `kind` within `r` (default: the type's perception, max 32; must be a number literal), excluding self. `kind` is a type name, `same_species` (aliases `same_type`, `same`) or `any`. |
| `nearest(kind, radius=r)` | entity | Nearest such entity (ties: lower id); nothing → actions on it make the rule ineligible. |
| `distance(e)` | number | Distance to an entity (`inf` if none). |
| `exists(e)` | true/false | Whether the entity expression found something. |
| `field(name)` | number | Resource or environment field at the entity's cell. |
| `population(kind)` | number | Global population at the start of the tick. |
| `random()` | number | Uniform in [0, 1) from the rule's keyed random stream. |
| `chance(p)` | true/false | `random() < p`. |
| `min(a, b)`, `max(a, b)`, `abs(x)`, `clamp(x, lo, hi)` | number | Arithmetic helpers. |

All distances follow the world topology (nearest periodic image on a torus, mirrored images on a Klein bottle or
cross-surface).

### 3.5 Events

| Event | Fires |
|---|---|
| `on_tick` (default) | Every tick. |
| `on_birth` | On the first tick of the entity's life (also for initial entities at tick 0). |
| `on_death` | When the entity dies, whatever the cause (including a `Catastrophe` random event). Only `emit` and `spawn` are allowed (no `set`/`add`, no `cost`); evaluated on the end-of-tick state. |
| `on_collision [with T]`, alias `on_contact` | While the entity touches another entity (of type `T`); `other` is the nearest touching one. |
| `on_state_enter(S)` | On the tick after the entity entered state `S`. |

### 3.6 Actions

Each entity has three **slots** per tick: one *movement*, one *exclusive* action and one *transformation*; effects
are unlimited. A rule claims all its slots at once — if one is taken by a higher-priority rule, the whole rule is
skipped (no cost, no cooldown).

| Action | Slot | Meaning |
|---|---|---|
| `move_towards(e, speed=s)` | movement | Steer to the entity (arrives without overshoot; organism-engine `arrive`). |
| `move_away(e, speed=s)` | movement | Flee (`flee`). |
| `wander(turn=t, speed=s)` | movement | Correlated random-walk step; default turn = the type's (π for random walks). |
| `stay()` | movement | Stand still. |
| `eat(e, efficiency=f)` | exclusive | `e` must touch the entity; it dies and the eater gains `f ×` its energy (default 1). Contested. An entity eaten (or killed by an attack) carries out no other exclusive action that tick — in particular it does not also reproduce, so its energy is not counted twice. |
| `attack(e, damage=d)` | exclusive | `e` must touch; it loses `d` energy (default 5) and dies at ≤ 0. Contested. |
| `reproduce(probability=p, partner=e)` | exclusive | Needs maturity, the reproduction cooldown and energy > cost. Asexual types take no partner; sexual types need `partner=` (a mate type within perception, itself eligible; contested). |
| `die()` | exclusive | |
| `trade(e, give=G, amount=a, receive=R, receive_amount=b)` | exclusive | Atomic exchange of carried resources with a touching entity (both must carry both resources, have the amounts and the capacity). Checked when the rule is decided, so an impossible trade never fires, costs nothing and starts no cooldown. Contested. |
| `transform_into(T)` | transformation | Becomes type `T` at the end of the tick: keeps position, energy (clamped), age, same-named properties and carried resources (the rest is released into the field); state = `T`'s initial state. |
| `emit(R, amount)` | effect | Releases a resource at the entity's cell — from its stock if it carries `R`, otherwise created. |
| `consume(R, amount)` | effect | Takes from the cell; competing requests on one cell are shared **proportionally** (organism-engine `resolve_shared`), so the field never goes negative. The amount fills the stock if `R` is carried, otherwise becomes energy × `energy_value`. |
| `set p = v`, `add p += v`, `add p -= v` | effect | Writable properties of the acting entity only. |
| `set_state(S)` | effect | Enters a state (fires `on_state_enter` next tick). |
| `spawn(T, n)` | effect | Creates `n` (0..100) entities of `T` next to the entity with `T`'s initial energy. |

### 3.7 Modifiers

| Modifier | Meaning |
|---|---|
| `priority n` | Higher first (default 0); ties keep declaration order; `|n| ≤ 1 000 000`. Generated rules use bands (§4). |
| `probability p` | The rule fires with probability `p` once eligible. |
| `cooldown n` | At least `n` ticks between firings of the rule by one entity. |
| `once` | At most once per entity lifetime. |
| `cost p a` | Subtracts `a` from writable property `p` on firing; the rule is not eligible unless affordable (counting costs already reserved this tick). |
| `produces X [n]` | A product released when the rule fires: a **state** of the type (entered), an **entity type** (`n` spawned) or a **resource** (`n` released). |

### 3.8 Pretty-printing

`dsl::print_rule` prints the canonical form (one clause per line, minimal parentheses). `parse(print(ast)) == ast`
(spans are ignored by equality) and printing is idempotent; this is how rules are stored in files. The AST
(`dsl::RuleAst`) also serialises to JSON/RON (externally tagged enums) for visual editors.

An editor that builds `RuleAst`s directly can create trees with no faithful text form (a name with a line break, a
path segment that is a keyword, a cooldown beyond the integer range, a NaN literal). Validation therefore re-parses
the canonical text of every hand-written rule and reports `E015` ("no faithful text form") when it differs, so a
system that compiles always saves and reloads (files, snapshots, undo) unchanged. Trees deeper than 256 levels are
rejected before any recursive pass.

---

## 4. Relationships

A relationship `(subject, kind, object, priority_shift)` is a typed directed edge. The compiler turns it into
ordinary rules — visible in the rule list with their origin (`RuleOrigin::Relationship`), in
`CompiledSystem::rules_text()` and in the descriptor's rule text — that go through the same validation as
hand-written rules. Priority bands: flee 900, defend 700, transform 600, feed 500–520, chase 450, compete 400,
reproduce 300, trade 200, protect/stay-close 150, produce 100; `priority_shift` (±99) moves all rules of one edge.

| Kind (parameters) | Generated rules for the subject |
|---|---|
| `Consumes(efficiency, hunger, radius, bite, handling)` with an entity object | `forage`: `when energy < hunger and nearby(O, radius=r) > 0 then move_towards(nearest(O, radius=r))` p500 (mobile subjects); `eat`: `on_collision with O when energy < hunger then eat(other, efficiency=e)` p520 `cooldown handling` (Holling type II handling time). |
| `Consumes(...)` with a resource object | `feed`: `when energy < hunger and field(O) > 0 then consume(O, bite)` p510. |
| `Protects(radius, damage)` | For each enemy `T` of `O` (types that consume or chase `O`, or that `O` avoids): `defend against T` (move towards `T` while both are near) p700 and `strike T` (`on_collision with T` → `attack`) p710; plus `stay close` p150. |
| `Chases(radius, speed_factor)` | `chase`: `move_towards(nearest(O), speed=self.speed * f)` p450. |
| `Avoids(radius, speed_factor)` | `flee`: `move_away(nearest(O), speed=self.speed * f)` p900. |
| `Transforms(trigger, probability)` | `transform`: `transform_into(O)` p600 with `trigger` `Always`, `Condition("dsl expression")`, `Contact("type")` (→ `on_collision with type`) or `AfterTicks(n)` (→ `state_age >= n`). |
| `Produces(amount, probability, cost)` | `produce`: `emit(O, amount)` (resource) or `spawn(O, amount)` (type) p100, with probability and energy cost. |
| `ReproducesWith(radius, probability, min_energy)` | `mate`: `when age >= maturity and energy >= max(cost, min_energy) and nearby(O, radius=r) > 0 then reproduce(partner=nearest(O, radius=r), probability=p)` p300; also makes `O` a valid partner type. |
| `CompetesWith(aggression, damage)` | `fight O`: `on_collision with O then attack(other, damage=d) probability a` p400, and the mirror rule for `O`. |
| `TradesWith(give, give_amount, receive, receive_amount, radius)` | `approach` p200 (mobile subjects) and `trade` (`on_collision with O`) p210. |

Entity types with `reproduction.rate > 0` also get `"T: reproduce"`: `when age >= maturity and energy >= 2 × cost
[and nearby(same_species, radius=perception) > 0] then reproduce([partner=nearest(same_species, ...), ]probability=rate)`
p300 (origin `RuleOrigin::EntityType`).

Rule names are `"S kind O: what"`. When several relationships share subject, kind and object (e.g. two `transforms`
edges with different triggers), the later ones get a ` #n` suffix (`n` = 1-based relationship index), so every
generated name is unique and stable.

`TradesWith` requires both types to carry both resources (`give ≠ receive`); otherwise the relationship itself is
reported (`E020`), not the rules it would generate.

Warnings: `W102` exact duplicate relationship, `W103` unconditional transformation cycle (`Transforms` with the
`Always` trigger leading back to the subject, e.g. `rabbit → fox → rabbit`), `W201` contradictory edges (consumes and
protects, chases and avoids), `W204` `protects` without enemies, `W205` competition without a shared food
relationship.

Example of generated text (preset `producers-consumers`):

```text
# generated from a `consumes` relationship
rule "grazer consumes plant: eat" for grazer:
    on_collision with plant
    when energy < 25
    then eat(other, efficiency=0.3)
    priority 520 cooldown 25
```

The compiled relationship graph (`CompiledSystem::graph()`, also in every view) lists entity types and resources as
nodes and one typed edge per relationship with the names of the rules generated from it.

---

## 5. Rhai scripts

### 5.1 Script rules

```ron
scripts: [(
    name: "wait",
    entity: "robot",
    event: Tick,              // Tick, Birth, Death, Collision(with: Some("type")), StateEnter(state: "s")
    priority: 10,             // among the type's scripts; scripts run after all DSL rules
    source: "fn run(me) { if me.state == \"waiting\" { stay(); } }",
    enabled: true,
)]
```

A script must define `fn run(me)`; it is compiled **once** when the system is compiled and called with
`call_fn_with_options(eval_ast(false))`, so top-level statements never run. `me` is the acting entity.

### 5.2 API

Entity handles (`me`, results of `nearest`, `other()`) have read-only getters `exists`, `id`, `kind`, `state`,
`energy`, `age`, `state_age`, `size`, `speed`, `x`, `y`, `generation`, `distance` and the method `prop("name")`.

| Queries | Actions (record intents only) |
|---|---|
| `nearby(kind[, radius])` → int | `move_towards(e[, speed])`, `move_away(e[, speed])`, `wander([turn])`, `stay()` |
| `nearest(kind[, radius])` → Entity | `eat(e[, efficiency])`, `attack(e[, damage])`, `die()` |
| `field(name)` → float | `reproduce([p])`, `reproduce_with(partner[, p])` |
| `population(kind)` → int | `transform_into(type)`, `set_state(s)` |
| `other()` → Entity (collision scripts) | `emit(res, amount)`, `consume(res, amount)` |
| `tick()` → int, `random()` → float | `set_prop(name, v)`, `add_prop(name, v)`, `spawn_entity(type[, n])` |

`spawn` is a reserved word in Rhai, hence `spawn_entity`. Action functions never touch the world: they append
intents that the engine checks and resolves exactly like DSL actions; within one call a later movement, exclusive or
transformation action replaces an earlier one, and the whole call claims its slots atomically after the DSL rules.
Acting on an entity that does not exist records nothing. Invalid names or values (unknown type, radius out of range,
read-only property, a sessile type moving, anything but `emit`/`spawn_entity` in an `on_death` script) raise a runtime
error.

### 5.3 Sandbox

Verified recipe for Rhai 1.27 (see the research note, §1):

* `Engine::new_raw()` with hand-picked packages (arithmetic, logic, math, bit fields, strings, iterators, function
  pointers, arrays, maps). `LanguageCorePackage` (`sleep`, `exit`, `parse_json`), `BasicTimePackage` (`timestamp`) and
  `BasicBlobPackage` are not registered; no package reaches files, the network, processes or the environment.
* `DummyModuleResolver`; `eval`, `import` and `export` disabled (`import`/`eval` are compile errors).
* Limits set explicitly from `script_limits` (defaults; ceilings in brackets): `max_operations` 20 000 per call
  [1 000 000], call levels 16 [64], expression depth 64 [128], string 1024 bytes, array 1024, map 256, variables 128,
  functions 64; modules 0. Strict variables, no shadowing, failing map property access. `print` is a no-op.
* `on_progress` checks a host cancellation flag (`CustomLifeSim::script_cancel_handle()`), cleared after every tick.
* Only the operation budget influences outcomes (it is deterministic); a cancellation only disables scripts.

Any compile error rejects the system (`E030` with line:column). Any runtime error **disables that script** for the
rest of the run and adds a `ScriptError { script, entity, tick, line, column, kind, message }` to the error list
(`CustomLifeSim::script_errors()`, the view's `script_errors`, the event log, the `script_errors` metric). Kinds:
`Budget` (endless loop), `Recursion`, `Memory` (string/array/map/variable limits), `UnknownFunction`, `Type`,
`Cancelled`, `Runtime`. Rhai 1.27 reports budget errors without a position; the message then names the budget.
`CustomLifeCommand::ReenableScripts` clears the list.

Scripts receive a random stream keyed by (seed, tick, entity, script), so they are deterministic and can run on
several threads. Carrying out their intents (wander turns, reproduction rolls) draws from a separate stream, so a
script's own `random()` calls never correlate with those rolls. They cost roughly 1 µs per call (research measurement); use DSL rules for large populations.

---

## 6. Evaluation order

Per tick:

1. **Freeze** the world into a read-only frame (entities, fields, spatial index).
2. **Decide** for every entity (id order; in parallel above 256 entities — results do not depend on it):
   1. DSL rules of its type in priority order (desc), then declaration order (manual rules, then relationship rules,
      then type rules). A rule is tried only if its event is active, its cooldown has expired and its costs are
      affordable; then its condition is evaluated, its probability rolled, its actions evaluated (a missing target,
      a target that does not touch for `eat`/`attack`/`trade`, or an impossible reproduction makes it ineligible) and
      its slots claimed atomically.
   2. Script rules of its type (priority order).
   3. The default movement if the movement slot is still free.
3. **Resolve intents** (organism-engine `intent`):
   1. Contested interactions (`eat`, `attack`, sexual `reproduce`, `trade`) as claims on the target, matched one-to-one
      (`resolve_matching`: priority, then distance, then id), then accepted in rank order so that every entity takes
      part in at most one interaction per tick. A refused firing is cancelled entirely.
   2. Firings that would exceed `max_entities` are cancelled entirely.
4. **Apply**, in this order: costs and cooldowns; `set`/`add`/`set_state`; interactions in rank order (eat, attack,
   trade, reproduction); asexual reproduction, `die`; `consume` (proportional sharing per cell); `emit`;
   metabolism; movement (`Kinematics::integrate`, then the topology); transformations; births and spawns; deaths
   (`die`, eaten, killed, starvation, old age) with `on_death` rules and scripts, dropping carried resources; ageing.
   An entity eaten or killed by an interaction carries out no further exclusive action (asexual `reproduce`, `die`).
5. **Fields**: decay, regrowth, diffusion.
6. **Random events** (catastrophe victims die like any other entity: `on_death` rules run, stocks are dropped, an
   extinction is logged), then **end conditions** (the first condition met finishes the run if pausing is on).

---

## 7. Commands, edits and parameters

`CustomLifeCommand`:

| Command | Effect |
|---|---|
| `Spawn { entity, x, y }` | Creates an entity (validated type and position; respects the cap). |
| `Remove { id }` | Removes an entity (its carried resources are dropped). |
| `PaintResource { resource, x, y, radius, amount, mode: Add \| Set }` | Disc brush on a resource field (never negative). |
| `ReplaceSystem(system)` | Definition edit (below). |
| `SetPauseOnEnd(bool)` | Whether end conditions finish (pause) the run. |
| `ReenableScripts` | Re-enables scripts disabled by errors. |

**Change-application policy** (`classify_edit`, `CustomLifeSim::edit_system`, which returns an `EditReport`):

| Kind of edit | Policy |
|---|---|
| Rules, scripts, relationships, rule modifiers; entity-type behaviour and appearance (colour, icon, sprite, size, movement, energy budget, reproduction, lifecycle, perception, property ranges); resource dynamics (regen, max, diffusion, decay, energy value, colour); environment fields; random events; end conditions; script limits; entity cap | `NextTick`: applied before the next tick. Live entities are kept; per-rule cooldowns restart; values are clamped to new ranges (carried stock above a lowered capacity is released into the field, keeping resources conserved); disabled scripts are re-enabled. |
| Adding, removing, renaming or reordering entity types, their states, properties or carried resources; adding, removing or renaming resource fields; world width, height or topology | `RequiresReset`: stored as the system of the next reset; the running world is untouched. |
| Initial populations and initial field contents | Only matter at reset (reported in the notes). |

An invalid system is rejected with all diagnostics and changes nothing.

Parameters (generated from the definition; `set_param` returns the policy): `seed`, `width`, `height`, `topology`
(`RequiresReset`), `max_entities` (`NextTick`), `pause_on_end` (`Immediate`); per resource
`resource.<name>.regen|max|diffusion|decay`, per type `entity.<name>.metabolism|reproduction_rate` (`NextTick`) and
`entity.<name>.initial_count` (`RequiresReset`), per random event `event.<name>.probability` (`NextTick`).

Parameter defaults are the values of the design: the system as last loaded (`CustomLifeSim::new`) or applied from
the editor (`edit_system` / `ReplaceSystem`), kept as `CustomLifeSim::reference()` and carried by snapshots (a
system holds no seed: the seed's default is 0). `set_param` changes the system of the next reset (and, for next-tick
parameters, the running system) but not that reference, so a UI can offer going back to the design. Reset-only
parameters show their pending values, those the next reset uses (`reset_target()`), and their ranges follow it: an
initial count is bounded by the entity cap of the next reset, and the initial counts listed are those of the next
reset's entity types.

---

## 8. Views and metrics

`CustomLifeViewRequest { include_fields, field_resolution, include_rules, recent_events }` →
`CustomLifeView { tick, width, height, topology, entities, fields, legend, graph, rules, script_errors, warnings,
finished, recent_events }`:

* `entities`: `id`, `type_index`, `position`, `heading`, `state`, `size`, `color`, normalised `energy`.
* `fields`: resource and environment grids, block-averaged to at most `field_resolution` samples per axis, with
  min/max and cell size.
* `legend`: per type name, description, colour, icon, sprite, states and live count.
* `graph`: relationship graph — nodes (types first, in legend order, then resources) with colour, icon and
  `trophic_level`; typed edges with the relation label, `summary` of the parameters, `symmetric` flag, relationship
  index and the names of the generated rules.
* `rules`: every rule with origin, canonical text and firing total.
* `diagnostics`: the compilation warnings as structured `Diagnostic`s (`warnings` has them rendered).
* `reset_pending`: an edit (structural definition change, size, topology, seed...) waits for the next reset.

Metrics: `population`, `births`, `deaths`, `rule_firings`, `script_errors`, `eaten`, `killed`, `trades`, `refused`,
`births_refused`, `random_events`; per type `population.T`, `energy.T`, `births.T`, `deaths.T`; per resource
`resource.R` (total) and `consumed.R`; per rule `rule.<index>.<type>.<name>` (firings in the last tick).

---

## 9. Diagnostics

Every problem is a `Diagnostic { code, severity, message, context, span, help, item }`, displayed as
`error[E001] in rule `forage` (rabbit) at 2:10: unknown property `enrgy` (help: did you mean `energy`?)`.
Positions are 1-based line:column inside the rule's own text. `item` (`ItemRef`) names the definition entry the
problem belongs to — `Rule(i)` (index in `rules`), `Relationship(i)` (also for its generated rules),
`EntityType(i)` (also for rules generated from its reproduction settings), `Script(i)`, `Resource(i)`,
`Environment(i)`, `RandomEvent(i)`, `EndCondition(i)`, `World`, `ScriptLimits` or `System` — so an editor can select
it. The parser recovers at the next `rule` (never skipping it), so all broken rules are reported at once.

Entry points: `compile(&system)` (all diagnostics in the error, warnings in `CompiledSystem::warnings()`),
`validate(&system) -> Vec<Diagnostic>` (errors and warnings in one list), `check_rule(&system, &rule)` and
`check_rule_text(&system, text)` (one rule against the definitions, for live feedback while typing). Every list of a
system is capped at 10 000 entries (256 entity types).

| Code | Meaning | Code | Meaning |
|---|---|---|---|
| E001 | unknown property | E012 | invalid count / whole number / priority outside ±1 000 000 |
| E002 | type mismatch, non-finite number | E013 | `other` outside a collision rule |
| E003 | unknown type, resource, state or field | E014 | unknown function or action |
| E004 | bad arguments (arity, unknown/duplicate name, order, partner) | E015 | syntax error |
| E005 | probability outside [0, 1] | E016 | duplicate name |
| E006 | sensing radius outside (0, 32] or not a literal | E017 | sessile type asked to move |
| E015 | (also) a hand-written rule without a faithful text form, or nested too deeply | E020 | (also) empty rule/script name, list too long |
| E007 | action, `set`/`add` or `cost` not allowed in `on_death` | E018 | type cannot reproduce |
| E008 | read-only property | E020 | invalid definition |
| E009 | invalid cost | E030 | script compile error |
| E010 | two movement / exclusive / transform actions in one rule | W102, W103, W105, W201, W204, W205 | warnings |

---

## 10. Developer notes

* **Pipeline**: `LifeSystem` → `compile::defs` (definition checks) → `compile::relations` (generated `RuleAst`s) →
  `compile::rules::lower_rule` (+ `compile::lower` expression typing, `compile::symbols`) → `CompiledRule` with
  `RExpr`/`RAction` IR → `sim::decide` (pure, keyed RNG) → `sim::resolve` (sequential).
* **Frames**: the entity store and fields live in `Arc`s; a frame shares them during the decision phase (scripts
  receive the frame through the Rhai call tag) and the simulation regains unique ownership afterwards, so no copy is
  made.
* **Determinism**: id order everywhere, no hash-map iteration, random streams keyed by (seed, tick, entity, rule),
  rayon only over the pure decision phase and field diffusion (double buffered). Verified by tests comparing 1 and 4
  threads and by snapshot replay.
* **Snapshots** (`schema_version` 2) contain the definition (rules as text), the reference design (parameter
  defaults, §7), entities, fields, the world RNG, counters, script status and the event log; restoring recompiles
  the definition and checks consistency. Version 1 snapshots (without the reference) are still restored, with their
  definition as the reference.
* **Vocabulary**: `compile::vocab::FUNCTIONS` lists every sensor and action with its parameters, value kinds, slot
  and whether it is allowed in `on_death`; the validator binds arguments against this table, so editor palettes built
  from it cannot drift from what is accepted. `BUILTIN_PROPERTIES`, `EVENTS`, `MODIFIERS` and
  `dsl::parser::KEYWORDS` complete it.
* **Tests**: `cargo test -p rule-engine` (DSL round-trip and diagnostics, sandbox, rule semantics, presets).
  Benchmarks: `cargo bench -p rule-engine`.

---

## 11. Editor integration

What a (Bevy/egui) editor needs, all in `rule_engine` (no renderer dependency):

| Need | API |
|---|---|
| Data to edit | `LifeSystem` and its parts (`EntityType`, `PropertyDef`, `CarriedResource`, `MovementDef`, `EnergyDef`, `ReproductionDef`, `LifecycleDef`, `InitialPopulation`, `Relationship` + `RelationKind`, `ScriptRule`, `WorldDef`, `ResourceField`, `EnvironmentField`, `RandomEvent`, `EndCondition`, `ScriptLimits`); rules as `dsl::RuleAst` (block editor) or text (`dsl::parse_rule` / `dsl::print_rule`). All are plain serde data; `LifeSystem::{to,from}_{ron,json}`. |
| Palettes and forms | `compile::vocab` (functions with parameters, value kinds, slots), `BUILTIN_PROPERTIES`, `EVENTS`, `MODIFIERS`, `RelationKind::{label, summary, priority_band}`; per system: `CompiledSystem::types()` (states, properties). |
| Problem list | `validate(&system)` → every `Diagnostic` (errors and warnings) with `item: Option<ItemRef>` and `span`. |
| Live feedback on one rule | `check_rule_text(&system, text)` → `(Option<RuleAst>, Vec<Diagnostic>)`; `check_rule(&system, &ast)`. |
| Generated rules | `compile(&system)?.rules()` with `origin` (`RuleOrigin::{Manual(i), Relationship{index, relation}, EntityType{entity}}`), `text`, `ast`; `rules_text()`. "Detach": copy a generated rule's `ast` into `system.rules` (rename it) and remove the relationship. |
| What an edit will do | `classify_edit(&running, &edited)` → `EditReport { policy: NextTick \| RequiresReset, notes }`. |
| Apply to the running model | Through the `Runner`: `Control::Edit(CustomLifeCommand::ReplaceSystem(Box::new(system)))` (undoable; compatible edits apply next tick, structural ones wait for `Control::Reset`; an invalid system is rejected and reported as a runner error — call `validate` first to show diagnostics). To start over with a new system: `Control::Replace(Box::new(CustomLifeSim::new(system, seed)?))`. Parameters: `Control::SetParam` (returns the `ApplyPolicy`). Directly on the model: `CustomLifeSim::edit_system(system) -> Result<EditReport, RuleEngineError>`. |
| Status in the view | `CustomLifeView::{diagnostics, warnings, script_errors, reset_pending, finished, recent_events}`; `ReenableScripts` command after fixing a script. |
| Graph view | `CustomLifeView::graph` / `CompiledSystem::graph()`: `RelationshipGraph { nodes: [GraphNode { name, resource, color, icon, trophic_level }], edges: [GraphEdge { from, to, relation, relationship, rules, summary, symmetric }] }`. Nodes `0..types` match `legend`; seed a force-directed layout with `trophic_level` as height. |

