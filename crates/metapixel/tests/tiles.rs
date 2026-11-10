//! Tile loading and programming against the published construction
//! (research note §2 and §4, `otca-spec.json`).

mod common;

use std::collections::BTreeSet;

use metapixel::spec::{
    self, Direction, PERIOD, POPULATION_OFF_UNPROGRAMMED, POPULATION_ON_UNPROGRAMMED, READOUT_ON_COUNT, READOUT_RECT,
    RuleTable, TILE_BOUNDING_BOX, TRANSITION_END, TRANSITION_START,
};
use metapixel::tile::{TileState, parse_tile};
use metapixel::{MetapixelError, Neighbourhood, Programming, TileSet};
use serde_json::Value;

fn spec_json() -> Value {
    let text = std::fs::read_to_string(common::otca_dir().join("otca-spec.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn point(v: &Value) -> (i64, i64) {
    (v[0].as_i64().unwrap(), v[1].as_i64().unwrap())
}

fn set(cells: &[(i64, i64)]) -> BTreeSet<(i64, i64)> {
    cells.iter().copied().collect()
}

#[test]
fn unprogrammed_tiles_have_the_published_populations_and_bounding_box() {
    let tiles = common::tiles();
    assert_eq!(tiles.cells(TileState::Off).len(), 23_920);
    assert_eq!(tiles.cells(TileState::On).len(), 66_203);
    for state in [TileState::Off, TileState::On] {
        let cells = tiles.cells(state);
        let (x0, x1) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.0).max().unwrap());
        let (y0, y1) = (cells.iter().map(|c| c.1).min().unwrap(), cells.iter().map(|c| c.1).max().unwrap());
        assert_eq!((x0, y0, x1, y1), (-5, -5, 2052, 2052), "{state:?} bounding box in tile-nominal coordinates");
        // Only the 4 tubs and the outer cells of the 4 corner blocks lie
        // outside the nominal square: 28 cells (research note §2.1).
        let outside = cells.iter().filter(|&&(x, y)| !(0..2048).contains(&x) || !(0..2048).contains(&y)).count();
        assert_eq!(outside, 28, "{state:?}");
    }
}

#[test]
fn b3s23_programming_reproduces_the_shipped_programmed_tiles_exactly() {
    let programmed = common::tiles().program(Programming::conway());
    for (state, file, population) in
        [(TileState::Off, "OTCAmetapixel-OFF-B3S23.rle", 23_923), (TileState::On, "OTCAmetapixel-ON-B3S23.rle", 66_206)]
    {
        let text = std::fs::read_to_string(common::otca_dir().join(file)).unwrap();
        let shipped = parse_tile(&text, state).unwrap();
        assert_eq!(shipped.len(), population);
        assert_eq!(programmed.cells(state), &shipped[..], "{file}: cell-for-cell identical");
        let base = set(common::tiles().cells(state));
        let added: Vec<_> = set(&shipped).difference(&base).copied().collect();
        assert_eq!(added, [(148, 1470), (162, 1472), (162, 1498)], "B3, S3 and S2 (tile-nominal)");
    }
    // The generated RLE (file coordinates) parses back to the same tile.
    let rle = programmed.to_rle(TileState::On);
    assert_eq!(parse_tile(&rle, TileState::On).unwrap(), programmed.cells(TileState::On));
}

#[test]
fn rule_bits_match_the_spec_for_a_second_rule_and_every_digit() {
    let json = spec_json();
    for (table, key) in [(RuleTable::Birth, "B"), (RuleTable::Survival, "S")] {
        for digit in 0..=8u8 {
            let expected = point(&json["rule_bits"][key][digit.to_string()]);
            assert_eq!(spec::rule_bit_cell(table, digit), expected, "{key}{digit}");
        }
    }
    // HighLife: B36/S23 adds exactly B3, B6, S2, S3 to both tiles.
    let tiles = common::tiles();
    let programmed = tiles.program(Programming::parse("B36/S23", "moore").unwrap());
    for state in [TileState::Off, TileState::On] {
        let added: BTreeSet<_> = set(programmed.cells(state)).difference(&set(tiles.cells(state))).copied().collect();
        let expected: BTreeSet<_> = [(148, 1470), (148, 1424), (162, 1498), (162, 1472)].into_iter().collect();
        assert_eq!(added, expected, "{state:?}");
        assert_eq!(programmed.cells(state).len(), tiles.cells(state).len() + 4);
    }
    // Every digit: B012345678/S012345678 adds 18 distinct cells.
    let all = tiles.program(Programming::parse("B012345678/S012345678", "moore").unwrap());
    assert_eq!(all.cells(TileState::Off).len(), 23_920 + 18);
}

