//! The mode's tutorial (steps with actions that load real catalog patterns,
//! select tools and run analyses) and its concepts page, and the handling of
//! tutorial actions.
//!
//! Facts quoted in the text come from the pattern catalog
//! (`assets/patterns/catalog.toml`, whose numbers were measured by
//! simulation), from `docs/models/cellular-automata.md` and
//! `docs/HASHLIFE.md`, or are checked by this module's tests against the
//! model (blinker, filled rectangle, Gosper gun at generation 2^20, the
//! automatic engine choice).

use bevy::prelude::*;
use interface::{RightPanelTab, ShellCommand, Tutorial, TutorialAction, TutorialPage};

use crate::commands::{ModeCommand, ModeCommands, TimeSettings};
use crate::library::ConwayLibrary;
use crate::session::{ConwayRunner, LoadRequest, LoadRequests};
use crate::style::RenderStyle;
use crate::tools::{StampPattern, StampSource, Tool, ToolSettings};
use crate::window::{PatternsTab, PatternsWindow};

/// What a tutorial button asks the mode to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TutorialCommand {
    /// A library pattern alone on an empty infinite plane.
    Pattern(String),
    /// A Classic Conway scenario.
    Scenario(String),
    /// Select a tool.
    Tool(Tool),
    /// A library pattern as the stamp.
    Stamp(String),
    /// A render style.
    Style(RenderStyle),
    /// A tab of the right panel.
    Tab(RightPanelTab),
    /// Start running.
    Play,
    /// Run the period analysis.
    Analyse,
    /// Hashlife jump of 2^k generations.
    Jump(u32),
    /// Generations per tick.
    GenerationsPerTick(u64),
    /// Open the pattern library.
    Library,
}

impl TutorialCommand {
    /// Parses an action id: `pattern:<id>`, `scenario:<id>`, `tool:<name>`,
    /// `stamp:<id>`, `style:<name>`, `tab:<name>`, `play`, `analyse`,
    /// `jump:<k>`, `gpt:<n>`, `library`.
    pub fn parse(action: &str) -> Option<Self> {
        let (verb, arg) = action.split_once(':').unwrap_or((action, ""));
        Some(match verb {
            "pattern" if !arg.is_empty() => Self::Pattern(arg.into()),
            "scenario" if !arg.is_empty() => Self::Scenario(arg.into()),
            "stamp" if !arg.is_empty() => Self::Stamp(arg.into()),
            "tool" => Self::Tool(match arg {
                "draw" => Tool::Draw,
                "erase" => Tool::Erase,
                "line" => Tool::Line,
                "rect" => Tool::Rectangle,
                "fill" => Tool::Fill,
                "select" => Tool::Select,
                "stamp" => Tool::Stamp,
                _ => return None,
            }),
            "style" => Self::Style(match arg {
                "classic" => RenderStyle::Classic,
                "age" => RenderStyle::Age,
                "trails" => RenderStyle::Trails,
                "heat" => RenderStyle::Heat,
                "density" => RenderStyle::Density,
                _ => return None,
            }),
            "tab" => Self::Tab(match arg {
                "properties" => RightPanelTab::Properties,
                "parameters" => RightPanelTab::Parameters,
                "rules" => RightPanelTab::Rules,
                "metrics" => RightPanelTab::Metrics,
                _ => return None,
            }),
            "play" => Self::Play,
            "analyse" => Self::Analyse,
            "library" => Self::Library,
            "jump" => Self::Jump(arg.parse().ok().filter(|k| (1..=40).contains(k))?),
            "gpt" => Self::GenerationsPerTick(arg.parse().ok().filter(|n| *n >= 1)?),
            _ => return None,
        })
    }
}

