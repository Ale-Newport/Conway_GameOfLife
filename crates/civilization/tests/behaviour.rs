//! Behavioural tests of the whole model: emergent roads, scarcity, groups,
//! conflict switches, determinism and snapshots.

use civilization::config::{Dist, PopulationSpec, PreferenceDist, Region, WorldSpec};
use civilization::{BuildingKind, CivilizationSim, Scenario, TerrainSpec, scenarios};
use simulation_core::{ParamSet, ParamValue, Simulation};

fn preset(id: &str) -> CivilizationSim {
    CivilizationSim::new(scenarios::builtin_by_id(id).expect("built-in scenario")).expect("valid scenario")
}

fn prefs(cooperation: f32) -> PreferenceDist {
    let d = |mean: f32| Dist { mean, sd: 0.05 };
    PreferenceDist { risk: d(0.4), cooperation: d(cooperation), trade: d(0.4), time: d(0.5) }
}

fn map_scenario(rows: &[&str], count: u32, region: Region, params: ParamSet) -> Scenario {
    Scenario {
        id: "test".into(),
        name: "Test".into(),
        description: "test map".into(),
        seed: 5,
        world: WorldSpec {
            terrain: TerrainSpec::Map { rows: rows.iter().map(|r| r.to_string()).collect() },
            richness: 1.0,
        },
        populations: vec![PopulationSpec {
            label: "Testers".into(),
            count,
            preferences: prefs(0.5),
            region: Some(region),
            kin_affinity: 0.0,
            starting_food: 2.0,
        }],
        params,
    }
}

/// Sum of a counter metric over `ticks` steps.
fn run_counting(sim: &mut CivilizationSim, ticks: u64, keys: &[&str]) -> Vec<f64> {
    let mut totals = vec![0.0; keys.len()];
    for _ in 0..ticks {
        sim.step();
        let m = sim.metrics();
        for (t, k) in totals.iter_mut().zip(keys) {
            *t += m.get(k).unwrap_or(0.0);
        }
    }
    totals
}

#[test]
fn roads_form_on_a_forced_corridor_between_resources() {
    // Water only in the west, food only in the east, joined by one corridor
    // through a mountain range (rows 7–8, columns 12–27).
    let mut rows: Vec<String> = Vec::new();
    for y in 0..16 {
        let mut row = String::new();
        for x in 0..40 {
            let c = match (x, y) {
                (12..=27, 7 | 8) => '.',
                (12..=27, _) => '^',
                (2..=5, 5..=10) => '~',
                (31..=38, 2..=13) => 'b',
                _ => '.',
            };
            row.push(c);
        }
        rows.push(row);
    }
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let params = ParamSet::new()
        .with("births_enabled", ParamValue::Bool(false))
        .with("conflict_enabled", ParamValue::Bool(false));
    let scenario = map_scenario(&rows, 16, Region { x0: 0.0, y0: 0.0, x1: 0.25, y1: 1.0 }, params);
    let mut sim = CivilizationSim::new(scenario).unwrap();
    sim.advance(1500);
    let world = sim.world();
    let threshold = sim.rules().road_threshold;
    let corridor: Vec<u32> = (14..=25).flat_map(|x| [7, 8].map(|y| world.index(x, y))).collect();
    let corridor_mean = corridor.iter().map(|&t| world.road(t)).sum::<f32>() / corridor.len() as f32;
    let mut elsewhere: Vec<f32> = (0..world.len() as u32)
        .filter(|t| world.passable(*t) && !corridor.contains(t))
        .map(|t| world.road(t))
        .collect();
    let elsewhere_mean = elsewhere.iter().sum::<f32>() / elsewhere.len() as f32;
    elsewhere.sort_by(f32::total_cmp);
    let p90 = elsewhere[elsewhere.len() * 9 / 10];
    let mut corridor_values: Vec<f32> = corridor.iter().map(|&t| world.road(t)).collect();
    corridor_values.sort_by(f32::total_cmp);
    let corridor_median = corridor_values[corridor_values.len() / 2];
    // The busiest corridor row is a continuous road from range to range.
    let continuous = [7u32, 8].iter().any(|&y| (14..=25).all(|x| world.road(world.index(x, y)) >= threshold))
        || (14..=25).all(|x| world.road(world.index(x, 7)).max(world.road(world.index(x, 8))) >= threshold);
    assert!(sim.agents().len() >= 8, "most agents survive by commuting: {}", sim.agents().len());
    assert!(corridor_mean > 0.3, "corridor trail intensity {corridor_mean}");
    assert!(corridor_mean > 3.0 * elsewhere_mean, "corridor {corridor_mean} vs elsewhere {elsewhere_mean}");
    assert!(corridor_median > p90, "corridor median {corridor_median} vs 90th percentile elsewhere {p90}");
    assert!(continuous, "a road spans the corridor");
    assert!(sim.metrics().get("road_length").unwrap() >= 12.0);
}

