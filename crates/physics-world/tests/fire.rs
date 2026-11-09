//! Fire: wind-driven spread, no spread without fuel, extinction by rain,
//! fuel-consumption reporting and lightning.

mod common;

use physics_world::{Disaster, FireState, World, WorldConfig};

/// Flat, dry world with uniform fuel and rain suppressed.
fn fuelled_world(size: usize, seed: u64, fuel: f32) -> World {
    let mut world = common::flat_world(size, seed);
    world.fuel_mut().fill(fuel);
    world.start_drought(1_000_000).expect("valid drought");
    world
}

/// Columns of the easternmost and westernmost cells touched by fire.
fn fire_extent(world: &World) -> (usize, usize) {
    let w = world.width();
    let touched = world
        .fire()
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_active() || matches!(s, FireState::Burned { .. }))
        .map(|(i, _)| i % w);
    touched.fold((usize::MAX, 0), |(lo, hi), x| (lo.min(x), hi.max(x)))
}

#[test]
fn fire_spreads_faster_downwind_than_upwind() {
    let size = 64;
    let centre = size / 2;
    let (mut downwind_total, mut upwind_total, mut downwind_wins) = (0usize, 0usize, 0);
    let seeds = 12;
    for seed in 0..seeds {
        let mut world = fuelled_world(size, seed, 1.5);
        assert!(world.ignite(centre, centre).expect("in bounds"), "dry fuel must ignite");
        for _ in 0..20 {
            world.step();
        }
        let (west, east) = fire_extent(&world);
        let downwind = east - centre;
        let upwind = centre - west;
        downwind_total += downwind;
        upwind_total += upwind;
        if downwind > upwind {
            downwind_wins += 1;
        }
    }
    // A westerly wind (blowing towards +x) drives the front east.
    assert!(downwind_wins >= 10, "downwind ahead in only {downwind_wins} of {seeds} runs");
    assert!(
        downwind_total as f64 > 2.0 * upwind_total as f64,
        "downwind {downwind_total} vs upwind {upwind_total} cells"
    );
}

#[test]
fn fire_does_not_spread_without_fuel() {
    let mut world = fuelled_world(32, 1, 0.0);
    assert!(!world.ignite(16, 16).expect("in bounds"), "no fuel, no fire");
    world.set_fuel(16, 16, 2.0).expect("valid fuel");
    assert!(world.ignite(16, 16).expect("in bounds"));
    let mut consumed = 0.0;
    for _ in 0..40 {
        let report = world.step();
        consumed += report.fuel_consumed.iter().map(|b| f64::from(b.amount)).sum::<f64>();
        assert!(report.fuel_consumed.iter().all(|b| (b.x, b.y) == (16, 16)));
    }
    assert!(world.burning_cells().is_empty(), "the fire burns out");
    assert_eq!(world.total_burned_cells(), 1, "only the fuelled cell burned");
    assert!(matches!(world.fire().get(16, 16), FireState::Burned { .. }));
    let left = f64::from(*world.fuel().get(16, 16));
    assert!((consumed + left - 2.0).abs() < 1e-4, "consumed {consumed} + left {left} = initial fuel");
}

#[test]
fn rain_extinguishes_fire() {
    // Standing water from heavy rain puts every fire out at once.
    let mut world = fuelled_world(48, 3, 2.0);
    for y in 10..38 {
        world.ignite(24, y).expect("in bounds");
    }
    world.step();
    assert!(world.burning_cells().len() > 20);
    world.add_rain(30.0).expect("valid rain");
    let report = world.step();
    assert_eq!(report.burning_cells, 0, "30 mm of rain extinguishes all fires");
    assert!(report.burned_cells > 20);

    // A storm (rain rate up to tens of mm/h) extinguishes fires much faster
    // than dry weather.
    let ignite_line = |world: &mut World| {
        for y in 5..43 {
            world.ignite(24, y).expect("in bounds");
        }
    };
    let (mut stormy_left, mut dry_left) = (0, 0);
    for seed in 0..4 {
        let mut stormy = fuelled_world(48, seed, 3.0);
        let mut dry = fuelled_world(48, seed, 3.0);
        stormy.trigger(Disaster::Storm { duration_ticks: 1000, intensity: 4.0 }).expect("valid storm");
        ignite_line(&mut stormy);
        ignite_line(&mut dry);
        for _ in 0..8 {
            stormy.step();
            dry.step();
        }
        stormy_left += stormy.burning_cells().len();
        dry_left += dry.burning_cells().len();
    }
    assert!(stormy_left * 4 < dry_left, "storm left {stormy_left} burning cells, dry weather {dry_left}");
}

