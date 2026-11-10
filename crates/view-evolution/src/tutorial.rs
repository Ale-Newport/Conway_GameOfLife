//! Tutorial of the Evolution Laboratory and the actions its steps trigger.
//!
//! Every statement here follows the model crate (`docs/models/evolution.md`):
//! there is no fitness function, gene frequencies change only through
//! differential survival and reproduction, drift and mutation.

use bevy::prelude::*;
use interface::{
    ActiveSession, ModeScenarios, RightPanelTab, ShellCommand, Toasts, Tutorial, TutorialAction, TutorialPage,
};
use simulation_core::ParamValue;

use crate::scene::SceneState;
use crate::session::{EvoData, EvoRunner, EvoScenarios, load_scenario};
use crate::ui_state::{DRIFT_GENE, EvoUi, GroundOverlay, PropertiesTab, Tool};

/// The tutorial registered for the mode.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "Welcome to the Evolution Laboratory",
                "Every creature carries a **genome of 13 heritable genes** and lives in a world of regrowing food, \
                 a temperature field and barriers. **Nothing in the model computes fitness.** Gene frequencies change \
                 only because some organisms survive and breed more than others, by chance (drift) and by mutation.\n\n\
                 The bodies you see are built from the genes:\n\
                 - **body size** sets the overall scale\n\
                 - **speed** sets leg length and posture (long-legged and upright when fast)\n\
                 - **thermal optimum** sets the coat colour, on the same scale as the ground's temperature tint\n\
                 - **perception** sets ear length and eye size\n\n\
                 Predators are dark canids. Grass height shows the food on each cell, and grazed or poor ground \
                 is darker.",
            )
            .with_action("load:speed_selection", "Load “Speed selection”"),
            TutorialPage::new(
                "Inheritance versus individual state",
                "Select a creature (Select tool, key **1**, then click it). The inspector separates three things:\n\n\
                 1. **Heritable genome** — copied (and recombined in sexual mode) into offspring at birth.\n\
                 2. **Acquired traits** — thermal *acclimation* and predator *experience* change during life. They \
                 live in the genome object but are declared non-heritable: offspring start at 0.\n\
                 3. **Individual state** — energy, health, age, offspring count, position. Never inherited; a newborn \
                 starts with a fresh state and the energy its parents invested.\n\n\
                 A well-fed parent does not pass on its energy reserve as a trait, and an acclimated parent does not \
                 pass on its acclimation.",
            )
            .with_action("select:prey", "Inspect a random prey"),
            TutorialPage::new(
                "Mutation",
                "At every birth each heritable gene mutates with probability **mutation_rate** (default 0.1) by a \
                 normal step of standard deviation **mutation_size** (default 0.05 of the gene's range), reflected at \
                 the range limits. Mutation is undirected: it does not know what would help.\n\n\
                 Open **Genes** and pick a gene: the band is the mean ± one standard deviation over time. Selection \
                 and drift move the mean and also narrow the band; mutation adds new values and keeps it from \
                 collapsing (recombination only reshuffles existing values between genes). The `mutations` metric \
                 counts mutated genes among each tick's newborns.",
            )
            .with_action("tab:genes", "Open the gene charts"),
            TutorialPage::new(
                "Selection emerges from survival",
                "In *Speed selection*, 40 identical predators of top speed 0.85 hunt prey whose speed varies around \
                 0.6. Fast prey outrun chases more often; slow prey are caught more often. No line of code rewards \
                 speed — yet the survivors, and therefore the parents of the next generation, are faster on average.\n\n\
                 Speed is not free: upkeep grows with `size × speed²` every tick and running costs with `size × v²`. \
                 Without predators nothing pays for those costs, so mean speed ends up **lower** than with predators \
                 (it may still wander up or down by drift). The response is small and varies between seeds: compare \
                 runs, not a single curve. Press **Play** (Space) and watch the speed chart against the dashed \
                 founders' mean.",
            )
            .with_action("gene:speed", "Chart the speed gene"),
            TutorialPage::new(
                "Try it: a controlled comparison",
                "A single run proves little: the same set-up with another seed can end differently. To test whether \
                 predators select on speed:\n\n\
                 1. Run the treatment for a fixed number of ticks (e.g. 2000) and note the mean speed.\n\
                 2. Run the **control**: same scenario, **same seed**, predators removed. With the same seed the prey \
                 founders are identical, so any difference comes from the treatment and from chance during the run.\n\
                 3. Repeat both arms on several seeds (*Next replicate* in the Tools panel) and compare the averages.\n\n\
                 A stricter control removes prey **at random** at the predators' kill rate (Cull tool: victims are \
                 chosen uniformly): it separates the selectivity of predation from its effect on density.",
            )
            .with_action("control:no_predators", "Set up the control (no predators, same seed)"),
            TutorialPage::new(
                "Genetic drift",
                "The two **neutral markers** (marker A and B) have no effect on the body or on survival. Their \
                 frequencies still change: each generation is a random sample of the parents, so allele classes \
                 wander and, in time, one of them fixes. In a population of N the expected heterozygosity of a \
                 neutral gene falls by about a factor `(1 − 1/N)` per generation (mutation slowly restores it), so \
                 **small populations drift fast** and large ones slowly.\n\n\
                 The allele-class chart of marker A shows drift: classes widen and narrow without a consistent \
                 direction. Compare with a selected gene, which moves consistently.",
            )
            .with_action("gene:marker_a", "Chart neutral marker A"),
            TutorialPage::new(
                "Isolation and divergence",
                "*Island evolution* puts one sexual population on three islands at about 10, 18 and 26 °C, with \
                 assortative mating (mates must be genetically closer than θ = 0.18). Separated by water, the islands \
                 adapt to their temperatures and drift apart: **F_ST** and the regional **net divergence** grow.\n\n\
                 The **Regions** tab shows the distance matrix between islands and the explicit speciation \
                 criterion. Raise the *land bridge* (Barriers, index 1) to bring the populations back into contact \
                 and see whether they still interbreed.",
            )
            .with_action("load:island_evolution", "Load “Island evolution”"),
            TutorialPage::new(
                "Simple example: adapting to warming",
                "*Environmental resistance* warms the whole field by 6 °C per 1000 ticks over a 10 °C gradient. Watch \
                 the ground shift towards warm colours and the prey coats follow behind it: the mean thermal optimum \
                 tracks the temperature with a lag of a few degrees. Acclimation (at most ±4 °C, not inherited) buffers \
                 individuals; only genetic change keeps the population alive in the long run. The optimum cannot \
                 evolve above 40 °C, so the population eventually declines.\n\n\
                 Use the Climate controls to warm faster (e.g. 15 °C / 1000 ticks) and evolution falls behind.",
            )
            .with_action("load:thermal_adaptation", "Load “Environmental resistance”"),
            TutorialPage::new(
                "Advanced example: an arms race",
                "In the *Predator–prey arms race* both species inherit and mutate speed, perception and size. Prey \
                 are selected to escape, predators to catch: both mean speeds tend to rise together (a \"Red Queen\" \
                 race) while the populations oscillate. Chart `speed` for **Prey** and for **Predators** in the Genes \
                 tab and follow a hunting predator with **F**.\n\n\
                 The outcome varies between seeds: in some replicates one side barely changes.",
            )
            .with_action("load:arms_race", "Load “Arms race”"),
            TutorialPage::new(
                "Reading the measurements",
                "- **Mean ± sd** per gene over time; the histogram shows the distribution now, the dashed line the \
                 founders' mean; *Δ (founder sd)* is the change since the start in units of the founders' spread.\n\
                 - **Allele classes**: the gene's range cut into equal classes; **H = 1 − Σ p_i²** is the chance that \
                 two random organisms fall in different classes.\n\
                 - **π (genetic diversity)**: mean RMS distance between two prey genomes over the 13 normalised genes.\n\
                 - **F_ST = (H_T − H_S) / H_T**: share of diversity found between regions (0 = alike, 1 = fixed \
                 differences).\n\
                 - **Net divergence** `d_AB − (d_A + d_B)/2`: how much further apart two regions are than their own \
                 members.\n\
                 - **Speciation** is declared only by the documented two-cluster criterion (sexual mode, θ < 1, \
                 persisting for several generations).",
            )
            .with_action("tab:regions", "Open the Regions tab"),
            TutorialPage::new(
                "Suggested experiments",
                "1. **Scarcity**: *Food scarcity adaptation* vs the same seed with `food_capacity_scale` 2 (Parameters, \
                 then Reset). Compare perception and efficiency.\n\
                 2. **r vs K**: *Competing strategies* — watch marker A (0.1 = r founders, 0.9 = K founders); raise \
                 `background_mortality` to 0.008 and reset.\n\
                 3. **Bottleneck**: cull 90 % of the prey. A random cull hardly changes π at once, but while the \
                 population is small drift erodes π and H, and mutation restores them only slowly.\n\
                 4. **Founder event**: spawn 10 clones of an unusual genome on an empty corner (Spawn tool, spread 0).\n\
                 5. **Gene flow**: in *Island evolution*, raise the land bridge and watch F_ST fall.\n\n\
                 Always compare against a control run with the same seed, and repeat on several seeds.",
            )
            .with_action("tool:spawn", "Pick the Spawn tool"),
        ],
        concepts: CONCEPTS.into(),
    }
}

