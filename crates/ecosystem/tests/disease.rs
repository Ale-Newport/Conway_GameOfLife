//! Infectious disease: SIR dynamics emerging from contacts.

mod common;

use common::{build, grassland, with_animals};
use ecosystem::{DeathCause, EcosystemCommand, EcosystemSim, Pathogen};
use simulation_core::Simulation;

fn pathogen(transmission: f32, mortality: f32, immunity_days: f32) -> Pathogen {
    Pathogen {
        id: "pox".into(),
        name: "Sheep pox".into(),
        hosts: vec!["sheep".into()],
        transmission_per_hour: transmission,
        contact_m: 40.0,
        infectious_days: 3.0,
        mortality_per_day: mortality,
        immunity_days,
        virulence: 0.3,
    }
}

/// A flock of 60 sheep on 20 × 20 cells with 3 infected at the start;
/// returns the run and the infected count after every simulated hour.
fn epidemic(transmission: f32, mortality: f32, days: u64) -> (EcosystemSim, Vec<f64>) {
    let mut config = with_animals(grassland(20), &[("sheep", 60)]);
    config.world.rainfall_mm = 900.0;
    config.pathogens = vec![pathogen(transmission, mortality, 0.0)];
    let mut sim = build(config, 31);
    sim.command(EcosystemCommand::Outbreak { pathogen: "pox".into(), species: "sheep".into(), count: 3 }).unwrap();
    let mut series = Vec::new();
    for _ in 0..days * 24 {
        sim.advance(10);
        series.push(sim.metrics().get("infected.pox").unwrap());
    }
    (sim, series)
}

#[test]
fn an_epidemic_rises_peaks_and_burns_out() {
    let (sim, series) = epidemic(0.05, 0.0, 16);
    let peak = series.iter().copied().fold(0.0, f64::max);
    let end = *series.last().unwrap();
    let m = sim.metrics();
    let recovered = m.get("recovered.pox").unwrap();
    eprintln!("peak {peak}, final infected {end}, recovered {recovered}");
    assert!(peak >= 15.0, "the infection spreads through the flock (peak {peak})");
    assert!(end <= peak / 3.0, "and burns out as hosts become immune (end {end})");
    assert!(recovered >= 20.0, "most sheep recovered: {recovered}");
    // Lifelong immunity: nobody is infected twice, and every infection ends
    // in recovery, death or is still running.
    let d = sim.disease();
    let infected_now = m.get("infected.pox").unwrap() as u64;
    assert_eq!(d.total_infections[0], d.recoveries[0] + d.deaths[0] + infected_now);
    assert!(d.total_infections[0] <= 60, "{} infections among 60 sheep", d.total_infections[0]);
}

#[test]
fn weak_transmission_dies_out() {
    let (sim, series) = epidemic(0.0001, 0.0, 14);
    let peak = series.iter().copied().fold(0.0, f64::max);
    assert!(peak <= 4.0, "R0 < 1: the outbreak fizzles (peak {peak})");
    assert_eq!(*series.last().unwrap(), 0.0, "and ends");
    assert!(sim.disease().total_infections[0] <= 6, "few secondary cases: {:?}", sim.disease().total_infections);
}

#[test]
fn virulent_infections_kill() {
    let (sim, _) = epidemic(0.05, 1.0, 6);
    let deaths = sim.counters().deaths_by_cause[DeathCause::Disease as usize];
    assert!(deaths >= 5, "infected sheep die of the disease: {deaths}");
    assert_eq!(sim.metrics().get("disease_deaths.pox"), Some(deaths as f64));
}

#[test]
fn outbreaks_are_validated() {
    let mut config = with_animals(grassland(16), &[("sheep", 5)]);
    config.pathogens = vec![pathogen(0.05, 0.0, 10.0)];
    let mut sim = build(config, 1);
    for bad in [
        EcosystemCommand::Outbreak { pathogen: "flu".into(), species: "sheep".into(), count: 1 },
        EcosystemCommand::Outbreak { pathogen: "pox".into(), species: "rabbit".into(), count: 1 },
    ] {
        assert!(sim.command(bad).is_err());
    }
    let mut invalid = pathogen(2.0, 0.0, 0.0);
    invalid.id = "x".into();
    let mut config = with_animals(grassland(16), &[("sheep", 5)]);
    config.pathogens = vec![invalid];
    let err = ecosystem::EcosystemSim::new(config, 1, ecosystem::SpeciesCatalog::builtin().unwrap()).err().unwrap();
    assert!(err.to_string().contains("transmission_per_hour"), "{err}");
}