#[test]
fn lightning_ignites_fires_during_storms() {
    let cfg = WorldConfig { lightning_rate: 5.0, ..common::flat_config(64, 4) };
    let mut world = World::from_heightmap(&cfg, simulation_core::Grid2::new(64, 64, 100.0)).expect("valid world");
    world.fuel_mut().fill(1.0);
    world.storm(200, 0.05).expect("valid storm");
    let mut ignitions = 0;
    let mut lightning_events = 0;
    for _ in 0..60 {
        let report = world.step();
        ignitions += report.ignitions;
        lightning_events += report.events.iter().filter(|e| e.kind == "fire").count();
    }
    assert!(lightning_events > 0 && ignitions > 0, "storms with a high strike rate must start fires");
}

#[test]
fn burned_cells_recover_when_fuel_regrows() {
    let mut world = fuelled_world(16, 2, 0.0);
    world.set_fuel(8, 8, 1.0).expect("valid fuel");
    world.ignite(8, 8).expect("in bounds");
    for _ in 0..30 {
        world.step();
    }
    assert!(matches!(world.fire().get(8, 8), FireState::Burned { .. }));
    world.set_fuel(8, 8, 1.0).expect("valid fuel");
    world.step();
    assert!(matches!(world.fire().get(8, 8), FireState::Burned { .. }), "recovery needs a full day");
    for _ in 0..world.calendar().ticks_per_day() {
        world.step();
    }
    assert_eq!(world.fire().get(8, 8), &FireState::Fuel);
}

#[test]
fn fire_spreads_faster_uphill_than_downhill() {
    // A calm ramp rising 25 m per 50 m cell towards +x (slope angle 26.6°):
    // the Alexandridis slope factor e^{0.078 θ} is 7.9 uphill and 0.13
    // downhill (orthogonal neighbours).
    let size = 64;
    let centre = size / 2;
    let (mut uphill_reach, mut downhill_reach, mut uphill_cells, mut downhill_cells) = (0, 0, 0, 0);
    for seed in 0..8 {
        let cfg = WorldConfig { wind_speed: 0.0, ..common::flat_config(size, seed) };
        let ramp = simulation_core::Grid2::from_fn(size, size, |x, _| 20.0 + 25.0 * x as f32);
        let mut world = World::from_heightmap(&cfg, ramp).expect("valid ramp");
        world.fuel_mut().fill(1.5);
        world.start_drought(1_000_000).expect("valid drought");
        assert!(world.ignite(centre, centre).expect("in bounds"));
        for _ in 0..15 {
            world.step();
        }
        let (west, east) = fire_extent(&world);
        uphill_reach += east - centre;
        downhill_reach += centre - west;
        for (i, state) in world.fire().as_slice().iter().enumerate() {
            if state.is_active() || matches!(state, FireState::Burned { .. }) {
                match (i % size).cmp(&centre) {
                    std::cmp::Ordering::Greater => uphill_cells += 1,
                    std::cmp::Ordering::Less => downhill_cells += 1,
                    std::cmp::Ordering::Equal => {}
                }
            }
        }
    }
    assert!(uphill_reach * 2 > downhill_reach * 3, "reach uphill {uphill_reach} vs downhill {downhill_reach} cells");
    assert!(uphill_cells > 3 * downhill_cells, "burned uphill {uphill_cells} vs downhill {downhill_cells} cells");
}

#[test]
fn lightning_strikes_land_with_the_configured_density() {
    // Half of the map is sea. Strikes are spread over the whole map, so the
    // land receives `rate × land km² × hours` of them on average.
    let size = 64;
    let cfg = WorldConfig { lightning_rate: 2.0, ..common::flat_config(size, 6) };
    let coast = simulation_core::Grid2::from_fn(size, size, |x, _| if x < size / 2 { -20.0 } else { 100.0 });
    let mut world = World::from_heightmap(&cfg, coast).expect("valid coast");
    let land = world.surface().as_slice().iter().filter(|s| **s != physics_world::SurfaceKind::Ocean).count();
    assert_eq!(land, size * size / 2);
    world.storm(10_000, 0.01).expect("valid storm");
    let ticks = 1000;
    let strikes: u32 = (0..ticks).map(|_| world.step().lightning_strikes).sum();
    let hours = f64::from(ticks) * f64::from(world.calendar().dt_hours());
    let expected = 2.0 * land as f64 * cfg.cell_area_m2() / 1e6 * hours;
    // Poisson(≈ 1024): ±15 % is about five standard deviations.
    assert!((f64::from(strikes) / expected - 1.0).abs() < 0.15, "{strikes} strikes on land, expected {expected:.0}");
}