const CONCEPTS: &str = "## Genotype, phenotype, state\n\
The **genotype** is the 13 heritable gene values (normalised to 0–1). The **phenotype** is their expression in \
real units (speed in units/tick, optimum in °C…). **State** (energy, health, age) belongs to the individual and is \
never inherited.\n\n\
## Natural selection\n\
Heritable variation plus differences in survival or reproduction that depend on that variation: the next \
generation differs. In this model selection is an **outcome** of physiology, behaviour, predators and climate; it is never \
computed.\n\n\
## Genetic drift\n\
Random change in gene frequencies because only some organisms happen to breed. Strongest in small populations; acts \
on every gene, and alone on neutral markers.\n\n\
## Mutation\n\
Random change of a gene at birth (probability `mutation_rate`, step `N(0, mutation_size)`). The only source of new \
variation besides spawning.\n\n\
## Isolation and gene flow\n\
Barriers stop migration. Isolated populations diverge by drift and local selection; gene flow (a land bridge) mixes \
them again.\n\n\
## Trade-offs\n\
Large bodies store more energy but cost more; speed costs grow with `size × speed²`; perception costs energy linearly; broad \
thermal tolerance raises basal metabolism; many offspring share the same energy budget.\n\n\
## Controlled comparisons\n\
Same seed + same interventions = identical run. Change **one** thing, keep the seed, then repeat with other seeds \
(replicates) before drawing conclusions.";

