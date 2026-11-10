//! Rule semantics of the custom-life simulation: priorities and slots,
//! cooldowns, costs, conflict resolution, conservation, events, commands,
//! edits, parameters, views, end conditions and determinism.

mod common;

use common::system;
use rule_engine::dsl::parse_rule;
use rule_engine::{CustomLifeCommand, CustomLifeSim, CustomLifeViewRequest, LifeSystem, PaintMode};
use simulation_core::{ApplyPolicy, ParamValue, Simulation, Topology};

/// One world, two types; `rules` are DSL rule strings.
fn world(types: &str, rules: &[&str], extra: &str) -> LifeSystem {
    let mut sys = system(&format!(
        r#"(name: "t", entity_types: [{types}],
            world: (width: 16, height: 16, resources: [(name: "food", initial: Uniform(1.0), max: 1.0)]{extra}))"#
    ));
    sys.rules = rules.iter().map(|r| parse_rule(r).unwrap_or_else(|d| panic!("{d:?}"))).collect();
    sys
}

fn at(x: f32, y: f32) -> String {
    format!("initial: (count: 1, distribution: Cluster(x: {x}, y: {y}, radius: 0.0))")
}

fn sim(sys: LifeSystem) -> CustomLifeSim {
    CustomLifeSim::new(sys, 1).unwrap_or_else(|e| panic!("{e}"))
}

fn fired(sim: &CustomLifeSim, name: &str) -> u64 {
    sim.compiled().rules().iter().zip(sim.rule_fire_totals()).filter(|(r, _)| r.name == name).map(|(_, n)| *n).sum()
}

fn energy_of(sim: &CustomLifeSim, kind: u16) -> f64 {
    sim.entities().filter(|(_, e)| e.kind == kind).map(|(_, e)| e.energy).sum()
}

const STILL: &str = "energy: (initial: 10.0, max: 100.0, metabolism: 0.0, starvation: false)";

