//! Life 1.05 (`.lif`, `.life`).
//!
//! ```text
//! #Life 1.05
//! #D Acorn
//! #N                  (normal Conway rules)  or  #R 23/3 (S/B rule)
//! #P -3 -1            top-left of the next block of rows
//! .*
//! ...*
//! **..***
//! ```
//!
//! `#D` (and `#C`) lines are comments; `#D Name:` / `#D Author:` lines, as
//! written by [`write()`], fill the name and author. Rows use `*` for live and
//! `.` for dead cells, may omit trailing dead cells, and continue downward
//! from the last `#P`. A `#P` without coordinates continues below the
//! previous block (Golly's behaviour), and rows before any `#P` start at
//! `(0, 0)`. The header may be missing (WinLifeSearch clipboard output).
//! Cells are returned sorted by `y` then `x` with duplicates merged.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::PatternError;
use crate::format::Format;
use crate::pattern::{Cell, MAX_CELLS, Pattern, sorted_unique};
use crate::plaintext::{check_dense_area, check_two_state, comment_lines, metadata};
use crate::rule::{LifeRule, Rule};
use crate::text::{lines, single_line, skip_indent};

/// Header line of the format.
pub const HEADER: &str = "#Life 1.05";

/// Maximum characters per row line; wider patterns are split into blocks.
const MAX_ROW: i64 = 80;

fn error(line: usize, column: usize, message: impl Into<String>) -> PatternError {
    PatternError::syntax(Format::Life105, line, column, message)
}

/// Parses Life 1.05 text. Cells keep the absolute coordinates given by `#P`.
pub fn parse(text: &str) -> Result<Pattern, PatternError> {
    let mut pattern = Pattern::default();
    let (mut left, mut y) = (0i64, 0i64);
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
                return Err(error(line_no, column, format!("expected `{HEADER}`, found `{trimmed}`")));
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let mut chars = rest.chars();
            let tag = chars.next();
            let body = chars.as_str();
            match tag {
                Some('N') => pattern.rule = Some("B3/S23".to_string()),
                Some('R') => {
                    let rule = body.trim();
                    if rule.is_empty() {
                        return Err(error(line_no, column + 2, "`#R` needs a rule"));
                    }
                    pattern.rule = Some(rule.to_string());
                }
                Some('P') => {
                    let coordinates = body.trim();
                    if !coordinates.is_empty() {
                        let mut parts = coordinates.split_whitespace().map(str::parse::<i64>);
                        match (parts.next(), parts.next(), parts.next()) {
                            (Some(Ok(px)), Some(Ok(py)), None) => (left, y) = (px, py),
                            _ => {
                                return Err(error(
                                    line_no,
                                    column + 2,
                                    format!("expected `#P x y`, found `{trimmed}`"),
                                ));
                            }
                        }
                    }
                }
                _ => {
                    let text = body.strip_prefix(' ').unwrap_or(body);
                    match (metadata(text, "name:"), metadata(text, "author:")) {
                        (Some(name), _) if pattern.name.is_empty() => pattern.name = name.to_string(),
                        (_, Some(author)) if pattern.author.is_empty() => pattern.author = author.to_string(),
                        _ => pattern.comments.push(text.to_string()),
                    }
                }
            }
            continue;
        }
        for (offset, ch) in trimmed.chars().enumerate() {
            match ch {
                '.' => {}
                '*' => {
                    if pattern.cells.len() >= MAX_CELLS {
                        return Err(PatternError::LimitExceeded(format!(
                            "Life 1.05 line {line_no}: more than {MAX_CELLS} live cells"
                        )));
                    }
                    let x = left
                        .checked_add(offset as i64)
                        .ok_or_else(|| error(line_no, column + offset, "coordinates overflow"))?;
                    pattern.cells.push(Cell::alive(x, y));
                }
                _ => {
                    return Err(error(
                        line_no,
                        column + offset,
                        format!("unexpected character '{ch}' (Life 1.05 rows use '.' and '*')"),
                    ));
                }
            }
        }
        y = y.checked_add(1).ok_or_else(|| error(line_no, column, "coordinates overflow"))?;
    }
    // Blocks may come in any order and may overlap.
    pattern.cells = sorted_unique(&pattern.cells);
    Ok(pattern)
}

