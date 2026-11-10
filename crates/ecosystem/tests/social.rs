//! Social behaviour: group structure of founders, territorial home ranges
//! and defence, schools (boids), pack hunting led by the alpha, alarm calls.

mod common;

use common::{build_with, content, edit_species, grassland, ids_of, pond, position, spawn, traits_of, with_animals};
use ecosystem::{Action, EcosystemSim, ReproState, SpeciesCatalog, Target};
use organism_engine::AgentId;
use simulation_core::Simulation;
use simulation_core::glam::Vec2;

/// Mean distance (cells) from each animal of `species` to its nearest conspecific.
fn mean_nearest_neighbour(sim: &EcosystemSim, species: &str) -> f32 {
    let ids = ids_of(sim, species);
    let positions: Vec<Vec2> = ids.iter().map(|id| sim.animals().get(AgentId(*id)).unwrap().kin.position).collect();
    let total: f32 = positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            positions.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, q)| p.distance(*q)).fold(f32::MAX, f32::min)
        })
        .sum();
    total / positions.len() as f32
}

#[test]
fn herd_founders_start_in_groups_and_solitary_founders_do_not() {
    let config = with_animals(grassland(48), &[("deer", 30), ("fox", 12)]);
    let sim = common::build(config, 12);
    let deer = mean_nearest_neighbour(&sim, "deer");
    let foxes = mean_nearest_neighbour(&sim, "fox");
    // 12 foxes placed independently on 48 × 48 cells: ≈ 0.5 / √density ≈ 7 cells.
    assert!(deer < 2.0, "deer stand together in herds: {deer:.2} cells to the nearest one");
    assert!(foxes > 2.0 * deer, "solitary foxes are spread out: {foxes:.2} vs {deer:.2} cells");
}

/// Ravens whose territory is 100 m (5 cells of 20 m) around the place where
/// they were founded, displaced 25 cells away; `territorial = false` is the
/// control. Returns the mean distance to the founding place after two days.
fn displaced_ravens(territorial: bool) -> f32 {
    let mut catalog = SpeciesCatalog::builtin().unwrap();
    let mut raven = catalog.animal("raven").unwrap().clone();
    raven.social.territorial = territorial;
    raven.social.territory_radius_m = 100.0;
    catalog.insert_animal(raven).unwrap();
    let mut sim = EcosystemSim::new(grassland(64), 31, catalog).unwrap();
    sim.advance(60); // 06:00: ravens are diurnal
    let traits = traits_of(&sim, "raven");
    let mut homes = Vec::new();
    for k in 0..6 {
        let origin = Vec2::new(12.0 + 7.0 * k as f32, 12.0);
        let id = spawn(&mut sim, "raven", origin.x, origin.y, 1)[0];
        let r = sim.animal_mut(id).unwrap();
        // Adult territorial founders get a territory centre where they are
        // placed (juvenile founders get one when they mature).
        if r.is_adult() {
            assert_eq!(r.home.is_some(), territorial, "adult territorial founders get a territory centre");
        } else if territorial {
            r.home = Some(r.kin.position);
        }
        // Fed, rested and adult: the ravens only wander.
        r.energy_kj = traits.reserve_max_kj;
        r.hydration = 1.0;
        r.fatigue = 0.0;
        r.repro = ReproState::Recovering { until: u64::MAX };
        let home = r.home.unwrap_or(r.kin.position);
        r.kin.position = home + Vec2::new(0.0, 25.0);
        homes.push((id, home));
    }
    // The map has no water: keep the ravens watered so that thirst does not
    // send them searching (only territoriality should matter here).
    for _ in 0..16 {
        sim.advance(30);
        for (id, _) in &homes {
            if let Some(r) = sim.animal_mut(*id) {
                r.hydration = 1.0;
            }
        }
    }
    let distances: Vec<f32> = homes
        .iter()
        .filter_map(|(id, home)| sim.animals().get(AgentId(*id)).map(|a| a.kin.position.distance(*home)))
        .collect();
    assert!(distances.len() >= 5, "the ravens survive the test");
    distances.iter().sum::<f32>() / distances.len() as f32
}

