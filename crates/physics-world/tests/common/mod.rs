//! Helpers shared by the integration tests.
//!
//! Every integration-test file is its own crate and uses only some of these
//! helpers, so unused-function warnings are expected per crate.
#![allow(dead_code)]

use physics_world::{World, WorldConfig};
use simulation_core::Grid2;

/// A flat 100 m plateau without rivers or lakes, dry climate and a steady
/// westerly wind (blowing towards +x).
pub fn flat_config(size: usize, seed: u64) -> WorldConfig {
    WorldConfig {
        width: size,
        height: size,
        seed,
        river_count: 0,
        lake_tendency: 0.0,
        rainfall_mm: 300.0,
        wind_speed: 8.0,
        wind_from_deg: 270.0,
        ..WorldConfig::default()
    }
}

/// World built from a constant heightmap.
pub fn flat_world(size: usize, seed: u64) -> World {
    let cfg = flat_config(size, seed);
    World::from_heightmap(&cfg, Grid2::new(size, size, 100.0)).expect("valid flat world")
}

/// Mean of a slice of `f32` values.
pub fn mean(values: &[f32]) -> f64 {
    values.iter().map(|&v| f64::from(v)).sum::<f64>() / values.len().max(1) as f64
}
