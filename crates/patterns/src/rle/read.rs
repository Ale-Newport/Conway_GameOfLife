//! RLE reader.

use std::num::IntErrorKind;

use crate::error::PatternError;
use crate::format::Format;
use crate::pattern::{Cell, MAX_CELLS, Pattern};
use crate::rule::BoundedGrid;
use crate::text::{lines, skip_indent};

/// A parsed RLE file with the information that does not fit in [`Pattern`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RleDocument {
    /// The pattern; cells are already offset by [`RleDocument::position`].
    pub pattern: Pattern,
    /// `(x, y)` from the header line, if present. Golly treats these as
    /// informational: they need not match the cells. Values above
    /// `u64::MAX` (a bounding box spanning the whole `i64` range is
    /// 2^64 cells wide) are clamped to `u64::MAX`.
    pub declared_size: Option<(u64, u64)>,
    /// Generation count from `#CXRLE Gen=`.
    pub generation: Option<u64>,
    /// Top-left corner of the encoded rectangle, when the file places it:
    /// `#CXRLE Pos=`, else Golly's placement in a bounded grid (header rule
    /// with a suffix such as `:T100,100` and no `Pos`), else `#P`/`#R`.
    /// `None` means the rectangle starts at `(0, 0)`.
    pub position: Option<(i64, i64)>,
}

/// Parses RLE text into a [`Pattern`]. See the [module documentation](super).
pub fn parse(text: &str) -> Result<Pattern, PatternError> {
    parse_document(text).map(|document| document.pattern)
}

/// Parses RLE text, also returning header size, generation and position.
pub fn parse_document(text: &str) -> Result<RleDocument, PatternError> {
    let mut reader = Reader::default();
    for (line_no, line) in lines(text) {
        if reader.finished {
            break;
        }
        reader.line(line_no, line)?;
    }
    Ok(reader.finish())
}

fn error(line: usize, column: usize, message: impl Into<String>) -> PatternError {
    PatternError::syntax(Format::Rle, line, column, message)
}

/// Text of a comment line after its two-character tag: one separating space
/// is removed and trailing whitespace trimmed.
fn comment_text(rest: &str) -> String {
    rest.strip_prefix(' ').unwrap_or(rest).trim_end().to_string()
}

/// Parses `"x y"` (two integers separated by whitespace).
fn parse_coordinates(text: &str) -> Option<(i64, i64)> {
    let mut parts = text.split_whitespace();
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((x, y))
}

