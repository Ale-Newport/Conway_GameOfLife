//! Two-state Life-like (outer-totalistic, Moore neighbourhood) rules.

use std::fmt;
use std::str::FromStr;

use super::notation::{self, Label, mask};
use super::{BoundedGrid, RuleError};

/// Bits 0..=8: the only meaningful neighbour counts on the Moore neighbourhood.
const VALID_MASK: u16 = 0x1FF;

/// A two-state Life-like rule.
///
/// A dead cell with *n* live Moore neighbours becomes alive when bit *n* of
/// `birth` is set; a live cell with *n* live neighbours stays alive when bit
/// *n* of `survive` is set; every other cell is dead in the next generation.
/// Only bits 0..=8 are meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LifeRule {
    /// Birth mask: bit *n* set = a dead cell with *n* live neighbours is born.
    pub birth: u16,
    /// Survival mask: bit *n* set = a live cell with *n* live neighbours survives.
    pub survive: u16,
}

/// A rule with a widely used name (LifeWiki "List of Life-like cellular automata").
struct NamedRule {
    name: &'static str,
    birth: u16,
    survive: u16,
}

const NAMED_RULES: &[NamedRule] = &[
    NamedRule { name: "Life", birth: mask("3"), survive: mask("23") },
    NamedRule { name: "HighLife", birth: mask("36"), survive: mask("23") },
    NamedRule { name: "Seeds", birth: mask("2"), survive: 0 },
    NamedRule { name: "Day & Night", birth: mask("3678"), survive: mask("34678") },
    NamedRule { name: "Life without Death", birth: mask("3"), survive: mask("012345678") },
    NamedRule { name: "Maze", birth: mask("3"), survive: mask("12345") },
    NamedRule { name: "Mazectric", birth: mask("3"), survive: mask("1234") },
    NamedRule { name: "2x2", birth: mask("36"), survive: mask("125") },
    NamedRule { name: "Replicator", birth: mask("1357"), survive: mask("1357") },
    NamedRule { name: "Fredkin", birth: mask("1357"), survive: mask("02468") },
    NamedRule { name: "Diamoeba", birth: mask("35678"), survive: mask("5678") },
    NamedRule { name: "Morley", birth: mask("368"), survive: mask("245") },
    NamedRule { name: "Anneal", birth: mask("4678"), survive: mask("35678") },
    NamedRule { name: "Coral", birth: mask("3"), survive: mask("45678") },
    NamedRule { name: "Gnarl", birth: mask("1"), survive: mask("1") },
    NamedRule { name: "Long Life", birth: mask("345"), survive: mask("5") },
    NamedRule { name: "Stains", birth: mask("3678"), survive: mask("235678") },
    NamedRule { name: "Assimilation", birth: mask("345"), survive: mask("4567") },
    NamedRule { name: "Amoeba", birth: mask("357"), survive: mask("1358") },
    NamedRule { name: "Pseudo Life", birth: mask("357"), survive: mask("238") },
    NamedRule { name: "Serviettes", birth: mask("234"), survive: 0 },
    NamedRule { name: "Live Free or Die", birth: mask("2"), survive: mask("0") },
    NamedRule { name: "DotLife", birth: mask("3"), survive: mask("023") },
    NamedRule { name: "Coagulations", birth: mask("378"), survive: mask("235678") },
    NamedRule { name: "Walled Cities", birth: mask("45678"), survive: mask("2345") },
    NamedRule { name: "Flock", birth: mask("3"), survive: mask("12") },
    NamedRule { name: "Vote", birth: mask("5678"), survive: mask("45678") },
];

impl LifeRule {
    /// Builds a rule from masks, rejecting bits above 8.
    pub fn new(birth: u16, survive: u16) -> Result<Self, RuleError> {
        if birth & !VALID_MASK != 0 || survive & !VALID_MASK != 0 {
            return Err(RuleError::new("neighbour-count masks may only use bits 0..=8"));
        }
        Ok(Self { birth, survive })
    }

