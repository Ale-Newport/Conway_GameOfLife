//! Plaintext (`.cells`) format.
//!
//! ```text
//! !Name: Glider
//! !Author: Richard K. Guy
//! !The smallest, most common, and first discovered spaceship.
//! .O.
//! ..O
//! OOO
//! ```
//!
//! Lines starting with `!` are comments; `!Name:` and `!Author:` (any case)
//! fill the pattern name and author. Every other line is a row: `.` is dead,
//! `O` is alive (`o` and the historical `*` are accepted too). Rows may omit
//! trailing dead cells; blank lines after the first row are empty rows (blank
//! lines before it are ignored). Plaintext is two-state only.

use crate::error::PatternError;
use crate::format::Format;
use crate::pattern::{Cell, MAX_CELLS, MAX_DENSE_AREA, Pattern, sorted_unique};
use crate::text::{lines, single_line, value_lines};

/// Strips a case-insensitive `key:` prefix (after optional spaces).
pub(crate) fn metadata<'a>(comment: &'a str, key: &str) -> Option<&'a str> {
    let trimmed = comment.trim_start();
    let head = trimmed.get(..key.len())?;
    head.eq_ignore_ascii_case(key).then(|| trimmed[key.len()..].trim())
}

/// Parses plaintext into a [`Pattern`] (rows start at `(0, 0)`).
pub fn parse(text: &str) -> Result<Pattern, PatternError> {
    let mut pattern = Pattern::default();
    let mut y: i64 = 0;
    let mut started = false;
    for (line_no, line) in lines(text) {
        if let Some(comment) = line.strip_prefix('!') {
            let comment = comment.trim_end();
            match (metadata(comment, "name:"), metadata(comment, "author:")) {
                (Some(name), _) if pattern.name.is_empty() => pattern.name = name.to_string(),
                (_, Some(author)) if pattern.author.is_empty() => pattern.author = author.to_string(),
                _ => pattern.comments.push(comment.to_string()),
            }
            continue;
        }
        let row = line.trim_end();
        if row.is_empty() {
            if started {
                y += 1;
            }
            continue;
        }
        started = true;
        for (x, ch) in row.chars().enumerate() {
            match ch {
                '.' => {}
                'O' | 'o' | '*' => {
                    if pattern.cells.len() >= MAX_CELLS {
                        return Err(PatternError::LimitExceeded(format!(
                            "plaintext line {line_no}: more than {MAX_CELLS} live cells"
                        )));
                    }
                    pattern.cells.push(Cell::alive(x as i64, y));
                }
                _ => {
                    return Err(PatternError::syntax(
                        Format::Plaintext,
                        line_no,
                        x + 1,
                        format!("unexpected character '{ch}' (plaintext uses '.' for dead and 'O' for live cells)"),
                    ));
                }
            }
        }
        y += 1;
    }
    Ok(pattern)
}

/// The description (when not empty) and the comments, one output line each.
pub(crate) fn comment_lines(pattern: &Pattern) -> impl Iterator<Item = &str> {
    let description = (!pattern.description.is_empty()).then_some(pattern.description.as_str());
    description.into_iter().chain(pattern.comments.iter().map(String::as_str)).flat_map(value_lines)
}

/// Rejects patterns a two-state dense format cannot hold.
pub(crate) fn check_two_state(pattern: &Pattern, format: Format) -> Result<(), PatternError> {
    if pattern.max_state() > 1 {
        return Err(PatternError::Unsupported(format!(
            "{format} stores only two states; this pattern has cells in state {} (use RLE)",
            pattern.max_state()
        )));
    }
    Ok(())
}

/// Rejects bounding boxes too large to write densely.
pub(crate) fn check_dense_area(pattern: &Pattern, format: Format) -> Result<(), PatternError> {
    let area = u128::from(pattern.width()) * u128::from(pattern.height());
    if area > MAX_DENSE_AREA {
        return Err(PatternError::LimitExceeded(format!(
            "a {} x {} bounding box is too large for {format} (limit {MAX_DENSE_AREA} cells; use RLE)",
            pattern.width(),
            pattern.height()
        )));
    }
    Ok(())
}

