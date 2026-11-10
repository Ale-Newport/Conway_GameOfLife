//! Parameters of the ecosystem model with their apply policies.
//!
//! * **World** (`width`, `height`, `cell_size_m`, `days_per_season`,
//!   `seed`) — `RequiresReset`: they shape the generated world and calendar.
//! * **Climate** — `NextTick`: written to the world's runtime configuration
//!   and used by its next step.
//! * **Vegetation, fungi, animals, nutrients** — `NextTick` for rates
//!   (queued and applied at the start of the next tick), `Immediate` for
//!   population caps.
//! * **NetLogo** (only with the NetLogo rule set) — gains, reproduction and
//!   regrowth `NextTick`; initial populations and the grass switch
//!   `RequiresReset`; `max_sheep` `Immediate`.

use serde::{Deserialize, Serialize};
use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError};

use crate::config::{EcosystemConfig, Rules};

/// A parameter value in a form every serde format can read back
/// ([`ParamValue`] is untagged, which binary formats cannot decode).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum StoredValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Color([f32; 4]),
}

impl From<&ParamValue> for StoredValue {
    fn from(v: &ParamValue) -> Self {
        match v {
            ParamValue::Bool(b) => Self::Bool(*b),
            ParamValue::Int(i) => Self::Int(*i),
            ParamValue::Float(f) => Self::Float(*f),
            ParamValue::Text(t) => Self::Text(t.clone()),
            ParamValue::Color(c) => Self::Color(*c),
        }
    }
}

impl From<&StoredValue> for ParamValue {
    fn from(v: &StoredValue) -> Self {
        match v {
            StoredValue::Bool(b) => Self::Bool(*b),
            StoredValue::Int(i) => Self::Int(*i),
            StoredValue::Float(f) => Self::Float(*f),
            StoredValue::Text(t) => Self::Text(t.clone()),
            StoredValue::Color(c) => Self::Color(*c),
        }
    }
}

/// Runtime parameters of the ecological rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeParams {
    /// Multiplier on vegetation-layer growth.
    pub plant_growth: f32,
    /// Multiplier on tree growth.
    pub tree_growth: f32,
    /// Largest number of living trunks per cell.
    pub max_trees_per_cell: u16,
    /// Largest number of trees on the map (seeding stops above it).
    pub max_trees: u32,
    /// Multiplier on decomposition rates.
    pub decomposition: f32,
    /// Multiplier on the mycorrhizal uptake boost.
    pub mycorrhizal_boost: f32,
    /// Seconds of continuous locomotion represented by one tick.
    pub motion_seconds: f32,
    /// Per-gene mutation probability of offspring.
    pub mutation_rate: f32,
    /// Multiplier on the odds of a successful attack.
    pub predation_efficiency: f32,
    /// Largest population of one species (births beyond it are suppressed).
    pub max_population_per_species: u32,
    /// Largest number of animals in total.
    pub max_population_total: u32,
    /// Atmospheric nutrient deposition onto land (g/m² per simulated year).
    pub deposition_g_m2_year: f32,
    /// Fraction of the nutrients of burned biomass lost to the atmosphere.
    pub volatilisation_fraction: f32,
    /// Whether nutrients leached by percolating water leave the ecosystem
    /// (`true`) or are retained in the root zone (`false`).
    pub leaching_loss: bool,
    /// Master switch of the external nutrient fluxes. `false` closes the
    /// budget: no deposition, no volatilisation (all burned nutrients return
    /// as ash) and leached nutrients are retained.
    pub external_fluxes: bool,
    /// Half-life of ant trail pheromone (hours).
    pub pheromone_half_life_h: f32,
    /// Fraction of trail pheromone exchanged with the 4 neighbouring cells per tick.
    pub pheromone_diffusion: f32,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self {
            plant_growth: 1.0,
            tree_growth: 1.0,
            max_trees_per_cell: 4,
            max_trees: 100_000,
            decomposition: 1.0,
            mycorrhizal_boost: 1.0,
            motion_seconds: 5.0,
            mutation_rate: 0.1,
            predation_efficiency: 1.0,
            max_population_per_species: 5_000,
            max_population_total: 20_000,
            deposition_g_m2_year: 0.5,
            volatilisation_fraction: 0.5,
            leaching_loss: true,
            external_fluxes: true,
            pheromone_half_life_h: 1.0,
            pheromone_diffusion: 0.05,
        }
    }
}

