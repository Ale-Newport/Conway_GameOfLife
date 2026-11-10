//! The mode's tutorial and concepts. Every statement is checked against
//! the model (`crates/ecosystem`, `docs/ECOSYSTEM.md`); numbers quoted are
//! the model's constants or the documentation's measured runs.

use interface::{Tutorial, TutorialPage};

use crate::palette::Overlay;
use crate::state::{CameraMode, PropertiesTab, Tool};

/// What a tutorial button asks the mode to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TutorialStep {
    /// Load a built-in scenario.
    Load(String),
    /// Show a Properties tab.
    Tab(PropertiesTab),
    /// Show an overlay.
    Overlay(Overlay),
    /// Start running.
    Play,
    /// Select an animal near the view centre and follow it.
    Follow,
    /// Pick a tool.
    Tool(Tool),
    /// Set the camera mode.
    Camera(CameraMode),
}

/// Parses a tutorial action id (`load:<id>`, `tab:<name>`, `overlay:<name>`,
/// `play`, `follow`, `tool:<name>`, `camera:<name>`); several steps are
/// separated by `;`.
pub fn parse_action(action: &str) -> Vec<TutorialStep> {
    action
        .split(';')
        .filter_map(|part| {
            let (verb, arg) = part.trim().split_once(':').unwrap_or((part.trim(), ""));
            Some(match verb {
                "load" => TutorialStep::Load(arg.to_string()),
                "tab" => TutorialStep::Tab(match arg {
                    "populations" => PropertiesTab::Populations,
                    "energy" => PropertiesTab::Energy,
                    "foodweb" => PropertiesTab::FoodWeb,
                    "deaths" => PropertiesTab::Deaths,
                    "climate" => PropertiesTab::Climate,
                    "inspector" => PropertiesTab::Inspector,
                    "species" => PropertiesTab::Species,
                    "scenario" => PropertiesTab::Scenario,
                    _ => return None,
                }),
                "overlay" => TutorialStep::Overlay(match arg {
                    "fire" => Overlay::FireRisk,
                    "disease" => Overlay::Disease,
                    "temperature" => Overlay::Temperature,
                    "moisture" => Overlay::Moisture,
                    "biomass" => Overlay::PlantBiomass,
                    "nutrients" => Overlay::Nutrients,
                    "territories" => Overlay::Territories,
                    "none" => Overlay::None,
                    _ => return None,
                }),
                "play" => TutorialStep::Play,
                "follow" => TutorialStep::Follow,
                "tool" => TutorialStep::Tool(match arg {
                    "ignite" => Tool::Ignite,
                    "spawn" => Tool::Spawn,
                    "cull" => Tool::Cull,
                    _ => return None,
                }),
                "camera" => TutorialStep::Camera(match arg {
                    "cinematic" => CameraMode::Cinematic,
                    "ground" => CameraMode::Ground,
                    "topdown" => CameraMode::TopDown,
                    "orbit" => CameraMode::Orbit,
                    _ => return None,
                }),
                _ => return None,
            })
        })
        .collect()
}

