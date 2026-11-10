//! Parameter overrides: parsing, coercion against the model's
//! [`ParamSpec`]s and application with the correct [`ApplyPolicy`].

use serde::Serialize;
use simulation_core::params::ParamKind;
use simulation_core::{ApplyPolicy, DynSimulation, ParamSpec, ParamValue};

use crate::error::RegistryError;

/// Key of the parameter through which many models publish (and accept)
/// their random seed.
pub const SEED_PARAM: &str = "seed";

/// A requested parameter change.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamOverride {
    /// Parameter key.
    pub key: String,
    /// Value. Text is converted to the parameter's type (`"0.5"` for a
    /// float, `"true"` for a switch, `"#ff8800"` for a colour); typed
    /// values are used as given.
    pub value: ParamValue,
}

impl ParamOverride {
    /// A typed override.
    pub fn new(key: &str, value: ParamValue) -> Self {
        Self { key: key.to_string(), value }
    }

    /// Parses `key=value` (the value stays text until it is checked
    /// against the model's parameter spec).
    ///
    /// # Errors
    /// [`RegistryError::MalformedOverride`] without `=` or with an empty key.
    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        match text.split_once('=') {
            Some((key, value)) if !key.trim().is_empty() => {
                Ok(Self { key: key.trim().to_string(), value: ParamValue::Text(value.trim().to_string()) })
            }
            _ => Err(RegistryError::MalformedOverride(text.to_string())),
        }
    }
}

/// An override as applied to a model.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppliedOverride {
    /// Parameter key.
    pub key: String,
    /// Validated value.
    pub value: ParamValue,
    /// When the model applies it.
    pub policy: ApplyPolicy,
}

/// Converts `value` to the type of `spec` and validates it (range, length,
/// choice options).
///
/// # Errors
/// [`RegistryError::InvalidParameter`] with the reason.
pub fn coerce(spec: &ParamSpec, value: &ParamValue) -> Result<ParamValue, RegistryError> {
    let invalid = |message: String| RegistryError::InvalidParameter { key: spec.key.clone(), message };
    let converted = match (&spec.kind, value) {
        (ParamKind::Bool, ParamValue::Text(t)) => match t.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => ParamValue::Bool(true),
            "false" | "no" | "off" | "0" => ParamValue::Bool(false),
            _ => return Err(invalid(format!("expected true or false, got `{t}`"))),
        },
        (ParamKind::Int { .. }, ParamValue::Text(t)) => {
            ParamValue::Int(t.trim().parse().map_err(|_| invalid(format!("expected an integer, got `{t}`")))?)
        }
        (ParamKind::Float { .. }, ParamValue::Text(t)) => {
            ParamValue::Float(t.trim().parse().map_err(|_| invalid(format!("expected a number, got `{t}`")))?)
        }
        (ParamKind::Color, ParamValue::Text(t)) => match ParamValue::parse_loose(t) {
            color @ ParamValue::Color(_) => color,
            _ => return Err(invalid(format!("expected a colour such as #ff8800 or #ff8800cc, got `{t}`"))),
        },
        (ParamKind::Text { .. } | ParamKind::Choice { .. }, other) if other.as_str().is_none() => {
            ParamValue::Text(other.to_string())
        }
        (_, other) => other.clone(),
    };
    spec.validate(&converted).map_err(|e| invalid(strip_key_prefix(&e.to_string(), &spec.key)))
}

/// `SimError::InvalidParameter` already names the key; avoid repeating it.
fn strip_key_prefix(message: &str, key: &str) -> String {
    let prefix = format!("invalid value for parameter `{key}`: ");
    message.strip_prefix(&prefix).unwrap_or(message).to_string()
}