/// Handles tutorial step actions (and the same ids from the standalone example).
#[allow(clippy::too_many_arguments)]
pub fn handle_tutorial_actions(
    mut actions: MessageReader<TutorialAction>,
    runner: Option<Res<EvoRunner>>,
    session: Option<Res<ActiveSession>>,
    scenarios: Res<EvoScenarios>,
    mut data: ResMut<EvoData>,
    mut ui: ResMut<EvoUi>,
    mut toasts: ResMut<Toasts>,
    mut scene: ResMut<SceneState>,
    mut shell: MessageWriter<ShellCommand>,
    mut picker: ResMut<ModeScenarios>,
) {
    let ids: Vec<String> = actions.read().map(|a| a.0.clone()).collect();
    let Some(runner) = runner else { return };
    for id in ids {
        let (verb, arg) = id.split_once(':').unwrap_or((id.as_str(), ""));
        match verb {
            "load" => match scenarios.get(arg) {
                Some(scenario) => {
                    if load_scenario(&runner, scenario, None, &mut data, &mut ui, &mut toasts, &mut picker) {
                        scene.frame_request = true;
                        if arg == "island_evolution" {
                            ui.overlay = GroundOverlay::Temperature;
                        }
                    }
                }
                None => toasts.warning(format!("Unknown scenario `{arg}`")),
            },
            "tab" => {
                if let Some(tab) = PropertiesTab::parse(arg) {
                    ui.tab = tab;
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
                }
            }
            "gene" => {
                if ui.select_gene(arg) {
                    ui.tab = PropertiesTab::Genes;
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
                }
            }
            "select" => {
                let species = if arg == "predator" { evolution::Species::Predator } else { evolution::Species::Prey };
                let pick = data.view.as_ref().and_then(|v| {
                    let ids: Vec<_> = v.organisms.iter().filter(|o| o.species == species).map(|o| o.id).collect();
                    (!ids.is_empty()).then(|| ids[ids.len() / 2])
                });
                if let Some(id) = pick {
                    ui.selected = Some(id);
                    ui.tab = PropertiesTab::Inspector;
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
                }
            }
            "tool" => {
                ui.tool = match arg {
                    "food" => Tool::Food,
                    "cull" => Tool::Cull,
                    "spawn" => Tool::Spawn,
                    "predators" => Tool::Predators,
                    _ => Tool::Select,
                };
            }
            "overlay" => {
                if let Some(o) = GroundOverlay::ALL.into_iter().find(|o| o.label().eq_ignore_ascii_case(arg)) {
                    ui.overlay = o;
                }
            }
            "control" if arg == "no_predators" => {
                if let Some(session) = &session {
                    // Same seed: the prey founders are drawn before the
                    // predators from the founder stream, so they are identical.
                    session.session().set_param("initial_predators", ParamValue::Int(0));
                    session.session().reset();
                    toasts.info("Control set up: initial_predators = 0, reset with the same seed. Press Play.");
                }
            }
            "play" => {
                if let Some(session) = &session {
                    session.session().play();
                }
            }
            _ => toasts.warning(format!("Unknown tutorial action `{id}`")),
        }
    }
}

/// Gene charted by the drift lesson.
pub const DRIFT_LESSON_GENE: usize = DRIFT_GENE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutorial_pages_reference_existing_scenarios_and_genes() {
        let t = tutorial();
        assert!(t.pages.len() >= 10);
        let scenarios = EvoScenarios::load();
        for page in &t.pages {
            assert!(!page.title.is_empty() && !page.body.is_empty());
            if let Some(action) = &page.action {
                let (verb, arg) = action.id.split_once(':').unwrap();
                match verb {
                    "load" => assert!(scenarios.get(arg).is_some(), "{arg}"),
                    "gene" => assert!(evolution::genes::gene_index(arg).is_some(), "{arg}"),
                    "tab" => assert!(PropertiesTab::parse(arg).is_some(), "{arg}"),
                    "select" | "tool" | "control" | "overlay" | "play" => {}
                    other => panic!("unknown verb {other}"),
                }
            }
        }
        assert!(t.concepts.contains("Controlled comparisons"));
        assert_eq!(DRIFT_LESSON_GENE, evolution::genes::MARKER_A);
    }
}
