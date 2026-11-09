//! Multi-state "Generations" rules (Golly's `generationsalgo`, MCell's
//! Generations family).

use std::fmt;
use std::str::FromStr;

use super::notation::{self, Label, mask};
use super::{BoundedGrid, LifeRule, RuleError};

/// A Generations rule: Life-like birth/survival on the Moore neighbourhood
/// plus a decay chain of "dying" states.
///
/// States are `0` (dead), `1` (alive) and `2..states` (dying). Only state-1
/// cells count as live neighbours. Transition, for *n* live neighbours:
///
/// * state 0 → 1 if bit *n* of `birth` is set, else stays 0;
/// * state 1 → stays 1 if bit *n* of `survive` is set, else starts dying
///   (state 2, or 0 when `states == 2`);
/// * state *k* ≥ 2 → *k* + 1, wrapping to 0 when *k* + 1 = `states`.
///
/// `states` is at least 2. Golly allows up to 256 states; this type stores the
/// count in a `u8`, so at most 255 are supported. B0 is rejected, as in Golly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GenerationsRule {
    /// Birth mask (bit *n* = birth with *n* live neighbours).
    pub birth: u16,
    /// Survival mask (bit *n* = a state-1 cell with *n* live neighbours stays alive).
    pub survive: u16,
    /// Total number of states including dead and alive (≥ 2).
    pub states: u8,
}

struct NamedGenerations {
    name: &'static str,
    birth: u16,
    survive: u16,
    states: u8,
}

/// Named rules from Golly's Generations help page and MCell's collection.
const NAMED_RULES: &[NamedGenerations] = &[
    NamedGenerations { name: "Brian's Brain", birth: mask("2"), survive: 0, states: 3 },
    NamedGenerations { name: "Star Wars", birth: mask("2"), survive: mask("345"), states: 4 },
    NamedGenerations { name: "Banners", birth: mask("3457"), survive: mask("2367"), states: 5 },
    NamedGenerations { name: "Bloomerang", birth: mask("34678"), survive: mask("234"), states: 24 },
    NamedGenerations { name: "Caterpillars", birth: mask("378"), survive: mask("124567"), states: 4 },
    NamedGenerations { name: "Cooties", birth: mask("2"), survive: mask("23"), states: 8 },
    NamedGenerations { name: "Fireworks", birth: mask("13"), survive: mask("2"), states: 21 },
    NamedGenerations { name: "Frogs", birth: mask("34"), survive: mask("12"), states: 3 },
    NamedGenerations { name: "Lava", birth: mask("45678"), survive: mask("12345"), states: 8 },
    NamedGenerations { name: "Lines", birth: mask("458"), survive: mask("012345"), states: 3 },
    NamedGenerations { name: "Sticks", birth: mask("2"), survive: mask("3456"), states: 6 },
    NamedGenerations { name: "Transers", birth: mask("26"), survive: mask("345"), states: 5 },
    NamedGenerations { name: "Xtasy", birth: mask("2356"), survive: mask("1456"), states: 16 },
];

impl GenerationsRule {
    /// Builds a rule, validating masks, the state count and the absence of B0.
    pub fn new(birth: u16, survive: u16, states: u8) -> Result<Self, RuleError> {
        LifeRule::new(birth, survive)?;
        if states < 2 {
            return Err(RuleError::new("a Generations rule needs at least 2 states"));
        }
        if birth & 1 != 0 {
            return Err(RuleError::new("Generations rules with B0 are not supported"));
        }
        Ok(Self { birth, survive, states })
    }

    /// Brian's Brain, `/2/3` (`B2/S/C3`).
    pub const fn brians_brain() -> Self {
        Self { birth: mask("2"), survive: 0, states: 3 }
    }

    /// Star Wars, `345/2/4` (`B2/S345/C4`).
    pub const fn star_wars() -> Self {
        Self { birth: mask("2"), survive: mask("345"), states: 4 }
    }

    /// Parses a Generations rule. Accepted forms (case-insensitive, whitespace
    /// ignored): Golly/MCell `S/B/C` (`345/2/4`, `/2/3`), LifeWiki/LifeViewer
    /// `B2/S/C3` and `B2/S345/C4` (also with `G` for `C`), Golly's `B2/S/3`,
    /// Catagolue's `g3b2s`, and named rules (`Brian's Brain`, `Star Wars`...).
    /// A bounded-grid suffix is validated and discarded; use
    /// [`GenerationsRule::parse_with_grid`] to keep it.
    pub fn parse(text: &str) -> Result<Self, RuleError> {
        Self::parse_with_grid(text).map(|(rule, _)| rule)
    }

