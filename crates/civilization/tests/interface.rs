//! Commands, parameters, views, descriptor and rule invariants observed
//! through the public interface.

use civilization::view::TileRect;
use civilization::{
    BuildingKind, CivCommand, CivViewRequest, CivilizationSim, Good, Resource, Terrain, scenarios, territory,
};
use simulation_core::{ApplyPolicy, ParamValue, SimError, Simulation};

fn preset(id: &str) -> CivilizationSim {
    CivilizationSim::new(scenarios::builtin_by_id(id).expect("built-in scenario")).expect("valid scenario")
}

#[test]
fn spawn_command_adds_agents_and_validates_input() {
    let mut sim = preset("abundant");
    let before = sim.agents().len();
    let land = (0..sim.world().len() as u32).find(|&t| sim.world().passable(t)).unwrap();
    let (x, y) = sim.world().coords(land);
    sim.apply(CivCommand::SpawnAgents { x: x as f32 + 0.5, y: y as f32 + 0.5, count: 5, population: 0 }).unwrap();
    assert_eq!(sim.agents().len(), before + 5);
    let newcomers: Vec<_> = sim.agents().iter().rev().take(5).collect();
    assert!(newcomers.iter().all(|(_, a)| a.is_adult() && a.memory.water.is_some()));
    let bad = [
        CivCommand::SpawnAgents { x: 1.0, y: 1.0, count: 0, population: 0 },
        CivCommand::SpawnAgents { x: 1.0, y: 1.0, count: 3, population: 7 },
        CivCommand::SpawnAgents { x: -4.0, y: 1.0, count: 3, population: 0 },
        CivCommand::SpawnAgents { x: f32::NAN, y: 1.0, count: 3, population: 0 },
        CivCommand::SpawnAgents { x: 1e6, y: 1.0, count: 3, population: 0 },
    ];
    for command in bad {
        assert!(matches!(sim.apply(command), Err(SimError::InvalidCommand(_))));
    }
    assert_eq!(sim.agents().len(), before + 5);
}

#[test]
fn resource_commands_change_tiles() {
    let mut sim = preset("abundant");
    let (cx, cy) = (32, 32);
    sim.apply(CivCommand::AddResources { x: cx, y: cy, radius: 3.0, resource: Resource::Berries }).unwrap();
    let w = sim.world();
    let centre = w.index(cx, cy);
    if w.building_at(centre) == 0 {
        assert_eq!(w.terrain(centre), Terrain::Berries);
        assert_eq!(w.stock(centre), w.capacity(centre));
    }
    sim.apply(CivCommand::RemoveResources { x: cx, y: cy, radius: 3.0, resource: Some(Resource::Berries) }).unwrap();
    let w = sim.world();
    let emptied = (0..w.len() as u32)
        .filter(|&t| w.terrain(t) == Terrain::Berries && w.center(t).distance(w.center(centre)) <= 3.0)
        .all(|t| w.stock(t) == 0.0);
    assert!(emptied);
    assert!(sim.apply(CivCommand::AddResources { x: cx, y: cy, radius: 2.0, resource: Resource::Crops }).is_err());
    assert!(sim.apply(CivCommand::RemoveResources { x: 9999, y: 0, radius: 2.0, resource: None }).is_err());
    assert!(sim.apply(CivCommand::RemoveResources { x: 1, y: 1, radius: 0.0, resource: None }).is_err());
    assert!(sim.apply(CivCommand::RemoveResources { x: 1, y: 1, radius: f32::INFINITY, resource: None }).is_err());
}

#[test]
fn regeneration_command_and_parameters_agree() {
    let mut sim = preset("scarcity");
    sim.apply(CivCommand::SetRegeneration { resource: Resource::Wood, multiplier: 2.5 }).unwrap();
    assert_eq!(sim.params().get("regen_wood"), Some(&ParamValue::Float(2.5)));
    assert!(sim.apply(CivCommand::SetRegeneration { resource: Resource::Wood, multiplier: -1.0 }).is_err());
    assert_eq!(sim.set_param("regen_wood", ParamValue::Float(0.5)), Ok(ApplyPolicy::NextTick));
    assert_eq!(sim.rules().regen_of(Resource::Wood), 0.5);
    // Scenario overrides survive (scarcity sets berry growback to 0.6) and rule
    // changes survive a reset.
    assert_eq!(sim.params().get("regen_berries"), Some(&ParamValue::Float(f64::from(0.6f32))));
    sim.advance(10);
    sim.reset();
    assert_eq!(sim.rules().regen_of(Resource::Wood), 0.5);
    assert!(matches!(sim.set_param("metabolism", ParamValue::Float(9.0)), Err(SimError::InvalidParameter { .. })));
    assert!(matches!(sim.set_param("warp_speed", ParamValue::Bool(true)), Err(SimError::UnknownParameter(_))));
}

