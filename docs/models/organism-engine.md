# organism-engine — generic agent infrastructure

`crates/organism-engine` is the foundation shared by every agent-based mode of
Emergence Lab: the ecosystem, evolution, civilization and custom-life
(rule-engine) models. It is **not a model**. It has no `Simulation`
implementation, parameters or metrics of its own. It provides deterministic
building blocks that those models combine:

| Module | What it gives a model |
|---|---|
| `id`, `store` | `AgentId`, `AgentStore<T>`: stable ids, storage in id order, spawn/despawn queued until a commit |
| `kinematics` | `Kinematics` point-mass motion with speed and acceleration limits, world `Bounds` (clamp / reflect / wrap) and a passability hook |
| `steering` | seek, flee, arrive, pursue, evade, wander, boids (separation, alignment, cohesion), sampled obstacle avoidance, weighted and prioritised blending |
| `perception` | radius and field-of-view queries on `simulation_core::SpatialGrid` (wrap-aware on a torus), sorted results, sampled line of sight |
| `utility`, `fsm` | response curves, considerations, compensated action scores, `choose_best`; a small state machine with a transition log |
| `intent` | `Claim`s and conflict resolution (exclusive, one-to-one matching, shared) that does not depend on input order |
| `genome`, `lineage` | gene schemas, normalised genomes, mutation, crossover, genetic distance; lineage records and a pruned genealogy |
| `drive` | needs that grow over time (hunger, thirst, fatigue...) |

Dependencies: `simulation-core` (for `SimRng`, `SpatialGrid`, `glam`),
`serde` and `thiserror`. There are no third-party crates beyond those, and
no renderer.

## 1. The tick pattern it supports

`ARCHITECTURE.md` §3.2 requires parallel code to be pure and every contested
decision to be resolved in a fixed order. The crate is built around this
five-phase tick:

```text
index    perception::index_agents(&mut grid, &store, |a| a.pos)    (slot = store index)
decide   for each agent (read-only, may be parallel):
           perceive -> utility scores -> choose_best -> steering accel + Claims
resolve  intent::resolve_exclusive / resolve_matching / resolve_shared   (serial, total order)
apply    store.split_mut(): integrate kinematics, update drives,
         queue spawns/despawns, record births/deaths in the Genealogy
commit   store.commit()   (spawns appended, then despawns; report returned)
```

## 2. Algorithms and equations

### 2.1 Agent store

* Two parallel `Vec`s (`ids`, `agents`) sorted by `AgentId`. Lookup is a binary
  search, `O(log n)`, with no hashing. Iteration is always in id order.
* Ids come from a counter that only increases. It starts at `1`, so
  `AgentId(0)` is never issued and dependents can use it as a sentinel. Ids are
  never reused. `queue_spawn` reserves the id immediately (so it can be
  recorded as a child in a genealogy) and appends at `commit()`. Every queued
  id is larger than every committed id, so appending keeps the order.
* `commit()` applies all spawns, then all despawns. Despawns are sorted and
  deduplicated, so the result does not depend on the order in which they were
  queued. The `CommitReport` lists the spawned ids, the despawned agents with
  their final state (for corpses or detritus), and despawns of unknown ids.
* `split_mut()` returns `(AgentsMut, &mut PendingChanges)`, so a system can
  mutate agents and queue births and deaths in the same loop.
* Serde includes the pending queue and the id counter. Deserialisation
  validates that the id and agent counts match, that committed ids and then
  queued spawn ids increase strictly, that none of them is the reserved `#0`,
  that every id is below the counter, and that the counter lies in
  `[1, 2⁶³]` (so a crafted snapshot can neither issue `#0` nor overflow the
  counter).

### 2.2 Kinematics (semi-implicit Euler)

```text
a  = truncate(a_desired, max_accel)
v' = truncate(v + a·dt, max_speed)
p' = bounds(p + v'·dt)            heading' = atan2(v'.y, v'.x)   if |v'| > 1e-6
```

