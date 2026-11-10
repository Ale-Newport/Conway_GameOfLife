//! Scheduled interventions of a scenario: inputs applied at a given
//! simulated day (an ignition, the start of a drought, the arrival of an
//! invasive species, the removal of a predator, an outbreak, a parameter
//! change). Only the intervention is scripted; its consequences emerge from
//! the model.
//!
//! An event of day `d` runs at the start of tick `round(d · ticks_per_day)`,
//! after queued parameter changes, in file order (days must not decrease).

use physics_world::Disaster;
use serde::{Deserialize, Serialize};
use simulation_core::{ParamKind, ParamSpec, ParamValue};

use crate::config::Rules;
use crate::error::EcosystemError;
use crate::species::SpeciesCatalog;

/// What a scheduled event does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// Sets a cell on fire (nothing happens if it has no dry fuel).
    Ignite {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
    },
    /// An intervention of the physics world (drought, storm, flood...).
    World(WorldEvent),
    /// Adds animals (individuals: founders with random age; density
    /// fields: individuals spread over the habitat of the region;
    /// colonies: `count` colonies).
    Introduce {
        /// Species id.
        species: String,
        /// Number of animals, field individuals or colonies.
        count: u32,
        /// Placement rectangle `(x0, y0, x1, y1)` in cells (`None` = the map).
        #[serde(default)]
        region: Option<(f32, f32, f32, f32)>,
    },
    /// Removes a fraction of a species from the ecosystem (hunting,
    /// eradication); `1` removes every animal (local extinction).
    Remove {
        /// Species id.
        species: String,
        /// Fraction in `[0, 1]`.
        fraction: f32,
    },
    /// Infects `count` random hosts of a species with a pathogen.
    Outbreak {
        /// Pathogen id.
        pathogen: String,
        /// Host species id.
        species: String,
        /// Number of animals infected.
        count: u32,
    },
    /// Changes a numeric or boolean parameter (booleans: 0 or 1), with the
    /// parameter's own apply policy.
    SetParam {
        /// Parameter key.
        key: String,
        /// New value.
        value: f64,
    },
}

/// A physics-world intervention of a schedule: the interventions of
/// [`physics_world::Disaster`] (ignitions are [`EventAction::Ignite`]),
/// with spells in days, in a form every serde format can store (the
/// schedule is part of snapshots).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldEvent {
    /// No rain and 1.5 × evaporation for `days`.
    Drought {
        /// Duration (days).
        days: f32,
    },
    /// +4–7 °C and 1.4 × evaporation for `days`.
    Heatwave {
        /// Duration (days).
        days: f32,
    },
    /// A thunderstorm (rain × 3 × intensity, gusts, lightning) for `days`.
    Storm {
        /// Duration (days).
        days: f32,
        /// Precipitation multiplier in `(0, 10]`.
        intensity: f32,
    },
    /// Rain (or snow) on every non-ocean cell.
    AddRain {
        /// Depth (mm).
        amount_mm: f32,
    },
    /// A dome of water around a cell.
    Flood {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius (cells).
        radius: f32,
        /// Depth at the centre (m).
        depth_m: f32,
    },
    /// Fault displacement and slope failures around an epicentre.
    Earthquake {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius (cells).
        radius: f32,
        /// Moment magnitude.
        magnitude: f32,
    },
    /// Wind damage along a polyline.
    Tornado {
        /// Path vertices `[x, y]` (cells).
        path: Vec<[f32; 2]>,
        /// Width of the swath (cells).
        width: f32,
    },
    /// Slope failures around a point.
    Landslide {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius (cells).
        radius: f32,
    },
    /// A volcanic cone, lava and ash.
    Volcano {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Cone radius (cells).
        radius: f32,
        /// Cone height (m).
        height_m: f32,
    },
}

