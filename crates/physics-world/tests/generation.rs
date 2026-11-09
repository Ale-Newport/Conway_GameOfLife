//! Terrain generation: determinism, ocean connectivity, drainage, biome
//! diversity, presets and soils.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use physics_world::climate::annual_mean_temperature;
use physics_world::{Biome, BiomePreset, OFF_MAP, SoilType, SurfaceKind, World, WorldConfig, WorldError};
use simulation_core::Grid2;

fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    (-1..=1)
        .flat_map(move |dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
        .filter(move |&(nx, ny)| (nx, ny) != (x, y) && nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64)
        .map(move |(nx, ny)| ny as usize * w + nx as usize)
}

fn biome_counts(world: &World) -> BTreeMap<Biome, usize> {
    let mut counts = BTreeMap::new();
    for b in world.biome().as_slice() {
        *counts.entry(*b).or_insert(0) += 1;
    }
    counts
}

fn dominant_land_biome(world: &World) -> Biome {
    let counts = biome_counts(world);
    counts
        .iter()
        .filter(|(b, _)| !b.is_water() && **b != Biome::Mountain)
        .max_by_key(|(b, c)| (**c, std::cmp::Reverse(**b)))
        .map(|(b, _)| *b)
        .expect("map has land")
}

#[test]
fn generation_is_deterministic_per_seed() {
    let cfg = WorldConfig::with_size(96, 80).with_seed(42);
    let a = World::generate(&cfg);
    let b = World::generate(&cfg);
    assert_eq!(a.elevation(), b.elevation());
    assert_eq!(a.biome(), b.biome());
    assert_eq!(a.soil(), b.soil());
    assert_eq!(a.surface(), b.surface());
    assert_eq!(a.water_depth(), b.water_depth());
    let c = World::generate(&cfg.clone().with_seed(43));
    assert_ne!(a.elevation(), c.elevation());
}

#[test]
fn ocean_is_connected_to_the_border() {
    for (seed, island) in [(1, false), (2, false), (3, true), (4, true)] {
        let cfg = WorldConfig { island, ..WorldConfig::with_size(128, 96).with_seed(seed) };
        let world = World::generate(&cfg);
        let (w, h) = (world.width(), world.height());
        let surface = world.surface().as_slice();
        let is_border = |i: usize| i.is_multiple_of(w) || i / w == 0 || i % w == w - 1 || i / w == h - 1;
        let mut reached = vec![false; w * h];
        let mut queue: VecDeque<usize> =
            (0..w * h).filter(|&i| is_border(i) && surface[i] == SurfaceKind::Ocean).collect();
        for &i in &queue {
            reached[i] = true;
        }
        while let Some(i) = queue.pop_front() {
            for j in neighbours(i, w, h) {
                if !reached[j] && surface[j] == SurfaceKind::Ocean {
                    reached[j] = true;
                    queue.push_back(j);
                }
            }
        }
        let ocean = surface.iter().filter(|s| **s == SurfaceKind::Ocean).count();
        assert!(ocean > 0, "seed {seed}: expected some ocean");
        for i in 0..w * h {
            if surface[i] == SurfaceKind::Ocean {
                assert!(reached[i], "seed {seed}: ocean cell {i} not connected to the border");
                assert!(world.elevation().as_slice()[i] < 0.0);
            }
            if island && is_border(i) {
                assert_eq!(surface[i], SurfaceKind::Ocean, "island border must be ocean");
            }
        }
    }
}

