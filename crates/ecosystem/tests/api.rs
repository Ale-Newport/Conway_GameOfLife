//! Simulation contract: scenarios, descriptor, parameters, commands, views,
//! inspection and the data files.

mod common;

use std::path::Path;

use common::{build, grassland, spawn, with_animals};
use ecosystem::scenario::{self, BUILTIN_FILES};
use ecosystem::{
    DeathCause, EcosystemCommand, EcosystemError, EcosystemViewRequest, OrganismInspection, OrganismRef, PlantLayer,
    SpeciesCatalog,
};
use organism_engine::AgentId;
use physics_world::Disaster;
use simulation_core::{ApplyPolicy, ParamValue, SimError, Simulation};

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap()
}

#[test]
fn builtin_scenarios_match_the_files_and_run() {
    let builtin = scenario::builtin().unwrap();
    assert_eq!(builtin.len(), BUILTIN_FILES.len());
    let catalog = SpeciesCatalog::builtin().unwrap();
    let on_disk = scenario::load_dir(&workspace().join("scenarios/ecosystem"), &catalog).unwrap();
    let mut a: Vec<_> = builtin.iter().map(|s| s.id.clone()).collect();
    let mut b: Vec<_> = on_disk.iter().map(|s| s.id.clone()).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    for s in &builtin {
        let mut sim = s.build().unwrap();
        sim.advance(30);
        assert_eq!(sim.tick(), 30);
        let d = sim.descriptor();
        assert_eq!(d.id, "ecosystem");
        assert!((d.dt - 6.0).abs() < 1e-9 && d.tick_unit == "minute");
        let declared: Vec<&str> = d.metrics.iter().map(|m| m.key.as_str()).collect();
        for key in sim.metrics().keys() {
            assert!(declared.contains(&key), "{}: metric {key} is declared", s.id);
        }
        let params = sim.params();
        assert_eq!(params.resolve(&d.params).unwrap(), params, "{}: reported parameters validate", s.id);
        assert!(!d.rules.is_empty());
    }
}

#[test]
fn species_directory_matches_the_embedded_catalogue() {
    let on_disk = SpeciesCatalog::load_dir(&workspace().join("assets/species")).unwrap();
    assert_eq!(on_disk, SpeciesCatalog::builtin().unwrap());
}