/// Whether a line is the `x = ..., y = ...` header: it starts with the key
/// `x` (or `y`, so the keys may come in any order) followed by `=`.
pub(crate) fn is_header(line: &str) -> bool {
    line.strip_prefix(['x', 'y']).is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Golly's placement of a pattern read into a bounded grid when the file has
/// no `#CXRLE Pos` (`readrle`): centred on the origin when the declared size
/// fits inside the grid, otherwise at the grid's top-left corner.
pub(super) fn grid_placement(rule: &str, (width, height): (u64, u64)) -> Option<(i64, i64)> {
    let (_, suffix) = rule.split_once(':')?;
    let grid = BoundedGrid::parse(suffix).ok()??;
    let fits = |size: u64, limit: u32| size > 0 && (limit == 0 || size <= u64::from(limit));
    let half = |size: u64| -i64::try_from(size / 2).unwrap_or(i64::MAX);
    Some(if fits(width, grid.width) && fits(height, grid.height) {
        (half(width), half(height))
    } else {
        grid.top_left()
    })
}

/// Parses a header size; Golly treats it as informational, so a value too
/// large for `u64` is clamped instead of rejected.
fn parse_size(value: &str) -> Option<u64> {
    match value.parse::<u64>() {
        Ok(number) => Some(number),
        Err(error) if *error.kind() == IntErrorKind::PosOverflow => Some(u64::MAX),
        Err(_) => None,
    }
}

#[derive(Default)]
struct Reader {
    pattern: Pattern,
    names: Vec<String>,
    authors: Vec<String>,
    /// Still before any line other than `#CXRLE`?
    past_top: bool,
    header_seen: bool,
    data_started: bool,
    finished: bool,
    declared_size: Option<(u64, u64)>,
    generation: Option<u64>,
    xrle_position: Option<(i64, i64)>,
    grid_position: Option<(i64, i64)>,
    comment_position: Option<(i64, i64)>,
    /// Absolute coordinates of the encoded rectangle's top-left corner.
    origin: (i128, i128),
    /// Cursor relative to `origin`. 128 bits so that a rectangle spanning the
    /// whole `i64` range (2^64 cells) can be read.
    x: i128,
    y: i128,
}

impl Reader {
    /// Top-left corner of the encoded rectangle (see [`RleDocument::position`]).
    fn placement(&self) -> Option<(i64, i64)> {
        self.xrle_position.or(self.grid_position).or(self.comment_position)
    }

    fn line(&mut self, line_no: usize, line: &str) -> Result<(), PatternError> {
        let (trimmed, indent_column) = skip_indent(line);
        if trimmed.trim_end().is_empty() {
            return Ok(());
        }
        if trimmed.starts_with('#') {
            return self.comment(line_no, indent_column, trimmed);
        }
        self.past_top = true;
        if is_header(trimmed) {
            if self.data_started {
                return Err(error(line_no, indent_column, "the header line must come before the pattern data"));
            }
            if self.header_seen {
                return Err(error(line_no, indent_column, "duplicate header line"));
            }
            self.header_seen = true;
            return self.header(line_no, indent_column, trimmed);
        }
        if !self.data_started {
            // Every source of the position precedes the data, so it is final now.
            self.data_started = true;
            let (x, y) = self.placement().unwrap_or_default();
            self.origin = (i128::from(x), i128::from(y));
        }
        self.data(line_no, line)
    }

    fn comment(&mut self, line_no: usize, column: usize, line: &str) -> Result<(), PatternError> {
        if !self.past_top && line.starts_with("#CXRLE") {
            return self.xrle(line_no, column, line);
        }
        self.past_top = true;
        let mut chars = line[1..].chars();
        let Some(tag) = chars.next() else {
            self.pattern.comments.push(String::new());
            return Ok(());
        };
        let rest = chars.as_str();
        match tag {
            'N' => self.names.push(rest.trim().to_string()),
            'O' => self.authors.push(rest.trim().to_string()),
            'r' => {
                let rule = rest
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| error(line_no, column + 2, "`#r` must be followed by a rule"))?;
                if self.pattern.rule.is_none() {
                    self.pattern.rule = Some(rule.to_string());
                }
            }
            // Position lines only count before the data, where XLife and
            // Life32 write them; later ones are kept as comments.
            'P' | 'p' | 'R' if !self.data_started => match parse_coordinates(rest) {
                Some(position) => {
                    self.comment_position.get_or_insert(position);
                }
                None => self.pattern.comments.push(comment_text(rest)),
            },
            _ => self.pattern.comments.push(comment_text(rest)),
        }
        Ok(())
    }

    /// `#CXRLE Pos=x,y Gen=n` (unknown keys are ignored for forward compatibility).
    fn xrle(&mut self, line_no: usize, column: usize, line: &str) -> Result<(), PatternError> {
        let mut offset = "#CXRLE".len();
        for word in line["#CXRLE".len()..].split_whitespace() {
            let word_offset = offset + line[offset..].find(word).unwrap_or(0);
            offset = word_offset + word.len();
            let word_column = column + line[..word_offset].chars().count();
            if let Some(value) = word.strip_prefix("Pos=") {
                let position = value
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
                    .ok_or_else(|| error(line_no, word_column, format!("invalid `{word}`: expected Pos=x,y")))?;
                self.xrle_position = Some(position);
            } else if let Some(value) = word.strip_prefix("Gen=") {
                let generation = value.parse().map_err(|_| {
                    error(line_no, word_column, format!("invalid `{word}`: expected a generation count below 2^64"))
                })?;
                self.generation = Some(generation);
            }
        }
        Ok(())
    }

    /// `x = 3, y = 3, rule = B3/S23` in any order and spacing; unknown keys
    /// are ignored. The rule value runs to the next whitespace (it may
    /// contain commas, e.g. `B3/S23:T10,10`).
    fn header(&mut self, line_no: usize, start_column: usize, line: &str) -> Result<(), PatternError> {
        let chars: Vec<char> = line.chars().collect();
        let column = |i: usize| start_column + i;
        let (mut width, mut height) = (None, None);
        let mut i = 0;
        loop {
            while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
                i += 1;
            }
            if i == chars.len() {
                break;
            }
            let key_start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i == key_start {
                return Err(error(line_no, column(i), format!("unexpected '{}' in header", chars[i])));
            }
            let key: String = chars[key_start..i].iter().collect::<String>().to_ascii_lowercase();
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if chars.get(i) != Some(&'=') {
                return Err(error(line_no, column(i), format!("expected '=' after `{key}` in header")));
            }
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value_start = i;
            let is_rule = key == "rule";
            while i < chars.len() && !chars[i].is_whitespace() && (is_rule || chars[i] != ',') {
                i += 1;
            }
            let mut value: String = chars[value_start..i].iter().collect();
            if is_rule && value.ends_with(',') {
                value.pop();
            }
            if value.is_empty() {
                return Err(error(line_no, column(value_start), format!("missing value for `{key}` in header")));
            }
            match key.as_str() {
                "x" | "y" => {
                    let number = parse_size(&value).ok_or_else(|| {
                        error(
                            line_no,
                            column(value_start),
                            format!(
                                "invalid {} `{value}`: expected a non-negative integer",
                                if key == "x" { "width" } else { "height" }
                            ),
                        )
                    })?;
                    if key == "x" {
                        width = Some(number);
                    } else {
                        height = Some(number);
                    }
                }
                "rule" if self.pattern.rule.is_none() => self.pattern.rule = Some(value),
                _ => {}
            }
        }
        let (Some(width), Some(height)) = (width, height) else {
            return Err(error(line_no, start_column, "the header must give both `x` and `y`"));
        };
        self.declared_size = Some((width, height));
        // Like Golly, the placement uses the rule known at the header line.
        self.grid_position = self.pattern.rule.as_deref().and_then(|rule| grid_placement(rule, (width, height)));
        Ok(())
    }

    fn data(&mut self, line_no: usize, line: &str) -> Result<(), PatternError> {
        let mut chars = line.chars().enumerate().peekable();
        // (count, column where it started)
        let mut count: Option<(u64, usize)> = None;
        while let Some((index, ch)) = chars.next() {
            let column = index + 1;
            if ch.is_whitespace() {
                if count.is_some() {
                    return Err(error(line_no, column, "whitespace between a run count and its tag"));
                }
                continue;
            }
            if let Some(digit) = ch.to_digit(10) {
                let (value, start) = match count {
                    None if digit == 0 => {
                        return Err(error(line_no, column, "run count with a leading zero"));
                    }
                    None => (0u64, column),
                    Some(current) => current,
                };
                let value = value
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(u64::from(digit)))
                    .ok_or_else(|| error(line_no, start, "run count is too large"))?;
                count = Some((value, start));
                continue;
            }
            let (run, run_column) = count.take().unwrap_or((1, column));
            match ch {
                'b' | '.' => self.x = advance(self.x, run, line_no, run_column)?,
                'o' => self.emit(run, 1, line_no, run_column)?,
                'A'..='X' => self.emit(run, ch as u8 - b'A' + 1, line_no, run_column)?,
                'p'..='y' => {
                    let state = match chars.peek() {
                        Some(&(_, next @ 'A'..='X')) => {
                            chars.next();
                            24 * (u32::from(ch as u8 - b'p') + 1) + u32::from(next as u8 - b'A') + 1
                        }
                        // Golly reads a lone prefix as a live cell.
                        _ => 1,
                    };
                    let state = u8::try_from(state).map_err(|_| {
                        error(line_no, column, format!("cell state {state} is out of range (maximum 255)"))
                    })?;
                    self.emit(run, state, line_no, run_column)?;
                }
                '$' => {
                    self.y = advance(self.y, run, line_no, run_column)?;
                    self.x = 0;
                }
                '!' => {
                    self.finished = true;
                    return Ok(());
                }
                _ => return Err(error(line_no, column, format!("unexpected character '{ch}' in RLE data"))),
            }
        }
        match count {
            Some((_, start)) => Err(error(
                line_no,
                start,
                "a run count at the end of a line must be followed by its tag on the same line",
            )),
            None => Ok(()),
        }
    }

    fn emit(&mut self, run: u64, state: u8, line_no: usize, column: usize) -> Result<(), PatternError> {
        let total = usize::try_from(run).ok().and_then(|run| run.checked_add(self.pattern.cells.len()));
        if total.is_none_or(|total| total > MAX_CELLS) {
            return Err(PatternError::LimitExceeded(format!(
                "RLE line {line_no}, column {column}: more than {MAX_CELLS} live cells"
            )));
        }
        let end = advance(self.x, run, line_no, column)?;
        let absolute = |origin: i128, relative: i128| origin.checked_add(relative).and_then(|v| i64::try_from(v).ok());
        // Run counts are at least 1, so the cells are `x..end` and the last one is at `end - 1`.
        let (Some(first), Some(last), Some(y)) =
            (absolute(self.origin.0, self.x), absolute(self.origin.0, end - 1), absolute(self.origin.1, self.y))
        else {
            return Err(error(line_no, column, "pattern coordinates overflow"));
        };
        self.pattern.cells.extend((first..=last).map(|x| Cell { x, y, state }));
        self.x = end;
        Ok(())
    }

    fn finish(mut self) -> RleDocument {
        self.pattern.name = self.names.join("\n");
        self.pattern.author = self.authors.join("\n");
        let position = self.placement();
        RleDocument { pattern: self.pattern, declared_size: self.declared_size, generation: self.generation, position }
    }
}

