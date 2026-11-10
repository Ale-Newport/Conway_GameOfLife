//! Behaviour and rule tests: grazing, predation, fleeing, reproduction,
//! mortality, fire and regrowth, floods.

mod common;

use common::{build, grassland, ids_of, spawn, traits_of, with_params};
use ecosystem::{
    DeathCause, EcosystemCommand, EcosystemSim, Flux, PlantLayer, ReproState, Sex, SpeciesCatalog, VegetationSetup,
};
use organism_engine::AgentId;
use physics_world::Disaster;
use simulation_core::{ParamValue, Simulation};

fn total_grass(sim: &ecosystem::EcosystemSim) -> f64 {
    sim.vegetation().total_kg(PlantLayer::Grass, sim.world().config().cell_area_m2())
}

fn distance(sim: &ecosystem::EcosystemSim, a: u64, b: u64) -> f32 {
    let pa = sim.animals().get(AgentId(a)).expect("alive").kin.position;
    let pb = sim.animals().get(AgentId(b)).expect("alive").kin.position;
    pa.distance(pb)
}

#[test]
fn grazing_removes_exactly_the_grass_that_is_eaten() {
    // Two identical worlds without plant growth; one with a hungry rabbit.
    let config = with_params(grassland(16), &[("plant_growth", ParamValue::Float(0.0))]);
    let mut control = build(config.clone(), 3);
    let mut grazed = build(config, 3);
    control.advance(60);
    grazed.advance(60);
    let rabbit = spawn(&mut grazed, "rabbit", 8.0, 8.0, 1)[0];
    {
        let traits = traits_of(&grazed, "rabbit");
        let r = grazed.animal_mut(rabbit).unwrap();
        r.energy_kj = 0.2 * traits.reserve_max_kj;
        r.fatigue = 0.0;
    }
    let reserve_before = grazed.animals().get(AgentId(rabbit)).unwrap().energy_kj;
    let herbivory_before = grazed.ledgers().energy.total(Flux::Herbivory);
    for _ in 0..60 {
        control.step();
        grazed.step();
    }
    let eaten_kj = grazed.ledgers().energy.total(Flux::Herbivory) - herbivory_before;
    assert!(eaten_kj > 0.0, "the hungry rabbit grazed");
    let eaten_kg = eaten_kj / ecosystem::units::PLANT_KJ_PER_KG;
    let missing = total_grass(&control) - total_grass(&grazed);
    assert!((missing - eaten_kg).abs() < 0.05 * eaten_kg + 1e-6, "missing {missing} kg vs eaten {eaten_kg} kg");
    let r = grazed.animals().get(AgentId(rabbit)).expect("alive");
    assert!(r.energy_kj > reserve_before, "assimilated energy exceeds the metabolism of an hour of grazing");
    assert!(r.stats.eaten_kj > 0.0);
}

#[test]
fn predation_transfers_prey_energy_to_the_predator() {
    let config = with_params(grassland(24), &[("predation_efficiency", ParamValue::Float(20.0))]);
    let mut sim = build(config, 5);
    sim.advance(60);
    let wolf = spawn(&mut sim, "wolf", 12.0, 12.0, 1)[0];
    let deer = spawn(&mut sim, "deer", 13.0, 12.0, 1)[0];
    let wolf_traits = sim.traits()[usize::from(sim.catalog().animal_index("wolf").unwrap())].clone();
    {
        let w = sim.animal_mut(wolf).unwrap();
        w.energy_kj = 0.15 * wolf_traits.reserve_max_kj;
        w.fatigue = 0.0;
        w.repro = ReproState::Ready;
    }
    let mut killed_at = None;
    for t in 0..400 {
        sim.step();
        if sim.animals().get(AgentId(deer)).is_none() {
            killed_at = Some(t);
            break;
        }
    }
    assert!(killed_at.is_some(), "the wolf caught the deer");
    let carcass =
        sim.remains().carcasses.iter().find(|c| c.killer == Some(AgentId(wolf))).expect("carcass of the kill");
    assert_eq!(carcass.cause, DeathCause::Predation);
    let at_kill = sim.animals().get(AgentId(wolf)).unwrap().energy_kj;
    for _ in 0..40 {
        sim.step();
    }
    let eaten = sim.ledgers().energy.total(Flux::Predation);
    assert!(eaten > 0.0, "meat of the kill was eaten");
    let w = sim.animals().get(AgentId(wolf)).expect("the wolf survives");
    assert!(w.energy_kj > at_kill, "the predator's reserves grew: {} → {}", at_kill, w.energy_kj);
    assert!(w.stats.kills >= 1);
    // Egested meat went to the litter.
    assert!(sim.ledgers().energy.total(Flux::Egestion) > 0.0);
}