/// The `#N` / `#R` line for a rule: `#N` for Conway's Life on the unbounded
/// plane, the S/B form the format specifies for other Life-like rules (with
/// any bounded-grid suffix, which Golly accepts there), the text on one line
/// otherwise. `None` for a blank rule.
fn rule_line(rule: &str) -> Option<String> {
    match Rule::parse_with_grid(rule) {
        Ok((Rule::Life(life), None)) if life == LifeRule::conway() => Some("#N".to_string()),
        Ok((Rule::Life(life), grid)) => {
            let digits = |counts: Vec<u8>| counts.into_iter().map(|n| char::from(b'0' + n)).collect::<String>();
            let suffix = grid.map(|grid| grid.to_string()).unwrap_or_default();
            Some(format!("#R {}/{}{suffix}", digits(life.survival_counts()), digits(life.birth_counts())))
        }
        _ => {
            let text = single_line(rule);
            (!text.is_empty()).then(|| format!("#R {text}"))
        }
    }
}

/// Writes Life 1.05: header, `#D Name:` / `#D Author:` / `#D` description and
/// comment lines, `#N` or `#R`, then blocks of at most 80 columns, each
/// introduced by `#P x y` with absolute coordinates. Empty rows inside a
/// block are written as `.`. Multi-state patterns and bounding boxes larger
/// than [`crate::pattern::MAX_DENSE_AREA`] are rejected.
pub fn write(pattern: &Pattern) -> Result<String, PatternError> {
    check_two_state(pattern, Format::Life105)?;
    check_dense_area(pattern, Format::Life105)?;
    let mut out = String::from(HEADER);
    out.push('\n');
    if !pattern.name.is_empty() {
        let _ = writeln!(out, "#D Name: {}", single_line(&pattern.name));
    }
    if !pattern.author.is_empty() {
        let _ = writeln!(out, "#D Author: {}", single_line(&pattern.author));
    }
    for line in comment_lines(pattern) {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str("#D\n");
        } else {
            let _ = writeln!(out, "#D {line}");
        }
    }
    if let Some(line) = pattern.rule.as_deref().and_then(rule_line) {
        out.push_str(&line);
        out.push('\n');
    }
    let Some((x0, _, _, _)) = pattern.bounding_box() else { return Ok(out) };
    // Vertical strips of MAX_ROW columns, each written as one block.
    let mut strips: BTreeMap<i64, Vec<Cell>> = BTreeMap::new();
    for cell in sorted_unique(&pattern.cells) {
        strips.entry((cell.x - x0) / MAX_ROW).or_default().push(cell);
    }
    for (strip, cells) in strips {
        let left = x0 + strip * MAX_ROW;
        let top = cells[0].y;
        let _ = writeln!(out, "#P {left} {top}");
        let mut next_y = top;
        for row in cells.chunk_by(|a, b| a.y == b.y) {
            for _ in next_y..row[0].y {
                out.push_str(".\n");
            }
            let mut line = vec![b'.'; (row[row.len() - 1].x - left + 1) as usize];
            for cell in row {
                line[(cell.x - left) as usize] = b'*';
            }
            out.extend(line.into_iter().map(char::from));
            out.push('\n');
            next_y = row[0].y + 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_golly_style_file() {
        let text = "#Life 1.05\n#D Acorn\n#D The most vigorously growing 7-cell methuselah.\n#P\n.*\n...*\n**..***\n";
        let p = parse(text).unwrap();
        assert_eq!(p.comments, vec!["Acorn", "The most vigorously growing 7-cell methuselah."]);
        assert_eq!(p.population(), 7);
        assert_eq!(p.to_ascii(), ".O.....\n...O...\nOO..OOO\n");
        assert_eq!(p.bounding_box(), Some((0, 0, 6, 2)));
    }

    #[test]
    fn blocks_rules_and_positions() {
        let text = "#Life 1.05\n#R 23/36\n#P -2 -1\n**\n\n.*\n#P 5 5\n*\n#P\n*\n";
        let p = parse(text).unwrap();
        assert_eq!(p.rule.as_deref(), Some("23/36"));
        assert_eq!(
            p.cells,
            vec![Cell::alive(-2, -1), Cell::alive(-1, -1), Cell::alive(-1, 0), Cell::alive(5, 5), Cell::alive(5, 6)]
        );
        assert_eq!(parse("#Life 1.05\n#N\n").unwrap().rule.as_deref(), Some("B3/S23"));
        assert_eq!(parse("#P 1 1\n*\n").unwrap().cells, vec![Cell::alive(1, 1)], "header is optional");
    }

    #[test]
    fn errors() {
        let err = parse("#Life 1.05\n#P 0 0\n.*o\n").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(3), Some(3)));
        assert!(parse("#Life 1.05\n#P 0\n").is_err());
        assert!(parse("#Life 1.05\n#R\n").is_err());
        assert!(parse("#Life 1.06\n").is_err());
    }

    #[test]
    fn writes_and_round_trips() {
        let mut p = Pattern::from_ascii(".O.\n..O\nOOO").unwrap().translated(-1, -1).with_rule("B3/S23");
        p.name = "Glider".into();
        p.comments = vec!["Moves diagonally.".into()];
        let text = write(&p).unwrap();
        assert_eq!(text, "#Life 1.05\n#D Name: Glider\n#D Moves diagonally.\n#N\n#P -1 -1\n.*\n..*\n***\n");
        let back = parse(&text).unwrap();
        assert_eq!(back.name, "Glider");
        assert_eq!(back.comments, p.comments);
        assert_eq!(back.rule.as_deref(), Some("B3/S23"));
        assert_eq!(back.cells, p.cells);
        let high = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("B36/S23");
        assert!(write(&high).unwrap().contains("#R 23/36\n"));
        // A bounded grid is kept (Golly reads `#R 23/3:T10,10`), so Life on a torus is not `#N`.
        let torus = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("B3/S23:t10");
        let text = write(&torus).unwrap();
        assert!(text.contains("#R 23/3:T10,10\n"), "{text}");
        let (rule, grid) = Rule::parse_with_grid(parse(&text).unwrap().rule.as_deref().unwrap()).unwrap();
        assert_eq!((rule, grid), (Rule::Life(LifeRule::conway()), Some(crate::rule::BoundedGrid::torus(10, 10))));
        // Rules this crate cannot parse are kept, on one line; a blank rule is omitted.
        let history = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("LifeHistory");
        assert!(write(&history).unwrap().contains("#R LifeHistory\n"));
        let broken = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("Life\r\nHistory");
        assert_eq!(parse(&write(&broken).unwrap()).unwrap().rule.as_deref(), Some("Life History"));
        let blank = Pattern::new(vec![Cell::alive(0, 0)]).with_rule(" ");
        assert_eq!(write(&blank).unwrap(), "#Life 1.05\n#P 0 0\n*\n");
        assert_eq!(Rule::parse("23/36").unwrap().to_string(), "B36/S23");
    }

    #[test]
    fn metadata_line_breaks_stay_in_comments() {
        let mut p = Pattern::new(vec![Cell::alive(0, 0)]);
        p.author = "A\rB".into();
        p.comments = vec!["note\r**".into()];
        let text = write(&p).unwrap();
        assert_eq!(text, "#Life 1.05\n#D Author: A B\n#D note\n#D **\n#P 0 0\n*\n");
        let back = parse(&text).unwrap();
        assert_eq!(back.cells, p.cells);
        assert_eq!((back.author.as_str(), back.comments.clone()), ("A B", vec!["note".to_string(), "**".to_string()]));
    }

    #[test]
    fn wide_patterns_are_split_into_blocks() {
        let cells: Vec<Cell> = [0, 79, 80, 200].iter().map(|&x| Cell::alive(x, x / 10)).collect();
        let p = Pattern::new(cells.clone());
        let text = write(&p).unwrap();
        assert!(text.lines().all(|line| line.len() <= 80), "{text}");
        assert_eq!(text.matches("#P ").count(), 3);
        assert_eq!(parse(&text).unwrap().cells, sorted_unique(&cells));
        assert!(write(&Pattern::new(vec![Cell::new(0, 0, 2)])).is_err());
    }
}
