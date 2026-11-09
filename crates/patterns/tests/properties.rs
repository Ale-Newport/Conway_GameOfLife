//! Randomised cross-checks (deterministic: every generator is a seeded
//! `SimRng`).
//!
//! * Rule strings written in every supported notation, with shuffled digits,
//!   parse back to the masks they were generated from.
//! * Every writer's output is read back to the same cells.
//! * Rotations and reflections agree with an independent dense-grid reference.
//! * The readers return errors, never panic, on hostile input, including
//!   numbers at the limits of `i64`/`u64`.

use patterns::{BoundedGrid, Cell, Format, GenerationsRule, LifeRule, Pattern, Rule, life105, life106, parse_any};
use patterns::{plaintext, rle};
use simulation_core::SimRng;

/// The neighbour counts of `mask` as digits, in a random order.
fn shuffled_digits(rng: &mut SimRng, mask: u16) -> String {
    let mut digits: Vec<char> = (0..=8u8).filter(|n| mask & (1 << n) != 0).map(|n| char::from(b'0' + n)).collect();
    for i in (1..digits.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        digits.swap(i, j);
    }
    digits.into_iter().collect()
}

#[test]
fn rule_notations_agree_with_their_masks() {
    let mut rng = SimRng::seed_from_u64(11);
    for _ in 0..5_000 {
        let birth = rng.below(512) as u16;
        let survive = rng.below(512) as u16;
        let (b, s) = (shuffled_digits(&mut rng, birth), shuffled_digits(&mut rng, survive));
        for text in [
            format!("B{b}/S{s}"),
            format!("b{b}s{s}"),
            format!("S{s}/B{b}"),
            format!("{s}/{b}"),
            format!(" B {b} / S {s} "),
        ] {
            let rule = LifeRule::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!((rule.birth, rule.survive), (birth, survive), "{text}");
            assert_eq!(LifeRule::parse(&rule.to_string()), Ok(rule), "{text}");
            assert_eq!(Rule::parse(&text), Ok(Rule::Life(rule)), "{text}");
            for n in 0..=8u8 {
                assert_eq!(rule.next_state(false, n), birth & (1 << n) != 0);
                assert_eq!(rule.next_state(true, n), survive & (1 << n) != 0);
            }
        }

        // Generations rules cannot have B0.
        let birth = birth & !1;
        let b = shuffled_digits(&mut rng, birth);
        let states = 2 + rng.below(254) as u8;
        for text in [format!("{s}/{b}/{states}"), format!("B{b}/S{s}/C{states}"), format!("g{states}b{b}s{s}")] {
            let rule = GenerationsRule::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!((rule.birth, rule.survive, rule.states), (birth, survive, states), "{text}");
            assert_eq!(GenerationsRule::parse(&rule.to_string()), Ok(rule), "{text}");
            assert_eq!(GenerationsRule::parse(&rule.to_bsc_string()), Ok(rule), "{text}");
        }
    }
}

fn random_pattern(rng: &mut SimRng, max_state: u8, span: u64) -> Pattern {
    let count = rng.below(60);
    let half = (span / 2) as i64;
    Pattern::new(
        (0..count)
            .map(|_| {
                let x = rng.below(span) as i64 - half;
                let y = rng.below(span) as i64 - half;
                Cell::new(x, y, 1 + rng.below(u64::from(max_state)) as u8)
            })
            .collect(),
    )
}