    /// Builds a rule from lists of neighbour counts.
    pub fn from_counts(birth: &[u8], survive: &[u8]) -> Result<Self, RuleError> {
        let to_mask = |counts: &[u8]| -> Result<u16, RuleError> {
            counts.iter().try_fold(0u16, |mask, &n| {
                if n > 8 {
                    Err(RuleError::new(format!("{n} is not a valid neighbour count (0-8)")))
                } else {
                    Ok(mask | 1 << n)
                }
            })
        };
        Ok(Self { birth: to_mask(birth)?, survive: to_mask(survive)? })
    }

    /// Conway's Game of Life, `B3/S23`.
    pub const fn conway() -> Self {
        Self { birth: mask("3"), survive: mask("23") }
    }

    /// HighLife, `B36/S23` (has a small replicator).
    pub const fn highlife() -> Self {
        Self { birth: mask("36"), survive: mask("23") }
    }

    /// Seeds, `B2/S` (every live cell dies each generation).
    pub const fn seeds() -> Self {
        Self { birth: mask("2"), survive: 0 }
    }

    /// Day & Night, `B3678/S34678` (symmetric under inverting all cells).
    pub const fn day_and_night() -> Self {
        Self { birth: mask("3678"), survive: mask("34678") }
    }

    /// Life without Death, `B3/S012345678`.
    pub const fn life_without_death() -> Self {
        Self { birth: mask("3"), survive: mask("012345678") }
    }

    /// Parses a rule string. Accepted forms (case-insensitive, whitespace
    /// ignored): `B3/S23`, `b3s23`, `S23/B3`, `B2/S`, `B234` (no survival),
    /// the S/B form `23/3` used by MCell and Life 1.05, and well-known names
    /// (`HighLife`, `Day & Night`...). A bounded-grid suffix (`:T100,100`) is
    /// validated and discarded; use [`LifeRule::parse_with_grid`] to keep it.
    pub fn parse(text: &str) -> Result<Self, RuleError> {
        Self::parse_with_grid(text).map(|(rule, _)| rule)
    }

    /// Parses a rule string and returns its bounded-grid suffix, if any. The
    /// grid is `None` when there is no suffix or it describes an unbounded
    /// universe.
    pub fn parse_with_grid(text: &str) -> Result<(Self, Option<BoundedGrid>), RuleError> {
        let (body, suffix) = notation::split_suffix(text);
        let grid = match suffix {
            Some((suffix, column)) => BoundedGrid::parse_at(suffix, column - 1)?,
            None => None,
        };
        if let Some(rule) = Self::from_name(body) {
            return Ok((rule, grid));
        }
        if let Some(error) = notation::name_error(body, false) {
            return Err(error);
        }
        let segments = notation::tokenize(body, false)?;
        if segments.len() == 1 && segments[0].label.is_none() {
            return Err(if segments[0].digits.is_empty() {
                RuleError::new("empty rule")
            } else {
                RuleError::new("a Life-like rule needs a '/' or B/S letters, e.g. B3/S23 or 23/3")
            });
        }
        let labelled = segments.iter().any(|s| s.label.is_some());
        let (birth, survive) = if labelled {
            let mut slots =
                notation::assign(segments, &[Label::Birth, Label::Survival], &[Label::Birth, Label::Survival])?;
            (slots[0].take(), slots[1].take())
        } else if segments.len() == 2 {
            // MCell / Life 1.05 order: survival first, then birth.
            let mut parts = segments.into_iter();
            let survive = parts.next();
            (parts.next(), survive)
        } else {
            return Err(RuleError::at(
                segments[2].column - 1,
                "a rule with three '/'-separated parts is a Generations rule (S/B/C)",
            ));
        };
        let birth = birth.map(|s| s.mask()).transpose()?.unwrap_or(0);
        let survive = survive.map(|s| s.mask()).transpose()?.unwrap_or(0);
        Ok((Self { birth, survive }, grid))
    }

    /// Looks up a well-known rule name (case, spaces and punctuation are
    /// ignored; `&` may be written "and").
    pub fn from_name(name: &str) -> Option<Self> {
        let key = notation::normalize_name(name);
        if key.is_empty() {
            return None;
        }
        NAMED_RULES
            .iter()
            .find(|named| notation::normalize_name(named.name) == key)
            .map(|named| Self { birth: named.birth, survive: named.survive })
    }

