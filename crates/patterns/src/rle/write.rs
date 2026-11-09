//! RLE writer following Golly's `writerle` conventions.

use std::fmt::Write as _;

use super::read::grid_placement;
use crate::pattern::{Cell, Pattern, sorted_unique};
use crate::rule::Rule;
use crate::text::value_lines;

/// Maximum length of a data line, as in Golly and the LifeWiki recommendation.
const MAX_LINE: usize = 70;

/// One RLE data token.
#[derive(Clone, Copy)]
enum Token {
    Dead,
    State(u8),
    EndOfRow,
    EndOfPattern,
}

/// Appends run-length tokens while keeping data lines within [`MAX_LINE`].
struct RunWriter<'a> {
    out: &'a mut String,
    line_len: usize,
    multistate: bool,
}

impl RunWriter<'_> {
    fn add(&mut self, run: u64, token: Token) {
        let count = if run > 1 { run.to_string() } else { String::new() };
        // Golly always reserves room for a two-letter state in multistate files.
        if self.line_len + count.len() + 1 + usize::from(self.multistate) > MAX_LINE {
            self.out.push('\n');
            self.line_len = 0;
        }
        self.out.push_str(&count);
        let before = self.out.len();
        match token {
            Token::Dead => self.out.push(if self.multistate { '.' } else { 'b' }),
            Token::EndOfRow => self.out.push('$'),
            Token::EndOfPattern => self.out.push('!'),
            Token::State(state) if !self.multistate => {
                debug_assert_eq!(state, 1);
                self.out.push('o');
            }
            Token::State(state) => {
                let mut state = state;
                if state > 24 {
                    let high = (state - 25) / 24;
                    self.out.push(char::from(b'p' + high));
                    state -= (high + 1) * 24;
                }
                self.out.push(char::from(b'A' + state - 1));
            }
        }
        self.line_len += count.len() + (self.out.len() - before);
    }
}

/// Number of cells from `from` up to (excluding) `to`, for `from <= to`.
fn distance(from: i128, to: i128) -> u64 {
    u64::try_from(to - from).unwrap_or(u64::MAX)
}

/// Rule text for the header and whether the rule has more than two states.
/// Rules this crate understands are written in Golly's canonical form (as
/// Golly itself does); other rule names (`LifeHistory`, `WireWorld`, ...) are
/// kept, minus any whitespace (a header rule value ends at the first
/// whitespace) and trailing commas (readers drop a comma ending the value).
/// `None` when nothing is left to write.
fn header_rule(rule: &str) -> Option<(String, bool)> {
    match Rule::parse_with_grid(rule) {
        Ok((parsed, grid)) => {
            let suffix = grid.map(|g| g.to_string()).unwrap_or_default();
            Some((format!("{parsed}{suffix}"), parsed.states() > 2))
        }
        Err(_) => {
            let text: String = rule.split_whitespace().collect();
            let text = text.trim_end_matches(',');
            (!text.is_empty()).then(|| (text.to_string(), false))
        }
    }
}

/// Writes `value` as `tag` lines (an empty value gives one bare tag).
fn push_tagged(out: &mut String, tag: &str, value: &str) {
    for line in value_lines(value) {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str(tag);
        } else {
            let _ = write!(out, "{tag} {line}");
        }
        out.push('\n');
    }
}

