//! The mode's tutorial (steps with actions), its concepts page, and the
//! handling of tutorial actions.
//!
//! Every number in the texts is asserted by this module's tests against the
//! models (HighLife replicator copies, Seeds, Brian's Brain, Wireworld
//! timings, Langton's ant counts, rule 90 rows) or comes from the bundled
//! pattern files and the model crate's own tests (`docs/models/cellular-automata.md` §10).

use bevy::prelude::*;
use interface::{RightPanelTab, ShellCommand, Tutorial, TutorialAction, TutorialPage};
use simulation_core::ParamValue;

use crate::session::{LabRunner, LoadRequests};
use crate::tools::{Tool, ToolSettings};
use crate::world::{Colouring, ViewSettings};

/// One step of a tutorial action (actions are `;`-separated lists).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TutorialCommand {
    /// Load a scenario by id.
    Load(String),
    /// Run this many ticks now.
    Step(u64),
    /// Select a tool.
    Tool(Tool),
    /// Show a tab of the right panel.
    Tab(RightPanelTab),
    /// Set the elementary rule number.
    WolframRule(u8),
    /// Choose the colouring.
    Colouring(Colouring),
    /// Start running.
    Play,
}

impl TutorialCommand {
    /// Parses one step: `load:<id>`, `step:<n>`, `tool:<draw|erase|line|fill|stamp|ant|toggle|inspect>`,
    /// `tab:<properties|parameters|rules|metrics>`, `wolfram:<0..255>`,
    /// `colouring:<states|trail|age|activity>`, `play`.
    pub fn parse(step: &str) -> Option<Self> {
        let (verb, arg) = step.trim().split_once(':').unwrap_or((step.trim(), ""));
        Some(match verb {
            "load" if !arg.is_empty() => Self::Load(arg.to_string()),
            "step" => Self::Step(arg.parse().ok().filter(|n| *n > 0)?),
            "tool" => Self::Tool(match arg {
                "inspect" => Tool::Inspect,
                "draw" => Tool::Draw,
                "erase" => Tool::Erase,
                "line" => Tool::Line,
                "fill" => Tool::Fill,
                "stamp" => Tool::Stamp,
                "ant" => Tool::AddAnt,
                "toggle" => Tool::Toggle,
                _ => return None,
            }),
            "tab" => Self::Tab(match arg {
                "properties" => RightPanelTab::Properties,
                "parameters" => RightPanelTab::Parameters,
                "rules" => RightPanelTab::Rules,
                "metrics" => RightPanelTab::Metrics,
                _ => return None,
            }),
            "wolfram" => Self::WolframRule(arg.parse().ok()?),
            "colouring" => Self::Colouring(match arg {
                "states" => Colouring::States,
                "trail" => Colouring::Trail,
                "age" => Colouring::Age,
                "activity" => Colouring::Activity,
                _ => return None,
            }),
            "play" => Self::Play,
            _ => return None,
        })
    }

    /// Parses a `;`-separated action; `None` if any step is unknown.
    pub fn parse_all(action: &str) -> Option<Vec<Self>> {
        action.split(';').filter(|s| !s.trim().is_empty()).map(Self::parse).collect()
    }
}

/// Executes tutorial actions. Steps that reach the runner (load, step,
/// rule) are sent in order, so "load, then run 84 ticks" runs the new
/// scenario.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tutorial_actions(
    mut actions: MessageReader<TutorialAction>,
    mut requests: ResMut<LoadRequests>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut shell: MessageWriter<ShellCommand>,
    mut deferred: Local<Vec<TutorialCommand>>,
    runner: Option<Res<LabRunner>>,
    session: Option<Res<interface::ActiveSession>>,
) {
    // Steps queued behind a scenario load run once the load went out (next frame).
    let mut queue: Vec<TutorialCommand> = std::mem::take(&mut *deferred);
    for action in actions.read() {
        match TutorialCommand::parse_all(&action.0) {
            Some(steps) => queue.extend(steps),
            None => warn!("unknown tutorial action `{}`", action.0),
        }
    }
    let mut loading = false;
    for command in queue {
        if loading {
            deferred.push(command);
            continue;
        }
        match command {
            TutorialCommand::Load(id) => {
                requests.0.push(id);
                loading = true;
            }
            TutorialCommand::Step(n) => {
                if let Some(runner) = &runner {
                    runner.0.step(n);
                }
            }
            TutorialCommand::Tool(tool) => tools.tool = tool,
            TutorialCommand::Tab(tab) => {
                shell.write(ShellCommand::ShowRightTab(tab));
            }
            TutorialCommand::WolframRule(rule) => {
                if let Some(session) = &session {
                    session.session().set_param("rule", ParamValue::Int(i64::from(rule)));
                }
            }
            TutorialCommand::Colouring(c) => settings.colouring = c,
            TutorialCommand::Play => {
                if let Some(runner) = &runner {
                    runner.0.play();
                }
            }
        }
    }
}

