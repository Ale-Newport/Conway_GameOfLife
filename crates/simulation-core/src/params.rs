//! Typed, self-describing simulation parameters.
//!
//! Every model publishes a list of [`ParamSpec`]s. The desktop interface builds
//! its property panels from these specs and the command-line tool validates
//! `--param key=value` overrides against them, so a parameter only has to be
//! declared once. Each spec states *when* a change takes effect through its
//! [`ApplyPolicy`].

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::SimError;

/// When a parameter change becomes effective.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPolicy {
    /// Takes effect immediately, even in the middle of an interaction.
    Immediate,
    /// Takes effect at the start of the next tick (the default for rules).
    #[default]
    NextTick,
    /// Only used when the simulation is (re)initialised, e.g. world size.
    RequiresReset,
}

impl ApplyPolicy {
    /// Short human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Immediate => "applies immediately",
            Self::NextTick => "applies next tick",
            Self::RequiresReset => "requires reset",
        }
    }
}

/// A parameter value. Serialised untagged so scenario files stay readable:
/// `true`, `12`, `0.5`, `"B3/S23"`, `[1.0, 0.5, 0.0, 1.0]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Color([f32; 4]),
}

impl ParamValue {
    /// Numeric view of the value (integers widen to floats).
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Int(i) => Some(i as f64),
            Self::Float(f) => Some(f),
            _ => None,
        }
    }

    /// Integer view of the value (floats are accepted only when integral).
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Int(i) => Some(i),
            Self::Float(f) if f.fract() == 0.0 && f.is_finite() => Some(f as i64),
            _ => None,
        }
    }

    /// Boolean view of the value.
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Self::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// String view of the value (text and choice parameters).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Colour view of the value.
    pub fn as_color(&self) -> Option<[f32; 4]> {
        match *self {
            Self::Color(c) => Some(c),
            _ => None,
        }
    }

    /// Parses a command-line style string (`true`, `3`, `0.25`, `#ff8800`,
    /// anything else is text) — used by `--param key=value` overrides.
    pub fn parse_loose(input: &str) -> Self {
        let trimmed = input.trim();
        if let Ok(b) = trimmed.parse::<bool>() {
            return Self::Bool(b);
        }
        if let Ok(i) = trimmed.parse::<i64>() {
            return Self::Int(i);
        }
        if let Ok(f) = trimmed.parse::<f64>() {
            return Self::Float(f);
        }
        if let Some(hex) = trimmed.strip_prefix('#')
            && let Some(c) = parse_hex_color(hex)
        {
            return Self::Color(c);
        }
        Self::Text(trimmed.to_string())
    }
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    match hex.len() {
        6 => Some([channel(0)?, channel(2)?, channel(4)?, 1.0]),
        8 => Some([channel(0)?, channel(2)?, channel(4)?, channel(6)?]),
        _ => None,
    }
}

impl fmt::Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(x) => write!(f, "{x}"),
            Self::Text(s) => write!(f, "{s}"),
            Self::Color([r, g, b, a]) => write!(f, "rgba({r:.3}, {g:.3}, {b:.3}, {a:.3})"),
        }
    }
}

/// One option of a [`ParamKind::Choice`] parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// Stored value.
    pub value: String,
    /// Label shown to users.
    pub label: String,
}

/// Type and admissible range of a parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamKind {
    Bool,
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64, step: f64, logarithmic: bool },
    Text { max_len: usize },
    Choice { options: Vec<ChoiceOption> },
    Color,
}

/// Declaration of a single parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    /// Stable machine key, e.g. `"birth_rate"`.
    pub key: String,
    /// Label for user interfaces.
    pub label: String,
    /// One or two sentences explaining the effect of the parameter.
    pub description: String,
    /// Grouping used to organise property panels (e.g. "Rules", "World").
    pub group: String,
    /// Type and range.
    pub kind: ParamKind,
    /// Default value (always valid for `kind`).
    pub default: ParamValue,
    /// When a change takes effect.
    pub apply: ApplyPolicy,
}