/// A fox that hunts but cannot move, so the rabbit's own behaviour decides
/// the distance.
fn frozen_fox_catalog(rabbit_flees: bool) -> SpeciesCatalog {
    let mut catalog = SpeciesCatalog::builtin().unwrap();
    let mut fox = catalog.animal("fox").unwrap().clone();
    fox.movement.sprint_speed_m_s = Some(0.001);
    catalog.insert_animal(fox).unwrap();
    if !rabbit_flees {
        let mut rabbit = catalog.animal("rabbit").unwrap().clone();
        rabbit.behaviour.weights.flee = 0.0;
        catalog.insert_animal(rabbit).unwrap();
    }
    catalog
}

fn flee_distance(rabbit_flees: bool) -> (f32, f32) {
    let config = grassland(40);
    let mut sim = ecosystem::EcosystemSim::new(config, 9, frozen_fox_catalog(rabbit_flees)).unwrap();
    sim.advance(60); // 06:00, the rabbits' active time
    let fox = spawn(&mut sim, "fox", 20.0, 20.0, 1)[0];
    let rabbit = spawn(&mut sim, "rabbit", 23.0, 20.0, 1)[0];
    let fox_traits = sim.traits()[usize::from(sim.catalog().animal_index("fox").unwrap())].clone();
    {
        let f = sim.animal_mut(fox).unwrap();
        f.energy_kj = 0.1 * fox_traits.reserve_max_kj;
        f.fatigue = 0.0;
        f.repro = ReproState::Ready;
        f.mass_kg = fox_traits.adult_mass_kg;
        let r = sim.animal_mut(rabbit).unwrap();
        r.fatigue = 0.0;
        // No burrow nearby: the rabbit has to run (hiding is another valid escape).
        r.home = None;
        r.kin.position.x = 23.0;
        r.kin.position.y = 20.0;
    }
    let start = distance(&sim, fox, rabbit);
    // The immediate response: once out of sight of a motionless fox the
    // rabbit forgets it and may wander back.
    for _ in 0..3 {
        sim.step();
    }
    (start, distance(&sim, fox, rabbit))
}

#[test]
fn prey_flee_from_a_hunting_predator() {
    let (start, fled) = flee_distance(true);
    let (_, control) = flee_distance(false);
    assert!(fled > start + 3.0, "the rabbit ran away: {start} → {fled}");
    assert!(fled > control + 2.0, "fleeing rabbit {fled} cells away, non-fleeing control {control}");
}

#[test]
fn mating_produces_offspring_that_inherit_parental_genes() {
    let config = with_params(
        grassland(24),
        &[("mutation_rate", ParamValue::Float(0.0)), ("predation_efficiency", ParamValue::Float(0.05))],
    );
    let mut sim = build(config, 11);
    sim.advance(60);
    let pair = spawn(&mut sim, "rabbit", 12.0, 12.0, 2);
    let rabbit = sim.catalog().animal_index("rabbit").unwrap();
    let traits = sim.traits()[usize::from(rabbit)].clone();
    for (k, &id) in pair.iter().enumerate() {
        let a = sim.animal_mut(id).unwrap();
        a.sex = if k == 0 { Sex::Female } else { Sex::Male };
        a.repro = ReproState::Ready;
        a.mass_kg = traits.adult_mass_kg;
        a.energy_kj = 0.95 * traits.reserve_max_kj;
        a.hydration = 1.0;
        a.birth_tick = -(traits.maturity_ticks as i64) - 10;
    }
    let (mother, father) = (pair[0], pair[1]);
    let mother_genes = sim.animals().get(AgentId(mother)).unwrap().genome.clone();
    let father_genes = sim.animals().get(AgentId(father)).unwrap().genome.clone();
    let mut young = Vec::new();
    for _ in 0..(traits.gestation_ticks as usize + 600) {
        sim.step();
        young = ids_of(&sim, "rabbit").into_iter().filter(|id| !pair.contains(id)).collect();
        if !young.is_empty() {
            break;
        }
    }
    assert!(!young.is_empty(), "the pair produced offspring within one gestation");
    for id in &young {
        let child = sim.animals().get(AgentId(*id)).unwrap();
        assert_eq!(child.lineage.parents, [Some(AgentId(mother)), Some(AgentId(father))]);
        assert_eq!(child.lineage.generation, 1);
        for (i, v) in child.genome.values().iter().enumerate() {
            assert!(
                *v == mother_genes.values()[i] || *v == father_genes.values()[i],
                "gene {i} comes from a parent (no mutation)"
            );
        }
        assert!(sim.genealogy().get(AgentId(*id)).is_some());
    }
    let m = sim.animals().get(AgentId(mother)).expect("mother alive");
    assert!(matches!(m.repro, ReproState::Recovering { .. }));
    assert_eq!(m.stats.offspring as usize, young.len());
}

