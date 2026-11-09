//! Golly's bounded-grid rule suffix (`Help/bounded.html`,
//! `lifealgo::setgridsize` / `canonicalsuffix`).
//!
//! Grammar (letters case-insensitive, whitespace ignored):
//!
//! ```text
//! suffix := ":" [ kind [ width [ "*" ] [ shift ] [ "," height [ "*" ] [ shift ] ] ] ]
//! kind   := "P" (plane) | "T" (torus/tube) | "K" (Klein bottle) | "C" (cross-surface) | "S" (sphere)
//! shift  := ("+" | "-") digits
//! ```
//!
//! A missing height equals the width; 0 means infinite (only for P and T).
//! `":"`, `":T"` and any suffix with both dimensions 0 describe the unbounded
//! universe and parse to `None`.

use std::fmt;
use std::str::FromStr;

use simulation_core::Topology;

use super::RuleError;

/// Largest width or height Golly accepts for a bounded grid.
pub const MAX_GRID_SIZE: u32 = 2_000_000_000;

/// Topology letter of a bounded grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GridKind {
    /// `P`: flat plane, cells outside are permanently dead.
    Plane,
    /// `T`: torus (or an infinite tube when one dimension is 0).
    Torus,
    /// `K`: Klein bottle (one pair of edges reversed before joining).
    KleinBottle,
    /// `C`: cross-surface / real projective plane (both pairs reversed).
    CrossSurface,
    /// `S`: sphere (top edge joined to left edge, right edge to bottom edge).
    Sphere,
}

impl GridKind {
    /// Golly's suffix letter.
    pub fn letter(self) -> char {
        match self {
            Self::Plane => 'P',
            Self::Torus => 'T',
            Self::KleinBottle => 'K',
            Self::CrossSurface => 'C',
            Self::Sphere => 'S',
        }
    }

    /// Human-readable name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plane => "plane",
            Self::Torus => "torus",
            Self::KleinBottle => "Klein bottle",
            Self::CrossSurface => "cross-surface",
            Self::Sphere => "sphere",
        }
    }
}

/// A bounded universe as described by a Golly rule suffix.
///
/// Twists and shifts use Golly's terminology: the *horizontal* edges are the
/// top and bottom edges, the *vertical* edges are the left and right edges.
/// A horizontal twist reverses the top/bottom edges before joining them; a
/// horizontal shift slides the top/bottom join sideways by that many cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoundedGrid {
    /// Topology.
    pub kind: GridKind,
    /// Width in cells; 0 = infinite (plane and torus/tube only).
    pub width: u32,
    /// Height in cells; 0 = infinite (plane and torus/tube only).
    pub height: u32,
    /// Top and bottom edges are reversed when joined (Klein bottle `w*`, cross-surface).
    pub horizontal_twist: bool,
    /// Left and right edges are reversed when joined (Klein bottle `,h*`, cross-surface).
    pub vertical_twist: bool,
    /// Shift along the horizontal (top/bottom) join; normalised like Golly
    /// (|shift| < width, and exactly 0 or 1 for a Klein bottle).
    pub horizontal_shift: i32,
    /// Shift along the vertical (left/right) join, normalised the same way.
    pub vertical_shift: i32,
}

/// Cursor over the non-whitespace characters of a suffix with their columns.
struct Cursor {
    chars: Vec<(char, usize)>,
    index: usize,
    end_column: usize,
}

