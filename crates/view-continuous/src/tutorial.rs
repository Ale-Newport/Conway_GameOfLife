//! The mode's tutorial (steps with actions) and concepts page, and the
//! handling of tutorial actions.
//!
//! Numbers quoted in the text come from `docs/models/continuous.md` ("How
//! it was verified") or were measured with this build of the `continuous`
//! crate (Orbium tutorial scenario, 600 ticks): Orbium glides at about 0.60
//! cells per tick with a mass near 73; with σ ≤ 0.013 or μ ≥ 0.16 it dies,
//! with σ = 0.025 it grows into chaotic matter covering about 22 % of the
//! world; doubling R together with the creature's size gives a creature four
//! times as heavy and twice as fast per tick; T = 5, 20, 50 give 1.10, 0.32
//! and 0.13 cells per tick; painted discs (radius 6–20, value 0.3–1) die.

use bevy::prelude::*;
use continuous::ViewLayer;
use interface::{RightPanelTab, ShellCommand, Tutorial, TutorialAction, TutorialPage};

use crate::session::{ContinuousRunner, LoadRequests, LoadedWorld};
use crate::tools::{Tool, ToolSettings};
use crate::world::ViewSettings;

/// What a tutorial button asks the mode to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TutorialCommand {
    /// Load a scenario by id.
    Load(String),
    /// Show a layer.
    Layer(ViewLayer),
    /// Select a tool.
    Tool(Tool),
    /// Show the centroid trail and start the run.
    RunWithTrail,
    /// Show a tab of the right panel.
    Tab(RightPanelTab),
}

impl TutorialCommand {
    /// Parses an action id (`load:<id>`, `layer:<state|potential|growth>`,
    /// `tool:<inspect|brush|erase|stamp>`, `run-trail`, `tab:<properties|parameters|rules|metrics>`).
    pub fn parse(action: &str) -> Option<Self> {
        let (verb, arg) = action.split_once(':').unwrap_or((action, ""));
        match verb {
            "load" if !arg.is_empty() => Some(Self::Load(arg.to_string())),
            "layer" => Some(Self::Layer(match arg {
                "state" => ViewLayer::State,
                "potential" => ViewLayer::Potential,
                "growth" => ViewLayer::Growth,
                _ => return None,
            })),
            "tool" => Some(Self::Tool(match arg {
                "inspect" => Tool::Inspect,
                "brush" => Tool::Brush,
                "erase" => Tool::Erase,
                "stamp" => Tool::Stamp,
                _ => return None,
            })),
            "run-trail" => Some(Self::RunWithTrail),
            "tab" => Some(Self::Tab(match arg {
                "properties" => RightPanelTab::Properties,
                "parameters" => RightPanelTab::Parameters,
                "rules" => RightPanelTab::Rules,
                "metrics" => RightPanelTab::Metrics,
                _ => return None,
            })),
            _ => None,
        }
    }
}

/// Executes tutorial actions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tutorial_actions(
    mut actions: MessageReader<TutorialAction>,
    mut requests: ResMut<LoadRequests>,
    mut settings: ResMut<ViewSettings>,
    mut tools: ResMut<ToolSettings>,
    mut shell: MessageWriter<ShellCommand>,
    loaded: Option<Res<LoadedWorld>>,
    runner: Option<Res<ContinuousRunner>>,
) {
    let channels = loaded.as_ref().map_or(1, |l| l.channels());
    for action in actions.read() {
        let Some(command) = TutorialCommand::parse(&action.0) else {
            warn!("unknown tutorial action `{}`", action.0);
            continue;
        };
        match command {
            TutorialCommand::Load(id) => requests.0.push(id),
            TutorialCommand::Layer(layer) => settings.set_layer(layer, channels),
            TutorialCommand::Tool(tool) => tools.tool = tool,
            TutorialCommand::RunWithTrail => {
                settings.show_trail = true;
                settings.set_layer(ViewLayer::State, channels);
                if let Some(runner) = &runner {
                    runner.0.play();
                }
            }
            TutorialCommand::Tab(tab) => {
                shell.write(ShellCommand::ShowRightTab(tab));
            }
        }
    }
}