/// Executes tutorial actions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tutorial_actions(
    mut actions: MessageReader<TutorialAction>,
    mut loads: ResMut<LoadRequests>,
    mut queue: ResMut<ModeCommands>,
    mut tools: ResMut<ToolSettings>,
    mut time: ResMut<TimeSettings>,
    mut window: ResMut<PatternsWindow>,
    mut library: ResMut<ConwayLibrary>,
    mut shell: MessageWriter<ShellCommand>,
    runner: Option<Res<ConwayRunner>>,
) {
    for action in actions.read() {
        let Some(command) = TutorialCommand::parse(&action.0) else {
            warn!("unknown tutorial action `{}`", action.0);
            continue;
        };
        match command {
            TutorialCommand::Pattern(id) => loads.0.push(LoadRequest::Pattern(id)),
            TutorialCommand::Scenario(id) => loads.0.push(LoadRequest::Scenario(id)),
            TutorialCommand::Tool(tool) => tools.tool = tool,
            TutorialCommand::Stamp(id) => match library.pattern(&id) {
                Ok(pattern) => {
                    let name = library.entry(&id).map_or_else(|| id.clone(), |e| e.name.clone());
                    queue.0.push(ModeCommand::UseStamp(StampPattern {
                        name,
                        source: StampSource::Library(id),
                        pattern,
                    }));
                }
                Err(e) => warn!("tutorial stamp `{id}`: {e}"),
            },
            TutorialCommand::Style(style) => queue.0.push(ModeCommand::SetStyle(style)),
            TutorialCommand::Tab(tab) => {
                shell.write(ShellCommand::ShowRightTab(tab));
            }
            TutorialCommand::Play => {
                if let Some(runner) = &runner {
                    runner.0.runner().play();
                }
            }
            TutorialCommand::Analyse => {
                queue.0.push(ModeCommand::Analyse);
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
            }
            TutorialCommand::Jump(k) => {
                time.jump_log2 = k;
                queue.0.push(ModeCommand::Jump(k));
            }
            TutorialCommand::GenerationsPerTick(n) => queue.0.push(ModeCommand::SetGenerationsPerTick(n)),
            TutorialCommand::Library => window.open(PatternsTab::Library),
        }
    }
}

/// Population of the Gosper glider gun alone on the plane at generation
/// 2^20 (checked by a test against the model).
pub const GUN_POPULATION_AT_2_POW_20: u64 = 174_804;

