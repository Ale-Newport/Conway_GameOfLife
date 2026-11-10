//! Validation of definitions and DSL rules: precise diagnostics with codes
//! and line:column positions; relationship compilation.

mod common;

use common::system;
use rule_engine::dsl::parse_rule;
use rule_engine::{Diagnostic, LifeSystem, RuleEngineError, compile};

fn base() -> LifeSystem {
    system(
        r#"(
        name: "v",
        entity_types: [
            (name: "rabbit", movement: RandomWalk(speed: 0.5), states: ["calm", "scared"],
             properties: [(name: "hunger", default: 0.0, min: 0.0, max: 100.0)],
             reproduction: (mode: Asexual, cost: 5.0)),
            (name: "grass"),
            (name: "fox", movement: Directed(speed: 0.8, turn: 0.5), reproduction: (mode: Sexual, cost: 10.0)),
        ],
        world: (width: 32, height: 32, resources: [(name: "water")],
                environment: [(name: "light", pattern: Constant(1.0))]),
    )"#,
    )
}

/// Diagnostics of `base()` plus one rule.
fn diagnose(rule: &str) -> Vec<Diagnostic> {
    let mut sys = base();
    sys.rules.push(parse_rule(rule).unwrap_or_else(|d| panic!("syntax: {d:?}")));
    match compile(&sys) {
        Ok(c) => c.warnings().to_vec(),
        Err(RuleEngineError::Invalid(d)) => d,
        Err(e) => panic!("{e}"),
    }
}

fn first_error(rule: &str) -> Diagnostic {
    diagnose(rule).into_iter().find(Diagnostic::is_error).unwrap_or_else(|| panic!("no error for {rule}"))
}

#[test]
fn valid_rules_compile() {
    for rule in [
        "rule a for rabbit: when hunger > 50 and nearby(grass, radius=3) > 0 then move_towards(nearest(grass))",
        "rule b for rabbit: on_collision with grass when self.state == \"calm\" then eat(other, efficiency=0.5)",
        "rule c for rabbit: when field(water) > 0.1 or field(light) < 0.5 then consume(water, 0.2), add hunger -= 1",
        "rule d for rabbit: when random() < 0.1 and tick > 10 then set_state(scared), set speed = speed * 2",
        "rule e for rabbit: on_state_enter(scared) then move_away(nearest(fox, radius=8), speed=1.2) cooldown 5",
        "rule f for fox: when exists(nearest(same_species)) then reproduce(partner=nearest(fox), probability=0.2)",
        "rule g for rabbit: on_death then spawn(grass, 2), emit(water, 1)",
        "rule h for rabbit: on_collision when other.type == fox and other.energy > 5 then die() priority 100",
        "rule i for rabbit: when population(any) < 50 then reproduce() cost energy 1 produces scared",
    ] {
        let d = diagnose(rule);
        assert!(d.iter().all(|d| !d.is_error()), "{rule}: {d:?}");
    }
}

#[test]
fn unknown_names_are_reported_with_positions_and_suggestions() {
    let d = first_error("rule a for rabbit:\n    when enrgy < 5\n    then die()");
    assert_eq!(d.code, "E001");
    let span = d.span.unwrap();
    assert_eq!((span.line, span.col), (2, 10), "{d}");
    assert_eq!(d.help.as_deref(), Some("did you mean `energy`?"));
    assert_eq!(d.context.as_deref(), Some("rule `a` (rabbit)"));

    let d = first_error("rule a for rabit: then die()");
    assert_eq!((d.code.as_str(), d.help.as_deref()), ("E003", Some("did you mean `rabbit`?")));
    let d = first_error("rule a for rabbit: when nearby(foxx) > 0 then die()");
    assert_eq!(d.code, "E003");
    assert!(d.message.contains("`foxx`"));
    let d = first_error("rule a for rabbit: when state == \"angry\" then die()");
    assert_eq!(d.code, "E003");
    assert!(d.help.unwrap().contains("calm, scared"));
    let d = first_error("rule a for rabbit: when field(lava) > 0 then die()");
    assert_eq!(d.code, "E003");
    let d = first_error("rule a for rabbit: when nearbyy(fox) > 0 then die()");
    assert_eq!((d.code.as_str(), d.help.as_deref()), ("E014", Some("did you mean `nearby`?")));
    let d = first_error("rule a for rabbit: then jump()");
    assert_eq!(d.code, "E014");
    let d = first_error("rule a for rabbit: when eat(other) then die()");
    assert!(d.help.unwrap().contains("is an action"));
}