impl Cursor {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).map(|&(c, _)| c)
    }

    fn column(&self) -> usize {
        self.chars.get(self.index).map_or(self.end_column, |&(_, col)| col)
    }

    fn bump(&mut self) {
        self.index += 1;
    }

    fn eat(&mut self, expected: char) -> bool {
        let found = self.peek() == Some(expected);
        if found {
            self.bump();
        }
        found
    }

    /// Reads a (possibly empty) decimal number; empty reads as 0.
    fn number(&mut self, what: &str) -> Result<u32, RuleError> {
        let start = self.column();
        let mut value: u64 = 0;
        while let Some(digit) = self.peek().and_then(|c| c.to_digit(10)) {
            value = value * 10 + u64::from(digit);
            if value > u64::from(MAX_GRID_SIZE) {
                return Err(RuleError::at(start, format!("{what} exceeds the maximum of {MAX_GRID_SIZE}")));
            }
            self.bump();
        }
        Ok(value as u32)
    }

    /// Reads an optional `+n` / `-n` shift.
    fn shift(&mut self) -> Result<Option<(i64, usize)>, RuleError> {
        let column = self.column();
        let sign = match self.peek() {
            Some('+') => 1,
            Some('-') => -1,
            _ => return Ok(None),
        };
        self.bump();
        if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return Err(RuleError::at(self.column(), "expected a shift amount after the sign"));
        }
        let amount = self.number("shift")?;
        Ok(Some((sign * i64::from(amount), column)))
    }
}

/// Golly's reduction of a shift: amounts ≥ the edge length wrap, sign kept.
fn reduce_shift(shift: i64, length: u32) -> i32 {
    let magnitude = shift.abs();
    let reduced = if magnitude >= i64::from(length) { magnitude % i64::from(length) } else { magnitude };
    (shift.signum() * reduced) as i32
}

impl BoundedGrid {
    /// A torus of the given size.
    pub fn torus(width: u32, height: u32) -> Self {
        Self::simple(GridKind::Torus, width, height)
    }

    /// A bounded plane of the given size.
    pub fn plane(width: u32, height: u32) -> Self {
        Self::simple(GridKind::Plane, width, height)
    }

    fn simple(kind: GridKind, width: u32, height: u32) -> Self {
        Self {
            kind,
            width,
            height,
            horizontal_twist: false,
            vertical_twist: false,
            horizontal_shift: 0,
            vertical_shift: 0,
        }
    }

    /// Parses a suffix such as `":T100,100"` (the leading colon is optional).
    /// Returns `None` for suffixes that describe an unbounded universe.
    pub fn parse(suffix: &str) -> Result<Option<Self>, RuleError> {
        Self::parse_at(suffix, 0)
    }