#[test]
fn visual_model_ids_exist_in_the_model_manifest() {
    let text = std::fs::read_to_string(workspace().join("assets/models/manifest.toml")).unwrap();
    let manifest: toml::Value = toml::from_str(&text).unwrap();
    let ids: Vec<&str> = manifest["model"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    let catalog = SpeciesCatalog::builtin().unwrap();
    let mut referenced: Vec<String> = Vec::new();
    for s in catalog.animals() {
        referenced.extend(s.visual.model.clone());
    }
    for t in catalog.trees() {
        referenced
            .extend(t.models.young.iter().chain(&t.models.mature).chain(&t.models.old).chain(&t.models.dead).cloned());
    }
    for l in catalog.layers() {
        referenced.extend(l.models.iter().cloned());
    }
    referenced.extend(catalog.fungi().mushroom_models.iter().chain(&catalog.fungi().bracket_models).cloned());
    for id in &referenced {
        assert!(ids.contains(&id.as_str()), "model `{id}` is in assets/models/manifest.toml");
    }
}

#[test]
fn parameter_policies() {
    let mut sim = build(with_animals(grassland(24), &[("rabbit", 5)]), 3);
    // NextTick: queued, reported, applied at the next step.
    assert_eq!(sim.set_param("plant_growth", ParamValue::Float(2.0)).unwrap(), ApplyPolicy::NextTick);
    assert_eq!(sim.params().get("plant_growth"), Some(&ParamValue::Float(2.0)));
    assert_eq!(sim.runtime_params().plant_growth, 1.0);
    sim.step();
    assert_eq!(sim.runtime_params().plant_growth, 2.0);
    // Immediate.
    assert_eq!(sim.set_param("max_population_total", ParamValue::Int(7)).unwrap(), ApplyPolicy::Immediate);
    assert_eq!(sim.runtime_params().max_population_total, 7);
    // Climate goes to the world.
    sim.set_param("rainfall_mm", ParamValue::Float(1200.0)).unwrap();
    assert_eq!(sim.world().config().rainfall_mm, 1200.0);
    // RequiresReset: only after reset.
    assert_eq!(sim.set_param("width", ParamValue::Int(30)).unwrap(), ApplyPolicy::RequiresReset);
    assert_eq!(sim.world().width(), 24);
    sim.reset();
    assert_eq!((sim.world().width(), sim.tick()), (30, 0));
    // Errors.
    assert!(matches!(sim.set_param("nope", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
    assert!(matches!(sim.set_param("plant_growth", ParamValue::Float(99.0)), Err(SimError::InvalidParameter { .. })));
    assert!(matches!(
        sim.set_param("sheep_gain_from_food", ParamValue::Float(1.0)),
        Err(SimError::UnknownParameter(_))
    ));
}

#[test]
fn a_new_seed_applies_at_reset_only() {
    let config = with_animals(grassland(24), &[("rabbit", 6), ("fox", 1)]);
    let mut changed = build(config.clone(), 3);
    let mut control = build(config.clone(), 3);
    assert_eq!(changed.set_param("seed", ParamValue::Int(99)).unwrap(), ApplyPolicy::RequiresReset);
    assert_eq!(changed.params().get("seed"), Some(&ParamValue::Int(99)), "reported as pending for the reset");
    changed.advance(50);
    control.advance(50);
    assert_eq!(changed.metrics(), control.metrics(), "the running world keeps its seed");
    changed.reset();
    let fresh = build(config, 99);
    assert_eq!(changed.snapshot().unwrap(), fresh.snapshot().unwrap(), "the reset world uses the new seed");
}

#[test]
fn commands_are_validated_and_applied() {
    let mut sim = build(with_animals(grassland(24), &[("rabbit", 4)]), 4);
    sim.advance(10);
    // Spawn.
    let deer = spawn(&mut sim, "deer", 12.0, 12.0, 3);
    assert_eq!(deer.len(), 3);
    assert!(matches!(
        sim.command(EcosystemCommand::SpawnAnimals { species: "dragon".into(), x: 1.0, y: 1.0, radius: 1.0, count: 1 }),
        Err(EcosystemError::UnknownSpecies(_))
    ));
    assert!(
        sim.apply(EcosystemCommand::SpawnAnimals { species: "deer".into(), x: -5.0, y: 1.0, radius: 1.0, count: 1 })
            .is_err()
    );
    assert!(
        sim.apply(EcosystemCommand::SpawnAnimals { species: "deer".into(), x: 5.0, y: 1.0, radius: 1.0, count: 0 })
            .is_err()
    );
    // Kill → carcass; remove → gone without carcass.
    let carcasses = sim.remains().carcasses.len();
    sim.command(EcosystemCommand::KillAnimal { id: deer[0] }).unwrap();
    assert_eq!(sim.remains().carcasses.len(), carcasses + 1);
    assert_eq!(sim.remains().carcasses.last().unwrap().cause, DeathCause::Killed);
    sim.command(EcosystemCommand::RemoveAnimal { id: deer[1] }).unwrap();
    assert_eq!(sim.remains().carcasses.len(), carcasses + 1);
    assert!(matches!(
        sim.command(EcosystemCommand::KillAnimal { id: deer[1] }),
        Err(EcosystemError::UnknownOrganism(_))
    ));
    // Trees.
    let trees = sim.trees().len();
    sim.command(EcosystemCommand::PlantTree { species: "oak".into(), x: 5.5, y: 5.5 }).unwrap();
    assert_eq!(sim.trees().len(), trees + 1);
    let tree = sim.trees().ids().last().unwrap().get();
    sim.command(EcosystemCommand::FellTree { id: tree }).unwrap();
    assert_eq!(sim.trees().len(), trees);
    assert!(sim.command(EcosystemCommand::PlantTree { species: "oak".into(), x: 500.0, y: 5.0 }).is_err());
    // World interventions and season length.
    sim.command(EcosystemCommand::Disaster(Disaster::AddRain { amount_mm: 5.0 })).unwrap();
    assert!(sim.command(EcosystemCommand::Disaster(Disaster::AddRain { amount_mm: -1.0 })).is_err());
    assert!(sim.command(EcosystemCommand::Ignite { x: 99, y: 0 }).is_err());
    sim.command(EcosystemCommand::SetSeasonLength { days_per_season: 15 }).unwrap();
    assert_eq!(sim.params().get("days_per_season"), Some(&ParamValue::Int(15)));
    assert!(sim.command(EcosystemCommand::SetSeasonLength { days_per_season: 0 }).is_err());
}

#[test]
fn selection_following_view_and_inspection() {
    let mut config = with_animals(grassland(32), &[("rabbit", 10), ("fox", 2)]);
    config.trees.density_per_ha = 40.0;
    config.world.rainfall_mm = 800.0;
    let mut sim = build(config, 6);
    let rabbit = sim.animals().ids()[0].get();
    sim.command(EcosystemCommand::Select(Some(OrganismRef::Animal(rabbit)))).unwrap();
    sim.command(EcosystemCommand::Follow(Some(OrganismRef::Animal(rabbit)))).unwrap();
    assert!(sim.command(EcosystemCommand::Select(Some(OrganismRef::Animal(999_999)))).is_err());
    sim.advance(120);
    let request =
        EcosystemViewRequest { vegetation: vec![PlantLayer::Grass], block: 4, ..EcosystemViewRequest::default() };
    let view = sim.view(&request);
    assert_eq!(view.animals.len(), sim.animals().len());
    assert_eq!(view.species.len(), sim.catalog().animals().len());
    let grid = &view.vegetation[0].1;
    assert_eq!((grid.width(), grid.height()), (8, 8));
    assert!(view.world.terrain.is_some(), "static terrain is sent on the first request");
    assert!(view.trees.iter().all(|t| t.model.is_some()), "every tree has a model id");
    if sim.animals().contains(AgentId(rabbit)) {
        assert!(matches!(view.selected, Some(OrganismInspection::Animal(_))));
        assert!(view.followed.is_some() && !view.trail.is_empty());
        let Ok(OrganismInspection::Animal(i)) = sim.inspect(OrganismRef::Animal(rabbit)) else { panic!("inspection") };
        assert_eq!(i.species, "rabbit");
        assert_eq!(i.genes.len(), 6);
        assert_eq!(i.utility.len(), ecosystem::animal::decide::ACTION_COUNT);
        assert!(i.traits.bmr_kj_per_day > 0.0);
        assert!(i.history.iter().any(|e| e.kind == "decision"), "decisions of the followed animal are traced");
    }
    let region = EcosystemViewRequest { region: Some((0.0, 0.0, 8.0, 8.0)), ..EcosystemViewRequest::default() };
    let view = sim.view(&region);
    assert!(view.animals.iter().all(|a| a.position.x <= 8.0 && a.position.y <= 8.0));
    let tree = sim.trees().ids()[0].get();
    assert!(matches!(sim.inspect(OrganismRef::Tree(tree)), Ok(OrganismInspection::Tree(_))));
}

#[test]
fn invalid_scenarios_are_rejected_with_their_source() {
    let err = scenario::Scenario::parse(
        "(id: \"x\", name: \"X\", description: \"\", seed: 1, config: (animals: [(species: \"unicorn\", count: 3)]))",
        "bad.ron",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("bad.ron") && err.contains("unicorn"), "{err}");
    let err = scenario::Scenario::parse(
        "(id: \"x\", name: \"X\", description: \"\", seed: 1, config: (params: {\"plant_growth\": 50.0}))",
        "p.ron",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("plant_growth"), "{err}");
    assert!(scenario::Scenario::parse("(id: \"Bad Id\", name: \"X\", description: \"\", seed: 1)", "id.ron").is_err());
}

#[test]
fn procedural_rig_ids_exist_in_the_renderer() {
    // Every `proc:` visual id names a rig of organism-render's species table.
    let dir = workspace().join("crates/organism-render/src/procedural");
    let mut sources = String::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("species")) {
            sources.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }
    let catalog = SpeciesCatalog::builtin().unwrap();
    for s in catalog.animals() {
        if let Some(rig) = &s.visual.procedural {
            assert!(sources.contains(&format!("\"{rig}\"")), "{}: rig `{rig}` is in organism-render", s.id);
        }
    }
}

#[test]
fn views_carry_altitudes_fields_colonies_and_grids() {
    let mut sim = scenario::find_builtin("african_savanna").unwrap().build().unwrap();
    sim.advance(120); // noon: bees and ants forage
    let request = EcosystemViewRequest {
        pheromone: true,
        pollination: true,
        max_markers: 500,
        block: 4,
        ..EcosystemViewRequest::default()
    };
    let view = sim.view(&request);
    assert!(!view.colonies.is_empty() && !view.foragers.is_empty(), "colonies and their squads are drawn");
    assert!(view.field_markers.len() <= 500 && !view.field_markers.is_empty(), "markers stand for the fields");
    assert!(view.pheromone.as_ref().is_some_and(|g| g.width() == 24));
    assert!(view.pollination.is_some());
    let eagle = sim.catalog().animal_index("eagle").unwrap();
    assert!(view.animals.iter().any(|a| a.species == eagle), "eagles are in the view");
    for info in &view.species {
        assert!(!info.visual_id.is_empty(), "{} has a visual id", info.id);
    }
    let bee = view.species.iter().find(|s| s.id == "bee").unwrap();
    assert_eq!((bee.representation.as_str(), bee.visual_id.as_str()), ("colony", "proc:bee"));
    assert!(view.species.iter().find(|s| s.id == "eagle").unwrap().flight_altitude_m > 50.0);
}

#[test]
fn scheduled_parameter_changes_do_not_survive_a_reset() {
    let mut sim = scenario::find_builtin("prolonged_drought").unwrap().build().unwrap();
    let rainfall = sim.params().get("rainfall_mm").cloned();
    sim.advance(10 * 240 + 1);
    assert_eq!(sim.params().get("rainfall_mm"), Some(&ParamValue::Float(160.0)), "the drought lowered the rainfall");
    sim.reset();
    assert_eq!(sim.params().get("rainfall_mm").cloned(), rainfall, "a reset restores the scenario's climate");
    // User changes, on the other hand, persist through resets.
    sim.set_param("rainfall_mm", ParamValue::Float(500.0)).unwrap();
    sim.reset();
    assert_eq!(sim.params().get("rainfall_mm"), Some(&ParamValue::Float(500.0)));
}
