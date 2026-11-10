//! Validation tests of species records.

use super::*;
use crate::species::catalog::parse_ron;

fn rabbit() -> Species {
    SpeciesCatalog::builtin().unwrap().animal("rabbit").unwrap().clone()
}

fn error_of(species: Species) -> String {
    species.validate().unwrap_err().to_string()
}

#[test]
fn builtin_species_are_valid() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    for s in catalog.animals() {
        s.validate().unwrap();
    }
}

#[test]
fn field_errors_name_species_and_field() {
    let mut s = rabbit();
    s.body.mass_kg = -1.0;
    let e = error_of(s);
    assert!(e.contains("rabbit") && e.contains("body.mass_kg"), "{e}");

    let mut s = rabbit();
    s.climate.tolerance_c = (10.0, 20.0);
    assert!(error_of(s).contains("tolerance range must contain"));

    let mut s = rabbit();
    s.reproduction.litter = (5, 2);
    assert!(error_of(s).contains("reproduction.litter"));

    let mut s = rabbit();
    s.life.maturity_days = 10_000.0;
    assert!(error_of(s).contains("maturity must come before"));

    let mut s = rabbit();
    s.id = "Rabbit!".into();
    assert!(error_of(s).contains("lowercase"));
}

#[test]
fn roles_must_match_diet_and_locomotion() {
    let mut s = rabbit();
    s.roles.push(Role::Predator);
    assert!(error_of(s).contains("Predator"));

    let mut s = rabbit();
    s.roles.push(Role::Solitary);
    assert!(error_of(s).contains("both social and solitary"));

    let mut s = rabbit();
    s.locomotion.retain(|l| *l != Locomotion::Burrow);
    assert!(error_of(s).contains("refuges"));

    let mut s = rabbit();
    s.habitat.aquatic = true;
    assert!(error_of(s).contains("swim"));

    let mut s = rabbit();
    s.diet.push(DietEntry { food: Food::Prey("rabbit".into()), preference: 0.5, assimilation: None });
    assert!(error_of(s).contains("itself"));
}

#[test]
fn hunters_of_individual_prey_need_a_hunting_style() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    let mut fox = catalog.animal("fox").unwrap().clone();
    fox.behaviour.hunting = HuntingStyle::None;
    let err = catalog.clone().insert_animal(fox).unwrap_err().to_string();
    assert!(err.contains("hunting style"), "{err}");
    // Insectivores that only glean density-field prey need none.
    let frog = catalog.animal("frog").unwrap();
    assert!(frog.hunts() && frog.behaviour.hunting == HuntingStyle::None);
    assert!(!catalog.hunts_animals(catalog.animal_index("frog").unwrap()));
    assert!(catalog.hunts_animals(catalog.animal_index("heron").unwrap()));
}

#[test]
fn representations_restrict_diets_and_prey() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    // A density field cannot eat carrion or individual animals.
    let mut beetle = catalog.animal("beetle").unwrap().clone();
    beetle.diet.push(DietEntry { food: Food::Carrion, preference: 0.5, assimilation: None });
    assert!(error_of(beetle).contains("cannot be eaten by a density-field species"));
    let mut spider = catalog.animal("spider").unwrap().clone();
    spider.diet.push(DietEntry { food: Food::Prey("rabbit".into()), preference: 0.5, assimilation: None });
    let err = catalog.clone().insert_animal(spider).unwrap_err().to_string();
    assert!(err.contains("only eat density-field prey"), "{err}");
    // Colonies are not prey.
    let mut fox = catalog.animal("fox").unwrap().clone();
    fox.diet.push(DietEntry { food: Food::Prey("bee".into()), preference: 0.5, assimilation: None });
    let err = catalog.clone().insert_animal(fox).unwrap_err().to_string();
    assert!(err.contains("colonies"), "{err}");
}

#[test]
fn seasonal_and_visual_sections_are_validated() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    let mut bear = catalog.animal("bear").unwrap().clone();
    if let Some(h) = bear.seasonal.hibernation.as_mut() {
        h.wake_above_c = h.below_c - 1.0;
    }
    assert!(error_of(bear).contains("wake_above_c"));
    let mut lion = catalog.animal("lion").unwrap().clone();
    lion.visual.procedural = None;
    assert!(error_of(lion).contains("procedural"));
    let lion = catalog.animal("lion").unwrap();
    assert_eq!(lion.visual.visual_id(), "proc:lion");
    assert_eq!(catalog.animal("zebra").unwrap().visual.visual_id(), "zebra");
}

#[test]
fn the_builtin_catalogue_covers_every_group() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    assert!(catalog.animals().len() >= 35, "{} species", catalog.animals().len());
    for id in [
        "rabbit",
        "deer",
        "stag",
        "sheep",
        "bison",
        "wolf",
        "fox",
        "bear",
        "lion",
        "zebra",
        "elephant",
        "boar",
        "rat",
        "bat",
        "horse",
        "alpaca",
        "eagle",
        "vulture",
        "duck",
        "songbird",
        "heron",
        "small_fish",
        "perch",
        "pike",
        "bream",
        "shark",
        "reef_fish",
        "manta",
        "dolphin",
        "frog",
        "snake",
        "turtle",
        "lizard",
        "ant",
        "bee",
        "butterfly",
        "spider",
        "beetle",
        "worm",
        "wasp",
    ] {
        let s = catalog.animal(id).unwrap_or_else(|| panic!("{id} is in the catalogue"));
        assert!(!s.visual.visual_id().is_empty(), "{id} has a visual id");
    }
    assert_eq!(catalog.field_species().len(), 6);
    assert_eq!(catalog.colony_species().len(), 2);
}

#[test]
fn unknown_fields_are_rejected_with_position() {
    let (_, text) = super::catalog::BUILTIN_ANIMALS.iter().find(|(name, _)| *name == "rabbit.ron").unwrap();
    let text = text.replacen("name:", "nmae:", 1);
    let err = parse_ron::<Species>(&text, "rabbit.ron").unwrap_err().to_string();
    assert!(err.contains("rabbit.ron") && err.contains("nmae"), "{err}");
}

#[test]
fn every_species_can_pay_for_its_mean_litter() {
    // A mother pays her litter (offspring tissue plus half a reserve each)
    // from the reserve above 20 % of its capacity and from what she eats
    // during gestation or egg formation. A species whose mean litter costs
    // more than that can never breed (as frogs and reef fish could not with
    // metamorphs of 5 % of the mother's mass formed in a day).
    let catalog = SpeciesCatalog::builtin().unwrap();
    let scale =
        crate::units::TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 20.0, motion_seconds: 5.0 };
    for s in catalog.animals().iter().filter(|s| s.is_individual()) {
        let t = crate::species::AnimalTraits::derive(s, &scale);
        let (e_off, _) = crate::animal::reproduction::offspring_cost(&t);
        let litter = 0.5 * f64::from(s.reproduction.litter.0 + s.reproduction.litter.1);
        let need = litter * e_off;
        let assimilation = s.diet.iter().map(|d| f64::from(d.assimilation())).fold(0.0, f64::max);
        let net_gain_per_tick =
            f64::from(t.intake_kj_per_tick) * assimilation - f64::from(t.field_kj_per_day) / f64::from(t.ticks_per_day);
        let available = 0.8 * f64::from(t.reserve_max_kj) + net_gain_per_tick.max(0.0) * t.gestation_ticks;
        assert!(
            need <= available,
            "{}: a mean litter costs {need:.1} kJ, the mother can gather {available:.1} kJ",
            s.id
        );
    }
}