#[test]
fn territorial_animals_return_to_their_territory() {
    let territorial = displaced_ravens(true);
    let control = displaced_ravens(false);
    eprintln!(
        "mean distance to the founding place after two days: territorial {territorial:.1}, control {control:.1} cells"
    );
    assert!(territorial < 10.0, "territorial ravens came back within twice the radius: {territorial:.1} cells");
    assert!(control > territorial + 8.0, "non-territorial ravens stay scattered: {control:.1} vs {territorial:.1}");
}

/// Mean local alignment of a species over `ticks` ticks: the cosine between
/// each animal's heading and its nearest conspecific's, averaged (0 for
/// independent headings, 1 for a perfectly aligned group).
fn local_alignment(sim: &mut EcosystemSim, species: &str, ticks: u32) -> f32 {
    let mut total = 0.0;
    for _ in 0..ticks {
        sim.step();
        let kin: Vec<_> =
            ids_of(sim, species).iter().filter_map(|id| sim.animals().get(AgentId(*id)).map(|a| a.kin)).collect();
        let mut sum = 0.0;
        for (i, k) in kin.iter().enumerate() {
            let nearest = kin
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .min_by(|a, b| a.1.position.distance(k.position).total_cmp(&b.1.position.distance(k.position)));
            if let Some((_, n)) = nearest {
                sum += k.velocity.normalize_or_zero().dot(n.velocity.normalize_or_zero());
            }
        }
        total += sum / kin.len().max(1) as f32;
    }
    total / ticks as f32
}

fn school(social: bool) -> f32 {
    // 1 m cells: a school is metres across.
    let config = with_animals(pond(60, 16.0, 1.0), &[("bream", 30)]);
    let mut sim = build_with(config, 21, |c| {
        edit_species(c, "bream", |s| {
            s.social.group_size = (30, 30);
            // Only travelling (the comparison is about how they move together).
            s.behaviour.weights.rest = 0.0;
            s.behaviour.weights.sleep = 0.0;
            s.behaviour.weights.mate = 0.0;
            s.behaviour.weights.forage = 0.0;
            if !social {
                s.social.cohesion = 0.0;
                s.social.alignment = Some(0.0);
            }
        })
    });
    sim.advance(60);
    local_alignment(&mut sim, "bream", 240)
}

#[test]
fn schools_align_their_headings() {
    let schooling = school(true);
    let control = school(false);
    eprintln!("alignment with the nearest neighbour: school {schooling:.2}, control {control:.2}");
    assert!(schooling > control + 0.2, "boids alignment orders the school: {schooling:.2} vs {control:.2}");
}

#[test]
fn pack_members_join_the_hunt_of_their_leader() {
    let mut sim = build_with(grassland(40), 8, |_| {});
    sim.advance(170); // dusk: wolves are crepuscular
    let wolves = spawn(&mut sim, "wolf", 10.0, 20.0, 4);
    let traits = traits_of(&sim, "wolf");
    for &id in &wolves {
        let w = sim.animal_mut(id).unwrap();
        w.group = Some(9_999);
        w.repro = ReproState::Recovering { until: u64::MAX };
        w.hydration = 1.0;
        w.fatigue = 0.0;
        w.energy_kj = 0.9 * traits.reserve_max_kj;
    }
    // The heaviest healthy adult leads; make it hungry so it starts a hunt.
    let leader = wolves[0];
    {
        let w = sim.animal_mut(leader).unwrap();
        w.mass_kg *= 1.3;
        w.energy_kj = 0.1 * traits.reserve_max_kj;
    }
    spawn(&mut sim, "deer", 18.0, 20.0, 3);
    let mut joined = 0;
    for _ in 0..40 {
        sim.step();
        let Some(l) = sim.animals().get(AgentId(leader)) else { break };
        if let (Action::Hunt, Target::Animal(prey)) = (l.action, l.target) {
            joined = wolves[1..]
                .iter()
                .filter(|id| {
                    sim.animals()
                        .get(AgentId(**id))
                        .is_some_and(|w| w.action == Action::Hunt && w.target == Target::Animal(prey))
                })
                .count()
                .max(joined);
        }
    }
    assert!(joined >= 2, "well-fed pack members join the leader's hunt (joined: {joined})");
}

