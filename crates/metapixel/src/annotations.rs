//! Labelled components of one tile, for micro-level overlays.
//!
//! Positions are tile-nominal (add the tile's origin for plane
//! coordinates). Programming sites, fuses and the readout rectangle are
//! **verified** by simulation (research note §4–§5). The other components
//! are subpattern anchors from Golly's `metafier` and Brice Due's slide
//! captions; their *role* in the mechanism is an interpretation that was
//! not tested, and they are flagged as unverified.

use serde::{Deserialize, Serialize};

use crate::programming::Programming;
use crate::spec::{CellRect, Direction, READOUT_RECT, RuleTable, rule_eater_rect};

/// What an annotation marks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationKind {
    /// A rule-table eater (`B0`..`B8`, `S0`..`S8`).
    RuleBit {
        /// Birth or survival table.
        table: RuleTable,
        /// Neighbour count 0..=8.
        digit: u8,
        /// Whether this construction completes the eater (digit in the rule).
        programmed: bool,
    },
    /// A neighbourhood fuse object (corner LWSS or edge block).
    Fuse {
        /// Name of the object (the corner or edge it sits on).
        object: Direction,
        /// The neighbour direction it lets every tile hear.
        hears: Direction,
        /// Whether the object is present in this construction.
        present: bool,
    },
    /// The nominal readout rectangle (core of the display).
    Readout,
    /// The 1720×1720 display ("pixel") area.
    Display,
    /// A circuit component whose role is inferred from slide captions.
    Mechanism,
}

/// One labelled region of a tile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Short label to draw (`"B3"`, `"NE fuse"`, `"readout"`...).
    pub label: String,
    /// Kind and programming details.
    pub kind: AnnotationKind,
    /// Tile-nominal rectangle (for anchors of unknown extent, one cell).
    pub rect: CellRect,
    /// Whether the position *and* role were verified by simulation.
    pub verified: bool,
    /// One-sentence explanation (states when something is unverified).
    pub note: String,
}

fn point(x: i64, y: i64) -> CellRect {
    CellRect { x0: x, y0: y, x1: x, y1: y }
}

fn bounding(cells: &[(i64, i64)]) -> CellRect {
    cells.iter().fold(CellRect { x0: i64::MAX, y0: i64::MAX, x1: i64::MIN, y1: i64::MIN }, |r, &(x, y)| CellRect {
        x0: r.x0.min(x),
        y0: r.y0.min(y),
        x1: r.x1.max(x),
        y1: r.y1.max(y),
    })
}

/// Unverified circuit anchors: `(label, x0, y0, x1, y1, note)`.
const MECHANISMS: [(&str, i64, i64, i64, i64, &str); 11] = [
    ("linear clock gun", 46, 1924, 46, 1924, "Glider stream up the west side (slide 22); presumed master sequencer."),
    ("linear clock cap", 83, 401, 83, 401, "End of the linear clock glider stream (slide 22)."),
    ("MWSS track", 521, 1973, 521, 1973, "MWSS circulating around the perimeter (slide 23); presumed state broadcast."),
    (
        "state boat",
        569,
        1970,
        571,
        1972,
        "Boat present in the OFF tile and absent in the ON tile (verified difference); its latch role is inferred.",
    ),
    ("LWSS packet gun", 108, 710, 108, 710, "Presumed to turn neighbour gliders into an LWSS count packet (slide 24)."),
    ("sync buffer", 114, 958, 190, 1040, "Slide 25 caption."),
    ("p46 to p40 converter", 116, 1059, 116, 1059, "Slide 25: 'really 14 p184 guns'."),
    ("B/S result latches", 108, 1317, 108, 1317, "Slide 26: logic core latches for the B/S results."),
    ("boat latch NE", 120, 232, 120, 232, "Slide 29: toggle distribution."),
    ("boat latch SW", 1534, 1849, 1534, 1849, "Slide 29: toggle distribution."),
    ("HWSS gun", 106, 87, 106, 87, "Slide 30: polls the latches and drives the top display emitters."),
];

/// Every annotation of a tile programmed with `programming`.
pub fn tile_annotations(programming: Programming) -> Vec<Annotation> {
    let mut out = Vec::new();
    for table in [RuleTable::Birth, RuleTable::Survival] {
        for digit in 0..=8u8 {
            let programmed = match table {
                RuleTable::Birth => programming.rule.births(digit),
                RuleTable::Survival => programming.rule.survives(digit),
            };
            out.push(Annotation {
                label: format!("{}{digit}", table.letter()),
                kind: AnnotationKind::RuleBit { table, digit, programmed },
                rect: rule_eater_rect(table, digit),
                verified: true,
                note: if programmed {
                    "Completed eater: this neighbour count is in the rule.".into()
                } else {
                    "Broken eater (burns cleanly): this neighbour count is not in the rule.".into()
                },
            });
        }
    }
    for object in Direction::ALL {
        let hears = object.deaf_when_fuse_removed();
        let present = programming.neighbourhood.listens(hears);
        let shape = if object.is_corner() { "LWSS" } else { "block" };
        out.push(Annotation {
            label: format!("{} fuse", object.name()),
            kind: AnnotationKind::Fuse { object, hears, present },
            rect: bounding(object.fuse_cells()),
            verified: true,
            note: format!(
                "{} {shape}: {} every tile hears its {} neighbour.",
                object.name(),
                if present { "present, so" } else { "removed, so no longer does" },
                hears.name()
            ),
        });
    }
    out.push(Annotation {
        label: "readout".into(),
        kind: AnnotationKind::Readout,
        rect: READOUT_RECT,
        verified: true,
        note: "Core counted for the macro state: 0 = OFF, 29 795 = ON at t = k·35 328.".into(),
    });
    out.push(Annotation {
        label: "display".into(),
        kind: AnnotationKind::Display,
        rect: CellRect { x0: 166, y0: 101, x1: 1946, y1: 1881 },
        verified: true,
        note: "Bounding box of the 43 nested LWSS lanes that differ between ON and OFF.".into(),
    });
    for (label, x0, y0, x1, y1, note) in MECHANISMS {
        out.push(Annotation {
            label: label.into(),
            kind: AnnotationKind::Mechanism,
            rect: if (x0, y0) == (x1, y1) { point(x0, y0) } else { CellRect { x0, y0, x1, y1 } },
            verified: false,
            note: format!("UNVERIFIED role (subpattern anchor from metafier / slide captions). {note}"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotations_reflect_the_programming() {
        let p = Programming::parse("B3/S23", "von_neumann").unwrap();
        let notes = tile_annotations(p);
        let programmed: Vec<&str> = notes
            .iter()
            .filter(|a| matches!(a.kind, AnnotationKind::RuleBit { programmed: true, .. }))
            .map(|a| a.label.as_str())
            .collect();
        assert_eq!(programmed, ["B3", "S2", "S3"]);
        let removed: Vec<&str> = notes
            .iter()
            .filter(|a| matches!(a.kind, AnnotationKind::Fuse { present: false, .. }))
            .map(|a| a.label.as_str())
            .collect();
        assert_eq!(removed, ["NE fuse", "SE fuse", "SW fuse", "NW fuse"]);
        assert!(notes.iter().filter(|a| !a.verified).all(|a| a.note.starts_with("UNVERIFIED")));
        let b3 = notes.iter().find(|a| a.label == "B3").unwrap();
        assert!(b3.rect.contains(148, 1470));
    }
}