/// Tutorial steps and concepts of the mode.
pub fn tutorial() -> Tutorial {
    let pages = vec![
        TutorialPage::new(
            "The Game of Life",
            "John Conway's **Game of Life** (published by Martin Gardner in *Scientific American*, October 1970) is \
             a cellular automaton: an infinite grid of square cells, each **alive** or **dead**, updated together in \
             discrete **generations**.\n\n\
             Every cell looks at its 8 neighbours (the Moore neighbourhood). With *n* live neighbours:\n\n\
             * a dead cell is **born** when *n* = 3;\n\
             * a live cell **survives** when *n* = 2 or 3;\n\
             * every other cell is dead in the next generation (isolation or overcrowding).\n\n\
             The rule is written **B3/S23**. Nothing else is needed: everything you will see in this laboratory \
             follows from these three lines.",
        )
        .with_action("tab:rules", "Show the formal rules"),
        TutorialPage::new(
            "Reading the rule: the blinker",
            "Load a **blinker**, three live cells in a row, alone on an empty plane.\n\n\
             * The middle cell has 2 live neighbours: it survives.\n\
             * The two end cells have only 1: they die.\n\
             * The dead cells directly above and below the middle have exactly 3: they are born.\n\n\
             The row turns into a column, and the column back into a row: an **oscillator of period 2**. Press \
             **N** to advance one generation at a time and check every count yourself (hover a cell to read its \
             coordinates).",
        )
        .with_action("pattern:blinker", "Load a blinker"),
        TutorialPage::new(
            "Draw your own cells",
            "Pick the **Draw** tool (key **1**) and drag in the world: every stroke is one undo step \
             (**Ctrl/Cmd+Z**). **Erase** (2) kills cells; **Line** (3) and **Rect** (4) draw shapes; **Fill** (5) \
             drops a random soup of the chosen density into a rectangle.\n\n\
             Try a 100 % fill of a small rectangle and press **N** once: every interior cell has 8 live \
             neighbours and dies, every edge cell except the corners has 5 and dies, the four corners have 3 and \
             survive, and a new row is born just outside each edge, where dead cells see exactly 3 neighbours.\n\n\
             Right-drag pans, the wheel zooms towards the pointer, **F** frames the pattern.",
        )
        .with_action("tool:draw", "Select the Draw tool"),
        TutorialPage::new(
            "Still lifes",
            "A **still life** never changes: every live cell has 2 or 3 live neighbours and no dead cell has \
             exactly 3. Common small ones are the **block** (4 cells), the **tub** (4), the **boat** (5), the \
             **beehive** (6) and the **loaf** (7); the block and the beehive are the two most common still lifes \
             in the ash of random soups.\n\n\
             Load a beehive and run it: nothing moves. Then press **Analyse** in *Properties › Period*: the \
             analysis runs a copy of the state and reports period 1.",
        )
        .with_action("pattern:beehive", "Load a beehive"),
        TutorialPage::new(
            "Oscillators",
            "An **oscillator** returns to its starting state after a fixed number of generations, its **period**. \
             Blinker, toad and beacon have period 2, the **pulsar** period 3 (it cycles through 48, 56 and 72 \
             cells) and the **pentadecathlon** period 15.\n\n\
             Load the pulsar, step with **N**, then measure its period with the button below (the result appears \
             in *Properties › Period*).",
        )
        .with_action("pattern:pulsar", "Load the pulsar"),
        TutorialPage::new(
            "Measure a period",
            "The analysis copies the current state, steps the copy up to 4,096 generations (4 seconds at most) and \
             reports the first return to the starting state. On the infinite plane it ignores translation, so it \
             also finds spaceships and their displacement. The live run is never touched.",
        )
        .with_action("analyse", "Analyse the current pattern"),
        TutorialPage::new(
            "Spaceships",
            "A **spaceship** returns to its own shape displaced. The **glider** (Richard Guy, 1970) moves one cell \
             diagonally every 4 generations: its speed is written **c/4**, where *c*, the \"speed of light\", is one \
             cell per generation, the fastest any influence can travel. The lightweight, middleweight and \
             heavyweight spaceships move 2 cells every 4 generations, **c/2** orthogonally.\n\n\
             Load a glider, press **Space**, and use **Analyse** to read period 4 and displacement (1, 1).",
        )
        .with_action("pattern:glider", "Load a glider"),
        TutorialPage::new(
            "Place patterns from the library",
            "The **pattern library** (key **P**) holds 80 patterns from the asset catalog, each with its \
             discoverer, year, period, speed, population, bounding box, measured behaviour and licence. **Use as \
             stamp** selects the **Stamp** tool: a ghost follows the pointer, **Q** / **E** rotate it, **M** \
             mirrors it, and a click places it (one undo step).\n\n\
             The button below makes the LWSS your stamp. Click a few times in the world, then run.",
        )
        .with_action("stamp:lwss", "Stamp a lightweight spaceship"),
        TutorialPage::new(
            "Guns",
            "A **gun** is an oscillator that emits spaceships. Bill Gosper's **glider gun** (1970), the first gun \
             and the first finite pattern known to grow forever, emits one glider every 30 generations, so its \
             population grows without bound.\n\n\
             This scenario shows the gun next to its mirror image: the two glider streams diverge and never meet. \
             Watch the population plot climb in steps in *Properties*.",
        )
        .with_action("scenario:glider-gun-showcase", "Load the gun showcase"),
        TutorialPage::new(
            "Methuselahs",
            "A **methuselah** is a small pattern that takes very long to settle. The **R-pentomino** (5 cells) \
             stabilises at generation **1103** with **116** cells, including 6 gliders that fly away. The **acorn** \
             (7 cells) needs **5206** generations and ends with 633 cells; **diehard** vanishes completely at \
             generation **130**.\n\n\
             Load the R-pentomino, choose 16 generations per tick in *Tools › Time* and press **Space**. The *Age* \
             style (*Tools › View › Style*; **V** cycles the styles) colours long-lived ash.",
        )
        .with_action("pattern:r-pentomino", "Load the R-pentomino"),
        TutorialPage::new(
            "Topologies",
            "Conway's grid is infinite; this laboratory also offers finite worlds of W × H cells:\n\n\
             * **Bounded plane**: everything outside is permanently dead; gliders die at the walls.\n\
             * **Torus**: opposite edges are joined.\n\
             * **Klein bottle**: left and right are joined, and crossing the top or bottom edge also mirrors x.\n\
             * **Cross-surface**: crossing the top or bottom edge mirrors x, crossing the left or right edge \
               mirrors y.\n\
             * **Infinite plane**: 64-bit coordinates in every direction.\n\n\
             This scenario puts the R-pentomino in a 128 × 128 box, where it evolves differently from the \
             unbounded one. Change *Topology* in the Parameters tab and press **R** to compare.",
        )
        .with_action("scenario:bounded-r-pentomino", "Load the R-pentomino in a box"),
        TutorialPage::new(
            "Engines and level of detail",
            "Four engines compute identical generations: **Dense** (a byte per cell), **Bit-packed** (64 cells per \
             machine word), **Sparse tiles** (64 × 64 tiles stepped only where something changed) and **Hashlife**. \
             *Automatic* picks Dense for finite worlds of at most 16,384 cells (128 × 128), Bit-packed for larger ones, \
             Hashlife on the infinite plane for ticks of 16 or more generations (a power of two) or very large \
             regular patterns, and Sparse otherwise. The *Engine* card in Properties says which one runs and why.\n\n\
             Zoomed out, the renderer asks the model only for the visible region, with one texel for 2^k × 2^k \
             cells; Hashlife builds such views from the cached populations of its quadtree squares, so even \
             astronomically large patterns draw quickly.",
        )
        .with_action("tab:properties", "Show the Engine card"),
        TutorialPage::new(
            "Why Hashlife is fast",
            "Hashlife (Bill Gosper, 1984) stores the plane as a quadtree in which **every distinct square exists \
             once** (hash-consing): a stream of identical gliders, or a gun repeated a thousand times, costs memory \
             for its distinct squares only. For each square of side 2^L it **memoises** its central square of side \
             2^(L-1) up to 2^(L-2) generations later, which depends only on that square because nothing travels \
             faster than one cell per generation. A repeated square is never recomputed, so for regular patterns the time to jump \
             2^k generations grows roughly with k, not with 2^k.\n\n\
             Load the Gosper gun alone on the plane; the next step jumps it far ahead.",
        )
        .with_action("pattern:gosper-glider-gun", "Load the Gosper gun"),
        TutorialPage::new(
            "Jump ahead",
            "This jumps **2^20 = 1,048,576 generations** in one memoised step (*Tools › Time › Hashlife jump*). \
             Afterwards the gun has fired about 35,000 gliders and the population is **174,804** cells, yet the \
             *Engine* card shows far fewer distinct quadtree nodes than live cells: the glider stream is made of \
             the same few squares. The crate's benchmark measured this jump from a cold memo at about half a \
             millisecond on an Apple M2 Max.\n\n\
             Undo (**Ctrl/Cmd+Z**) returns to the generation before the jump.",
        )
        .with_action("jump:20", "Jump 2^20 generations"),
        TutorialPage::new(
            "When Hashlife is slow",
            "Chaos defeats memoisation: in a random soup almost no square repeats, so Hashlife pays hash-table \
             work for every active 16 × 16 square each generation, tens of times slower than a bit-packed grid of \
             the same area. That is why *Automatic* never picks Hashlife for single-generation ticks of small or \
             irregular patterns. (Finite worlds always use the dense or bit-packed engines: Hashlife and the \
             sparse tiles run only the infinite plane.)\n\n\
             This scenario runs a 2048 × 2048 torus (about 4.2 million cells) with the bit-packed engine.",
        )
        .with_action("scenario:large-bit-soup", "Load the large soup"),
        TutorialPage::new(
            "Metrics",
            "The *Metrics* tab records after every tick, and *Properties* shows:\n\n\
             * **population**: live cells (shown exactly, however large);\n\
             * **births / deaths**: cells that became alive or died during the last tick, summed over its \
               generations (Hashlife jumps without tracking do not count them);\n\
             * **bounding box**: the smallest rectangle holding every live cell;\n\
             * **density**: population divided by the world area (finite worlds) or the bounding-box area (infinite \
               plane).\n\n\
             *Properties* and the bottom bar also show the **generation rate**: generations per second actually \
             computed. A soup's population falls quickly and then levels off as it burns down to ash.",
        )
        .with_action("scenario:random-soup-torus", "Load a soup on a torus"),
        TutorialPage::new(
            "Experiments",
            "* **Soup densities.** Fill four rectangles at 10 %, 35 %, 60 % and 90 % (Fill tool) and run: which \
               leave the most ash?\n\
             * **Edges.** Run the bounded R-pentomino, then switch Topology to torus and Klein bottle (reset with \
               **R**) and compare the final populations.\n\
             * **Collisions.** Stamp two gliders on crossing paths with different offsets: some collisions make a \
               block, some annihilate.\n\
             * **Eaters.** Open *pattern collisions*: eater 1 absorbs a glider and recovers.\n\
             * **Growth.** Run *Breeder 1 (Hashlife)* with large jumps and watch the population plot curve upwards \
               (quadratic growth).",
        )
        .with_action("scenario:pattern-collisions", "Load the collisions"),
    ];
    Tutorial { pages, concepts: CONCEPTS.to_string() }
}

