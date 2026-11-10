//! Reading Golly's metapixel tilings (`Patterns/HashLife/Metacell/*.mc.gz`).
//!
//! Brice Due's `tile-otcamp.pl` writes each tile as a trimmed 2048×2048
//! level-11 node (method B of the research note) and places tile `(i, j)` at
//! `(−2^(L−1) + 2048·i, 1 − 2^(L−1) + 2048·j)` in Golly's frame, where `L` is
//! the level of the file's root. This module recovers that geometry and the
//! rule/neighbourhood programmed into the tiles, so the demos can be checked
//! against the macro rule independently of our own construction.

use hashlife::Universe;

use crate::error::MetapixelError;
use crate::gzip;
use crate::programming::{Neighbourhood, Programming};
use crate::readout::TileGeometry;
use crate::spec::{Direction, RuleTable, TILE_PITCH, rule_bit_cell};

/// Largest number of tile columns or rows accepted from a Golly file
/// (Golly's own demos are at most 142 tiles wide).
pub const MAX_GOLLY_SIDE: u32 = 4096;

/// A Golly metapixel tiling loaded into Hashlife.
#[derive(Debug)]
pub struct GollyTiling {
    /// The physical pattern at the file's generation.
    pub universe: Universe,
    /// Where the tiles are.
    pub geometry: TileGeometry,
    /// Rule and neighbourhood programmed into every existing tile.
    pub programming: Programming,
}

/// Reads a Golly metapixel tiling from `.mc` or `.mc.gz` bytes.
///
/// # Errors
/// [`MetapixelError::Gzip`] for a corrupt archive,
/// [`MetapixelError::Hashlife`] for an invalid macrocell document and
/// [`MetapixelError::GollyTiling`] if the pattern is not a metapixel tiling
/// at generation 0 (no root line, cells outside the tile grid, more than
/// [`MAX_GOLLY_SIDE`] tiles per side, inconsistent programming).
pub fn read_golly_tiling(bytes: &[u8]) -> Result<GollyTiling, MetapixelError> {
    let decompressed;
    let raw = if gzip::is_gzip(bytes) {
        decompressed = gzip::decompress(bytes, gzip::DEFAULT_MAX_OUTPUT)?;
        &decompressed[..]
    } else {
        bytes
    };
    let text = std::str::from_utf8(raw)
        .map_err(|e| MetapixelError::GollyTiling(format!("the macrocell text is not UTF-8: {e}")))?;
    let level = root_level(text)?;
    if !(12..=62).contains(&level) {
        return Err(MetapixelError::GollyTiling(format!(
            "root level {level} cannot hold a grid of 2048×2048 tiles (expected 12..=62)"
        )));
    }
    let universe = hashlife::macrocell::read(text)?;
    if universe.generation() != 0 {
        return Err(MetapixelError::GollyTiling(format!(
            "the file is at generation {}; the tile programming can only be read at generation 0",
            universe.generation()
        )));
    }
    let half = 1i64 << (level - 1);
    let (origin_x, origin_y) = (-half, 1 - half);
    let (x0, y0, x1, y1) =
        universe.bounding_box().ok_or_else(|| MetapixelError::GollyTiling("the pattern is empty".into()))?;
    if x0 < origin_x || y0 < origin_y {
        return Err(MetapixelError::GollyTiling(format!(
            "cells at ({x0}, {y0}) lie before the first tile at ({origin_x}, {origin_y})"
        )));
    }
    let side = |extent: i64, what: &str| {
        u32::try_from(extent / TILE_PITCH + 1).ok().filter(|&n| n <= MAX_GOLLY_SIDE).ok_or_else(|| {
            MetapixelError::GollyTiling(format!(
                "the pattern spans {} tile {what}; at most {MAX_GOLLY_SIDE} are supported",
                extent / TILE_PITCH + 1
            ))
        })
    };
    // Both extents are non-negative and below 2^62 (cells lie inside the root).
    let columns = side(x1 - origin_x, "columns")?;
    let rows = side(y1 - origin_y, "rows")?;
    let geometry = TileGeometry { origin_x, origin_y, columns, rows };
    let programming = detect_programming(&universe, &geometry)?;
    Ok(GollyTiling { universe, geometry, programming })
}