/// Tutorial steps and concepts of the mode.
pub fn tutorial() -> Tutorial {
    let pages = vec![
        TutorialPage::new(
            "The Cellular Automata Lab",
            "This laboratory runs **other cellular automata** than Conway's Game of Life, one at a time:\n\n\
             * **Life-like** rules: two states and a birth/survival rule (HighLife, Seeds, Day & Night, Maze...).\n\
             * **Generations** rules: Life-like rules with extra *dying* states (Brian's Brain, Star Wars).\n\
             * **Wireworld**: four states for digital circuits.\n\
             * **Langton's ant** and other turmites: ants that walk on colours.\n\
             * **Elementary** one-dimensional automata (Wolfram's 256 rules).\n\n\
             Switch families with the *Model* buttons of the Tools panel or with the scenario menu of the top \
             bar. Press **Space** to run, **N** to advance one tick, **R** to reset. Drag with the right mouse \
             button to pan and use the wheel to zoom.",
        )
        .with_action("load:highlife-replicator", "Open HighLife"),
        TutorialPage::new(
            "What a rule string means",
            "Life-like rules are written **B**/**S**: the numbers of live neighbours (out of the 8 surrounding \
             cells) for which a dead cell is **b**orn and a live cell **s**urvives.\n\n\
             | Rule | Born with | Survives with |\n|---|---|---|\n\
             | `B3/S23` (Conway) | 3 | 2 or 3 |\n\
             | `B36/S23` (HighLife) | 3 or 6 | 2 or 3 |\n\
             | `B2/S` (Seeds) | 2 | never |\n\n\
             The older form without letters lists survival first: `23/3` is `B3/S23`. Generations rules add the \
             number of states: `B2/S/C3` (Golly writes `/2/3`). A turmite rule has one letter per colour \
             (`RL`, `LLRR`), and an elementary rule is a number from 0 to 255.\n\n\
             Type a rule in *Tools › Rule*: it is checked as you type, and *Properties › Rule* compares it with \
             Conway's.",
        )
        .with_action("load:seeds;tab:properties", "Open Seeds and compare"),
        TutorialPage::new(
            "Simple example: HighLife's replicator",
            "HighLife differs from Life only by **birth on 6 neighbours**, and that is enough for a 12-cell \
             **replicator**: every 12 generations each copy duplicates along the diagonal, and copies that meet \
             annihilate. After 12·n generations the copies sit where rule 90 puts its live cells after n steps, \
             so there are 2 copies at generation 12, 4 at 36 and **8 at 84** (96 cells).\n\n\
             Run 84 generations and count the copies.",
        )
        .with_action("load:highlife-replicator;step:84", "Run to generation 84"),
        TutorialPage::new(
            "Draw and edit",
            "Pick a tool in the Tools panel (keys **1** to **6**) and drag with the left button in the world:\n\n\
             * **Draw / Paint** and **Erase** use a round brush (**[** and **]** change its radius).\n\
             * **Line** draws a straight line; in Wireworld it is the **Wire** tool.\n\
             * **Random** replaces a dragged rectangle by random cells at the chosen density.\n\n\
             Each stroke is one undo step (**Ctrl/Cmd+Z**). Edits made at generation 0 become part of the \
             reset state, as in Golly.",
        )
        .with_action("tool:draw", "Choose the brush"),
        TutorialPage::new(
            "Why rules behave differently",
            "Wolfram (1984) sorted the behaviour of automata started from random states into four classes:\n\n\
             * **Class I** — everything becomes uniform.\n\
             * **Class II** — simple stable or periodic structures.\n\
             * **Class III** — chaotic, aperiodic patterns.\n\
             * **Class IV** — complex localised structures that move and interact.\n\n\
             The classes differ in what happens to a small change. In an elementary rule a change travels at \
             most one cell per step in each direction: class I rules erase it, class II rules keep it local, \
             class III rules spread it through the whole row, and class IV rules carry it along in moving \
             structures. *Properties › Behaviour class* measures the class of an elementary rule from random \
             rows. Rule 30 is Wolfram's standard class III example and rule 110 his class IV example.",
        )
        .with_action("load:wolfram-rule-30;tab:properties", "Open rule 30"),
        TutorialPage::new(
            "The rule table of an elementary automaton",
            "Each cell of a row looks at its left neighbour, itself and its right neighbour: 8 possible \
             arrangements, from `111` to `000`. The rule number written in binary gives the new cell for each \
             of them; rule 90 is `01011010`, the new cell is *left XOR right*. From a single cell it draws the \
             Sierpiński triangle: row *n* has 2 to the power (number of 1s in the binary form of *n*) live \
             cells.\n\n\
             Click the eight boxes in *Tools › Rule* to flip outputs: the new rule applies from the next row, \
             so the diagram changes below the accent line.",
        )
        .with_action("load:wolfram-rule-90;tool:toggle", "Open rule 90"),
        TutorialPage::new(
            "Refractory states: Brian's Brain",
            "Brian's Brain (`B2/S/C3`) has three states: **ready** (dead), **firing** (alive) and \
             **refractory** (dying). A ready cell fires when exactly 2 of its neighbours fire; a firing cell \
             always becomes refractory, and a refractory cell becomes ready again. Only firing cells count as \
             neighbours.\n\n\
             No cell can fire twice in a row, so nothing stands still: a firing cell is refractory one \
             generation later and ready the next. The *domino*, two firing cells beside two refractory ones, \
             is a spaceship: it moves one cell per generation towards its firing side.",
        )
        .with_action("load:brians-brain", "Open Brian's Brain"),
        TutorialPage::new(
            "Logic in Wireworld",
            "Wireworld cells are **empty**, **conductor** (orange), **electron head** (blue) or **electron \
             tail** (white). A head becomes a tail, a tail becomes a conductor, and a conductor becomes a head \
             when 1 or 2 of its neighbours are heads. A head followed by its tail is an electron: it moves one \
             cell per generation along a wire, away from its tail.\n\n\
             The scenario holds, from the top: a **diode** (an electron passes left to right and arrives at \
             generation 9; from the right it is absorbed), clocks of **period 8** and **12**, an **OR** gate, \
             an **XOR** gate (two simultaneous inputs cancel) and McIntosh's **AND** gate (one pulse only when \
             both inputs arrive). Step with **N** and follow the electrons.",
        )
        .with_action("load:wireworld-circuits", "Open the circuits"),
        TutorialPage::new(
            "Build a circuit",
            "Choose **Wire** (key 4) and drag to lay conductor. Then choose **Paint**, pick *electron tail* \
             and *electron head* with **,** and **.**, and paint a tail next to a head on the wire: the \
             electron runs away from its tail. **Circuit** (key 5) stamps the bundled diode, clocks and gates; \
             **Q** and **E** turn them.",
        )
        .with_action("tool:line", "Choose the wire tool"),
        TutorialPage::new(
            "Advanced example: Langton's ant builds a highway",
            "Langton's ant (rule `RL`) stands on a grid of colour 0 cells (Langton's *white*). On colour 0 it \
             turns right, on colour 1 (*black*) left; it then advances the cell to the next colour and steps \
             forward.\n\n\
             For about 10,000 steps it wanders chaotically. From step **9977** the colours it reads repeat \
             every **104** steps: it moves 2 cells diagonally per period and leaves a straight **highway**, \
             12 more coloured cells per period. At step 10,000 there are **720** coloured cells, at step 11,000 \
             **834**.\n\n\
             Run to step 11,000 (the scenario advances 10 steps per tick) and keep *Follow the first ant* on.",
        )
        .with_action("load:langtons-ant-highway;step:1100", "Run to step 11,000"),
        TutorialPage::new(
            "Reading the metrics",
            "The *Metrics* tab plots what the model measures after every tick:\n\n\
             * Life-like: **population**, **births** and **deaths** (summed over the generations of the tick), \
             **density**, bounding box.\n\
             * Generations: **alive** (state 1), **dying** (states 2 and above), births, deaths.\n\
             * Wireworld: **heads**, **tails** and **conductors**.\n\
             * Ants: **coloured cells** (colour other than 0), **ants**, **steps**.\n\
             * Elementary: **ones** and **density** of the newest row — one point per row.\n\n\
             *Properties › Cells in view* counts the states in the displayed region; the bottom bar shows the \
             tick and the measured speed (and the population of Life-like rules).",
        )
        .with_action("tab:metrics", "Show the metrics"),
        TutorialPage::new(
            "Experiments to try",
            "* **Life without Death**: watch the population plot — it never goes down, because no cell ever \
             dies.\n\
             * **Seeds**: no cell survives, so each generation consists only of newborn cells. Compare births \
             with the population in *Metrics*.\n\
             * **Day & Night**: draw a dead hole into a live area; it behaves like a live blob in a dead area.\n\
             * **Rule 30**: flip one box of the rule table and see whether the measured class changes.\n\
             * **Ants**: add a second ant to the highway, or try `LLRR`, whose figure keeps becoming \
             mirror-symmetric.\n\
             * **Wireworld**: erase one input electron (head and tail) of the XOR gate and watch the output \
             pulse appear.",
        )
        .with_action("load:life-without-death;play", "Start Life without Death"),
    ];
    Tutorial { pages, concepts: CONCEPTS.to_string() }
}