    /// Parses a rule string and returns its bounded-grid suffix, if any.
    pub fn parse_with_grid(text: &str) -> Result<(Self, Option<BoundedGrid>), RuleError> {
        let (body, suffix) = notation::split_suffix(text);
        let grid = match suffix {
            Some((suffix, column)) => BoundedGrid::parse_at(suffix, column - 1)?,
            None => None,
        };
        if let Some(rule) = Self::from_name(body) {
            return Ok((rule, grid));
        }
        if let Some(error) = notation::name_error(body, true) {
            return Err(error);
        }
        let segments = notation::tokenize(body, true)?;
        if segments.len() == 1 && segments[0].label.is_none() && segments[0].digits.is_empty() {
            return Err(RuleError::new("empty rule"));
        }
        let labelled = segments.iter().any(|s| s.label.is_some());
        let slots = [Label::Birth, Label::Survival, Label::States];
        let mut parts = if labelled {
            // Unlabelled parts fill the missing slots; in `B2/S/3` that is the state count.
            notation::assign(segments, &slots, &[Label::States, Label::Birth, Label::Survival])?
        } else if segments.len() == 3 {
            // Golly / MCell order: survival, birth, states.
            notation::assign(segments, &slots, &[Label::Survival, Label::Birth, Label::States])?
        } else {
            return Err(RuleError::new(
                "a Generations rule needs three parts: survival/birth/states (e.g. 345/2/4) or B2/S345/C4",
            ));
        };
        let states_segment =
            parts[2].take().ok_or_else(|| RuleError::new("a Generations rule needs a state count (e.g. B2/S/C3)"))?;
        let states = parse_state_count(&states_segment)?;
        let birth = parts[0].take().map(|s| s.mask()).transpose()?.unwrap_or(0);
        let survive = parts[1].take().map(|s| s.mask()).transpose()?.unwrap_or(0);
        if birth & 1 != 0 {
            return Err(RuleError::new("Generations rules with B0 are not supported (Golly rejects them too)"));
        }
        Ok((Self { birth, survive, states }, grid))
    }

    /// Looks up a named Generations rule (case and punctuation ignored).
    pub fn from_name(name: &str) -> Option<Self> {
        let key = notation::normalize_name(name);
        if key.is_empty() {
            return None;
        }
        NAMED_RULES.iter().find(|named| notation::normalize_name(named.name) == key).map(|named| Self {
            birth: named.birth,
            survive: named.survive,
            states: named.states,
        })
    }

    /// Well-known name of this rule, if it has one.
    pub fn name(&self) -> Option<&'static str> {
        NAMED_RULES
            .iter()
            .find(|n| n.birth == self.birth && n.survive == self.survive && n.states == self.states)
            .map(|n| n.name)
    }

    /// All named Generations rules.
    pub fn known_rules() -> impl Iterator<Item = (&'static str, GenerationsRule)> {
        NAMED_RULES.iter().map(|n| (n.name, GenerationsRule { birth: n.birth, survive: n.survive, states: n.states }))
    }

    /// Whether a dead cell with `n` live (state-1) neighbours is born.
    pub fn births(&self, n: u8) -> bool {
        n <= 8 && self.birth & (1 << n) != 0
    }

    /// Whether a state-1 cell with `n` live neighbours stays alive.
    pub fn survives(&self, n: u8) -> bool {
        n <= 8 && self.survive & (1 << n) != 0
    }

    /// Next state of a cell in `state` with `live_neighbours` state-1
    /// neighbours. States outside `0..states` are treated as the last dying
    /// state and decay to 0.
    pub fn next_state(&self, state: u8, live_neighbours: u8) -> u8 {
        match state {
            0 => u8::from(self.births(live_neighbours)),
            1 if self.survives(live_neighbours) => 1,
            _ => {
                let next = u16::from(state) + 1;
                if next >= u16::from(self.states) { 0 } else { next as u8 }
            }
        }
    }

    /// The two-state Life-like rule with the same birth and survival sets.
    pub fn life_rule(&self) -> LifeRule {
        LifeRule { birth: self.birth, survive: self.survive }
    }

    /// LifeWiki / LifeViewer notation, e.g. `B2/S/C3`.
    pub fn to_bsc_string(&self) -> String {
        format!("B{}/S{}/C{}", notation::digits(self.birth), notation::digits(self.survive), self.states)
    }
}

/// Parses the state-count part: decimal digits, 2..=255.
fn parse_state_count(segment: &notation::Segment) -> Result<u8, RuleError> {
    if segment.digits.is_empty() {
        return Err(RuleError::at(segment.column, "missing number of states"));
    }
    let mut value: u32 = 0;
    for &(digit, _) in &segment.digits {
        value = value.saturating_mul(10).saturating_add(u32::from(digit));
    }
    if value < 2 {
        return Err(RuleError::at(segment.column, format!("a Generations rule needs at least 2 states, got {value}")));
    }
    u8::try_from(value).map_err(|_| {
        RuleError::at(segment.column, format!("{value} states requested; at most 255 states are supported"))
    })
}

