//! The macro rule and neighbourhood being edited in the Tools panel, and
//! what programming them changes in every tile.
//!
//! The OTCA metapixel computes any two-state outer-totalistic rule
//! (`B0` included) on any subset of the eight Moore directions: one cell
//! completes the broken eater of each birth / survival digit, and removing
//! a fuse object makes every tile deaf to one direction. The physical rule
//! stays B3/S23 whatever is programmed.

use metapixel::spec::{Direction, RuleTable, rule_bit_cell};
use metapixel::{Neighbourhood, Programming};
use patterns::LifeRule;

/// Draft of a macro rule and neighbourhood (applied by a rebuild).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleDraft {
    /// Birth digits 0..=8.
    pub birth: [bool; 9],
    /// Survival digits 0..=8.
    pub survive: [bool; 9],
    /// Listened directions, in [`Direction::ALL`] order.
    pub listens: [bool; 8],
}

impl Default for RuleDraft {
    fn default() -> Self {
        Self::from_programming(Programming::conway())
    }
}

impl RuleDraft {
    /// The draft of a programming.
    pub fn from_programming(p: Programming) -> Self {
        Self {
            birth: std::array::from_fn(|d| p.rule.births(d as u8)),
            survive: std::array::from_fn(|d| p.rule.survives(d as u8)),
            listens: Direction::ALL.map(|d| p.neighbourhood.listens(d)),
        }
    }

    /// The draft of the model's parameter texts, if they parse.
    pub fn from_texts(rule: &str, neighbourhood: &str) -> Option<Self> {
        Programming::parse(rule, neighbourhood).ok().map(Self::from_programming)
    }

    /// The rule as `B…/S…` text.
    pub fn rule_text(&self) -> String {
        self.programming().rule.to_string()
    }

    /// The neighbourhood as the model's parameter text.
    pub fn neighbourhood_text(&self) -> String {
        self.neighbourhood().to_string()
    }

    /// The listened directions.
    pub fn neighbourhood(&self) -> Neighbourhood {
        Neighbourhood::from_directions(Direction::ALL.into_iter().filter(|d| self.listens[d.index()]))
    }

    /// The programming (always valid: masks of digits 0..=8).
    pub fn programming(&self) -> Programming {
        let digits = |set: &[bool; 9]| (0..=8u8).filter(|&d| set[usize::from(d)]).collect::<Vec<_>>();
        let rule = LifeRule::from_counts(&digits(&self.birth), &digits(&self.survive)).unwrap_or(LifeRule::conway());
        Programming { rule, neighbourhood: self.neighbourhood() }
    }

    /// Replaces the rule digits by those of `rule`.
    pub fn set_rule(&mut self, rule: LifeRule) {
        self.birth = std::array::from_fn(|d| rule.births(d as u8));
        self.survive = std::array::from_fn(|d| rule.survives(d as u8));
    }

    /// Number of listened directions: the largest neighbour count a tile
    /// can see.
    pub fn max_count(&self) -> usize {
        self.listens.iter().filter(|l| **l).count()
    }

    /// Digits that are set but can never occur with this neighbourhood.
    pub fn unreachable_digits(&self) -> Vec<String> {
        let max = self.max_count();
        let mut out = Vec::new();
        for (letter, set) in [('B', &self.birth), ('S', &self.survive)] {
            for (d, on) in set.iter().enumerate() {
                if *on && d > max {
                    out.push(format!("{letter}{d}"));
                }
            }
        }
        out
    }
}

/// Named rules offered as presets (the pattern library's catalogue).
pub fn rule_presets() -> Vec<(&'static str, LifeRule)> {
    LifeRule::known_rules().collect()
}

/// Neighbourhood presets: label, tooltip and the listened directions.
pub fn neighbourhood_presets() -> [(&'static str, &'static str, Neighbourhood); 2] {
    [
        ("Moore", "All eight neighbours (every fuse present)", Neighbourhood::moore()),
        (
            "von Neumann",
            "N, E, S and W only: the four corner LWSS fuses are removed from every tile",
            Neighbourhood::von_neumann(),
        ),
    ]
}

