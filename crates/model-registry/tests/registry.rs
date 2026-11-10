//! Registry tests: every model and built-in scenario builds, advances and
//! replays deterministically; overrides follow their apply policies; ids,
//! overrides and files are rejected with clear errors.

use std::path::{Path, PathBuf};

use model_registry::{
    BuildRequest, CheckStatus, MODELS, ParamOverride, Registry, RegistryConfig, RegistryError, ScenarioSource,
};
use simulation_core::{ApplyPolicy, ParamValue};

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn registry() -> Registry {
    Registry::new(RegistryConfig { assets: Some(assets()), scenario_dirs: Vec::new() })
}

/// Fresh, empty folder under the system temporary directory.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("model-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn every_model_has_a_default_scenario_and_describes_itself() {
    let registry = registry();
    assert!(registry.problems().is_empty(), "{:?}", registry.problems());
    for spec in MODELS {
        let default = registry.default_scenario(spec.id).unwrap();
        assert_eq!(default.info.model_id, spec.id, "default scenario of {}", spec.id);
        let info = registry.model_info(spec.id).unwrap();
        assert_eq!(info.descriptor.id, spec.id);
        assert!(!info.name.is_empty() && !info.summary.is_empty(), "{}", spec.id);
        assert!(!info.descriptor.metrics.is_empty(), "{} declares metrics", spec.id);
        assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(spec.docs).is_file(), "{}", spec.docs);
        assert!(registry.scenarios_of(spec.id).count() >= 1);
    }
    // Every scenario belongs to a registered model.
    for s in registry.scenarios() {
        assert!(model_registry::model_spec(&s.info.model_id).is_some(), "{} -> {}", s.info.id, s.info.model_id);
    }
}

#[test]
fn every_builtin_scenario_builds_advances_and_replays() {
    let registry = registry();
    let mut count = 0;
    for scenario in registry.scenarios() {
        assert!(matches!(scenario.info.source, ScenarioSource::Builtin(_)));
        let mut built = registry.build(&BuildRequest::new(&scenario.info.id)).unwrap();
        assert_eq!(built.sim.descriptor().id, scenario.info.model_id, "{}", scenario.info.id);
        assert_eq!(built.seed, scenario.info.seed);
        let start = built.sim.tick();
        assert!(built.sim.advance(2) > 0, "{} advances", scenario.info.id);
        assert!(built.sim.tick() > start);

        let report = model_registry::validate(&registry, &scenario.info.id, None, 4)
            .unwrap_or_else(|e| panic!("{}: {e}", scenario.info.id));
        for check in &report.checks {
            assert_ne!(check.status, CheckStatus::Failed, "{}: {} — {}", scenario.info.id, check.name, check.detail);
        }
        let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"determinism") && names.contains(&"snapshot round trip"));
        assert!(
            report.checks.iter().take(2).all(|c| c.status == CheckStatus::Passed),
            "{}: {:?}",
            scenario.info.id,
            report.checks
        );
        count += 1;
    }
    // 9 Conway + 20 CA lab + 18 Hashlife patterns + 7 metapixel + 10 continuous + 12 voxel
    // + 10 environment presets + 14 ecosystem + 7 custom life + 6 evolution + 6 civilization.
    assert_eq!(count, 119);
}

#[test]
fn model_specific_checks_run() {
    let registry = registry();
    let expect = |target: &str, name: &str| {
        let report = model_registry::validate(&registry, target, None, 8).unwrap();
        let check = report.checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{target}: {name}"));
        assert_eq!(check.status, CheckStatus::Passed, "{target}: {}", check.detail);
    };
    expect("life.classic", "engine agreement");
    expect("ca.life_like", "engine agreement");
    expect("life.hashlife", "pattern periods");
    expect("continuous.flow_lenia", "mass conservation");
    expect("ecosystem/balanced_meadow", "energy and nutrient books");
    expect("life.metapixel", "macro readout");
}