impl WorldEvent {
    /// Machine-readable kind (the [`Disaster`] kind).
    pub fn kind(&self) -> &'static str {
        self.to_disaster(1.0).kind()
    }

    /// Checks the durations (the world validates the rest when it runs).
    fn validate(&self) -> Result<(), String> {
        match *self {
            Self::Drought { days } | Self::Heatwave { days } | Self::Storm { days, .. }
                if !(days.is_finite() && days > 0.0 && days <= 10_000.0) =>
            {
                Err(format!("duration of {days} days is outside (0, 10000]"))
            }
            _ => Ok(()),
        }
    }

    /// The physics-world intervention.
    pub fn to_disaster(&self, ticks_per_day: f64) -> Disaster {
        let ticks = |days: f32| (f64::from(days) * ticks_per_day).round().max(1.0) as u64;
        match self.clone() {
            Self::Drought { days } => Disaster::Drought { duration_ticks: ticks(days) },
            Self::Heatwave { days } => Disaster::Heatwave { duration_ticks: ticks(days) },
            Self::Storm { days, intensity } => Disaster::Storm { duration_ticks: ticks(days), intensity },
            Self::AddRain { amount_mm } => Disaster::AddRain { amount_mm },
            Self::Flood { x, y, radius, depth_m } => Disaster::Flood { x, y, radius, depth_m },
            Self::Earthquake { x, y, radius, magnitude } => Disaster::Earthquake { x, y, radius, magnitude },
            Self::Tornado { path, width } => Disaster::Tornado { path, width },
            Self::Landslide { x, y, radius } => Disaster::Landslide { x, y, radius },
            Self::Volcano { x, y, radius, height_m } => Disaster::Volcano { x, y, radius, height_m },
        }
    }
}

/// One scheduled event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledEvent {
    /// Simulated day (fractional) at which it happens.
    pub day: f32,
    /// What it does.
    pub action: EventAction,
}

impl EventAction {
    /// Short machine-readable kind (event log).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ignite { .. } => "ignite",
            Self::World(w) => w.kind(),
            Self::Introduce { .. } => "introduce",
            Self::Remove { .. } => "remove",
            Self::Outbreak { .. } => "outbreak",
            Self::SetParam { .. } => "set_param",
        }
    }
}

/// The value of a numeric parameter change for its spec (integers must be
/// whole, booleans 0 or 1).
pub(crate) fn param_value(spec: &ParamSpec, value: f64) -> Result<ParamValue, EcosystemError> {
    let bad = |reason: &str| EcosystemError::config(&format!("events.set_param.{}", spec.key), reason.to_string());
    let v = match spec.kind {
        ParamKind::Bool => {
            if value == 0.0 {
                ParamValue::Bool(false)
            } else if value == 1.0 {
                ParamValue::Bool(true)
            } else {
                return Err(bad("boolean parameters take 0 or 1"));
            }
        }
        ParamKind::Int { .. } => {
            if value.fract() != 0.0 || !value.is_finite() {
                return Err(bad("integer parameters need a whole number"));
            }
            ParamValue::Int(value as i64)
        }
        ParamKind::Float { .. } => ParamValue::Float(value),
        _ => return Err(bad("only numeric and boolean parameters can be scheduled")),
    };
    spec.validate(&v).map_err(|e| bad(&e.to_string()))
}