#[test]
fn arity_and_argument_errors() {
    let d = first_error("rule a for rabbit: when nearby() > 0 then die()");
    assert_eq!(d.code, "E004");
    assert!(d.message.contains("needs the argument `kind`"), "{d}");
    let d = first_error("rule a for rabbit: when nearby(fox, 3, 4) > 0 then die()");
    assert!(d.message.contains("at most 2"), "{d}");
    let d = first_error("rule a for rabbit: when nearby(fox, range=3) > 0 then die()");
    assert!(d.message.contains("no argument `range`"), "{d}");
    let d = first_error("rule a for rabbit: when nearby(kind=fox, 3) > 0 then die()");
    assert!(d.message.contains("positional argument after a named one"), "{d}");
    let d = first_error("rule a for rabbit: when nearby(fox, radius=50) > 0 then die()");
    assert_eq!(d.code, "E006");
    let d = first_error("rule a for rabbit: then reproduce(probability=1.5)");
    assert_eq!(d.code, "E005");
}

#[test]
fn type_mismatches() {
    let d = first_error("rule a for rabbit: when energy then die()");
    assert_eq!(d.code, "E002");
    assert!(d.help.unwrap().contains("compare it"));
    let d = first_error("rule a for rabbit: when energy < \"food\" then die()");
    assert_eq!(d.code, "E002");
    let d = first_error("rule a for rabbit: when state < 3 then die()");
    assert_eq!(d.code, "E002");
    let d = first_error("rule a for rabbit: then move_towards(3)");
    assert_eq!(d.code, "E002");
    let d = first_error("rule a for rabbit: when (energy > 1) + 2 > 0 then die()");
    assert_eq!(d.code, "E002");
}

#[test]
fn semantic_restrictions() {
    let cases = [
        ("rule a for rabbit: then set age = 3", "E008"),
        ("rule a for rabbit: then set other.energy = 3", "E008"),
        ("rule a for rabbit: when other.energy > 0 then die()", "E013"),
        ("rule a for grass: then wander()", "E017"),
        ("rule a for grass: then reproduce()", "E018"),
        ("rule a for fox: then reproduce()", "E004"),
        ("rule a for rabbit: then reproduce(partner=nearest(rabbit))", "E004"),
        ("rule a for rabbit: on_collision then eat(other), die()", "E010"),
        ("rule a for rabbit: then wander(), stay()", "E010"),
        ("rule a for rabbit: then transform_into(fox), transform_into(grass)", "E010"),
        ("rule a for rabbit: on_death then wander()", "E007"),
        ("rule a for rabbit: then die() probability 2", "E005"),
        ("rule a for rabbit: then die() cost age 1", "E008"),
        ("rule a for rabbit: then die() produces lava", "E003"),
        ("rule a for rabbit: then spawn(grass, 500)", "E012"),
        ("rule a for rabbit: on_collision with grass when other.hunger > 1 then die()", "E001"),
        ("rule a for rabbit: on_collision when other.hunger > 1 then die()", "E001"),
        ("rule a for rabbit: on_state_enter(asleep) then die()", "E003"),
    ];
    for (rule, code) in cases {
        let d = first_error(rule);
        assert_eq!(d.code, code, "{rule}: {d}");
    }
}

#[test]
fn warnings_do_not_block() {
    let d = diagnose("rule a for rabbit: then spawn(grass)");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, "W105");
    let mut sys = base();
    sys.relationships.push(system_rel("rabbit", "Protects(radius: 3.0)", "grass"));
    sys.relationships.push(system_rel("rabbit", "CompetesWith()", "fox"));
    let compiled = compile(&sys).unwrap();
    let codes: Vec<&str> = compiled.warnings().iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, vec!["W204", "W205"]);
}