/// The concepts page (Markdown).
pub const CONCEPTS: &str = "\
## The rule B3/S23

Cells are dead or alive on a square grid; all cells update simultaneously. With *n* live cells among the 8 \
neighbours: a dead cell is born iff *n* = 3, a live cell survives iff *n* = 2 or 3, every other cell is dead in \
the next generation.

## Kinds of objects

| Kind | Behaviour | Classic example |
|---|---|---|
| Still life | unchanged every generation | block, beehive, loaf, boat, tub |
| Oscillator | returns after *p* generations in place | blinker (2), pulsar (3), pentadecathlon (15) |
| Spaceship | returns displaced | glider (c/4 diagonal), LWSS (c/2 orthogonal) |
| Gun | oscillator that emits spaceships | Gosper glider gun (period 30) |
| Puffer | spaceship that leaves debris | puffer train |
| Methuselah | small, settles after a long time | R-pentomino (1103), acorn (5206) |
| Breeder | grows quadratically | Breeder 1 |

**Speed** *c/n* means one cell per *n* generations; *c* (one cell per generation) is the largest speed possible, \
because a cell only sees its immediate neighbours.

## Worlds

| Topology | Edges |
|---|---|
| Bounded plane | outside cells are dead |
| Torus | opposite edges joined |
| Klein bottle | left/right joined; top/bottom joined with x mirrored |
| Cross-surface | top/bottom with x mirrored, left/right with y mirrored |
| Infinite plane | none; 64-bit coordinates |