impl RuntimeParams {
    /// Effective deposition (0 when external fluxes are disabled).
    pub fn deposition(&self) -> f32 {
        if self.external_fluxes { self.deposition_g_m2_year } else { 0.0 }
    }

    /// Effective volatilisation (0 when external fluxes are disabled: all
    /// nutrients of burned biomass return as ash).
    pub fn volatilisation(&self) -> f32 {
        if self.external_fluxes { self.volatilisation_fraction } else { 0.0 }
    }

    /// Whether leached nutrients are returned to the soil of the cells that
    /// lost them (closed budget or `leaching_loss` off).
    pub fn leaching_retained(&self) -> bool {
        !(self.external_fluxes && self.leaching_loss)
    }
}

fn group(specs: Vec<ParamSpec>, name: &str, policy: ApplyPolicy) -> Vec<ParamSpec> {
    specs.into_iter().map(|s| s.group(name).apply(policy)).collect()
}

/// Parameter specs for a rule set.
pub fn param_specs(rules: &Rules) -> Vec<ParamSpec> {
    let d = RuntimeParams::default();
    let w = EcosystemConfig::default().world;
    let mut specs = group(
        vec![
            ParamSpec::int("width", "Width", 16, 1024, w.width as i64).describe("World width in cells."),
            ParamSpec::int("height", "Height", 16, 1024, w.height as i64).describe("World height in cells."),
            ParamSpec::float("cell_size_m", "Cell size", 1.0, 100.0, f64::from(w.cell_size_m))
                .describe("Edge of one terrain cell in metres; speeds and perception are converted to cells with it."),
            ParamSpec::int("days_per_season", "Days per season", 1, 3650, i64::from(w.days_per_season))
                .describe("Length of a season (the calendar of the physics world is fixed for a run)."),
            ParamSpec::int("seed", "Seed", 0, i64::MAX, 1).describe("Seed of the world and of every random stream."),
        ],
        "World",
        ApplyPolicy::RequiresReset,
    );
    specs.extend(group(
        vec![
            ParamSpec::float("rainfall_mm", "Rainfall", 0.0, 8000.0, f64::from(w.rainfall_mm))
                .describe("Mean annual precipitation over land (mm)."),
            ParamSpec::float("base_temperature_c", "Base temperature", -40.0, 45.0, f64::from(w.base_temperature_c))
                .describe("Annual mean sea-level temperature at the middle row (°C)."),
            ParamSpec::float("seasonality", "Seasonality", 0.0, 40.0, f64::from(w.seasonality))
                .describe("Seasonal temperature amplitude (°C)."),
            ParamSpec::float("wind_speed", "Wind speed", 0.0, 60.0, f64::from(w.wind_speed))
                .describe("Prevailing wind (m/s)."),
            ParamSpec::float("fire_spread", "Fire spread", 0.0, 5.0, f64::from(w.fire_spread))
                .describe("Multiplier on the fire-spread probability."),
            ParamSpec::float("lightning_rate", "Lightning rate", 0.0, 10.0, f64::from(w.lightning_rate))
                .describe("Cloud-to-ground strikes per km² per hour of storm."),
        ],
        "Climate",
        ApplyPolicy::NextTick,
    ));
    specs.extend(group(
        vec![
            ParamSpec::float("plant_growth", "Plant growth", 0.0, 5.0, f64::from(d.plant_growth))
                .describe("Multiplier on the growth rate of every vegetation layer."),
            ParamSpec::float("tree_growth", "Tree growth", 0.0, 5.0, f64::from(d.tree_growth))
                .describe("Multiplier on tree height growth."),
            ParamSpec::float("decomposition", "Decomposition", 0.0, 5.0, f64::from(d.decomposition))
                .describe("Multiplier on litter and dead-wood decay."),
            ParamSpec::float("mycorrhizal_boost", "Mycorrhizal boost", 0.0, 5.0, f64::from(d.mycorrhizal_boost))
                .describe("Multiplier on the nutrient-uptake boost mycorrhiza give their hosts."),
        ],
        "Vegetation & fungi",
        ApplyPolicy::NextTick,
    ));
    specs.extend(group(
        vec![
            ParamSpec::int("max_trees_per_cell", "Trees per cell", 1, 64, i64::from(d.max_trees_per_cell))
                .describe("Largest number of living trunks in one cell."),
            ParamSpec::int("max_trees", "Tree cap", 0, 1_000_000, i64::from(d.max_trees))
                .describe("Seeding stops while the map holds this many trees (performance safeguard)."),
        ],
        "Vegetation & fungi",
        ApplyPolicy::Immediate,
    ));
    specs.extend(group(
        vec![
            ParamSpec::float("motion_seconds", "Motion per tick", 1.0, 120.0, f64::from(d.motion_seconds))
                .describe("Seconds of continuous locomotion one tick represents (speed in cells per tick = m/s × this / cell size)."),
            ParamSpec::float("mutation_rate", "Mutation rate", 0.0, 1.0, f64::from(d.mutation_rate))
                .describe("Probability that each gene of a newborn mutates."),
            ParamSpec::float("predation_efficiency", "Predation efficiency", 0.05, 20.0, f64::from(d.predation_efficiency))
                .describe("Multiplier on the odds that an attack kills.")
                .logarithmic(),
        ],
        "Animals",
        ApplyPolicy::NextTick,
    ));
    specs.extend(group(
        vec![
            ParamSpec::int(
                "max_population_per_species",
                "Species cap",
                1,
                1_000_000,
                i64::from(d.max_population_per_species),
            )
            .describe("Births are suppressed while a species has this many animals (performance safeguard)."),
            ParamSpec::int("max_population_total", "Animal cap", 1, 1_000_000, i64::from(d.max_population_total))
                .describe("Births are suppressed while the map holds this many animals."),
        ],
        "Animals",
        ApplyPolicy::Immediate,
    ));
    specs.extend(group(
        vec![
            ParamSpec::float("deposition_g_m2_year", "Deposition", 0.0, 100.0, f64::from(d.deposition_g_m2_year))
                .describe("Atmospheric nutrient input onto land (g/m² per simulated year)."),
            ParamSpec::float(
                "volatilisation_fraction",
                "Volatilisation",
                0.0,
                1.0,
                f64::from(d.volatilisation_fraction),
            )
            .describe("Fraction of the nutrients of burned biomass lost to the air (the rest returns as ash)."),
            ParamSpec::boolean("leaching_loss", "Leaching losses", d.leaching_loss).describe(
                "Nutrients leached by percolating water leave the ecosystem; when off they are retained in the root zone.",
            ),
            ParamSpec::boolean("external_fluxes", "External fluxes", d.external_fluxes)
                .describe("Disable to close the nutrient budget (no deposition, no volatilisation, no leaching losses)."),
        ],
        "Nutrients",
        ApplyPolicy::NextTick,
    ));
    specs.extend(group(
        vec![
            ParamSpec::float(
                "pheromone_half_life_h",
                "Pheromone half-life",
                0.05,
                100.0,
                f64::from(d.pheromone_half_life_h),
            )
            .describe("Half-life of ant trail pheromone (hours); short-lived trails track the current food sources.")
            .logarithmic(),
            ParamSpec::float("pheromone_diffusion", "Pheromone diffusion", 0.0, 0.25, f64::from(d.pheromone_diffusion))
                .describe("Fraction of trail pheromone exchanged with the 4 neighbouring cells per tick."),
        ],
        "Colonies",
        ApplyPolicy::NextTick,
    ));
    if let Rules::NetLogo(n) = rules {
        specs.extend(group(
            vec![
                ParamSpec::float(
                    "sheep_gain_from_food",
                    "Sheep gain from food",
                    0.0,
                    1000.0,
                    f64::from(n.sheep_gain_from_food),
                ),
                ParamSpec::float(
                    "wolf_gain_from_food",
                    "Wolf gain from food",
                    0.0,
                    1000.0,
                    f64::from(n.wolf_gain_from_food),
                ),
                ParamSpec::float(
                    "sheep_reproduce_percent",
                    "Sheep reproduce %",
                    0.0,
                    100.0,
                    f64::from(n.sheep_reproduce_percent),
                ),
                ParamSpec::float(
                    "wolf_reproduce_percent",
                    "Wolf reproduce %",
                    0.0,
                    100.0,
                    f64::from(n.wolf_reproduce_percent),
                ),
                ParamSpec::int(
                    "grass_regrowth_time",
                    "Grass regrowth time",
                    0,
                    100_000,
                    i64::from(n.grass_regrowth_time),
                ),
            ],
            "NetLogo",
            ApplyPolicy::NextTick,
        ));
        specs.extend(group(
            vec![ParamSpec::int("max_sheep", "Max sheep", 0, 10_000_000, i64::from(n.max_sheep))],
            "NetLogo",
            ApplyPolicy::Immediate,
        ));
        specs.extend(group(
            vec![
                ParamSpec::int("initial_sheep", "Initial sheep", 0, 100_000, i64::from(n.initial_sheep)),
                ParamSpec::int("initial_wolves", "Initial wolves", 0, 100_000, i64::from(n.initial_wolves)),
                ParamSpec::boolean("grass", "Grass version", n.grass),
            ],
            "NetLogo",
            ApplyPolicy::RequiresReset,
        ));
    }
    specs
}

