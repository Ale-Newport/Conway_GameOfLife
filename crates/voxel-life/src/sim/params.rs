//! Parameter declarations and changes.
//!
//! `rule` applies at the next tick. Everything that describes the world or
//! its initial state (edges, storage, size, initialisation, seed) is stored
//! as a *pending* configuration and used by the next reset; each change is
//! validated against the whole pending configuration, so the pending
//! configuration is always valid.

use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError};

use super::VoxelLife;
use crate::geometry::{MAX_SIDE, WorldKind};
use crate::init::{InitKind, Mirror};
use crate::rule::Rule3D;

const SIZE_KEYS: [&str; 3] = ["size_x", "size_y", "size_z"];

/// Parameter declarations; defaults are the scenario's values.
pub(super) fn param_specs(life: &VoxelLife) -> Vec<ParamSpec> {
    let scenario = life.scenario();
    let world_options: Vec<(&str, &str)> = WorldKind::ALL.iter().map(|k| (k.key(), k.label())).collect();
    let init_options: Vec<(&str, &str)> = InitKind::ALL
        .iter()
        .filter(|k| **k != InitKind::Pattern || !scenario.init.pattern.is_empty())
        .map(|k| (k.key(), k.label()))
        .collect();
    let mirror_options: Vec<(&str, &str)> = Mirror::ALL.iter().map(|m| (m.key(), m.key())).collect();
    let mut specs = vec![
        ParamSpec::text("rule", "Rule", 256, &scenario.rule.to_string())
            .describe(
                "Softology S/B/C/N (4/4/5/M), letters B5/S45 (optional /C<states>, /N), Bays digits 5766 or Golly \
                 3D5..7/6. Voxels in states the new rule lacks are cleared.",
            )
            .group("Rules")
            .apply(ApplyPolicy::NextTick),
        ParamSpec::choice("world", "Edges and storage", &world_options, scenario.world.key())
            .describe("Dense grids (bounded or toroidal) or sparse 16³ chunks (bounded or unbounded).")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
    ];
    for (axis, key) in SIZE_KEYS.iter().enumerate() {
        let name = ["x", "y", "z"][axis];
        specs.push(
            ParamSpec::int(key, &format!("Size {name}"), 1, i64::from(MAX_SIDE), i64::from(scenario.size[axis]))
                .describe("World size along this axis (dense worlds hold at most 256³ voxels).")
                .group("World")
                .apply(ApplyPolicy::RequiresReset),
        );
    }
    specs.extend([
        ParamSpec::choice("init", "Initial state", &init_options, scenario.init.kind.key())
            .describe("What the world contains after a reset.")
            .group("Initial state")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("cube_side", "Random cube side", 1, i64::from(MAX_SIDE), i64::from(scenario.init.cube_side))
            .describe("Side of the centred random cube (reset with a random cube, and Randomize).")
            .group("Initial state")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::float("density", "Density", 0.0, 1.0, scenario.init.density)
            .step(0.01)
            .describe("Probability that a voxel of the random cube is alive.")
            .group("Initial state")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::choice("mirror", "Mirror symmetry", &mirror_options, scenario.init.mirror.key())
            .describe("Axes along which the random cube is mirror-symmetric about its centre.")
            .group("Initial state")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("seed", "Seed", 0, i64::MAX, i64::try_from(scenario.seed).unwrap_or(i64::MAX))
            .describe("Seed of the random initial state and of seedless Randomize commands.")
            .group("Initial state")
            .apply(ApplyPolicy::RequiresReset),
    ]);
    specs
}

/// Current values (pending values for reset-time parameters).
pub(super) fn current(life: &VoxelLife) -> ParamSet {
    let pending = life.pending_config();
    let mut set = ParamSet::new()
        .with("rule", ParamValue::Text(life.rule().to_string()))
        .with("world", ParamValue::Text(pending.kind.key().to_string()));
    for (axis, key) in SIZE_KEYS.iter().enumerate() {
        set.set(*key, ParamValue::Int(i64::from(pending.size[axis])));
    }
    set.with("init", ParamValue::Text(pending.init.kind.key().to_string()))
        .with("cube_side", ParamValue::Int(i64::from(pending.init.cube_side)))
        .with("density", ParamValue::Float(pending.init.density))
        .with("mirror", ParamValue::Text(pending.init.mirror.key().to_string()))
        .with("seed", ParamValue::Int(i64::try_from(pending.seed).unwrap_or(i64::MAX)))
}

/// Validates and applies one parameter change.
pub(super) fn set(life: &mut VoxelLife, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
    let spec = param_specs(life)
        .into_iter()
        .find(|s| s.key == key)
        .ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
    let value = spec.validate(&value)?;
    let invalid = |reason: String| SimError::invalid_param(key, reason);
    let text = || value.as_str().unwrap_or_default();
    // Integers were range-checked by the spec (sizes and sides fit in u32).
    let integer = || value.as_i64().unwrap_or_default();
    if key == "rule" {
        let rule = Rule3D::parse(text()).map_err(|e| invalid(e.to_string()))?;
        life.set_rule(rule);
        return Ok(spec.apply);
    }
    let mut candidate = life.pending_config().clone();
    match key {
        "world" => {
            candidate.kind = WorldKind::from_key(text()).ok_or_else(|| invalid(format!("unknown `{}`", text())))?
        }
        "init" => {
            candidate.init.kind = InitKind::from_key(text()).ok_or_else(|| invalid(format!("unknown `{}`", text())))?
        }
        "mirror" => {
            candidate.init.mirror = Mirror::from_key(text()).ok_or_else(|| invalid(format!("unknown `{}`", text())))?
        }
        "cube_side" => candidate.init.cube_side = integer() as u32,
        "density" => candidate.init.density = value.as_f64().unwrap_or_default(),
        "seed" => candidate.seed = integer() as u64,
        size_key => {
            let axis = SIZE_KEYS.iter().position(|k| *k == size_key).ok_or_else(|| invalid("unknown key".into()))?;
            candidate.size[axis] = integer() as u32;
        }
    }
    candidate.validate().map_err(|e| invalid(e.to_string()))?;
    life.set_pending(candidate);
    Ok(spec.apply)
}