fn system_rel(subject: &str, kind: &str, object: &str) -> rule_engine::Relationship {
    ron::from_str(&format!(r#"(subject: "{subject}", kind: {kind}, object: "{object}")"#)).unwrap()
}

#[test]
fn duplicate_names_and_bad_definitions() {
    let mut sys = base();
    sys.rules.push(parse_rule("rule a for rabbit: then die()").unwrap());
    sys.rules.push(parse_rule("rule a for rabbit: then stay()").unwrap());
    let err = compile(&sys).unwrap_err();
    assert!(err.diagnostics().iter().any(|d| d.code == "E016"), "{err}");

    let mut sys = base();
    sys.entity_types.push(rule_engine::EntityType::new("fox"));
    sys.world.resources[0].diffusion = 0.4;
    sys.world.topology = simulation_core::Topology::Infinite;
    sys.relationships.push(system_rel("rabbit", "Avoids()", "wolf"));
    let err = compile(&sys).unwrap_err();
    let codes: Vec<&str> = err.diagnostics().iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&"E016"), "{codes:?}");
    let text = err.to_string();
    assert!(text.contains("diffusion"), "{text}");
    assert!(text.contains("Infinite"), "{text}");
    assert!(text.contains("unknown entity type `wolf`"), "{text}");
}

#[test]
fn relationships_compile_into_visible_rules() {
    let mut sys = base();
    sys.relationships.push(system_rel(
        "fox",
        "Consumes(efficiency: 0.5, hunger: Some(30.0), radius: 6.0, handling: 10)",
        "rabbit",
    ));
    sys.relationships.push(system_rel("rabbit", "Avoids(radius: 4.0, speed_factor: 1.5)", "fox"));
    sys.relationships.push(system_rel("rabbit", "Consumes(bite: 0.5)", "water"));
    sys.relationships.push(system_rel(
        "rabbit",
        "Transforms(trigger: Condition(\"age > 30\"), probability: 0.5)",
        "fox",
    ));
    sys.relationships.push(system_rel("fox", "ReproducesWith(radius: 5.0)", "fox"));
    let compiled = compile(&sys).unwrap();
    let text = compiled.rules_text();
    for expected in [
        "rule \"fox consumes rabbit: forage\" for fox:\n    when energy < 30 and nearby(\"rabbit\", radius=6) > 0\n    then move_towards(nearest(\"rabbit\", radius=6))\n    priority 500",
        "rule \"fox consumes rabbit: eat\" for fox:\n    on_collision with rabbit\n    when energy < 30\n    then eat(other, efficiency=0.5)\n    priority 520 cooldown 10",
        "rule \"rabbit avoids fox: flee\" for rabbit:\n    when nearby(\"fox\", radius=4) > 0\n    then move_away(nearest(\"fox\", radius=4), speed=self.speed * 1.5)\n    priority 900",
        "rule \"rabbit consumes water: feed\" for rabbit:\n    when field(\"water\") > 0\n    then consume(\"water\", 0.5)\n    priority 510",
        "rule \"rabbit transforms fox: transform\" for rabbit:\n    when age > 30\n    then transform_into(\"fox\")\n    priority 600 probability 0.5",
        "# generated from a `reproduces_with` relationship",
    ] {
        assert!(text.contains(expected), "missing:\n{expected}\n--- in ---\n{text}");
    }
    // Rules are evaluated by priority: flee before transform before feed.
    let rabbit_rules: Vec<&str> =
        compiled.types()[0].rules.iter().map(|&r| compiled.rules()[r as usize].name.as_str()).collect();
    assert_eq!(
        rabbit_rules,
        vec!["rabbit avoids fox: flee", "rabbit transforms fox: transform", "rabbit consumes water: feed"]
    );
    // The graph has one typed edge per relationship.
    let g = compiled.graph();
    assert_eq!(g.nodes.len(), 4, "3 types + 1 resource");
    assert_eq!(g.edges.len(), 5);
    assert_eq!(g.edges[0].relation, "consumes");
    assert_eq!(g.edges[0].rules, vec!["fox consumes rabbit: forage", "fox consumes rabbit: eat"]);
    assert!(g.nodes[g.edges[2].to].resource);
}