/// Writes `pattern` as extended RLE: `#CXRLE Pos=x,y` when a reader would
/// otherwise place the bounding box elsewhere (at the origin, or Golly's
/// position in a bounded grid), `#N` (name), `#O` (author), `#C` (description
/// then comments), the header `x = W, y = H[, rule = R]`, and run-length data
/// with lines of at most 70 characters, ending with `!`. Letters `.`/`A`..`X`/
/// `pA`.. are used when any cell has a state above 1 or the rule has more than
/// two states; otherwise `b`/`o`. Duplicate positions keep the later cell.
pub fn write(pattern: &Pattern) -> String {
    let cells = sorted_unique(&pattern.cells);
    let bbox = cells.first().zip(cells.last()).map(|(first, last)| {
        let min_x = cells.iter().map(|c| c.x).min().unwrap_or(first.x);
        let max_x = cells.iter().map(|c| c.x).max().unwrap_or(first.x);
        (min_x, first.y, max_x, last.y)
    });
    let (rule, many_states) = match pattern.rule.as_deref().and_then(header_rule) {
        Some((text, many)) => (Some(text), many),
        None => (None, false),
    };
    // Sizes as written in the header (2^64 for a box spanning all of i64) and
    // as the reader stores them (clamped to u64).
    let size = bbox.map(|(x0, y0, x1, y1)| (i128::from(x1) - i128::from(x0) + 1, i128::from(y1) - i128::from(y0) + 1));

    let mut out = String::new();
    if let (Some((x0, y0, _, _)), Some((width, height))) = (bbox, size) {
        let clamp = |size: i128| u64::try_from(size).unwrap_or(u64::MAX);
        let implied = rule.as_deref().and_then(|rule| grid_placement(rule, (clamp(width), clamp(height))));
        if implied.unwrap_or((0, 0)) != (x0, y0) {
            let _ = writeln!(out, "#CXRLE Pos={x0},{y0}");
        }
    }
    for (tag, value) in [("#N", &pattern.name), ("#O", &pattern.author), ("#C", &pattern.description)] {
        if !value.is_empty() {
            push_tagged(&mut out, tag, value);
        }
    }
    for comment in &pattern.comments {
        push_tagged(&mut out, "#C", comment);
    }

    let rule_suffix = rule.map(|r| format!(", rule = {r}")).unwrap_or_default();
    let (Some((x0, _, _, _)), Some((width, height))) = (bbox, size) else {
        let _ = write!(out, "x = 0, y = 0{rule_suffix}\n!\n");
        return out;
    };
    let _ = writeln!(out, "x = {width}, y = {height}{rule_suffix}");

    let multistate = many_states || cells.iter().any(|c| c.state > 1);
    let mut writer = RunWriter { out: &mut out, line_len: 0, multistate };
    let mut previous_y: Option<i64> = None;
    for row in cells.chunk_by(|a, b| a.y == b.y) {
        let y = row[0].y;
        let mut pending_rows = previous_y.map_or(0, |py| distance(i128::from(py), i128::from(y)));
        let mut cursor = i128::from(x0);
        for run in row.chunk_by(|a: &Cell, b: &Cell| b.x == a.x + 1 && b.state == a.state) {
            if pending_rows > 0 {
                writer.add(pending_rows, Token::EndOfRow);
                pending_rows = 0;
            }
            let start = i128::from(run[0].x);
            if start > cursor {
                writer.add(distance(cursor, start), Token::Dead);
            }
            writer.add(run.len() as u64, Token::State(run[0].state));
            cursor = i128::from(run[run.len() - 1].x) + 1;
        }
        previous_y = Some(y);
    }
    writer.add(1, Token::EndOfPattern);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rle::{parse, parse_document};

    #[test]
    fn writes_standard_glider() {
        let mut glider = parse("bo$2bo$3o!").unwrap();
        glider.name = "Glider".into();
        glider.author = "Richard K. Guy".into();
        glider.comments = vec!["The smallest spaceship.".into(), String::new()];
        glider.rule = Some("b3s23".into());
        let text = write(&glider);
        assert_eq!(
            text,
            "#N Glider\n#O Richard K. Guy\n#C The smallest spaceship.\n#C\nx = 3, y = 3, rule = B3/S23\nbo$2bo$3o!\n"
        );
    }

    #[test]
    fn blank_rows_dead_runs_and_position() {
        let p = Pattern::from_ascii("O...O\n.....\n.....\nOO.OO").unwrap().translated(-2, 5);
        let text = write(&p);
        assert_eq!(text, "#CXRLE Pos=-2,5\nx = 5, y = 4\no3bo3$2ob2o!\n");
        let doc = parse_document(&text).unwrap();
        assert_eq!(doc.pattern.cells, p.cells);
        assert_eq!(doc.position, Some((-2, 5)));
    }

    #[test]
    fn multistate_letters() {
        let p = Pattern::new(vec![Cell::new(0, 0, 1), Cell::new(1, 0, 2), Cell::new(3, 0, 25), Cell::new(0, 1, 255)])
            .with_rule("/2/3");
        let text = write(&p);
        assert_eq!(text, "x = 4, y = 2, rule = /2/3\nAB.pA$yO!\n");
        assert_eq!(parse(&text).unwrap().cells, p.cells);
        // A Generations rule switches to letters even for state-1 cells, like Golly.
        let brain = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("B2/S/C3");
        assert_eq!(write(&brain), "x = 1, y = 1, rule = /2/3\nA!\n");
    }

    #[test]
    fn position_is_written_whenever_a_reader_would_place_the_pattern_elsewhere() {
        let block = Pattern::from_ascii("OO\nOO").unwrap();
        // In a bounded grid a reader centres a file without `Pos` (as Golly does)...
        let at_origin = block.clone().with_rule("B3/S23:T10,10");
        let text = write(&at_origin);
        assert!(text.starts_with("#CXRLE Pos=0,0\n"), "{text}");
        assert_eq!(parse(&text).unwrap().cells, at_origin.cells);
        // ...so a pattern already at that position needs no `Pos` line.
        let centred = block.translated(-1, -1).with_rule("B3/S23:T10,10");
        let text = write(&centred);
        assert!(!text.contains("#CXRLE"), "{text}");
        assert_eq!(parse(&text).unwrap().cells, centred.cells);
        assert!(!write(&block).contains("#CXRLE"));
    }

    #[test]
    fn keeps_unknown_rules_and_grid_suffixes() {
        let p = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("LifeHistory:T30+7,20");
        assert!(write(&p).contains("rule = LifeHistory:T30+7,20\n"));
        // A header rule value ends at whitespace, so whitespace is dropped and a blank rule is omitted.
        let spaced = Pattern::new(vec![Cell::alive(0, 0)]).with_rule(" Life History ");
        let text = write(&spaced);
        assert_eq!(text, "x = 1, y = 1, rule = LifeHistory\no!\n");
        assert_eq!(parse(&text).unwrap().rule.as_deref(), Some("LifeHistory"));
        assert_eq!(write(&Pattern::new(vec![Cell::alive(0, 0)]).with_rule("  ")), "x = 1, y = 1\no!\n");
        assert_eq!(write(&Pattern::new(vec![Cell::alive(0, 0)]).with_rule(",")), "x = 1, y = 1\no!\n");
        let comma = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("R1,C2,");
        assert_eq!(parse(&write(&comma)).unwrap().rule.as_deref(), Some("R1,C2"));
        let p = Pattern::new(vec![Cell::alive(0, 0)]).with_rule("23/3:t10");
        assert!(write(&p).contains("rule = B3/S23:T10,10\n"));
    }

    #[test]
    fn lines_never_exceed_70_characters() {
        // A checkerboard row forces many short tokens.
        let cells: Vec<Cell> = (0..300).filter(|x| x % 2 == 0).map(|x| Cell::alive(x, 0)).collect();
        let p = Pattern::new(cells.clone());
        let text = write(&p);
        let data: Vec<&str> = text.lines().skip(1).collect();
        assert!(data.len() > 1);
        assert!(data.iter().all(|line| line.len() <= 70), "{text}");
        assert_eq!(parse(&text).unwrap().cells, cells);
        // Multistate lines too.
        let cells: Vec<Cell> = (0..200).map(|x| Cell::new(x * 2, 0, (x % 255 + 1) as u8)).collect();
        let text = write(&Pattern::new(cells.clone()));
        assert!(text.lines().all(|line| line.len() <= 70), "{text}");
        assert_eq!(parse(&text).unwrap().cells, cells);
    }

    #[test]
    fn empty_and_duplicates() {
        assert_eq!(write(&Pattern::default()), "x = 0, y = 0\n!\n");
        assert_eq!(write(&Pattern::default().with_rule("B3/S23")), "x = 0, y = 0, rule = B3/S23\n!\n");
        let dup = Pattern::new(vec![Cell::alive(0, 0), Cell::new(0, 0, 3)]);
        assert_eq!(write(&dup), "x = 1, y = 1\nC!\n");
    }

    #[test]
    fn description_and_multiline_metadata() {
        let mut p = Pattern::new(vec![Cell::alive(0, 0)]);
        p.name = "two\nlines".into();
        p.description = "Described here.".into();
        p.comments = vec!["after".into()];
        let text = write(&p);
        assert_eq!(text, "#N two\n#N lines\n#C Described here.\n#C after\nx = 1, y = 1\no!\n");
        let back = parse(&text).unwrap();
        assert_eq!(back.name, "two\nlines");
        assert_eq!(back.comments, vec!["Described here.", "after"]);
    }

    #[test]
    fn far_apart_cells_stay_sparse() {
        let far = 1i64 << 61;
        let p = Pattern::from_live_cells([(-far, -far), (far, far)]);
        let text = write(&p);
        assert!(text.len() < 200, "{text}");
        assert!(text.contains("x = 4611686018427387905, y = 4611686018427387905"), "{text}");
        assert_eq!(parse(&text).unwrap().cells, p.cells);
        // Extreme spans are still written compactly (the header uses 128-bit
        // sizes) and read back exactly.
        let extreme = Pattern::from_live_cells([(i64::MIN, i64::MIN), (i64::MAX, i64::MAX)]);
        let text = write(&extreme);
        assert!(text.contains("x = 18446744073709551616, y = 18446744073709551616"), "{text}");
        assert_eq!(parse(&text).unwrap().cells, extreme.cells);
    }

    #[test]
    fn line_breaks_inside_metadata_cannot_leak_into_the_data() {
        let mut p = Pattern::new(vec![Cell::alive(0, 0)]);
        p.name = "a\rbo".into();
        p.author = "x = 1\r\ny = 2".into();
        p.comments = vec!["c\r3o".into(), String::new()];
        let text = write(&p);
        assert_eq!(text, "#N a\n#N bo\n#O x = 1\n#O y = 2\n#C c\n#C 3o\n#C\nx = 1, y = 1\no!\n");
        let back = parse(&text).unwrap();
        assert_eq!(back.cells, p.cells);
        assert_eq!((back.name.as_str(), back.author.as_str()), ("a\nbo", "x = 1\ny = 2"));
        assert_eq!(back.comments, vec!["c", "3o", ""]);
    }
}
