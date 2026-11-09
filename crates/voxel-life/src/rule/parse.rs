//! Parsers for the supported rule notations.
//!
//! * **Softology** `S/B/C/N` — exactly four fields: survival list, birth
//!   list, number of states, neighbourhood letter (`M` or `N`). Lists are
//!   comma separated with `a-b` ranges (`13-14,17-19`).
//! * **Letters** `B…/S…` in either order, plus optional `C<n>`/`G<n>` or
//!   bare `<n>` (number of states, as in Golly's `B2/S/3`) and `M`/`N`
//!   fields. A list without `,`, `-` or `.`
//!   is read digit by digit (`S45` = {4, 5}); otherwise it is itemised like
//!   a Softology list (`S13-26`, `B5,10`).
//! * **Bays** `E_l E_u F_l F_u` — four digits (`5766`) or four integers
//!   (`(10 21 10 21)`); survive iff `E_l ≤ n ≤ E_u`, born iff `F_l ≤ n ≤ F_u`,
//!   two states, Moore neighbourhood.
//! * **Golly** `3D<S>/<B>[F|V]` — survival first, lists with `a..b` ranges,
//!   optional `F`/`V` suffix for the face (von Neumann) neighbourhood.
//!
//! Itemised lists accept both `a-b` and `a..b` ranges.

use super::{CountSet, Neighbourhood, Rule3D};
use crate::error::RuleError;

/// How a list of counts is written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListStyle {
    /// Comma-separated numbers and ranges.
    Itemised,
    /// Digits (one count each) unless the text contains `,`, `-` or `.`.
    Letters,
}

pub(super) fn parse(text: &str) -> Result<Rule3D, RuleError> {
    let s = text.trim();
    if s.is_empty() {
        return Err(RuleError::Empty);
    }
    if let Some(rest) = s.get(..2).filter(|p| p.eq_ignore_ascii_case("3D")).and_then(|_| s.get(2..)) {
        return parse_golly(rest);
    }
    if !s.contains('/') {
        return if s.starts_with(|c: char| c.is_ascii_digit() || c == '(') {
            parse_bays(s)
        } else {
            Err(RuleError::UnknownNotation(s.to_string()))
        };
    }
    let fields: Vec<&str> = s.split('/').map(str::trim).collect();
    if fields.iter().any(|f| f.starts_with(['B', 'b', 'S', 's'])) {
        return parse_letters(&fields);
    }
    if fields.len() == 4 {
        return parse_softology(&fields);
    }
    Err(RuleError::UnknownNotation(s.to_string()))
}

fn field_error(field: &'static str, reason: impl Into<String>) -> RuleError {
    RuleError::Field { field, reason: reason.into() }
}

fn parse_count(text: &str, field: &'static str, neighbourhood: Neighbourhood) -> Result<u32, RuleError> {
    let text = text.trim();
    let count: u32 = text.parse().map_err(|_| field_error(field, format!("`{text}` is not a neighbour count")))?;
    if count > neighbourhood.size() {
        return Err(RuleError::CountOutOfRange {
            field,
            count,
            size: neighbourhood.size(),
            neighbourhood: neighbourhood.label(),
        });
    }
    Ok(count)
}

fn parse_list(
    text: &str,
    field: &'static str,
    style: ListStyle,
    neighbourhood: Neighbourhood,
) -> Result<CountSet, RuleError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(CountSet::EMPTY);
    }
    if style == ListStyle::Letters && !text.contains([',', '-', '.']) {
        let mut counts = Vec::with_capacity(text.len());
        for c in text.chars() {
            let digit = c.to_digit(10).ok_or_else(|| field_error(field, format!("`{c}` is not a digit")))?;
            counts.push(parse_count(&digit.to_string(), field, neighbourhood)?);
        }
        return Ok(counts.into_iter().collect());
    }
    let mut counts = Vec::new();
    for item in text.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(field_error(field, format!("empty item in `{text}`")));
        }
        let bounds = item.split_once("..").or_else(|| item.split_once('-'));
        let (lo, hi) = match bounds {
            Some((a, b)) => (parse_count(a, field, neighbourhood)?, parse_count(b, field, neighbourhood)?),
            None => {
                let n = parse_count(item, field, neighbourhood)?;
                (n, n)
            }
        };
        if lo > hi {
            return Err(field_error(field, format!("range `{item}` is reversed")));
        }
        counts.extend(lo..=hi);
    }
    Ok(counts.into_iter().collect())
}

fn parse_states(text: &str) -> Result<u16, RuleError> {
    let text = text.trim();
    text.parse::<u16>().ok().filter(|n| (2..=256).contains(n)).ok_or_else(|| RuleError::States(text.to_string()))
}

fn parse_neighbourhood(text: &str) -> Result<Neighbourhood, RuleError> {
    match text.trim() {
        "M" | "m" => Ok(Neighbourhood::Moore),
        "N" | "n" => Ok(Neighbourhood::VonNeumann),
        other => Err(RuleError::Neighbourhood(other.to_string())),
    }
}