/// An owner fox (territory radius 100 m = 5 cells) and a same-sex intruder
/// placed 2 cells from the owner's home; returns the intruder's distance
/// to that home after half a night, and the fights fought.
fn intrusion(territorial: bool) -> (f32, u64) {
    let mut sim = build_with(grassland(48), 13, |c| {
        edit_species(c, "fox", |s| {
            s.social.territorial = territorial;
            s.social.territory_radius_m = 100.0;
            s.combat.aggression = 0.9;
        })
    });
    sim.advance(10); // night: foxes are nocturnal
    let owner = spawn(&mut sim, "fox", 24.0, 24.0, 1)[0];
    let intruder = spawn(&mut sim, "fox", 26.0, 24.0, 1)[0];
    for id in [owner, intruder] {
        content(&mut sim, id);
        let a = sim.animal_mut(id).unwrap();
        a.repro = ReproState::Recovering { until: u64::MAX };
        a.sex = ecosystem::Sex::Male;
        a.home = territorial.then_some(Vec2::new(24.0, 24.0)).filter(|_| id == owner);
    }
    let mut fights = 0u64;
    for _ in 0..60 {
        sim.step();
        fights += u64::from(sim.counters().fights);
        if let Some(a) = sim.animal_mut(owner) {
            a.hydration = 1.0;
        }
        if let Some(a) = sim.animal_mut(intruder) {
            a.hydration = 1.0;
        }
    }
    let d = sim.animals().get(AgentId(intruder)).map_or(f32::MAX, |a| a.kin.position.distance(Vec2::new(24.0, 24.0)));
    (d, fights)
}

#[test]
fn territory_owners_chase_intruders_away() {
    let (defended, fights) = intrusion(true);
    let (control, _) = intrusion(false);
    eprintln!("intruder distance from the owner's home: defended {defended:.1}, control {control:.1}; fights {fights}");
    assert!(fights > 0, "the owner fights the intruder");
    assert!(defended > 5.0, "the intruder is driven beyond the territory radius: {defended:.1} cells");
    assert!(defended > control + 2.0, "without territoriality it stays closer: {control:.1} cells");
}

/// Tick at which rabbit B, out of sight and hearing of a hunting fox but
/// within hearing of rabbit A (which sees the fox), first flees. The fox
/// is held in place so that only an alarm call can reach B.
fn alarm_response(calls: bool) -> Option<u64> {
    let mut sim = build_with(grassland(48), 17, |c| edit_species(c, "rabbit", |s| s.social.alarm_calls = calls));
    sim.advance(10);
    let lair = Vec2::new(8.0, 24.0);
    let fox = spawn(&mut sim, "fox", lair.x, lair.y, 1)[0];
    let a = spawn(&mut sim, "rabbit", 12.0, 24.0, 1)[0];
    let b = spawn(&mut sim, "rabbit", 16.5, 24.0, 1)[0];
    for id in [fox, a, b] {
        content(&mut sim, id);
        // In the open, away from their burrows (a burrowed rabbit is invisible).
        sim.animal_mut(id).unwrap().home = None;
    }
    let fox_traits = traits_of(&sim, "fox");
    let rabbit = traits_of(&sim, "rabbit");
    sim.animal_mut(fox).unwrap().energy_kj = 0.05 * fox_traits.reserve_max_kj;
    let b_start = position(&sim, b);
    assert!(
        b_start.distance(lair) > rabbit.vision_cells.max(rabbit.hearing_cells),
        "B can neither see nor hear the fox"
    );
    for t in 0..20 {
        sim.step();
        let f = sim.animal_mut(fox).unwrap();
        f.kin.position = lair;
        f.kin.velocity = Vec2::ZERO;
        let rb = sim.animals().get(AgentId(b))?;
        assert!(rb.kin.position.distance(lair) > rabbit.vision_cells, "B never comes within sight of the fox");
        if rb.action == Action::Flee {
            return Some(t);
        }
    }
    None
}

#[test]
fn alarm_calls_spread_fear_beyond_sight() {
    let with_calls = alarm_response(true);
    let without = alarm_response(false);
    eprintln!("rabbit B flees out of sight of the fox at tick: with calls {with_calls:?}, without {without:?}");
    assert!(with_calls.is_some(), "the alarm of rabbit A makes rabbit B flee before it sees the fox");
    assert!(without.is_none(), "without alarm calls B only flees once it sees the fox");
}

