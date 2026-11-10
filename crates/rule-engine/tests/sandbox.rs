//! Rhai sandbox: limits, disabled features, error reporting and the
//! intent-recording API.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::system;
use rule_engine::def::ScriptLimits;
use rule_engine::script::sandbox::sandboxed_engine;
use rule_engine::{CustomLifeCommand, CustomLifeSim, LifeSystem, RuleEngineError, ScriptErrorKind};
use simulation_core::Simulation;

/// A world of `n` sessile "bot"s plus one mobile "food" source, running `script`.
fn world_with_script(script: &str, n: u32) -> LifeSystem {
    let mut sys = system(&format!(
        r#"(
        name: "sandbox",
        entity_types: [
            (name: "bot", size: 1.0, movement: RandomWalk(speed: 0.5), states: ["idle", "busy"],
             properties: [(name: "mood", default: 1.0, min: 0.0, max: 10.0)],
             energy: (initial: 10.0, max: 50.0, metabolism: 0.0, starvation: false),
             initial: (count: {n}, distribution: Grid)),
            (name: "food", size: 1.0, energy: (initial: 5.0, max: 5.0, starvation: false),
             initial: (count: 1, distribution: Cluster(x: 8.0, y: 8.0, radius: 0.0))),
        ],
        world: (width: 16, height: 16, resources: [(name: "slime", initial: Uniform(0.0), max: 100.0)]),
    )"#
    ));
    sys.scripts.push(rule_engine::ScriptRule {
        name: "brain".into(),
        entity: "bot".into(),
        event: Default::default(),
        priority: 0,
        source: script.into(),
        enabled: true,
    });
    sys
}

fn run_failing(script: &str) -> rule_engine::ScriptError {
    let mut sim = CustomLifeSim::new(world_with_script(script, 4), 1).unwrap();
    sim.step();
    sim.step();
    let errors = sim.script_errors();
    assert_eq!(errors.len(), 1, "one error per disabled script: {errors:?}");
    // The simulation keeps running without the script.
    assert_eq!(sim.tick(), 2);
    errors[0].clone()
}

#[test]
fn endless_loops_hit_the_operation_budget() {
    let e = run_failing("fn run(me) {\n    let x = 0;\n    loop { x += 1; }\n}");
    assert_eq!(e.kind, ScriptErrorKind::Budget);
    assert_eq!(e.script, "brain");
    assert_eq!(e.entity, "bot");
    // Rhai 1.27 reports budget errors without a position.
    assert!(e.message.contains("budget of 20000 operations"), "{}", e.message);
    assert!(e.to_string().starts_with("script `brain` (bot), tick 0: Too many operations"), "{e}");
}

#[test]
fn deep_recursion_is_stopped() {
    let e = run_failing("fn dive(n) { dive(n + 1) }\nfn run(me) { dive(0); }");
    assert_eq!(e.kind, ScriptErrorKind::Recursion, "{e}");
}

#[test]
fn huge_strings_and_arrays_are_refused() {
    let e = run_failing("fn run(me) { let s = \"xxxxxxxx\"; for i in 0..20 { s += s; } }");
    assert_eq!(e.kind, ScriptErrorKind::Memory, "{e}");
    let e = run_failing("fn run(me) { let a = []; a.pad(100000, 0); }");
    assert_eq!(e.kind, ScriptErrorKind::Memory, "{e}");
}

#[test]
fn missing_and_forbidden_functions_fail_cleanly() {
    for (src, what) in [
        ("fn run(me) { launch_missiles(); }", "launch_missiles"),
        ("fn run(me) { sleep(1); }", "sleep"),
        ("fn run(me) { timestamp(); }", "timestamp"),
        ("fn run(me) { open_file(\"/etc/passwd\"); }", "open_file"),
        ("fn run(me) { parse_json(\"{}\"); }", "parse_json"),
    ] {
        let e = run_failing(src);
        assert_eq!(e.kind, ScriptErrorKind::UnknownFunction, "{src}: {e}");
        assert!(e.message.contains(what), "{e}");
    }
}

#[test]
fn eval_import_and_missing_entry_points_are_compile_errors() {
    for (src, needle) in [
        ("import \"secret\" as m;\nfn run(me) {}", "import"),
        ("fn run(me) { eval(\"40 + 2\"); }", "eval"),
        ("fn main() {}", "fn run(me)"),
        ("fn run(me) {\n  let x = ;\n}", ""),
    ] {
        let err = CustomLifeSim::new(world_with_script(src, 1), 1).unwrap_err();
        let RuleEngineError::Invalid(diags) = &err else { panic!("{err}") };
        let d = &diags[0];
        assert_eq!(d.code, "E030", "{d}");
        assert!(d.to_string().contains(needle), "{src}: {d}");
        if src.contains("let x = ;") {
            assert_eq!(d.span.map(|s| s.line), Some(2), "{d}");
        }
    }
}

