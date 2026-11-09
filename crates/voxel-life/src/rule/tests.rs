use super::*;

fn rule(text: &str) -> Rule3D {
    match Rule3D::parse(text) {
        Ok(rule) => rule,
        Err(e) => panic!("`{text}` should parse: {e}"),
    }
}

fn set(counts: &[u32]) -> CountSet {
    counts.iter().copied().collect()
}

/// Every Softology rule of the research catalogue (§7.4) except the
/// "Expanding Shell" string, which is printed with a typo in the source.
const SOFTOLOGY: &[(&str, &str)] = &[
    ("3D Brain", "/4/2/M"),
    ("445", "4/4/5/M"),
    ("Amoeba", "9-26/5-7,12-13,15/5/M"),
    ("Architecture", "4-6/3/2/M"),
    ("Builder 1", "2,6,9/4,6,8-9/10/M"),
    ("Builder 2", "5-7/1/2/M"),
    ("Clouds 1", "13-26/13-14,17-19/2/M"),
    ("Clouds 2", "12-26/13-14/2/M"),
    ("Construction", "0-2,4,6-11,13-17,21-26/9-10,16,23-24/2/M"),
    ("Coral", "5-8/6-7,9,12/4/M"),
    ("Crystal Growth 1", "0-6/1,3/2/N"),
    ("Crystal Growth 2", "1-2/1,3/5/N"),
    ("Diamond Growth", "5-6/1-3/7/N"),
    ("More Structures", "7-26/4/4/M"),
    ("Pulse Waves", "3/1-3/10/M"),
    ("Pyroclastic", "4-7/6-8/10/M"),
    ("Sample 1", "10-26/5,8-26/4/M"),
    ("Shells", "3,5,7,9,11,15,17,19,21,23-24,26/3,6,8-9,11,14-17,19,24/7/M"),
    ("Single Point Replication", "/1/2/M"),
    ("Slow Decay 1", "13-26/10-26/3/M"),
    ("Slow Decay 2", "1,4,8,11,13-26/13-26/5/M"),
    ("Spiky Growth", "0-3,7-9,11-13,18,21-22,24,26/13,17,20-26/4/M"),
    ("Stable Structures", "13-26/14-19/2/M"),
    ("Symmetry", "/2/10/M"),
    ("von Neumann Builder", "1-3/1,4-5/5/N"),
];

#[test]
fn softology_catalogue_round_trips_exactly() {
    for (name, text) in SOFTOLOGY {
        let r = rule(text);
        assert_eq!(r.to_softology(), *text, "{name} must print back unchanged");
        assert_eq!(rule(&r.to_softology()), r, "{name}");
        assert_eq!(rule(&r.to_letters()), r, "{name}: letter form {}", r.to_letters());
        if let Some(golly) = r.to_golly() {
            assert_eq!(rule(&golly), r, "{name}: Golly form {golly}");
        }
    }
}

#[test]
fn softology_fields_are_decoded() {
    let clouds = rule("13-26/13-14,17-19/2/M");
    assert_eq!(clouds.survive(), CountSet::range(13, 26));
    assert_eq!(clouds.birth(), set(&[13, 14, 17, 18, 19]));
    assert_eq!(clouds.states(), 2);
    assert_eq!(clouds.neighbourhood(), Neighbourhood::Moore);

    let builder = rule("2,6,9/4,6,8-9/10/M");
    assert_eq!(builder.survive(), set(&[2, 6, 9]));
    assert_eq!(builder.birth(), set(&[4, 6, 8, 9]));
    assert_eq!(builder.states(), 10);

    let crystal = rule("0-6/1,3/2/N");
    assert_eq!(crystal.survive(), CountSet::range(0, 6));
    assert_eq!(crystal.neighbourhood(), Neighbourhood::VonNeumann);

    let brain = rule("/4/2/m");
    assert!(brain.survive().is_empty());
    assert_eq!(brain.birth(), set(&[4]));
}

