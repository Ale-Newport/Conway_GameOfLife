//! Parameters of [`LifeSim`]: specs, current values and changes.

use patterns::LifeRule;
use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError, Topology};

use super::{LifeSim, MAX_GENERATIONS_PER_TICK, MAX_TRACKED_CELLS, RulePreset};
use crate::engine::{EngineChoice, EngineKind, MAX_SWITCH_CELLS};
use crate::error::CaError;
use crate::geometry::{MAX_EDIT_CELLS, MAX_WORLD_SIDE, World};

/// Topology choices (machine name, label).
const TOPOLOGIES: [(&str, &str); 5] = [
    ("bounded", "Bounded plane (dead outside)"),
    ("torus", "Torus (wrap-around)"),
    ("klein_bottle", "Klein bottle"),
    ("cross_surface", "Cross-surface (projective plane)"),
    ("infinite", "Infinite plane"),
];

/// Machine name of a topology.
pub(crate) fn topology_name(topology: Topology) -> &'static str {
    match topology {
        Topology::Bounded => "bounded",
        Topology::Torus => "torus",
        Topology::KleinBottle => "klein_bottle",
        Topology::CrossSurface => "cross_surface",
        Topology::Infinite => "infinite",
    }
}

/// Parses a topology machine name.
pub(crate) fn parse_topology(name: &str) -> Option<Topology> {
    Topology::ALL.into_iter().find(|&t| topology_name(t) == name)
}

/// Description of a `track_*` parameter: `what` it records, then the limits
/// of tracking.
fn tracking_description(what: &str) -> String {
    format!(
        "{what} Tracking lists every live cell to start, so it stays off above {MAX_TRACKED_CELLS} live cells and \
         turns off when a reset or a pattern load brings more, or an edit leaves more than {MAX_EDIT_CELLS} in its \
         region."
    )
}

/// Parameter specs (defaults are the defaults of a new model).
pub(super) fn specs() -> Vec<ParamSpec> {
    let presets: Vec<(&str, &str)> = RulePreset::ALL.iter().map(|p| (p.as_str(), p.label())).collect();
    let default_rule = LifeRule::conway().to_string();
    vec![
        ParamSpec::choice("preset", "Rule preset", &presets, RulePreset::Conway.as_str())
            .group("Rules")
            .apply(ApplyPolicy::NextTick)
            .describe(
                "Named Life-like rules. Presets are fixed: editing the rule selects \"Custom rule\" and never \
                 changes a preset, so Conway's B3/S23 cannot be altered by accident.",
            ),
        ParamSpec::text("rule", "Rule", 64, &default_rule).group("Rules").apply(ApplyPolicy::NextTick).describe(
            "Birth/survival rule, e.g. B3/S23, B36/S23 or the legacy S/B form 23/3. B0 rules are only allowed \
             on finite worlds.",
        ),
        ParamSpec::choice("topology", "Topology", &TOPOLOGIES, "torus")
            .group("World")
            .apply(ApplyPolicy::RequiresReset)
            .describe("Edge behaviour. The infinite plane uses 64-bit coordinates."),
        ParamSpec::int("width", "Width", 1, i64::from(MAX_WORLD_SIDE), 256)
            .group("World")
            .apply(ApplyPolicy::RequiresReset)
            .describe("World width in cells (the home region on the infinite plane)."),
        ParamSpec::int("height", "Height", 1, i64::from(MAX_WORLD_SIDE), 256)
            .group("World")
            .apply(ApplyPolicy::RequiresReset)
            .describe("World height in cells (the home region on the infinite plane)."),
        ParamSpec::choice(
            "engine",
            "Engine",
            &[
                ("auto", "Automatic"),
                ("dense", "Dense (byte per cell)"),
                ("bit", "Bit-packed SWAR (finite worlds)"),
                ("sparse", "Sparse tiles (infinite plane)"),
                ("hashlife", "Hashlife (infinite plane)"),
            ],
            "auto",
        )
        .group("Engine")
        .apply(ApplyPolicy::Immediate)
        .describe(&format!(
            "Simulation engine. All engines compute the same generations; switching keeps every cell. \
             Automatic: Dense for small finite worlds, Bit for large ones, Sparse on the infinite plane, \
             Hashlife for large power-of-two steps or large regular patterns. A switch copies every live cell, \
             so it is refused above {MAX_SWITCH_CELLS} live cells (the automatic choice then keeps the running \
             engine)."
        )),
        ParamSpec::int("generations_per_tick", "Generations per tick", 1, MAX_GENERATIONS_PER_TICK as i64, 1)
            .group("Engine")
            .apply(ApplyPolicy::NextTick)
            .describe("Generations advanced by one tick. Must be a power of two while Hashlife runs."),
        ParamSpec::boolean("track_ages", "Track ages", false).group("Display").apply(ApplyPolicy::Immediate).describe(
            &tracking_description(
                "Record how many generations each live cell has been alive (view channel G). Updated every \
                 generation.",
            ),
        ),
        ParamSpec::boolean("track_activity", "Track activity", false)
            .group("Display")
            .apply(ApplyPolicy::Immediate)
            .describe(&tracking_description(
                "Record cells that changed during the last tick (view channel B = 255), fading by 16 per tick.",
            )),
    ]
}

