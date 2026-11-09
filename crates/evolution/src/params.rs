//! Parameters exposed to the user interface and the command-line tool, and
//! their mapping onto [`EvolutionConfig`] fields.
//!
//! Each parameter is one config field; [`get`] and [`set`] translate between
//! the two, and [`EvolutionConfig::validate`] reuses the specs' ranges, so a
//! range is declared exactly once.

use organism_engine::{Crossover, MutationBounds};
use simulation_core::{ApplyPolicy, ParamSpec, ParamValue};

use crate::config::{EvolutionConfig, ReproductionMode};
use crate::error::EvolutionError;

/// `(parameter key, config field path)` for every parameter, in display order.
const FIELDS: [(&str, &str); 41] = [
    ("width", "world.width"),
    ("height", "world.height"),
    ("initial_prey", "prey.initial"),
    ("initial_predators", "predators.initial"),
    ("genealogy_max_nodes", "statistics.genealogy_max_nodes"),
    ("max_ticks", "max_ticks"),
    ("food_regrowth_rate", "food.regrowth_rate"),
    ("food_capacity_scale", "food.capacity_scale"),
    ("food_energy", "food.energy"),
    ("base_temperature", "climate.base_temperature"),
    ("temperature_gradient_x", "climate.gradient_x"),
    ("temperature_gradient_y", "climate.gradient_y"),
    ("temperature_shift_rate", "climate.shift_rate"),
    ("seasonal_amplitude", "climate.seasonal_amplitude"),
    ("seasonal_period", "climate.seasonal_period"),
    ("reproduction_mode", "reproduction.mode"),
    ("crossover", "reproduction.crossover"),
    ("breeding_threshold", "reproduction.threshold"),
    ("refractory_ticks", "reproduction.refractory"),
    ("mate_distance_limit", "reproduction.mate_distance_limit"),
    ("mutation_rate", "mutation.rate"),
    ("mutation_size", "mutation.size"),
    ("mutation_bounds", "mutation.bounds"),
    ("predators_evolve", "predators.evolve"),
    ("predators_constant", "predators.constant"),
    ("capture_probability", "predators.capture_probability"),
    ("predator_assimilation", "predators.assimilation"),
    ("handling_ticks", "predators.handling_ticks"),
    ("predator_interference", "predators.interference"),
    ("basal_cost", "physiology.basal_cost"),
    ("movement_cost", "physiology.movement_cost"),
    ("speed_upkeep", "physiology.speed_upkeep"),
    ("perception_cost", "physiology.perception_cost"),
    ("tolerance_cost", "physiology.tolerance_cost"),
    ("thermal_stress_cost", "physiology.thermal_stress_cost"),
    ("background_mortality", "physiology.background_mortality"),
    ("longevity_ratio", "physiology.longevity_ratio"),
    ("histogram_bins", "statistics.histogram_bins"),
    ("allele_classes", "statistics.allele_classes"),
    ("speciation_enabled", "speciation.enabled"),
    ("speciation_generations", "speciation.generations"),
];

/// Config field path of a parameter key (the key itself if unknown).
pub fn field_path(key: &str) -> &str {
    FIELDS.iter().find(|(k, _)| *k == key).map_or(key, |(_, path)| path)
}