#[test]
fn young_follow_their_mother() {
    let mut sim = build_with(grassland(40), 23, |_| {});
    let mother = spawn(&mut sim, "deer", 10.0, 20.0, 1)[0];
    content(&mut sim, mother);
    // A calf born to her (lineage), placed 4 cells away.
    let calf = spawn(&mut sim, "deer", 14.0, 20.0, 1)[0];
    {
        let traits = traits_of(&sim, "deer");
        let tick = sim.tick() as i64;
        let c = sim.animal_mut(calf).unwrap();
        c.lineage.parents[0] = Some(AgentId(mother));
        c.repro = ReproState::Juvenile;
        c.birth_tick = tick;
        c.mass_kg = traits.birth_mass_kg;
        c.energy_kj = 0.9 * c.reserve_max_kj(&traits);
        c.hydration = 1.0;
    }
    let start = position(&sim, calf).distance(position(&sim, mother));
    sim.advance(30);
    let end = position(&sim, calf).distance(position(&sim, mother));
    assert!(end < start - 1.5, "the calf closes in on its mother: {start:.1} → {end:.1} cells");
}

/// Gestating wolves among `ids`.
fn gestating(sim: &EcosystemSim, ids: &[u64]) -> Vec<u64> {
    ids.iter()
        .copied()
        .filter(|id| sim.animals().get(AgentId(*id)).is_some_and(|a| matches!(a.repro, ReproState::Gestating(_))))
        .collect()
}

#[test]
fn packs_are_founded_by_a_breeding_pair_and_only_the_dominant_pair_breeds() {
    let mut config = grassland(40);
    config.animals = vec![ecosystem::AnimalSetup {
        species: "wolf".into(),
        count: 4,
        region: Some((15.0, 15.0, 25.0, 25.0)),
        groups: Some(1),
    }];
    // Wolves that may breed in any season.
    let mut sim = build_with(config, 31, |c| edit_species(c, "wolf", |s| s.reproduction.breeding_seasons.clear()));
    let ids = ids_of(&sim, "wolf");
    let sex = |sim: &EcosystemSim, id: u64| sim.animals().get(AgentId(id)).unwrap().sex;
    let adults = |sex_wanted: ecosystem::Sex| {
        ids.iter()
            .filter(|id| {
                let a = sim.animals().get(AgentId(**id)).unwrap();
                a.sex == sex_wanted && a.is_adult()
            })
            .count()
    };
    assert!(
        adults(ecosystem::Sex::Female) >= 1 && adults(ecosystem::Sex::Male) >= 1,
        "a breeding pair founds the pack"
    );
    // Two adult females and two adult males, all ready and in prime
    // condition; the heavier female and male rank first.
    let traits = traits_of(&sim, "wolf");
    let (mut females, mut males) = (Vec::new(), Vec::new());
    for (k, id) in ids.iter().enumerate() {
        let female = k % 2 == 0;
        let a = sim.animal_mut(*id).unwrap();
        a.sex = if female { ecosystem::Sex::Female } else { ecosystem::Sex::Male };
        a.repro = ReproState::Ready;
        a.mass_kg = traits.adult_mass_kg * if k < 2 { 1.1 } else { 0.9 };
        a.energy_kj = 0.95 * a.reserve_max_kj(&traits);
        a.hydration = 1.0;
        a.health = 1.0;
        a.fatigue = 0.0;
        if female { females.push(*id) } else { males.push(*id) }
    }
    let (dominant, subordinate) = (females[0], females[1]);
    for _ in 0..4 {
        sim.advance(240);
        if !gestating(&sim, &females).is_empty() {
            break;
        }
    }
    sim.advance(240);
    assert_eq!(gestating(&sim, &females), vec![dominant], "only the dominant female conceives");
    let sub = sim.animals().get(AgentId(subordinate)).unwrap();
    assert!(matches!(sub.repro, ReproState::Ready) && sub.health > 0.5, "the subordinate female could breed");
    assert_eq!(sex(&sim, subordinate), ecosystem::Sex::Female);
    // Without the dominant female the next one breeds.
    sim.command(ecosystem::EcosystemCommand::RemoveAnimal { id: dominant }).unwrap();
    for _ in 0..4 {
        for id in &ids {
            if let Some(a) = sim.animal_mut(*id) {
                a.energy_kj = a.energy_kj.max(0.9 * a.reserve_max_kj(&traits));
                a.hydration = 1.0;
            }
        }
        sim.advance(240);
        if !gestating(&sim, &females).is_empty() {
            break;
        }
    }
    assert_eq!(gestating(&sim, &females), vec![subordinate], "the next-ranking female takes over");
}