#[test]
fn ids_resolve_by_model_qualified_and_bare_id() {
    let registry = registry();
    assert_eq!(registry.resolve("life.classic").unwrap().info.id, "conway/glider-gun-showcase");
    assert_eq!(registry.resolve("ca-lab/seeds").unwrap().info.model_id, "ca.life_like");
    assert_eq!(registry.resolve("seeds").unwrap().info.id, "ca-lab/seeds");
    match registry.resolve("island_evolution") {
        Err(RegistryError::AmbiguousScenario { candidates, .. }) => {
            assert_eq!(candidates, ["ecosystem/island_evolution", "evolution/island_evolution"]);
        }
        other => panic!("expected an ambiguity, got {:?}", other.map(|s| s.info.id.clone())),
    }
    let err = registry.build(&BuildRequest::new("no-such-thing")).err().unwrap();
    assert!(matches!(err, RegistryError::UnknownTarget { .. }), "{err}");
    assert!(err.to_string().contains("`no-such-thing` is not a model or scenario id"), "{err}");
    let err = registry.resolve("glider-gun").err().unwrap().to_string();
    assert!(err.contains("did you mean") && err.contains("conway/glider-gun-showcase"), "{err}");
    let err = registry.model("life.nope").err().unwrap().to_string();
    assert!(err.contains("unknown model `life.nope`") && err.contains("life.classic"), "{err}");
}

#[test]
fn seed_override_changes_random_content_reproducibly() {
    let registry = registry();
    let population = |seed| {
        let built = registry.build(&BuildRequest::new("conway/random-soup-torus").seed(seed)).unwrap();
        assert_eq!(built.seed, seed);
        built.sim.metrics().get("population").unwrap()
    };
    assert_eq!(population(5), population(5));
    assert_ne!(population(5), population(6));
}

#[test]
fn seeds_change_random_scenarios_of_every_family_and_replay_identically() {
    let registry = registry();
    // Metrics at tick 0 and after each of `steps` ticks.
    let trace = |target: &str, seed: u64, params: &[&str], steps: usize| {
        let mut request = BuildRequest::new(target).seed(seed);
        for p in params {
            request = request.param(ParamOverride::parse(p).unwrap());
        }
        let mut built = registry.build(&request).unwrap();
        let mut out = vec![built.sim.metrics()];
        for _ in 0..steps {
            built.sim.step();
            out.push(built.sim.metrics());
        }
        out
    };
    let cases: [(&str, &[&str], usize); 12] = [
        ("conway/random-soup-torus", &[], 1),
        ("ca-lab/brians-brain", &[], 1),
        ("ca.wolfram", &["start=random"], 1),
        ("metapixel/metapixel.soup", &[], 0),
        ("continuous/lenia.soup", &[], 1),
        ("continuous/flow_lenia.demo", &[], 1),
        ("voxel/life-5766-soup", &[], 1),
        ("environment/temperate", &[], 1),
        ("ecosystem/wolf_sheep_grass", &[], 2),
        ("custom-life/bacterial-growth", &[], 2),
        ("evolution/speed_selection", &[], 2),
        ("civilization/abundant", &[], 2),
    ];
    for (target, params, steps) in cases {
        let a = trace(target, 101, params, steps);
        assert_eq!(a, trace(target, 101, params, steps), "{target}: the same seed replays identically");
        assert_ne!(a, trace(target, 202, params, steps), "{target}: another seed gives another run");
    }
}

#[test]
fn seeds_must_fit_the_seed_parameter() {
    let registry = registry();
    for target in ["ecosystem", "continuous.lenia", "custom_life", "life.metapixel"] {
        let err = registry.build(&BuildRequest::new(target).seed(u64::MAX)).err().unwrap();
        assert!(matches!(err, RegistryError::InvalidSeed { .. }), "{target}: {err}");
        assert!(err.to_string().contains("its `seed` parameter accepts 0 to 9223372036854775807"), "{err}");
    }
    // These crates check the seed themselves.
    for target in ["civilization", "voxel.life", "environment.world"] {
        let err = registry.build(&BuildRequest::new(target).seed(u64::MAX)).err().unwrap();
        assert!(matches!(err, RegistryError::Build { .. }), "{target}: {err}");
        assert!(err.to_string().contains("seed") && err.to_string().contains("18446744073709551615"), "{err}");
    }
    // Models without a seed parameter take every 64-bit seed.
    assert_eq!(registry.build(&BuildRequest::new("conway/random-soup-torus").seed(u64::MAX)).unwrap().seed, u64::MAX);
}

