//! Configuration of an ecosystem run (the `config` section of a scenario).

use physics_world::WorldConfig;
use serde::{Deserialize, Serialize};
use simulation_core::ParamSet;

use crate::disease::Pathogen;
use crate::error::EcosystemError;
use crate::schedule::ScheduledEvent;
use crate::species::{Representation, SpeciesCatalog};

/// How the terrain is made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    /// Procedural terrain from the world configuration (noise, rivers, lakes).
    #[default]
    Generated,
    /// A flat plain of land at a fixed elevation (classic agent-based models).
    Flat {
        /// Elevation of every cell (m above sea level).
        elevation_m: f32,
    },
    /// A plain with a round basin in the middle that the world fills with a
    /// freshwater lake (with a lake tendency ≥ 0.5 and a lake of at most a
    /// quarter of the map): deepest `depth_m` at the centre,
    /// `depth (1 − (d/r)²)` towards the shore. The plain rises 2 cm per
    /// cell from the map edge so that it drains outwards.
    Basin {
        /// Elevation of the plain (m above sea level).
        elevation_m: f32,
        /// Depth of the basin at its centre (m).
        depth_m: f32,
        /// Radius of the basin (cells).
        radius_cells: f32,
    },
    /// A coast: the west of the map is sea (`sea_fraction` of the width,
    /// `sea_depth_m` deep offshore, shoaling to a beach), the east a flat
    /// plain.
    Coast {
        /// Elevation of the plain (m above sea level).
        elevation_m: f32,
        /// Depth of the open sea (m).
        sea_depth_m: f32,
        /// Fraction of the width that is sea, in `(0, 1)`.
        sea_fraction: f32,
    },
}

impl Terrain {
    /// The heightmap of the simple terrains (`None` for generated terrain).
    pub(crate) fn heightmap(&self, width: usize, height: usize) -> Option<simulation_core::Grid2<f32>> {
        match *self {
            Self::Generated => None,
            Self::Flat { elevation_m } => Some(simulation_core::Grid2::new(width, height, elevation_m)),
            Self::Basin { elevation_m, depth_m, radius_cells } => {
                let c = (width as f32 / 2.0, height as f32 / 2.0);
                Some(simulation_core::Grid2::from_fn(width, height, |x, y| {
                    let d = ((x as f32 + 0.5 - c.0).powi(2) + (y as f32 + 0.5 - c.1).powi(2)).sqrt();
                    // The plain rises 2 cm per cell away from the map edge (a 0.1 %
                    // slope at 20 m cells) so that it drains outwards and only the
                    // basin holds water.
                    let edge = x.min(y).min(width - 1 - x).min(height - 1 - y) as f32;
                    elevation_m + 0.02 * edge - depth_m * (1.0 - (d / radius_cells).powi(2)).max(0.0)
                }))
            }
            Self::Coast { elevation_m, sea_depth_m, sea_fraction } => {
                let shore = sea_fraction * width as f32;
                Some(simulation_core::Grid2::from_fn(width, height, |x, _| {
                    let from_shore = x as f32 + 0.5 - shore;
                    if from_shore >= 0.0 {
                        elevation_m.min(0.5 + from_shore)
                    } else {
                        // Shoaling over 10 cells to the open-sea depth.
                        -(sea_depth_m * (-from_shore / 10.0).min(1.0)).max(0.5)
                    }
                }))
            }
        }
    }
}

/// Which rules drive the animals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Rules {
    /// The full ecological model (utility AI, physiology, vegetation, fungi,
    /// nutrient cycle, environment coupling).
    #[default]
    Ecological,
    /// The rules of NetLogo's Wolf Sheep Predation (Wilensky 1997),
    /// re-implemented: correlated random walk, unit energy costs, fixed gains,
    /// fixed reproduction probabilities, countdown grass. Written `netlogo((...))`.
    #[serde(rename = "netlogo")]
    NetLogo(NetLogoConfig),
}