#[test]
fn rivers_flow_downhill_to_sea_lakes_or_edge() {
    for seed in [1, 5, 9] {
        let cfg = WorldConfig { river_count: 12, ..WorldConfig::with_size(128, 128).with_seed(seed) };
        let world = World::generate(&cfg);
        let (w, h) = (world.width(), world.height());
        let surface = world.surface().as_slice();
        let z = world.elevation().as_slice();
        let receivers = world.flow_receivers().as_slice();
        let downstream = |i: usize| -> Option<usize> {
            let r = receivers[i];
            if r >= 8 {
                return None;
            }
            let (dx, dy) = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)][r as usize];
            let (nx, ny) = ((i % w) as i64 + dx, (i / w) as i64 + dy);
            Some(ny as usize * w + nx as usize)
        };
        let rivers: Vec<usize> = (0..w * h).filter(|&i| surface[i] == SurfaceKind::River).collect();
        assert!(rivers.len() >= 12, "seed {seed}: only {} river cells", rivers.len());
        for &start in &rivers {
            let mut cur = start;
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps <= w * h, "seed {seed}: drainage cycle from {start}");
                match downstream(cur) {
                    None => {
                        assert_eq!(receivers[cur], OFF_MAP, "a river may only end at the map edge");
                        break;
                    }
                    Some(next) => {
                        if surface[cur] == SurfaceKind::River && surface[next] == SurfaceKind::River {
                            assert!(z[next] < z[cur], "seed {seed}: river climbs from {cur} to {next}");
                        }
                        if surface[next] == SurfaceKind::Ocean || surface[next] == SurfaceKind::Lake {
                            break;
                        }
                        if surface[cur] == SurfaceKind::River {
                            assert_eq!(surface[next], SurfaceKind::River, "rivers are continuous");
                        }
                        cur = next;
                    }
                }
            }
        }
    }
}

#[test]
fn default_world_has_diverse_biomes_and_consistent_soils() {
    for seed in [1, 2, 3] {
        let world = World::generate(&WorldConfig::default().with_seed(seed));
        let counts = biome_counts(&world);
        assert!(counts.len() >= 8, "seed {seed}: only {} biomes: {counts:?}", counts.len());
        let distinct_soils: BTreeSet<SoilType> = world.soil().as_slice().iter().copied().collect();
        assert!(distinct_soils.len() >= 5, "seed {seed}: soils {distinct_soils:?}");
        for i in 0..world.width() * world.height() {
            let biome = world.biome().as_slice()[i];
            let fertility = world.fertility().as_slice()[i];
            assert!((0.0..=1.0).contains(&fertility));
            match biome {
                Biome::Ocean | Biome::Reef => assert_eq!(world.surface().as_slice()[i], SurfaceKind::Ocean),
                Biome::Lake => assert_eq!(world.surface().as_slice()[i], SurfaceKind::Lake),
                Biome::River => assert_eq!(world.surface().as_slice()[i], SurfaceKind::River),
                Biome::Beach => assert_eq!(world.soil().as_slice()[i], SoilType::Sand),
                _ => assert_eq!(world.surface().as_slice()[i], SurfaceKind::Land),
            }
            if biome.is_water() {
                assert_eq!(fertility, 0.0);
                assert!(world.nutrients().as_slice()[i] == 0.0);
            }
        }
    }
}

#[test]
fn presets_bias_the_landscape() {
    let expectations = [
        (BiomePreset::Savanna, Biome::Savanna),
        (BiomePreset::Rainforest, Biome::TropicalRainforest),
        (BiomePreset::Tundra, Biome::Tundra),
        (BiomePreset::Desert, Biome::Desert),
        (BiomePreset::Temperate, Biome::TemperateForest),
    ];
    for (preset, biome) in expectations {
        for seed in [1, 2] {
            let world = World::generate(&WorldConfig::with_size(128, 128).with_seed(seed).with_preset(preset));
            assert_eq!(dominant_land_biome(&world), biome, "{preset:?} seed {seed}: {:?}", biome_counts(&world));
        }
    }
    let n = 128.0 * 128.0;
    let ocean = World::generate(&WorldConfig::with_size(128, 128).with_preset(BiomePreset::Ocean));
    let ocean_fraction = ocean.surface().as_slice().iter().filter(|s| **s == SurfaceKind::Ocean).count() as f64 / n;
    assert!(ocean_fraction > 0.6, "ocean preset: {ocean_fraction}");
    let lakes = World::generate(&WorldConfig::with_size(128, 128).with_preset(BiomePreset::Lakes));
    let lake_fraction = lakes.surface().as_slice().iter().filter(|s| **s == SurfaceKind::Lake).count() as f64 / n;
    let default = World::generate(&WorldConfig::with_size(128, 128));
    let default_lakes = default.surface().as_slice().iter().filter(|s| **s == SurfaceKind::Lake).count() as f64 / n;
    assert!(lake_fraction > 0.05 && lake_fraction > 2.0 * default_lakes, "lakes {lake_fraction} vs {default_lakes}");
    let island = World::generate(&WorldConfig::with_size(128, 128).with_preset(BiomePreset::Island));
    let land = island.surface().as_slice().iter().filter(|s| **s != SurfaceKind::Ocean).count();
    assert!(land > 1000, "island preset needs land");
    let warm = World::generate(&WorldConfig::with_size(128, 128).with_seed(2).with_preset(BiomePreset::Ocean));
    assert!(warm.biome().as_slice().contains(&Biome::Reef), "warm shallow seas grow reefs");
}