#[test]
fn bad_relationship_conditions_are_reported() {
    let mut sys = base();
    sys.relationships.push(system_rel("rabbit", "Transforms(trigger: Condition(\"age >\"))", "fox"));
    let err = compile(&sys).unwrap_err();
    assert!(err.to_string().contains("trigger condition"), "{err}");
}

// ---------------------------------------------------------------- review fixes

use rule_engine::dsl::{Action, BinOp, Expr, RuleAst};
use rule_engine::{ItemRef, check_rule, check_rule_text, validate};

#[test]
fn diagnostics_are_anchored_to_the_item_they_belong_to() {
    let mut sys = base();
    sys.entity_types[1].size = -1.0;
    sys.world.resources[0].diffusion = 0.9;
    sys.relationships.push(system_rel("rabbit", "Avoids()", "wolf"));
    let err = compile(&sys).unwrap_err();
    let item_of = |needle: &str| err.diagnostics().iter().find(|d| d.to_string().contains(needle)).and_then(|d| d.item);
    assert_eq!(item_of("size"), Some(ItemRef::EntityType(1)));
    assert_eq!(item_of("diffusion"), Some(ItemRef::Resource(0)));
    assert_eq!(item_of("wolf"), Some(ItemRef::Relationship(0)));

    let mut sys = base();
    sys.rules.push(parse_rule("rule ok for rabbit: then stay()").unwrap());
    sys.rules.push(parse_rule("rule bad for rabbit: when enrgy > 1 then stay()").unwrap());
    sys.relationships.push(system_rel("rabbit", "Protects(radius: 3.0)", "grass"));
    sys.scripts.push(rule_engine::ScriptRule {
        name: "s".into(),
        entity: "rabbit".into(),
        event: Default::default(),
        priority: 0,
        source: "fn run(me) { let x = ; }".into(),
        enabled: true,
    });
    let all = validate(&sys);
    let find = |code: &str| all.iter().find(|d| d.code == code).unwrap_or_else(|| panic!("{code} in {all:?}"));
    assert_eq!(find("E001").item, Some(ItemRef::Rule(1)), "index into LifeSystem::rules");
    assert_eq!(find("W204").item, Some(ItemRef::Relationship(0)), "warnings are included");
    assert_eq!(find("E030").item, Some(ItemRef::Script(0)));
    // A valid system yields its warnings only.
    sys.rules.pop();
    sys.scripts.clear();
    assert!(validate(&sys).iter().all(|d| !d.is_error()));
}

#[test]
fn single_rules_can_be_checked_live_against_the_definitions() {
    let sys = base();
    let (rule, diags) = check_rule_text(&sys, "rule a for rabbit:\n  when enrgy < 5 then die()");
    assert!(rule.is_some(), "syntactically valid text yields the rule");
    assert_eq!(diags[0].code, "E001");
    assert_eq!(diags[0].span.map(|s| (s.line, s.col)), Some((2, 8)));
    let (rule, diags) = check_rule_text(&sys, "rule a for rabbit: when then die()");
    assert!(rule.is_none());
    assert_eq!(diags[0].code, "E015");
    assert!(check_rule(&sys, &parse_rule("rule a for rabbit: then stay()").unwrap()).is_empty());
    let w = check_rule(&sys, &parse_rule("rule a for rabbit: then spawn(grass)").unwrap());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, "W105");
    // Broken definitions are reported instead.
    let mut broken = base();
    broken.world.width = 1;
    assert!(check_rule(&broken, &parse_rule("rule a for rabbit: then stay()").unwrap())[0].code == "E020");
}