    /// Like [`BoundedGrid::parse`]; error columns are offset by `column_offset`
    /// so they point into the full rule string.
    pub(crate) fn parse_at(suffix: &str, column_offset: usize) -> Result<Option<Self>, RuleError> {
        let chars: Vec<(char, usize)> = suffix
            .chars()
            .enumerate()
            .filter(|(_, c)| !c.is_whitespace())
            .map(|(i, c)| (c, column_offset + i + 1))
            .collect();
        let end_column = column_offset + suffix.chars().count() + 1;
        let mut cur = Cursor { chars, index: 0, end_column };
        cur.eat(':');

        let Some(letter) = cur.peek() else { return Ok(None) };
        let kind = match letter.to_ascii_uppercase() {
            'P' => GridKind::Plane,
            'T' => GridKind::Torus,
            'K' => GridKind::KleinBottle,
            'C' => GridKind::CrossSurface,
            'S' => GridKind::Sphere,
            _ => {
                return Err(RuleError::at(
                    cur.column(),
                    format!("unknown grid topology '{letter}' (expected P, T, K, C or S)"),
                ));
            }
        };
        cur.bump();
        if cur.peek().is_none() {
            // Golly treats ":T" like ":T0,0", i.e. unbounded.
            return Ok(None);
        }

        let cross = kind == GridKind::CrossSurface;
        let mut grid = Self { horizontal_twist: cross, vertical_twist: cross, ..Self::simple(kind, 0, 0) };
        grid.width = cur.number("grid width")?;
        if cur.peek() == Some('*') {
            if kind != GridKind::KleinBottle {
                return Err(RuleError::at(cur.column(), "only a Klein bottle can have a twist ('*')"));
            }
            grid.horizontal_twist = true;
            cur.bump();
        }
        if let Some((shift, column)) = cur.shift()? {
            grid.check_shift(column, grid.horizontal_twist, grid.width, "width")?;
            grid.horizontal_shift = reduce_shift(shift, grid.width);
        }

        if cur.eat(',') {
            if kind == GridKind::Sphere {
                return Err(RuleError::at(cur.column() - 1, "a sphere is square: give only its width (e.g. :S30)"));
            }
            grid.height = cur.number("grid height")?;
            if cur.peek() == Some('*') {
                if kind != GridKind::KleinBottle {
                    return Err(RuleError::at(cur.column(), "only a Klein bottle can have a twist ('*')"));
                }
                if grid.horizontal_twist {
                    return Err(RuleError::at(
                        cur.column(),
                        "a Klein bottle can't have both horizontal and vertical twists",
                    ));
                }
                grid.vertical_twist = true;
                cur.bump();
            }
            if let Some((shift, column)) = cur.shift()? {
                grid.check_shift(column, grid.vertical_twist, grid.height, "height")?;
                // As in Golly, a horizontal shift that reduced to 0 does not count.
                if grid.horizontal_shift != 0 {
                    return Err(RuleError::at(column, "a grid can't have both horizontal and vertical shifts"));
                }
                grid.vertical_shift = reduce_shift(shift, grid.height);
            }
            if let Some(c) = cur.peek() {
                return Err(RuleError::at(cur.column(), format!("unexpected '{c}' after grid height")));
            }
        } else if let Some(c) = cur.peek() {
            return Err(RuleError::at(cur.column(), format!("unexpected '{c}' after grid width")));
        } else {
            grid.height = grid.width;
        }

        if matches!(kind, GridKind::KleinBottle | GridKind::CrossSurface | GridKind::Sphere) && grid.width == 0 {
            return Err(RuleError::new(format!("a {} can't have an infinite width", kind.label())));
        }
        if matches!(kind, GridKind::KleinBottle | GridKind::CrossSurface) && grid.height == 0 {
            return Err(RuleError::new(format!("a {} can't have an infinite height", kind.label())));
        }
        if (grid.horizontal_shift != 0 || grid.vertical_shift != 0) && (grid.width == 0 || grid.height == 0) {
            return Err(RuleError::new("shifting is not allowed if either grid dimension is infinite"));
        }
        if kind == GridKind::KleinBottle {
            if !grid.horizontal_twist && !grid.vertical_twist {
                // Golly reads ":K10,20" as ":K10,20*".
                grid.vertical_twist = true;
            }
            // All shifts of a twisted edge are equivalent to 1, and only an
            // even number of cells along that edge can be shifted.
            grid.horizontal_shift = i32::from(grid.horizontal_shift != 0 && grid.width.is_multiple_of(2));
            grid.vertical_shift = i32::from(grid.vertical_shift != 0 && grid.height.is_multiple_of(2));
        }
        if grid.width == 0 && grid.height == 0 {
            return Ok(None);
        }
        Ok(Some(grid))
    }

    fn check_shift(&self, column: usize, twisted: bool, length: u32, dimension: &str) -> Result<(), RuleError> {
        let message = match self.kind {
            GridKind::Plane => Some("a plane can't have a shift".to_string()),
            GridKind::Sphere => Some("a sphere can't have a shift".to_string()),
            GridKind::CrossSurface => Some("a cross-surface can't have a shift".to_string()),
            GridKind::KleinBottle if !twisted => {
                Some("a Klein bottle's shift must be on its twisted edges".to_string())
            }
            _ if length == 0 => Some(format!("can't shift an infinite {dimension}")),
            _ => None,
        };
        match message {
            Some(message) => Err(RuleError::at(column, message)),
            None => Ok(()),
        }
    }