/// Validates the schedule of a configuration.
pub(crate) fn validate(
    events: &[ScheduledEvent],
    catalog: &SpeciesCatalog,
    pathogens: &[crate::disease::Pathogen],
    rules: &Rules,
) -> Result<(), EcosystemError> {
    let specs = crate::params::param_specs(rules);
    let mut previous = 0.0f32;
    for (i, event) in events.iter().enumerate() {
        let field = format!("events[{i}]");
        if !(event.day.is_finite() && event.day >= 0.0) {
            return Err(EcosystemError::config(&field, format!("day {} must be ≥ 0", event.day)));
        }
        if event.day < previous {
            return Err(EcosystemError::config(&field, "events must be listed in order of their day"));
        }
        previous = event.day;
        match &event.action {
            EventAction::Ignite { .. } => {}
            EventAction::World(w) => w.validate().map_err(|reason| EcosystemError::config(&field, reason))?,
            EventAction::Introduce { species, count, region } => {
                catalog.animal_index(species)?;
                if *count == 0 {
                    return Err(EcosystemError::config(&field, "introduces no animals (count 0)"));
                }
                if let Some((x0, y0, x1, y1)) = region
                    && !([x0, y0, x1, y1].iter().all(|v| v.is_finite()) && x0 < x1 && y0 < y1)
                {
                    return Err(EcosystemError::config(&field, "region needs x0 < x1 and y0 < y1"));
                }
            }
            EventAction::Remove { species, fraction } => {
                catalog.animal_index(species)?;
                if !(0.0..=1.0).contains(fraction) {
                    return Err(EcosystemError::config(&field, format!("fraction {fraction} is outside [0, 1]")));
                }
            }
            EventAction::Outbreak { pathogen, species, .. } => {
                let Some(p) = pathogens.iter().find(|p| &p.id == pathogen) else {
                    return Err(EcosystemError::config(&field, format!("unknown pathogen `{pathogen}`")));
                };
                if !p.hosts.contains(species) {
                    return Err(EcosystemError::config(&field, format!("`{species}` is not a host of `{pathogen}`")));
                }
            }
            EventAction::SetParam { key, value } => {
                let spec = specs
                    .iter()
                    .find(|s| &s.key == key)
                    .ok_or_else(|| EcosystemError::config(&field, format!("unknown parameter `{key}`")))?;
                param_value(spec, *value)?;
            }
        }
    }
    Ok(())
}

impl crate::state::EcoState {
    /// Runs the scheduled events due at the current tick (in order). A
    /// failing event (e.g. a disaster outside the map) is reported in the
    /// event log; it does not stop the run.
    pub(crate) fn run_schedule(&mut self, rt: &mut crate::state::Runtime) {
        let ticks_per_day = f64::from(self.world.calendar().ticks_per_day());
        while let Some(event) = self.config.events.get(self.schedule_done).cloned() {
            let due = (f64::from(event.day) * ticks_per_day).round() as u64;
            if due > self.tick {
                break;
            }
            self.schedule_done += 1;
            let kind = event.action.kind();
            let message = match self.run_event(rt, &event.action) {
                Ok(text) => format!("scheduled {kind} (day {}): {text}", event.day),
                Err(e) => format!("scheduled {kind} (day {}) failed: {e}", event.day),
            };
            self.log(
                simulation_core::SimEvent::new(self.tick, "scheduled", message)
                    .severity(simulation_core::EventSeverity::Major),
            );
        }
    }

    fn run_event(&mut self, rt: &mut crate::state::Runtime, action: &EventAction) -> Result<String, EcosystemError> {
        use crate::command::EcosystemCommand;
        match action {
            EventAction::Ignite { x, y } => {
                self.apply_command(rt, EcosystemCommand::Ignite { x: *x, y: *y })?;
                Ok(format!("fire started at ({x}, {y})"))
            }
            EventAction::World(w) => {
                let ticks_per_day = f64::from(self.world.calendar().ticks_per_day());
                let disaster = w.to_disaster(ticks_per_day);
                self.apply_command(rt, EcosystemCommand::Disaster(disaster.clone()))?;
                Ok(format!("{disaster:?}"))
            }
            EventAction::Introduce { species, count, region } => {
                let index = self.catalog.animal_index(species)?;
                self.introduce_booked(rt, index, *count, *region)?;
                Ok(format!("{count} {species} introduced"))
            }
            EventAction::Remove { species, fraction } => {
                let removed = self.remove_fraction(species, *fraction)?;
                Ok(format!("{removed:.0} {species} removed"))
            }
            EventAction::Outbreak { pathogen, species, count } => {
                let infected = self.outbreak(pathogen, species, *count)?;
                Ok(format!("{infected} {species} infected with {pathogen}"))
            }
            EventAction::SetParam { key, value } => {
                let specs = crate::params::param_specs(&self.config.rules);
                let spec = specs
                    .iter()
                    .find(|s| &s.key == key)
                    .ok_or_else(|| EcosystemError::config("events.set_param", format!("unknown parameter `{key}`")))?;
                let v = param_value(spec, *value)?;
                let policy = self.set_param(key, v).map_err(|e| EcosystemError::config(key, e.to_string()))?;
                Ok(format!("{key} = {value} ({})", policy.label()))
            }
        }
    }
}