#[test]
fn weaned_juveniles_get_their_water_from_fresh_forage() {
    // No open water anywhere: a young rabbit must cover its water need from
    // the grass it eats, measured against its own (not an adult's) needs.
    let config = with_params(grassland(16), &[("predation_efficiency", ParamValue::Float(0.05))]);
    let mut sim = build(config, 14);
    sim.advance(60);
    let young = spawn(&mut sim, "rabbit", 8.0, 8.0, 1)[0];
    let traits = traits_of(&sim, "rabbit");
    let now = sim.tick() as i64;
    {
        let r = sim.animal_mut(young).unwrap();
        r.repro = ReproState::Juvenile;
        // Just weaned: a small fraction of the adult mass, moderately fed.
        r.birth_tick = now - (0.2 * traits.maturity_ticks) as i64;
        r.mass_kg = 0.08 * traits.adult_mass_kg;
        r.energy_kj = 0.6 * r.reserve_max_kj(&traits);
        r.hydration = 0.7;
    }
    for _ in 0..(2 * 240) {
        sim.step();
    }
    let r = sim.animals().get(AgentId(young)).expect("the young rabbit survives two days without drinking");
    assert!(r.stats.eaten_kj > 0.0, "it grazed");
    assert!(r.hydration > 0.6, "fresh forage kept it hydrated: {}", r.hydration);
    assert_eq!(sim.counters().deaths_by_cause[DeathCause::Dehydration as usize], 0);
}

#[test]
fn starvation_and_old_age_kill() {
    // No plants at all: a rabbit with almost empty reserves starves.
    let mut config = grassland(12);
    config.vegetation = VegetationSetup { initial_fraction: 0.0, seed_bank: 0.0 };
    let mut sim = build(config, 2);
    let hungry = spawn(&mut sim, "rabbit", 6.0, 6.0, 1)[0];
    let old = spawn(&mut sim, "rabbit", 3.0, 3.0, 1)[0];
    let traits = traits_of(&sim, "rabbit");
    sim.animal_mut(hungry).unwrap().energy_kj = 0.01 * traits.reserve_max_kj;
    {
        let o = sim.animal_mut(old).unwrap();
        o.birth_tick = -((2.5 * traits.lifespan_ticks) as i64);
        o.energy_kj = traits.reserve_max_kj;
    }
    for _ in 0..(3 * 240) {
        sim.step();
    }
    let causes = sim.counters().deaths_by_cause;
    assert!(sim.animals().get(AgentId(hungry)).is_none(), "the starving rabbit died");
    assert!(sim.animals().get(AgentId(old)).is_none(), "the very old rabbit died");
    assert!(causes[DeathCause::Starvation as usize] >= 1, "{causes:?}");
    assert!(causes[DeathCause::OldAge as usize] >= 1, "{causes:?}");
    assert!(sim.ledgers().energy.total(Flux::AnimalMortality) > 0.0, "bodies became carcasses");
}