/// Parameters of the NetLogo Wolf Sheep Predation variant (defaults are the
/// NetLogo slider defaults, research note §1.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NetLogoConfig {
    /// Species playing the sheep.
    pub sheep_species: String,
    /// Species playing the wolves.
    pub wolf_species: String,
    /// Initial number of sheep.
    pub initial_sheep: u32,
    /// Initial number of wolves.
    pub initial_wolves: u32,
    /// Energy per grass patch eaten.
    pub sheep_gain_from_food: f32,
    /// Energy per sheep eaten.
    pub wolf_gain_from_food: f32,
    /// Sheep reproduction probability per tick (%).
    pub sheep_reproduce_percent: f32,
    /// Wolf reproduction probability per tick (%).
    pub wolf_reproduce_percent: f32,
    /// Grass version (`false`: sheep never gain or lose energy).
    pub grass: bool,
    /// Ticks a grazed patch needs to regrow.
    pub grass_regrowth_time: u32,
    /// Stop when wolves are extinct and sheep exceed this number.
    pub max_sheep: u32,
}

impl Default for NetLogoConfig {
    fn default() -> Self {
        Self {
            sheep_species: "sheep".into(),
            wolf_species: "wolf".into(),
            initial_sheep: 100,
            initial_wolves: 50,
            sheep_gain_from_food: 4.0,
            wolf_gain_from_food: 20.0,
            sheep_reproduce_percent: 4.0,
            wolf_reproduce_percent: 5.0,
            grass: true,
            grass_regrowth_time: 30,
            max_sheep: 30_000,
        }
    }
}

impl NetLogoConfig {
    /// Validates the values (ranges of the NetLogo sliders, widened).
    pub fn validate(&self, catalog: &SpeciesCatalog) -> Result<(), EcosystemError> {
        catalog.animal_index(&self.sheep_species)?;
        catalog.animal_index(&self.wolf_species)?;
        if self.sheep_species == self.wolf_species {
            return Err(EcosystemError::config("rules.net_logo", "sheep and wolves must be different species"));
        }
        let check = |name: &str, v: f32, max: f32| {
            if !(0.0..=max).contains(&v) {
                return Err(EcosystemError::config(
                    &format!("rules.net_logo.{name}"),
                    format!("{v} is outside [0, {max}]"),
                ));
            }
            Ok(())
        };
        check("sheep_gain_from_food", self.sheep_gain_from_food, 1000.0)?;
        check("wolf_gain_from_food", self.wolf_gain_from_food, 1000.0)?;
        check("sheep_reproduce_percent", self.sheep_reproduce_percent, 100.0)?;
        check("wolf_reproduce_percent", self.wolf_reproduce_percent, 100.0)?;
        if self.initial_sheep > 100_000 || self.initial_wolves > 100_000 {
            return Err(EcosystemError::config("rules.net_logo", "initial populations are limited to 100000"));
        }
        if self.grass_regrowth_time > 100_000 {
            return Err(EcosystemError::config("rules.net_logo.grass_regrowth_time", "must be at most 100000"));
        }
        Ok(())
    }
}

/// Initial vegetation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VegetationSetup {
    /// Initial biomass as a fraction of the carrying capacity (with ±30 % noise).
    pub initial_fraction: f32,
    /// Initial seed-bank presence where the layer can grow.
    pub seed_bank: f32,
}

impl Default for VegetationSetup {
    fn default() -> Self {
        Self { initial_fraction: 0.6, seed_bank: 1.0 }
    }
}

/// Initial trees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TreeSetup {
    /// Trees per hectare in suitable cells (forest biomes get the full
    /// density, grassland a fifth).
    pub density_per_ha: f32,
    /// Tree species to plant (empty = every species suited to the cell).
    pub species: Vec<String>,
}

impl Default for TreeSetup {
    fn default() -> Self {
        Self { density_per_ha: 60.0, species: Vec::new() }
    }
}