#[test]
fn seed_requires_reset() {
    let mut sim = preset("abundant");
    let terrain_before: Vec<Terrain> = sim.world().terrain_grid().as_slice().to_vec();
    assert_eq!(sim.set_param("seed", ParamValue::Int(99)), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.world().terrain_grid().as_slice(), &terrain_before[..], "not applied before reset");
    sim.reset();
    assert_eq!(sim.seed(), 99);
    assert_ne!(sim.world().terrain_grid().as_slice(), &terrain_before[..]);
    assert!(sim.set_param("seed", ParamValue::Int(-1)).is_err());
}

#[test]
fn disaster_destroys_buildings_in_radius() {
    let mut sim = preset("territorial_expansion");
    sim.advance(600);
    let target = sim.buildings().iter().find(|b| b.is_complete()).map(|b| b.tile).expect("buildings after 600 ticks");
    let (x, y) = sim.world().coords(target);
    let before = sim.buildings().len();
    sim.apply(CivCommand::Disaster { x, y, radius: 4.0 }).unwrap();
    let w = sim.world();
    assert!(sim.buildings().len() < before);
    assert!(sim.buildings().iter().all(|b| w.center(b.tile).distance(w.center(target)) > 4.0));
    assert_eq!(w.building_at(target), 0);
    assert!(w.danger(target) > 0.0);
    assert!(sim.agents().iter().all(|(_, a)| a.home.is_none_or(|h| sim.buildings().get(h).is_some())));
    assert!(sim.events().iter().any(|e| e.kind == "disaster"));
    // Territory follows the surviving buildings at once (not at the next day).
    assert_territory_explained(&sim);
}

#[test]
fn reseed_keeps_the_seed_parameter_representable() {
    let mut sim = preset("abundant");
    sim.reseed(u64::MAX).unwrap();
    assert_eq!(sim.seed(), i64::MAX as u64);
    assert_eq!(sim.params().get("seed"), Some(&ParamValue::Int(i64::MAX)));
    assert_eq!(sim.tick(), 0);
}

#[test]
fn snapshots_of_an_older_schema_are_rejected() {
    let mut sim = preset("abundant");
    sim.advance(5);
    let mut old = sim.snapshot().unwrap();
    old.schema_version = civilization::sim::SCHEMA_VERSION - 1;
    assert!(matches!(sim.restore(&old), Err(SimError::UnsupportedVersion { .. })));
    assert_eq!(sim.tick(), 5, "state unchanged");
}

#[test]
fn large_area_edits_are_consistent() {
    // A radius-30 edit rewrites most of the map in one batch (region labels
    // are rebuilt once per command, not once per tile).
    let mut sim = preset("territorial_expansion");
    let before: Vec<Terrain> = sim.world().terrain_grid().as_slice().to_vec();
    sim.apply(CivCommand::AddResources { x: 48, y: 48, radius: 30.0, resource: Resource::Wood }).unwrap();
    let w = sim.world();
    let centre = w.center(w.index(48, 48));
    for t in 0..w.len() as u32 {
        let inside = w.center(t).distance(centre) <= 30.0;
        let expected = if inside && w.building_at(t) == 0 { Terrain::Forest } else { before[t as usize] };
        assert_eq!(w.terrain(t), expected, "tile {t}");
    }
    // Regions agree with a fresh labelling of the edited terrain.
    let mut fresh = w.clone();
    fresh.rebuild_regions();
    let land: Vec<u32> = (0..w.len() as u32).filter(|&t| w.passable(t)).collect();
    for &t in land.iter().step_by(37) {
        for &u in land.iter().step_by(53) {
            assert_eq!(w.connected(t, u), fresh.connected(t, u));
        }
    }
}