#[test]
fn editor_built_rules_need_finite_numbers_and_a_faithful_text_form() {
    let die = || vec![Action::call("die", vec![])];
    let mut nan = RuleAst::new("nan", "rabbit", die());
    nan.condition = Some(Expr::binary(BinOp::Gt, Expr::path(&["energy"]), Expr::num(f64::NAN)));
    let mut multiline = RuleAst::new("two\nlines", "rabbit", die());
    multiline.meta.priority = 1;
    let mut huge_cooldown = RuleAst::new("c", "rabbit", die());
    huge_cooldown.meta.cooldown = Some(u64::MAX);
    let mut deep = RuleAst::new("deep", "rabbit", die());
    // Deep enough to overflow the recursive validator on a test thread,
    // shallow enough for Rust's own (recursive) clone and drop.
    let mut e = Expr::path(&["energy"]);
    for _ in 0..5_000 {
        e = Expr::binary(BinOp::Add, e, Expr::num(1.0));
    }
    deep.condition = Some(Expr::binary(BinOp::Gt, e, Expr::num(0.0)));
    for (rule, code) in [(nan, "E002"), (multiline, "E015"), (huge_cooldown, "E015"), (deep, "E015")] {
        let mut sys = base();
        sys.rules.push(rule);
        let err = compile(&sys).unwrap_err();
        let d = err.diagnostics().iter().find(|d| d.is_error()).unwrap();
        assert_eq!(d.code, code, "{d}");
        assert_eq!(d.item, Some(ItemRef::Rule(0)));
    }
    // Every system that compiles survives a RON and JSON round trip unchanged.
    for preset in rule_engine::scenarios::builtin().unwrap() {
        let sys = &preset.system;
        assert_eq!(&LifeSystem::from_ron(&sys.to_ron().unwrap()).unwrap(), sys, "{}", preset.id);
        assert_eq!(&LifeSystem::from_json(&sys.to_json().unwrap()).unwrap(), sys, "{}", preset.id);
    }
}

#[test]
fn on_death_rules_cannot_set_add_or_pay_and_priorities_are_bounded() {
    for rule in [
        "rule a for rabbit: on_death then set hunger = 1",
        "rule a for rabbit: on_death then add energy += 1",
        "rule a for rabbit: on_death then spawn(grass) cost energy 1",
    ] {
        assert_eq!(first_error(rule).code, "E007", "{rule}");
    }
    assert_eq!(first_error("rule a for rabbit: then stay() priority 2000000").code, "E012");
    assert!(diagnose("rule a for rabbit: then stay() priority -1000000").iter().all(|d| !d.is_error()));
    let d = first_error("rule a for rabbit: when nearby(fox, radius=1e300) > 0 then die()");
    assert!(d.message.contains("1e300"), "numbers in messages stay short: {d}");
}