#[test]
fn scarcity_lowers_population_relative_to_abundance() {
    let mut rich = preset("abundant");
    let mut poor = preset("scarcity");
    // The presets share settlers, seed and the shares of water, forest and
    // hills; the scarce one has rare berries, smaller stocks and slower food.
    let keys = ["births", "deaths_starvation", "deaths_thirst"];
    let rich_counts = run_counting(&mut rich, 2500, &keys);
    let poor_counts = run_counting(&mut poor, 2500, &keys);
    let (rich_m, poor_m) = (rich.metrics(), poor.metrics());
    let pop = |m: &simulation_core::Metrics| m.get("population").unwrap();
    assert!(pop(&poor_m) < 0.75 * pop(&rich_m), "scarcity {} vs abundance {}", pop(&poor_m), pop(&rich_m));
    assert!(poor_counts[0] < rich_counts[0], "fewer births under scarcity: {poor_counts:?} vs {rich_counts:?}");
    assert!(poor_counts[1] > rich_counts[1], "more starvation under scarcity: {poor_counts:?} vs {rich_counts:?}");
    assert!(poor_counts[1] > poor_counts[2], "hunger, not thirst, is what kills: {poor_counts:?}");
    assert!(poor_m.get("food_stock").unwrap() < rich_m.get("food_stock").unwrap());
}

#[test]
fn groups_emerge_and_cooperators_bond_more_than_competitors() {
    let mut sim = preset("cooperation_vs_competition");
    sim.advance(400);
    assert!(sim.groups().len() > 1, "groups: {}", sim.groups().len());
    sim.advance(1100);
    let share = |population: u8| {
        let members: Vec<_> = sim.agents().iter().filter(|(_, a)| a.population == population).collect();
        members.iter().filter(|(_, a)| a.group.is_some()).count() as f64 / members.len().max(1) as f64
    };
    let (cooperators, competitors) = (share(0), share(1));
    assert!(cooperators > competitors + 0.15, "grouped: cooperators {cooperators:.2}, competitors {competitors:.2}");
    // Group colours and ids are distinct; members agree with group sizes.
    let groups: Vec<_> = sim.groups().iter().collect();
    for g in &groups {
        let n = sim.agents().iter().filter(|(_, a)| a.group == Some(g.id)).count();
        assert_eq!(n as u32, g.members);
        assert!(n >= civilization::groups::MIN_GROUP_SIZE);
    }
}

#[test]
fn conflict_switch_controls_theft_and_fights() {
    let keys = ["thefts", "conflicts", "deaths_conflict"];
    let mut peaceful = preset("scarcity");
    peaceful.set_param("conflict_enabled", ParamValue::Bool(false)).unwrap();
    assert_eq!(run_counting(&mut peaceful, 2500, &keys), vec![0.0, 0.0, 0.0]);
    let mut violent = preset("scarcity");
    let counts = run_counting(&mut violent, 2500, &keys);
    assert!(counts[0] + counts[1] > 0.0, "desperate agents steal or fight: {counts:?}");
}

#[test]
fn agents_without_water_die_of_thirst() {
    let rows = vec!["bbbbbbbbbbbb"; 12];
    let scenario =
        map_scenario(&rows, 6, Region::WHOLE, ParamSet::new().with("births_enabled", ParamValue::Bool(false)));
    let mut sim = CivilizationSim::new(scenario).unwrap();
    let deaths = run_counting(&mut sim, 400, &["deaths_thirst", "deaths", "deaths_starvation"]);
    assert_eq!(deaths, vec![6.0, 6.0, 0.0], "every death is from thirst");
    assert!(sim.is_finished());
    assert!(sim.events().iter().filter(|e| e.kind == "death").all(|e| e.message.contains("thirst")));
}

#[test]
fn runs_are_deterministic_and_seed_dependent() {
    let run = |seed: Option<i64>| {
        let mut sim = preset("trade_routes");
        if let Some(seed) = seed {
            sim.set_param("seed", ParamValue::Int(seed)).unwrap();
            sim.reset();
        }
        sim.advance(400);
        sim.snapshot().unwrap().payload
    };
    assert_eq!(run(None), run(None), "same seed, same trajectory");
    assert_ne!(run(None), run(Some(1234)), "another seed differs");
}

