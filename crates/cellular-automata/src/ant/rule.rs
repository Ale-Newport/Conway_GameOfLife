//! Turmite rules (`RL`, `LLRR`...), turns and headings.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CaError;

/// Largest number of colours of a turmite rule.
pub const MAX_COLORS: usize = 16;

/// A turn applied on a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Turn {
    /// 90° counter-clockwise.
    Left,
    /// 90° clockwise.
    Right,
    /// Straight on.
    None,
    /// 180°.
    UTurn,
}

/// Direction an ant faces (`y` grows downward, so north is `(0, −1)`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heading {
    /// Up.
    #[default]
    North,
    /// Right.
    East,
    /// Down.
    South,
    /// Left.
    West,
}

impl Heading {
    const ORDER: [Heading; 4] = [Self::North, Self::East, Self::South, Self::West];

    fn index(self) -> usize {
        self as usize
    }

    /// The heading after `turn`.
    pub fn turned(self, turn: Turn) -> Self {
        let quarter = match turn {
            Turn::None => 0,
            Turn::Right => 1,
            Turn::UTurn => 2,
            Turn::Left => 3,
        };
        Self::ORDER[(self.index() + quarter) % 4]
    }

    /// Unit step `(dx, dy)`.
    pub fn delta(self) -> (i64, i64) {
        match self {
            Self::North => (0, -1),
            Self::East => (1, 0),
            Self::South => (0, 1),
            Self::West => (-1, 0),
        }
    }
}

/// A turmite rule: one turn per colour; colour `i` becomes `(i + 1) mod n`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TurmiteRule {
    turns: Vec<Turn>,
}

impl TurmiteRule {
    /// Langton's ant, `RL`.
    pub fn langton() -> Self {
        Self { turns: vec![Turn::Right, Turn::Left] }
    }

    /// Parses a string of `L`, `R`, `N`, `U` (case-insensitive), 2 to 16 letters.
    ///
    /// # Errors
    /// Other characters or a bad length, with the offending position.
    pub fn parse(text: &str) -> Result<Self, CaError> {
        let text = text.trim();
        let turns = text
            .chars()
            .enumerate()
            .map(|(i, c)| match c.to_ascii_uppercase() {
                'L' => Ok(Turn::Left),
                'R' => Ok(Turn::Right),
                'N' => Ok(Turn::None),
                'U' => Ok(Turn::UTurn),
                other => Err(CaError::invalid(format!(
                    "turmite rule `{text}`: character {} is `{other}`; use L, R, N (no turn) or U (U-turn)",
                    i + 1
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !(2..=MAX_COLORS).contains(&turns.len()) {
            return Err(CaError::invalid(format!(
                "turmite rule `{text}` has {} letters; it needs 2 to {MAX_COLORS} (one per colour)",
                turns.len()
            )));
        }
        Ok(Self { turns })
    }

    /// Number of colours.
    pub fn colors(&self) -> u8 {
        self.turns.len() as u8
    }

    /// Turn on `color`.
    pub fn turn(&self, color: u8) -> Turn {
        self.turns[usize::from(color) % self.turns.len()]
    }
}

impl fmt::Display for TurmiteRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for t in &self.turns {
            let c = match t {
                Turn::Left => 'L',
                Turn::Right => 'R',
                Turn::None => 'N',
                Turn::UTurn => 'U',
            };
            write!(f, "{c}")?;
        }
        Ok(())
    }
}