`truncate` returns zero for non-finite vectors, so a degenerate steering
result can never put NaN into agent state. Finite vectors whose squared length
overflows `f32` are rescaled by their largest component before normalising, so
they keep their direction. Bounds:

* **Clamp** clamps the position into `[min, max]` and removes the outward
  velocity component.
* **Reflect** mirrors the position at the edge and reverses the outward
  component.
* **Wrap** maps the position into `[min, max)`; a result that rounds up to
  `max` is mapped to `min` (the same point of the torus). `Bounds::delta`
  gives the shortest wrapped displacement.
* An axis with non-finite or unordered bounds (or a zero period for Wrap) is
  left unconstrained, and `contains` agrees with `apply` on every axis.

`integrate_with(.., is_passable)` tests only the destination. If the full step
is blocked, the agent tries to slide along the axis with the larger
displacement first, then along the other axis. The velocity component of the
blocked axis is dropped. If both slides are blocked, the agent stops
(`Movement::Blocked`).

### 2.3 Steering (Reynolds 1999)

Every behaviour returns a desired acceleration with `|a| ≤ max_accel`. Most use
`steer = desired_velocity − velocity`:

| Behaviour | Desired velocity / rule |
|---|---|
| seek | `normalize(target − p) · max_speed` |
| flee | `normalize(p − threat) · max_speed` (current direction if `p == threat`) |
| arrive | `normalize(target − p) · max_speed · min(1, d / slowing_radius)` |
| pursue / evade | seek / flee `q + v_q·T` with `T = min(d / speed, max_prediction)`, and `T = max_prediction` while standing still (Millington & Funge) |
| wander | target on a circle of `radius` placed `distance` ahead; its angle does a random walk `θ += U(−jitter, jitter)` (one `SimRng` draw per call); seek the target |
| separation | `Σ (p − p_j) / |p − p_j|²` over neighbours within `radius`, normalised, at `max_speed` |
| alignment | mean neighbour velocity direction at `max_speed` (Shiffman's variant of Reynolds' rule) |
| cohesion | seek the centroid of neighbours within `radius` |
| avoid_obstacles | three feelers (centre, `±spread`) sampled `samples` times over `length`. The closest blocked sample `k` (1 = nearest) gives `urgency = (samples − k + 1)/samples`. The agent turns away from the side that hits closer (towards the positive-angle side on a tie) with lateral `max_accel·urgency`, and brakes with `max_accel·urgency` when the centre feeler hits |

Co-located neighbours (distance 0) are ignored by the boids rules, so the
neighbour slice may include the agent itself. A negative or NaN radius selects
no neighbours.

Desired velocities use `MotionLimits::desired_speed()`: `max_speed` with NaN
or negative mapped to 0 and `∞` mapped to `f32::MAX`. An unlimited agent
therefore steers at full acceleration (`∞ · 0` would otherwise be NaN and be
discarded by `truncate`). `avoid_obstacles` uses
`truncate(v·a, a) = a·truncate(v, 1)` for the same reason with an unlimited
`max_accel`.

Blending:

* `combine_weighted` computes `truncate(Σ wᵢ·aᵢ, max_accel)`.
* `combine_prioritized` implements Reynolds' prioritised acceleration
  allocation. Behaviours are taken in order and each one adds as much of
  itself as the remaining budget `max_accel − |Σ|` allows. A typical order is
  avoid, flee, separation, flock, wander.

### 2.4 Perception

A `Query` is a radius plus an optional `FieldOfView`, which is a cone of full
aperture `angle` around a direction. An aperture of `2π` or more, or a zero
direction, sees everything. The observer's own position is always visible.

* `nearest`, `count` and `within` take a filter `(slot, position) -> bool`.
* `nearest` and `within` share one ranking: distance, then slot. Because
  `index_agents` uses the store index as the slot, ties go to the older agent.
* `line_of_sight(from, to, step, is_clear)` tests evenly spaced points strictly
  between the endpoints: `ceil(d/step) − 1` samples, at most 1024.