#[test]
fn higher_priority_claims_the_slot_and_lower_rules_are_skipped() {
    let t = format!(r#"(name: "a", movement: RandomWalk(speed: 1.0), {STILL}, {})"#, at(8.0, 8.0));
    let mut s = sim(world(
        &t,
        &[
            "rule low for a: then wander() priority 1",
            "rule high for a: then stay() priority 5",
            "rule effect for a: then add energy += 1 priority 0",
        ],
        "",
    ));
    s.advance(10);
    assert_eq!(fired(&s, "high"), 10);
    assert_eq!(fired(&s, "low"), 0, "the movement slot is taken");
    assert_eq!(fired(&s, "effect"), 10, "effects never conflict");
    let (_, e) = s.entities().next().unwrap();
    assert_eq!(e.kin.position.x, 8.0, "stay() keeps the entity still");
    assert_eq!(e.energy, 20.0);
}

#[test]
fn cooldown_once_probability_and_costs() {
    let t = format!(r#"(name: "a", {STILL}, initial: (count: 200, distribution: Uniform))"#);
    let mut s = sim(world(
        &t,
        &[
            "rule slow for a: then add energy += 0 cooldown 5",
            "rule single for a: then add energy += 0 once",
            "rule coin for a: then add energy += 0 probability 0.25",
            "rule paid for a: then add energy += 0 cost energy 3",
        ],
        "",
    ));
    s.advance(20);
    assert_eq!(fired(&s, "slow"), 200 * 4, "ticks 0, 5, 10, 15");
    assert_eq!(fired(&s, "single"), 200);
    let coin = fired(&s, "coin") as f64 / (200.0 * 20.0);
    assert!((coin - 0.25).abs() < 0.03, "probability 0.25 → {coin}");
    assert_eq!(fired(&s, "paid"), 200 * 3, "10 energy pays for three firings of cost 3");
    assert!(s.entities().all(|(_, e)| (e.energy - 1.0).abs() < 1e-9));
}

#[test]
fn contested_prey_is_eaten_once_and_losers_pay_nothing() {
    let hunter = |x: f32| format!(r#"(name: "h{x}", {STILL}, {})"#, at(x, 8.0));
    let types = format!(
        r#"{}, {}, (name: "prey", energy: (initial: 8.0, max: 10.0, starvation: false), {})"#,
        hunter(7.5),
        hunter(8.6),
        at(8.0, 8.0)
    );
    let types = types.replace("name: \"h7.5\"", "name: \"near\"").replace("name: \"h8.6\"", "name: \"far\"");
    let mut s = sim(world(
        &types,
        &[
            "rule eat for near: on_collision with prey then eat(other, efficiency=0.5) cost energy 2",
            "rule eat for far: on_collision with prey then eat(other, efficiency=0.5) cost energy 2",
        ],
        "",
    ));
    s.step();
    assert_eq!(s.population("prey"), 0);
    // Equal priority: the closer claimant wins; the loser's firing is cancelled.
    assert_eq!(energy_of(&s, 0), 10.0 - 2.0 + 4.0);
    assert_eq!(energy_of(&s, 1), 10.0, "no cost charged to the loser");
    assert_eq!(s.last_tick_stats().interactions_refused, 1);
    assert_eq!(s.last_tick_stats().eaten, 1);
}

#[test]
fn consumption_is_shared_proportionally_and_conserves_resources() {
    let t = r#"(name: "a", energy: (initial: 0.0, max: 100.0, metabolism: 0.0, starvation: false), initial: (count: 3, distribution: Cluster(x: 4.5, y: 4.5, radius: 0.0)))"#.to_string();
    let mut s = sim(world(&t, &["rule eat for a: then consume(food, 0.5)"], ""));
    let total = |s: &CustomLifeSim| s.resource_total("food") + energy_of(s, 0);
    let before = total(&s);
    s.step();
    let e: Vec<f64> = s.entities().map(|(_, e)| e.energy).collect();
    assert!(e.iter().all(|x| (x - 1.0 / 3.0).abs() < 1e-6), "1.0 shared by three requests of 0.5: {e:?}");
    assert!((total(&s) - before).abs() < 1e-4, "resource moves into energy without loss");
    s.advance(10);
    assert!((total(&s) - before).abs() < 1e-4);
}

#[test]
fn reproduction_transfers_energy_without_creating_it() {
    let t = format!(
        r#"(name: "a", energy: (initial: 30.0, max: 100.0, metabolism: 0.0, starvation: false),
            reproduction: (mode: Asexual, cost: 10.0, cooldown: 3, litter: 2), {})"#,
        at(8.0, 8.0)
    );
    let mut s = sim(world(&t, &["rule split for a: when energy > 12 then reproduce()"], ""));
    s.step();
    assert_eq!(s.population("a"), 3);
    assert!((energy_of(&s, 0) - 30.0).abs() < 1e-9, "energy is conserved");
    let mut gens: Vec<u32> = s.entities().map(|(_, e)| e.generation).collect();
    gens.sort_unstable();
    assert_eq!(gens, vec![0, 1, 1]);
    s.step();
    assert_eq!(s.population("a"), 3, "the parent's reproduction cooldown");
    // Sexual reproduction: both parents pay.
    let t = r#"(name: "m", energy: (initial: 30.0, max: 100.0, metabolism: 0.0, starvation: false),
            reproduction: (mode: Sexual, cost: 10.0), perception: 3.0, initial: (count: 2, distribution: Cluster(x: 8.0, y: 8.0, radius: 1.0)))"#;
    let mut s = sim(world(t, &["rule mate for m: then reproduce(partner=nearest(m))"], ""));
    s.step();
    assert_eq!(s.population("m"), 3, "one pair, one offspring (the partner is busy)");
    let mut energies: Vec<f64> = s.entities().map(|(_, e)| e.energy).collect();
    energies.sort_by(f64::total_cmp);
    assert_eq!(energies, vec![20.0, 20.0, 20.0]);
}

#[test]
fn events_birth_death_state_and_transform() {
    let types = format!(
        r#"(name: "egg", states: ["fresh", "ripe"], {STILL}, reproduction: (mode: Asexual, cost: 1.0),
            properties: [(name: "mark", default: 0.0, min: 0.0, max: 10.0)], {}),
           (name: "chick", {STILL})"#,
        at(8.0, 8.0)
    );
    let mut s = sim(world(
        &types,
        &[
            "rule hello for egg: on_birth then add mark += 1",
            "rule ripen for egg: when age >= 2 and state == fresh then set_state(ripe)",
            "rule hatch for egg: on_state_enter(ripe) then transform_into(chick)",
            "rule farewell for chick: when state_age >= 1 then die()",
            "rule last for chick: on_death then spawn(egg, 2), emit(food, 5)",
        ],
        "",
    ));
    s.step();
    let (_, e) = s.entities().next().unwrap();
    assert_eq!(e.props[0], 1.0, "on_birth fired once on the first tick");
    s.advance(2);
    assert_eq!(s.entities().next().unwrap().1.state, 1, "ripe after age 2");
    s.step();
    assert_eq!(s.population("chick"), 1, "on_state_enter fires on the tick after the change");
    let food = s.resource_total("food");
    s.advance(2);
    assert_eq!(s.population("chick"), 0);
    assert_eq!(s.population("egg"), 2, "on_death spawned two eggs");
    assert!((s.resource_total("food") - food - 5.0).abs() < 1e-4);
    s.step();
    assert_eq!(fired(&s, "hello"), 3, "new eggs run on_birth on their first tick");
}

