//! Embedded library of classic Conway's Life patterns.
//!
//! The cell data was transcribed by hand for this crate (a configuration of
//! cells is a mathematical fact; no third-party file was copied). Ids,
//! categories and kinds match `assets/patterns/catalog.toml` so tools can
//! fall back to this list transparently. Populations, periods, displacements
//! and lifespans are checked by the crate's tests with an independent Life
//! simulator (`tests/builtin_dynamics.rs`).

use crate::error::PatternError;
use crate::rle;

use super::CatalogEntry;
use crate::pattern::Pattern;

/// One embedded pattern with its metadata and RLE source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinPattern {
    /// Identifier (same as in `assets/patterns/catalog.toml`).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Category (`still-lifes`, `oscillators`, `spaceships`, `guns`, `methuselahs`).
    pub category: &'static str,
    /// Kind (`still_life`, `oscillator`, `spaceship`, `gun`, `methuselah`).
    pub kind: &'static str,
    /// Short description.
    pub description: &'static str,
    /// Discoverer, when well established.
    pub discoverer: Option<&'static str>,
    /// Year of discovery, when well established.
    pub year: Option<i32>,
    /// Period (0 for still lifes and methuselahs, as in the asset catalog).
    pub period: u64,
    /// Displacement per period for spaceships, in this RLE's orientation.
    pub displacement: Option<(i64, i64)>,
    /// Initial population.
    pub population: u64,
    /// Bounding-box width.
    pub width: u64,
    /// Bounding-box height.
    pub height: u64,
    /// Expected behaviour, verified by the tests.
    pub expected_behavior: &'static str,
    /// RLE source (header and data, rule B3/S23).
    pub rle: &'static str,
}

const CONWAY: Option<&str> = Some("John Conway");