* A negative or NaN radius sees nothing (`Query::accepts` agrees with the grid).

**Toroidal worlds.** `Query::in_world(Bounds::Wrap { min, max })` makes every
query see each item at its nearest periodic image: the distance is
`|bounds.delta(origin, p)|` and `Percept::position = origin + delta`, the image
in the observer's frame, which steering can use directly. Each item is
reported at most once.

* If the query diameter is below 99 % of both periods, the circles around the
  images of the origin are disjoint. The grid is then queried around the
  wrapped origin and around its images shifted by `±period` on each axis
  whose edge the circle crosses (at most four grid queries).
* Otherwise every indexed item is visited once and measured with `delta`
  (`O(n)`, but such a radius covers most of the world anyway).
* Indexed positions must lie inside `[min, max)` (as `Bounds::apply` keeps
  them), and the rectangle should match the grid's world.

### 2.5 Utility AI (Infinite Axis Utility System style)

Curves map an input in `[0, 1]` to an output in `[0, 1]`. Both input and
output are clamped, and NaN becomes 0:

| Curve | `y(x)` |
|---|---|
| `Linear` | `x` |
| `InverseLinear` | `1 − x` |
| `Quadratic` | `x²` |
| `Exponential { k }` | `(e^{kx} − 1)/(e^k − 1)`; convex for `k > 0`, concave for `k < 0`, linear for `k → 0`; evaluated in an overflow-free form |
| `Logistic { k, x0 }` | `1/(1 + e^{−k(x − x0)})` |
| `Step { threshold }` | `1` if `x ≥ threshold`, else `0` |
| `Custom(points)` | piecewise-linear through validated points (`x` strictly increasing, everything in `[0, 1]`), constant outside the first and last point |

* A **consideration** scores `s = 1 − w·(1 − curve(input))`, where the weight
  `w ∈ [0, 1]` is its importance. `w = 0` makes it neutral.
* An **action score** with `n` considerations applies Dave Mark's
  compensation factor to each score, `s' = s + (1 − s)·(1 − 1/n)·s`, and then
  multiplies: `value = action_weight · Π s'`. Any zero score vetoes the
  action, and `vetoed_by` records which consideration did it.
* `choose_best(scores, rng, noise)` returns the index of the largest positive
  finite score. Ties go to the lowest index. If every score is zero, negative
  or non-finite it returns `None`.
* The optional noise multiplies each score by `1 + noise·(2u − 1)`. Exactly
  one draw is made per score whenever an rng is given, so the random stream
  advances the same way whatever the score values are.

`StateMachine<S>` stores the current state and its entry tick, counts the
transitions, and keeps a bounded log (16 by default). `time_in_state(now)` is
`now − entered_at`. A transition to the current state is ignored.

### 2.6 Intent resolution

The rank inside a target is a total order. Earlier keys decide first:

1. priority, descending (NaN ranks last)
2. distance, ascending (NaN counts as infinitely far)
3. claimant id, ascending
4. requested amount, descending
5. raw bit patterns, used only to make the order total

`-0.0` and `+0.0` rank equally. Sorting with this order makes every resolver
independent of input order.

* `resolve_exclusive` gives each target to its best claim. A claimant can win
  several targets.
* `resolve_matching` is a greedy maximal matching over all claims in global
  order (priority, distance, claimant, then **target ascending**, with the
  amount and bit keys only ordering exact duplicates): each claimant gets at
  most one target and each target at most one claimant. Agents can submit
  lower-priority fallback claims.
* `resolve_shared(claims, available, rule)` splits divisible targets:
  * `Proportional`: if `Σ requested > available`, every claimant receives
    `requested · available / Σ requested`.
  * `Ordered`: claimants are served in rank order, and each receives
    `min(requested, remaining)`.

  The proportional fraction `available / Σ requested` is computed in `f64`,
  so requests whose `f32` sum would overflow are still shared correctly. An
  infinite request receives whatever remains when it is served (the limit of
  the proportional rule).

  In both rules the grants are produced by sequential subtraction from the
  available amount, in the returned order. Subtracting them from a stock in
  that order therefore never goes below zero, which matters for nutrient
  accounting. Negative or NaN availability counts as 0, and `INFINITY` means
  unlimited.