#[test]
fn definition_checks_added_by_the_review() {
    // Initial populations must fit under the entity cap.
    let mut sys = base();
    sys.entity_types[0].initial.count = 3;
    sys.entity_types[1].initial.count = 3;
    sys.world.max_entities = 5;
    let err = compile(&sys).unwrap_err();
    let d = err.diagnostics().iter().find(|d| d.message.contains("max_entities")).unwrap();
    assert_eq!(d.item, Some(ItemRef::World));

    // trades_with: both parties must carry both resources; give ≠ receive.
    let mut sys = base();
    sys.world.resources.push(rule_engine::def::ResourceField::new("ore"));
    sys.entity_types[0].carries.push(ron::from_str(r#"(resource: "ore", capacity: 5.0)"#).unwrap());
    sys.relationships.push(system_rel("rabbit", r#"TradesWith(give: "ore", receive: "water")"#, "fox"));
    let err = compile(&sys).unwrap_err();
    let msgs: Vec<String> = err.diagnostics().iter().map(|d| d.message.clone()).collect();
    for m in ["`rabbit` must carry `water`", "`fox` must carry `ore`", "`fox` must carry `water`"] {
        assert!(msgs.iter().any(|x| x.contains(m)), "{m} in {msgs:?}");
    }
    assert!(err.diagnostics().iter().all(|d| d.item == Some(ItemRef::Relationship(0))));
    let mut same = base();
    same.relationships.push(system_rel("rabbit", r#"TradesWith(give: "water", receive: "water")"#, "fox"));
    assert!(compile(&same).unwrap_err().to_string().contains("give ≠ receive"));
}

#[test]
fn repeated_relationships_get_unique_rule_names_and_warnings() {
    let mut sys = base();
    sys.relationships.push(system_rel("rabbit", "Transforms(trigger: AfterTicks(50))", "fox"));
    sys.relationships.push(system_rel("rabbit", "Transforms(trigger: Contact(\"fox\"), probability: 0.5)", "fox"));
    sys.relationships.push(system_rel("rabbit", "Avoids()", "fox"));
    sys.relationships.push(system_rel("rabbit", "Avoids()", "fox"));
    let compiled = compile(&sys).unwrap();
    let names: Vec<&str> = compiled.rules().iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"rabbit transforms fox: transform"));
    assert!(names.contains(&"rabbit transforms fox #2: transform"), "{names:?}");
    assert!(names.contains(&"rabbit avoids fox #4: flee"), "{names:?}");
    let w = compiled.warnings();
    assert!(w.iter().any(|d| d.code == "W102" && d.item == Some(ItemRef::Relationship(3))), "{w:?}");

    // Unconditional transformation cycles are flagged.
    let mut sys = base();
    sys.relationships.push(system_rel("rabbit", "Transforms()", "fox"));
    sys.relationships.push(system_rel("fox", "Transforms(probability: 0.1)", "rabbit"));
    let compiled = compile(&sys).unwrap();
    let w103: Vec<_> = compiled.warnings().iter().filter(|d| d.code == "W103").collect();
    assert_eq!(w103.len(), 2, "{w103:?}");
    assert!(w103[0].message.contains("rabbit -> fox -> rabbit"), "{}", w103[0].message);
}

#[test]
fn the_graph_carries_trophic_levels_and_edge_summaries() {
    let p = rule_engine::scenarios::builtin_by_id("producers-consumers").unwrap();
    let compiled = compile(&p.system).unwrap();
    let g = compiled.graph();
    let level = |n: &str| g.nodes.iter().find(|x| x.name == n).unwrap().trophic_level;
    assert!(level("hunter") > level("grazer") && level("grazer") > level("plant"), "{:?}", g.nodes);
    assert!(g.edges.iter().any(|e| e.relation == "consumes" && e.summary.contains("efficiency")), "{:?}", g.edges);
    let mut sys = base();
    sys.relationships.push(system_rel("rabbit", "CompetesWith()", "fox"));
    sys.relationships.push(system_rel("rabbit", "Consumes()", "fox"));
    sys.relationships.push(system_rel("fox", "Consumes()", "rabbit"));
    let compiled = compile(&sys).unwrap();
    assert!(compiled.graph().edges[0].symmetric);
    // Feeding loops stay bounded.
    assert!(compiled.graph().nodes.iter().all(|n| n.trophic_level <= compiled.graph().nodes.len() as f64));
}

#[test]
fn the_vocabulary_is_what_validation_accepts() {
    use rule_engine::compile::vocab::{FUNCTIONS, FunctionKind};
    let sys = base();
    for f in FUNCTIONS {
        // Too many arguments are rejected with the listed signature.
        let args = vec!["1"; f.params.len() + 1].join(", ");
        let rule = match f.kind {
            FunctionKind::Sensor => format!("rule a for rabbit: when {}({args}) > 0 then stay()", f.name),
            FunctionKind::Action => format!("rule a for rabbit: then {}({args})", f.name),
        };
        let d = check_rule(&sys, &parse_rule(&rule).unwrap());
        let e = d.iter().find(|d| d.code == "E004").unwrap_or_else(|| panic!("{rule}: {d:?}"));
        for p in f.params {
            assert!(e.help.as_deref().unwrap_or("").contains(p.name), "{}: {e}", f.name);
        }
    }
}