#[test]
fn bays_and_golly_and_letters_agree() {
    // Research checklist item 1: 3D5..7/6 = Bays 5766 = Softology 5-7/6/2/M.
    let equivalent_5766 = ["5766", "(5 7 6 6)", "3D5..7/6", "3d5,6,7/6", "5-7/6/2/M", "B6/S567", "S567/B6", "b6/s5-7"];
    for text in equivalent_5766 {
        assert_eq!(rule(text), Rule3D::bays_5766(), "{text}");
    }
    let equivalent_4555 = ["4555", "3D4,5/5", "4-5/5/2/M", "B5/S45", "B5/S45/C2"];
    for text in equivalent_4555 {
        assert_eq!(rule(text), Rule3D::bays_4555(), "{text}");
    }
    assert_eq!(rule("(10 21 10 21)"), rule("10-21/10-21/2/M"));
    assert_eq!(rule("3D5,6/5"), rule("5655"));
    assert_eq!(rule("3D6..8/5"), rule("6855"));
    assert_eq!(rule("3D4,7/5,8").survive(), set(&[4, 7]));
    assert_eq!(rule("3D0..6/1,3F"), rule("0-6/1,3/2/N"));
    assert_eq!(rule("3D1/1V"), rule("1/1/2/N"));
    assert_eq!(rule("B13-14,17-19/S13-26"), rule("13-26/13-14,17-19/2/M"));
    assert_eq!(rule("B4/S/C2"), rule("/4/2/M"));
    assert_eq!(rule("B1,3/S0-6/N"), rule("0-6/1,3/2/N"));
    assert_eq!(rule("S4/B4/G5"), rule("4/4/5/M"));
    assert_eq!(rule("B4/S4/5"), rule("4/4/5/M"), "bare state count, as Golly's B2/S/3");
    assert_eq!(rule("B13/S12/5/N"), rule("1-2/1,3/5/N"));
}

/// Random rules (every count set, state count and neighbourhood) printed in
/// every notation that can express them parse back to the same rule.
#[test]
fn random_rules_round_trip_through_every_notation() {
    let mut rng = simulation_core::SimRng::seed_from_u64(2024);
    for _ in 0..20_000 {
        let neighbourhood = if rng.chance(0.5) { Neighbourhood::Moore } else { Neighbourhood::VonNeumann };
        let size = neighbourhood.size();
        let density = rng.f64();
        let survive: CountSet = (0..=size).filter(|_| rng.chance(density)).collect();
        let birth: CountSet = (1..=size).filter(|_| rng.chance(density)).collect();
        let states = if rng.chance(0.5) { 2 } else { 2 + rng.below(255) as u16 };
        let Ok(r) = Rule3D::new(survive, birth, states, neighbourhood) else {
            panic!("generated rule must be valid");
        };
        let forms = [Some(r.to_softology()), Some(r.to_letters()), r.to_golly(), r.to_bays()];
        for form in forms.into_iter().flatten() {
            assert_eq!(Rule3D::parse(&form).as_ref(), Ok(&r), "`{form}`");
        }
    }
    // Two-state Moore interval rules always have a Bays form.
    for (el, eu, fl, fu) in [(0, 26, 1, 26), (4, 5, 5, 5), (9, 12, 10, 21)] {
        let r = Rule3D::new(CountSet::range(el, eu), CountSet::range(fl, fu), 2, Neighbourhood::Moore);
        let bays = r.as_ref().ok().and_then(Rule3D::to_bays);
        assert_eq!(bays.as_deref().map(Rule3D::parse), Some(r), "({el} {eu} {fl} {fu})");
    }
}

#[test]
fn canonical_printing() {
    let r5766 = Rule3D::bays_5766();
    assert_eq!(r5766.to_string(), "5-7/6/2/M");
    assert_eq!(r5766.to_letters(), "B6/S567");
    assert_eq!(r5766.to_bays().as_deref(), Some("5766"));
    assert_eq!(r5766.to_golly().as_deref(), Some("3D5..7/6"));
    let r4555 = Rule3D::bays_4555();
    assert_eq!(r4555.to_golly().as_deref(), Some("3D4,5/5"));
    assert_eq!(r4555.to_bays().as_deref(), Some("4555"));
    assert_eq!(rule("10-21/10-21/2/M").to_bays().as_deref(), Some("(10 21 10 21)"));
    // Bays notation only exists for single intervals, two states, Moore.
    assert_eq!(rule("4,7/5,8/2/M").to_bays(), None);
    assert_eq!(rule("4/4/5/M").to_bays(), None);
    assert_eq!(rule("4/4/2/N").to_bays(), None);
    // Golly has no multi-state 3D rules.
    assert_eq!(rule("4/4/5/M").to_golly(), None);
    assert_eq!(rule("0-6/1,3/2/N").to_golly().as_deref(), Some("3D0..6/1,3F"));
    // Letter form with two-digit counts and the lone-count escape.
    assert_eq!(rule("13-26/13/2/M").to_letters(), "B13-13/S13-26");
    assert_eq!(rule("B13-13/S13-26"), rule("13-26/13/2/M"));
    assert_eq!(rule("4/4/5/M").to_letters(), "B4/S4/C5");
    assert_eq!(rule("1-2/1,3/5/N").to_letters(), "B13/S12/C5/N");
}

#[test]
fn letter_lists_are_read_digit_by_digit_unless_itemised() {
    assert_eq!(rule("B13/S12").birth(), set(&[1, 3]));
    assert_eq!(rule("B13,14/S12").birth(), set(&[13, 14]));
    assert_eq!(rule("B5..7/S4").birth(), set(&[5, 6, 7]));
}