* `resolve_claims(claims, Resolution, available)` is a single entry point for
  data-driven models. For the winner-only modes, the winner is granted
  `min(requested, available)`.

### 2.7 Genomes

* A `GeneSpec` has `name`, `min`, `max`, `default`, `mutation_sd` and
  `heritable`. `mutation_sd` is a fraction of the range and defaults to 0.05;
  `heritable` defaults to `true`.
* A `GenomeSchema` validates its genes: unique non-empty names, finite values,
  `min ≤ default ≤ max`, `mutation_sd ≥ 0`. `min == max` declares a constant
  trait.
* A `Genome` stores one normalised value `g ∈ [0, 1]` per gene. Expression is
  `min + g·(max − min)`.
* **Mutation**: for each heritable gene, with probability `rate` (clamped to
  `[0, 1]`, NaN counts as 0), `g ← bounds(g + N(0, mutation_sd))`. `bounds` is either `Clamp` (the
  default, as specified) or `Reflect`, which folds like a mirror and avoids
  the pile-up of probability at the bounds that clamping produces.
  * One uniform draw is made per heritable gene when `0 < rate < 1`, plus two
    draws (Box–Muller) per mutated gene.
* **Crossover**:
  * Uniform: one 64-bit draw per heritable gene, and each gene comes from
    either parent with probability ½.
  * One-point: one draw picks a cut `c ∈ [1, n)`. Genes before the cut come
    from `a` and the rest from `b`. With fewer than two genes there is no draw
    and the child copies `a`.
* **Non-heritable genes** are acquired traits. They are not mutated or
  inherited (offspring start at the default), not randomised, and not counted
  in distance. A model changes them with `set_normalised`.
* **Distance**: `sqrt(Σ_heritable (aᵢ − bᵢ)² / m) ∈ [0, 1]`, where `m` is the
  number of heritable genes.
* The cross-crate contract names `genome::{mutate, crossover,
  genetic_distance}`. `genome::mutate(&mut genome, &schema, rate, bounds,
  &mut rng)` and `genome::crossover(&a, &b, &schema, operator, &mut rng)` are
  the free-function forms of `Genome::mutate_with` and `Genome::crossover`.

### 2.8 Lineage and genealogy

`Lineage { id, parents: [Option<AgentId>; 2], generation, birth_tick }`. A
child's generation is `1 + max(parent generations)`.

`Genealogy` records births and deaths and keeps only the useful part of the
family DAG. Its invariant is that every recorded node is alive or has at least
one recorded child.

* **Birth order.** Births must be recorded in increasing id order
  (`BirthOutOfOrder` otherwise), and every parent must have a smaller id than
  its child (`SelfParent`, `ParentNotOlder`). `AgentStore` ids satisfy both
  naturally, as does the research-note rule of assigning ids from one
  increasing counter. The rule guarantees that a parent is recorded before
  its children, so the parent → child links are complete (otherwise a founder
  recorded after its child would be pruned while the child lives, and the
  snapshot would fail its own validation). It also guarantees that a pruned id
  can never be recorded, and counted, a second time.

* When a node is dead and childless it is removed, and the removal cascades up
  through its parents. These removals are counted as `pruned`. By induction,
  the node set is always exactly the living agents plus all their recorded
  ancestors.
* Because that ancestry still grows with the number of generations,
  `with_max_nodes(n)` also **truncates** the oldest dead ancestors, in id
  order, whenever more than `n` nodes are stored. These removals are counted
  as `truncated`. Living agents are never removed, so after every birth *and*
  every death the node count is at most `max(n, living)`. The dead nodes are
  indexed in a `BTreeSet`, so finding the oldest one costs `O(log n)` even when
  the oldest ids belong to long-lived agents.