/// Level of the root node (the last node line) of a macrocell document.
fn root_level(text: &str) -> Result<u32, MetapixelError> {
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['))
        .ok_or_else(|| MetapixelError::GollyTiling("the document has no node lines".into()))?;
    if line.starts_with(['.', '*', '$']) {
        return Err(MetapixelError::GollyTiling("the root is a single 8×8 leaf, not a tiling".into()));
    }
    line.split_whitespace()
        .next()
        .and_then(|level| level.parse().ok())
        .ok_or_else(|| MetapixelError::GollyTiling(format!("malformed root line `{line}`")))
}

/// Reads the programming of every existing tile at generation 0 (a rule
/// digit is programmed iff its completion cell is alive; a direction is
/// heard iff its fuse object is complete) and checks that all tiles agree,
/// as Brice Due's tiling rules require. Missing tiles (empty squares, as in
/// sparse tilings) are skipped.
///
/// # Errors
/// [`MetapixelError::GollyTiling`] if no tile exists, a fuse object is only
/// partly present, or two tiles are programmed differently, and the errors
/// of [`crate::readout::tile_populations`].
pub fn detect_programming(universe: &Universe, geometry: &TileGeometry) -> Result<Programming, MetapixelError> {
    let populations = crate::readout::tile_populations(universe, geometry)?;
    let mut found: Option<(Programming, (u32, u32))> = None;
    for (i, &population) in populations.iter().enumerate() {
        if population == 0 {
            continue;
        }
        let tile = (i as u32 % geometry.columns, i as u32 / geometry.columns);
        let programming = tile_programming(universe, geometry, tile)?;
        match found {
            None => found = Some((programming, tile)),
            Some((first, at)) if first != programming => {
                return Err(MetapixelError::GollyTiling(format!(
                    "tile {at:?} is programmed as {} ({}) but tile {tile:?} as {} ({})",
                    first.rule, first.neighbourhood, programming.rule, programming.neighbourhood
                )));
            }
            Some(_) => {}
        }
    }
    found.map(|(p, _)| p).ok_or_else(|| MetapixelError::GollyTiling("the tiling has no tiles".into()))
}

/// Programming of one tile at generation 0.
fn tile_programming(
    universe: &Universe,
    geometry: &TileGeometry,
    (col, row): (u32, u32),
) -> Result<Programming, MetapixelError> {
    let (ox, oy) = geometry.tile_origin(col, row);
    let alive = |(x, y): (i64, i64)| universe.get_cell(ox + x, oy + y);
    let mut birth = 0u16;
    let mut survive = 0u16;
    for digit in 0..=8u8 {
        if alive(rule_bit_cell(RuleTable::Birth, digit)) {
            birth |= 1 << digit;
        }
        if alive(rule_bit_cell(RuleTable::Survival, digit)) {
            survive |= 1 << digit;
        }
    }
    let rule =
        patterns::LifeRule::new(birth, survive).map_err(|e| MetapixelError::GollyTiling(format!("rule bits: {e}")))?;
    let mut heard = Vec::new();
    for fuse in Direction::ALL {
        let present = fuse.fuse_cells().iter().filter(|&&c| alive(c)).count();
        match present {
            0 => {}
            n if n == fuse.fuse_cells().len() => heard.push(fuse.deaf_when_fuse_removed()),
            n => {
                return Err(MetapixelError::GollyTiling(format!(
                    "the {} fuse of tile ({col}, {row}) is incomplete ({n} of {} cells)",
                    fuse.name(),
                    fuse.fuse_cells().len()
                )));
            }
        }
    }
    Ok(Programming { rule, neighbourhood: Neighbourhood::from_directions(heard) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_tilings_are_rejected_with_clear_errors() {
        let fail = |text: &str| read_golly_tiling(text.as_bytes()).unwrap_err().to_string();
        assert!(fail("[M2]\n#R B3/S23\n").contains("no node lines"));
        assert!(fail("[M2]\n#R B3/S23\n*$\n").contains("single 8×8 leaf"));
        assert!(fail("[M2]\n#R B3/S23\n*$\n4 1 0 0 0\n").contains("root level 4"));
        // A level-12 root at generation 5.
        let mut text = String::from("[M2]\n#R B3/S23\n#G 5\n*$\n");
        for level in 4..=12 {
            text.push_str(&format!("{level} {} 0 0 0\n", level - 3));
        }
        assert!(fail(&text).contains("generation 5"));
        // A single cell 2^40 cells away from the first tile: far too many tiles.
        let mut text = String::from("[M2]\n#R B3/S23\n*$\n");
        for level in 4..=42 {
            text.push_str(&format!("{level} 0 0 0 {}\n", level - 3));
        }
        assert!(fail(&text).contains("at most 4096"), "{}", fail(&text));
    }
}