impl fmt::Display for GenerationsRule {
    /// Golly's canonical `S/B/C` form, e.g. `/2/3` or `345/2/4`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", notation::digits(self.survive), notation::digits(self.birth), self.states)
    }
}

impl FromStr for GenerationsRule {
    type Err = RuleError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_notations() {
        let table: &[(&str, &str)] = &[
            ("/2/3", "/2/3"),
            ("B2/S/C3", "/2/3"),
            ("b2/s/c3", "/2/3"),
            ("B2/S/3", "/2/3"),
            ("g3b2s", "/2/3"),
            ("B2/S345/C4", "345/2/4"),
            ("S345/B2/C4", "345/2/4"),
            ("B2/S345/G4", "345/2/4"),
            ("345/2/4", "345/2/4"),
            ("2367/3457/5", "2367/3457/5"),
            ("1456/2356/16", "1456/2356/16"),
            ("23/3/2", "23/3/2"),
            (" / 2 / 3 ", "/2/3"),
            ("Brian's Brain", "/2/3"),
            ("star wars", "345/2/4"),
            ("12/34/255", "12/34/255"),
        ];
        for (input, canonical) in table {
            let rule = GenerationsRule::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(rule.to_string(), *canonical, "input {input}");
            assert_eq!(GenerationsRule::parse(canonical).unwrap(), rule);
            assert_eq!(GenerationsRule::parse(&rule.to_bsc_string()).unwrap(), rule);
        }
        assert_eq!(GenerationsRule::brians_brain().to_bsc_string(), "B2/S/C3");
        assert_eq!(GenerationsRule::star_wars().to_bsc_string(), "B2/S345/C4");
    }

    #[test]
    fn rejects_invalid_rules() {
        let cases: &[(&str, &str)] = &[
            ("/2/1", "at least 2 states"),
            ("/2/256", "at most 255"),
            ("/2/", "missing number of states"),
            ("B2/S", "state count"),
            ("2/3", "three parts"),
            ("/02/3", "B0"),
            ("/9/3", "digit 9"),
            ("B2/S/C3/4", "too many"),
            ("", "empty rule"),
            ("HighLife", "is a two-state Life-like rule"),
            ("WireWorld", "unknown rule name"),
        ];
        for (input, message) in cases {
            let err = GenerationsRule::parse(input).expect_err(input);
            assert!(err.message.contains(message), "{input}: got `{err}`, expected `{message}`");
        }
    }

    #[test]
    fn transitions_follow_golly_semantics() {
        let brain = GenerationsRule::brians_brain();
        assert_eq!(brain.next_state(0, 2), 1);
        assert_eq!(brain.next_state(0, 3), 0);
        assert_eq!(brain.next_state(1, 2), 2, "Brian's Brain has no survival: live cells start dying");
        assert_eq!(brain.next_state(2, 5), 0, "last dying state decays to dead");
        let wars = GenerationsRule::star_wars();
        assert_eq!(wars.next_state(1, 4), 1);
        assert_eq!(wars.next_state(1, 2), 2);
        assert_eq!(wars.next_state(2, 2), 3, "dying cells ignore neighbours");
        assert_eq!(wars.next_state(3, 0), 0);
        let two = GenerationsRule::parse("23/3/2").unwrap();
        assert_eq!(two.next_state(1, 1), 0, "with two states a dying cell is dead");
        let max = GenerationsRule::parse("/2/255").unwrap();
        assert_eq!(max.next_state(254, 0), 0);
        assert_eq!(max.next_state(253, 0), 254);
    }

    #[test]
    fn names_and_constructors() {
        assert_eq!(GenerationsRule::brians_brain().name(), Some("Brian's Brain"));
        assert_eq!(GenerationsRule::star_wars().name(), Some("Star Wars"));
        assert_eq!(GenerationsRule::parse("/2/4").unwrap().name(), None);
        for (name, rule) in GenerationsRule::known_rules() {
            assert_eq!(rule.name(), Some(name));
            assert_eq!(GenerationsRule::from_name(name), Some(rule));
        }
        assert!(GenerationsRule::new(1, 0, 3).is_err());
        assert!(GenerationsRule::new(4, 0, 1).is_err());
        assert_eq!(GenerationsRule::new(4, 0, 3).unwrap(), GenerationsRule::brians_brain());
        assert_eq!(GenerationsRule::star_wars().life_rule().to_string(), "B2/S345");
    }
}
