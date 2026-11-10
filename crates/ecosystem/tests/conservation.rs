//! Energy and nutrient accounting.

mod common;

use common::{build, grassland, with_animals, with_params};
use ecosystem::{EcosystemCommand, Flux, FluxKind};
use physics_world::Disaster;
use simulation_core::{ParamValue, Simulation};

#[test]
fn nutrients_are_conserved_without_external_fluxes() {
    // Closed budget in a drying climate: no deposition, no volatilisation and
    // (without percolation) no leaching at all.
    let mut config = with_animals(grassland(40), &[("rabbit", 30), ("deer", 6), ("fox", 3), ("raven", 3)]);
    config.trees.density_per_ha = 25.0;
    config.world.base_temperature_c = 16.0;
    let config = with_params(config, &[("external_fluxes", ParamValue::Bool(false))]);
    let mut sim = build(config, 21);
    // Generated as grassland, then without rain: soils dry below field
    // capacity, so nothing percolates and nothing is leached.
    sim.set_param("rainfall_mm", ParamValue::Float(0.0)).unwrap();
    let start = sim.stocks().nutrients();
    for _ in 0..10 {
        sim.advance(240);
        let stock = sim.stocks().nutrients();
        assert!((stock - start).abs() / start < 1e-5, "nutrient stock drifted: {start} → {stock}");
    }
    let n = &sim.ledgers().nutrients;
    assert_eq!(n.total(Flux::Leaching), 0.0, "no percolation without rain");
    assert_eq!(n.inputs(), 0.0);
    assert_eq!(n.outputs(), 0.0);
    // The cycle actually turned over.
    for flux in [Flux::PrimaryProduction, Flux::Herbivory, Flux::Litterfall, Flux::Mineralisation, Flux::Egestion] {
        assert!(n.total(flux) > 0.0, "{flux:?} moved nutrients");
    }
}

/// A wet grassland where heavy rain percolates below the root zone.
fn wet_grassland(external_fluxes: bool) -> ecosystem::EcosystemSim {
    let mut config = with_animals(grassland(32), &[("rabbit", 20), ("deer", 4), ("fox", 2)]);
    config.trees.density_per_ha = 25.0;
    config.world.rainfall_mm = 1500.0;
    let config = with_params(
        config,
        &[("external_fluxes", ParamValue::Bool(external_fluxes)), ("deposition_g_m2_year", ParamValue::Float(0.0))],
    );
    build(config, 23)
}

fn drench(sim: &mut ecosystem::EcosystemSim) {
    for _ in 0..4 {
        sim.command(EcosystemCommand::Disaster(Disaster::AddRain { amount_mm: 40.0 })).unwrap();
        sim.advance(240);
    }
}

#[test]
fn a_closed_budget_retains_leached_nutrients() {
    let mut sim = wet_grassland(false);
    let start = sim.stocks().nutrients();
    drench(&mut sim);
    let stock = sim.stocks().nutrients();
    assert!((stock - start).abs() / start < 1e-5, "nutrient stock drifted: {start} → {stock}");
    let n = &sim.ledgers().nutrients;
    assert!(n.total(Flux::LeachingRetained) > 0.0, "percolating water leached nutrients");
    assert_eq!(n.total(Flux::Leaching), 0.0, "nothing left the ecosystem");
    assert_eq!((n.inputs(), n.outputs()), (0.0, 0.0));
    // The physics world itself did leach: the nutrients came back to the soil.
    assert!(sim.world().nutrient_budget().leached_g > 0.0);
}

#[test]
fn open_budget_loses_leached_nutrients_and_still_balances() {
    let mut sim = wet_grassland(true);
    let start = sim.stocks().nutrients();
    drench(&mut sim);
    let stocks = sim.stocks();
    let n = &sim.ledgers().nutrients;
    assert!(n.total(Flux::Leaching) > 0.0 && n.total(Flux::LeachingRetained) == 0.0);
    assert!(stocks.nutrients() < start, "leaching removed nutrients");
    assert!(n.relative_error(stocks.nutrients()) < 1e-5, "books close: {}", n.relative_error(stocks.nutrients()));
}

#[test]
fn ledgers_close_with_external_fluxes_fire_and_interventions() {
    let config = with_params(
        with_animals(grassland(32), &[("rabbit", 20), ("fox", 2)]),
        &[("deposition_g_m2_year", ParamValue::Float(5.0)), ("volatilisation_fraction", ParamValue::Float(0.6))],
    );
    let mut sim = build(config, 22);
    sim.advance(120);
    sim.command(EcosystemCommand::Disaster(Disaster::Drought { duration_ticks: 300 })).unwrap();
    sim.advance(5);
    sim.command(EcosystemCommand::Ignite { x: 10, y: 10 }).unwrap();
    sim.command(EcosystemCommand::SpawnAnimals { species: "deer".into(), x: 20.0, y: 20.0, radius: 2.0, count: 3 })
        .unwrap();
    let victim = sim.animals().ids()[0].get();
    sim.command(EcosystemCommand::RemoveAnimal { id: victim }).unwrap();
    for _ in 0..6 {
        sim.advance(200);
        let stocks = sim.stocks();
        let e = sim.ledgers().energy.relative_error(stocks.energy());
        let n = sim.ledgers().nutrients.relative_error(stocks.nutrients());
        assert!(e < 1e-5 && n < 1e-5, "energy error {e}, nutrient error {n}");
    }
    let n = &sim.ledgers().nutrients;
    assert!(n.total(Flux::Deposition) > 0.0 && n.total(Flux::Volatilisation) > 0.0 && n.total(Flux::Ash) > 0.0);
    assert!(sim.ledgers().energy.total(Flux::Combustion) > 0.0);
    assert!(
        sim.ledgers().energy.total(Flux::InterventionIn) > 0.0
            && sim.ledgers().energy.total(Flux::InterventionOut) > 0.0
    );
    assert!(sim.metrics().get("nutrient_balance_error").unwrap() < 1e-5);
}

#[test]
fn balanced_meadow_books_balance() {
    let mut sim = ecosystem::scenario::find_builtin("balanced_meadow").unwrap().build().unwrap();
    sim.advance(480);
    let stocks = sim.stocks();
    let e = &sim.ledgers().energy;
    let energy_error = e.relative_error(stocks.energy());
    let nutrient_error = sim.ledgers().nutrients.relative_error(stocks.nutrients());
    assert!(energy_error < 1e-5, "{energy_error}");
    assert!(nutrient_error < 1e-5, "{nutrient_error}");
    // Every booked flux has the role the ledger declares.
    for flux in Flux::ALL {
        if flux.energy_kind() == FluxKind::Unused {
            assert_eq!(sim.ledgers().energy.total(*flux), 0.0, "{flux:?}");
        }
        if flux.nutrient_kind() == FluxKind::Unused {
            assert_eq!(sim.ledgers().nutrients.total(*flux), 0.0, "{flux:?}");
        }
    }
    assert!(e.total(Flux::PrimaryProduction) > 0.0 && e.total(Flux::AnimalRespiration) > 0.0);
}