    /// The equivalent [`Topology`] of `simulation_core`, when one exists:
    /// unshifted planes and tori with both dimensions finite, Klein bottles
    /// with a horizontal twist and no shift (core's orientation), and
    /// cross-surfaces. Tubes, half-planes, spheres, shifted tori and Klein
    /// bottles twisted on their vertical edges have no core equivalent.
    pub fn topology(&self) -> Option<Topology> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let unshifted = self.horizontal_shift == 0 && self.vertical_shift == 0;
        match self.kind {
            GridKind::Plane => Some(Topology::Bounded),
            GridKind::Torus if unshifted => Some(Topology::Torus),
            GridKind::KleinBottle if self.horizontal_twist && unshifted => Some(Topology::KleinBottle),
            GridKind::CrossSurface => Some(Topology::CrossSurface),
            _ => None,
        }
    }

    /// Whether the grid is finite in both directions.
    pub fn is_finite(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Whether the cell `(x, y)`, in Golly's coordinates (see
    /// [`BoundedGrid::top_left`]), lies inside the grid. An infinite
    /// dimension contains every coordinate. Golly drops the cells of a
    /// pattern file that fall outside the grid; filtering with this method
    /// does the same.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        let inside =
            |value: i64, start: i64, size: u32| size == 0 || (value >= start && value < start + i64::from(size));
        let (left, top) = self.top_left();
        inside(x, left, self.width) && inside(y, top, self.height)
    }

    /// Golly coordinate of the top-left grid cell, `(-(w/2), -(h/2))` (0 for
    /// an infinite dimension). Subtract it from pattern coordinates read with
    /// `#CXRLE Pos` to get grid indices.
    pub fn top_left(&self) -> (i64, i64) {
        (-i64::from(self.width / 2), -i64::from(self.height / 2))
    }
}

impl fmt::Display for BoundedGrid {
    /// Golly's canonical suffix, including the leading colon (`:T30+5,20`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (w, h) = (self.width, self.height);
        match self.kind {
            GridKind::Plane => write!(f, ":P{w},{h}"),
            GridKind::Sphere => write!(f, ":S{w}"),
            GridKind::CrossSurface => write!(f, ":C{w},{h}"),
            GridKind::KleinBottle if self.horizontal_twist => {
                let shift = if self.horizontal_shift != 0 { "+1" } else { "" };
                write!(f, ":K{w}*{shift},{h}")
            }
            GridKind::KleinBottle => {
                let shift = if self.vertical_shift != 0 { "+1" } else { "" };
                write!(f, ":K{w},{h}*{shift}")
            }
            GridKind::Torus if self.horizontal_shift != 0 => write!(f, ":T{w}{:+},{h}", self.horizontal_shift),
            GridKind::Torus if self.vertical_shift != 0 => write!(f, ":T{w},{h}{:+}", self.vertical_shift),
            GridKind::Torus => write!(f, ":T{w},{h}"),
        }
    }
}