/// Writes `pattern` as plaintext: `!Name:`, `!Author:`, the description and
/// comments as `!` lines, then every row of the bounding box padded to its
/// full width. The bounding box's top-left becomes the origin. Multi-state
/// patterns and bounding boxes larger than [`MAX_DENSE_AREA`] are rejected.
pub fn write(pattern: &Pattern) -> Result<String, PatternError> {
    check_two_state(pattern, Format::Plaintext)?;
    check_dense_area(pattern, Format::Plaintext)?;
    let mut out = String::new();
    if !pattern.name.is_empty() {
        out.push_str(&format!("!Name: {}\n", single_line(&pattern.name)));
    }
    if !pattern.author.is_empty() {
        out.push_str(&format!("!Author: {}\n", single_line(&pattern.author)));
    }
    for line in comment_lines(pattern) {
        out.push('!');
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let Some((x0, y0, _, _)) = pattern.bounding_box() else { return Ok(out) };
    let width = pattern.width() as usize;
    let cells = sorted_unique(&pattern.cells);
    let mut row = vec![b'.'; width];
    let mut next_y = y0;
    for chunk in cells.chunk_by(|a, b| a.y == b.y) {
        let y = chunk[0].y;
        for _ in next_y..y {
            out.push_str(&".".repeat(width));
            out.push('\n');
        }
        row.fill(b'.');
        for cell in chunk {
            row[(cell.x - x0) as usize] = b'O';
        }
        out.extend(row.iter().map(|&b| char::from(b)));
        out.push('\n');
        next_y = y + 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLIDER: &str = "!Name: Glider\n!Author: Richard K. Guy\n!The smallest spaceship.\n!\n.O.\n..O\nOOO\n";

    #[test]
    fn parses_lifewiki_style_file() {
        let p = parse(GLIDER).unwrap();
        assert_eq!(p.name, "Glider");
        assert_eq!(p.author, "Richard K. Guy");
        assert_eq!(p.comments, vec!["The smallest spaceship.", ""]);
        assert_eq!(p.to_ascii(), ".O.\n..O\nOOO\n");
        assert_eq!(p.bounding_box(), Some((0, 0, 2, 2)));
    }

    #[test]
    fn accepts_variants() {
        // Star cells, trailing dead cells omitted, CRLF, blank rows.
        let p = parse("!name:  Thing \r\n\r\n*.*\r\n\r\n.o\r\n").unwrap();
        assert_eq!(p.name, "Thing");
        assert_eq!(p.cells, vec![Cell::alive(0, 0), Cell::alive(2, 0), Cell::alive(1, 2)]);
    }

    #[test]
    fn reports_bad_characters() {
        let err = parse("!x\n.O.\n.X.\n").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(3), Some(2)));
        assert!(err.to_string().contains("unexpected character 'X'"));
    }

    #[test]
    fn writes_and_round_trips() {
        let p = parse(GLIDER).unwrap();
        let text = write(&p).unwrap();
        assert_eq!(text, GLIDER);
        assert_eq!(parse(&text).unwrap(), p);
        // Blank rows in the middle are written as full-width dead rows.
        let gap = Pattern::from_ascii("O.O\n...\n...\n.O.").unwrap().translated(5, -3);
        let text = write(&gap).unwrap();
        assert_eq!(text, "O.O\n...\n...\n.O.\n");
        assert_eq!(parse(&text).unwrap().cells, gap.normalized().cells);
    }

    #[test]
    fn metadata_line_breaks_stay_in_comments() {
        let mut p = Pattern::new(vec![Cell::alive(0, 0)]);
        p.name = "two\rparts".into();
        p.description = "first\rOO".into();
        p.comments = vec![String::new(), "x\r\n.O".into()];
        let text = write(&p).unwrap();
        assert_eq!(text, "!Name: two parts\n!first\n!OO\n!\n!x\n!.O\nO\n");
        let back = parse(&text).unwrap();
        assert_eq!(back.cells, p.cells);
        assert_eq!(back.comments, vec!["first", "OO", "", "x", ".O"]);
    }

    #[test]
    fn rejects_unrepresentable_patterns() {
        let multi = Pattern::new(vec![Cell::new(0, 0, 2)]);
        assert!(matches!(write(&multi), Err(PatternError::Unsupported(_))));
        let wide = Pattern::from_live_cells([(0, 0), (1 << 40, 0)]);
        assert!(matches!(write(&wide), Err(PatternError::LimitExceeded(_))));
        assert_eq!(write(&Pattern::default()).unwrap(), "");
        assert_eq!(write(&Pattern::default().with_name("Empty")).unwrap(), "!Name: Empty\n");
    }
}
