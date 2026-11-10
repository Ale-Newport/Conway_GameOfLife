//! What a metapixel construction computes: the macro rule and the macro
//! neighbourhood that every tile is programmed with.

use std::fmt;

use patterns::LifeRule;
use serde::{Deserialize, Serialize};

use crate::error::MetapixelError;
use crate::spec::Direction;

/// The set of neighbour directions every tile *listens to*.
///
/// The default is Moore (all eight). Removing a direction removes the
/// matching fuse object from every tile (see
/// [`Direction::fuse_silencing`]), so every tile becomes deaf to that
/// neighbour. Bit `i` of the mask is [`Direction::ALL`]`[i]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Neighbourhood(u8);

impl Neighbourhood {
    /// All eight neighbours.
    pub const fn moore() -> Self {
        Self(0xFF)
    }

    /// The four orthogonal neighbours (corner LWSSs removed).
    pub const fn von_neumann() -> Self {
        Self(0b0101_0101)
    }

    /// The neighbourhood listening to exactly `directions`.
    pub fn from_directions(directions: impl IntoIterator<Item = Direction>) -> Self {
        Self(directions.into_iter().fold(0, |mask, d| mask | 1 << d.index()))
    }

    /// Raw mask (bit `i` = [`Direction::ALL`]`[i]`).
    pub fn mask(self) -> u8 {
        self.0
    }

    /// Whether tiles listen to their neighbour in direction `d`.
    pub fn listens(self, d: Direction) -> bool {
        self.0 >> d.index() & 1 == 1
    }

    /// Listened directions, clockwise from north.
    pub fn directions(self) -> impl Iterator<Item = Direction> {
        Direction::ALL.into_iter().filter(move |&d| self.listens(d))
    }

    /// Number of listened directions (the largest possible neighbour count).
    pub fn size(self) -> u32 {
        self.0.count_ones()
    }

    /// Parses `moore`, `von_neumann` (also `von-neumann`, `vonneumann`,
    /// `vn`), or a comma/space separated list of listened directions such as
    /// `N, E, S, W, NE`. Case is ignored; an empty list is accepted (tiles
    /// then never hear anything).
    ///
    /// # Errors
    /// [`MetapixelError::InvalidNeighbourhood`] for unknown words or
    /// repeated directions.
    pub fn parse(text: &str) -> Result<Self, MetapixelError> {
        let fail = |message: String| MetapixelError::InvalidNeighbourhood { text: text.to_string(), message };
        let trimmed = text.trim();
        match trimmed.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "moore" => return Ok(Self::moore()),
            "von_neumann" | "vonneumann" | "vn" => return Ok(Self::von_neumann()),
            "none" => return Ok(Self(0)),
            _ => {}
        }
        let mut mask = 0u8;
        for word in trimmed.split([',', ' ', ';']).filter(|w| !w.is_empty()) {
            let d = Direction::parse(word).ok_or_else(|| {
                fail(format!(
                    "`{word}` is not a direction; use moore, von_neumann, none, or a list of N, NE, E, SE, S, SW, W, NW"
                ))
            })?;
            if mask >> d.index() & 1 == 1 {
                return Err(fail(format!("direction {} is listed twice", d.name())));
            }
            mask |= 1 << d.index();
        }
        Ok(Self(mask))
    }
}

impl Default for Neighbourhood {
    fn default() -> Self {
        Self::moore()
    }
}

impl fmt::Display for Neighbourhood {
    /// `moore`, `von_neumann`, `none`, or the listened directions (`N,E,SW`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::moore() {
            return f.write_str("moore");
        }
        if *self == Self::von_neumann() {
            return f.write_str("von_neumann");
        }
        if self.0 == 0 {
            return f.write_str("none");
        }
        let names: Vec<&str> = self.directions().map(Direction::name).collect();
        f.write_str(&names.join(","))
    }
}

/// Rule and neighbourhood programmed identically into every tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Programming {
    /// Macro rule: a two-state outer-totalistic rule (`B0` allowed, see the
    /// crate documentation on finite grids).
    #[serde(with = "rule_text")]
    pub rule: LifeRule,
    /// Directions every tile listens to.
    pub neighbourhood: Neighbourhood,
}

impl Programming {
    /// Conway's Life on the Moore neighbourhood.
    pub fn conway() -> Self {
        Self { rule: LifeRule::conway(), neighbourhood: Neighbourhood::moore() }
    }

    /// Parses a rule (`B3/S23`, `23/3`, `b3s23`) and a neighbourhood (see
    /// [`Neighbourhood::parse`]).
    ///
    /// # Errors
    /// [`MetapixelError::InvalidRule`] or
    /// [`MetapixelError::InvalidNeighbourhood`].
    pub fn parse(rule: &str, neighbourhood: &str) -> Result<Self, MetapixelError> {
        Ok(Self { rule: parse_rule(rule)?, neighbourhood: Neighbourhood::parse(neighbourhood)? })
    }
}

impl Default for Programming {
    fn default() -> Self {
        Self::conway()
    }
}

/// Parses a two-state Life-like rule for the macro level.
///
/// # Errors
/// [`MetapixelError::InvalidRule`] with the parser's explanation.
pub fn parse_rule(text: &str) -> Result<LifeRule, MetapixelError> {
    LifeRule::parse(text).map_err(|e| MetapixelError::InvalidRule { rule: text.to_string(), message: e.to_string() })
}

/// Serialises a [`LifeRule`] as its canonical `B…/S…` text (re-validated on
/// load, so a snapshot cannot carry an invalid mask).
mod rule_text {
    use patterns::LifeRule;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(rule: &LifeRule, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&rule.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LifeRule, D::Error> {
        let text = String::deserialize(deserializer)?;
        LifeRule::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbourhood_parsing_and_display_round_trip() {
        assert_eq!(Neighbourhood::parse("Moore").unwrap(), Neighbourhood::moore());
        assert_eq!(Neighbourhood::parse("von-Neumann").unwrap(), Neighbourhood::von_neumann());
        assert_eq!(Neighbourhood::parse(" vn ").unwrap(), Neighbourhood::von_neumann());
        let custom = Neighbourhood::parse("n, e,SW").unwrap();
        assert_eq!(custom.to_string(), "N,E,SW");
        assert_eq!(Neighbourhood::parse(&custom.to_string()).unwrap(), custom);
        assert_eq!(custom.size(), 3);
        assert!(Neighbourhood::parse("N,N").is_err());
        assert!(Neighbourhood::parse("up").is_err());
        assert_eq!(Neighbourhood::parse("none").unwrap().size(), 0);
        assert_eq!(
            Neighbourhood::von_neumann().directions().map(Direction::name).collect::<Vec<_>>(),
            ["N", "E", "S", "W"]
        );
    }

    #[test]
    fn programming_parses_rules_including_b0() {
        let p = Programming::parse("b36s23", "moore").unwrap();
        assert_eq!(p.rule.to_string(), "B36/S23");
        assert!(Programming::parse("B0/S", "moore").unwrap().rule.has_b0());
        assert!(matches!(Programming::parse("B9/S", "moore"), Err(MetapixelError::InvalidRule { .. })));
    }
}