/// Moves a relative cursor by `run` cells.
fn advance(coordinate: i128, run: u64, line_no: usize, column: usize) -> Result<i128, PatternError> {
    coordinate.checked_add(i128::from(run)).ok_or_else(|| error(line_no, column, "pattern coordinates overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(p: &Pattern) -> String {
        p.to_ascii()
    }

    #[test]
    fn parses_standard_file() {
        let text = "#N Glider\n#O Richard K. Guy\n#C The first spaceship found.\n#C www.conwaylife.com\n\
                    x = 3, y = 3, rule = B3/S23\nbo$2bo$3o!\n";
        let doc = parse_document(text).unwrap();
        let p = &doc.pattern;
        assert_eq!(p.name, "Glider");
        assert_eq!(p.author, "Richard K. Guy");
        assert_eq!(p.comments, vec!["The first spaceship found.", "www.conwaylife.com"]);
        assert_eq!(p.rule.as_deref(), Some("B3/S23"));
        assert_eq!(cells(p), ".O.\n..O\nOOO\n");
        assert_eq!(doc.declared_size, Some((3, 3)));
        assert_eq!(doc.position, None);
        assert_eq!(p.bounding_box(), Some((0, 0, 2, 2)));
    }

    #[test]
    fn header_is_optional_and_flexible() {
        assert_eq!(cells(&parse("bo$2bo$3o!").unwrap()), ".O.\n..O\nOOO\n");
        for header in ["x=3,y=3,rule=B3/S23", "  x = 3 ,y= 3 , rule =B3/S23", "y = 3, x = 3, rule = B3/S23", "x =3,y=3"]
        {
            let p = parse(&format!("{header}\nbo$2bo$3o!")).unwrap();
            assert!(p.rule.is_none() || p.rule.as_deref() == Some("B3/S23"), "{header}");
            assert_eq!(p.population(), 5);
        }
        let doc = parse_document("x = 4, y = 5, rule = LifeHistory:T30+7,20\nA.A$3.A!").unwrap();
        assert_eq!(doc.pattern.rule.as_deref(), Some("LifeHistory:T30+7,20"));
        let doc = parse_document("x = 1, y = 1, rule = R2,C0,S4-7,9-12,B4,11-12,NB\no!").unwrap();
        assert_eq!(doc.pattern.rule.as_deref(), Some("R2,C0,S4-7,9-12,B4,11-12,NB"));
        assert_eq!(parse("x = 1, y = 1\no!").unwrap().rule, None);
    }

    #[test]
    fn run_counts_rows_and_terminator() {
        let p = parse("x = 5, y = 4\n2o3b$\n2$5o!ignored oooo\nmore ignored").unwrap();
        assert_eq!(cells(&p), "OO...\n.....\n.....\nOOOOO\n");
        // Multi-row dollar counts and wrapped lines between tokens.
        let p = parse("o2$\no\n$o!").unwrap();
        assert_eq!(p.cells, vec![Cell::alive(0, 0), Cell::alive(0, 2), Cell::alive(0, 3)]);
        // Missing '!' is fine.
        assert_eq!(parse("3o").unwrap().population(), 3);
        // Count before '!' is ignored.
        assert_eq!(parse("o3!").unwrap().population(), 1);
    }

    #[test]
    fn crlf_and_whitespace_tolerance() {
        let p = parse("#N Blinker\r\nx = 3, y = 1, rule = B3/S23\r\n  3o !\r\n").unwrap();
        assert_eq!(p.name, "Blinker");
        assert_eq!(p.population(), 3);
        let p = parse("x = 3, y = 3\rb o $ 2b o $\r3o!").unwrap();
        assert_eq!(cells(&p), ".O.\n..O\nOOO\n");
    }

    #[test]
    fn multi_state_tokens() {
        let p = parse("x = 6, y = 2, rule = /2/3\n.A2B$pAqBXyO!").unwrap();
        let states: Vec<u8> = p.cells.iter().map(|c| c.state).collect();
        assert_eq!(states, vec![1, 2, 2, 25, 50, 24, 255]);
        assert_eq!(p.cells[3], Cell::new(0, 1, 25));
        assert_eq!(p.cells[6], Cell::new(3, 1, 255));
        // 'b'/'.' and 'o'/'A' are interchangeable.
        assert_eq!(parse("bob.A").unwrap().cells, vec![Cell::alive(1, 0), Cell::alive(4, 0)]);
        // Golly reads a lone prefix as state 1.
        assert_eq!(parse("2x$o").unwrap().cells, vec![Cell::alive(0, 0), Cell::alive(1, 0), Cell::alive(0, 1)]);
    }

    #[test]
    fn xrle_and_position_lines() {
        let doc = parse_document("#CXRLE Pos=-10,-3 Gen=42\n#C note\nx = 2, y = 1\n2o!").unwrap();
        assert_eq!(doc.position, Some((-10, -3)));
        assert_eq!(doc.generation, Some(42));
        assert_eq!(doc.pattern.cells, vec![Cell::alive(-10, -3), Cell::alive(-9, -3)]);
        assert_eq!(doc.pattern.comments, vec!["note"]);
        // #CXRLE after another line is just a comment (as in Golly).
        let doc = parse_document("#C first\n#CXRLE Pos=5,5\no!").unwrap();
        assert_eq!(doc.position, None);
        assert_eq!(doc.pattern.comments, vec!["first", "XRLE Pos=5,5"]);
        // #P / #R give the top-left corner when there is no Pos.
        let doc = parse_document("#R -3 -2\no!").unwrap();
        assert_eq!(doc.pattern.cells, vec![Cell::alive(-3, -2)]);
        let doc = parse_document("#P 7 8\no!").unwrap();
        assert_eq!(doc.position, Some((7, 8)));
        // #r gives the rule, ignored when one is already known.
        let p = parse("#r 23/3\nx = 1, y = 1, rule = B36/S23\no!").unwrap();
        assert_eq!(p.rule.as_deref(), Some("23/3"));
    }

    #[test]
    fn bounded_grid_placement_follows_golly() {
        // The header size fits in the grid: the rectangle is centred on the origin.
        let doc = parse_document("x = 5, y = 3, rule = B3/S23:T30,20\no3bo$$4bo!").unwrap();
        assert_eq!(doc.position, Some((-2, -1)));
        assert_eq!(doc.pattern.cells, vec![Cell::alive(-2, -1), Cell::alive(2, -1), Cell::alive(2, 1)]);
        let grid = BoundedGrid::parse(":T30,20").unwrap().unwrap();
        assert!(doc.pattern.cells.iter().all(|c| grid.contains(c.x, c.y)));
        // Too large for the grid: the rectangle starts at the grid's top-left corner.
        let doc = parse_document("x = 40, y = 3, rule = B3/S23:P30,20\no!").unwrap();
        assert_eq!(doc.position, Some((-15, -10)));
        // A tube (infinite width) only limits the height; a rule known from `#r` counts too.
        let doc = parse_document("#r B3/S23:T0,8\nx = 7, y = 9\no!").unwrap();
        assert_eq!(doc.position, Some((0, -4)));
        // Unknown rule families keep their suffix (Golly's LifeHistory examples).
        let doc = parse_document("x = 4, y = 4, rule = LifeHistory:K40*,20\nA!").unwrap();
        assert_eq!(doc.pattern.cells, vec![Cell::alive(-2, -2)]);
        // `#CXRLE Pos` wins, as in Golly; no suffix or an unbounded one means no placement.
        let doc = parse_document("#CXRLE Pos=3,4\nx = 5, y = 3, rule = B3/S23:T30,20\no!").unwrap();
        assert_eq!(doc.position, Some((3, 4)));
        assert_eq!(parse_document("x = 5, y = 3, rule = B3/S23:T0,0\no!").unwrap().position, None);
        assert_eq!(parse_document("x = 5, y = 3, rule = B3/S23\no!").unwrap().position, None);
        // Golly fixes the placement at the header line; a later `#r` does not move it.
        assert_eq!(parse_document("x = 5, y = 3\n#r B3/S23:T30,20\no!").unwrap().position, None);
    }

    #[test]
    fn position_lines_only_count_before_the_data() {
        let doc = parse_document("#P 7 8\nx = 1, y = 1\no$\n#P 100 100\no!").unwrap();
        assert_eq!(doc.pattern.cells, vec![Cell::alive(7, 8), Cell::alive(7, 9)]);
        assert_eq!(doc.pattern.comments, vec!["100 100"]);
    }

    #[test]
    fn rectangles_spanning_the_whole_i64_range() {
        let text = "#CXRLE Pos=-9223372036854775808,-9223372036854775808\n\
                    x = 18446744073709551616, y = 2\no18446744073709551614bo$o!";
        let doc = parse_document(text).unwrap();
        assert_eq!(doc.declared_size, Some((u64::MAX, 2)), "an oversized header value is clamped");
        assert_eq!(
            doc.pattern.cells,
            vec![Cell::alive(i64::MIN, i64::MIN), Cell::alive(i64::MAX, i64::MIN), Cell::alive(i64::MIN, i64::MIN + 1)]
        );
        // The last column is i64::MAX; one cell further does not fit.
        assert_eq!(
            parse("#CXRLE Pos=-9223372036854775808,0\n18446744073709551615bo!").unwrap().cells,
            vec![Cell::alive(i64::MAX, 0)]
        );
        let err = parse("#CXRLE Pos=-9223372036854775808,0\n18446744073709551615b2o!").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(2), Some(22)), "{err}");
        assert!(err.to_string().contains("overflow"));
        // A dead run past the end of the range is harmless.
        assert_eq!(parse("#CXRLE Pos=9223372036854775807,0\no18446744073709551615b!").unwrap().population(), 1);
    }

    #[test]
    fn comment_variants() {
        let p = parse("#N Multi\n#N Line\n#c lower\n#C\n#C   indented\n#D other tag\n#\no!").unwrap();
        assert_eq!(p.name, "Multi\nLine");
        assert_eq!(p.comments, vec!["lower", "", "  indented", "other tag", ""]);
    }

    #[test]
    fn errors_carry_positions() {
        let cases: &[(&str, usize, usize, &str)] = &[
            ("x = 3, y = 3\nbo$2bo$3%!", 2, 9, "unexpected character '%'"),
            ("x = 3, y = 3\n2 o!", 2, 2, "whitespace between a run count"),
            ("x = 3, y = 3\n03o!", 2, 1, "leading zero"),
            ("x = 3, y = 3\nbo$2\no!", 2, 4, "end of a line"),
            ("x = 3, y = 3\nzo!", 2, 1, "unexpected character 'z'"),
            ("x = a, y = 3\no!", 1, 5, "invalid width"),
            ("x = 3, y = -1\no!", 1, 12, "invalid height"),
            ("x = 3\no!", 1, 1, "both `x` and `y`"),
            ("x = 3, y 3\no!", 1, 10, "expected '='"),
            ("x = 3, y = 3, rule =\no!", 1, 21, "missing value"),
            ("o\nx = 1, y = 1\no!", 2, 1, "before the pattern data"),
            ("x = 1, y = 1\nx = 1, y = 1\no!", 2, 1, "duplicate header"),
            ("#CXRLE Pos=1\no!", 1, 8, "expected Pos=x,y"),
            ("#CXRLE Gen=-4\no!", 1, 8, "generation count"),
            ("x = 1, y = 1\n2yX!", 2, 2, "out of range"),
            ("99999999999999999999o!", 1, 1, "too large"),
            ("#r\no!", 1, 3, "followed by a rule"),
        ];
        for (text, line, column, message) in cases {
            let err = parse(text).expect_err(text);
            let shown = err.to_string();
            assert!(shown.contains(message), "{text:?}: got `{shown}`, expected `{message}`");
            assert_eq!((err.line(), err.column()), (Some(*line), Some(*column)), "{text:?}: {shown}");
        }
    }

    #[test]
    fn limits_and_overflow() {
        assert!(matches!(parse("999999999o!"), Err(PatternError::LimitExceeded(_))));
        let err = parse("9223372036854775807b2o!").unwrap_err();
        assert!(err.to_string().contains("overflow"), "{err}");
        assert!(parse("#CXRLE Pos=9223372036854775807,0\n2o!").is_err());
        // Huge dead runs and blank rows cost nothing.
        let p = parse("4000000000000b o 3000000000000$o!").unwrap();
        assert_eq!(p.cells, vec![Cell::alive(4_000_000_000_000, 0), Cell::alive(0, 3_000_000_000_000)]);
    }

    #[test]
    fn empty_patterns() {
        let doc = parse_document("x = 0, y = 0, rule = B3/S23\n!\n").unwrap();
        assert!(doc.pattern.is_empty());
        assert_eq!(doc.declared_size, Some((0, 0)));
        assert!(parse("").unwrap().is_empty());
    }
}