#[test]
fn fuse_removal_matches_the_spec_table() {
    let json = spec_json();
    let fuses = &json["neighbourhood_fuses"];
    for d in Direction::ALL {
        let expected: Vec<(i64, i64)> = fuses[d.name()].as_array().unwrap().iter().map(point).collect();
        assert_eq!(set(d.fuse_cells()), set(&expected), "{} fuse cells", d.name());
        assert_eq!(fuses["deaf_direction_when_removed"][d.name()].as_str().unwrap(), d.deaf_when_fuse_removed().name());
    }
    let tiles = common::tiles();
    // von Neumann removes the four corner LWSSs: 36 cells from each tile.
    let vn = tiles.program(Programming::parse("B3/S23", "von_neumann").unwrap());
    let moore = tiles.program(Programming::conway());
    for state in [TileState::Off, TileState::On] {
        let removed: BTreeSet<_> = set(moore.cells(state)).difference(&set(vn.cells(state))).copied().collect();
        assert_eq!(removed.len(), 36);
        for corner in [Direction::NW, Direction::NE, Direction::SE, Direction::SW] {
            assert!(corner.fuse_cells().iter().all(|c| removed.contains(c)));
        }
    }
    // Deaf to NW only = remove the SE corner LWSS.
    let deaf_nw = Neighbourhood::from_directions(Direction::ALL.into_iter().filter(|&d| d != Direction::NW));
    let p = tiles.program(Programming { rule: patterns::LifeRule::conway(), neighbourhood: deaf_nw });
    let removed: BTreeSet<_> =
        set(moore.cells(TileState::Off)).difference(&set(p.cells(TileState::Off))).copied().collect();
    assert_eq!(removed, set(Direction::SE.fuse_cells()));
    // Deaf to W only = remove the W edge block (the eater next to it stays).
    let deaf_w = Neighbourhood::from_directions(Direction::ALL.into_iter().filter(|&d| d != Direction::W));
    let p = tiles.program(Programming { rule: patterns::LifeRule::conway(), neighbourhood: deaf_w });
    assert_eq!(p.cells(TileState::On).len(), moore.cells(TileState::On).len() - 4);
}

#[test]
fn spec_constants_match_the_json() {
    let json = spec_json();
    assert_eq!(json["period"].as_u64(), Some(PERIOD));
    assert_eq!(json["tile_pitch"].as_i64(), Some(spec::TILE_PITCH));
    let bbox: Vec<i64> = json["tile_bbox_nominal"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!(bbox, [TILE_BOUNDING_BOX.x0, TILE_BOUNDING_BOX.y0, TILE_BOUNDING_BOX.x1, TILE_BOUNDING_BOX.y1]);
    assert_eq!(json["population_unprogrammed"]["OFF"].as_u64(), Some(POPULATION_OFF_UNPROGRAMMED as u64));
    assert_eq!(json["population_unprogrammed"]["ON"].as_u64(), Some(POPULATION_ON_UNPROGRAMMED as u64));
    let readout = &json["readout"];
    let rect: Vec<i64> =
        readout["rect_nominal_inclusive"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!(rect, [READOUT_RECT.x0, READOUT_RECT.y0, READOUT_RECT.x1, READOUT_RECT.y1]);
    assert_eq!(readout["ON_count_when_t_mod_46_eq_0"].as_u64(), Some(READOUT_ON_COUNT));
    assert_eq!(readout["ON_count_range_steady"][0].as_u64(), Some(spec::READOUT_ON_STEADY_MIN));
    assert_eq!(readout["ON_count_period"].as_u64(), Some(spec::READOUT_ON_PERIOD));
    assert_eq!(readout["transition_window_t_mod_P"][0].as_u64(), Some(TRANSITION_START));
    assert_eq!(readout["transition_window_t_mod_P"][1].as_u64(), Some(TRANSITION_END));
}

#[test]
fn loading_errors_are_clear() {
    let missing = std::path::Path::new("/nonexistent/otca");
    let error = TileSet::from_dir(missing).unwrap_err();
    assert!(matches!(error, MetapixelError::TileFileMissing { .. }));
    assert!(error.to_string().contains("OTCAmetapixel-OFF.rle") && error.to_string().contains("PROVENANCE"));
    // A programmed tile is not an unprogrammed base tile.
    let dir = common::otca_dir();
    let off_b3s23 = std::fs::read_to_string(dir.join("OTCAmetapixel-OFF-B3S23.rle")).unwrap();
    let on = std::fs::read_to_string(dir.join("OTCAmetapixel-ON.rle")).unwrap();
    let error = TileSet::from_rle(&off_b3s23, &on).unwrap_err();
    assert!(matches!(error, MetapixelError::TileMismatch { .. }), "{error}");
    let error = TileSet::from_rle("x = 3, y = 1\n3o!", &on).unwrap_err();
    assert!(error.to_string().contains("population"), "{error}");
    let error = TileSet::from_rle("x = 3, y = 1\n3%!", &on).unwrap_err();
    assert!(matches!(error, MetapixelError::TileParse { .. }), "{error}");
}