    /// Well-known name of this rule (`"Life"`, `"HighLife"`, `"Seeds"`,
    /// `"Day & Night"`, `"Life without Death"`, `"Maze"`, `"2x2"`,
    /// `"Replicator"`, `"Diamoeba"`, `"Morley"`, `"Anneal"`, ...).
    pub fn name(&self) -> Option<&'static str> {
        let (birth, survive) = (self.birth & VALID_MASK, self.survive & VALID_MASK);
        NAMED_RULES.iter().find(|named| named.birth == birth && named.survive == survive).map(|named| named.name)
    }

    /// All named rules, in catalogue order.
    pub fn known_rules() -> impl Iterator<Item = (&'static str, LifeRule)> {
        NAMED_RULES.iter().map(|named| (named.name, LifeRule { birth: named.birth, survive: named.survive }))
    }

    /// Whether empty space gives birth (B0). Such rules turn an infinite dead
    /// background alive, which sparse engines and Hashlife cannot represent
    /// directly.
    pub fn has_b0(&self) -> bool {
        self.birth & 1 != 0
    }

    /// Whether a dead cell with `n` live neighbours is born.
    pub fn births(&self, n: u8) -> bool {
        n <= 8 && self.birth & (1 << n) != 0
    }

    /// Whether a live cell with `n` live neighbours survives.
    pub fn survives(&self, n: u8) -> bool {
        n <= 8 && self.survive & (1 << n) != 0
    }

    /// Next state of a cell that is `alive` and has `live_neighbours` live
    /// Moore neighbours.
    pub fn next_state(&self, alive: bool, live_neighbours: u8) -> bool {
        if alive { self.survives(live_neighbours) } else { self.births(live_neighbours) }
    }

    /// Neighbour counts that cause a birth, ascending.
    pub fn birth_counts(&self) -> Vec<u8> {
        (0..=8).filter(|&n| self.births(n)).collect()
    }

    /// Neighbour counts that allow survival, ascending.
    pub fn survival_counts(&self) -> Vec<u8> {
        (0..=8).filter(|&n| self.survives(n)).collect()
    }
}

impl Default for LifeRule {
    fn default() -> Self {
        Self::conway()
    }
}

impl fmt::Display for LifeRule {
    /// Canonical `B…/S…` notation with ascending digits, e.g. `B3/S23`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B{}/S{}", notation::digits(self.birth), notation::digits(self.survive))
    }
}

impl FromStr for LifeRule {
    type Err = RuleError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::GridKind;

    #[test]
    fn parses_rule_table() {
        let table: &[(&str, &str)] = &[
            ("B3/S23", "B3/S23"),
            ("b3s23", "B3/S23"),
            ("B3S23", "B3/S23"),
            ("S23/B3", "B3/S23"),
            ("s23b3", "B3/S23"),
            ("23/3", "B3/S23"),
            ("B36/S23", "B36/S23"),
            ("B2/S", "B2/S"),
            ("B3/S012345678", "B3/S012345678"),
            ("B234", "B234/S"),
            ("/3", "B3/S"),
            ("B3/", "B3/S"),
            ("/S23", "B/S23"),
            ("B3/23", "B3/S23"),
            ("  B 3 / S 2 3  ", "B3/S23"),
            ("B32/S32", "B23/S23"),
            ("B33/S2", "B3/S2"),
            ("B0/S8", "B0/S8"),
            ("12345/3", "B3/S12345"),
            ("Life", "B3/S23"),
            ("highlife", "B36/S23"),
            ("Day & Night", "B3678/S34678"),
            ("day and night", "B3678/S34678"),
            ("Life without Death", "B3/S012345678"),
            ("2x2", "B36/S125"),
        ];
        for (input, canonical) in table {
            let rule = LifeRule::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(rule.to_string(), *canonical, "input {input}");
            assert_eq!(LifeRule::parse(canonical).unwrap(), rule, "canonical form of {input} must round-trip");
        }
    }