/// Current parameter values (pending NextTick / RequiresReset values shown).
pub(super) fn values(sim: &LifeSim) -> ParamSet {
    let (rule, preset) = sim.pending_rule.unwrap_or((sim.rule, sim.preset));
    ParamSet::new()
        .with("preset", ParamValue::Text(preset.as_str().into()))
        .with("rule", ParamValue::Text(rule.to_string()))
        .with("topology", ParamValue::Text(topology_name(sim.next_world.topology).into()))
        .with("width", ParamValue::Int(i64::from(sim.next_world.width)))
        .with("height", ParamValue::Int(i64::from(sim.next_world.height)))
        .with("engine", ParamValue::Text(sim.engine_choice.as_str().into()))
        .with("generations_per_tick", ParamValue::Int(sim.generations_per_tick() as i64))
        .with("track_ages", ParamValue::Bool(sim.tracker.ages_on()))
        .with("track_activity", ParamValue::Bool(sim.tracker.activity_on()))
}

/// The rule that will run next.
fn effective_rule(sim: &LifeSim) -> LifeRule {
    sim.pending_rule.map_or(sim.rule, |(rule, _)| rule)
}

/// Checks that `rule` can run on `world` with the engine choice.
fn check_combination(sim: &LifeSim, world: &World, rule: LifeRule, generations: u64) -> Result<(), CaError> {
    LifeSim::choose(sim.engine_choice, *world, rule, 0, generations, None).map(|_| ())
}

/// Checks a rule against the running world and the world of the next reset.
fn check_rule(sim: &LifeSim, rule: LifeRule, generations: u64) -> Result<(), CaError> {
    check_combination(sim, &sim.world, rule, generations)?;
    check_combination(sim, &sim.next_world, rule, generations)
}

/// Applies one parameter change.
pub(super) fn set(sim: &mut LifeSim, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
    let spec = specs().into_iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.into()))?;
    let value = spec.validate(&value)?;
    let invalid = |e: CaError| SimError::invalid_param(key, e.to_string());
    let text = value.as_str().unwrap_or_default().to_string();
    let generations = sim.generations_per_tick();
    match key {
        "preset" => {
            let preset = RulePreset::parse(&text).ok_or_else(|| SimError::invalid_param(key, "unknown preset"))?;
            let rule = preset.rule().unwrap_or_else(|| effective_rule(sim));
            if preset != RulePreset::Custom {
                check_rule(sim, rule, generations).map_err(invalid)?;
            }
            sim.pending_rule = Some((rule, preset));
        }
        "rule" => {
            let rule = LifeRule::parse(&text).map_err(|e| SimError::invalid_param(key, e.to_string()))?;
            check_rule(sim, rule, generations).map_err(invalid)?;
            sim.pending_rule = Some((rule, RulePreset::matching(rule)));
        }
        "topology" | "width" | "height" => {
            let mut next = sim.next_world;
            match key {
                "topology" => {
                    next.topology =
                        parse_topology(&text).ok_or_else(|| SimError::invalid_param(key, "unknown topology"))?;
                }
                "width" => next.width = value.as_i64().unwrap_or(1) as u32,
                _ => next.height = value.as_i64().unwrap_or(1) as u32,
            }
            let next = World::new(next.topology, next.width, next.height).map_err(invalid)?;
            // The next reset must be able to rebuild the scenario on this world.
            sim.check_rebuild(next, sim.engine_choice, effective_rule(sim), generations).map_err(invalid)?;
            sim.next_world = next;
        }
        "engine" => {
            let choice = EngineChoice::parse(&text).ok_or_else(|| SimError::invalid_param(key, "unknown engine"))?;
            let previous = sim.engine_choice;
            let kind = match choice.fixed() {
                Some(_) => LifeSim::choose(choice, sim.world, sim.rule, 0, sim.generations_per_tick, None)
                    .and_then(|kind| {
                        // The pending rule and generations, and the next reset, must suit the engine too.
                        sim.check_rebuild(sim.next_world, choice, effective_rule(sim), generations).map(|()| kind)
                    })
                    .map_err(invalid)?,
                None => EngineKind::Dense, // replaced by the automatic choice below
            };
            sim.engine_choice = choice;
            let switched = if choice == EngineChoice::Auto { sim.reselect() } else { sim.set_engine_kind(kind) };
            if let Err(e) = switched {
                sim.engine_choice = previous;
                return Err(invalid(e));
            }
        }
        "generations_per_tick" => {
            let n = value.as_i64().unwrap_or(1) as u64;
            if sim.engine_choice == EngineChoice::Hashlife && !n.is_power_of_two() {
                return Err(SimError::invalid_param(
                    key,
                    format!("Hashlife advances in powers of two; {n} is not one (use 1, 2, 4, 8, ...)"),
                ));
            }
            sim.pending_generations = Some(n);
        }
        "track_ages" | "track_activity" => {
            let on = value.as_bool().unwrap_or(false);
            let (ages, activity) =
                if key == "track_ages" { (on, sim.tracker.activity_on()) } else { (sim.tracker.ages_on(), on) };
            sim.set_tracking(ages, activity).map_err(invalid)?;
        }
        _ => return Err(SimError::UnknownParameter(key.into())),
    }
    Ok(spec.apply)
}