## Engines

| Engine | Storage | Best for |
|---|---|---|
| Dense | one byte per cell | small finite worlds |
| Bit-packed | 64 cells per word | large finite worlds, soups |
| Sparse tiles | 64 x 64 tiles, only changed areas stepped | infinite plane, local activity |
| Hashlife | hash-consed quadtree with memoised results | regular patterns, huge jumps in time |

All four compute exactly the same generations; switching keeps every cell.

## Hashlife in one paragraph

The plane is a quadtree whose nodes are hash-consed: identical squares are one node. The result of a square of \
side 2^L after 2^(L-2) generations (its centre square) is computed once and memoised; nothing travels faster than \
one cell per generation, so it depends on that square only. Jumps of 2^k generations reuse these results, which \
makes regular patterns (guns, breeders, metapixels) cheap far into the future. Chaotic patterns repeat little and \
are better served by the bit-packed and sparse engines.

## Rendering

The renderer requests only the visible cells. When zoomed out, each texel aggregates 2^k x 2^k cells (alive if \
any cell is alive; the *Density* style shows the fraction). Age and activity styles read channels the model fills \
while *track_ages* / *track_activity* are on: age counts generations alive (255 at most); activity is 255 for a \
cell that changed during the last tick and decreases by 16 per tick.
";