static BUILTINS: [BuiltinPattern; 18] = [
    BuiltinPattern {
        id: "block",
        name: "Block",
        category: "still-lifes",
        kind: "still_life",
        description: "A 2x2 square: the smallest and most common still life.",
        discoverer: CONWAY,
        year: None,
        period: 0,
        displacement: None,
        population: 4,
        width: 2,
        height: 2,
        expected_behavior: "Static forever.",
        rle: "x = 2, y = 2, rule = B3/S23\n2o$2o!\n",
    },
    BuiltinPattern {
        id: "beehive",
        name: "Beehive",
        category: "still-lifes",
        kind: "still_life",
        description: "A six-cell hexagonal still life, second most common after the block.",
        discoverer: None,
        year: None,
        period: 0,
        displacement: None,
        population: 6,
        width: 4,
        height: 3,
        expected_behavior: "Static forever.",
        rle: "x = 4, y = 3, rule = B3/S23\nb2o$o2bo$b2o!\n",
    },
    BuiltinPattern {
        id: "loaf",
        name: "Loaf",
        category: "still-lifes",
        kind: "still_life",
        description: "A seven-cell still life: a beehive with one corner pushed in.",
        discoverer: None,
        year: None,
        period: 0,
        displacement: None,
        population: 7,
        width: 4,
        height: 4,
        expected_behavior: "Static forever.",
        rle: "x = 4, y = 4, rule = B3/S23\nb2o$o2bo$bobo$2bo!\n",
    },
    BuiltinPattern {
        id: "boat",
        name: "Boat",
        category: "still-lifes",
        kind: "still_life",
        description: "The only five-cell still life.",
        discoverer: None,
        year: None,
        period: 0,
        displacement: None,
        population: 5,
        width: 3,
        height: 3,
        expected_behavior: "Static forever.",
        rle: "x = 3, y = 3, rule = B3/S23\n2o$obo$bo!\n",
    },
    BuiltinPattern {
        id: "tub",
        name: "Tub",
        category: "still-lifes",
        kind: "still_life",
        description: "Four cells in a diamond around an empty centre.",
        discoverer: None,
        year: None,
        period: 0,
        displacement: None,
        population: 4,
        width: 3,
        height: 3,
        expected_behavior: "Static forever.",
        rle: "x = 3, y = 3, rule = B3/S23\nbo$obo$bo!\n",
    },
    BuiltinPattern {
        id: "blinker",
        name: "Blinker",
        category: "oscillators",
        kind: "oscillator",
        description: "Three cells in a row, alternating between horizontal and vertical.",
        discoverer: CONWAY,
        year: None,
        period: 2,
        displacement: None,
        population: 3,
        width: 3,
        height: 1,
        expected_behavior: "Period 2 oscillation.",
        rle: "x = 3, y = 1, rule = B3/S23\n3o!\n",
    },
    BuiltinPattern {
        id: "toad",
        name: "Toad",
        category: "oscillators",
        kind: "oscillator",
        description: "Two offset rows of three cells that breathe in and out.",
        discoverer: Some("Simon Norton"),
        year: None,
        period: 2,
        displacement: None,
        population: 6,
        width: 4,
        height: 2,
        expected_behavior: "Period 2 oscillation.",
        rle: "x = 4, y = 2, rule = B3/S23\nb3o$3o!\n",
    },
    BuiltinPattern {
        id: "beacon",
        name: "Beacon",
        category: "oscillators",
        kind: "oscillator",
        description: "Two diagonal blocks whose touching corners blink on and off.",
        discoverer: CONWAY,
        year: None,
        period: 2,
        displacement: None,
        population: 8,
        width: 4,
        height: 4,
        expected_behavior: "Period 2 oscillation (8 and 6 cells).",
        rle: "x = 4, y = 4, rule = B3/S23\n2o$2o$2b2o$2b2o!\n",
    },
    BuiltinPattern {
        id: "pulsar",
        name: "Pulsar",
        category: "oscillators",
        kind: "oscillator",
        description: "A 48-cell oscillator with four-fold symmetry, the most common period-3 oscillator.",
        discoverer: CONWAY,
        year: None,
        period: 3,
        displacement: None,
        population: 48,
        width: 13,
        height: 13,
        expected_behavior: "Period 3 oscillation.",
        rle: "x = 13, y = 13, rule = B3/S23\n2b3o3b3o2$o4bobo4bo$o4bobo4bo$o4bobo4bo$2b3o3b3o2$2b3o3b3o$o4bobo4bo$o\n\
              4bobo4bo$o4bobo4bo2$2b3o3b3o!\n",
    },
    BuiltinPattern {
        id: "pentadecathlon",
        name: "Pentadecathlon",
        category: "oscillators",
        kind: "oscillator",
        description: "A period-15 oscillator that starts as a row of ten cells.",
        discoverer: CONWAY,
        year: None,
        period: 15,
        displacement: None,
        population: 12,
        width: 10,
        height: 3,
        expected_behavior: "Period 15 oscillation.",
        rle: "x = 10, y = 3, rule = B3/S23\n2bo4bo$2ob4ob2o$2bo4bo!\n",
    },
    BuiltinPattern {
        id: "glider",
        name: "Glider",
        category: "spaceships",
        kind: "spaceship",
        description: "The smallest spaceship: five cells moving one cell diagonally every four generations.",
        discoverer: Some("Richard K. Guy"),
        year: Some(1970),
        period: 4,
        displacement: Some((1, 1)),
        population: 5,
        width: 3,
        height: 3,
        expected_behavior: "Moves diagonally (down and right) at c/4.",
        rle: "x = 3, y = 3, rule = B3/S23\nbo$2bo$3o!\n",
    },
    BuiltinPattern {
        id: "lwss",
        name: "Lightweight spaceship (LWSS)",
        category: "spaceships",
        kind: "spaceship",
        description: "The smallest orthogonal spaceship, travelling two cells every four generations.",
        discoverer: CONWAY,
        year: Some(1970),
        period: 4,
        displacement: Some((-2, 0)),
        population: 9,
        width: 5,
        height: 4,
        expected_behavior: "Moves left at c/2.",
        rle: "x = 5, y = 4, rule = B3/S23\nbo2bo$o$o3bo$4o!\n",
    },
    BuiltinPattern {
        id: "mwss",
        name: "Middleweight spaceship (MWSS)",
        category: "spaceships",
        kind: "spaceship",
        description: "An eleven-cell orthogonal spaceship, one cell longer than the LWSS.",
        discoverer: CONWAY,
        year: Some(1970),
        period: 4,
        displacement: Some((-2, 0)),
        population: 11,
        width: 6,
        height: 5,
        expected_behavior: "Moves left at c/2.",
        rle: "x = 6, y = 5, rule = B3/S23\n3bo$bo3bo$o$o4bo$5o!\n",
    },
    BuiltinPattern {
        id: "hwss",
        name: "Heavyweight spaceship (HWSS)",
        category: "spaceships",
        kind: "spaceship",
        description: "A thirteen-cell orthogonal spaceship, the largest of the three classic c/2 ships.",
        discoverer: CONWAY,
        year: Some(1970),
        period: 4,
        displacement: Some((-2, 0)),
        population: 13,
        width: 7,
        height: 5,
        expected_behavior: "Moves left at c/2.",
        rle: "x = 7, y = 5, rule = B3/S23\n3b2o$bo4bo$o$o5bo$6o!\n",
    },
    BuiltinPattern {
        id: "gosper-glider-gun",
        name: "Gosper glider gun",
        category: "guns",
        kind: "gun",
        description: "The first known gun: two queen bee shuttles stabilised by blocks emit a glider every 30 generations.",
        discoverer: Some("Bill Gosper"),
        year: Some(1970),
        period: 30,
        displacement: None,
        population: 36,
        width: 36,
        height: 9,
        expected_behavior: "Emits one glider (moving down and right) every 30 generations.",
        rle: "x = 36, y = 9, rule = B3/S23\n24bo$22bobo$12b2o6b2o12b2o$11bo3bo4b2o12b2o$2o8bo5bo3b2o$2o8bo3bob2o4b\n\
              obo$10bo5bo7bo$11bo3bo$12b2o!\n",
    },
    BuiltinPattern {
        id: "r-pentomino",
        name: "R-pentomino",
        category: "methuselahs",
        kind: "methuselah",
        description: "Five cells that evolve for over a thousand generations, releasing six gliders.",
        discoverer: CONWAY,
        year: None,
        period: 0,
        displacement: None,
        population: 5,
        width: 3,
        height: 3,
        expected_behavior: "Stabilises at generation 1103 with 116 cells (including 6 gliders).",
        rle: "x = 3, y = 3, rule = B3/S23\nb2o$2o$bo!\n",
    },
    BuiltinPattern {
        id: "diehard",
        name: "Diehard",
        category: "methuselahs",
        kind: "methuselah",
        description: "Seven cells that survive for 130 generations and then vanish completely.",
        discoverer: None,
        year: None,
        period: 0,
        displacement: None,
        population: 7,
        width: 8,
        height: 3,
        expected_behavior: "Dies out completely at generation 130.",
        rle: "x = 8, y = 3, rule = B3/S23\n6bo$2o$bo3b3o!\n",
    },
    BuiltinPattern {
        id: "acorn",
        name: "Acorn",
        category: "methuselahs",
        kind: "methuselah",
        description: "Seven cells that grow into a large debris field and 13 gliders before settling.",
        discoverer: Some("Charles Corderman"),
        year: None,
        period: 0,
        displacement: None,
        population: 7,
        width: 7,
        height: 3,
        expected_behavior: "Stabilises at generation 5206 with 633 cells (including 13 gliders).",
        rle: "x = 7, y = 3, rule = B3/S23\nbo$3bo$2o2b3o!\n",
    },
];