/// Every claimed tile is reached by a finished building of its owner, and
/// the group statistics add up to the claimed area.
fn assert_territory_explained(sim: &CivilizationSim) {
    let w = sim.world();
    let falloff = sim.rules().territory_falloff;
    let mut claimed = 0;
    for t in 0..w.len() as u32 {
        let Some(owner) = w.claim(t).owner else { continue };
        claimed += 1;
        let members = sim.groups().get(owner).map(|g| g.members).unwrap_or(0) as f32;
        // Some finished building of the owner reaches this tile.
        let reached = sim.buildings().iter().filter(|b| b.is_complete() && b.group == Some(owner)).any(|b| {
            let strength = territory::BASE_STRENGTH * b.kind.influence() * (1.0 + 0.25 * members.max(1.0).ln());
            strength - falloff * w.center(b.tile).distance(w.center(t)) > 0.0
        });
        assert!(reached, "tile {t} claimed by {owner:?} without a building in range");
    }
    let total: u32 = sim.groups().iter().map(|g| g.territory).sum();
    assert_eq!(total, claimed);
}

#[test]
fn territory_is_explained_by_group_buildings() {
    let mut sim = preset("territorial_expansion");
    sim.advance(1200);
    assert!((0..sim.world().len() as u32).any(|t| sim.world().claim(t).owner.is_some()), "territory is claimed");
    assert_territory_explained(&sim);
}

#[test]
fn economy_develops_buildings_tools_skills_and_children() {
    let mut sim = preset("abundant");
    sim.advance(1500);
    let built = |kind: BuildingKind| sim.buildings().iter().filter(|b| b.is_complete() && b.kind == kind).count();
    assert!(built(BuildingKind::Hut) > 5, "huts");
    assert!(built(BuildingKind::FarmPlot) > 2, "farm plots");
    let children: Vec<_> = sim.agents().iter().filter(|(_, a)| a.generation > 0).collect();
    assert!(!children.is_empty(), "children are born");
    for (_, child) in &children {
        let [mother, father] = child.parents;
        assert!(mother.is_some() && father.is_some());
    }
    assert!(sim.agents().iter().any(|(_, a)| a.inventory.get(Good::Tools) > 0.0), "tools are crafted");
    let m = sim.metrics();
    assert!(m.get("mean_skill").unwrap() > 0.1);
    assert!((0.0..=1.0).contains(&m.get("specialization").unwrap()));
    assert!((0.0..=1.0).contains(&m.get("gini_wealth").unwrap()));
}

#[test]
fn view_respects_requests_and_regions() {
    let mut sim = preset("trade_routes");
    sim.advance(50);
    let full = sim.view(&CivViewRequest::default());
    let n = (full.world_width * full.world_height) as usize;
    assert_eq!((full.terrain.len(), full.roads.len(), full.territory.len(), full.resources.len()), (n, n, n, n));
    assert_eq!(full.agents.len(), sim.agents().len());
    let region = TileRect { x: 10, y: 5, width: 20, height: 15 };
    let part = sim.view(&CivViewRequest { region: Some(region), tiles: true, roads: false, territory: false });
    assert_eq!(part.region, region);
    assert_eq!(part.terrain.len(), 300);
    assert!(part.roads.is_empty() && part.territory.is_empty());
    assert!(
        part.agents
            .iter()
            .all(|a| a.position.x >= 10.0 && a.position.x < 30.0 && a.position.y >= 5.0 && a.position.y < 20.0)
    );
    assert_eq!(part.terrain[0], sim.world().terrain(sim.world().index(10, 5)));
    // Regions are clipped to the world.
    let clipped = sim
        .view(&CivViewRequest { region: Some(TileRect { x: 70, y: 40, width: 50, height: 50 }), ..Default::default() });
    assert_eq!((clipped.region.width, clipped.region.height), (10, 8));
}

#[test]
fn descriptor_matches_parameters_and_metrics() {
    let sim = preset("abundant");
    let d = sim.descriptor();
    assert_eq!(d.id, civilization::MODEL_ID);
    assert!(d.rules.contains("MRS") && d.rules.contains("trail"));
    let params = sim.params();
    for spec in &d.params {
        let value = params.get(&spec.key).unwrap_or_else(|| panic!("missing value for {}", spec.key));
        assert!(spec.validate(value).is_ok(), "{} = {value}", spec.key);
    }
    assert_eq!(params.len(), d.params.len());
    let metrics = sim.metrics();
    let keys: Vec<&str> = metrics.keys().collect();
    let declared: Vec<&str> = d.metrics.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, declared);
    assert!(sim.memory_bytes() > 0);
}