/// Tutorial steps and concepts of the mode.
pub fn tutorial() -> Tutorial {
    let pages = vec![
        TutorialPage::new(
            "Continuous artificial life",
            "This laboratory runs **continuous cellular automata**: Lenia, Expanded Lenia, SmoothLife and \
             Flow-Lenia. As in Conway's Game of Life, every cell of a grid is updated by the same local rule, \
             but states, neighbourhoods and time are (almost) continuous, and the creatures that appear look \
             strikingly organic.\n\n\
             The world is a **torus**: what leaves on one side comes back on the other. Pick models in the \
             *Model* section of the Tools panel or in the scenario menu of the top bar.\n\n\
             Load *Orbium unicaudatus*, Lenia's best-known glider, and press **Space** to run it.",
        )
        .with_action("load:lenia.orbium", "Load Orbium"),
        TutorialPage::new(
            "Discrete versus continuous",
            "| | Game of Life | Lenia |\n|---|---|---|\n\
             | State of a cell | 0 or 1 | any real number in [0, 1] |\n\
             | Neighbourhood | the 8 adjacent cells | a smooth ring of radius **R** cells (R = 13 for Orbium: about 500 cells) |\n\
             | Rule | birth on 3, survival on 2 or 3 | a smooth growth function of the weighted neighbourhood |\n\
             | Time | one jump per generation | small steps **dt = 1/T** (T = 10 for Orbium) |\n\n\
             With smooth kernels and growth functions, a small change of a parameter or of the state gives a \
             small change of the next state (over many ticks differences can still grow). Making R and T \
             larger approaches a continuum in space and time; a creature keeps its shape when R and the \
             creature are scaled together (see the experiments).\n\n\
             The colours show the state: dark is 0, bright is 1 (colormap in the View section).",
        )
        .with_action("layer:state", "Show the state"),
        TutorialPage::new(
            "The kernel: a smooth neighbourhood",
            "Each cell looks at its surroundings through a **kernel K**: a radially symmetric weight that is \
             zero beyond R cells and has one or more concentric **rings** (peaks **β**). The weights are \
             normalised so that they sum to 1, so the **potential**\n\n\
             `U = K * A`\n\n\
             is a weighted average of the states around each cell, between 0 and 1.\n\n\
             The *Kernel and growth* section of the Properties tab plots K against the distance; the button \
             shows U itself: bright where a cell's ring-shaped neighbourhood is well filled.",
        )
        .with_action("layer:potential", "Show the potential U"),
        TutorialPage::new(
            "The growth function",
            "The potential is turned into a **growth** by a bell-shaped function with centre **μ** and width \
             **σ**:\n\n\
             `G(U) = 2·exp(−(U − μ)² / (2σ²)) − 1`\n\n\
             G is +1 when U = μ, positive while |U − μ| < σ·√(2 ln 2) ≈ 1.18 σ, and tends to −1 far from μ \
             (empty space has U = 0, so it shrinks towards 0). Every tick adds a fraction of it:\n\n\
             `A(t + dt) = clip(A(t) + dt·G(U), 0, 1)`\n\n\
             The growth plot shades the band of positive growth. The button shows the growth field of the \
             last tick: positive where matter is being added, negative where it is being removed.",
        )
        .with_action("layer:growth", "Show the growth field"),
        TutorialPage::new(
            "Why Orbium moves",
            "Nothing in the rule mentions motion: the kernel is the same in every direction and the same \
             update is applied to every cell. Orbium moves because of its **shape**. Its front and back \
             differ, so at its leading edge the potential falls inside the growth band and matter is added, \
             while along its trailing edge the potential is outside the band and matter decays. Adding at \
             the front and removing at the back moves the pattern by about **0.6 cells per tick** while its \
             mass stays near 73.\n\n\
             The motion is **emergent**: the direction comes only from the creature's own asymmetry. \
             Rotate a stamped Orbium by 90° and it travels in a direction rotated by 90°. Press the button \
             to run with the trajectory of the centre of mass drawn on the world.",
        )
        .with_action("run-trail", "Run with the centroid trail"),
        TutorialPage::new(
            "Try it: paint and erase",
            "Select the **Brush** (key 2) and drag in the world; **Erase** (key 3) removes matter. The brush \
             blends cells towards its value with a soft edge; each stroke is one undo step (Ctrl/Cmd+Z).\n\n\
             Most painted blobs die: a disc of radius 6 to 20 cells painted at value 0.3 to 1 under the \
             Orbium rule fades away. Lenia creatures are narrow attractors — you have to be close to one \
             before the rule pulls the shape in.",
        )
        .with_action("tool:brush", "Select the brush"),
        TutorialPage::new(
            "Try it: place creatures",
            "Select the **Stamp** (key 4), pick a creature and click in the world. **Q / E** rotate the \
             preview by 15°, **[ / ]** scale it. Creatures belong to the rule they were found in (shown in \
             the list); in another rule they usually dissolve or explode.\n\n\
             Place two Orbiums on crossing paths and watch what the collision produces: the outcome depends \
             on the angle and the timing.",
        )
        .with_action("tool:stamp", "Select the stamp"),
        TutorialPage::new(
            "Example: Orbium (simple)",
            "Orbium uses one channel and one kernel: R = 13, T = 10, μ = 0.15, σ = 0.015. Change μ and σ in \
             the Properties tab while it runs — the plots follow, and changes apply at the next tick.\n\n\
             Measured with this engine over 600 ticks: σ = 0.014 to 0.020 keeps a glider (heavier and faster \
             as σ grows); σ ≤ 0.013 kills it; σ = 0.025 makes it grow until it fills about a fifth of the \
             world with chaotic matter. μ between 0.13 and 0.155 keeps it alive; μ ≥ 0.16 kills it.",
        )
        .with_action("load:lenia.orbium", "Reload Orbium"),
        TutorialPage::new(
            "Example: Tessellatium (advanced)",
            "**Expanded Lenia** (Chan 2020) uses several channels and kernels. *Tessellatium gyrans* has \
             **3 channels and 15 kernels**: each kernel reads one channel and adds its growth to another, so \
             the channels behave like interacting tissues. The view shows the channels as an RGB composite \
             (channel 0 red, 1 green, 2 blue); pick a single channel in the View section to see it alone.\n\n\
             Starting from the tutorial cells it reorganises during the first ~100 ticks into a compact \
             glider of mass about 810.",
        )
        .with_action("load:lenia.tessellatium_gyrans", "Load Tessellatium"),
        TutorialPage::new(
            "SmoothLife and Flow-Lenia",
            "**SmoothLife** (Rafler 2011) measures two fillings: the inner disc **m** (is this cell alive?) \
             and the surrounding annulus **n**. A smooth version of Life's birth and survival intervals gives \
             the new state. Its smooth glider moves about **5.3 cells per step**; the growth plot shows the \
             transition for dead and live cells.\n\n\
             **Flow-Lenia** (Plantec et al. 2023) moves matter instead of creating it: the growth becomes an \
             *affinity* and matter flows up its gradient, so the **total mass is conserved** (the \
             *relative mass drift* metric stays near 1e-8, floating-point error).",
        )
        .with_action("load:smoothlife.glider", "Load the SmoothLife glider"),
        TutorialPage::new(
            "Reading the metrics",
            "The Metrics tab plots every tick:\n\n\
             - **Total mass** — Σ A over all cells and channels.\n\
             - **Active fraction** — cells whose summed value exceeds 0.01.\n\
             - **Centroid x, y** — the mass-weighted circular mean position on the torus.\n\
             - **Centroid speed** — how far the centroid moved in the last tick (cells/tick). With several \
               creatures it describes the group, not an individual.\n\
             - **Mean growth** — the average growth of the last tick (Flow-Lenia: mean affinity).\n\
             - **Relative mass drift** (Flow-Lenia) — deviation from the mass after the last reset or edit.\n\n\
             A healthy glider shows a flat mass and a constant speed.",
        )
        .with_action("tab:metrics", "Open the Metrics tab"),
        TutorialPage::new(
            "Suggested experiments",
            "- **Scale invariance**: set R = 26, clear, and stamp Orbium at scale 2 — it survives with about \
               four times the mass and moves twice as many cells per tick (the same speed in kernel radii).\n\
             - **Time resolution**: raise T from 10 to 20 or 50. The speed per tick drops roughly as 1/T (0.60, \
               0.32, 0.13 cells/tick) while the speed per time unit stays between 6 and 7 cells.\n\
             - **Edge of life**: lower σ in steps of 0.001 until Orbium dies; look at the growth field just \
               before it does.\n\
             - **Kernel shape**: switch the kernel core between Gaussian bump and polynomial.\n\
             - **Soups**: load the Lenia random soup: the dense disc of random matter spreads over the \
               world into a persistent chaotic phase (about 23 % of the cells active, Metrics tab). In \
               SmoothLife, **Randomize** in the bottom bar scatters discs drawn from the seed; about half of \
               the seeds condense into gliders, the others die out.\n\
             - **Conservation**: in Flow-Lenia, paint matter and watch the mass jump once and then stay \
               constant.",
        ),
    ];
    Tutorial { pages, concepts: CONCEPTS.to_string() }
}