#[test]
fn a_seed_parameter_override_sets_the_seed_of_the_run() {
    let registry = registry();
    let target = "civilization/abundant";
    let seed_param = || ParamOverride::parse("seed=7").unwrap();
    let mut by_param = registry.build(&BuildRequest::new(target).param(seed_param())).unwrap();
    assert_eq!(by_param.seed, 7);
    assert!(by_param.reset_applied);
    let mut by_seed = registry.build(&BuildRequest::new(target).seed(7)).unwrap();
    for _ in 0..3 {
        by_param.sim.step();
        by_seed.sim.step();
        assert_eq!(by_param.sim.metrics(), by_seed.sim.metrics());
    }
    assert_eq!(registry.build(&BuildRequest::new(target).seed(7).param(seed_param())).unwrap().seed, 7);
    let err = registry.build(&BuildRequest::new(target).seed(5).param(seed_param())).err().unwrap().to_string();
    assert_eq!(err, "parameter `seed`: the seed is given twice with different values (5 and 7); give it once");
}

#[test]
fn reset_only_parameters_take_effect_before_the_first_tick() {
    let registry = registry();
    let request = BuildRequest::new("conway/random-soup-torus")
        .param(ParamOverride::parse("width=96").unwrap())
        .param(ParamOverride::parse("height=64").unwrap())
        .param(ParamOverride::parse("rule=B36/S23").unwrap());
    let mut built = registry.build(&request).unwrap();
    assert!(built.reset_applied);
    let policies: Vec<ApplyPolicy> = built.overrides.iter().map(|o| o.policy).collect();
    assert_eq!(policies, [ApplyPolicy::RequiresReset, ApplyPolicy::RequiresReset, ApplyPolicy::NextTick]);
    assert_eq!(built.sim.tick(), 0);
    let params = built.sim.params();
    assert_eq!(params.get("width"), Some(&ParamValue::Int(96)));
    assert_eq!(params.get("height"), Some(&ParamValue::Int(64)));
    built.sim.advance(1);
    assert_eq!(built.sim.params().get("rule"), Some(&ParamValue::Text("B36/S23".into())));
    // The world really is 96×64: the density is population / 6144.
    let m = built.sim.metrics();
    let (population, density) = (m.get("population").unwrap(), m.get("density").unwrap());
    assert!((density - population / (96.0 * 64.0)).abs() < 1e-12, "{population} {density}");
}

#[test]
fn reset_only_text_values_are_recorded_as_the_model_normalises_them() {
    let registry = registry();
    let request = BuildRequest::new("life.metapixel")
        .param(ParamOverride::parse("rule=b36s23").unwrap())
        .param(ParamOverride::parse("width=4").unwrap())
        .param(ParamOverride::parse("width=5").unwrap());
    let built = registry.build(&request).unwrap();
    assert!(built.reset_applied);
    assert_eq!(built.overrides[0].value, ParamValue::Text("B36/S23".into()));
    assert_eq!(built.sim.params().get("rule"), Some(&ParamValue::Text("B36/S23".into())));
    // A repeated key: the last value is the one in effect.
    assert_eq!(built.sim.params().get("width"), Some(&ParamValue::Int(5)));
}

#[test]
fn bad_overrides_are_rejected_with_reasons() {
    let registry = registry();
    let build = |o: &str| registry.build(&BuildRequest::new("life.classic").param(ParamOverride::parse(o).unwrap()));
    let err = build("speed=3").err().unwrap();
    match &err {
        RegistryError::UnknownParameter { model, known, .. } => {
            assert_eq!(model, "life.classic");
            assert!(known.iter().any(|k| k == "width"));
        }
        other => panic!("{other}"),
    }
    let err = build("width=0").err().unwrap().to_string();
    assert!(err.starts_with("parameter `width`: 0 is outside [1,"), "{err}");
    let err = build("width=wide").err().unwrap().to_string();
    assert_eq!(err, "parameter `width`: expected an integer, got `wide`");
    let err = build("topology=sphere").err().unwrap().to_string();
    assert!(err.contains("`sphere` is not one of"), "{err}");
    let err = build("rule=B3/S23/X").err().unwrap().to_string();
    assert!(err.starts_with("parameter `rule`:"), "{err}");
    assert!(matches!(ParamOverride::parse("width"), Err(RegistryError::MalformedOverride(_))));
}

#[test]
fn metapixel_without_assets_explains_what_is_missing() {
    let err = Registry::builtin().build(&BuildRequest::new("life.metapixel")).err().unwrap().to_string();
    assert!(err.contains("assets/patterns/otca") && err.contains("--assets"), "{err}");
}