#[test]
fn the_raw_engine_has_no_sleep_eval_or_file_import() {
    let engine = sandboxed_engine(&ScriptLimits::default(), Arc::new(AtomicBool::new(false)));
    assert!(engine.run("sleep(0)").is_err());
    assert!(engine.compile("eval(\"1\")").is_err());
    assert!(engine.compile("import \"x\" as y;").is_err());
    assert_eq!(engine.eval::<i64>("let x = 40; x + 2").unwrap(), 42);
    assert!(engine.run("loop {}").is_err(), "the budget stops endless loops");
}

#[test]
fn runtime_api_errors_report_the_line_and_disable_only_that_script() {
    let e = run_failing("fn run(me) {\n    move_towards(nearest(\"unicorn\", 3));\n}");
    assert_eq!(e.kind, ScriptErrorKind::Runtime);
    assert_eq!(e.line, Some(2));
    assert!(e.message.contains("unknown entity type `unicorn`"), "{}", e.message);
    let e = run_failing("fn run(me) { set_prop(\"age\", 3); }");
    assert!(e.message.contains("read-only"), "{e}");
    let e = run_failing("fn run(me) { nearby(\"food\", 500); }");
    assert!(e.message.contains("radius"), "{e}");
}

#[test]
fn scripts_read_the_world_and_record_intents() {
    let script = r#"
        fn run(me) {
            let f = nearest("food", 30);
            if f.exists && me.prop("mood") > 0.5 {
                move_towards(f);
                emit("slime", 0.5);
                add_prop("mood", 1);
                set_state("busy");
            }
        }
    "#;
    let mut sim = CustomLifeSim::new(world_with_script(script, 4), 3).unwrap();
    let food = sim.entities().find(|(_, e)| e.kind == 1).map(|(_, e)| e.kin.position).unwrap();
    let dist = |sim: &CustomLifeSim| -> f32 {
        sim.entities().filter(|(_, e)| e.kind == 0).map(|(_, e)| e.kin.position.distance(food)).sum()
    };
    let before = dist(&sim);
    sim.step();
    assert!(sim.script_errors().is_empty(), "{:?}", sim.script_errors());
    assert!(dist(&sim) < before - 1.5, "bots approach the food");
    assert!((sim.resource_total("slime") - 2.0).abs() < 1e-6, "4 bots × 0.5 slime");
    for (_, e) in sim.entities().filter(|(_, e)| e.kind == 0) {
        assert_eq!(e.props[0], 2.0);
        assert_eq!(e.state, 1);
    }
}

#[test]
fn scripts_are_deterministic_and_disabled_scripts_can_be_reenabled() {
    let script = "fn run(me) { if random() < 0.5 { wander(); } else { stay(); } if tick() == 3 { boom(); } }";
    let run = || {
        let mut sim = CustomLifeSim::new(world_with_script(script, 300), 9).unwrap();
        sim.advance(6);
        (sim.metrics(), sim.entities().map(|(_, e)| e.kin.position).collect::<Vec<_>>(), sim.script_errors().len())
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b, "identical runs (also when decisions run in parallel)");
    assert_eq!(a.2, 1);
    let mut sim = CustomLifeSim::new(world_with_script(script, 2), 9).unwrap();
    sim.advance(5);
    assert_eq!(sim.view(&Default::default()).script_errors.len(), 1, "visible to the UI");
    sim.apply(CustomLifeCommand::ReenableScripts).unwrap();
    assert!(sim.script_errors().is_empty());
}

#[test]
fn host_cancellation_disables_running_scripts() {
    let mut sim = CustomLifeSim::new(world_with_script("fn run(me) { let x = 1; x += 1; }", 2), 1).unwrap();
    let cancel = sim.script_cancel_handle();
    cancel.store(true, Ordering::Relaxed);
    sim.step();
    assert_eq!(sim.script_errors()[0].kind, ScriptErrorKind::Cancelled);
    assert!(!cancel.load(Ordering::Relaxed), "the flag is cleared after the tick");
}

#[test]
fn on_death_scripts_may_only_emit_and_spawn() {
    let mut sys = world_with_script("fn run(me) { spawn_entity(\"food\", 2); emit(\"slime\", 1.0); }", 2);
    sys.scripts[0].event = rule_engine::dsl::Event::Death;
    sys.rules.push(rule_engine::dsl::parse_rule("rule perish for bot: when age >= 2 then die()").unwrap());
    let mut sim = CustomLifeSim::new(sys.clone(), 1).unwrap();
    sim.advance(4);
    assert_eq!(sim.population("bot"), 0);
    assert_eq!(sim.population("food"), 1 + 2 * 2);
    assert!((sim.resource_total("slime") - 2.0).abs() < 1e-6);
    sys.scripts[0].source = "fn run(me) { wander(); }".into();
    let mut sim = CustomLifeSim::new(sys, 1).unwrap();
    sim.advance(4);
    assert!(sim.script_errors()[0].message.contains("not allowed in on_death"), "{:?}", sim.script_errors());
}
