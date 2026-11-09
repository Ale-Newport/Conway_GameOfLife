//! The pattern model: a list of non-dead cells plus descriptive metadata.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::error::PatternError;
use crate::format::Format;
use crate::text::lines;

/// Maximum number of live cells a reader will produce (2^25 ≈ 33.5 million).
/// It protects against tiny files with huge run counts exhausting memory.
pub const MAX_CELLS: usize = 1 << 25;

/// Maximum number of characters (bounding-box area) produced by the dense text
/// writers (plaintext, Life 1.05, [`Pattern::to_ascii`]) and by
/// [`Pattern::to_grid`].
pub const MAX_DENSE_AREA: u128 = 1 << 28;

/// One non-dead cell. `state` is 1 for live cells of two-state rules and
/// 1..=255 for multi-state rules; state 0 (dead) cells are never produced by
/// the readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cell {
    /// Column, growing to the right.
    pub x: i64,
    /// Row, growing downward.
    pub y: i64,
    /// Cell state (1..=255).
    pub state: u8,
}

impl Cell {
    /// A cell with an explicit state.
    pub const fn new(x: i64, y: i64, state: u8) -> Self {
        Self { x, y, state }
    }

    /// A live (state 1) cell.
    pub const fn alive(x: i64, y: i64) -> Self {
        Self { x, y, state: 1 }
    }
}

/// A cellular-automaton pattern.
///
/// `cells` lists the non-dead cells in no particular order. Readers produce
/// row-major order without duplicates; [`Pattern::normalized`] gives the
/// canonical form (top-left at the origin, sorted by `y` then `x`,
/// duplicates removed with the later entry winning). Cells with state 0 are
/// ignored by every query and dropped by normalisation.
///
/// Geometric transforms keep the top-left corner of the bounding box fixed,
/// so a pattern rotates or flips "in place". Coordinates saturate at the
/// `i64` range instead of overflowing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    /// Pattern name (`#N` in RLE, `!Name:` in plaintext).
    pub name: String,
    /// Free-form description. No file format has a dedicated field for it:
    /// writers emit it as the first comment lines and readers return those
    /// lines in `comments`. The catalog fills it from `catalog.toml`.
    pub description: String,
    /// Author / discoverer (`#O` in RLE, `!Author:` in plaintext).
    pub author: String,
    /// Rule string exactly as found in the file (Golly accepts many rule
    /// families, so it is not validated here; see [`crate::rule::Rule::parse`]).
    pub rule: Option<String>,
    /// Comment lines (`#C` in RLE, `!` lines in plaintext, `#D` in Life 1.05).
    pub comments: Vec<String>,
    /// Non-dead cells.
    pub cells: Vec<Cell>,
}