#[test]
fn errors_are_specific() {
    let err = |text: &str| match Rule3D::parse(text) {
        Ok(r) => panic!("`{text}` should fail but gave {r}"),
        Err(e) => e,
    };
    assert_eq!(err("   "), RuleError::Empty);
    assert!(matches!(err("23/3"), RuleError::UnknownNotation(_)));
    assert!(matches!(err("Life"), RuleError::UnknownNotation(_)));
    assert!(matches!(err("4/4/5/X"), RuleError::Neighbourhood(s) if s == "X"));
    assert!(matches!(err("4/4/1/M"), RuleError::States(s) if s == "1"));
    assert!(matches!(err("4/4/257/M"), RuleError::States(_)));
    assert!(matches!(err("4/4/five/M"), RuleError::States(_)));
    assert!(matches!(err("4/27/2/M"), RuleError::CountOutOfRange { count: 27, size: 26, .. }));
    assert!(matches!(err("7/1/2/N"), RuleError::CountOutOfRange { count: 7, size: 6, field: "survival list", .. }));
    assert_eq!(err("4/0-3/2/M"), RuleError::BirthOnZero);
    assert_eq!(err("B0/S1"), RuleError::BirthOnZero);
    assert!(matches!(err("7-5/1/2/M"), RuleError::Field { reason, .. } if reason.contains("reversed")));
    assert!(matches!(err("4,,5/1/2/M"), RuleError::Field { reason, .. } if reason.contains("empty item")));
    assert!(matches!(err("4/x/2/M"), RuleError::Field { field: "birth list", .. }));
    assert!(matches!(err("576"), RuleError::Field { field: "Bays rule", .. }));
    assert!(matches!(err("7566"), RuleError::Field { reason, .. } if reason.contains("survival interval")));
    assert!(matches!(err("5776"), RuleError::Field { reason, .. } if reason.contains("birth interval")));
    assert!(matches!(err("(10 21 10 30)"), RuleError::CountOutOfRange { count: 30, .. }));
    assert!(matches!(err("3D3/3H"), RuleError::Field { reason, .. } if reason.contains("hexahedral")));
    assert!(matches!(err("3DBB"), RuleError::Field { reason, .. } if reason.contains("Busy Boxes")));
    assert!(matches!(err("3D5,6,7"), RuleError::Field { .. }));
    assert!(matches!(err("B3/S2/S3"), RuleError::Field { reason, .. } if reason.contains("twice")));
    assert!(matches!(err("S23"), RuleError::UnknownNotation(_)));
    assert!(matches!(err("B3/X2"), RuleError::Field { .. }));
    assert!(matches!(err("B3"), RuleError::UnknownNotation(_)));
    assert!(matches!(err("B3/C4"), RuleError::Field { field: "survival list", .. }));
    // Messages name the offending part.
    assert!(err("7/1/2/N").to_string().contains("6-voxel von Neumann"));
}

#[test]
fn next_state_follows_generations_semantics() {
    let r = rule("4/4/5/M");
    assert_eq!(r.next_state(0, 4), 1, "birth at 4");
    assert_eq!(r.next_state(0, 3), 0);
    assert_eq!(r.next_state(1, 4), 1, "survival at 4");
    assert_eq!(r.next_state(1, 5), 2, "a dying cell enters state 2");
    for k in 2..4u8 {
        for n in [0, 4, 26] {
            assert_eq!(r.next_state(k, n), k + 1, "dying states advance whatever the neighbours");
        }
    }
    assert_eq!(r.next_state(4, 4), 0, "the last state returns to empty");
    let two_state = Rule3D::bays_5766();
    assert_eq!(two_state.next_state(1, 4), 0, "two-state rules die directly");
    let table = r.transition();
    for state in 0..5u8 {
        for n in 0..=26u8 {
            assert_eq!(table.apply(state, n), r.next_state(state, u32::from(n)));
        }
    }
}

#[test]
fn neighbourhood_offsets() {
    let moore = Neighbourhood::Moore.offsets();
    assert_eq!(moore.len(), 26);
    assert!(moore.iter().all(|o| o.iter().all(|c| c.abs() <= 1) && *o != [0, 0, 0]));
    let mut sorted = moore.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 26);
    let von_neumann = Neighbourhood::VonNeumann.offsets();
    assert_eq!(von_neumann.len(), 6);
    assert!(von_neumann.iter().all(|o| o.iter().map(|c| c.abs()).sum::<i32>() == 1));
}

#[test]
fn serde_uses_the_canonical_string() {
    let r = rule("B6/S567");
    let text = ron::to_string(&r).unwrap_or_default();
    assert_eq!(text, "\"5-7/6/2/M\"");
    let back: Result<Rule3D, _> = ron::from_str("\"3D4,5/5\"");
    assert_eq!(back.ok(), Some(Rule3D::bays_4555()));
    let bad: Result<Rule3D, _> = ron::from_str("\"4/27/2/M\"");
    assert!(bad.is_err());
}