/// The tutorial.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "A living landscape",
                "Every vertebrate and large animal you see is an **individual** with its own body mass, energy \
                 reserve, hydration, fatigue, health, injuries, infections, genes and reproductive state. Each \
                 tick (6 simulated minutes) it perceives its surroundings and chooses what to do. Insects, \
                 spiders, worms and forage fish are **density fields** (individuals per m² in every cell), drawn \
                 as a few representative markers near the camera; ants and bees live in **colonies**. Plants grow per cell in seven \
                 layers (grass, shrubs, flowers, moss, algae, aquatic plants, desert plants); trees are \
                 individuals that grow, seed and die; fungi decompose litter and dead wood.\n\n\
                 Click an animal to open the **Inspector**; drag to orbit, right-drag to pan, scroll to zoom.",
            )
            .with_action("load:balanced_meadow;tab:scenario", "Open the balanced meadow"),
            TutorialPage::new(
                "Trophic levels",
                "The model sorts species by their roles: **primary consumers** (herbivores, nectarivores, \
                 detritivores), **secondary consumers** (omnivores, carnivores, predators and scavengers that \
                 are not apex) and **apex predators**. Plants are the **producers**. The metrics \
                 `biomass_producers`, `biomass_primary_consumers`, `biomass_secondary_consumers`, `biomass_apex` \
                 and `biomass_decomposers` weigh each level. The **Food web** tab draws who eats whom from the \
                 species' diets, with the kills and the grazing the run has observed.",
            )
            .with_action("tab:foodweb", "Show the food web"),
            TutorialPage::new(
                "Energy flow and the 10 % rule",
                "Energy enters as **primary production** (plant growth, 18 kJ per gram of dry matter) and \
                 leaves as **respiration**, **combustion** and **humification**. An eater assimilates a fraction \
                 of its food — by default prey 0.8, carrion 0.75, nectar 0.9, grass 0.5, shrubs 0.4, browse \
                 0.35, detritus 0.3 (a species file may set its own) — and the rest becomes dung; most of what it assimilates is respired by its \
                 metabolism (Kleiber / metabolic theory, activity, locomotion, thermoregulation).\n\n\
                 There is **no 10 % constant in the code**: the efficiency of each step emerges from these \
                 rules. The **Energy** tab divides the run's cumulative flows (herbivory by primary \
                 production, meat eaten by herbivory) so you can see what this ecosystem actually transfers.",
            )
            .with_action("tab:energy", "Show the energy budget"),
            TutorialPage::new(
                "Nutrients go round",
                "Nutrients are one mineral pool (standing for nitrogen and phosphorus) counted in grams. Plants \
                 take them from the soil; animal tissue holds 30 g/kg; dung, urine, carcasses and litter return \
                 them; decomposer fungi **mineralise** them back into the soil. Deposition, leaching and fire \
                 volatilisation are declared per scenario; without them (and without interventions) no nutrient \
                 enters or leaves, and the total stays constant.\n\n\
                 Every transfer is booked in two ledgers (energy and nutrients) and the model reports how far \
                 its pools drift from its books: `nutrient_balance_error` and `energy_balance_error`, shown \
                 honestly on the **Energy** tab.",
            )
            .with_action("tab:energy;overlay:nutrients", "Show the nutrient pools"),
            TutorialPage::new(
                "Predator–prey oscillations",
                "The *Wolves, sheep and grass* scenario re-implements the rules of NetLogo's Wolf Sheep \
                 Predation: sheep eat green patches that regrow after 30 ticks, wolves eat sheep on their patch, \
                 both reproduce with fixed probabilities. Measured over eight seeds, both species persist with \
                 sheep ≈ 160 and wolves ≈ 76 in noisy cycles. Switch the `grass` parameter off and reset: \
                 without the grass feedback the system collapses in every seed.",
            )
            .with_action("load:wolf_sheep_grass;tab:populations;play", "Run wolves and sheep"),
            TutorialPage::new(
                "Fire and succession",
                "Fire is part of the physics world: a burning cell ignites a neighbour with a probability \
                 that grows with wind, slope and fuel and falls with fuel moisture. Trees in a burning cell die \
                 with `1 − exp(−2 I (1 − bark) / (1 + H/10))`; resprouting species lose only their stem. \
                 Herbs come back from the seed bank and ash nutrients. In the *Forest fire* scenario a drought \
                 starts on day 8 and three ignitions follow on day 22; the **Fire risk** overlay shows where \
                 flames would spread now. Try the **Ignite** tool on dry ground.",
            )
            .with_action("load:forest_fire;overlay:fire;tool:ignite", "Open the forest fire"),
            TutorialPage::new(
                "Disease",
                "Pathogens spread by contact (SIR): a susceptible host with `k` infectious hosts within its \
                 contact distance is infected with `P = 1 − (1 − β)^{k Δt}`; infected hosts recover (then stay \
                 immune for a while), or die; while sick they are slower and burn more energy. In the \
                 *Temperate forest*, rabbit haemorrhagic disease arrives on day 20. The **Disease prevalence** \
                 overlay shows where it is; the **Deaths & disease** tab counts infections, recoveries and \
                 deaths.",
            )
            .with_action("load:temperate_forest;overlay:disease;tab:deaths", "Open the temperate forest"),
            TutorialPage::new(
                "Nothing is scripted",
                "No population curve, cycle or cascade is written into the model. Each animal scores its \
                 options (forage, hunt, flee, drink, rest, sleep, mate, defend, migrate, hibernate…) from its \
                 own state and what it perceives, and the outcome of every meal, chase, fight and birth is \
                 resolved by the rules. Scenarios only schedule interventions — a drought, an ignition, an \
                 introduction, a removal, an outbreak. Follow an animal: the Inspector shows its utility scores \
                 and recent decisions as they happen.",
            )
            .with_action("follow;tab:inspector;camera:cinematic", "Follow an animal"),
        ],
        concepts: CONCEPTS.into(),
    }
}

const CONCEPTS: &str = "## Time and space\n\
One tick is 6 simulated minutes (240 ticks per day); a year has four seasons of 30 days. Life-history \
durations (lifespan, maturity, gestation) are compressed by `days per year / 365`, physiological rates are per \
simulated day. Cells are 20 m in every built-in scenario; one tick of movement stands for 5 s of locomotion (`motion_seconds`).\n\n\
## Representations\n\
* **Individuals** — vertebrates and large animals, each with its own state and decisions.\n\
* **Density fields** — insects, spiders, worms and forage fish as individuals per m² in every cell (feeding, \
births, deaths, dispersal); the view draws representative markers near the camera.\n\
* **Colonies** — ants and bees as super-agents with workers, brood, stores and forager squads.\n\n\
## Energy and matter\n\
Plant, litter and fungal dry matter carry 18 kJ/g, animal tissue 7 kJ/g. Every transfer is a *flux* booked \
as an input, output or internal transfer; the stock must equal the initial stock plus inputs minus outputs.\n\n\
## Metabolism\n\
Basal metabolism follows metabolic theory, `B = 0.78 c M^{3/4} e^{−E/kT}` W (c = 1 for endotherms, 0.16 for \
ectotherms); field metabolism multiplies it; reserves last `fasting_days`; locomotion costs `10.7 M^{0.68}` J \
per metre (× 0.35 in flight, × 0.12 swimming).\n\n\
## Death\n\
Animals die of predation, starvation, dehydration, old age (Gompertz hazard: 1 % reach the lifespan), fire, \
drowning, exposure, injuries, disease, or a cull. Dead animals become carcasses that scavengers eat and that \
decay into litter.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_action_parses() {
        let t = tutorial();
        assert!(t.pages.len() >= 8);
        for page in &t.pages {
            let Some(action) = &page.action else { continue };
            let steps = parse_action(&action.id);
            assert_eq!(steps.len(), action.id.split(';').count(), "{}", action.id);
        }
        assert_eq!(parse_action("load:forest_fire;tab:energy").len(), 2);
        assert!(parse_action("dance").is_empty());
    }

    #[test]
    fn tutorial_scenarios_exist() {
        for page in tutorial().pages {
            for step in page.action.map(|a| parse_action(&a.id)).unwrap_or_default() {
                if let TutorialStep::Load(id) = step {
                    assert!(ecosystem::scenario::find_builtin(&id).is_ok(), "{id}");
                }
            }
        }
    }
}