    #[test]
    fn strips_and_returns_grid_suffix() {
        let (rule, grid) = LifeRule::parse_with_grid("B3/S23:T100,100").unwrap();
        assert_eq!(rule, LifeRule::conway());
        let grid = grid.unwrap();
        assert_eq!((grid.kind, grid.width, grid.height), (GridKind::Torus, 100, 100));
        assert_eq!(LifeRule::parse("B3/S23:T100,100").unwrap(), LifeRule::conway());
        assert_eq!(LifeRule::parse_with_grid("B3/S23:").unwrap().1, None);
        let err = LifeRule::parse("B3/S23:Q10").unwrap_err();
        assert_eq!(err.column, Some(8), "{err}");
    }

    #[test]
    fn rejects_invalid_rules_with_clear_messages() {
        let cases: &[(&str, &str, Option<usize>)] = &[
            ("B39/S23", "digit 9 is not a valid neighbour count (0-8)", Some(3)),
            ("23/9", "digit 9 is not a valid neighbour count (0-8)", Some(4)),
            ("", "empty rule", None),
            ("23", "needs a '/' or B/S letters", None),
            ("B3/S23/C3", "state count is only valid in a Generations rule", Some(8)),
            ("2/3/4", "Generations rule", Some(4)),
            ("B3/B6", "'B' appears more than once", Some(4)),
            ("B3/S23/1", "too many", Some(8)),
            ("B3x/S23", "unexpected character 'x'", Some(3)),
            ("B2e3/S23", "non-totalistic", Some(3)),
            ("B3/S23V", "von Neumann", Some(7)),
            ("Brian's Brain", "is a Generations rule", None),
            ("LifeHistory", "unknown rule name `LifeHistory`", None),
        ];
        for (input, message, column) in cases {
            let err = LifeRule::parse(input).expect_err(input);
            assert!(err.message.contains(message), "{input}: got `{err}`, expected `{message}`");
            assert_eq!(err.column, *column, "{input}: {err}");
        }
    }

    #[test]
    fn transition_queries() {
        let life = LifeRule::conway();
        assert!(life.births(3) && !life.births(2) && !life.births(9));
        assert!(life.survives(2) && life.survives(3) && !life.survives(4));
        assert!(life.next_state(false, 3));
        assert!(!life.next_state(true, 1));
        assert!(life.next_state(true, 2));
        assert!(!life.has_b0());
        assert!(LifeRule::parse("B0123478/S01234678").unwrap().has_b0());
        assert_eq!(life.birth_counts(), vec![3]);
        assert_eq!(life.survival_counts(), vec![2, 3]);
        assert_eq!(LifeRule::default(), life);
    }

    #[test]
    fn names_for_well_known_rules() {
        let expected = [
            ("B3/S23", "Life"),
            ("B36/S23", "HighLife"),
            ("B2/S", "Seeds"),
            ("B3678/S34678", "Day & Night"),
            ("B3/S012345678", "Life without Death"),
            ("B3/S12345", "Maze"),
            ("B36/S125", "2x2"),
            ("B1357/S1357", "Replicator"),
            ("B35678/S5678", "Diamoeba"),
            ("B368/S245", "Morley"),
            ("B4678/S35678", "Anneal"),
        ];
        for (rule, name) in expected {
            assert_eq!(LifeRule::parse(rule).unwrap().name(), Some(name), "{rule}");
            assert_eq!(LifeRule::from_name(name).unwrap().to_string(), rule);
        }
        assert_eq!(LifeRule::parse("B37/S23").unwrap().name(), None);
        assert_eq!(LifeRule::seeds().name(), Some("Seeds"));
        assert_eq!(LifeRule::day_and_night().name(), Some("Day & Night"));
        assert_eq!(LifeRule::life_without_death().name(), Some("Life without Death"));
        // Every named rule maps back to its own name (no duplicate masks).
        for (name, rule) in LifeRule::known_rules() {
            assert_eq!(rule.name(), Some(name));
        }
    }

    #[test]
    fn constructors_validate() {
        assert!(LifeRule::new(1 << 9, 0).is_err());
        assert_eq!(LifeRule::new(1 << 3, 0b1100).unwrap(), LifeRule::conway());
        assert_eq!(LifeRule::from_counts(&[3, 6], &[2, 3]).unwrap(), LifeRule::highlife());
        assert!(LifeRule::from_counts(&[9], &[]).is_err());
    }
}