/// Initial dead matter and fungi.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SoilSetup {
    /// Litter on land (kg/m²).
    pub litter_kg_m2: f32,
    /// Dead wood on land (kg/m²).
    pub deadwood_kg_m2: f32,
    /// Saprotrophic mycelium on land (kg/m²).
    pub mycelium_kg_m2: f32,
    /// Mycorrhizal biomass where hosts grow (kg/m²).
    pub mycorrhiza_kg_m2: f32,
    /// Dissolved mineral nutrients added to every water cell (lakes,
    /// rivers, the sea; g/m²). The physical world gives water no soil
    /// fertility; this stock feeds algae and water plants, and is recycled
    /// by decomposition of what dies in the water.
    pub water_nutrients_g_m2: f32,
}

impl Default for SoilSetup {
    fn default() -> Self {
        Self {
            litter_kg_m2: 0.1,
            deadwood_kg_m2: 0.05,
            mycelium_kg_m2: 0.01,
            mycorrhiza_kg_m2: 0.002,
            water_nutrients_g_m2: 2.0,
        }
    }
}

/// Initial animals of one species.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimalSetup {
    /// Species id.
    pub species: String,
    /// Number of animals; for density-field species the number of
    /// individuals spread over the habitat of the region, for colony
    /// species the number of colonies.
    pub count: u32,
    /// Optional placement rectangle `(x0, y0, x1, y1)` in cells.
    #[serde(default)]
    pub region: Option<(f32, f32, f32, f32)>,
    /// Number of social groups (herds, packs, prides) the founders form;
    /// `None` draws group sizes from the species' `group_size`.
    #[serde(default)]
    pub groups: Option<u32>,
}

/// Configuration of an ecosystem run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EcosystemConfig {
    /// Abiotic world (size, cell size, climate...). Its `seed` is replaced
    /// by the scenario seed.
    pub world: WorldConfig,
    /// Terrain source.
    pub terrain: Terrain,
    /// Rule set.
    pub rules: Rules,
    /// Initial vegetation.
    pub vegetation: VegetationSetup,
    /// Initial trees.
    pub trees: TreeSetup,
    /// Initial litter, dead wood and fungi.
    pub soil: SoilSetup,
    /// Initial animals.
    pub animals: Vec<AnimalSetup>,
    /// Parameter overrides (keys of [`crate::params::param_specs`]).
    pub params: ParamSet,
    /// Pathogens that can infect the animals (outbreaks are scheduled events).
    pub pathogens: Vec<Pathogen>,
    /// Scheduled interventions, in order of their day.
    pub events: Vec<ScheduledEvent>,
}