#[cfg(test)]
mod tests {
    use super::*;
    use cellular_automata::{
        CellRect, EngineKind, LifeCommand, LifeConfig, LifeMode, LifeSim, PatternLibrary, World, select_engine,
    };
    use simulation_core::{ParamValue, Simulation, Topology};

    fn plane_with(id: &str) -> LifeSim {
        crate::session::pattern_world(id, &ConwayLibrary::builtin("test".into())).unwrap()
    }

    #[test]
    fn every_action_parses_and_every_scenario_exists() {
        let tutorial = tutorial();
        assert!(tutorial.pages.len() >= 15);
        let scenarios = crate::session::ConwayScenarios::builtin();
        let library = ConwayLibrary::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
        for page in &tutorial.pages {
            let Some(action) = &page.action else { continue };
            let command = TutorialCommand::parse(&action.id).unwrap_or_else(|| panic!("bad action {}", action.id));
            match command {
                TutorialCommand::Scenario(id) => assert!(scenarios.get(&id).is_some(), "{id}"),
                TutorialCommand::Pattern(id) | TutorialCommand::Stamp(id) => {
                    assert!(library.entry(&id).is_some(), "{id}")
                }
                _ => {}
            }
        }
        assert_eq!(TutorialCommand::parse("jump:41"), None);
        assert_eq!(TutorialCommand::parse("tool:rect"), Some(TutorialCommand::Tool(Tool::Rectangle)));
        assert_eq!(TutorialCommand::parse("nope"), None);
    }

    #[test]
    fn the_spaceship_and_still_life_pages_are_right() {
        use crate::analysis::{AnalysisResult, analyse};
        use std::sync::atomic::AtomicBool;
        let budget = std::time::Duration::from_secs(30);
        let mut glider = plane_with("glider");
        assert_eq!(
            analyse(&mut glider, true, &AtomicBool::new(false), budget),
            AnalysisResult::Spaceship { period: 4, dx: 1, dy: 1 }
        );
        assert_eq!(
            analyse(&mut plane_with("beehive"), true, &AtomicBool::new(false), budget),
            AnalysisResult::StillLife
        );
        assert_eq!(
            analyse(&mut plane_with("pulsar"), true, &AtomicBool::new(false), budget),
            AnalysisResult::Oscillator { period: 3 }
        );
    }

    #[test]
    fn the_blinker_page_is_right() {
        let mut sim = plane_with("blinker");
        let before = sim.live_cells();
        sim.step();
        let after = sim.live_cells();
        assert_ne!(before, after);
        assert_eq!(after.len(), 3);
        sim.step();
        assert_eq!(sim.live_cells(), before, "period 2");
    }

    #[test]
    fn the_filled_rectangle_experiment_is_right() {
        let config = LifeConfig { topology: Topology::Infinite, init: vec![], ..LifeConfig::default() };
        let mut sim = LifeSim::new(&config, 1, &PatternLibrary::builtin(), LifeMode::Classic).unwrap();
        let rect = CellRect::new(0, 0, 9, 5);
        sim.apply(LifeCommand::Rectangle { rect, filled: true, alive: true }).unwrap();
        sim.step();
        let cells = sim.live_cells();
        for &(x, y) in &cells {
            let corner = (x == 0 || x == 9) && (y == 0 || y == 5);
            let outside_row = (y == -1 || y == 6) && (1..=8).contains(&x);
            let outside_column = (x == -1 || x == 10) && (1..=4).contains(&y);
            assert!(corner || outside_row || outside_column, "unexpected live cell ({x}, {y})");
        }
        assert_eq!(cells.len(), 4 + 2 * 8 + 2 * 4);
    }