const CONCEPTS: &str = "\
## Discrete and continuous automata

A cellular automaton updates every cell of a grid with one local rule. Conway's Game of Life uses two states, \
the eight adjacent cells and a table (birth on 3, survival on 2 or 3). **Continuous** automata replace the \
states by real numbers, the neighbourhood by a smooth weighting over a disc of radius R, the table by a smooth \
function and the generation by a time step dt. Chan (2019) introduced Lenia as such a generalisation; the Game of \
Life is its limiting case with radius 1, one update per generation and two states.

## Lenia

One tick of Lenia (Chan 2019; Expanded Lenia, Chan 2020):

1. Potential `U_k = K_k * A_src(k)` for every kernel k (a periodic convolution, computed with FFTs).
2. Growth `D_j = Σ h_k · G_k(U_k)` over the kernels that feed channel j (or their weighted mean).
3. Update `A_j(t + dt) = clip(A_j(t) + dt · D_j, 0, 1)` with `dt = 1/T`.

The **kernel** is proportional to `β_i · K_C(Bρ − i)` with `i = floor(Bρ)` and `ρ = |n| / (R·r)`: a set of B concentric rings with \
peaks β and a core profile K_C (Gaussian bump, exponential, polynomial, rectangular or staircase), normalised \
to sum to 1. The **growth function** `G(u) = 2·exp(−(u − μ)²/(2σ²)) − 1` (or a polynomial or rectangular \
variant) rewards potentials near μ and punishes all others.