/// The parameter specs, with the defaults of [`EvolutionConfig::default`].
pub fn specs() -> Vec<ParamSpec> {
    use ApplyPolicy::{Immediate, RequiresReset};
    let d = EvolutionConfig::default();
    let f =
        |key: &str, label: &str, lo: f64, hi: f64, default: f32| ParamSpec::float(key, label, lo, hi, widen(default));
    let i = |key: &str, label: &str, lo: i64, hi: i64, default: u64| ParamSpec::int(key, label, lo, hi, default as i64);
    vec![
        i("width", "World width", 16, 512, d.world.width.into())
            .describe("World width in cells.")
            .group("World")
            .apply(RequiresReset),
        i("height", "World height", 16, 512, d.world.height.into())
            .describe("World height in cells.")
            .group("World")
            .apply(RequiresReset),
        i("initial_prey", "Founding prey", 0, 5000, d.prey.initial.into())
            .describe("Prey placed at the start of the experiment.")
            .group("World")
            .apply(RequiresReset),
        i("initial_predators", "Founding predators", 0, 2000, d.predators.initial.into())
            .describe("Predators placed at the start (0: none until introduced).")
            .group("World")
            .apply(RequiresReset),
        i("genealogy_max_nodes", "Genealogy node cap", 100, 1_000_000, d.statistics.genealogy_max_nodes.into())
            .describe("Recorded genealogy nodes; the oldest dead ancestors are truncated beyond it.")
            .group("World")
            .apply(RequiresReset),
        i("max_ticks", "Stop after ticks", 0, 1_000_000_000, d.max_ticks)
            .describe("The run finishes at this tick (0: only at extinction).")
            .group("World")
            .apply(Immediate),
        f("food_regrowth_rate", "Food regrowth rate", 0.0, 0.5, d.food.regrowth_rate)
            .describe("Logistic growth rate r of food per tick.")
            .group("Food"),
        f("food_capacity_scale", "Food capacity scale", 0.05, 5.0, d.food.capacity_scale)
            .describe("Multiplies every cell's carrying capacity (scarcity < 1 < abundance).")
            .group("Food"),
        f("food_energy", "Energy per food unit", 0.1, 10.0, d.food.energy)
            .describe("Energy an organism gains per unit of food eaten.")
            .group("Food"),
        f("base_temperature", "Base temperature", -20.0, 50.0, d.climate.base_temperature)
            .describe("Temperature at the world centre before shifts and seasons (°C).")
            .group("Climate"),
        f("temperature_gradient_x", "Gradient left→right", -60.0, 60.0, d.climate.gradient_x)
            .describe("Temperature difference between the right and left edges (°C).")
            .group("Climate"),
        f("temperature_gradient_y", "Gradient top→bottom", -60.0, 60.0, d.climate.gradient_y)
            .describe("Temperature difference between the bottom and top edges (°C).")
            .group("Climate"),
        f("temperature_shift_rate", "Climate shift", -20.0, 20.0, d.climate.shift_rate)
            .describe("Directional temperature change in °C per 1000 ticks.")
            .group("Climate")
            .apply(Immediate),
        f("seasonal_amplitude", "Seasonal amplitude", 0.0, 30.0, d.climate.seasonal_amplitude)
            .describe("Amplitude of the sinusoidal seasonal cycle (°C).")
            .group("Climate"),
        i("seasonal_period", "Seasonal period", 10, 100_000, d.climate.seasonal_period.into())
            .describe("Length of one seasonal cycle in ticks.")
            .group("Climate"),
        ParamSpec::choice(
            "reproduction_mode",
            "Reproduction",
            &[("asexual", "Asexual (clonal)"), ("sexual", "Sexual (two parents)")],
            mode_name(d.reproduction.mode),
        )
        .describe("Asexual: mutated clones. Sexual: two parents in contact, recombined genomes.")
        .group("Reproduction"),
        ParamSpec::choice(
            "crossover",
            "Crossover",
            &[("uniform", "Uniform"), ("one_point", "One-point")],
            crossover_name(d.reproduction.crossover),
        )
        .describe("Recombination operator used in sexual reproduction.")
        .group("Reproduction"),
        f("breeding_threshold", "Breeding threshold", 0.1, 1.0, d.reproduction.threshold)
            .describe("Energy, as a fraction of storage capacity, needed to breed.")
            .group("Reproduction"),
        i("refractory_ticks", "Refractory period", 0, 1000, d.reproduction.refractory.into())
            .describe("Ticks after breeding before an organism may breed again.")
            .group("Reproduction"),
        f("mate_distance_limit", "Mating distance limit", 0.0, 1.0, d.reproduction.mate_distance_limit)
            .describe("Largest genetic distance between mates (1 = no restriction; < 1 = assortative mating).")
            .group("Reproduction"),
        f("mutation_rate", "Mutation rate", 0.0, 1.0, d.mutation.rate)
            .describe("Probability that a heritable gene mutates at birth.")
            .group("Mutation"),
        f("mutation_size", "Mutation size", 0.0, 0.5, d.mutation.size)
            .describe("Standard deviation of a mutation step as a fraction of the gene range.")
            .group("Mutation"),
        ParamSpec::choice(
            "mutation_bounds",
            "Mutation bounds",
            &[("reflect", "Reflect at bounds"), ("clamp", "Clamp to bounds")],
            bounds_name(d.mutation.bounds),
        )
        .describe("How mutated values outside a gene's range are brought back.")
        .group("Mutation"),
        ParamSpec::boolean("predators_evolve", "Predators evolve", d.predators.evolve)
            .describe("Predator offspring recombine and mutate (co-evolution) instead of cloning.")
            .group("Predators"),
        ParamSpec::boolean("predators_constant", "Constant predation", d.predators.constant)
            .describe("Experimental treatment: predators neither breed nor die, so predation pressure stays constant.")
            .group("Predators"),
        f("capture_probability", "Capture probability", 0.0, 1.0, d.predators.capture_probability)
            .describe("Chance that a contact attack kills a prey of equal size.")
            .group("Predators"),
        f("predator_assimilation", "Predator assimilation", 0.0, 1.0, d.predators.assimilation)
            .describe("Fraction of prey energy a predator assimilates.")
            .group("Predators"),
        i("handling_ticks", "Handling time", 0, 1000, d.predators.handling_ticks.into())
            .describe("Ticks a predator spends eating a kill before it can hunt again (saturating predation).")
            .group("Predators"),
        f("predator_interference", "Predator interference", 0.0, 5.0, d.predators.interference)
            .describe("Chance per rival within 3 units that a hungry predator loses the tick (1 − e^(−c·n)).")
            .group("Predators"),
        f("basal_cost", "Basal cost", 0.0, 2.0, d.physiology.basal_cost)
            .describe("Basal metabolism per tick of a unit-size organism (scales as size^0.75 / efficiency).")
            .group("Physiology"),
        f("movement_cost", "Movement cost", 0.0, 2.0, d.physiology.movement_cost)
            .describe("Movement cost coefficient: c × size × speed² per tick, for the speed actually used.")
            .group("Physiology"),
        f("speed_upkeep", "Speed upkeep", 0.0, 1.0, d.physiology.speed_upkeep)
            .describe("Upkeep of locomotor capacity: c × size × max speed² per tick, paid even at rest.")
            .group("Physiology"),
        f("perception_cost", "Perception cost", 0.0, 0.2, d.physiology.perception_cost)
            .describe("Energy per tick per unit of perception radius.")
            .group("Physiology"),
        f("tolerance_cost", "Generalist cost", 0.0, 0.5, d.physiology.tolerance_cost)
            .describe("Basal cost increase per °C of thermal tolerance above 1 °C.")
            .group("Physiology"),
        f("thermal_stress_cost", "Thermal stress cost", 0.0, 5.0, d.physiology.thermal_stress_cost)
            .describe("Basal cost multiplier per squared normalised thermal mismatch.")
            .group("Physiology"),
        f("background_mortality", "Background mortality", 0.0, 0.05, d.physiology.background_mortality)
            .describe("Age-independent death probability per tick.")
            .group("Physiology"),
        f("longevity_ratio", "Longevity ratio", 1.0, 20.0, d.physiology.longevity_ratio)
            .describe("Age (in maturity ages) at which the senescence hazard has grown 1000-fold.")
            .group("Physiology"),
        i("histogram_bins", "Histogram bins", 2, 64, d.statistics.histogram_bins.into())
            .describe("Bins of the per-gene histograms.")
            .group("Statistics")
            .apply(Immediate),
        i("allele_classes", "Allele classes", 2, 16, d.statistics.allele_classes.into())
            .describe("Equal-width value classes per gene used as alleles for frequencies, heterozygosity and F_ST.")
            .group("Statistics")
            .apply(Immediate),
        ParamSpec::boolean("speciation_enabled", "Speciation criterion", d.speciation.enabled)
            .describe("Evaluate the explicit speciation criterion (needs sexual, assortative mating).")
            .group("Statistics"),
        i("speciation_generations", "Speciation persistence", 1, 1000, d.speciation.generations.into())
            .describe("Generations two separated clusters must persist before a split is declared.")
            .group("Statistics"),
    ]
}

