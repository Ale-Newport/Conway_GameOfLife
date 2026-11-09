//! Tokenizer shared by the Life-like and Generations rule parsers.
//!
//! A rule body is split into *segments*. A `/` always ends a segment; a label
//! letter (`B`, `S`, and for Generations `C` or `G`) starts a new labelled
//! segment unless the current segment is still empty. So `B3/S23`, `b3s23`,
//! `S23/B3`, `23/3`, `/2/3`, `B2/S/C3` and `g3b2s` all tokenize to the same
//! kind of segment list, and the callers decide which part is which.

use super::RuleError;

/// Meaning of a labelled segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    Birth,
    Survival,
    States,
}

impl Label {
    pub(crate) fn letter(self) -> char {
        match self {
            Self::Birth => 'B',
            Self::Survival => 'S',
            Self::States => 'C',
        }
    }
}

/// One part of a rule body.
#[derive(Clone, Debug)]
pub(crate) struct Segment {
    /// Explicit label, if the segment started with a label letter.
    pub label: Option<Label>,
    /// 1-based column where the segment starts.
    pub column: usize,
    /// Digits with their 1-based columns.
    pub digits: Vec<(u8, usize)>,
}

impl Segment {
    fn new(column: usize) -> Self {
        Self { label: None, column, digits: Vec::new() }
    }

    fn is_blank(&self) -> bool {
        self.label.is_none() && self.digits.is_empty()
    }

    /// Converts the digits to a neighbour-count mask (bit n = count n).
    pub(crate) fn mask(&self) -> Result<u16, RuleError> {
        let mut mask = 0u16;
        for &(digit, column) in &self.digits {
            if digit > 8 {
                return Err(RuleError::at(column, format!("digit {digit} is not a valid neighbour count (0-8)")));
            }
            mask |= 1 << digit;
        }
        Ok(mask)
    }
}

/// Splits a rule into its body and the bounded-grid suffix (everything from
/// the first `:`), returning the suffix with the 1-based column of the colon.
pub(crate) fn split_suffix(rule: &str) -> (&str, Option<(&str, usize)>) {
    match rule.find(':') {
        Some(index) => (&rule[..index], Some((&rule[index..], rule[..index].chars().count() + 1))),
        None => (rule, None),
    }
}

