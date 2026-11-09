//! Two-state outer-totalistic Moore-neighbourhood rules (`B.../S...`).
//!
//! The crate is standalone (it must not depend on `patterns`), so it carries
//! its own small parser for the rule notations Hashlife can execute.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::HashlifeError;

/// Bits 0..=8 of a birth or survival mask (neighbour counts 0 to 8).
const COUNT_MASK: u16 = 0x1FF;

/// A two-state outer-totalistic rule on the Moore neighbourhood.
///
/// Bit `n` of [`Rule::birth`] is set when a dead cell with exactly `n` live
/// neighbours becomes alive; bit `n` of [`Rule::survive`] is set when a live
/// cell with `n` live neighbours stays alive. Rules with `B0` are rejected,
/// because Hashlife needs empty space to stay empty.
///
/// With serde a rule is (de)serialised as its canonical text (`"B3/S23"`), and
/// deserialisation goes through [`Rule::parse`]: a scenario or snapshot file
/// cannot smuggle in a `B0` rule or bits above 8, which the engine would
/// otherwise run with wrong results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rule {
    birth: u16,
    survive: u16,
}

impl Rule {
    /// Conway's Game of Life, `B3/S23`.
    pub const fn conway() -> Self {
        Self { birth: 1 << 3, survive: (1 << 2) | (1 << 3) }
    }

    /// HighLife, `B36/S23` (has a replicator).
    pub const fn highlife() -> Self {
        Self { birth: (1 << 3) | (1 << 6), survive: (1 << 2) | (1 << 3) }
    }

    /// Builds a rule from masks (bit `n` = neighbour count `n`).
    ///
    /// # Errors
    /// [`HashlifeError::InvalidRule`] if a bit above 8 is set, and
    /// [`HashlifeError::BirthOnZero`] if bit 0 of `birth` is set.
    pub fn new(birth: u16, survive: u16) -> Result<Self, HashlifeError> {
        let rule = Self { birth, survive };
        if (birth | survive) & !COUNT_MASK != 0 {
            return Err(HashlifeError::InvalidRule {
                rule: format!("birth mask {birth:#06x}, survive mask {survive:#06x}"),
                reason: "only bits 0 to 8 may be set (a Moore neighbourhood has at most 8 live neighbours)".into(),
            });
        }
        if birth & 1 != 0 {
            return Err(HashlifeError::BirthOnZero { rule: rule.to_string() });
        }
        Ok(rule)
    }

    /// Birth mask: bit `n` set when a dead cell with `n` live neighbours is born.
    pub fn birth(self) -> u16 {
        self.birth
    }

    /// Survival mask: bit `n` set when a live cell with `n` live neighbours survives.
    pub fn survive(self) -> u16 {
        self.survive
    }

    /// Reference transition function: the next state of a cell that is
    /// `alive` and has `neighbours` live neighbours (0 to 8).
    pub fn next_state(self, alive: bool, neighbours: u32) -> bool {
        let mask = if alive { self.survive } else { self.birth };
        neighbours <= 8 && mask >> neighbours & 1 == 1
    }