#[test]
fn collisions_bind_other_and_read_its_properties() {
    let types = format!(
        r#"(name: "a", {STILL}, {}), (name: "b", energy: (initial: 7.0, max: 10.0, starvation: false), {})"#,
        at(8.0, 8.0),
        at(8.5, 8.0)
    );
    let mut s = sim(world(
        &types,
        &["rule copy for a: on_collision with b when other.energy == 7 then set energy = other.energy * 2"],
        "",
    ));
    s.step();
    assert_eq!(energy_of(&s, 0), 14.0);
}

#[test]
fn carried_resources_emit_trade_and_drop_conservatively() {
    let types = format!(
        r#"(name: "miner", {STILL}, carries: [(resource: "food", initial: 0.0, capacity: 5.0), (resource: "ore", initial: 4.0, capacity: 10.0)], {}),
           (name: "trader", {STILL}, carries: [(resource: "food", initial: 3.0, capacity: 10.0), (resource: "ore", initial: 0.0, capacity: 10.0)], {})"#,
        at(4.5, 4.5),
        at(5.0, 4.5)
    );
    let mut sys = world(
        &types,
        &["rule dig for miner: then consume(food, 1)", "rule drop for trader: when tick == 3 then die()"],
        "",
    );
    sys.world.resources.push(rule_engine::def::ResourceField::new("ore"));
    sys.relationships.push(ron::from_str(r#"(subject: "miner", kind: TradesWith(give: "ore", give_amount: 2.0, receive: "food", receive_amount: 1.0), object: "trader")"#).unwrap());
    let mut s = sim(sys);
    let carried = |s: &CustomLifeSim, i: usize| -> f64 { s.entities().map(|(_, e)| e.carried[i]).sum() };
    let food0 = s.resource_total("food") + carried(&s, 0);
    let ore0 = s.resource_total("ore") + carried(&s, 1);
    s.advance(5);
    assert!(fired(&s, "miner trades_with trader: trade") >= 1, "the miner traded ore for food");
    let food1 = s.resource_total("food") + carried(&s, 0);
    let ore1 = s.resource_total("ore") + carried(&s, 1);
    assert!((food1 - food0).abs() < 1e-4, "food conserved: {food0} -> {food1}");
    assert!((ore1 - ore0).abs() < 1e-4, "ore conserved: {ore0} -> {ore1}");
    assert_eq!(s.population("trader"), 0);
    assert!(s.resource_total("ore") > 0.0, "the dead trader dropped its ore");
}

#[test]
fn commands_validate_their_input() {
    let t = format!(r#"(name: "a", {STILL}, {})"#, at(8.0, 8.0));
    let mut s = sim(world(&t, &[], ""));
    s.apply(CustomLifeCommand::Spawn { entity: "a".into(), x: 2.0, y: 3.0 }).unwrap();
    assert_eq!(s.population("a"), 2);
    assert!(
        s.apply(CustomLifeCommand::Spawn { entity: "dragon".into(), x: 2.0, y: 3.0 })
            .unwrap_err()
            .to_string()
            .contains("unknown entity type `dragon`")
    );
    assert!(s.apply(CustomLifeCommand::Spawn { entity: "a".into(), x: 99.0, y: 3.0 }).is_err());
    let id = s.entities().next().unwrap().0.get();
    s.apply(CustomLifeCommand::Remove { id }).unwrap();
    assert_eq!(s.population("a"), 1);
    assert!(s.apply(CustomLifeCommand::Remove { id }).is_err());
    let before = s.resource_total("food");
    // A brush of radius 1 centred on a cell centre covers that cell and its 4 neighbours.
    s.apply(CustomLifeCommand::PaintResource {
        resource: "food".into(),
        x: 8.5,
        y: 8.5,
        radius: 1.0,
        amount: 2.0,
        mode: PaintMode::Add,
    })
    .unwrap();
    assert!((s.resource_total("food") - before - 10.0).abs() < 1e-4, "5 cells × 2");
    s.apply(CustomLifeCommand::PaintResource {
        resource: "food".into(),
        x: 8.5,
        y: 8.5,
        radius: 1.0,
        amount: 0.0,
        mode: PaintMode::Set,
    })
    .unwrap();
    assert!((s.resource_total("food") - before + 5.0).abs() < 1e-4);
    assert!(
        s.apply(CustomLifeCommand::PaintResource {
            resource: "lava".into(),
            x: 1.0,
            y: 1.0,
            radius: 1.0,
            amount: 1.0,
            mode: PaintMode::Add
        })
        .is_err()
    );
}

#[test]
fn definition_edits_apply_next_tick_or_on_reset() {
    let t = format!(r#"(name: "a", {STILL}, initial: (count: 5, distribution: Uniform))"#);
    let mut s = sim(world(&t, &[], ""));
    s.advance(3);
    let mut edited = s.system().clone();
    edited.rules.push(parse_rule("rule gain for a: then add energy += 1").unwrap());
    let report = s.edit_system(edited.clone()).unwrap();
    assert_eq!(report.policy, ApplyPolicy::NextTick);
    s.step();
    assert_eq!(energy_of(&s, 0), 5.0 * 11.0, "the new rule runs on the next tick for live entities");
    assert_eq!(s.tick(), 4, "no reset");
    // Structural edit: stored for the next reset, the run is untouched.
    let mut structural = edited.clone();
    structural.entity_types.push(rule_engine::EntityType::new("b"));
    structural.entity_types[1].initial.count = 2;
    let report = s.edit_system(structural).unwrap();
    assert_eq!(report.policy, ApplyPolicy::RequiresReset);
    assert_eq!(s.system().entity_types.len(), 1);
    s.reset();
    assert_eq!((s.tick(), s.population("b")), (0, 2));
    // Invalid edits are rejected and change nothing.
    let mut broken = s.system().clone();
    broken.rules.push(parse_rule("rule bad for a: then fly()").unwrap());
    assert!(s.apply(CustomLifeCommand::ReplaceSystem(Box::new(broken))).is_err());
    assert_eq!(s.system().rules.len(), 1);
}

#[test]
fn parameters_report_their_policy() {
    let t = format!(r#"(name: "a", {STILL}, initial: (count: 5, distribution: Uniform))"#);
    let mut s = sim(world(
        &t,
        &[],
        ", random_events: [(name: \"rain\", probability: 0.0, effect: ResourcePulse(resource: \"food\", amount: 1.0, radius: 2.0))]",
    ));
    let d = s.descriptor();
    for key in [
        "seed",
        "width",
        "topology",
        "max_entities",
        "pause_on_end",
        "resource.food.regen",
        "entity.a.metabolism",
        "entity.a.initial_count",
        "event.rain.probability",
    ] {
        assert!(d.param(key).is_some(), "missing parameter {key}");
    }
    assert_eq!(s.set_param("resource.food.regen", ParamValue::Float(0.5)).unwrap(), ApplyPolicy::NextTick);
    assert_eq!(s.system().world.resources[0].regen, 0.5);
    assert_eq!(s.set_param("width", ParamValue::Int(32)).unwrap(), ApplyPolicy::RequiresReset);
    assert_eq!(s.system().world.width, 16, "applies at reset");
    assert_eq!(s.set_param("pause_on_end", ParamValue::Bool(false)).unwrap(), ApplyPolicy::Immediate);
    assert!(s.set_param("resource.food.diffusion", ParamValue::Float(0.9)).is_err(), "out of range");
    assert!(s.set_param("nope", ParamValue::Int(1)).is_err());
    assert_eq!(s.set_param("event.rain.probability", ParamValue::Float(1.0)).unwrap(), ApplyPolicy::NextTick);
    s.step();
    assert_eq!(s.last_tick_stats().random_events, 1);
    assert!(s.events().iter().any(|e| e.kind == "random_event"));
    s.reset();
    assert_eq!(s.system().world.width, 32);
    assert_eq!(s.params().get("width"), Some(&ParamValue::Int(32)));
}

#[test]
fn parameter_defaults_are_the_design_as_last_loaded_or_applied() {
    let t = format!(r#"(name: "a", {STILL}, initial: (count: 5, distribution: Uniform))"#);
    let mut s = sim(world(&t, &[], ""));
    let default = |s: &CustomLifeSim, key: &str| s.descriptor().param(key).unwrap().default.clone();
    // Parameter changes move the values, not the defaults, so a UI can
    // offer going back to the design.
    s.set_param("entity.a.metabolism", ParamValue::Float(0.5)).unwrap();
    s.set_param("width", ParamValue::Int(32)).unwrap();
    s.reset();
    assert_eq!(s.params().get("entity.a.metabolism"), Some(&ParamValue::Float(0.5)));
    assert_eq!(default(&s, "entity.a.metabolism"), ParamValue::Float(0.0));
    assert_eq!(s.params().get("width"), Some(&ParamValue::Int(32)));
    assert_eq!(default(&s, "width"), ParamValue::Int(16));
    let tuned = s.snapshot().unwrap();
    // Applying a definition makes it the design.
    let mut applied = s.system().clone();
    applied.entity_types[0].energy.metabolism = 0.25;
    s.edit_system(applied).unwrap();
    assert_eq!(default(&s, "entity.a.metabolism"), ParamValue::Float(0.25));
    assert_eq!(default(&s, "width"), ParamValue::Int(32));
    // Undo, history jumps and loads bring back the design of their state.
    s.restore(&tuned).unwrap();
    assert_eq!(s.params().get("entity.a.metabolism"), Some(&ParamValue::Float(0.5)));
    assert_eq!(default(&s, "entity.a.metabolism"), ParamValue::Float(0.0), "snapshots carry the design");
    // Defaults stay valid: a design value outside the running range, or a
    // parameter the design lacks, defaults to the running value.
    s.set_param("entity.a.initial_count", ParamValue::Int(2)).unwrap();
    s.reset();
    s.set_param("max_entities", ParamValue::Int(3)).unwrap();
    assert_eq!(default(&s, "entity.a.initial_count"), ParamValue::Int(2), "the design's 5 exceeds the cap of 3");
    let mut renamed = s.system().clone();
    renamed.entity_types[0].name = "b".into();
    s.edit_system(renamed).unwrap();
    assert_eq!(default(&s, "entity.a.metabolism"), ParamValue::Float(0.5), "`a` is renamed at the next reset");
}

#[test]
fn end_conditions_finish_the_run_unless_disabled() {
    let t = format!(r#"(name: "a", {STILL}, lifecycle: (max_age: Some(3)), {})"#, at(8.0, 8.0));
    let mut sys = world(&t, &[], "");
    sys.end_conditions.push(rule_engine::def::EndCondition::Extinction("a".into()));
    let mut s = sim(sys.clone());
    let advanced = s.advance(100);
    assert_eq!(advanced, 3, "advance stops when the end condition is met");
    assert!(s.is_finished());
    assert_eq!(s.finished_reason(), Some("`a` went extinct"));
    s.apply(CustomLifeCommand::SetPauseOnEnd(false)).unwrap();
    assert!(!s.is_finished());
    sys.end_conditions = vec![rule_engine::def::EndCondition::TickLimit(7)];
    let mut s = sim(sys);
    assert_eq!(s.advance(100), 7);
}

#[test]
fn views_carry_entities_fields_legend_graph_and_rules() {
    let types = format!(
        r#"(name: "a", icon: "🐇", color: (1.0, 0.0, 0.0, 1.0), states: ["x", "y"], {STILL}, initial: (count: 4, distribution: Uniform)), (name: "b", {STILL})"#
    );
    let mut sys = world(
        &types,
        &["rule r for a: then stay()"],
        ", environment: [(name: \"heat\", pattern: Gradient(from: 0.0, to: 1.0, axis: X))]",
    );
    sys.relationships.push(ron::from_str(r#"(subject: "a", kind: Chases(), object: "b")"#).unwrap());
    sys.entity_types[0].movement = rule_engine::def::MovementDef::RandomWalk { speed: 1.0 };
    let mut s = sim(sys);
    s.step();
    let v = s.view(&CustomLifeViewRequest { field_resolution: 4, include_rules: true, ..Default::default() });
    assert_eq!(v.entities.len(), 4);
    assert_eq!(v.entities[0].color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(v.fields.len(), 2);
    assert_eq!((v.fields[0].width, v.fields[0].height, v.fields[0].cell_width), (4, 4, 4.0));
    assert!(v.fields[1].min < 0.2 && v.fields[1].max > 0.8, "environment gradient sampled");
    assert_eq!(v.legend[0].icon, "🐇");
    assert_eq!(v.legend[0].count, 4);
    assert_eq!(v.legend[0].states, vec!["x", "y"]);
    assert_eq!(v.graph.edges.len(), 1);
    assert_eq!(v.graph.edges[0].relation, "chases");
    assert_eq!(v.rules.len(), 2);
    assert!(v.rules.iter().any(|r| r.origin.is_generated() && r.text.contains("move_towards")));
    let d = s.descriptor();
    assert!(d.rules.contains("rule r for a:"), "descriptor shows the executed rules");
    assert!(s.metrics().get("population.a").is_some() && s.metrics().get("resource.food").is_some());
}

#[test]
fn twisted_topologies_keep_entities_inside_the_world() {
    for topology in [Topology::Bounded, Topology::Torus, Topology::KleinBottle, Topology::CrossSurface] {
        let t = format!(
            r#"(name: "a", movement: Directed(speed: 1.5, turn: 0.3), {STILL}, initial: (count: 50, distribution: Uniform))"#
        );
        let mut sys = world(&t, &[], "");
        sys.world.topology = topology;
        let mut s = sim(sys);
        s.advance(200);
        for (_, e) in s.entities() {
            let p = e.kin.position;
            assert!((0.0..=16.0).contains(&p.x) && (0.0..=16.0).contains(&p.y), "{topology:?}: {p:?}");
        }
    }
}

#[test]
fn decisions_do_not_depend_on_thread_count() {
    let t = r#"(name: "a", movement: RandomWalk(speed: 0.7), energy: (initial: 5.0, max: 20.0, metabolism: 0.05), reproduction: (mode: Asexual, cost: 3.0), initial: (count: 400, distribution: Uniform))"#.to_string();
    let sys = world(
        &t,
        &[
            "rule eat for a: when field(food) > 0 then consume(food, 0.3)",
            "rule split for a: when energy > 8 and nearby(a, radius=2) < 3 then reproduce()",
        ],
        "",
    );
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| {
            let mut s = sim(sys.clone());
            s.advance(30);
            (s.metrics(), s.entities().map(|(id, e)| (id, e.kin.position, e.energy)).collect::<Vec<_>>())
        })
    };
    assert_eq!(run(1), run(4));
}

// ---------------------------------------------------------------- review fixes

#[test]
fn an_eaten_entity_does_not_also_reproduce_so_energy_is_conserved() {
    let types = format!(
        r#"(name: "hunter", energy: (initial: 10.0, max: 1000.0, metabolism: 0.0, starvation: false), {}),
           (name: "prey", energy: (initial: 20.0, max: 1000.0, metabolism: 0.0, starvation: false),
            reproduction: (mode: Asexual, cost: 10.0), {})"#,
        at(8.0, 8.0),
        at(8.4, 8.0)
    );
    let mut s = sim(world(
        &types,
        &["rule eat for hunter: on_collision with prey then eat(other)", "rule split for prey: then reproduce()"],
        "",
    ));
    let total = |s: &CustomLifeSim| energy_of(s, 0) + energy_of(s, 1);
    let before = total(&s);
    s.step();
    assert_eq!(s.population("prey"), 0, "the prey was eaten and left no offspring");
    assert!((total(&s) - before).abs() < 1e-9, "energy {before} -> {}", total(&s));
    assert_eq!(fired(&s, "split"), 1, "the decision was taken (and counted) but not carried out");
}

#[test]
fn trades_that_cannot_complete_are_never_charged_or_counted() {
    let types = format!(
        r#"(name: "miner", {STILL}, carries: [(resource: "food", initial: 0.0, capacity: 5.0), (resource: "ore", initial: 4.0, capacity: 10.0)], {}),
           (name: "trader", {STILL}, carries: [(resource: "food", initial: 0.5, capacity: 10.0), (resource: "ore", initial: 0.0, capacity: 10.0)], {})"#,
        at(4.5, 4.5),
        at(5.0, 4.5)
    );
    let mut sys = world(&types, &[], "");
    sys.world.resources.push(rule_engine::def::ResourceField::new("ore"));
    // The trader holds 0.5 food but the miner asks for 1: the trade is impossible.
    sys.rules.push(
        parse_rule(
            "rule swap for miner: on_collision with trader \
             then trade(other, give=ore, amount=2, receive=food, receive_amount=1) cost energy 1 cooldown 3",
        )
        .unwrap(),
    );
    let mut s = sim(sys);
    s.advance(5);
    assert_eq!(fired(&s, "swap"), 0);
    assert_eq!(s.last_tick_stats().trades, 0);
    assert_eq!(energy_of(&s, 0), 10.0, "no cost charged");
}

#[test]
fn script_random_draws_are_independent_of_how_actions_are_carried_out() {
    // P(random() < 0.5) × P(reproduction roll 0.5) = 0.25; sharing one stream
    // between the script and the roll made it 0.5.
    let mut sys = system(
        r#"(name: "r", entity_types: [(name: "cell", energy: (initial: 10.0, max: 100.0, starvation: false),
            reproduction: (mode: Asexual, cost: 0.0, cooldown: 1000), initial: (count: 4000, distribution: Uniform))],
            world: (width: 64, height: 64, max_entities: 20000))"#,
    );
    sys.scripts.push(rule_engine::ScriptRule {
        name: "maybe".into(),
        entity: "cell".into(),
        event: Default::default(),
        priority: 0,
        source: "fn run(me) { if random() < 0.5 { reproduce(0.5); } }".into(),
        enabled: true,
    });
    let mut s = sim(sys);
    s.step();
    let rate = (s.population("cell") - 4000) as f64 / 4000.0;
    assert!((rate - 0.25).abs() < 0.03, "birth rate {rate}");
}

#[test]
fn the_view_says_when_an_edit_waits_for_a_reset() {
    let t = format!(r#"(name: "a", {STILL}, {})"#, at(8.0, 8.0));
    let mut s = sim(world(&t, &[], ""));
    assert!(!s.view(&Default::default()).reset_pending);
    let mut wider = s.system().clone();
    wider.world.width = 32;
    s.edit_system(wider).unwrap();
    let v = s.view(&Default::default());
    assert!(v.reset_pending && s.reset_pending());
    s.reset();
    assert!(!s.reset_pending());
    s.set_param("seed", ParamValue::Int(99)).unwrap();
    assert!(s.reset_pending(), "a new seed also waits for the reset");
    s.reset();
    assert!(!s.reset_pending());
    assert!(s.view(&Default::default()).diagnostics.is_empty());
}

#[test]
fn catastrophe_victims_run_their_on_death_rules() {
    let types = format!(r#"(name: "a", {STILL}, initial: (count: 6, distribution: Uniform)), (name: "seed", {STILL})"#);
    let mut sys = world(
        &types,
        &["rule last for a: on_death then spawn(seed, 1), emit(food, 2)"],
        ", random_events: [(name: \"meteor\", probability: 1.0, effect: Catastrophe(radius: 100.0, kill_probability: 1.0, entity: Some(\"a\")))]",
    );
    sys.world.resources[0].initial = rule_engine::def::FieldInit::Uniform(0.0);
    let mut s = sim(sys);
    s.step();
    assert_eq!(s.population("a"), 0);
    assert_eq!(s.population("seed"), 6, "each victim left a seed");
    assert!((s.resource_total("food") - 12.0).abs() < 1e-4);
    assert_eq!(fired(&s, "last"), 6);
    assert!(s.events().iter().any(|e| e.kind == "extinction" && e.message.contains("`a`")));
}

#[test]
fn lowering_a_capacity_releases_the_excess_stock() {
    let t = format!(
        r#"(name: "a", {STILL}, carries: [(resource: "food", initial: 4.0, capacity: 5.0)], {})"#,
        at(8.5, 8.5)
    );
    let mut s = sim(world(&t, &[], ""));
    let total = |s: &CustomLifeSim| s.resource_total("food") + s.entities().map(|(_, e)| e.carried[0]).sum::<f64>();
    let before = total(&s);
    let mut edited = s.system().clone();
    edited.entity_types[0].carries[0].capacity = 1.0;
    edited.entity_types[0].carries[0].initial = 1.0;
    assert_eq!(s.edit_system(edited).unwrap().policy, ApplyPolicy::NextTick);
    assert_eq!(s.entities().next().unwrap().1.carried[0], 1.0);
    assert!((total(&s) - before).abs() < 1e-4, "{before} -> {}", total(&s));
}