## Why creatures move

The rule is isotropic and the same everywhere; motion is a property of the pattern. A glider such as Orbium is \
asymmetric front to back: at the front the potential lies in the growth band, at the back it does not, so \
matter is created ahead and destroyed behind. The shape is regenerated one step further along: a travelling \
solution of the equations, not a programmed movement. Rotating the creature rotates its path.

## SmoothLife

Rafler (2011) computes, for every cell, the inner filling **m** (a disc of radius r_i) and the outer filling \
**n** (the annulus between r_i and r_a). The transition `s(n, m)` is a smooth version of Life's intervals: \
birth when n is in [b₁, b₂] for a dead cell, survival when n is in [d₁, d₂] for a live one, with sigmoids of \
widths α_n and α_m. In discrete mode the new state is `f = s(n, m)`.

## Flow-Lenia

Plantec et al. (2023) keep Lenia's kernels and growth but use them as an **affinity** U. Matter moves along \
`F = (1 − α)·grad U − α·grad A_Σ`, where `α = (A_Σ/θ_A)^n` switches to diffusion when matter piles up, and is \
redistributed by *reintegration tracking*. Total mass is conserved exactly up to rounding.

## Reading the display

- **State / Mass** — the cells' values (RGB composite for several channels).
- **Potential** — `K * A` of kernel 1 (SmoothLife: the fillings m and n).
- **Growth** — the last tick's growth (SmoothLife: `2s − 1`; Flow-Lenia: the affinity).
- The scale bar in the overlay is in cells; time is ticks × dt (time units).

## References

- B. W.-C. Chan, *Lenia — Biology of Artificial Life*, Complex Systems 28(3), 2019.
- B. W.-C. Chan, *Lenia and Expanded Universe*, ALIFE 2020.
- S. Rafler, *Generalization of Conway's Game of Life to a continuous domain — SmoothLife*, 2011.
- E. Plantec et al., *Flow-Lenia*, ALIFE 2023; Artificial Life 31(2), 2025.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Catalog;

    #[test]
    fn actions_parse() {
        assert_eq!(TutorialCommand::parse("load:lenia.orbium"), Some(TutorialCommand::Load("lenia.orbium".into())));
        assert_eq!(TutorialCommand::parse("layer:growth"), Some(TutorialCommand::Layer(ViewLayer::Growth)));
        assert_eq!(TutorialCommand::parse("tool:stamp"), Some(TutorialCommand::Tool(Tool::Stamp)));
        assert_eq!(TutorialCommand::parse("run-trail"), Some(TutorialCommand::RunWithTrail));
        assert_eq!(TutorialCommand::parse("tab:metrics"), Some(TutorialCommand::Tab(RightPanelTab::Metrics)));
        assert_eq!(TutorialCommand::parse("load:"), None);
        assert_eq!(TutorialCommand::parse("layer:colour"), None);
        assert_eq!(TutorialCommand::parse("dance"), None);
    }

    #[test]
    fn every_page_action_is_understood_and_every_scenario_exists() {
        let catalog = Catalog::builtin();
        let t = tutorial();
        assert!(t.pages.len() >= 10);
        for page in &t.pages {
            let Some(action) = &page.action else { continue };
            let command = TutorialCommand::parse(&action.id);
            assert!(command.is_some(), "page `{}`: unknown action {}", page.title, action.id);
            if let Some(TutorialCommand::Load(id)) = command {
                assert!(catalog.get(&id).is_some(), "page `{}` loads missing scenario {id}", page.title);
            }
        }
        assert!(t.concepts.contains("## Lenia") && t.concepts.contains("## Why creatures move"));
    }
}