    #[test]
    fn the_jump_page_is_right() {
        let mut sim = plane_with("gosper-glider-gun");
        crate::commands::prepare_jump(&mut sim).unwrap();
        sim.set_param("generations_per_tick", ParamValue::Int(1 << 20)).unwrap();
        sim.step();
        assert_eq!(sim.tick(), 1 << 20);
        assert_eq!(sim.population(), GUN_POPULATION_AT_2_POW_20);
        assert!((34_000..36_000).contains(&((GUN_POPULATION_AT_2_POW_20 - 36) / 5)), "about 35,000 gliders");
        let nodes = sim
            .engine()
            .as_any()
            .downcast_ref::<cellular_automata::HashlifeEngine>()
            .map(|h| h.universe().node_count())
            .expect("Hashlife runs");
        assert!((nodes as u64) < sim.population() / 4, "{nodes} nodes for {} cells", sim.population());
        let text = crate::labels::thousands(u128::from(GUN_POPULATION_AT_2_POW_20));
        assert!(tutorial().pages.iter().any(|p| p.body.contains(&text)), "the page quotes {text}");
    }

    #[test]
    fn the_engine_page_matches_the_automatic_choice() {
        let rule = patterns::LifeRule::conway();
        let pick = |topology, w, h, generations, population, regularity| {
            let world = World::new(topology, w, h).unwrap();
            select_engine(&cellular_automata::SelectionInput {
                world,
                rule,
                population,
                generations_per_tick: generations,
                regularity,
            })
            .unwrap()
        };
        assert_eq!(pick(Topology::Torus, 128, 128, 1, 0, None), EngineKind::Dense);
        assert_eq!(pick(Topology::Torus, 129, 128, 1, 0, None), EngineKind::Bit);
        assert_eq!(pick(Topology::Infinite, 64, 64, 16, 10, None), EngineKind::Hashlife);
        assert_eq!(pick(Topology::Infinite, 64, 64, 8, 10, None), EngineKind::Sparse);
        assert_eq!(pick(Topology::Torus, 2048, 2048, 1, 0, None), EngineKind::Bit, "the large soup page");
        // 2048 x 2048 = 4,194,304 cells ("about 4.2 million").
        assert_eq!(2048u64 * 2048, 4_194_304);
        assert_eq!(cellular_automata::engine::AUTO_DENSE_MAX_CELLS, 16_384);
        assert_eq!(pick(Topology::Torus, 256, 64, 1, 0, None), EngineKind::Dense, "16,384 cells, not 128 wide");
        assert!(tutorial().pages.iter().any(|p| p.body.contains("at most 16,384 cells (128 × 128)")));
    }

    #[test]
    fn methuselah_and_oscillator_numbers_match_the_catalog() {
        let library = ConwayLibrary::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
        let extra = |id: &str, key: &str| library.entry(id).and_then(|e| e.extra.get(key).cloned());
        assert_eq!(extra("r-pentomino", "stabilizes_at").as_deref(), Some("1103"));
        assert_eq!(extra("r-pentomino", "final_population").as_deref(), Some("116"));
        assert_eq!(extra("acorn", "stabilizes_at").as_deref(), Some("5206"));
        assert_eq!(extra("acorn", "final_population").as_deref(), Some("633"));
        assert_eq!(extra("diehard", "dies_at").as_deref(), Some("130"));
        assert_eq!(library.entry("pulsar").and_then(|e| e.period), Some(3));
        assert_eq!(library.entry("pentadecathlon").and_then(|e| e.period), Some(15));
        assert_eq!(library.entry("gosper-glider-gun").and_then(|e| e.period), Some(30));
        assert_eq!(library.entry("glider").and_then(|e| e.year), Some(1970));
        assert_eq!(library.catalog.len(), 80);
        for (id, n) in [("block", 4), ("tub", 4), ("boat", 5), ("beehive", 6), ("loaf", 7)] {
            assert_eq!(library.entry(id).and_then(|e| e.population), Some(n), "{id}");
        }
    }
}