/// Widens an `f32` config value to the decimal `f64` it was written as
/// (`0.05f32` becomes `0.05`, not `0.05000000074505806`), so parameter
/// panels and exported settings show the value the user typed. Converting
/// back with `as f32` yields the original `f32` exactly.
fn widen(x: f32) -> f64 {
    x.to_string().parse().unwrap_or_else(|_| f64::from(x))
}

fn mode_name(mode: ReproductionMode) -> &'static str {
    match mode {
        ReproductionMode::Asexual => "asexual",
        ReproductionMode::Sexual => "sexual",
    }
}

fn crossover_name(crossover: Crossover) -> &'static str {
    match crossover {
        Crossover::Uniform => "uniform",
        Crossover::OnePoint => "one_point",
    }
}

fn bounds_name(bounds: MutationBounds) -> &'static str {
    match bounds {
        MutationBounds::Clamp => "clamp",
        MutationBounds::Reflect => "reflect",
    }
}

fn unknown(key: &str) -> EvolutionError {
    EvolutionError::config(key, "unknown parameter")
}

/// Current value of parameter `key`.
pub fn get(c: &EvolutionConfig, key: &str) -> Result<ParamValue, EvolutionError> {
    use ParamValue::{Bool, Float, Int, Text};
    let f = |x: f32| Float(widen(x));
    Ok(match key {
        "width" => Int(c.world.width.into()),
        "height" => Int(c.world.height.into()),
        "initial_prey" => Int(c.prey.initial.into()),
        "initial_predators" => Int(c.predators.initial.into()),
        "genealogy_max_nodes" => Int(c.statistics.genealogy_max_nodes.into()),
        "max_ticks" => Int(i64::try_from(c.max_ticks).unwrap_or(i64::MAX)),
        "food_regrowth_rate" => f(c.food.regrowth_rate),
        "food_capacity_scale" => f(c.food.capacity_scale),
        "food_energy" => f(c.food.energy),
        "base_temperature" => f(c.climate.base_temperature),
        "temperature_gradient_x" => f(c.climate.gradient_x),
        "temperature_gradient_y" => f(c.climate.gradient_y),
        "temperature_shift_rate" => f(c.climate.shift_rate),
        "seasonal_amplitude" => f(c.climate.seasonal_amplitude),
        "seasonal_period" => Int(c.climate.seasonal_period.into()),
        "reproduction_mode" => Text(mode_name(c.reproduction.mode).into()),
        "crossover" => Text(crossover_name(c.reproduction.crossover).into()),
        "breeding_threshold" => f(c.reproduction.threshold),
        "refractory_ticks" => Int(c.reproduction.refractory.into()),
        "mate_distance_limit" => f(c.reproduction.mate_distance_limit),
        "mutation_rate" => f(c.mutation.rate),
        "mutation_size" => f(c.mutation.size),
        "mutation_bounds" => Text(bounds_name(c.mutation.bounds).into()),
        "predators_evolve" => Bool(c.predators.evolve),
        "predators_constant" => Bool(c.predators.constant),
        "capture_probability" => f(c.predators.capture_probability),
        "predator_assimilation" => f(c.predators.assimilation),
        "handling_ticks" => Int(c.predators.handling_ticks.into()),
        "predator_interference" => f(c.predators.interference),
        "basal_cost" => f(c.physiology.basal_cost),
        "movement_cost" => f(c.physiology.movement_cost),
        "speed_upkeep" => f(c.physiology.speed_upkeep),
        "perception_cost" => f(c.physiology.perception_cost),
        "tolerance_cost" => f(c.physiology.tolerance_cost),
        "thermal_stress_cost" => f(c.physiology.thermal_stress_cost),
        "background_mortality" => f(c.physiology.background_mortality),
        "longevity_ratio" => f(c.physiology.longevity_ratio),
        "histogram_bins" => Int(c.statistics.histogram_bins.into()),
        "allele_classes" => Int(c.statistics.allele_classes.into()),
        "speciation_enabled" => Bool(c.speciation.enabled),
        "speciation_generations" => Int(c.speciation.generations.into()),
        _ => return Err(unknown(key)),
    })
}