/// The embedded classic patterns (block, beehive, loaf, boat, tub, blinker,
/// toad, beacon, pulsar, pentadecathlon, glider, LWSS, MWSS, HWSS, Gosper
/// glider gun, R-pentomino, diehard, acorn).
pub fn builtin_patterns() -> &'static [BuiltinPattern] {
    &BUILTINS
}

impl BuiltinPattern {
    /// Parses the embedded RLE and fills name, description and author.
    pub fn pattern(&self) -> Result<Pattern, PatternError> {
        let mut pattern = rle::parse(self.rle)?;
        pattern.name = self.name.to_string();
        pattern.description = self.description.to_string();
        pattern.author = self.discoverer.unwrap_or_default().to_string();
        Ok(pattern)
    }

    /// Catalog metadata for this pattern (`file` is a virtual `<id>.rle`).
    pub fn entry(&self) -> CatalogEntry {
        CatalogEntry {
            id: self.id.to_string(),
            name: self.name.to_string(),
            file: format!("{}.rle", self.id),
            category: Some(self.category.to_string()),
            kind: Some(self.kind.to_string()),
            description: Some(self.description.to_string()),
            discoverer: self.discoverer.map(str::to_string),
            year: self.year,
            period: Some(self.period),
            displacement: self.displacement,
            width: Some(self.width),
            height: Some(self.height),
            population: Some(self.population),
            expected_behavior: Some(self.expected_behavior.to_string()),
            complexity: Some(if self.kind == "gun" { 3 } else { 1 }),
            source: Some("Transcribed by hand for Emergence Lab (embedded in the patterns crate)".to_string()),
            license: Some("Public-domain mathematical object (transcribed)".to_string()),
            rule: Some("B3/S23".to_string()),
            extra: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_with_the_declared_population_and_size() {
        for builtin in builtin_patterns() {
            let document = rle::parse_document(builtin.rle).unwrap_or_else(|e| panic!("{}: {e}", builtin.id));
            let pattern = &document.pattern;
            assert_eq!(pattern.population() as u64, builtin.population, "{}", builtin.id);
            assert_eq!(pattern.bounding_box().map(|(x, y, ..)| (x, y)), Some((0, 0)), "{}", builtin.id);
            assert_eq!(document.declared_size, Some((builtin.width, builtin.height)), "{}", builtin.id);
            assert_eq!((pattern.width(), pattern.height()), (builtin.width, builtin.height), "{}", builtin.id);
            assert_eq!(pattern.rule.as_deref(), Some("B3/S23"));
            // The canonical writer reproduces the embedded text exactly.
            assert_eq!(rle::write(pattern), builtin.rle, "{}", builtin.id);
        }
    }

    #[test]
    fn known_populations() {
        let expected = [
            ("glider", 5),
            ("lwss", 9),
            ("mwss", 11),
            ("hwss", 13),
            ("gosper-glider-gun", 36),
            ("pulsar", 48),
            ("pentadecathlon", 12),
            ("r-pentomino", 5),
            ("acorn", 7),
            ("diehard", 7),
            ("block", 4),
            ("beehive", 6),
            ("loaf", 7),
            ("boat", 5),
            ("tub", 4),
            ("blinker", 3),
            ("toad", 6),
            ("beacon", 8),
        ];
        assert_eq!(expected.len(), builtin_patterns().len());
        for (id, population) in expected {
            let builtin = builtin_patterns().iter().find(|b| b.id == id).unwrap();
            assert_eq!(builtin.pattern().unwrap().population(), population, "{id}");
        }
    }

    #[test]
    fn ids_are_unique_and_entries_complete() {
        let mut ids: Vec<_> = builtin_patterns().iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), builtin_patterns().len());
        let glider = builtin_patterns().iter().find(|b| b.id == "glider").unwrap().entry();
        assert_eq!((glider.width, glider.height, glider.displacement), (Some(3), Some(3), Some((1, 1))));
        assert_eq!(glider.file, "glider.rle");
    }
}
