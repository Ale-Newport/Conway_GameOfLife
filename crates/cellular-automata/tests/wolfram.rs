//! Elementary cellular automata: rule numbering, boundaries, rule 90 as
//! Pascal's triangle mod 2, rule 30's centre column (OEIS A051023),
//! conservation in rule 184 and the space-time view.

use cellular_automata::{Boundary, EcaStart, GridViewRequest, WolframCommand, WolframConfig, WolframSim};
use simulation_core::{ParamValue, Simulation};

fn sim(rule: u8, width: u32, boundary: Boundary, start: EcaStart) -> WolframSim {
    let config = WolframConfig { rule, width, boundary, start, density: 0.3, history: 4096 };
    WolframSim::new(&config, 37).unwrap()
}

/// First 257 terms of OEIS A051023 (centre column of rule 30 from a single cell).
const A051023: &str = "11011100110001011001001110101110011101010110000110010101101010111111000011110001010111000001001011000111000110110110100000001000111110111010011100011101011100000110010001100111100111111000000111111101100101101110000010110001101100011000111011011001010111111";

#[test]
fn rule_90_is_pascals_triangle_mod_2() {
    let width = 2 * 300 + 3;
    let mut s = sim(90, width as u32, Boundary::Fixed0, EcaStart::SingleCell);
    let centre = width as i64 / 2;
    // Row t holds C(t, k) mod 2 at x = centre − t + 2k (k = 0..=t), zero elsewhere.
    let mut pascal = vec![1u8];
    for t in 0..=300u64 {
        let row = s.current_row();
        for (x, &cell) in row.iter().enumerate() {
            let d = x as i64 - centre;
            let expected = if d.abs() <= t as i64 && (d + t as i64) % 2 == 0 {
                pascal[((d + t as i64) / 2) as usize] == 1
            } else {
                false
            };
            assert_eq!(cell, expected, "row {t}, offset {d}");
        }
        assert_eq!(row.iter().filter(|&&c| c).count() as u32, 2u32.pow(t.count_ones()), "Gould's sequence at row {t}");
        let mut next = vec![1u8; pascal.len() + 1];
        for k in 1..pascal.len() {
            next[k] = pascal[k - 1] ^ pascal[k];
        }
        pascal = next;
        s.step();
    }
}

#[test]
fn rule_30_centre_column_matches_oeis() {
    let n = A051023.len();
    let width = 2 * n + 3;
    let mut s = sim(30, width as u32, Boundary::Fixed0, EcaStart::SingleCell);
    let centre = width / 2;
    let mut column = String::new();
    for _ in 0..n {
        column.push(if s.current_row()[centre] { '1' } else { '0' });
        s.step();
    }
    assert_eq!(column, A051023);
}

#[test]
fn rule_184_conserves_cars_and_frees_traffic() {
    let mut s = sim(184, 1000, Boundary::Periodic, EcaStart::Random);
    let cars = s.metrics().get("ones").unwrap();
    for _ in 0..1000 {
        s.step();
        assert_eq!(s.metrics().get("ones"), Some(cars));
    }
    let row = s.current_row();
    let jammed = (0..row.len()).any(|i| row[i] && row[(i + 1) % row.len()]);
    assert!(!jammed, "at density 0.3 no two cars are adjacent after 1000 steps");
}

#[test]
fn rule_110_edges() {
    let width = 401;
    let mut s = sim(110, width, Boundary::Fixed0, EcaStart::SingleCell);
    let centre = (width / 2) as usize;
    for t in 1..150usize {
        s.step();
        let row = s.current_row();
        let first = row.iter().position(|&c| c).unwrap();
        let last = row.iter().rposition(|&c| c).unwrap();
        assert_eq!(last, centre, "the right edge stays fixed");
        assert_eq!(first, centre - t, "the left edge advances one cell per step");
    }
}