    /// Parses a rule string.
    ///
    /// Accepted forms (case-insensitive, surrounding whitespace ignored):
    /// * `B3/S23`, `B3S23`, `b36/s23`, `S23/B3` — birth and survival digits;
    /// * `23/3` — the legacy `survival/birth` notation;
    /// * `Life` — alias for `B3/S23`.
    ///
    /// # Errors
    /// [`HashlifeError::InvalidRule`] for anything else (non-totalistic
    /// letters, bounded-grid suffixes, digit 9, ...), and
    /// [`HashlifeError::BirthOnZero`] for rules containing `B0`.
    pub fn parse(text: &str) -> Result<Self, HashlifeError> {
        let trimmed = text.trim();
        let invalid =
            |reason: &str| HashlifeError::InvalidRule { rule: trimmed.to_string(), reason: reason.to_string() };
        if trimmed.is_empty() {
            return Err(invalid("the rule is empty; expected something like B3/S23"));
        }
        if trimmed.eq_ignore_ascii_case("life") {
            return Ok(Self::conway());
        }
        if trimmed.contains(':') {
            return Err(invalid(
                "bounded-grid suffixes (`:T...`, `:P...`) are not supported: the Hashlife plane is unbounded",
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        let (birth, survive) = if lower.starts_with('b') || lower.starts_with('s') {
            parse_bs(&lower).map_err(|reason| invalid(&reason))?
        } else {
            parse_legacy(&lower).map_err(|reason| invalid(&reason))?
        };
        if birth & 1 != 0 {
            return Err(HashlifeError::BirthOnZero { rule: trimmed.to_string() });
        }
        Ok(Self { birth, survive })
    }
}

impl Default for Rule {
    fn default() -> Self {
        Self::conway()
    }
}

/// Parses `b.../s...` in either order, with an optional `/` separator.
fn parse_bs(lower: &str) -> Result<(u16, u16), String> {
    let mut birth: Option<u16> = None;
    let mut survive: Option<u16> = None;
    let mut current: Option<char> = None;
    for ch in lower.chars() {
        match ch {
            'b' | 's' => {
                let slot = if ch == 'b' { &mut birth } else { &mut survive };
                if slot.is_some() {
                    return Err(format!("`{}` appears twice", ch.to_ascii_uppercase()));
                }
                *slot = Some(0);
                current = Some(ch);
            }
            '/' => {}
            '0'..='8' => {
                let bit = 1u16 << (ch as u32 - '0' as u32);
                match current {
                    Some('b') => birth = birth.map(|m| m | bit),
                    Some('s') => survive = survive.map(|m| m | bit),
                    _ => return Err("digits must follow `B` or `S`".to_string()),
                }
            }
            '9' => return Err("neighbour count 9 is impossible in the Moore neighbourhood".to_string()),
            other => return Err(unsupported_char(other)),
        }
    }
    match (birth, survive) {
        (Some(b), Some(s)) => Ok((b, s)),
        _ => Err("expected both a `B` (birth) and an `S` (survival) part, e.g. B3/S23".to_string()),
    }
}

/// Parses the legacy `survival/birth` digit notation (`23/3`).
fn parse_legacy(lower: &str) -> Result<(u16, u16), String> {
    let Some((s_part, b_part)) = lower.split_once('/') else {
        return Err("expected B/S notation (B3/S23) or survival/birth digits (23/3)".to_string());
    };
    let digits = |part: &str| -> Result<u16, String> {
        let mut mask = 0u16;
        for ch in part.chars() {
            match ch {
                '0'..='8' => mask |= 1 << (ch as u32 - '0' as u32),
                '9' => return Err("neighbour count 9 is impossible in the Moore neighbourhood".to_string()),
                other => return Err(unsupported_char(other)),
            }
        }
        Ok(mask)
    };
    Ok((digits(b_part)?, digits(s_part)?))
}

fn unsupported_char(ch: char) -> String {
    if ch.is_ascii_alphabetic() || ch == '-' {
        format!(
            "`{ch}` is not supported: Hashlife here runs totalistic rules only (no isotropic non-totalistic \
             letters, MAP strings or Generations states)"
        )
    } else {
        format!("unexpected character `{ch}`")
    }
}

impl fmt::Display for Rule {
    /// Canonical `B3/S23` form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits =
            |mask: u16| -> String { (0..=8u8).filter(|n| mask >> n & 1 == 1).map(|n| char::from(b'0' + n)).collect() };
        write!(f, "B{}/S{}", digits(self.birth), digits(self.survive))
    }
}

impl FromStr for Rule {
    type Err = HashlifeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Rule {
    type Error = HashlifeError;

    /// Parses the text with [`Rule::parse`] (used by serde).
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Rule> for String {
    /// The canonical `B3/S23` text (used by serde).
    fn from(rule: Rule) -> Self {
        rule.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_notations() {
        assert_eq!(Rule::parse("B3/S23").unwrap(), Rule::conway());
        assert_eq!(Rule::parse("b3s23").unwrap(), Rule::conway());
        assert_eq!(Rule::parse(" S23/B3 ").unwrap(), Rule::conway());
        assert_eq!(Rule::parse("23/3").unwrap(), Rule::conway());
        assert_eq!(Rule::parse("life").unwrap(), Rule::conway());
        assert_eq!(Rule::parse("B36/S23").unwrap(), Rule::highlife());
        let seeds = Rule::parse("B2/S").unwrap();
        assert_eq!((seeds.birth(), seeds.survive()), (1 << 2, 0));
        assert_eq!(Rule::parse("/2").unwrap(), seeds);
    }

    #[test]
    fn displays_canonically() {
        assert_eq!(Rule::conway().to_string(), "B3/S23");
        assert_eq!(Rule::parse("s012345678/b3").unwrap().to_string(), "B3/S012345678");
        assert_eq!(Rule::parse("B2/S").unwrap().to_string(), "B2/S");
    }

    #[test]
    fn rejects_b0_and_bad_input() {
        assert!(matches!(Rule::parse("B03/S23"), Err(HashlifeError::BirthOnZero { .. })));
        assert!(matches!(Rule::new(1, 0), Err(HashlifeError::BirthOnZero { .. })));
        assert!(matches!(Rule::new(1 << 9, 0), Err(HashlifeError::InvalidRule { .. })));
        for bad in ["", "B3", "B39/S23", "B3-jknr/S23", "B3/S23:T10,10", "hello", "B3/B4/S2", "3/2/1"] {
            assert!(Rule::parse(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn serde_uses_the_validated_canonical_text() {
        use simulation_core::Snapshot;
        // Snapshot payloads are postcard, which encodes a `String` and a rule
        // serialised as text identically.
        let snapshot = Snapshot::encode("t", 1, 0, &Rule::highlife()).unwrap();
        assert_eq!(snapshot.decode::<String>("t", 1).unwrap(), "B36/S23");
        assert_eq!(snapshot.decode::<Rule>("t", 1).unwrap(), Rule::highlife());
        for bad in ["B0/S8", "B3/S23:T10,10", "B3-k/S23"] {
            let snapshot = Snapshot::encode("t", 1, 0, &bad.to_string()).unwrap();
            assert!(snapshot.decode::<Rule>("t", 1).is_err(), "`{bad}` must not deserialise");
        }
    }

    #[test]
    fn transition_function_matches_masks() {
        let r = Rule::conway();
        assert!(r.next_state(false, 3));
        assert!(!r.next_state(false, 2));
        assert!(r.next_state(true, 2) && r.next_state(true, 3));
        assert!(!r.next_state(true, 4) && !r.next_state(true, 9));
    }
}
