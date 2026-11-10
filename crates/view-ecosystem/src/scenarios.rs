//! Scenario cards: what to watch and the dynamics the model's documentation
//! reports for each built-in scenario (`docs/ECOSYSTEM.md` §18–19), shown
//! next to the scenario file's own description.

/// What to watch and what the documentation measured for a scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScenarioCard {
    /// Scenario id.
    pub id: &'static str,
    /// Metrics and behaviours to watch.
    pub watch: &'static str,
    /// Dynamics the documentation reports: measured runs (`measured`) or
    /// the behaviour the model's rules produce in this scenario.
    pub expected: &'static str,
    /// `expected` quotes measured runs of `docs/ECOSYSTEM.md` §19.
    pub measured: bool,
    /// Tab that shows the main story best.
    pub focus: crate::state::PropertiesTab,
}

use crate::state::PropertiesTab as Tab;

/// Cards of the 14 built-in scenarios.
pub const CARDS: [ScenarioCard; 14] = [
    ScenarioCard {
        id: "wolf_sheep_grass",
        watch: "population.sheep, population.wolf and green_patches: the classic predator–prey oscillation.",
        expected: "With the grass version both species persist in 8 of 8 seeds (3,000 ticks): sheep ≈ 160 \
                   (sd 20–31), wolves ≈ 76 (sd 12–16), ≈ 975 green patches, in noisy quasi-cycles. Without grass \
                   (parameter `grass` false, then reset) every seed collapses: in 5 of 8 the wolves eat the sheep \
                   out (ticks 142–175) and then starve; in 3 the wolves die out (ticks 205–270) and the sheep \
                   overrun the map (more than 30,000 by tick 374–417).",
        measured: true,
        focus: Tab::Populations,
    },
    ScenarioCard {
        id: "balanced_meadow",
        watch: "population.*, biomass_* per trophic level, energy.* flows and the nutrient pools; \
                nutrient_balance_error stays at rounding level.",
        expected: "Over two simulated years rabbits breed in spring and summer (from 150 to ≈ 4,200 in the second \
                   autumn) and then lose condition in a dry season (deaths by dehydration); foxes rise \
                   (from 8 to 59) as the main rabbit predators; the wolf pack's dominant pair breeds once a year; deer \
                   decline slowly (from 40 to 26); bees and ants follow the flowering season; trees colonise open \
                   grassland (from 4,330 to ≈ 8,700). Nutrient error grows by ≈ 3e-7 per day, energy error stays \
                   below 2e-6.",
        measured: true,
        focus: Tab::Populations,
    },
    ScenarioCard {
        id: "temperate_forest",
        watch: "biomass_* (five trophic levels), infected.rhd after the outbreak on day 20, fights, alarms, \
                hibernating.",
        expected: "Rabbit haemorrhagic disease is introduced on day 20 and spreads by contact through the \
                   rabbits (SIR: infected, recovered and immune, deaths); territory holders (foxes, wolves, \
                   eagles, ravens, warblers) chase and fight intruders of their kind, and prey that flee call \
                   alarms their group relays; bears, bats, frogs and snakes go dormant in the cold season.",
        measured: false,
        focus: Tab::Deaths,
    },
    ScenarioCard {
        id: "african_savanna",
        watch: "pack hunts of the lion pride, energy.scavenging (vultures at carcasses), flowers.pollination.",
        expected: "Lions hunt as a pride whose leader's target the members join; vultures and other \
                   scavengers find the carcasses; bees and ants forage; zebra herds and elephant families \
                   move together and call alarms.",
        measured: false,
        focus: Tab::FoodWeb,
    },
    ScenarioCard {
        id: "freshwater_lake",
        watch: "fish confined to the lake, herons fishing the shallows, frogs gathering and spawning in water.",
        expected: "Bream, perch and pike stay in fresh water of the lake; 400,000 sticklebacks are a density \
                   field gleaned by perch, pike, herons (in the shallows) and other piscivores; frogs gather \
                   at the water to mate and spawn.",
        measured: false,
        focus: Tab::FoodWeb,
    },
    ScenarioCard {
        id: "ocean_ecosystem",
        watch: "a marine food web in salt water only: algae, eaten by damselfish and sticklebacks, eaten by sharks and dolphins.",
        expected: "Manta rays graze algae, damselfish and the stickleback field feed sharks and a dolphin pod; \
                   lizards on the beach have no fresh water and live on the water in their prey.",
        measured: false,
        focus: Tab::FoodWeb,
    },
    ScenarioCard {
        id: "tropical_rainforest",
        watch: "fast decay (fungi and litter), insect density fields, ant pheromone trails (Ant trails overlay).",
        expected: "With 3,200 mm of rain, litter decomposes fast and the nutrient cycle turns quickly; twelve \
                   ant colonies lay pheromone trails to food; insects are abundant density fields.",
        measured: false,
        focus: Tab::Energy,
    },
    ScenarioCard {
        id: "isolated_island",
        watch: "small closed populations on a steep volcanic island (no shallows for waders).",
        expected: "An eagle pair hunts mallards and lizards; warblers, frogs and lizards live on the insect \
                   fields; with no immigration, chance births and deaths move the small populations.",
        measured: false,
        focus: Tab::Populations,
    },
    ScenarioCard {
        id: "forest_fire",
        watch: "burned_area, burned_biomass and the recovery of the herb layer (Fire risk overlay).",
        expected: "A 14-day drought from day 8 dries the fuel; three ignitions on day 22 spread with the \
                   westerly wind: herbs and shrubs fall from ≈ 1,370 t to ≈ 35 t within 20 days and recover to \
                   ≈ 184 t after 70 days from seed banks, resprouting shrubs and ash nutrients; 4,334 of 6,400 \
                   cells burned. Deer dash through the front to burned ground (12 of 30 survive); rabbits hide in \
                   burrows but starve afterwards on the burned ground.",
        measured: true,
        focus: Tab::Energy,
    },
    ScenarioCard {
        id: "prolonged_drought",
        watch: "drought_index, soil_moisture, plants and herbivores (compare with the scenario's own control run).",
        expected: "From day 10 a 50-day drought and 160 mm instead of 750 mm of annual rain: by day 100 soil \
                   moisture is 0.10 against 0.25, producers 1.9 against 2.6 thousand tonnes, 125 herbivores \
                   starved (none in the control) and herbivores are 20 % fewer, rabbits first.",
        measured: true,
        focus: Tab::Climate,
    },
    ScenarioCard {
        id: "invasive_species",
        watch: "rats against the native insectivores (lizards, frogs, warblers).",
        expected: "24 brown rats arrive on day 3 and reach the population cap of 5,000 within six months; they \
                   eat the ground beetles the natives live on and also lizards, frogs and eggs: after 180 days \
                   lizards are 18 %, frogs 37 % and warblers 17 % fewer than without rats.",
        measured: true,
        focus: Tab::Populations,
    },
    ScenarioCard {
        id: "apex_predator_extinction",
        watch: "herbivores, kills, starvation and herbs after all wolves are removed on day 15.",
        expected: "The pack of 12 kills 44 herbivores in 120 days — what twelve wolves need, about a tenth \
                   of the herbivore biomass. Without wolves herbivores are up to 6 % more numerous in the first \
                   three months and graze 1.4 % more; the cascade is weak because predation is bounded by the \
                   wolves' energy budget and grass regrowth compensates extra grazing.",
        measured: true,
        focus: Tab::Populations,
    },
    ScenarioCard {
        id: "island_evolution",
        watch: "genes (thermal_shift_c, size, speed…) of the lizards over years, in the inspector.",
        expected: "120 sand lizards at the cold edge of their niche on a cool wet island, with mutation rate \
                   0.3: offspring inherit a crossover of their parents' genes with mutation, so selection by \
                   exposure, starvation and predation can shift the expressed genes over generations.",
        measured: false,
        focus: Tab::Inspector,
    },
    ScenarioCard {
        id: "extreme_seasons",
        watch: "hibernating, migrating, exposure deaths across a ±25 °C year.",
        expected: "Bears, bats, frogs and snakes hibernate in the cold season (dormant animals pay a fraction \
                   of their metabolism and wake to drink); ducks, warblers and wapiti migrate towards their \
                   preferred climate; residents endure, and exposure deaths mark the extremes.",
        measured: false,
        focus: Tab::Climate,
    },
];

/// Card of a scenario.
pub fn card(id: &str) -> Option<&'static ScenarioCard> {
    CARDS.iter().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_scenario_has_a_card() {
        let scenarios = ecosystem::scenario::builtin().unwrap();
        assert_eq!(scenarios.len(), CARDS.len());
        for s in scenarios {
            let c = card(&s.id).unwrap_or_else(|| panic!("{} has no card", s.id));
            assert!(!c.watch.is_empty() && !c.expected.is_empty());
            // Measured cards are the scenarios of docs/ECOSYSTEM.md §19.
            let in_section_19 = matches!(
                c.id,
                "wolf_sheep_grass"
                    | "balanced_meadow"
                    | "forest_fire"
                    | "prolonged_drought"
                    | "invasive_species"
                    | "apex_predator_extinction"
            );
            assert_eq!(c.measured, in_section_19, "{}", c.id);
        }
    }
}