#[test]
fn fire_burns_vegetation_which_then_regrows() {
    let mut sim = build(grassland(32), 4);
    // Generated as grassland; a wetter climate afterwards lets it regrow.
    sim.set_param("rainfall_mm", ParamValue::Float(800.0)).unwrap();
    sim.advance(120); // noon
    sim.command(EcosystemCommand::Disaster(Disaster::Drought { duration_ticks: 240 })).unwrap();
    sim.advance(10);
    let before = total_grass(&sim);
    sim.command(EcosystemCommand::Ignite { x: 16, y: 16 }).expect("dry grass ignites");
    let mut burned = 0.0;
    for _ in 0..240 {
        sim.step();
        burned += sim.counters().burned_kg;
    }
    let after_fire = total_grass(&sim);
    assert!(burned > 0.0 && sim.ledgers().energy.total(Flux::Combustion) > 0.0, "biomass burned");
    assert!(after_fire < 0.8 * before, "grass burned: {before} → {after_fire}");
    assert!(sim.ledgers().nutrients.total(Flux::Ash) > 0.0, "ash returned nutrients to the soil");
    // Rain returns; grass regrows from survivors and the seed bank.
    sim.command(EcosystemCommand::Disaster(Disaster::AddRain { amount_mm: 20.0 })).unwrap();
    let mut lowest = after_fire;
    for _ in 0..30 {
        sim.advance(240);
        lowest = lowest.min(total_grass(&sim));
    }
    let regrown = total_grass(&sim);
    assert!(regrown > 1.5 * lowest, "grass regrows: lowest {lowest} → {regrown}");
}

#[test]
fn animals_escape_a_fire_front_to_burned_ground() {
    // A wind-driven grass fire sweeps a dry plain from west to east; deer
    // grazing on the leeward (east) side cannot outrun it to the map edge,
    // so they dash through the front to the burned ground behind it.
    let mut config = grassland(36);
    config.world.wind_speed = 7.0;
    config.world.wind_from_deg = 270.0;
    config.world.lightning_rate = 0.0;
    let mut sim = build(config, 15);
    sim.advance(120);
    sim.command(EcosystemCommand::Disaster(Disaster::Drought { duration_ticks: 2000 })).unwrap();
    sim.advance(240);
    let deer = spawn(&mut sim, "deer", 30.0, 18.0, 10);
    for &id in &deer {
        common::content(&mut sim, id);
    }
    for y in (2..34).step_by(4) {
        // Some cells may be too green to ignite; the line still starts the fire.
        let _ = sim.command(EcosystemCommand::Ignite { x: 2, y });
    }
    for _ in 0..480 {
        sim.step();
        for &id in &deer {
            if let Some(d) = sim.animal_mut(id) {
                d.hydration = 1.0; // no water on this plain
            }
        }
    }
    let burned = sim.world().burned_area_cells();
    let survivors = deer.iter().filter(|id| sim.animals().contains(AgentId(**id))).count();
    let burned_to_death = sim.counters().deaths_by_cause[DeathCause::Fire as usize];
    eprintln!("fire burned {burned} of 1296 cells; {survivors} of 10 deer survived, {burned_to_death} animals burned");
    assert!(burned > 600, "the fire swept most of the plain ({burned} cells)");
    assert!(survivors >= 6, "deer escape to burned ground: {survivors} of 10 survived");
}

#[test]
fn floods_drown_non_swimmers_but_not_swimmers() {
    let mut sim = build(grassland(20), 8);
    sim.advance(60);
    let rabbits = spawn(&mut sim, "rabbit", 10.0, 10.0, 6);
    let deer = spawn(&mut sim, "deer", 10.0, 10.0, 4);
    sim.command(EcosystemCommand::Disaster(Disaster::Flood { x: 10, y: 10, radius: 40.0, depth_m: 3.0 })).unwrap();
    for _ in 0..12 {
        sim.step();
    }
    let drowned = rabbits.iter().filter(|id| sim.animals().get(AgentId(**id)).is_none()).count();
    assert!(drowned >= 4, "{drowned} of 6 rabbits drowned");
    assert!(deer.iter().all(|id| sim.animals().get(AgentId(*id)).is_some()), "deer swim");
    assert!(sim.counters().deaths_by_cause[DeathCause::Drowning as usize] >= 4);
}