* `summary()` holds the births, deaths, living, pruned and truncated counts and
  the largest generation. These counts survive pruning.
* `edges()` exports `(parent, child)` links between recorded nodes, ordered by
  child id and then parent slot, for tree views.

### 2.9 Drives

`Drive { value, rate, threshold }`:

* `value` is the deprivation level in `[0, 1]`.
* `update(dt)`: `value ← clamp(value + rate·dt, 0, 1)`. A negative rate models
  spontaneous recovery.
* `satisfy(amount)` lowers the value and returns the reduction actually
  achieved.
* `is_urgent()` is `value ≥ threshold`.
* `urgency()` is 0 below the threshold and rises linearly to 1 at a value of 1.
* `time_until_urgent()` is the time left before the threshold is reached at the
  current rate.
* `most_urgent(&[Drive])` picks the urgent drive with the highest urgency, then
  the highest value, then the lowest index.

## 3. Parameters

The crate has no `ParamSpec`s of its own. Models expose these values as their
own parameters:

| Item | Meaning | Typical values |
|---|---|---|
| `MotionLimits { max_speed, max_accel }` | vehicle limits (world units per time unit) | species data / genes |
| `WanderParams { distance, radius, jitter }` | wander circle ahead of the agent, angular jitter per call | 2, 1, 0.5 rad |
| `Feelers { length, spread, samples }` | obstacle feelers | 3, 0.6 rad, 6 |
| `FieldOfView::new(dir, angle)` | full aperture in radians | π to 1.5π for prey, about 0.7π for predators |
| `line_of_sight(step)` | sample spacing | ≤ thinnest obstacle (1 cell) |
| `Consideration.weight`, action weight | importance and multiplier | `[0, 1]`, 1–5 |
| `choose_best(noise)` | multiplicative jitter amplitude | 0–0.2 |
| `ShareRule` | proportional or ordered sharing | data driven |
| `GeneSpec.mutation_sd`, `mutate(rate)` | step size (fraction of the range), per-gene probability | 0.02–0.1, 0.01–0.1 |
| `Genealogy::with_max_nodes` | memory cap of the family graph | 10⁴–10⁶ |
| `Drive { rate, threshold }` | growth per tick, urgency threshold | species data |

## 4. Determinism

* Randomness comes only from a `SimRng` passed in by the caller. Each function
  documents how many numbers it draws. For per-agent streams that do not depend
  on how many other agents drew, pass `SimRng::derive(seed ^ tick, id.get())`
  or a forked stream.
* Ties are always broken by the lower id, slot or index. The intent order is
  total.
* No `HashMap` or `HashSet` is used anywhere. Sets and maps are `BTreeSet` and
  `BTreeMap`, and stores are sorted `Vec`s.
* All state types (`AgentStore`, `Kinematics`, `WanderState`, `Genome`,
  `GenomeSchema`, `Genealogy`, `StateMachine`, `Drive`, ...) implement serde.
  Snapshot payloads that contain them, together with the model's `SimRng`,
  replay exactly. The integration test `tests/predator_prey.rs` checks
  bit-identical replay after a postcard round trip.

## 5. Approximations and limitations