impl FromStr for BoundedGrid {
    type Err = RuleError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)?.ok_or_else(|| RuleError::new(format!("`{s}` describes an unbounded universe")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(text: &str) -> BoundedGrid {
        BoundedGrid::parse(text)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .unwrap_or_else(|| panic!("{text}: unbounded"))
    }

    #[test]
    fn canonical_forms_match_golly_help_examples() {
        let table: &[(&str, &str)] = &[
            (":P30,20", ":P30,20"),
            (":P30", ":P30,30"),
            (":P30,0", ":P30,0"),
            (":T0,20", ":T0,20"),
            (":T30,20", ":T30,20"),
            (":t30,20", ":T30,20"),
            (":T30+5,20", ":T30+5,20"),
            (":T30,20-2", ":T30,20-2"),
            (":T30+35,20", ":T30+5,20"),
            (":T30-35,20", ":T30-5,20"),
            // A shift that reduces to 0 is no shift, so a vertical shift is still allowed (as in Golly).
            (":T30+30,20+5", ":T30,20+5"),
            (":K30*+30,20", ":K30*,20"),
            (":K30*,20", ":K30*,20"),
            (":K30,20*", ":K30,20*"),
            (":K30,20", ":K30,20*"),
            (":K30*+1,20", ":K30*+1,20"),
            (":K30*+7,20", ":K30*+1,20"),
            (":K31*+1,20", ":K31*,20"),
            (":K3,4*+1", ":K3,4*+1"),
            (":C30,20", ":C30,20"),
            (":S30", ":S30"),
            ("T 30 , 20", ":T30,20"),
        ];
        for (input, canonical) in table {
            let parsed = grid(input);
            assert_eq!(parsed.to_string(), *canonical, "input {input}");
            assert_eq!(grid(canonical), parsed, "canonical form of {input} must round-trip");
        }
    }

    #[test]
    fn fields_and_topology_mapping() {
        let t = grid(":T30+5,20");
        assert_eq!((t.kind, t.width, t.height, t.horizontal_shift, t.vertical_shift), (GridKind::Torus, 30, 20, 5, 0));
        assert_eq!(t.topology(), None, "shifted tori have no core equivalent");
        assert_eq!(grid(":T30,20").topology(), Some(Topology::Torus));
        assert_eq!(grid(":P64,64").topology(), Some(Topology::Bounded));
        assert_eq!(grid(":K30*,20").topology(), Some(Topology::KleinBottle));
        assert_eq!(grid(":K30,20*").topology(), None);
        assert_eq!(grid(":C30,20").topology(), Some(Topology::CrossSurface));
        assert_eq!(grid(":S30").topology(), None);
        assert_eq!(grid(":T0,20").topology(), None);
        let c = grid(":C4,3");
        assert!(c.horizontal_twist && c.vertical_twist);
        assert!(!grid(":P30,0").is_finite());
        assert!(grid(":P30,20").is_finite());
        assert_eq!(grid(":T30,20").top_left(), (-15, -10));
        assert_eq!(grid(":T5,3").top_left(), (-2, -1));
        assert_eq!(BoundedGrid::torus(4, 3), grid(":T4,3"));
        // Golly's 5x3 grid spans x = -2..=2 and y = -1..=1.
        let small = grid(":P5,3");
        assert!(small.contains(-2, -1) && small.contains(2, 1) && small.contains(0, 0));
        assert!(!small.contains(3, 0) && !small.contains(-3, 0) && !small.contains(0, 2) && !small.contains(0, -2));
        // Infinite dimensions contain everything along them.
        let tube = grid(":T0,4");
        assert!(tube.contains(i64::MIN, -2) && tube.contains(i64::MAX, 1) && !tube.contains(0, 2));
        assert_eq!(BoundedGrid::plane(4, 3), grid(":P4,3"));
    }

    #[test]
    fn unbounded_suffixes() {
        for text in [":", "", ":T", ":P", ":T0,0", ":P0", ":T0"] {
            assert_eq!(BoundedGrid::parse(text).unwrap(), None, "{text}");
        }
        assert!(":T0,0".parse::<BoundedGrid>().is_err());
    }

    #[test]
    fn rejects_invalid_suffixes() {
        let cases: &[(&str, &str, Option<usize>)] = &[
            (":Q10", "unknown grid topology 'Q'", Some(2)),
            (":T10*,10", "only a Klein bottle can have a twist", Some(5)),
            (":P10+1,10", "a plane can't have a shift", Some(5)),
            (":S10+1", "a sphere can't have a shift", Some(5)),
            (":C10+1,10", "a cross-surface can't have a shift", Some(5)),
            (":K10+1,10*", "shift must be on its twisted edges", Some(5)),
            (":K10*,10*", "both horizontal and vertical twists", Some(9)),
            (":T0+1,10", "can't shift an infinite width", Some(4)),
            (":T10+1,10+1", "both horizontal and vertical shifts", Some(10)),
            (":T10,0+1", "can't shift an infinite height", Some(7)),
            (":S10,10", "a sphere is square", Some(5)),
            (":T10x", "unexpected 'x' after grid width", Some(5)),
            (":T10,10x", "unexpected 'x' after grid height", Some(8)),
            (":K0,10", "can't have an infinite width", None),
            (":C10,0", "can't have an infinite height", None),
            (":T3000000000,10", "exceeds the maximum", Some(3)),
            (":T10+,10", "expected a shift amount", Some(6)),
        ];
        for (input, message, column) in cases {
            let err = BoundedGrid::parse(input).expect_err(input);
            assert!(err.message.contains(message), "{input}: got `{err}`, expected `{message}`");
            assert_eq!(err.column, *column, "{input}: {err}");
        }
    }
}