#[test]
fn scenario_directories_add_replace_and_report_files() {
    let root = temp_dir("dirs");
    let conway = root.join("conway");
    std::fs::create_dir_all(&conway).unwrap();
    let original = std::fs::read_to_string(assets().join("../scenarios/conway/bounded-r-pentomino.ron")).unwrap();
    // A new scenario, a replacement of a built-in one and a broken file.
    std::fs::write(conway.join("my-copy.ron"), original.replace("\"bounded-r-pentomino\"", "\"my-copy\"")).unwrap();
    std::fs::write(conway.join("bounded-r-pentomino.ron"), &original).unwrap();
    std::fs::write(conway.join("broken.ron"), "(id: \"broken\", name: \"x\",").unwrap();
    std::fs::create_dir_all(root.join("evolution")).unwrap();
    std::fs::write(root.join("evolution/bad-seed.ron"), "(id: \"Bad\", name: \"x\", description: \"\", seed: 1)")
        .unwrap();

    let registry = Registry::new(RegistryConfig { assets: Some(assets()), scenario_dirs: vec![root.clone()] });
    let added = registry.scenario("conway/my-copy").unwrap();
    assert_eq!(added.info.source, ScenarioSource::File(conway.join("my-copy.ron")));
    assert_eq!(added.info.model_id, "life.classic");
    assert!(registry.build(&BuildRequest::new("my-copy")).is_ok());
    let replaced = registry.scenario("conway/bounded-r-pentomino").unwrap();
    assert_eq!(replaced.info.source, ScenarioSource::File(conway.join("bounded-r-pentomino.ron")));
    assert_eq!(replaced.info.replaces, Some(ScenarioSource::Builtin("cellular-automata".into())));
    // Grouped with the other Conway scenarios.
    let position = |id: &str| registry.scenarios().iter().position(|s| s.info.id == id).unwrap();
    assert!(position("conway/my-copy") < position("ca-lab/seeds"));

    let problems = registry.problems();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(problems[0].path, conway.join("broken.ron"));
    assert!(!problems[0].message.is_empty());
    assert_eq!(problems[1].path, root.join("evolution/bad-seed.ron"));
    assert!(problems[1].message.contains("Bad"), "{}", problems[1].message);

    // A file can also be built directly by path; its folder names the format.
    let path = conway.join("my-copy.ron");
    let built = registry.build(&BuildRequest::new(path.to_str().unwrap())).unwrap();
    assert_eq!(built.scenario.id, "conway/my-copy");
    let stray = root.join("loose.ron");
    std::fs::write(&stray, &original).unwrap();
    let err = registry.build(&BuildRequest::new(stray.to_str().unwrap())).err().unwrap().to_string();
    assert!(err.contains("scenario files must be in a folder named one of: conway, ca-lab"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn custom_life_systems_saved_by_the_editor_are_scenarios() {
    let root = temp_dir("library");
    let library = root.join("custom-life");
    std::fs::create_dir_all(&library).unwrap();
    // The Custom Life editor saves plain systems (no header, no seed) here.
    let preset = rule_engine::scenarios::builtin().unwrap().remove(0);
    std::fs::write(library.join("my-bacteria.ron"), preset.system.to_ron().unwrap()).unwrap();
    std::fs::write(library.join("garbage.ron"), "(name: 3)").unwrap();
    let registry = Registry::new(RegistryConfig { assets: Some(assets()), scenario_dirs: vec![root.clone()] });
    let scenario = registry.scenario("custom-life/my-bacteria").unwrap();
    assert_eq!(scenario.info.model_id, "custom_life");
    assert_eq!(scenario.info.title, preset.system.name);
    assert_eq!(scenario.info.seed, 0, "the default of the model's seed parameter");
    let mut built = registry.build(&BuildRequest::new("my-bacteria").seed(preset.seed)).unwrap();
    let mut reference = registry.build(&BuildRequest::new(&format!("custom-life/{}", preset.id))).unwrap();
    built.sim.advance(3);
    reference.sim.advance(3);
    assert_eq!(built.sim.metrics(), reference.sim.metrics(), "the same system and seed run identically");
    let problems = registry.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].message.starts_with("neither a Custom Life preset ("), "{}", problems[0].message);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn repository_scenario_folders_match_the_builtins() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let with_files = Registry::new(RegistryConfig { assets: Some(assets()), scenario_dirs: vec![repo] });
    assert!(with_files.problems().is_empty(), "{:?}", with_files.problems());
    let builtin = registry();
    assert_eq!(with_files.scenarios().len(), builtin.scenarios().len(), "the folders hold the embedded files");
    for (a, b) in with_files.scenarios().iter().zip(builtin.scenarios()) {
        assert_eq!(a.info.id, b.info.id);
    }
}
