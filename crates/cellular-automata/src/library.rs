//! Pattern files embedded in the crate (in addition to the classics embedded
//! in the `patterns` crate): the permissively licensed Wireworld circuits of
//! `assets/patterns/wireworld/` (see its `PROVENANCE.md`).
//!
//! The McIntosh AND gate (`wireworld/and-gate-mcintosh`) comes from the Golly
//! pattern collection under GPL-2.0-or-later. Like the OTCA tiles it is never
//! compiled into the binary: it is read at run time from the asset directory
//! through [`crate::PatternLibrary::resolve`].

/// `(id, RLE text)` of every embedded pattern.
pub const EMBEDDED_PATTERNS: [(&str, &str); 5] = [
    ("wireworld/diode", include_str!("../../../assets/patterns/wireworld/diode.rle")),
    ("wireworld/clock-8", include_str!("../../../assets/patterns/wireworld/clock-8.rle")),
    ("wireworld/clock-12", include_str!("../../../assets/patterns/wireworld/clock-12.rle")),
    ("wireworld/or-gate", include_str!("../../../assets/patterns/wireworld/or-gate.rle")),
    ("wireworld/xor-gate", include_str!("../../../assets/patterns/wireworld/xor-gate.rle")),
];

/// The RLE text of an embedded pattern.
pub fn embedded_pattern(id: &str) -> Option<&'static str> {
    EMBEDDED_PATTERNS.iter().find(|(name, _)| *name == id).map(|(_, text)| *text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_patterns_parse_with_wireworld_states() {
        for (id, text) in EMBEDDED_PATTERNS {
            let p = patterns::rle::parse(text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(p.rule.as_deref(), Some("WireWorld"), "{id}");
            assert!(p.cells.iter().all(|c| (1..=3).contains(&c.state)), "{id}");
            let (x0, y0, x1, y1) = p.bounding_box().expect("non-empty");
            assert!(x0 >= 0 && y0 == 0 && x1 < 64 && y1 < 16, "{id}: small circuit placed at the origin");
        }
    }
}