#[test]
fn snapshot_round_trip_replays_exactly() {
    let mut sim = preset("two_strategies");
    sim.advance(300);
    let snap = sim.snapshot().unwrap();
    sim.advance(300);
    let later = sim.snapshot().unwrap();
    let later_metrics = sim.metrics();
    // Restoring into a different simulation instance replays identically.
    let mut other = preset("abundant");
    other.restore(&snap).unwrap();
    assert_eq!(other.tick(), 300);
    assert_eq!(other.scenario().id, "two_strategies");
    other.advance(300);
    assert_eq!(other.snapshot().unwrap(), later);
    assert_eq!(other.metrics(), later_metrics);
    // Reset after restore returns to the restored scenario's start.
    other.reset();
    assert_eq!(other.tick(), 0);
    assert_eq!(other.scenario().id, "two_strategies");
    // Foreign or corrupted snapshots are rejected without changing the state.
    let mut foreign = snap.clone();
    foreign.model_id = "ecosystem".into();
    assert!(other.restore(&foreign).is_err());
    let mut corrupt = snap.clone();
    corrupt.payload.truncate(corrupt.payload.len() / 2);
    assert!(other.restore(&corrupt).is_err());
    let mut newer = snap;
    newer.schema_version = 99;
    assert!(other.restore(&newer).is_err());
    assert_eq!(other.tick(), 0);
}

#[test]
fn barter_between_separated_regions_saves_lives_and_raises_specialisation() {
    // Same world, seed and settlers; only the barter switch differs.
    let run = |trade: bool| {
        let mut sim = preset("trade_routes");
        sim.set_param("trade_enabled", ParamValue::Bool(trade)).unwrap();
        let totals = run_counting(&mut sim, 1500, &["deaths", "trade_volume"]);
        (totals, sim.metrics())
    };
    let ((with_counts, with), (without_counts, without)) = (run(true), run(false));
    assert!(with_counts[1] > 0.0, "barter happens");
    assert_eq!(without_counts[1], 0.0, "no barter when switched off");
    assert!(
        with_counts[0] < without_counts[0],
        "deaths with barter {} vs without {}",
        with_counts[0],
        without_counts[0]
    );
    let spec = |m: &simulation_core::Metrics| m.get("specialization").unwrap();
    assert!(spec(&with) > spec(&without) + 0.05, "specialisation {} vs {}", spec(&with), spec(&without));
    assert!(with.get("road_length").unwrap() > 0.0, "journeys leave roads");
}

#[test]
fn surplus_food_goes_home_so_building_plans_complete() {
    // Regression: without household stores, harvests filled the carrying
    // capacity with food, nobody could gather materials and no group store
    // was ever built in this preset.
    let mut sim = preset("abundant");
    sim.advance(1500);
    let stores = sim.buildings().iter().filter(|b| b.is_complete() && b.kind == BuildingKind::Storage).count();
    assert!(stores >= 1, "group stores are built");
    let adults: Vec<_> = sim.agents().iter().filter(|(_, a)| a.is_adult()).collect();
    let full = adults.iter().filter(|(_, a)| a.free_capacity() < 1.0).count();
    assert!(full * 4 <= adults.len(), "{full} of {} adults cannot carry anything more", adults.len());
    let at_home: f32 = sim
        .buildings()
        .iter()
        .filter(|b| b.kind == BuildingKind::Hut)
        .map(|b| b.store.get(civilization::Good::Food))
        .sum();
    assert!(at_home > 0.0, "huts hold household food");
    // Gathering saturates: people do not carry food far beyond what they keep.
    for (_, a) in &adults {
        assert!(a.inventory.get(civilization::Good::Food) <= a.food_to_carry() + 12.0);
    }
}

#[test]
fn snapshots_keep_scenario_rule_overrides() {
    // `scarcity` overrides rule parameters; those untagged values must survive
    // a snapshot round trip (postcard alone cannot decode them).
    let mut sim = preset("scarcity");
    assert!(!sim.scenario().params.is_empty(), "the scenario has rule overrides");
    sim.advance(50);
    let snap = sim.snapshot().unwrap();
    let mut other = preset("abundant");
    other.restore(&snap).expect("restore a scenario with rule overrides");
    assert_eq!(other.scenario().params, sim.scenario().params);
    sim.advance(50);
    other.advance(50);
    assert_eq!(other.metrics(), sim.metrics());
}