fn parse_softology(fields: &[&str]) -> Result<Rule3D, RuleError> {
    let [survive, birth, states, neighbourhood] = fields else {
        return Err(field_error("Softology rule", "expected four fields S/B/C/N"));
    };
    let neighbourhood = parse_neighbourhood(neighbourhood)?;
    let survive = parse_list(survive, "survival list", ListStyle::Itemised, neighbourhood)?;
    let birth = parse_list(birth, "birth list", ListStyle::Itemised, neighbourhood)?;
    Rule3D::new(survive, birth, parse_states(states)?, neighbourhood)
}

fn parse_letters(fields: &[&str]) -> Result<Rule3D, RuleError> {
    let mut birth = None;
    let mut survive = None;
    let mut states = None;
    let mut neighbourhood = None;
    for &field in fields {
        let mut chars = field.chars();
        let (slot, name, rest) = match chars.next().map(|c| c.to_ascii_uppercase()) {
            Some('B') => (&mut birth, "birth list", chars.as_str()),
            Some('S') => (&mut survive, "survival list", chars.as_str()),
            Some('C' | 'G') => (&mut states, "number of states", chars.as_str()),
            Some(c) if c.is_ascii_digit() => (&mut states, "number of states", field),
            Some('M' | 'N') if field.len() == 1 => (&mut neighbourhood, "neighbourhood", field),
            _ => {
                return Err(field_error(
                    "rule",
                    format!(
                        "field `{field}` must start with B, S, C or G, be a number of states, or be the \
                         neighbourhood letter M or N"
                    ),
                ));
            }
        };
        if slot.replace(rest).is_some() {
            return Err(field_error(name, "given twice"));
        }
    }
    let neighbourhood = neighbourhood.map(parse_neighbourhood).transpose()?.unwrap_or_default();
    let birth = birth.ok_or_else(|| field_error("birth list", "the B field is missing"))?;
    let survive = survive.ok_or_else(|| field_error("survival list", "the S field is missing"))?;
    let survive = parse_list(survive, "survival list", ListStyle::Letters, neighbourhood)?;
    let birth = parse_list(birth, "birth list", ListStyle::Letters, neighbourhood)?;
    let states = states.map(parse_states).transpose()?.unwrap_or(2);
    Rule3D::new(survive, birth, states, neighbourhood)
}

fn parse_bays(text: &str) -> Result<Rule3D, RuleError> {
    let inner = text.strip_prefix('(').and_then(|r| r.strip_suffix(')')).unwrap_or(text);
    let tokens: Vec<&str> = inner.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
    let field = "Bays rule";
    let moore = Neighbourhood::Moore;
    let bounds: Vec<u32> = match tokens.as_slice() {
        [digits] if digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit()) => {
            digits.bytes().map(|b| u32::from(b - b'0')).collect()
        }
        [_, _, _, _] => tokens.iter().map(|t| parse_count(t, field, moore)).collect::<Result<_, _>>()?,
        _ => {
            return Err(field_error(
                field,
                format!("`{text}` must give four bounds E_l E_u F_l F_u, e.g. `5766` or `(10 21 10 21)`"),
            ));
        }
    };
    let &[el, eu, fl, fu] = bounds.as_slice() else {
        return Err(field_error(field, "expected four bounds"));
    };
    if el > eu {
        return Err(field_error(field, format!("survival interval is reversed (E_l = {el} > E_u = {eu})")));
    }
    if fl > fu {
        return Err(field_error(field, format!("birth interval is reversed (F_l = {fl} > F_u = {fu})")));
    }
    Rule3D::new(CountSet::range(el, eu), CountSet::range(fl, fu), 2, moore)
}

fn parse_golly(rest: &str) -> Result<Rule3D, RuleError> {
    let rest = rest.trim();
    let field = "Golly 3D rule";
    if rest.eq_ignore_ascii_case("BB") || rest.eq_ignore_ascii_case("BBW") {
        return Err(field_error(field, "Busy Boxes (BB, BBW) is a reversible partitioning rule and is not supported"));
    }
    let (survive, birth_and_suffix) = rest
        .split_once('/')
        .ok_or_else(|| field_error(field, "expected `3D<survival>/<birth>` with an optional F/V"))?;
    if birth_and_suffix.contains('/') {
        return Err(field_error(field, "a Golly 3D rule has exactly one `/`"));
    }
    let split = birth_and_suffix.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(birth_and_suffix.len());
    let (birth, suffix) = birth_and_suffix.split_at(split);
    let neighbourhood = match suffix.trim().to_ascii_uppercase().as_str() {
        "" => Neighbourhood::Moore,
        "F" | "V" => Neighbourhood::VonNeumann,
        "C" | "E" | "H" => {
            return Err(field_error(
                field,
                format!(
                    "suffix `{suffix}` (corner, edge or hexahedral neighbourhood) is not supported; \
                     use no suffix (Moore) or F/V (face, von Neumann)"
                ),
            ));
        }
        _ => return Err(RuleError::Neighbourhood(suffix.to_string())),
    };
    let survive = parse_list(survive, "survival list", ListStyle::Itemised, neighbourhood)?;
    let birth = parse_list(birth, "birth list", ListStyle::Itemised, neighbourhood)?;
    Rule3D::new(survive, birth, 2, neighbourhood)
}