* **Floating point.** Results are reproducible on the same platform and build,
  but not bit-exact across platforms (`exp`, `atan2`, `sin` and `cos` come
  from the platform's libm).
* **Integration.** Semi-implicit Euler with a fixed `dt`. There is no mass,
  drag or turning-radius model: point masses can reverse direction within the
  acceleration limit.
* **Passability.** Only destinations are tested. A step longer than an
  obstacle is thick can tunnel through it, so keep `max_speed·dt` below one
  cell. Sliding is axis-aligned.
* **Obstacle avoidance and line of sight** are sampled. Obstacles thinner than
  the sample spacing can be missed. Avoidance is a reactive heuristic, not path
  planning: dead ends and concave obstacles can trap agents.
* **Steering targets** are taken literally. In toroidal worlds, pass
  `p + bounds.delta(p, target)` or the `Percept::position` of a wrap-aware
  query, which is already that image. Wrapped queries require indexed
  positions inside the wrapped rectangle, and a radius of half a period or
  more costs a full `O(n)` pass.
* **Separation and alignment** use the desired-velocity formulation (Shiffman's
  version), so a single neighbour at the edge of the radius already produces
  full-strength steering. Reynolds' force-proportional variant is not
  provided.
* **Pursuit** predicts from the current speed only. Reynolds' alignment-based
  reduction of the horizon is not applied.
* **Utility.** The compensation formula follows the commonly cited version of
  Dave Mark's GDC 2015 talk. The research notes mark its exact form as
  unverified. `choose_best` always takes the arg-max; it does not do weighted
  random selection among the top-N. Momentum (a bonus for the current action)
  is left to the model, for example by multiplying that action's score by
  1.1–1.25.
* **Intents.** The id tie-break favours older agents when priority and
  distance tie exactly. Models that need fairness should add a seeded jitter
  to `priority`. Proportional sharing uses `f32` with sequential subtraction:
  conservation holds in the returned order, but the shares are only
  proportional up to `f32` rounding. `resolve_matching` is greedy, so it is
  maximal but not maximum-cardinality and not a stable matching. Reciprocal
  intents (mutual mating or trade consent) are not built in; models match them
  before claiming.
* **Genomes.**
  * Genes are real-valued and linear-mapped. Log-scale traits should be
    declared over `ln(value)` and exponentiated by the model.
  * There are no diploid or discrete loci, no linkage, and no structural
    mutations.
  * `Clamp` mutation piles probability on the bounds; use
    `MutationBounds::Reflect` to avoid this.
  * Distance weighs all heritable genes equally.
* **Genealogy.**
  * One genealogy per id space: births must arrive in increasing id order, so
    agents from different `AgentStore`s (which reuse the same ids) need
    separate genealogies.
  * Parents that are not recorded (pruned or truncated ancestors, founders'
    parents) are kept in `Lineage::parents` but produce no edge.
  * Truncation cuts history: after it, `edges()` no longer reaches the
    founders.
  * With sexual reproduction, the retained ancestry is close to the whole
    population per generation, so set a cap for long runs.
* **Drives** grow linearly. Saturating or exponential dynamics, and the
  conversion of food or water into drive units, are left to the model.

## 6. Example

The full program is `crates/organism-engine/examples/wolves_and_deer.rs`. It
is compiled by `cargo clippy --all-targets`. This is its core tick:

```rust
use organism_engine::intent::resolve_matching;
use organism_engine::perception::{self, Query};
use organism_engine::{steering, ActionScore, AgentId, Claim, Consideration, Crossover, Curve, Genome,
                      Lineage, MotionLimits, choose_best};

let hunger_curve = Curve::Logistic { k: 10.0, x0: 0.5 };
let near_curve = Curve::InverseLinear;
perception::index_agents(&mut grid, &deer, |d| d.kin.position)?;

// 1. Decide from read-only state (pure: could run in parallel).
let mut claims = Vec::new();
let mut accelerations = Vec::new();
for (id, wolf) in wolves.iter() {
    let limits = MotionLimits::new(wolf.genome.express(&schema, SPEED), 1.5);
    let sight = wolf.genome.express_named(&schema, "sight").unwrap_or(10.0);
    let prey = perception::nearest(&grid, &Query::new(wolf.kin.position, sight), |_, _| true);
    let hunt = prey.map_or(0.0, |p| {
        let considerations = [
            Consideration::new(wolf.hunger.value, &hunger_curve),
            Consideration::new(p.distance / sight, &near_curve).weighted(0.5),
        ];
        ActionScore::evaluate(1.0, &considerations).value
    });
    let accel = match (choose_best(&[hunt, 0.1], None, 0.0), prey) {
        (Some(0), Some(p)) => {
            let target = deer.ids()[p.slot as usize];
            claims.push(Claim::new(id, target.get()).priority(wolf.hunger.value).distance(p.distance));
            steering::seek(&wolf.kin, p.position, limits)
        }
        _ => steering::arrive(&wolf.kin, wolf.kin.position, 1.0, limits), // rest: brake
    };
    accelerations.push((accel, limits));
}

// 2. Resolve: at most one wolf per deer and one deer per wolf.
let winners = resolve_matching(&claims);

// 3. Apply in id order, queueing births and deaths.
let (mut pack, births) = wolves.split_mut();
for ((_, wolf), (accel, limits)) in pack.iter_mut().zip(accelerations) {
    wolf.kin.integrate(accel, 0.2, limits, bounds);
    wolf.hunger.update(1.0);
}
for claim in winners.iter().filter(|c| c.distance < 1.0) {
    deer.queue_despawn(AgentId(claim.target));
    let Some(&mate_id) = pack.ids().iter().find(|&&w| w != claim.claimant) else { continue };
    let Some((parent, mate)) = pack.get_pair_mut(claim.claimant, mate_id) else { continue };
    parent.hunger.satisfy(0.6);
    let mut genome = Genome::crossover(&parent.genome, &mate.genome, &schema, Crossover::Uniform, &mut rng)?;
    genome.mutate(&schema, 0.5, &mut rng)?;
    let child_id = births.spawn(Wolf { kin: parent.kin, hunger: hungry, genome });
    let lineage = |id| genealogy.get(id).map(|n| n.lineage).ok_or(OrganismError::UnknownAgent(id));
    let (first, second) = (lineage(claim.claimant)?, lineage(mate_id)?);
    genealogy.record_birth(Lineage::offspring(child_id, &first, Some(&second), tick))?;
}

// 4. Commit.
let eaten = deer.commit().despawned.len();
wolves.commit();
```

Run it with `cargo run -p organism-engine --example wolves_and_deer`. It
prints the kills per tick, the genetic distance between two wolves and the
family edges.

## 7. How to test

Always use the agent-specific target directory:

```sh
export CARGO_TARGET_DIR=target/agents/organism-engine
cargo test   -p organism-engine                                # unit + integration + doc tests
cargo clippy -p organism-engine --all-targets -- -D warnings
cargo fmt    -p organism-engine --check
RUSTDOCFLAGS="-D warnings" cargo doc -p organism-engine --no-deps
cargo run    -p organism-engine --example wolves_and_deer
```

What the tests check:

| Area | Tests |
|---|---|
| store | ids stay stable and are never reused; iteration is in id order; spawns and despawns are deferred; 300 random batches checked against a `BTreeMap` reference model (live, queued, dead, `#0` and unissued despawns); the result does not depend on despawn order; spawn and despawn in the same batch; `split_mut` scheduling; pair access; serde round trips (JSON and postcard); rejection of corrupted data, including a zero or overflowing counter and the reserved id `#0` |
| kinematics | Euler scheme with limits; NaN and negative `dt` safety; clamp, reflect and wrap bounds with `delta`; `contains(apply(p))` for random and edge-adjacent points on valid, degenerate, inverted and infinite rectangles (including the wrap rounding regression); overflow-safe `truncate`; wall sliding and blocking; serde |
| steering | seek and flee directions; arrive braking (exact value) and convergence; pursue and evade equal to seek/flee of the hand-computed prediction for both horizon cases, a standing pursuer, and NaN or negative horizons; wander determinism and angle wrapping; separation pushing apart, with the closest neighbour dominating; alignment; cohesion; negative and NaN radii; unlimited speed and acceleration; feeler avoidance (braking, turn side, urgency rising with proximity); prioritised truncation |
| perception | nearest with filter, cone and slot tie-break; `within` sorted and equal to a brute-force count; `nearest == within[0]`; on a torus, 300 random queries (origins inside and outside the world, radii below and above half a period, random cones) against a brute-force nearest-image search: every item at most once, images and distances exact, nothing missed, `count` and `nearest` consistent; FOV edge cases; line-of-sight sampling and cap; `index_agents` mapping |
| utility | curve shapes, including `Exponential` overflow safety and NaN handling; custom curve interpolation, validation and serde; consideration weights; the compensation formula (two scores of 0.5 give 0.390625); vetoes; ties go to the lowest index; noise determinism and bounds; one draw per score |
| fsm | time in state, log contents and bound, serde |
| intent | priority, distance and id ordering; results unchanged under 5 shuffles of 200 random claims, including NaN fields; 50 random instances checked by brute force (each exclusive winner beats every rival; matching is one-to-one and every rejected claim lost to a better conflicting winner); target-before-amount order in matching; NaN and `-0.0` handling; matching versus exclusive; proportional fairness and exact conservation; shares whose `f32` total overflows; ordered serving; dispatch; serde |
| genome | schema validation messages; expression; random genomes; mutation mean and standard deviation over 20,000 samples; mutation rate statistics; a NaN rate draws nothing; clamping pile-up probability (about 0.484); reflection without pile-up; uniform crossover (each gene from one parent, about 50/50); one-point cut distribution (uniform over `1..n`); the contract free functions match the methods; rejection of mismatched genomes; distance metric; serde with defaults |
| lineage | generations; a dying line pruned with its ancestors; two-parent families; error cases; out-of-order births, younger parents and re-recorded pruned ids rejected without side effects; the node cap truncating the oldest first and never the living; a random 3,000-tick population whose recorded set equals exactly the living plus their ancestors; a capped population with 200 immortal founders checked every 50 ticks (exactly the living alive, no dead leaves, `births = pruned + truncated + nodes`, `nodes ≤ max(cap, living)`, postcard round trip passes validation); serde round trip and rejection of corrupted links |
| drive | growth, saturation, satisfaction, urgency, recovery, validation, `most_urgent` tie-breaks |
| integration (`tests/predator_prey.rs`) | an 80-agent predator–prey world using every module, run both walled (reflecting edges) and on a torus (wrap-aware perception, steering towards perceived images): same seed gives a bit-identical world, a different seed gives a different world; a postcard snapshot at tick 150 replays the next 150 ticks exactly; the genealogy's living count equals the store size; on the torus agents are perceived across the edges, in the walled world never |

## 8. References

* C. W. Reynolds, "Flocks, herds and schools: a distributed behavioral model",
  *SIGGRAPH '87*, Computer Graphics 21(4):25–34, doi:10.1145/37402.37406.
* C. W. Reynolds, "Steering Behaviors For Autonomous Characters", GDC 1999,
  <https://www.red3d.com/cwr/steer/gdc99/>.
* I. Millington, J. Funge, *Artificial Intelligence for Games*, 2nd ed.,
  Morgan Kaufmann, 2009: kinematic and steering movement, pursue and evade
  prediction.
* D. Shiffman, *The Nature of Code*, 2012, ch. 6, "Autonomous Agents": the
  desired-velocity formulation of the boids rules.
* D. Mark, *Behavioral Mathematics for Game AI*, Course Technology, 2009.
  D. Mark, "Building a Better Centaur: AI at Massive Scale", GDC AI Summit
  2015: the Infinite Axis Utility System and the compensation factor;
  <https://gameai.com/iaus.php>.
* J. Kehoe, *The Specification of Sugarscape*, 2016, arXiv:1505.06012. It
  discusses the need for explicit conflict resolution in parallel agent
  updates (cited via `docs/research/ecology-alife.md` §7.7).
* B. A. Huberman, N. S. Glance, "Evolutionary games and computer simulations",
  *PNAS* 90:7716–7718 (1993): synchronous and asynchronous updating.
* D. Blackman, S. Vigna, "Scrambled linear pseudorandom number generators"
  (2018): xoshiro256++, the `SimRng` used for every draw.
* Project research notes: `docs/research/ecology-alife.md` §7 (agent AI layers,
  spatial hashing, the intent → sort → resolve pattern) and §8 (mutation
  bounds, recombination).