#[test]
fn lakes_hold_water_and_temperature_follows_the_lapse_rate() {
    let world = World::generate(&WorldConfig { lake_tendency: 0.8, ..WorldConfig::with_size(128, 128) });
    let lakes: Vec<usize> = (0..128 * 128).filter(|&i| world.surface().as_slice()[i] == SurfaceKind::Lake).collect();
    assert!(!lakes.is_empty());
    assert!(lakes.iter().all(|&i| world.water_depth().as_slice()[i] > 0.0));
    // Annual mean temperature of every cell follows
    // T = T_base + G (y/(H−1) − ½) − Γ max(z_surface, 0)/1000, with the
    // water surface for lakes and sea level for the ocean.
    let cfg = world.config();
    let t = world.climate().mean_temperature.as_slice();
    let z = world.elevation().as_slice();
    let mut highest_land: Option<usize> = None;
    for i in 0..128 * 128 {
        let surface_m = match world.surface().as_slice()[i] {
            SurfaceKind::Ocean => 0.0,
            SurfaceKind::Lake => z[i] + world.water_depth().as_slice()[i],
            SurfaceKind::Land | SurfaceKind::River => z[i],
        };
        let expected = annual_mean_temperature(cfg, i / 128, surface_m);
        assert!((t[i] - expected).abs() < 1e-4, "cell {i}: {} vs {expected}", t[i]);
        if world.surface().as_slice()[i] == SurfaceKind::Land && highest_land.is_none_or(|h| z[i] > z[h]) {
            highest_land = Some(i);
        }
    }
    // The highest summit is colder than sea level on its row by Γ z / 1000.
    let peak = highest_land.expect("land");
    assert!(z[peak] > 100.0);
    let sea_level_t = annual_mean_temperature(cfg, peak / 128, 0.0);
    assert!((sea_level_t - t[peak] - cfg.lapse_rate_c_per_km * z[peak] / 1000.0).abs() < 1e-3);
}

#[test]
fn heightmaps_and_configs_are_validated() {
    let cfg = WorldConfig::with_size(32, 32);
    let err = World::from_heightmap(&cfg, Grid2::new(16, 32, 0.0)).unwrap_err();
    assert!(matches!(err, WorldError::HeightmapSize { found_width: 16, .. }));
    let mut bad = Grid2::new(32, 32, 10.0);
    bad.set(3, 4, f32::NAN);
    assert!(matches!(World::from_heightmap(&cfg, bad), Err(WorldError::InvalidArgument { name: "elevation", .. })));
    let invalid = WorldConfig { relief: 3.0, ..WorldConfig::with_size(32, 32) };
    assert!(matches!(World::try_generate(&invalid), Err(WorldError::InvalidConfig { field: "relief", .. })));
    // The infallible constructor clamps instead.
    let clamped = World::generate(&invalid);
    assert_eq!(clamped.config().relief, 1.0);
    // A user heightmap: a single hill in a sea.
    let hill = Grid2::from_fn(32, 32, |x, y| {
        let d = ((x as f32 - 16.0).powi(2) + (y as f32 - 16.0).powi(2)).sqrt();
        300.0 - 30.0 * d
    });
    let world = World::from_heightmap(&cfg, hill).expect("valid heightmap");
    assert_eq!(world.surface().get(0, 0), &SurfaceKind::Ocean);
    assert_eq!(world.surface().get(16, 16), &SurfaceKind::Land);
}