#[test]
fn writers_round_trip_random_patterns() {
    let mut rng = SimRng::seed_from_u64(3);
    for i in 0..2_000 {
        let two_state = i % 2 == 0;
        let pattern = random_pattern(&mut rng, if two_state { 1 } else { 255 }, 300);
        // `normalized` keeps the later of two cells at one position, like the writers.
        let expected = pattern.normalized().cells;
        let canonical = |p: &Pattern| p.normalized().cells;

        let text = rle::write(&pattern);
        let back = rle::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(canonical(&back), expected, "{text}");
        assert_eq!(back.bounding_box(), pattern.bounding_box(), "RLE keeps the position");
        if !two_state || pattern.is_empty() {
            continue;
        }
        assert_eq!(life106::parse(&life106::write(&pattern).unwrap()).unwrap().bounding_box(), pattern.bounding_box());
        assert_eq!(life105::parse(&life105::write(&pattern).unwrap()).unwrap().bounding_box(), pattern.bounding_box());
        for format in Format::ALL {
            let text = format.write(&pattern).unwrap();
            let file_name = format!("pattern.{}", format.extension());
            let back = parse_any(&text, Some(&file_name)).unwrap_or_else(|e| panic!("{format}: {e}\n{text}"));
            assert_eq!(canonical(&back), expected, "{format}\n{text}");
        }
        assert_eq!(canonical(&plaintext::parse(&plaintext::write(&pattern).unwrap()).unwrap()), expected);
    }
}

/// Names, comments and rules are free text: whatever they contain, every
/// writer's output must parse again and give back the same cells.
#[test]
fn writers_survive_arbitrary_metadata() {
    let mut rng = SimRng::seed_from_u64(13);
    let alphabet: Vec<char> = "ab o$!#.*O0123456789:=,/\r\n\t xyNCR".chars().collect();
    let rules = ["", " ", "B3/S23", "b36s23:t10", "LifeHistory", "Life History", "/2/3", "B3/S23:P20,10", "x\ny", "#"];
    let text = |rng: &mut SimRng| -> String {
        let len = rng.below(12) as usize;
        (0..len).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect()
    };
    for _ in 0..3_000 {
        let mut pattern = random_pattern(&mut rng, 1, 40);
        pattern.name = text(&mut rng);
        pattern.author = text(&mut rng);
        pattern.description = text(&mut rng);
        pattern.comments = (0..rng.below(3)).map(|_| text(&mut rng)).collect();
        pattern.rule = match rng.below(rules.len() as u64 + 2) as usize {
            i if i < rules.len() => Some(rules[i].to_string()),
            i if i == rules.len() => Some(text(&mut rng)),
            _ => None,
        };
        let expected = pattern.normalized().cells;
        let back = rle::parse(&rle::write(&pattern)).unwrap_or_else(|e| panic!("RLE {pattern:?}: {e}"));
        assert_eq!(back.normalized().cells, expected, "RLE {pattern:?}");
        assert_eq!(back.bounding_box(), pattern.bounding_box(), "RLE {pattern:?}");
        for format in [Format::Plaintext, Format::Life105, Format::Life106] {
            let written = format.write(&pattern).unwrap();
            let back = format.parse(&written).unwrap_or_else(|e| panic!("{format} {pattern:?}\n{written}: {e}"));
            assert_eq!(back.normalized().cells, expected, "{format} {pattern:?}\n{written}");
        }
    }
}

/// Dense reference: `grid[row][column]` of the bounding box.
fn dense(pattern: &Pattern) -> Vec<Vec<u8>> {
    let grid = pattern.to_grid().unwrap();
    (0..grid.height()).map(|y| grid.row(y).to_vec()).collect()
}