fn float(v: f32) -> ParamValue {
    // Shortest decimal that round-trips the stored f32 (reports what was set).
    ParamValue::Float(v.to_string().parse::<f64>().unwrap_or(f64::from(v)))
}

/// Current values of every parameter.
/// The seed is the one the next reset uses (`config.world.seed`).
pub(crate) fn current(config: &EcosystemConfig, params: &RuntimeParams) -> ParamSet {
    let w = &config.world;
    let mut set = ParamSet::new()
        .with("width", ParamValue::Int(w.width as i64))
        .with("height", ParamValue::Int(w.height as i64))
        .with("cell_size_m", float(w.cell_size_m))
        .with("days_per_season", ParamValue::Int(i64::from(w.days_per_season)))
        .with("seed", ParamValue::Int(i64::try_from(w.seed).unwrap_or(i64::MAX)))
        .with("rainfall_mm", float(w.rainfall_mm))
        .with("base_temperature_c", float(w.base_temperature_c))
        .with("seasonality", float(w.seasonality))
        .with("wind_speed", float(w.wind_speed))
        .with("fire_spread", float(w.fire_spread))
        .with("lightning_rate", float(w.lightning_rate))
        .with("plant_growth", float(params.plant_growth))
        .with("tree_growth", float(params.tree_growth))
        .with("decomposition", float(params.decomposition))
        .with("mycorrhizal_boost", float(params.mycorrhizal_boost))
        .with("max_trees_per_cell", ParamValue::Int(i64::from(params.max_trees_per_cell)))
        .with("max_trees", ParamValue::Int(i64::from(params.max_trees)))
        .with("motion_seconds", float(params.motion_seconds))
        .with("mutation_rate", float(params.mutation_rate))
        .with("predation_efficiency", float(params.predation_efficiency))
        .with("max_population_per_species", ParamValue::Int(i64::from(params.max_population_per_species)))
        .with("max_population_total", ParamValue::Int(i64::from(params.max_population_total)))
        .with("deposition_g_m2_year", float(params.deposition_g_m2_year))
        .with("volatilisation_fraction", float(params.volatilisation_fraction))
        .with("leaching_loss", ParamValue::Bool(params.leaching_loss))
        .with("external_fluxes", ParamValue::Bool(params.external_fluxes))
        .with("pheromone_half_life_h", float(params.pheromone_half_life_h))
        .with("pheromone_diffusion", float(params.pheromone_diffusion));
    if let Rules::NetLogo(n) = &config.rules {
        set = set
            .with("sheep_gain_from_food", float(n.sheep_gain_from_food))
            .with("wolf_gain_from_food", float(n.wolf_gain_from_food))
            .with("sheep_reproduce_percent", float(n.sheep_reproduce_percent))
            .with("wolf_reproduce_percent", float(n.wolf_reproduce_percent))
            .with("grass_regrowth_time", ParamValue::Int(i64::from(n.grass_regrowth_time)))
            .with("max_sheep", ParamValue::Int(i64::from(n.max_sheep)))
            .with("initial_sheep", ParamValue::Int(i64::from(n.initial_sheep)))
            .with("initial_wolves", ParamValue::Int(i64::from(n.initial_wolves)))
            .with("grass", ParamValue::Bool(n.grass));
    }
    set
}