/// Validates `value` against the spec of `key`, writes it into `c` and
/// returns the spec's apply policy. Nothing changes on error.
pub fn set(c: &mut EvolutionConfig, key: &str, value: &ParamValue) -> Result<ApplyPolicy, EvolutionError> {
    let spec = specs().into_iter().find(|s| s.key == key).ok_or_else(|| unknown(key))?;
    let value = spec.validate(value).map_err(|e| EvolutionError::config(field_path(key), e.to_string()))?;
    let bad = || EvolutionError::config(field_path(key), format!("cannot store {value}"));
    // Validated values are in range, so the narrowing conversions cannot fail.
    let float = || value.as_f64().map(|x| x as f32).ok_or_else(bad);
    let int = || value.as_i64().and_then(|x| u32::try_from(x).ok()).ok_or_else(bad);
    let text = || value.as_str().ok_or_else(bad);
    let boolean = || value.as_bool().ok_or_else(bad);
    match key {
        "width" => c.world.width = int()?,
        "height" => c.world.height = int()?,
        "initial_prey" => c.prey.initial = int()?,
        "initial_predators" => c.predators.initial = int()?,
        "genealogy_max_nodes" => c.statistics.genealogy_max_nodes = int()?,
        "max_ticks" => c.max_ticks = value.as_i64().and_then(|x| u64::try_from(x).ok()).ok_or_else(bad)?,
        "food_regrowth_rate" => c.food.regrowth_rate = float()?,
        "food_capacity_scale" => c.food.capacity_scale = float()?,
        "food_energy" => c.food.energy = float()?,
        "base_temperature" => c.climate.base_temperature = float()?,
        "temperature_gradient_x" => c.climate.gradient_x = float()?,
        "temperature_gradient_y" => c.climate.gradient_y = float()?,
        "temperature_shift_rate" => c.climate.shift_rate = float()?,
        "seasonal_amplitude" => c.climate.seasonal_amplitude = float()?,
        "seasonal_period" => c.climate.seasonal_period = int()?,
        "reproduction_mode" => {
            c.reproduction.mode = if text()? == "sexual" { ReproductionMode::Sexual } else { ReproductionMode::Asexual }
        }
        "crossover" => {
            c.reproduction.crossover = if text()? == "one_point" { Crossover::OnePoint } else { Crossover::Uniform }
        }
        "breeding_threshold" => c.reproduction.threshold = float()?,
        "refractory_ticks" => c.reproduction.refractory = int()?,
        "mate_distance_limit" => c.reproduction.mate_distance_limit = float()?,
        "mutation_rate" => c.mutation.rate = float()?,
        "mutation_size" => c.mutation.size = float()?,
        "mutation_bounds" => {
            c.mutation.bounds = if text()? == "clamp" { MutationBounds::Clamp } else { MutationBounds::Reflect }
        }
        "predators_evolve" => c.predators.evolve = boolean()?,
        "predators_constant" => c.predators.constant = boolean()?,
        "capture_probability" => c.predators.capture_probability = float()?,
        "predator_assimilation" => c.predators.assimilation = float()?,
        "handling_ticks" => c.predators.handling_ticks = int()?,
        "predator_interference" => c.predators.interference = float()?,
        "basal_cost" => c.physiology.basal_cost = float()?,
        "movement_cost" => c.physiology.movement_cost = float()?,
        "speed_upkeep" => c.physiology.speed_upkeep = float()?,
        "perception_cost" => c.physiology.perception_cost = float()?,
        "tolerance_cost" => c.physiology.tolerance_cost = float()?,
        "thermal_stress_cost" => c.physiology.thermal_stress_cost = float()?,
        "background_mortality" => c.physiology.background_mortality = float()?,
        "longevity_ratio" => c.physiology.longevity_ratio = float()?,
        "histogram_bins" => c.statistics.histogram_bins = int()?,
        "allele_classes" => c.statistics.allele_classes = int()?,
        "speciation_enabled" => c.speciation.enabled = boolean()?,
        "speciation_generations" => c.speciation.generations = int()?,
        _ => return Err(unknown(key)),
    }
    Ok(spec.apply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_has_a_field_and_round_trips() {
        let specs = specs();
        assert_eq!(specs.len(), FIELDS.len());
        let mut config = EvolutionConfig::default();
        for (spec, (key, _)) in specs.iter().zip(FIELDS) {
            assert_eq!(spec.key, key, "FIELDS and specs share one order");
            let value = get(&config, key).unwrap();
            assert_eq!(spec.validate(&value).unwrap(), spec.default, "{key}: default matches the config default");
            assert_eq!(set(&mut config, key, &value).unwrap(), spec.apply);
            assert_eq!(get(&config, key).unwrap(), value);
        }
        assert_eq!(config, EvolutionConfig::default());
    }

    #[test]
    fn set_rejects_bad_values_without_changing_anything() {
        let mut c = EvolutionConfig::default();
        let before = c.clone();
        assert!(set(&mut c, "mutation_rate", &ParamValue::Float(1.5)).is_err());
        assert!(set(&mut c, "reproduction_mode", &ParamValue::Text("budding".into())).is_err());
        assert!(set(&mut c, "nope", &ParamValue::Int(1)).is_err());
        assert_eq!(c, before);
        assert_eq!(
            set(&mut c, "reproduction_mode", &ParamValue::Text("sexual".into())).unwrap(),
            ApplyPolicy::NextTick
        );
        assert_eq!(c.reproduction.mode, ReproductionMode::Sexual);
        assert_eq!(set(&mut c, "width", &ParamValue::Int(40)).unwrap(), ApplyPolicy::RequiresReset);
        assert_eq!(field_path("food_energy"), "food.energy");
        assert_eq!(get(&c, "mutation_size").unwrap(), ParamValue::Float(0.05), "shown as typed");
    }
}