impl ParamSpec {
    fn new(key: &str, label: &str, kind: ParamKind, default: ParamValue) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            description: String::new(),
            group: "General".to_string(),
            kind,
            default,
            apply: ApplyPolicy::NextTick,
        }
    }

    /// Floating point parameter in `[min, max]`.
    pub fn float(key: &str, label: &str, min: f64, max: f64, default: f64) -> Self {
        let step = ((max - min) / 1000.0).max(f64::EPSILON);
        Self::new(key, label, ParamKind::Float { min, max, step, logarithmic: false }, ParamValue::Float(default))
    }

    /// Integer parameter in `[min, max]`.
    pub fn int(key: &str, label: &str, min: i64, max: i64, default: i64) -> Self {
        Self::new(key, label, ParamKind::Int { min, max }, ParamValue::Int(default))
    }

    /// Boolean switch.
    pub fn boolean(key: &str, label: &str, default: bool) -> Self {
        Self::new(key, label, ParamKind::Bool, ParamValue::Bool(default))
    }

    /// Free text (e.g. a rule string) limited to `max_len` characters.
    pub fn text(key: &str, label: &str, max_len: usize, default: &str) -> Self {
        Self::new(key, label, ParamKind::Text { max_len }, ParamValue::Text(default.to_string()))
    }

    /// Choice among `(value, label)` options.
    pub fn choice(key: &str, label: &str, options: &[(&str, &str)], default: &str) -> Self {
        let options = options
            .iter()
            .map(|(value, label)| ChoiceOption { value: (*value).to_string(), label: (*label).to_string() })
            .collect();
        Self::new(key, label, ParamKind::Choice { options }, ParamValue::Text(default.to_string()))
    }

    /// RGBA colour.
    pub fn color(key: &str, label: &str, default: [f32; 4]) -> Self {
        Self::new(key, label, ParamKind::Color, ParamValue::Color(default))
    }

    /// Sets the description.
    pub fn describe(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    /// Sets the panel group.
    pub fn group(mut self, group: &str) -> Self {
        self.group = group.to_string();
        self
    }

    /// Sets the apply policy.
    pub fn apply(mut self, apply: ApplyPolicy) -> Self {
        self.apply = apply;
        self
    }

    /// Sets the slider step of a float parameter (no-op for other kinds).
    pub fn step(mut self, step: f64) -> Self {
        if let ParamKind::Float { step: s, .. } = &mut self.kind {
            *s = step;
        }
        self
    }

    /// Marks a float parameter as best edited on a logarithmic scale.
    pub fn logarithmic(mut self) -> Self {
        if let ParamKind::Float { logarithmic, .. } = &mut self.kind {
            *logarithmic = true;
        }
        self
    }

    /// Validates (and normalises) a value against this spec. Integers are
    /// accepted for float parameters; out-of-range numbers are rejected rather
    /// than silently clamped so that typos in scenario files are visible.
    pub fn validate(&self, value: &ParamValue) -> Result<ParamValue, SimError> {
        let err = |reason: String| SimError::invalid_param(&self.key, reason);
        match &self.kind {
            ParamKind::Bool => {
                value.as_bool().map(ParamValue::Bool).ok_or_else(|| err(format!("expected true/false, got {value}")))
            }
            ParamKind::Int { min, max } => {
                let v = value.as_i64().ok_or_else(|| err(format!("expected an integer, got {value}")))?;
                if v < *min || v > *max {
                    return Err(err(format!("{v} is outside [{min}, {max}]")));
                }
                Ok(ParamValue::Int(v))
            }
            ParamKind::Float { min, max, .. } => {
                let v = value.as_f64().ok_or_else(|| err(format!("expected a number, got {value}")))?;
                if !v.is_finite() {
                    return Err(err("value must be finite".to_string()));
                }
                if v < *min || v > *max {
                    return Err(err(format!("{v} is outside [{min}, {max}]")));
                }
                Ok(ParamValue::Float(v))
            }
            ParamKind::Text { max_len } => {
                let s = value.as_str().ok_or_else(|| err(format!("expected text, got {value}")))?;
                if s.chars().count() > *max_len {
                    return Err(err(format!("text longer than {max_len} characters")));
                }
                Ok(ParamValue::Text(s.to_string()))
            }
            ParamKind::Choice { options } => {
                let s = value.as_str().ok_or_else(|| err(format!("expected one of the options, got {value}")))?;
                if options.iter().any(|o| o.value == s) {
                    Ok(ParamValue::Text(s.to_string()))
                } else {
                    let names: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
                    Err(err(format!("`{s}` is not one of {}", names.join(", "))))
                }
            }
            ParamKind::Color => value
                .as_color()
                .map(ParamValue::Color)
                .ok_or_else(|| err(format!("expected an RGBA colour, got {value}"))),
        }
    }
}

/// A set of parameter values keyed by parameter key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamSet(BTreeMap<String, ParamValue>);

impl ParamSet {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set populated with the defaults of every spec.
    pub fn from_defaults(specs: &[ParamSpec]) -> Self {
        Self(specs.iter().map(|s| (s.key.clone(), s.default.clone())).collect())
    }

    /// Inserts or replaces a value without validation.
    pub fn set(&mut self, key: impl Into<String>, value: ParamValue) {
        self.0.insert(key.into(), value);
    }

    /// Builder-style [`ParamSet::set`].
    pub fn with(mut self, key: impl Into<String>, value: ParamValue) -> Self {
        self.set(key, value);
        self
    }