fn same_value(a: &ParamValue, b: &ParamValue) -> bool {
    match (a.as_f64(), b.as_f64()) {
        // Models may store floats as f32.
        (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    }
}

/// Applies `overrides` in order before the first tick. Each value is
/// checked against the model's current descriptor (descriptors may change
/// with other parameters). When any parameter requires a reset the model is
/// reset once at the end, the other overrides are applied again (a reset may
/// rebuild the state they live in) and every reset-only parameter is
/// verified to be in effect: numbers, switches, choices and colours must be
/// reported with the value given (the last one when a key is repeated).
/// Text values (rule notations, lists) are accepted by the model's own
/// parser, which may normalise them (`b36s23` becomes `B36/S23`); the
/// applied override then records the value the model reports.
///
/// Returns the applied overrides and whether a reset was performed.
///
/// # Errors
/// Unknown keys, invalid values, values the model rejects and reset-only
/// parameters the model does not report after the reset.
pub fn apply_overrides(
    sim: &mut dyn DynSimulation,
    overrides: &[ParamOverride],
) -> Result<(Vec<AppliedOverride>, bool), RegistryError> {
    let mut applied = Vec::with_capacity(overrides.len());
    for o in overrides {
        let descriptor = sim.descriptor();
        let spec = descriptor.param(&o.key).ok_or_else(|| RegistryError::UnknownParameter {
            model: descriptor.id.clone(),
            key: o.key.clone(),
            known: descriptor.params.iter().map(|p| p.key.clone()).collect(),
        })?;
        let value = coerce(spec, &o.value)?;
        let policy = sim.set_param(&o.key, value.clone()).map_err(|e| RegistryError::InvalidParameter {
            key: o.key.clone(),
            message: strip_key_prefix(&e.to_string(), &o.key),
        })?;
        applied.push(AppliedOverride { key: o.key.clone(), value, policy });
    }
    let needs_reset = applied.iter().any(|a| a.policy == ApplyPolicy::RequiresReset);
    if needs_reset {
        sim.reset();
        for a in applied.iter().filter(|a| a.policy != ApplyPolicy::RequiresReset) {
            sim.set_param(&a.key, a.value.clone()).map_err(|e| RegistryError::InvalidParameter {
                key: a.key.clone(),
                message: format!("rejected after the reset: {}", strip_key_prefix(&e.to_string(), &a.key)),
            })?;
        }
        let params = sim.params();
        let descriptor = sim.descriptor();
        for (i, a) in applied.iter_mut().enumerate() {
            let repeated_later = overrides[i + 1..].iter().any(|o| o.key == a.key);
            if a.policy != ApplyPolicy::RequiresReset || repeated_later {
                continue;
            }
            let Some(current) = params.get(&a.key) else { continue };
            let text = descriptor.param(&a.key).is_some_and(|p| matches!(p.kind, ParamKind::Text { .. }));
            if text {
                a.value = current.clone();
            } else if !same_value(current, &a.value) {
                return Err(RegistryError::InvalidParameter {
                    key: a.key.clone(),
                    message: format!("set to {} but the model reports {current} after the reset", a.value),
                });
            }
        }
    }
    Ok((applied, needs_reset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_requires_key_and_equals() {
        let o = ParamOverride::parse(" rule = B36/S23 ").unwrap();
        assert_eq!(o.key, "rule");
        assert_eq!(o.value, ParamValue::Text("B36/S23".into()));
        assert!(ParamOverride::parse("rule").is_err());
        assert!(ParamOverride::parse("=3").is_err());
    }

    #[test]
    fn coercion_follows_the_spec() {
        let int = ParamSpec::int("width", "Width", 4, 100, 10);
        assert_eq!(coerce(&int, &ParamValue::Text("12".into())).unwrap(), ParamValue::Int(12));
        let err = coerce(&int, &ParamValue::Text("1.5".into())).unwrap_err().to_string();
        assert!(err.contains("expected an integer"), "{err}");
        let err = coerce(&int, &ParamValue::Text("400".into())).unwrap_err().to_string();
        assert_eq!(err, "parameter `width`: 400 is outside [4, 100]");
        let float = ParamSpec::float("rate", "Rate", 0.0, 1.0, 0.5);
        assert_eq!(coerce(&float, &ParamValue::Text("0.25".into())).unwrap(), ParamValue::Float(0.25));
        assert_eq!(coerce(&float, &ParamValue::Int(1)).unwrap(), ParamValue::Float(1.0));
        let text = ParamSpec::text("rule", "Rule", 32, "B3/S23");
        assert_eq!(coerce(&text, &ParamValue::Int(23)).unwrap(), ParamValue::Text("23".into()));
        let flag = ParamSpec::boolean("wrap", "Wrap", true);
        assert_eq!(coerce(&flag, &ParamValue::Text("off".into())).unwrap(), ParamValue::Bool(false));
        let choice = ParamSpec::choice("t", "T", &[("torus", "Torus"), ("plane", "Plane")], "torus");
        assert!(coerce(&choice, &ParamValue::Text("sphere".into())).unwrap_err().to_string().contains("torus"));
        let color = ParamSpec::color("c", "C", [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            coerce(&color, &ParamValue::Text("#ff0000".into())).unwrap(),
            ParamValue::Color([1.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn float_comparison_tolerates_f32_storage() {
        assert!(same_value(&ParamValue::Float(0.1), &ParamValue::Float(f64::from(0.1f32))));
        assert!(!same_value(&ParamValue::Float(0.1), &ParamValue::Float(0.2)));
        assert!(same_value(&ParamValue::Text("a".into()), &ParamValue::Text("a".into())));
    }
}