/// Writes a validated value into the configuration / runtime parameters.
/// World, seed and climate keys go to `config.world` (the seed and the world
/// shape take effect at the next reset); NetLogo keys to the rules.
pub(crate) fn write(
    key: &str,
    value: &ParamValue,
    config: &mut EcosystemConfig,
    params: &mut RuntimeParams,
) -> Result<(), SimError> {
    let f = || value.as_f64().map(|v| v as f32).ok_or_else(|| SimError::invalid_param(key, "expected a number"));
    let i = || value.as_i64().ok_or_else(|| SimError::invalid_param(key, "expected an integer"));
    let b = || value.as_bool().ok_or_else(|| SimError::invalid_param(key, "expected true or false"));
    let int = |v: i64| u32::try_from(v).map_err(|_| SimError::invalid_param(key, format!("{v} is out of range")));
    // Editing a field that a biome preset controls first turns the preset
    // into explicit values, so the edit is not overwritten at the next reset.
    const PRESET_FIELDS: [&str; 3] = ["rainfall_mm", "base_temperature_c", "seasonality"];
    if config.world.biome_override.is_some() && PRESET_FIELDS.contains(&key) {
        config.world = config.world.effective();
        config.world.biome_override = None;
    }
    let w = &mut config.world;
    match key {
        "width" => w.width = i()? as usize,
        "height" => w.height = i()? as usize,
        "cell_size_m" => w.cell_size_m = f()?,
        "days_per_season" => w.days_per_season = int(i()?)?,
        "seed" => {
            w.seed = u64::try_from(i()?).map_err(|_| SimError::invalid_param(key, "the seed must not be negative"))?
        }
        "rainfall_mm" => w.rainfall_mm = f()?,
        "base_temperature_c" => w.base_temperature_c = f()?,
        "seasonality" => w.seasonality = f()?,
        "wind_speed" => w.wind_speed = f()?,
        "fire_spread" => w.fire_spread = f()?,
        "lightning_rate" => w.lightning_rate = f()?,
        "plant_growth" => params.plant_growth = f()?,
        "tree_growth" => params.tree_growth = f()?,
        "decomposition" => params.decomposition = f()?,
        "mycorrhizal_boost" => params.mycorrhizal_boost = f()?,
        "max_trees_per_cell" => params.max_trees_per_cell = int(i()?)?.min(u32::from(u16::MAX)) as u16,
        "max_trees" => params.max_trees = int(i()?)?,
        "motion_seconds" => params.motion_seconds = f()?,
        "mutation_rate" => params.mutation_rate = f()?,
        "predation_efficiency" => params.predation_efficiency = f()?,
        "max_population_per_species" => params.max_population_per_species = int(i()?)?,
        "max_population_total" => params.max_population_total = int(i()?)?,
        "deposition_g_m2_year" => params.deposition_g_m2_year = f()?,
        "volatilisation_fraction" => params.volatilisation_fraction = f()?,
        "leaching_loss" => params.leaching_loss = b()?,
        "external_fluxes" => params.external_fluxes = b()?,
        "pheromone_half_life_h" => params.pheromone_half_life_h = f()?,
        "pheromone_diffusion" => params.pheromone_diffusion = f()?,
        _ => {
            let Rules::NetLogo(n) = &mut config.rules else {
                return Err(SimError::UnknownParameter(key.to_string()));
            };
            match key {
                "sheep_gain_from_food" => n.sheep_gain_from_food = f()?,
                "wolf_gain_from_food" => n.wolf_gain_from_food = f()?,
                "sheep_reproduce_percent" => n.sheep_reproduce_percent = f()?,
                "wolf_reproduce_percent" => n.wolf_reproduce_percent = f()?,
                "grass_regrowth_time" => n.grass_regrowth_time = int(i()?)?,
                "max_sheep" => n.max_sheep = int(i()?)?,
                "initial_sheep" => n.initial_sheep = int(i()?)?,
                "initial_wolves" => n.initial_wolves = int(i()?)?,
                "grass" => n.grass = b()?,
                _ => return Err(SimError::UnknownParameter(key.to_string())),
            }
        }
    }
    Ok(())
}