/// Cells (tile-nominal) added to every tile to program the rule digits,
/// with their labels (`B3`, `S2`...).
pub fn programming_cells(programming: Programming) -> Vec<(String, (i64, i64))> {
    let mut out = Vec::new();
    for (table, letter) in [(RuleTable::Birth, 'B'), (RuleTable::Survival, 'S')] {
        for digit in 0..=8u8 {
            let set = match table {
                RuleTable::Birth => programming.rule.births(digit),
                RuleTable::Survival => programming.rule.survives(digit),
            };
            if set {
                out.push((format!("{letter}{digit}"), rule_bit_cell(table, digit)));
            }
        }
    }
    out
}

/// Fuse objects removed from every tile for the unheard directions:
/// `(object, deaf direction, cell count)`.
pub fn removed_fuses(programming: Programming) -> Vec<(Direction, Direction, usize)> {
    Direction::ALL
        .into_iter()
        .filter(|d| !programming.neighbourhood.listens(*d))
        .map(|deaf| {
            let object = deaf.fuse_silencing();
            (object, deaf, object.fuse_cells().len())
        })
        .collect()
}

/// Neighbour count of tile `(col, row)` in `grid` as the tile hears it
/// (unheard directions and tiles outside the grid count as OFF).
pub fn heard_count(grid: &metapixel::MacroGrid, programming: Programming, col: u32, row: u32) -> u8 {
    metapixel::MacroModel::new(programming).neighbour_count(grid, col, row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_round_trip_through_the_model_texts() {
        let draft = RuleDraft::from_texts("B36/S23", "von_neumann").unwrap();
        assert_eq!(draft.rule_text(), "B36/S23");
        assert_eq!(draft.neighbourhood_text(), "von_neumann");
        assert_eq!(draft.max_count(), 4);
        assert_eq!(draft.unreachable_digits(), ["B6"]);
        let custom = RuleDraft::from_texts("23/3", "N, E, SW").unwrap();
        assert_eq!(custom.rule_text(), "B3/S23");
        assert_eq!(custom.neighbourhood_text(), "N,E,SW");
        assert_eq!(RuleDraft::from_texts(&custom.rule_text(), &custom.neighbourhood_text()), Some(custom));
        assert!(RuleDraft::from_texts("B9/S", "moore").is_none());
        let b0 = RuleDraft::from_texts("B0/S", "none").unwrap();
        assert_eq!((b0.rule_text(), b0.neighbourhood_text(), b0.max_count()), ("B0/S".into(), "none".into(), 0));
        assert!(b0.unreachable_digits().is_empty(), "B0 needs no neighbour");
        assert_eq!(RuleDraft::default().programming(), Programming::conway());
    }

    #[test]
    fn presets_and_programming_cells() {
        let presets = rule_presets();
        assert!(presets.iter().any(|(name, rule)| *name == "Life" && *rule == LifeRule::conway()));
        let mut draft = RuleDraft::default();
        draft.set_rule(LifeRule::highlife());
        assert_eq!(draft.rule_text(), "B36/S23");
        let cells = programming_cells(Programming::conway());
        let labels: Vec<&str> = cells.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["B3", "S2", "S3"]);
        // The three cells the shipped B3/S23 tiles add (PROVENANCE.md).
        let coords: Vec<(i64, i64)> = cells.iter().map(|(_, c)| *c).collect();
        assert_eq!(coords, [(148, 1470), (162, 1498), (162, 1472)]);
        let removed = removed_fuses(Programming::parse("B3/S23", "von_neumann").unwrap());
        assert_eq!(removed.len(), 4);
        assert!(removed.iter().all(|(object, deaf, cells)| object.opposite() == *deaf && *cells == 9));
        let deaf_north = removed_fuses(Programming::parse("B3/S23", "NE,E,SE,S,SW,W,NW").unwrap());
        assert_eq!(deaf_north, [(Direction::N, Direction::N, 4)], "edge blocks silence their own edge");
        assert_eq!(neighbourhood_presets()[1].2, Neighbourhood::von_neumann());
    }

    #[test]
    fn heard_counts_follow_the_neighbourhood() {
        let grid = metapixel::MacroGrid::from_rows(&["O.O", "...", "O.."]).unwrap();
        assert_eq!(heard_count(&grid, Programming::conway(), 1, 1), 3);
        let vn = Programming::parse("B3/S23", "von_neumann").unwrap();
        assert_eq!(heard_count(&grid, vn, 1, 1), 0);
        assert_eq!(heard_count(&grid, Programming::conway(), 0, 0), 0, "outside the grid counts as OFF");
    }
}
