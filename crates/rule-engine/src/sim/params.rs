//! Parameter schema of the custom-life model.
//!
//! Parameters are generated from the current definition: world settings,
//! one group per resource field, entity type and random event. Each spec
//! states its [`ApplyPolicy`]; editing a parameter edits the definition
//! (next-tick parameters go through the same path as
//! [`crate::CustomLifeSim::edit_system`]).

use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError, Topology};

use crate::compile::{MAX_ENTITIES_CEILING, MAX_WORLD_SIDE, MIN_WORLD_SIDE};
use crate::def::{LifeSystem, ReproductionMode};

const TOPOLOGIES: [(&str, &str); 4] = [
    ("bounded", "Bounded (reflecting walls)"),
    ("torus", "Torus (wrap-around)"),
    ("klein_bottle", "Klein bottle"),
    ("cross_surface", "Cross-surface"),
];

fn topology_key(t: Topology) -> &'static str {
    match t {
        Topology::Bounded => "bounded",
        Topology::Torus => "torus",
        Topology::KleinBottle => "klein_bottle",
        Topology::CrossSurface => "cross_surface",
        Topology::Infinite => "infinite",
    }
}

/// Parameter specs for `sys`.
pub fn param_specs(sys: &LifeSystem) -> Vec<ParamSpec> {
    let side = (i64::from(MIN_WORLD_SIDE), i64::from(MAX_WORLD_SIDE));
    let mut specs = vec![
        ParamSpec::int("seed", "Seed", 0, i64::MAX, 0)
            .describe("Random seed of the scenario (placement, fields, decisions, events).")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("width", "Width", side.0, side.1, i64::from(sys.world.width))
            .describe("World width in cells.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("height", "Height", side.0, side.1, i64::from(sys.world.height))
            .describe("World height in cells.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::choice("topology", "Topology", &TOPOLOGIES, topology_key(sys.world.topology))
            .describe("How the world edges connect.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int(
            "max_entities",
            "Entity cap",
            1,
            i64::from(MAX_ENTITIES_CEILING),
            i64::from(sys.world.max_entities),
        )
        .describe("Births and spawns beyond this many live entities are refused.")
        .group("World")
        .apply(ApplyPolicy::NextTick),
        ParamSpec::boolean("pause_on_end", "Pause on end condition", true)
            .describe("Finish (pause) the run when an end condition is met.")
            .group("World")
            .apply(ApplyPolicy::Immediate),
    ];
    for r in &sys.world.resources {
        let group = format!("Resource: {}", r.name);
        let key = |k: &str| format!("resource.{}.{k}", r.name);
        specs.push(
            ParamSpec::float(&key("regen"), "Regrowth per tick", 0.0, 100.0, f64::from(r.regen))
                .describe("Amount added to every cell below the maximum each tick.")
                .group(&group)
                .apply(ApplyPolicy::NextTick),
        );
        specs.push(
            ParamSpec::float(&key("max"), "Maximum per cell", 0.0, 1.0e6, f64::from(r.max))
                .describe("Regrowth ceiling per cell.")
                .group(&group)
                .apply(ApplyPolicy::NextTick),
        );
        specs.push(
            ParamSpec::float(&key("diffusion"), "Diffusion", 0.0, 0.25, f64::from(r.diffusion))
                .describe("Explicit 5-point diffusion coefficient (stable up to 0.25).")
                .group(&group)
                .apply(ApplyPolicy::NextTick),
        );
        specs.push(
            ParamSpec::float(&key("decay"), "Decay", 0.0, 1.0, f64::from(r.decay))
                .describe("Fraction lost per tick.")
                .group(&group)
                .apply(ApplyPolicy::NextTick),
        );
    }
    for t in &sys.entity_types {
        let group = format!("Entity: {}", t.name);
        let key = |k: &str| format!("entity.{}.{k}", t.name);
        specs.push(
            ParamSpec::float(&key("metabolism"), "Metabolism", 0.0, 1000.0, t.energy.metabolism)
                .describe("Energy lost per tick.")
                .group(&group)
                .apply(ApplyPolicy::NextTick),
        );
        if t.reproduction.mode != ReproductionMode::None {
            specs.push(
                ParamSpec::float(&key("reproduction_rate"), "Reproduction rate", 0.0, 1.0, t.reproduction.rate)
                    .describe("Per-tick chance of reproducing when eligible (generated rule; 0 = rules only).")
                    .group(&group)
                    .apply(ApplyPolicy::NextTick),
            );
        }
        specs.push(
            ParamSpec::int(
                &key("initial_count"),
                "Initial count",
                0,
                i64::from(sys.world.max_entities),
                i64::from(t.initial.count),
            )
            .describe("Entities created at reset.")
            .group(&group)
            .apply(ApplyPolicy::RequiresReset),
        );
    }
    for ev in &sys.world.random_events {
        specs.push(
            ParamSpec::float(
                &format!("event.{}.probability", ev.name),
                "Probability per tick",
                0.0,
                1.0,
                ev.probability,
            )
            .describe("Chance that the event happens in a tick.")
            .group(&format!("Event: {}", ev.name))
            .apply(ApplyPolicy::NextTick),
        );
    }
    specs
}

/// Current values (the specs' defaults are the definition's values).
pub fn current_params(sys: &LifeSystem, seed: u64, pause_on_end: bool) -> ParamSet {
    let mut set = ParamSet::from_defaults(&param_specs(sys));
    set.set("seed", ParamValue::Int((seed & i64::MAX as u64) as i64));
    set.set("pause_on_end", ParamValue::Bool(pause_on_end));
    set
}

/// Writes a validated parameter value into a definition.
pub fn apply_param(sys: &mut LifeSystem, key: &str, value: &ParamValue) -> Result<(), SimError> {
    let f = || value.as_f64().ok_or_else(|| SimError::invalid_param(key, "expected a number"));
    let i = || value.as_i64().ok_or_else(|| SimError::invalid_param(key, "expected an integer"));
    match key {
        "width" => sys.world.width = i()? as u32,
        "height" => sys.world.height = i()? as u32,
        "max_entities" => sys.world.max_entities = i()? as u32,
        "topology" => {
            sys.world.topology = match value.as_str() {
                Some("bounded") => Topology::Bounded,
                Some("torus") => Topology::Torus,
                Some("klein_bottle") => Topology::KleinBottle,
                Some("cross_surface") => Topology::CrossSurface,
                _ => return Err(SimError::invalid_param(key, "unknown topology")),
            }
        }
        _ => {
            let unknown = || SimError::UnknownParameter(key.to_string());
            // Names may contain dots, so the field is split off from the right.
            let (scope, field) = key.rsplit_once('.').ok_or_else(unknown)?;
            if let Some(name) = scope.strip_prefix("resource.") {
                let r = sys.world.resources.iter_mut().find(|r| r.name == name).ok_or_else(unknown)?;
                let v = f()? as f32;
                match field {
                    "regen" => r.regen = v,
                    "max" => r.max = v,
                    "diffusion" => r.diffusion = v,
                    "decay" => r.decay = v,
                    _ => return Err(unknown()),
                }
            } else if let Some(name) = scope.strip_prefix("entity.") {
                let t = sys.entity_types.iter_mut().find(|t| t.name == name).ok_or_else(unknown)?;
                match field {
                    "metabolism" => t.energy.metabolism = f()?,
                    "reproduction_rate" => t.reproduction.rate = f()?,
                    "initial_count" => t.initial.count = i()? as u32,
                    _ => return Err(unknown()),
                }
            } else if let Some(name) = scope.strip_prefix("event.")
                && field == "probability"
            {
                let e = sys.world.random_events.iter_mut().find(|e| e.name == name).ok_or_else(unknown)?;
                e.probability = f()?;
            } else {
                return Err(unknown());
            }
        }
    }
    Ok(())
}