#[test]
fn mirror_and_complement_equivalents_of_rule_30() {
    // 86 = mirror(30), 135 = complement(30), 149 = mirror(complement(30)).
    let width = 257u32;
    let mut base = sim(30, width, Boundary::Periodic, EcaStart::Random);
    let mut mirror = sim(86, width, Boundary::Periodic, EcaStart::Random);
    let mut compl = sim(135, width, Boundary::Periodic, EcaStart::Random);
    let row = base.current_row();
    for x in 0..width as i64 {
        let c = row[x as usize];
        mirror.apply(WolframCommand::SetCell { x: width as i64 - 1 - x, alive: c }).unwrap();
        compl.apply(WolframCommand::SetCell { x, alive: !c }).unwrap();
    }
    for _ in 0..100 {
        base.step();
        mirror.step();
        compl.step();
        let (b, m, k) = (base.current_row(), mirror.current_row(), compl.current_row());
        for x in 0..width as usize {
            assert_eq!(m[width as usize - 1 - x], b[x]);
            assert_eq!(k[x], !b[x]);
        }
    }
}

#[test]
fn boundaries_and_contract() {
    let mut ones = sim(0, 8, Boundary::Fixed1, EcaStart::SingleCell);
    ones.set_param("rule", ParamValue::Int(254)).unwrap(); // any 1 in the neighbourhood -> 1
    ones.apply(WolframCommand::Clear).unwrap();
    ones.step();
    assert_eq!(
        ones.current_row(),
        [true, false, false, false, false, false, false, true],
        "fixed-1 ends feed the edges"
    );
    let mut s = sim(30, 65, Boundary::Fixed0, EcaStart::SingleCell);
    assert_eq!(s.descriptor().id, "ca.wolfram");
    assert!(s.descriptor().rules.contains("100->1"));
    assert!(s.apply(WolframCommand::SetCell { x: 65, alive: true }).is_err());
    s.advance(20);
    let snap = s.snapshot().unwrap();
    s.advance(10);
    let expected = s.current_row();
    let mut other = sim(90, 9, Boundary::Periodic, EcaStart::Random);
    other.restore(&snap).unwrap();
    other.advance(10);
    assert_eq!(other.current_row(), expected);
    // The view is the space-time diagram: y = time.
    let view = s.view(&GridViewRequest { x0: 0, y0: 0, x1: 64, y1: 30, max_width: 128, max_height: 128 });
    assert_eq!((view.width, view.height, view.cells_per_texel), (65, 31, 1));
    for t in 0..=30u64 {
        let row = s.row_at(t).unwrap();
        for (x, &c) in row.iter().enumerate() {
            let texel = view.texel_at_cell(x as i64, t as i64).unwrap();
            assert_eq!(texel[0], u8::from(c));
            if t > 0 {
                let changed = c != s.row_at(t - 1).unwrap()[x];
                assert_eq!(texel[2] == 255, changed, "B marks cells that changed in that step");
            }
        }
    }
    assert!(s.set_param("width", ParamValue::Int(2)).is_err());
    s.set_param("history", ParamValue::Int(10)).unwrap();
    s.reset();
    s.advance(30);
    assert_eq!(s.row_at(5), None, "only the last 10 rows are kept");
    assert!(s.row_at(25).is_some());
}

#[test]
fn accessors_describe_the_running_diagram() {
    let mut s = sim(30, 33, Boundary::Fixed0, EcaStart::SingleCell);
    s.set_param("history", ParamValue::Int(8)).unwrap();
    s.reset();
    s.set_param("boundary", ParamValue::Text("periodic".into())).unwrap();
    s.set_param("width", ParamValue::Int(65)).unwrap();
    assert_eq!((s.width(), s.boundary(), s.first_row()), (33, Boundary::Fixed0, 0), "changes are pending");
    s.advance(20);
    assert_eq!(s.boundary(), Boundary::Periodic, "the boundary applies from the next row");
    assert_eq!((s.first_row(), s.tick()), (13, 20), "the last 8 rows are kept");
    assert!(s.row_at(12).is_none() && s.row_at(13).is_some());
    s.reset();
    assert_eq!((s.width(), s.first_row()), (65, 0), "the width applies on reset");
}