#[test]
fn transforms_match_a_dense_reference() {
    let mut rng = SimRng::seed_from_u64(5);
    for _ in 0..2_000 {
        let pattern = random_pattern(&mut rng, 4, 24).normalized().translated(rng.below(100) as i64 - 50, 7);
        let Some((x0, y0, ..)) = pattern.bounding_box() else { continue };
        let g = dense(&pattern);
        let (h, w) = (g.len(), g[0].len());
        let build = |rows: usize, cols: usize, f: &dyn Fn(usize, usize) -> u8| -> Vec<Vec<u8>> {
            (0..rows).map(|r| (0..cols).map(|c| f(r, c)).collect()).collect()
        };
        let cases = [
            ("rotate_cw", pattern.rotate_cw(), build(w, h, &|r, c| g[h - 1 - c][r])),
            ("rotate_ccw", pattern.rotate_ccw(), build(w, h, &|r, c| g[c][w - 1 - r])),
            ("rotate_180", pattern.rotate_180(), build(h, w, &|r, c| g[h - 1 - r][w - 1 - c])),
            ("flip_horizontal", pattern.flip_horizontal(), build(h, w, &|r, c| g[r][w - 1 - c])),
            ("flip_vertical", pattern.flip_vertical(), build(h, w, &|r, c| g[h - 1 - r][c])),
        ];
        for (name, transformed, expected) in cases {
            assert_eq!(dense(&transformed), expected, "{name}\n{}", pattern.to_ascii());
            let (tx0, ty0, ..) = transformed.bounding_box().unwrap();
            assert_eq!((tx0, ty0), (x0, y0), "{name} keeps the top-left corner");
            assert_eq!(transformed.population(), pattern.population(), "{name}");
        }
    }
}

#[test]
fn readers_never_panic_on_random_input() {
    let mut rng = SimRng::seed_from_u64(7);
    let alphabet: Vec<char> = "bo.$!0123456789 \n\r\t#xy=,ABCXpqyNCOrRP-+:/TKSsBc*éLife1.056[]M".chars().collect();
    for i in 0..50_000 {
        let len = 1 + rng.below(40) as usize;
        let text: String = (0..len).map(|_| alphabet[rng.below(alphabet.len() as u64) as usize]).collect();
        let file_name = ["a.rle", "a.lif", "a.cells", "a"][i % 4];
        if let Ok(pattern) = parse_any(&text, Some(file_name)) {
            assert_eq!(rle::parse(&rle::write(&pattern)).unwrap().normalized().cells, pattern.normalized().cells);
        }
        let _ = (rle::parse(&text), plaintext::parse(&text), life105::parse(&text), life106::parse(&text));
        let _ = (Rule::parse(&text), BoundedGrid::parse(&text), LifeRule::parse(&text), GenerationsRule::parse(&text));
    }
}

#[test]
fn extreme_numbers_are_errors_or_round_trip() {
    const NUMBERS: [&str; 14] = [
        "0",
        "1",
        "2",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615",
        "18446744073709551616",
        "2000000000",
        "2000000001",
        "-1",
        "+5",
        "99999999999999999999999",
    ];
    let mut rng = SimRng::seed_from_u64(9);
    let mut pick = || NUMBERS[rng.below(NUMBERS.len() as u64) as usize];
    for _ in 0..20_000 {
        let sources = [
            format!(
                "#CXRLE Pos={},{} Gen={}\nx = {}, y = {}, rule = B3/S23:T{},{}\n{}o{}b{}$o!",
                pick(),
                pick(),
                pick(),
                pick(),
                pick(),
                pick(),
                pick(),
                pick(),
                pick(),
                pick()
            ),
            format!("#P {} {}\n{}o{}$2o!", pick(), pick(), pick(), pick()),
            format!("#Life 1.05\n#P {} {}\n.*\n**", pick(), pick()),
            format!("#Life 1.06\n{} {}\n{} {}", pick(), pick(), pick(), pick()),
        ];
        for source in &sources {
            if let Ok(pattern) = parse_any(source, None) {
                let text = rle::write(&pattern);
                let back = rle::parse(&text).unwrap_or_else(|e| panic!("{source:?} -> {text:?}: {e}"));
                // The readers return row-major cells without duplicates, as the RLE reader does.
                assert_eq!(back.cells, pattern.cells, "{source:?}");
            }
        }
        let suffix = format!(":T{}{}{},{}", pick(), ["", "+", "-"][pick().len() % 3], pick(), pick());
        if let Ok(Some(grid)) = BoundedGrid::parse(&suffix) {
            assert_eq!(BoundedGrid::parse(&grid.to_string()), Ok(Some(grid)), "{suffix}");
        }
    }
}