/// A gravid female of `species` whose eggs are due lays a clutch of
/// `litter` on the next tick (the surplus of the generous embryo budget
/// returns to her); a hungry rat is then released next to the clutch.
/// Returns the run, the mother, the clutch as laid and the rat.
fn clutch_and_rat(species: &str, litter: u16) -> (EcosystemSim, u64, ecosystem::carcass::Clutch, u64) {
    let mut sim = build(grassland(24), 12);
    sim.advance(60);
    let mother = spawn(&mut sim, species, 12.0, 12.0, 1)[0];
    let tick = sim.tick();
    {
        let m = sim.animal_mut(mother).unwrap();
        m.sex = Sex::Female;
        m.repro = ReproState::Gestating(Box::new(ecosystem::animal::Pregnancy {
            conceived: tick,
            due: tick,
            litter,
            father: None,
            father_lineage: None,
            father_genome: None,
            embryo_kj: 500.0,
            embryo_g: 5.0,
        }));
    }
    sim.step();
    let clutch = sim.remains().clutches.first().cloned().expect("the mother laid a clutch");
    assert_eq!(clutch.eggs, litter);
    let rat = spawn(&mut sim, "rat", clutch.position.x + 0.5, clutch.position.y, 1)[0];
    {
        let traits = traits_of(&sim, "rat");
        let r = sim.animal_mut(rat).unwrap();
        r.energy_kj = 0.2 * r.reserve_max_kj(&traits);
        r.hydration = 1.0;
        r.home = None;
    }
    (sim, mother, clutch, rat)
}

/// Steps until the clutch is gone (at most `ticks`); whether it went.
fn until_clutch_gone(sim: &mut EcosystemSim, clutch: u64, ticks: u32) -> bool {
    for _ in 0..ticks {
        sim.step();
        if !sim.remains().clutches.iter().any(|c| c.id == clutch) {
            return true;
        }
    }
    false
}

#[test]
fn nest_predators_raid_clutches() {
    let (mut sim, _, clutch, _) = clutch_and_rat("lizard", 6);
    let predation_before = sim.ledgers().energy.total(Flux::Predation);
    until_clutch_gone(&mut sim, clutch.id, 60);
    let remaining = sim.remains().clutches.iter().find(|c| c.id == clutch.id).map_or(0, |c| c.eggs);
    assert!(remaining < clutch.eggs, "the rat ate eggs: {remaining} of {} left", clutch.eggs);
    assert!(sim.ledgers().energy.total(Flux::Predation) > predation_before, "egg predation is booked");
}

#[test]
fn birds_that_lose_their_clutch_lay_again() {
    for (species, relays) in [("songbird", true), ("lizard", false)] {
        let (mut sim, mother, clutch, rat) = clutch_and_rat(species, 4);
        assert!(
            matches!(sim.animals().get(AgentId(mother)).unwrap().repro, ReproState::Recovering { .. }),
            "{species}: after laying the mother waits for her next breeding"
        );
        // Keep the rat hungry until the whole clutch is eaten.
        let traits = traits_of(&sim, "rat");
        let mut gone = false;
        for _ in 0..8 {
            if let Some(r) = sim.animal_mut(rat) {
                r.energy_kj = 0.2 * r.reserve_max_kj(&traits);
                r.hydration = 1.0;
            }
            if until_clutch_gone(&mut sim, clutch.id, 30) {
                gone = true;
                break;
            }
        }
        assert!(gone, "{species}: the rat eats the whole clutch");
        let repro = &sim.animals().get(AgentId(mother)).expect("the mother is alive").repro;
        assert_eq!(matches!(repro, ReproState::Ready), relays, "{species}: replacement clutch {relays} ({repro:?})");
    }
}

#[test]
fn injuries_slow_animals_down_and_heal() {
    let mut sim = build(grassland(32), 14);
    sim.advance(60);
    let deer = spawn(&mut sim, "deer", 16.0, 16.0, 1)[0];
    let traits = traits_of(&sim, "deer");
    {
        let d = sim.animal_mut(deer).unwrap();
        d.injury = 0.8;
        d.energy_kj = 0.9 * d.reserve_max_kj(&traits);
        d.hydration = 1.0;
    }
    let a = sim.animals().get(AgentId(deer)).unwrap();
    assert!(a.impairment() < 0.6, "an injured deer runs at {} of its speed", a.impairment());
    let mut injury = 0.8;
    for _ in 0..4 {
        sim.advance(240);
        if let Some(d) = sim.animal_mut(deer) {
            d.hydration = 1.0;
            d.energy_kj = d.energy_kj.max(0.8 * d.reserve_max_kj(&traits));
            injury = d.injury;
        }
    }
    assert!(injury < 0.3, "injuries heal over days when the animal is fed: {injury}");
}
