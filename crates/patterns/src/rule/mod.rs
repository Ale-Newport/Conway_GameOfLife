//! Rule notation for two-dimensional outer-totalistic cellular automata.
//!
//! * [`LifeRule`] — two-state Life-like rules on the Moore neighbourhood
//!   (`B3/S23`, `b3s23`, `S23/B3`, the old MCell/Life 1.05 form `23/3`, and
//!   well-known names such as `HighLife`).
//! * [`GenerationsRule`] — multi-state "Generations" rules (`/2/3`, `345/2/4`,
//!   `B2/S/C3`, `B2/S345/C4`, Catagolue's `g3b2s`).
//! * [`BoundedGrid`] — Golly's bounded-grid suffix (`:T100,100`, `:P64,64`,
//!   `:K30*,20`, `:C30,20`, `:S30`), mapped to [`simulation_core::Topology`]
//!   where the topology exists there.
//! * [`Rule`] — either kind, as found in RLE headers.
//!
//! Parsing is case-insensitive and ignores whitespace. Non-totalistic
//! (isotropic Hensel) letters, hexagonal/von Neumann neighbourhood suffixes and
//! `MAP` rules are rejected with an explicit message.

mod bounded;
mod generations;
mod life;
mod notation;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use bounded::{BoundedGrid, GridKind, MAX_GRID_SIZE};
pub use generations::GenerationsRule;
pub use life::LifeRule;

/// A rejected rule string.
///
/// `column` is the 1-based character position in the string that was passed
/// to the parser, when the error can be pinned to one character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    /// What went wrong.
    pub message: String,
    /// 1-based character column of the offending character, if known.
    pub column: Option<usize>,
}

impl RuleError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), column: None }
    }

    pub(crate) fn at(column: usize, message: impl Into<String>) -> Self {
        Self { message: message.into(), column: Some(column) }
    }
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.column {
            Some(column) => write!(f, "{} (at column {column})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for RuleError {}

impl From<RuleError> for simulation_core::SimError {
    fn from(error: RuleError) -> Self {
        simulation_core::SimError::Parse(error.to_string())
    }
}

/// Either kind of rule supported by the notation parsers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    /// Two-state Life-like rule.
    Life(LifeRule),
    /// Multi-state Generations rule with at least three states (a
    /// Generations rule with two states is the Life-like rule it names and is
    /// returned as [`Rule::Life`]).
    Generations(GenerationsRule),
}

impl Rule {
    /// Parses a Life-like or Generations rule. A bounded-grid suffix is
    /// validated and discarded; use [`Rule::parse_with_grid`] to keep it.
    pub fn parse(text: &str) -> Result<Self, RuleError> {
        Self::parse_with_grid(text).map(|(rule, _)| rule)
    }

    /// Parses a rule and its optional bounded-grid suffix. The grid is `None`
    /// when there is no suffix or the suffix describes an unbounded universe
    /// (`":"`, `":T0,0"`).
    pub fn parse_with_grid(text: &str) -> Result<(Self, Option<BoundedGrid>), RuleError> {
        let (body, _) = notation::split_suffix(text);
        if notation::looks_like_generations(body) {
            let (rule, grid) = GenerationsRule::parse_with_grid(text)?;
            Ok((Self::from(rule), grid))
        } else {
            let (rule, grid) = LifeRule::parse_with_grid(text)?;
            Ok((Self::Life(rule), grid))
        }
    }

    /// Number of cell states (2 for Life-like rules).
    pub fn states(&self) -> u8 {
        match self {
            Self::Life(_) => 2,
            Self::Generations(rule) => rule.states,
        }
    }

    /// Birth mask (bit *n* = birth with *n* live neighbours).
    pub fn birth(&self) -> u16 {
        match self {
            Self::Life(rule) => rule.birth,
            Self::Generations(rule) => rule.birth,
        }
    }

    /// Survival mask (bit *n* = survival with *n* live neighbours).
    pub fn survive(&self) -> u16 {
        match self {
            Self::Life(rule) => rule.survive,
            Self::Generations(rule) => rule.survive,
        }
    }

    /// Well-known name of the rule, if it has one.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Self::Life(rule) => rule.name(),
            Self::Generations(rule) => rule.name(),
        }
    }
}

impl From<LifeRule> for Rule {
    fn from(rule: LifeRule) -> Self {
        Self::Life(rule)
    }
}

impl From<GenerationsRule> for Rule {
    fn from(rule: GenerationsRule) -> Self {
        if rule.states == 2 {
            Self::Life(LifeRule { birth: rule.birth, survive: rule.survive })
        } else {
            Self::Generations(rule)
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Life(rule) => rule.fmt(f),
            Self::Generations(rule) => rule.fmt(f),
        }
    }
}

impl FromStr for Rule {
    type Err = RuleError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Implements string-based serde for a rule type: rules are stored in
/// scenario files as their canonical notation (`"B3/S23"`) and parsed back
/// with full validation.
macro_rules! serde_as_string {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

serde_as_string!(Rule);
serde_as_string!(LifeRule);
serde_as_string!(GenerationsRule);
serde_as_string!(BoundedGrid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_between_life_and_generations() {
        assert_eq!(Rule::parse("B3/S23").unwrap(), Rule::Life(LifeRule::conway()));
        assert_eq!(Rule::parse("23/3").unwrap(), Rule::Life(LifeRule::conway()));
        assert_eq!(Rule::parse("/2/3").unwrap(), Rule::Generations(GenerationsRule::brians_brain()));
        assert_eq!(Rule::parse("B2/S/C3").unwrap(), Rule::Generations(GenerationsRule::brians_brain()));
        assert_eq!(Rule::parse("Brian's Brain").unwrap(), Rule::Generations(GenerationsRule::brians_brain()));
        assert_eq!(Rule::parse("HighLife").unwrap(), Rule::Life(LifeRule::highlife()));
        // A two-state Generations rule is the Life-like rule it names.
        assert_eq!(Rule::parse("23/3/2").unwrap(), Rule::Life(LifeRule::conway()));
        assert_eq!(Rule::parse("B3/S23/C2").unwrap().states(), 2);
    }

    #[test]
    fn keeps_grid_suffix() {
        let (rule, grid) = Rule::parse_with_grid("345/2/4:T64,32").unwrap();
        assert_eq!(rule, Rule::Generations(GenerationsRule::star_wars()));
        let grid = grid.unwrap();
        assert_eq!((grid.kind, grid.width, grid.height), (GridKind::Torus, 64, 32));
        assert_eq!(rule.to_string(), "345/2/4");
    }

    #[test]
    fn reports_errors_with_columns() {
        let err = Rule::parse("B39/S23").unwrap_err();
        assert_eq!(err.column, Some(3));
        assert_eq!(err.to_string(), "digit 9 is not a valid neighbour count (0-8) (at column 3)");
        assert!(Rule::parse("").is_err());
    }

    #[test]
    fn serde_uses_canonical_strings() {
        let rule: Rule = serde_json::from_str("\"b36s23\"").unwrap();
        assert_eq!(rule, Rule::Life(LifeRule::highlife()));
        assert_eq!(serde_json::to_string(&rule).unwrap(), "\"B36/S23\"");
        let grid: BoundedGrid = serde_json::from_str("\":t10,20\"").unwrap();
        assert_eq!(serde_json::to_string(&grid).unwrap(), "\":T10,20\"");
        assert!(serde_json::from_str::<LifeRule>("\"B9/S\"").is_err());
    }
}