/// Converts a wide intermediate coordinate to `i64`, saturating.
fn clamp(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

impl Pattern {
    /// A pattern with the given cells and no metadata.
    pub fn new(cells: Vec<Cell>) -> Self {
        Self { cells, ..Self::default() }
    }

    /// A pattern of live (state 1) cells from `(x, y)` coordinates, e.g. the
    /// cell lists exchanged with Hashlife.
    pub fn from_live_cells(cells: impl IntoIterator<Item = (i64, i64)>) -> Self {
        Self::new(cells.into_iter().map(|(x, y)| Cell::alive(x, y)).collect())
    }

    /// Sets the name (builder style).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Sets the rule string (builder style).
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Coordinates of every non-dead cell, in storage order.
    pub fn live_cells(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.cells.iter().filter(|c| c.state != 0).map(|c| (c.x, c.y))
    }

    /// Whether the pattern has no non-dead cells.
    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|c| c.state == 0)
    }

    /// Number of non-dead cell entries. Readers and transforms never produce
    /// duplicates; for hand-built patterns call [`Pattern::normalized`] first
    /// if the list may contain the same position twice.
    pub fn population(&self) -> usize {
        self.cells.iter().filter(|c| c.state != 0).count()
    }

    /// Highest cell state present (0 for an empty pattern).
    pub fn max_state(&self) -> u8 {
        self.cells.iter().map(|c| c.state).max().unwrap_or(0)
    }

    /// Inclusive bounding box `(min_x, min_y, max_x, max_y)` of the non-dead
    /// cells, or `None` for an empty pattern.
    pub fn bounding_box(&self) -> Option<(i64, i64, i64, i64)> {
        let mut live = self.cells.iter().filter(|c| c.state != 0);
        let first = live.next()?;
        Some(live.fold((first.x, first.y, first.x, first.y), |(x0, y0, x1, y1), c| {
            (x0.min(c.x), y0.min(c.y), x1.max(c.x), y1.max(c.y))
        }))
    }

    /// Bounding-box width in cells (0 when empty), saturating at `u64::MAX`.
    pub fn width(&self) -> u64 {
        self.bounding_box().map_or(0, |(x0, _, x1, _)| extent(x0, x1))
    }

    /// Bounding-box height in cells (0 when empty), saturating at `u64::MAX`.
    pub fn height(&self) -> u64 {
        self.bounding_box().map_or(0, |(_, y0, _, y1)| extent(y0, y1))
    }

    /// Copy with the cells sorted by `y` then `x`, duplicate positions merged
    /// (the later entry wins), state-0 cells removed, and the bounding box
    /// moved so its top-left corner is at `(0, 0)`. Metadata is kept.
    pub fn normalized(&self) -> Pattern {
        let mut cells = sorted_unique(&self.cells);
        if let Some(&first) = cells.first() {
            let min_x = cells.iter().map(|c| c.x).min().unwrap_or(first.x);
            let min_y = first.y;
            for cell in &mut cells {
                cell.x = clamp(i128::from(cell.x) - i128::from(min_x));
                cell.y = clamp(i128::from(cell.y) - i128::from(min_y));
            }
        }
        Pattern { cells, ..self.metadata() }
    }

    /// Copy with the same metadata and no cells.
    fn metadata(&self) -> Pattern {
        Pattern {
            name: self.name.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            rule: self.rule.clone(),
            comments: self.comments.clone(),
            cells: Vec::new(),
        }
    }

    /// Applies `map(cell, bounding_box)` to every cell (wide arithmetic).
    fn map_cells(&self, map: impl Fn(i128, i128, [i128; 4]) -> (i128, i128)) -> Pattern {
        let Some((x0, y0, x1, y1)) = self.bounding_box() else { return self.clone() };
        let bbox = [i128::from(x0), i128::from(y0), i128::from(x1), i128::from(y1)];
        let cells = self
            .cells
            .iter()
            .map(|c| {
                let (x, y) = map(i128::from(c.x), i128::from(c.y), bbox);
                Cell { x: clamp(x), y: clamp(y), state: c.state }
            })
            .collect();
        Pattern { cells, ..self.metadata() }
    }

    /// Rotates 90° clockwise on screen (y down) about the bounding box,
    /// keeping its top-left corner fixed.
    pub fn rotate_cw(&self) -> Pattern {
        self.map_cells(|x, y, [x0, y0, _, y1]| (x0 + (y1 - y), y0 + (x - x0)))
    }

    /// Rotates 90° counter-clockwise, keeping the top-left corner fixed.
    pub fn rotate_ccw(&self) -> Pattern {
        self.map_cells(|x, y, [x0, y0, x1, _]| (x0 + (y - y0), y0 + (x1 - x)))
    }

    /// Rotates 180° in place.
    pub fn rotate_180(&self) -> Pattern {
        self.map_cells(|x, y, [x0, y0, x1, y1]| (x0 + x1 - x, y0 + y1 - y))
    }

    /// Mirrors left ↔ right in place.
    pub fn flip_horizontal(&self) -> Pattern {
        self.map_cells(|x, y, [x0, _, x1, _]| (x0 + x1 - x, y))
    }

    /// Mirrors top ↔ bottom in place.
    pub fn flip_vertical(&self) -> Pattern {
        self.map_cells(|x, y, [_, y0, _, y1]| (x, y0 + y1 - y))
    }

    /// Copy moved by `(dx, dy)`.
    pub fn translated(&self, dx: i64, dy: i64) -> Pattern {
        let cells = self
            .cells
            .iter()
            .map(|c| Cell { x: c.x.saturating_add(dx), y: c.y.saturating_add(dy), state: c.state })
            .collect();
        Pattern { cells, ..self.metadata() }
    }

    /// Union of this pattern and `other` moved by `(dx, dy)`. Where both have
    /// a cell, `other`'s state wins (like pasting in Golly's copy mode). The
    /// result is sorted by `y` then `x` without duplicates; metadata comes
    /// from `self`.
    pub fn merged(&self, other: &Pattern, dx: i64, dy: i64) -> Pattern {
        let mut all = self.cells.clone();
        all.extend(other.translated(dx, dy).cells);
        Pattern { cells: sorted_unique(&all), ..self.metadata() }
    }

    /// Dense debugging picture of the bounding box: `.` dead, `O` state 1,
    /// `2`..`9` for states 2-9 and `#` for higher states, one line per row,
    /// each ending with `\n`. Empty patterns give an empty string; patterns
    /// whose bounding box exceeds [`MAX_DENSE_AREA`] give a one-line summary.
    pub fn to_ascii(&self) -> String {
        let Some((x0, y0, _, _)) = self.bounding_box() else { return String::new() };
        let (width, height) = (self.width(), self.height());
        if u128::from(width) * u128::from(height) > MAX_DENSE_AREA {
            return format!("<pattern too large to draw: {width} x {height} cells>\n");
        }
        let (w, h) = (width as usize, height as usize);
        let mut rows = vec![vec![b'.'; w]; h];
        for cell in sorted_unique(&self.cells) {
            let symbol = match cell.state {
                1 => b'O',
                s @ 2..=9 => b'0' + s,
                _ => b'#',
            };
            rows[(cell.y - y0) as usize][(cell.x - x0) as usize] = symbol;
        }
        let mut out = String::with_capacity((w + 1) * h);
        for row in rows {
            out.extend(row.into_iter().map(char::from));
            out.push('\n');
        }
        out
    }

    /// Parses a dense picture: `.` dead, `O`, `o` or `*` live, digits `1`-`9`
    /// for that state. Each line is trimmed, blank lines before the first and
    /// after the last row are ignored and blank lines in between are empty
    /// rows, so indented string literals work in tests. The first row is
    /// `y = 0` and the first column `x = 0`.
    pub fn from_ascii(text: &str) -> Result<Pattern, PatternError> {
        // (line number, trimmed row, characters of indentation)
        let rows: Vec<(usize, &str, usize)> = lines(text)
            .map(|(n, raw)| {
                let row = raw.trim_start();
                (n, row.trim_end(), raw.chars().count() - row.chars().count())
            })
            .collect();
        let first = rows.iter().position(|(_, row, _)| !row.is_empty());
        let last = rows.iter().rposition(|(_, row, _)| !row.is_empty());
        let mut cells = Vec::new();
        if let (Some(first), Some(last)) = (first, last) {
            for (y, &(line_no, row, indent)) in rows[first..=last].iter().enumerate() {
                for (x, ch) in row.chars().enumerate() {
                    let state = match ch {
                        '.' => continue,
                        'O' | 'o' | '*' => 1,
                        '1'..='9' => ch as u8 - b'0',
                        _ => {
                            return Err(PatternError::syntax(
                                Format::Plaintext,
                                line_no,
                                indent + x + 1,
                                format!("unexpected character '{ch}' (use '.' for dead and 'O' for live cells)"),
                            ));
                        }
                    };
                    cells.push(Cell::new(x as i64, y as i64, state));
                }
            }
        }
        Ok(Pattern::new(cells))
    }

    /// Dense grid of the bounding box (top-left = bounding-box corner) with
    /// cell states, or an error when the area exceeds [`MAX_DENSE_AREA`].
    /// Empty patterns give a 0 × 0 grid.
    pub fn to_grid(&self) -> Result<Grid2<u8>, PatternError> {
        let Some((x0, y0, _, _)) = self.bounding_box() else { return Ok(Grid2::new(0, 0, 0)) };
        let (width, height) = (self.width(), self.height());
        if u128::from(width) * u128::from(height) > MAX_DENSE_AREA {
            return Err(PatternError::LimitExceeded(format!(
                "a {width} x {height} bounding box is too large for a dense grid (limit {MAX_DENSE_AREA} cells)"
            )));
        }
        let mut grid = Grid2::new(width as usize, height as usize, 0u8);
        for cell in sorted_unique(&self.cells) {
            grid.set((cell.x - x0) as usize, (cell.y - y0) as usize, cell.state);
        }
        Ok(grid)
    }

    /// Pattern of the non-zero cells of a dense grid (cell `(x, y)` of the
    /// grid becomes `Cell { x, y, .. }`).
    pub fn from_grid(grid: &Grid2<u8>) -> Pattern {
        Pattern::new(
            grid.iter()
                .filter(|&(_, _, &state)| state != 0)
                .map(|(x, y, &state)| Cell::new(x as i64, y as i64, state))
                .collect(),
        )
    }
}