impl crate::state::EcoState {
    /// Validates and applies a parameter change with its apply policy
    /// (shared by `Simulation::set_param` and scheduled events). Values are
    /// applied to copies first, so a rejected value changes nothing.
    pub(crate) fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let specs = param_specs(&self.config.rules);
        let spec = specs.iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(&value)?;
        const CLIMATE: [&str; 6] =
            ["rainfall_mm", "base_temperature_c", "seasonality", "wind_speed", "fire_spread", "lightning_rate"];
        let s = self;
        match spec.apply {
            ApplyPolicy::RequiresReset | ApplyPolicy::Immediate => {
                // Applied to copies first, so a rejected value changes nothing.
                let mut config = s.config.clone();
                let mut params = s.params.clone();
                write(key, &value, &mut config, &mut params)?;
                if spec.apply == ApplyPolicy::RequiresReset {
                    config.world.effective().validate().map_err(|e| SimError::invalid_param(key, e.to_string()))?;
                }
                s.config = config;
                s.params = params;
            }
            ApplyPolicy::NextTick if CLIMATE.contains(&key) => {
                // Climate goes straight into the world's runtime configuration;
                // the world uses it from its next step on.
                let mut config = s.config.clone();
                write(key, &value, &mut config, &mut s.params)?;
                let c = &config.world;
                let mut runtime = s.world.config().clone();
                runtime.rainfall_mm = c.rainfall_mm;
                runtime.base_temperature_c = c.base_temperature_c;
                runtime.seasonality = c.seasonality;
                runtime.wind_speed = c.wind_speed;
                runtime.fire_spread = c.fire_spread;
                runtime.lightning_rate = c.lightning_rate;
                s.world.set_runtime_config(&runtime).map_err(|e| SimError::invalid_param(key, e.to_string()))?;
                s.config = config;
            }
            ApplyPolicy::NextTick => s.queue_param(key, value),
        }
        Ok(spec.apply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetLogoConfig;

    #[test]
    fn current_values_validate_against_their_specs() {
        for rules in [Rules::Ecological, Rules::NetLogo(NetLogoConfig::default())] {
            let config = EcosystemConfig { rules: rules.clone(), ..EcosystemConfig::default() };
            let specs = param_specs(&rules);
            let set = current(&config, &RuntimeParams::default());
            assert_eq!(set.len(), specs.len(), "every spec has a value");
            let resolved = set.resolve(&specs).unwrap();
            assert_eq!(resolved, set);
        }
    }

    #[test]
    fn netlogo_keys_are_unknown_in_ecological_mode() {
        let mut config = EcosystemConfig::default();
        let mut params = RuntimeParams::default();
        let err = write("sheep_gain_from_food", &ParamValue::Float(3.0), &mut config, &mut params).unwrap_err();
        assert!(matches!(err, SimError::UnknownParameter(_)));
    }
}