    /// Raw value lookup.
    pub fn get(&self, key: &str) -> Option<&ParamValue> {
        self.0.get(key)
    }

    /// Float lookup with fallback.
    pub fn f64_or(&self, key: &str, fallback: f64) -> f64 {
        self.get(key).and_then(ParamValue::as_f64).unwrap_or(fallback)
    }

    /// Float lookup with fallback, narrowed to `f32`.
    pub fn f32_or(&self, key: &str, fallback: f32) -> f32 {
        self.get(key).and_then(ParamValue::as_f64).map_or(fallback, |v| v as f32)
    }

    /// Integer lookup with fallback.
    pub fn i64_or(&self, key: &str, fallback: i64) -> i64 {
        self.get(key).and_then(ParamValue::as_i64).unwrap_or(fallback)
    }

    /// Boolean lookup with fallback.
    pub fn bool_or(&self, key: &str, fallback: bool) -> bool {
        self.get(key).and_then(ParamValue::as_bool).unwrap_or(fallback)
    }

    /// Text lookup with fallback.
    pub fn str_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).and_then(ParamValue::as_str).unwrap_or(fallback)
    }

    /// Iterates over `(key, value)` pairs in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ParamValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Overlays `other` on top of `self` (values in `other` win).
    pub fn merge(&mut self, other: &ParamSet) {
        for (k, v) in &other.0 {
            self.0.insert(k.clone(), v.clone());
        }
    }

    /// Validates every value against `specs`, returning a normalised set that
    /// contains defaults for missing keys. Unknown keys are errors.
    pub fn resolve(&self, specs: &[ParamSpec]) -> Result<ParamSet, Vec<SimError>> {
        let mut out = ParamSet::from_defaults(specs);
        let mut errors = Vec::new();
        for (key, value) in &self.0 {
            match specs.iter().find(|s| &s.key == key) {
                Some(spec) => match spec.validate(value) {
                    Ok(v) => out.set(key.clone(), v),
                    Err(e) => errors.push(e),
                },
                None => errors.push(SimError::UnknownParameter(key.clone())),
            }
        }
        if errors.is_empty() { Ok(out) } else { Err(errors) }
    }
}

impl FromIterator<(String, ParamValue)> for ParamSet {
    fn from_iter<T: IntoIterator<Item = (String, ParamValue)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec::float("density", "Density", 0.0, 1.0, 0.3),
            ParamSpec::int("width", "Width", 4, 8192, 256).apply(ApplyPolicy::RequiresReset),
            ParamSpec::choice("topology", "Topology", &[("torus", "Torus"), ("bounded", "Bounded")], "torus"),
            ParamSpec::text("rule", "Rule", 64, "B3/S23"),
        ]
    }

    #[test]
    fn defaults_and_lookup() {
        let set = ParamSet::from_defaults(&specs());
        assert_eq!(set.f64_or("density", 0.0), 0.3);
        assert_eq!(set.i64_or("width", 0), 256);
        assert_eq!(set.str_or("topology", ""), "torus");
    }

    #[test]
    fn resolve_validates_and_fills_defaults() {
        let input =
            ParamSet::new().with("density", ParamValue::Int(1)).with("rule", ParamValue::Text("B36/S23".into()));
        let resolved = input.resolve(&specs()).unwrap();
        assert_eq!(resolved.get("density"), Some(&ParamValue::Float(1.0)));
        assert_eq!(resolved.str_or("rule", ""), "B36/S23");
        assert_eq!(resolved.i64_or("width", 0), 256);
    }

    #[test]
    fn resolve_reports_every_error() {
        let input = ParamSet::new()
            .with("density", ParamValue::Float(2.0))
            .with("topology", ParamValue::Text("sphere".into()))
            .with("nope", ParamValue::Bool(true));
        let errors = input.resolve(&specs()).unwrap_err();
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn untagged_serialisation_is_readable() {
        let set = ParamSet::new()
            .with("a", ParamValue::Bool(true))
            .with("b", ParamValue::Int(3))
            .with("c", ParamValue::Float(0.5))
            .with("d", ParamValue::Text("x".into()));
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"{"a":true,"b":3,"c":0.5,"d":"x"}"#);
        let back: ParamSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, set);
    }

    #[test]
    fn loose_parsing() {
        assert_eq!(ParamValue::parse_loose("true"), ParamValue::Bool(true));
        assert_eq!(ParamValue::parse_loose("12"), ParamValue::Int(12));
        assert_eq!(ParamValue::parse_loose("0.5"), ParamValue::Float(0.5));
        assert_eq!(ParamValue::parse_loose("#ff0000"), ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(ParamValue::parse_loose("B3/S23"), ParamValue::Text("B3/S23".into()));
    }
}