/// Number of cells between two inclusive coordinates, saturating.
fn extent(min: i64, max: i64) -> u64 {
    u64::try_from(i128::from(max) - i128::from(min) + 1).unwrap_or(u64::MAX)
}

/// Non-dead cells sorted by `(y, x)`; for duplicate positions the entry that
/// came last in `cells` wins.
pub(crate) fn sorted_unique(cells: &[Cell]) -> Vec<Cell> {
    let mut indexed: Vec<(usize, Cell)> = cells.iter().copied().enumerate().collect();
    indexed.sort_by_key(|&(index, c)| (c.y, c.x, index));
    let mut out: Vec<Cell> = Vec::with_capacity(indexed.len());
    for (_, cell) in indexed {
        match out.last_mut() {
            Some(last) if last.x == cell.x && last.y == cell.y => *last = cell,
            _ => out.push(cell),
        }
    }
    out.retain(|c| c.state != 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glider() -> Pattern {
        Pattern::from_ascii(
            "
            .O.
            ..O
            OOO
            ",
        )
        .unwrap()
    }

    #[test]
    fn ascii_round_trip_and_queries() {
        let g = glider();
        assert_eq!(g.population(), 5);
        assert_eq!(g.bounding_box(), Some((0, 0, 2, 2)));
        assert_eq!((g.width(), g.height()), (3, 3));
        assert_eq!(g.to_ascii(), ".O.\n..O\nOOO\n");
        assert_eq!(Pattern::from_ascii(&g.to_ascii()).unwrap(), g);
        assert_eq!(g.max_state(), 1);
        assert!(!g.is_empty());
    }

    #[test]
    fn empty_pattern_behaviour() {
        let empty = Pattern::default();
        assert!(empty.is_empty());
        assert_eq!(empty.bounding_box(), None);
        assert_eq!((empty.width(), empty.height(), empty.population()), (0, 0, 0));
        assert_eq!(empty.to_ascii(), "");
        assert_eq!(empty.rotate_cw(), empty);
        assert_eq!(empty.normalized(), empty);
        assert_eq!(Pattern::from_ascii("\n  \n").unwrap(), empty);
        let grid = empty.to_grid().unwrap();
        assert_eq!((grid.width(), grid.height()), (0, 0));
    }

    #[test]
    fn rotations_and_flips() {
        let g = glider();
        assert_eq!(g.rotate_cw().to_ascii(), "O..\nO.O\nOO.\n");
        assert_eq!(g.rotate_ccw().to_ascii(), ".OO\nO.O\n..O\n");
        assert_eq!(g.rotate_180().to_ascii(), "OOO\nO..\n.O.\n");
        assert_eq!(g.flip_horizontal().to_ascii(), ".O.\nO..\nOOO\n");
        assert_eq!(g.flip_vertical().to_ascii(), "OOO\n..O\n.O.\n");
    }

    #[test]
    fn transforms_compose_to_identity() {
        let g = glider().translated(-7, 12);
        let id = |p: &Pattern| p.normalized().cells;
        assert_eq!(g.rotate_cw().rotate_cw().rotate_cw().rotate_cw(), g);
        assert_eq!(g.rotate_ccw().rotate_ccw().rotate_ccw().rotate_ccw(), g);
        assert_eq!(id(&g.rotate_cw().rotate_ccw()), id(&g));
        assert_eq!(id(&g.rotate_cw().rotate_cw()), id(&g.rotate_180()));
        assert_eq!(id(&g.flip_horizontal().flip_horizontal()), id(&g));
        assert_eq!(id(&g.flip_horizontal().flip_vertical()), id(&g.rotate_180()));
        // Transforms rotate in place: the top-left corner stays fixed.
        let (x0, y0, ..) = g.bounding_box().unwrap();
        let (rx0, ry0, ..) = g.rotate_cw().bounding_box().unwrap();
        assert_eq!((x0, y0), (rx0, ry0));
    }

    #[test]
    fn rotation_of_non_square_pattern() {
        let bar = Pattern::from_ascii("OOO.O").unwrap().translated(10, 20);
        let r = bar.rotate_cw();
        assert_eq!(r.bounding_box(), Some((10, 20, 10, 24)));
        assert_eq!(r.to_ascii(), "O\nO\nO\n.\nO\n");
        assert_eq!(bar.rotate_ccw().to_ascii(), "O\n.\nO\nO\nO\n");
    }

    #[test]
    fn normalization_sorts_dedups_and_moves_to_origin() {
        let p = Pattern::new(vec![
            Cell::alive(5, 9),
            Cell::new(3, 7, 2),
            Cell::alive(4, 7),
            Cell::new(3, 7, 3),
            Cell::new(9, 9, 0),
        ]);
        let n = p.normalized();
        assert_eq!(n.cells, vec![Cell::new(0, 0, 3), Cell::alive(1, 0), Cell::alive(2, 2)]);
        assert_eq!(p.bounding_box(), Some((3, 7, 5, 9)), "state-0 cells are ignored");
        assert_eq!(p.population(), 4);
    }

    #[test]
    fn translate_and_merge() {
        let block = Pattern::from_ascii("OO\nOO").unwrap().with_name("block");
        let moved = block.translated(3, -2);
        assert_eq!(moved.bounding_box(), Some((3, -2, 4, -1)));
        let both = block.merged(&block, 1, 0);
        assert_eq!(both.population(), 6);
        assert_eq!(both.to_ascii(), "OOO\nOOO\n");
        assert_eq!(both.name, "block");
        let coloured = Pattern::new(vec![Cell::new(0, 0, 4)]);
        assert_eq!(block.merged(&coloured, 1, 1).to_ascii(), "OO\nO4\n");
        // Saturating instead of overflowing.
        assert_eq!(Pattern::new(vec![Cell::alive(i64::MAX, 0)]).translated(5, 0).cells[0].x, i64::MAX);
    }

    #[test]
    fn multi_state_ascii_and_errors() {
        let p = Pattern::from_ascii("1.2\n.9.").unwrap();
        assert_eq!(p.cells, vec![Cell::new(0, 0, 1), Cell::new(2, 0, 2), Cell::new(1, 1, 9)]);
        assert_eq!(p.to_ascii(), "O.2\n.9.\n");
        assert_eq!(Pattern::new(vec![Cell::new(0, 0, 200)]).to_ascii(), "#\n");
        let err = Pattern::from_ascii("..\n  .x").unwrap_err();
        assert_eq!((err.line(), err.column()), (Some(2), Some(4)), "{err}");
        let huge = Pattern::from_live_cells([(0, 0), (1 << 40, 1 << 40)]);
        assert!(huge.to_ascii().starts_with("<pattern too large"));
        assert!(matches!(huge.to_grid(), Err(PatternError::LimitExceeded(_))));
    }

    #[test]
    fn dense_grid_round_trip() {
        let g = glider().translated(-4, 6);
        let grid = g.to_grid().unwrap();
        assert_eq!((grid.width(), grid.height()), (3, 3));
        assert_eq!(*grid.get(1, 0), 1);
        assert_eq!(*grid.get(0, 0), 0);
        assert_eq!(Pattern::from_grid(&grid).normalized().cells, g.normalized().cells);
        let live: Vec<_> = g.live_cells().collect();
        assert_eq!(Pattern::from_live_cells(live), Pattern::new(g.cells.clone()));
    }

    #[test]
    fn serde_round_trip() {
        let g = glider().with_rule("B3/S23").with_name("Glider");
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Pattern>(&json).unwrap(), g);
    }
}
