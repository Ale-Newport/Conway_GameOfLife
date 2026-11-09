//! Life 1.06 (`.lif`, `.life`): a `#Life 1.06` header followed by one live
//! cell per line as `x y` (y grows downward, as in Golly).
//!
//! The reader also accepts the extensions Golly honours in this format —
//! `#D`/`#C` comment lines, `#N` (normal Conway rules) and `#R rule` — and a
//! missing header (when the format was chosen from the file extension).
//! Duplicate coordinates are merged. The writer produces the strict format
//! (header and coordinates only), so names, comments and rules are not
//! stored; use RLE to keep them.

use std::fmt::Write as _;

use crate::error::PatternError;
use crate::format::Format;
use crate::pattern::{Cell, MAX_CELLS, Pattern, sorted_unique};
use crate::plaintext::check_two_state;
use crate::text::{lines, skip_indent};

/// Header line of the format.
pub const HEADER: &str = "#Life 1.06";

/// Parses Life 1.06 text. Cells keep their absolute coordinates and are
/// returned sorted by `y` then `x`.
pub fn parse(text: &str) -> Result<Pattern, PatternError> {
    let mut pattern = Pattern::default();
    let mut first_line = true;
    for (line_no, line) in lines(text) {
        let (trimmed, column) = skip_indent(line);
        let trimmed = trimmed.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let is_first = std::mem::replace(&mut first_line, false);
        if is_first && trimmed.starts_with("#Life") {
            if trimmed != HEADER {
                return Err(PatternError::syntax(
                    Format::Life106,
                    line_no,
                    column,
                    format!("expected `{HEADER}`, found `{trimmed}`"),
                ));
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let mut chars = rest.chars();
            match chars.next() {
                Some('N') => pattern.rule = Some("B3/S23".to_string()),
                Some('R') => {
                    let rule = chars.as_str().trim();
                    if rule.is_empty() {
                        return Err(PatternError::syntax(Format::Life106, line_no, column + 2, "`#R` needs a rule"));
                    }
                    pattern.rule = Some(rule.to_string());
                }
                Some(_) => {
                    pattern.comments.push(chars.as_str().strip_prefix(' ').unwrap_or(chars.as_str()).to_string())
                }
                None => pattern.comments.push(String::new()),
            }
            continue;
        }
        let cell = parse_coordinates(trimmed).ok_or_else(|| {
            PatternError::syntax(
                Format::Life106,
                line_no,
                column,
                format!("expected two integers `x y`, found `{trimmed}`"),
            )
        })?;
        if pattern.cells.len() >= MAX_CELLS {
            return Err(PatternError::LimitExceeded(format!("Life 1.06 line {line_no}: more than {MAX_CELLS} cells")));
        }
        pattern.cells.push(cell);
    }
    pattern.cells = sorted_unique(&pattern.cells);
    Ok(pattern)
}

fn parse_coordinates(text: &str) -> Option<Cell> {
    let mut parts = text.split_whitespace();
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some(Cell::alive(x, y))
}

/// Writes the strict format: the header and one `x y` line per live cell,
/// sorted by `y` then `x`, at absolute coordinates. Multi-state patterns are
/// rejected.
pub fn write(pattern: &Pattern) -> Result<String, PatternError> {
    check_two_state(pattern, Format::Life106)?;
    let mut out = String::from(HEADER);
    out.push('\n');
    for cell in sorted_unique(&pattern.cells) {
        let _ = writeln!(out, "{} {}", cell.x, cell.y);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coordinates_and_extensions() {
        let text = "#Life 1.06\n#D A glider\n#N\n0 -1\n1 0\r\n-1 1\n 0 1\n1 1\n1 1\n";
        let p = parse(text).unwrap();
        assert_eq!(p.comments, vec!["A glider"]);
        assert_eq!(p.rule.as_deref(), Some("B3/S23"));
        assert_eq!(p.population(), 5, "duplicates are merged");
        assert_eq!(p.bounding_box(), Some((-1, -1, 1, 1)));
        assert_eq!(p.to_ascii(), ".O.\n..O\nOOO\n");
        assert_eq!(parse("#Life 1.06\n#R B36/S23\n").unwrap().rule.as_deref(), Some("B36/S23"));
        assert_eq!(parse("3 4\n").unwrap().cells, vec![Cell::alive(3, 4)], "header is optional");
    }

    #[test]
    fn errors() {
        let err = parse("#Life 1.06\n0 0\n1 x\n").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(3), Some(1)));
        assert!(parse("#Life 1.06\n1 2 3\n").is_err());
        assert!(parse("#Life 1.05\n").unwrap_err().to_string().contains("expected `#Life 1.06`"));
        assert!(parse("#Life 1.06\n#R\n").is_err());
    }

    #[test]
    fn writes_and_round_trips() {
        let p = parse("#Life 1.06\n0 -1\n1 0\n-1 1\n0 1\n1 1\n").unwrap();
        let text = write(&p).unwrap();
        assert_eq!(text, "#Life 1.06\n0 -1\n1 0\n-1 1\n0 1\n1 1\n");
        assert_eq!(parse(&text).unwrap().cells, p.cells);
        assert!(write(&Pattern::new(vec![Cell::new(0, 0, 3)])).is_err());
        assert_eq!(write(&Pattern::default()).unwrap(), "#Life 1.06\n");
    }
}
