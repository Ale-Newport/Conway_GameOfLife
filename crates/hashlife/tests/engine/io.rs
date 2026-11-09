//! Macrocell round trips through evolved patterns.

use hashlife::{HashlifeError, Rule, Universe, macrocell};

use crate::common::{GLIDER, GOSPER_GUN, cells_of, soup};

#[test]
fn round_trip_of_an_evolving_gun() {
    let mut universe = Universe::conway();
    universe.insert_cells(GOSPER_GUN);
    for _ in 0..5 {
        universe.advance(997).unwrap();
        let text = macrocell::write(&universe);
        let back = macrocell::read(&text).unwrap();
        assert_eq!(back.generation(), universe.generation());
        assert_eq!(back.rule(), Rule::conway());
        assert_eq!(back.population(), universe.population());
        assert_eq!(cells_of(&back), cells_of(&universe));
        assert_eq!(macrocell::write(&back), text);
    }
}

#[test]
fn round_trip_keeps_evolution_identical() {
    let start = soup(3, -40, -40, 80, 80, 0.3);
    let mut universe = Universe::with_rule(Rule::parse("B3678/S34678").unwrap());
    universe.insert_cells(start.iter().copied());
    universe.advance(64).unwrap();
    let mut back = macrocell::read(&macrocell::write(&universe)).unwrap();
    universe.advance(100).unwrap();
    back.advance(100).unwrap();
    assert_eq!(cells_of(&back), cells_of(&universe));
}

#[test]
fn read_into_replaces_content_and_keeps_state_on_error() {
    let mut glider = Universe::conway();
    glider.insert_cells(GLIDER);
    glider.set_generation(12);
    let text = macrocell::write(&glider);
    let mut target = Universe::with_rule(Rule::highlife());
    target.insert_cells([(100, 100), (101, 100), (102, 100)]);
    macrocell::read_into(&mut target, &text).unwrap();
    assert_eq!(cells_of(&target), cells_of(&glider));
    assert_eq!(target.rule(), Rule::conway());
    assert_eq!(target.generation(), 12);
    let error = macrocell::read_into(&mut target, "[M2]\n*$\n4 0 0 0 9\n").unwrap_err();
    assert!(matches!(error, HashlifeError::Macrocell { line: 3, .. }));
    assert_eq!(cells_of(&target), cells_of(&glider), "a failed read leaves the universe unchanged");
}

#[test]
fn rule_errors_are_explained() {
    let error = macrocell::read("[M2] (golly 4.1)\n#R B3-jknr4ity5ijk6i8/S23-a4city6c7c\n").unwrap_err();
    assert!(error.to_string().contains("line 2"), "{error}");
    assert!(error.to_string().contains("totalistic"), "{error}");
    let error = macrocell::read("[M2]\n#R B0123/S\n").unwrap_err();
    assert!(error.to_string().contains("B0"), "{error}");
}