/// Normalises a rule name for lookup: lowercase ASCII letters and digits only,
/// with `&` read as "and" (`"Day & Night"` → `"dayandnight"`).
pub(crate) fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch == '&' {
            out.push_str("and");
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

/// Builds a neighbour-count mask from a string of digits at compile time.
pub(crate) const fn mask(digits: &str) -> u16 {
    let bytes = digits.as_bytes();
    let mut mask = 0u16;
    let mut i = 0;
    while i < bytes.len() {
        mask |= 1 << (bytes[i] - b'0');
        i += 1;
    }
    mask
}

/// Writes the neighbour counts set in `mask` in ascending order.
pub(crate) fn digits(mask: u16) -> String {
    (0..=8u8).filter(|n| mask & (1 << n) != 0).map(|n| char::from(b'0' + n)).collect()
}

/// Cheap structural test used by [`super::Rule::parse_with_grid`]: a body with
/// a state-count label or three `/`-separated parts is a Generations rule.
pub(crate) fn looks_like_generations(body: &str) -> bool {
    if body.chars().filter(|&c| c == '/').count() >= 2 {
        return true;
    }
    if super::GenerationsRule::from_name(body).is_some() {
        return true;
    }
    if super::LifeRule::from_name(body).is_some() {
        return false;
    }
    // A `C`/`G` label at the start of a part marks a state count (`B2/S/C3`, `g3b2s`).
    let compact: Vec<char> = body.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    compact.iter().enumerate().any(|(i, &c)| (c == 'c' || c == 'g') && (i == 0 || compact[i - 1] == '/'))
}

/// Explains why `body` is not notation when it looks like a rule name: a name
/// of the other rule family, or an unknown name (letters only, no digits or
/// slashes, and not just B/S/C/G labels).
pub(crate) fn name_error(body: &str, generations: bool) -> Option<RuleError> {
    let trimmed = body.trim();
    if !generations && super::GenerationsRule::from_name(trimmed).is_some() {
        return Some(RuleError::new(format!("`{trimmed}` is a Generations rule, not a two-state Life-like rule")));
    }
    if generations && super::LifeRule::from_name(trimmed).is_some() {
        return Some(RuleError::new(format!("`{trimmed}` is a two-state Life-like rule, not a Generations rule")));
    }
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    let only_labels =
        trimmed.chars().all(|c| c.is_whitespace() || matches!(c.to_ascii_lowercase(), 'b' | 's' | 'c' | 'g'));
    let notation = trimmed.chars().any(|c| c.is_ascii_digit() || c == '/');
    (letters >= 2 && !only_labels && !notation).then(|| RuleError::new(format!("unknown rule name `{trimmed}`")))
}

/// Tokenizes a rule body. `allow_states` enables the `C`/`G` label used by
/// Generations rules.
pub(crate) fn tokenize(body: &str, allow_states: bool) -> Result<Vec<Segment>, RuleError> {
    let lower = body.trim_start().to_ascii_lowercase();
    if lower.starts_with("map") {
        return Err(RuleError::new("MAP rules are not supported; use B/S notation"));
    }
    if lower.starts_with('w') {
        return Err(RuleError::new("one-dimensional Wolfram rules (W...) are not supported"));
    }
    if lower.starts_with('r') && lower[1..].starts_with(|c: char| c.is_ascii_digit()) {
        return Err(RuleError::new("Larger than Life rules (R<range>,C..,M..,S..,B..,N..) are not supported"));
    }
    let mut segments = Vec::new();
    let mut current = Segment::new(1);
    for (index, ch) in body.chars().enumerate() {
        let column = index + 1;
        let lower = ch.to_ascii_lowercase();
        match lower {
            c if c.is_whitespace() => {}
            '/' => {
                segments.push(std::mem::replace(&mut current, Segment::new(column + 1)));
            }
            '0'..='9' => current.digits.push((lower as u8 - b'0', column)),
            'b' | 's' => {
                if !current.is_blank() {
                    segments.push(std::mem::replace(&mut current, Segment::new(column)));
                }
                current.label = Some(if lower == 'b' { Label::Birth } else { Label::Survival });
                current.column = column;
            }
            'c' | 'g' => {
                // A state count label only starts a part (`B2/S/C3`, `g3b2s`);
                // after a digit the letter would be isotropic notation (`B2c`).
                if !current.is_blank() {
                    return Err(RuleError::at(
                        column,
                        format!("non-totalistic (isotropic) rule notation is not supported: unexpected '{ch}'"),
                    ));
                }
                if !allow_states {
                    return Err(RuleError::at(
                        column,
                        format!("unexpected '{ch}': a state count is only valid in a Generations rule"),
                    ));
                }
                current.label = Some(Label::States);
                current.column = column;
            }
            'h' | 'v' => {
                return Err(RuleError::at(
                    column,
                    "hexagonal (H) and von Neumann (V) neighbourhoods are not supported; only the Moore neighbourhood is",
                ));
            }
            'a' | 'e' | 'i' | 'j' | 'k' | 'n' | 'q' | 'r' | 't' | 'y' | 'z' | '-' => {
                return Err(RuleError::at(
                    column,
                    format!("non-totalistic (isotropic) rule notation is not supported: unexpected '{ch}'"),
                ));
            }
            _ => return Err(RuleError::at(column, format!("unexpected character '{ch}' in rule"))),
        }
    }
    segments.push(current);
    Ok(segments)
}

/// Assigns segments to slots. Labelled segments go to their slot; each label
/// may appear once. Unlabelled segments fill the remaining slots in the order
/// given by `fallback_order`.
pub(crate) fn assign(
    segments: Vec<Segment>,
    slots: &[Label],
    fallback_order: &[Label],
) -> Result<Vec<Option<Segment>>, RuleError> {
    let mut assigned: Vec<Option<Segment>> = vec![None; slots.len()];
    let mut unlabelled = Vec::new();
    for segment in segments {
        match segment.label {
            Some(label) => {
                let slot = slots.iter().position(|&s| s == label).ok_or_else(|| {
                    RuleError::at(segment.column, format!("'{}' is not valid in this rule", label.letter()))
                })?;
                if assigned[slot].is_some() {
                    return Err(RuleError::at(segment.column, format!("'{}' appears more than once", label.letter())));
                }
                assigned[slot] = Some(segment);
            }
            None => unlabelled.push(segment),
        }
    }
    let free_slots: Vec<usize> = fallback_order
        .iter()
        .filter_map(|label| {
            let slot = slots.iter().position(|s| s == label)?;
            assigned[slot].is_none().then_some(slot)
        })
        .collect();
    if unlabelled.len() > free_slots.len() {
        let extra = &unlabelled[free_slots.len()];
        return Err(RuleError::at(extra.column, "too many '/'-separated parts in rule"));
    }
    for (segment, slot) in unlabelled.into_iter().zip(free_slots) {
        assigned[slot] = Some(segment);
    }
    Ok(assigned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(body: &str) -> Vec<(Option<Label>, String)> {
        tokenize(body, true)
            .unwrap()
            .into_iter()
            .map(|s| (s.label, s.digits.iter().map(|(d, _)| char::from(b'0' + d)).collect()))
            .collect()
    }

    #[test]
    fn tokenizes_common_forms() {
        use Label::*;
        assert_eq!(shape("B3/S23"), vec![(Some(Birth), "3".into()), (Some(Survival), "23".into())]);
        assert_eq!(shape("b3s23"), vec![(Some(Birth), "3".into()), (Some(Survival), "23".into())]);
        assert_eq!(shape("23/3"), vec![(None, "23".into()), (None, "3".into())]);
        assert_eq!(shape("/2/3"), vec![(None, "".into()), (None, "2".into()), (None, "3".into())]);
        assert_eq!(
            shape("g3b2s"),
            vec![(Some(States), "3".into()), (Some(Birth), "2".into()), (Some(Survival), "".into())]
        );
        assert_eq!(shape(" B 3 / S 2 3 "), vec![(Some(Birth), "3".into()), (Some(Survival), "23".into())]);
    }

    #[test]
    fn masks_and_digits() {
        assert_eq!(mask("23"), 0b1100);
        assert_eq!(digits(mask("012345678")), "012345678");
        assert_eq!(digits(0), "");
    }

    #[test]
    fn splits_suffix_with_column() {
        assert_eq!(split_suffix("B3/S23:T10,10"), ("B3/S23", Some((":T10,10", 7))));
        assert_eq!(split_suffix("B3/S23"), ("B3/S23", None));
    }

    #[test]
    fn rejects_unsupported_notations() {
        assert!(tokenize("B2e/S", false).unwrap_err().message.contains("non-totalistic"));
        assert!(tokenize("B2c3/S23", true).unwrap_err().message.contains("non-totalistic"));
        assert!(tokenize("B3/S23/C2", false).unwrap_err().message.contains("Generations"));
        assert!(tokenize("B2/S34H", false).unwrap_err().message.contains("hexagonal"));
        assert!(
            tokenize(
                "MAPARYXfhZofugWaH7oaIDogBZofuhogOiAaIDogIAAgAAWaH7oaIDogGiA6ICAAIAAaIDogIAAgACAAIAAAAAAAA",
                false
            )
            .unwrap_err()
            .message
            .contains("MAP")
        );
        assert!(tokenize("W110", false).unwrap_err().message.contains("Wolfram"));
        assert!(tokenize("R2,C0,M1,S2..3,B3..3,NM", true).unwrap_err().message.contains("Larger than Life"));
        assert!(tokenize(" r5,c0,m1,s34..58,b34..45,nm", false).unwrap_err().message.contains("Larger than Life"));
        assert_eq!(tokenize("B3/S2%3", false).unwrap_err().column, Some(6));
    }

    #[test]
    fn detects_generations_shape() {
        assert!(looks_like_generations("/2/3"));
        assert!(looks_like_generations("B2/S/C3"));
        assert!(!looks_like_generations("B2c/S23"));
        assert!(looks_like_generations("g3b2s"));
        assert!(looks_like_generations("Star Wars"));
        assert!(!looks_like_generations("B3/S23"));
        assert!(!looks_like_generations("23/3"));
        assert!(!looks_like_generations("Coral"));
    }

    #[test]
    fn explains_names() {
        assert!(name_error("Brian's Brain", false).unwrap().message.contains("Generations rule"));
        assert!(name_error("HighLife", true).unwrap().message.contains("Life-like"));
        assert_eq!(name_error("LifeHistory", false).unwrap().message, "unknown rule name `LifeHistory`");
        assert!(name_error("B3/S23", false).is_none());
        assert!(name_error("bs", false).is_none());
        assert!(name_error("B2e", false).is_none());
    }
}