/// The concepts page (Markdown).
pub const CONCEPTS: &str = "## Rule notations\n\n\
| Family | Example | Meaning |\n|---|---|---|\n\
| Life-like | `B36/S23` | born with 3 or 6 live neighbours, survives with 2 or 3 |\n\
| Life-like, S/B form | `23/36` | the same rule, survival first |\n\
| Generations | `B2/S/C3` = `/2/3` | Brian's Brain: born with 2, never survives, 3 states |\n\
| Turmite | `LLRR` | one turn per colour: left, left, right, right |\n\
| Elementary | `110` | the outputs for neighbourhoods 111 ... 000 are 0 1 1 0 1 1 1 0 |\n\n\
## Neighbourhoods\n\n\
Life-like, Generations and Wireworld rules count the **8 surrounding cells** (Moore neighbourhood). \
Generations count only cells in state 1, Wireworld only electron heads. Elementary automata use the cell and \
its two neighbours in the row. An ant reads only the cell under it.\n\n\
## Generations\n\n\
States: 0 dead, 1 alive, 2 to C − 1 dying. A dead cell is born when its number of live neighbours is in B; a \
live cell stays alive when it is in S, otherwise it moves to state 2 (or dies when C = 2); a dying cell moves \
one state on each generation and becomes dead after state C − 1. Dying cells can neither be born again nor \
count as neighbours.\n\n\
## Wolfram's classes\n\n\
Class I: uniform. Class II: stable or periodic structures. Class III: chaos. Class IV: complex interacting \
structures (rule 110, proved Turing-complete by Cook in 2004). The lab's measurement runs 3 random rows on a \
ring of 199 cells for 1200 steps: uniform rows give class I, rows that repeat (up to a shift) class II, \
anything else class III or IV.\n\n\
Equivalent rules behave the same up to a mirror image or swapping 0 and 1: rule 30 is equivalent to 86, 135 \
and 149; rule 110 to 124, 137 and 193.\n\n\
## Worlds\n\n\
* **Bounded plane**: cells outside the rectangle are always dead.\n\
* **Torus**: opposite edges are joined.\n\
* **Klein bottle**: left and right edges are joined; crossing the top or bottom edge also mirrors left and \
right. **Cross-surface**: both pairs of edges are joined with a mirror. (Life-like, Generations and \
Wireworld.)\n\
* **Infinite plane**: 64-bit coordinates; the camera can travel anywhere. (Life-like rules and ants.)\n\n\
Elementary rows have fixed-0, fixed-1 or periodic (ring) edges. Change worlds in the *Parameters* tab; size \
and topology changes apply on reset.\n\n\
## Display\n\n\
Colourings: **States**, **States + trail** (recently changed empty cells glow), **Age** (Life-like with age \
tracking) and **Activity heat map**. When zoomed out so far that a texel stands for a block of cells, \
two-state rules (Life-like and elementary) show the **fraction of live cells** of the block as brightness, \
and multi-state rules show the **most frequent non-zero state** of the block; the overlay says when this \
happens. The lab uses one texel per physical pixel, so on high-density displays single cells stay visible \
down to one cell per physical pixel.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{LabModel, LabSim};
    use cellular_automata::scenario::builtin_scenario;
    use cellular_automata::{GenerationsSim, PatternLibrary};
    use simulation_core::Simulation;

    fn sim(id: &str) -> LabSim {
        crate::sim::tests::lab(id)
    }

    #[test]
    fn every_action_parses_and_loads_a_bundled_scenario() {
        let t = tutorial();
        assert!(t.pages.len() >= 10);
        for page in &t.pages {
            let Some(action) = &page.action else { continue };
            let steps = TutorialCommand::parse_all(&action.id).unwrap_or_else(|| panic!("bad action {}", action.id));
            for step in steps {
                if let TutorialCommand::Load(id) = step {
                    assert!(builtin_scenario(&id).is_ok(), "{id}");
                }
            }
        }
        assert_eq!(TutorialCommand::parse("step:0"), None);
        assert_eq!(TutorialCommand::parse("wolfram:300"), None);
        assert_eq!(
            TutorialCommand::parse_all("load:seeds; tab:properties"),
            Some(vec![TutorialCommand::Load("seeds".into()), TutorialCommand::Tab(RightPanelTab::Properties)])
        );
        assert_eq!(TutorialCommand::parse_all("load:seeds;nope"), None);
    }

    #[test]
    fn highlife_replicator_has_8_copies_at_generation_84() {
        let mut s = sim("highlife-replicator");
        let mut populations = Vec::new();
        for _ in 0..84 {
            s.step();
            populations.push(s.metrics().get("population").unwrap());
        }
        assert_eq!(populations[11], 24.0, "2 copies at generation 12");
        assert_eq!(populations[35], 48.0, "4 copies at generation 36");
        assert_eq!(populations[83], 96.0, "8 copies at generation 84");
    }

    #[test]
    fn seeds_cells_never_survive() {
        let mut s = sim("seeds");
        for _ in 0..30 {
            s.step();
            let m = s.metrics();
            assert_eq!(m.get("births"), m.get("population"), "every live cell is newborn");
        }
    }

    #[test]
    fn life_without_death_never_shrinks() {
        let mut s = sim("life-without-death");
        let mut last = s.metrics().get("population").unwrap();
        for _ in 0..40 {
            s.step();
            let p = s.metrics().get("population").unwrap();
            assert!(p >= last);
            last = p;
        }
    }

    #[test]
    fn brians_brain_cells_fire_once_and_the_domino_moves() {
        let mut s = sim("brians-brain");
        s.step();
        let LabModel::Generations(before) = s.model() else { panic!() };
        let firing: Vec<(i64, i64)> = before.cells().into_iter().filter(|c| c.2 == 1).map(|c| (c.0, c.1)).collect();
        let refractory: Vec<(i64, i64)> = before.cells().into_iter().filter(|c| c.2 == 2).map(|c| (c.0, c.1)).collect();
        s.step();
        let LabModel::Generations(after) = s.model() else { panic!() };
        assert!(firing.iter().all(|&(x, y)| after.cell(x, y) == 2), "firing cells become refractory");
        assert!(refractory.iter().all(|&(x, y)| after.cell(x, y) == 0), "refractory cells become ready");
        // The domino alone: two firing cells beside two refractory ones.
        let config = cellular_automata::GenerationsConfig {
            init: vec![cellular_automata::StateInit::Pattern {
                source: cellular_automata::PatternSource::Text("x = 2, y = 2, rule = /2/3\nBA$BA!".into()),
                x: 10,
                y: 10,
                rotation: cellular_automata::Rotation::None,
                mirror: false,
            }],
            ..Default::default()
        };
        let mut domino = GenerationsSim::new(&config, 0, &PatternLibrary::builtin()).unwrap();
        for k in 1..=5i64 {
            domino.step();
            assert_eq!(domino.cell(11 + k, 10), 1, "moves one cell per generation towards its firing side");
            assert_eq!(domino.cell(10 + k, 11), 2);
            assert_eq!(domino.cells().len(), 4);
        }
    }

    #[test]
    fn wireworld_diode_delivers_at_generation_9() {
        // The diode is placed at (2, 2); its output end is x = 10 of the pattern, row 1.
        let mut s = sim("wireworld-circuits");
        let LabModel::Wireworld(w) = s.model() else { panic!() };
        assert_eq!(w.cell(3, 3), 1, "head of the diode's electron");
        for generation in 1..=9 {
            s.step();
            let LabModel::Wireworld(w) = s.model() else { panic!() };
            assert_eq!(w.cell(12, 3) == 1, generation == 9, "generation {generation}");
        }
    }

    #[test]
    fn langtons_ant_counts() {
        let mut s = sim("langtons-ant-highway");
        for _ in 0..1000 {
            s.step();
        }
        assert_eq!(s.tick(), 10_000);
        assert_eq!(s.metrics().get("colored"), Some(720.0));
        for _ in 0..100 {
            s.step();
        }
        assert_eq!(s.metrics().get("colored"), Some(834.0));
    }

    #[test]
    fn rule_90_rows_follow_gould() {
        let mut s = sim("wolfram-rule-90");
        for n in 1..=64u32 {
            s.step();
            assert_eq!(s.metrics().get("ones"), Some(f64::from(1u32 << n.count_ones())), "row {n}");
        }
        assert_eq!(format!("{:08b}", 90), "01011010");
        assert_eq!(format!("{:08b}", 110), "01101110");
    }
}