impl Default for EcosystemConfig {
    fn default() -> Self {
        Self {
            world: WorldConfig {
                width: 96,
                height: 96,
                cell_size_m: 20.0,
                sea_level: 0.0,
                relief: 0.0,
                roughness: 0.4,
                river_count: 1,
                lake_tendency: 0.5,
                rainfall_mm: 650.0,
                base_temperature_c: 13.0,
                latitude_gradient: 2.0,
                seasonality: 9.0,
                ..WorldConfig::default()
            },
            terrain: Terrain::Generated,
            rules: Rules::Ecological,
            vegetation: VegetationSetup::default(),
            trees: TreeSetup::default(),
            soil: SoilSetup::default(),
            animals: Vec::new(),
            params: ParamSet::new(),
            pathogens: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl EcosystemConfig {
    /// Validates the configuration against a species catalogue.
    pub fn validate(&self, catalog: &SpeciesCatalog) -> Result<(), EcosystemError> {
        self.world.effective().validate()?;
        let check = |field: &str, v: f32, lo: f32, hi: f32| {
            if v.is_finite() && (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(EcosystemError::config(&format!("terrain.{field}"), format!("{v} is outside [{lo}, {hi}]")))
            }
        };
        match self.terrain {
            Terrain::Generated => {}
            Terrain::Flat { elevation_m } => check("flat.elevation_m", elevation_m, 0.5, 5000.0)?,
            Terrain::Basin { elevation_m, depth_m, radius_cells } => {
                check("basin.elevation_m", elevation_m, 0.5, 5000.0)?;
                check("basin.depth_m", depth_m, 0.6, 200.0)?;
                check("basin.radius_cells", radius_cells, 1.0, 4096.0)?;
            }
            Terrain::Coast { elevation_m, sea_depth_m, sea_fraction } => {
                check("coast.elevation_m", elevation_m, 0.5, 5000.0)?;
                check("coast.sea_depth_m", sea_depth_m, 0.5, 5000.0)?;
                check("coast.sea_fraction", sea_fraction, 0.01, 0.99)?;
            }
        }
        if let Rules::NetLogo(n) = &self.rules {
            n.validate(catalog)?;
        }
        let v = &self.vegetation;
        if !(0.0..=1.0).contains(&v.initial_fraction) || !(0.0..=1.0).contains(&v.seed_bank) {
            return Err(EcosystemError::config("vegetation", "initial_fraction and seed_bank must be in [0, 1]"));
        }
        if !(self.trees.density_per_ha.is_finite() && (0.0..=2000.0).contains(&self.trees.density_per_ha)) {
            return Err(EcosystemError::config("trees.density_per_ha", "must be in [0, 2000]"));
        }
        for id in &self.trees.species {
            catalog.tree_index(id)?;
        }
        let s = &self.soil;
        for (name, v) in [
            ("litter_kg_m2", s.litter_kg_m2),
            ("deadwood_kg_m2", s.deadwood_kg_m2),
            ("mycelium_kg_m2", s.mycelium_kg_m2),
            ("mycorrhiza_kg_m2", s.mycorrhiza_kg_m2),
            ("water_nutrients_g_m2", s.water_nutrients_g_m2),
        ] {
            if !(v.is_finite() && (0.0..=50.0).contains(&v)) {
                return Err(EcosystemError::config(&format!("soil.{name}"), format!("{v} is outside [0, 50]")));
            }
        }
        let mut total = 0u64;
        for a in &self.animals {
            let index = catalog.animal_index(&a.species)?;
            if let Rules::NetLogo(_) = self.rules
                && !catalog.is_individual(index)
            {
                return Err(EcosystemError::config(
                    &format!("animals.{}", a.species),
                    "the NetLogo rule set only runs individual animals",
                ));
            }
            // Field individuals are not agents: they do not count towards the limit.
            if matches!(catalog.animals()[usize::from(index)].representation, Representation::Field(_)) {
                continue;
            }
            total += u64::from(a.count);
            if a.groups == Some(0) || a.groups.is_some_and(|g| g > a.count.max(1)) {
                return Err(EcosystemError::config(
                    &format!("animals.{}.groups", a.species),
                    "needs between 1 and `count` groups",
                ));
            }
            if let Some((x0, y0, x1, y1)) = a.region {
                let ok = [x0, y0, x1, y1].iter().all(|v| v.is_finite()) && x0 < x1 && y0 < y1;
                if !ok {
                    return Err(EcosystemError::config(
                        &format!("animals.{}.region", a.species),
                        "needs x0 < x1 and y0 < y1",
                    ));
                }
            }
        }
        if total > 200_000 {
            return Err(EcosystemError::config("animals", format!("{total} initial animals exceed 200000")));
        }
        for (i, p) in self.pathogens.iter().enumerate() {
            p.validate(catalog)?;
            if self.pathogens[..i].iter().any(|o| o.id == p.id) {
                return Err(EcosystemError::config("pathogens", format!("pathogen `{}` is defined twice", p.id)));
            }
        }
        if self.pathogens.len() > usize::from(u16::MAX) {
            return Err(EcosystemError::config("pathogens", "too many pathogens"));
        }
        crate::schedule::validate(&self.events, catalog, &self.pathogens, &self.rules)
    }
}
