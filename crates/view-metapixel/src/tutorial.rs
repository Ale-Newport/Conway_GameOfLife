//! The tutorial of the mode (steps with actions, and a concepts page) and
//! the handling of its action buttons. Every number here comes from the
//! model (`metapixel::spec`, the scenarios) or from the validation runs
//! documented in `docs/OTCA_METAPIXEL.md`.

use bevy::prelude::*;
use interface::{RightPanelTab, ShellCommand, Tutorial, TutorialAction, TutorialPage};
use metapixel::StepSize;
use simulation_core::{Control, ParamValue, Speed};

use crate::macro_layer::{LayerMode, MacroSource};
use crate::panels::properties::PropertiesTab;
use crate::session::{Lifecycle, LoadRequests, MpSession, SimData};
use crate::tools::{Tool, ToolState, rule_table_rect};
use crate::world::{CameraFit, ViewSettings, current_shape};

/// The mode's tutorial.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "What a metapixel is",
                "Brice Due's **OTCA metapixel** (Outer Totalistic Cellular Automata Meta-Pixel, built in 2005 and \
                 2006) is a pattern of Conway's Game of Life that behaves like **one cell of another Life-like \
                 automaton**. Each tile is 2,048 × 2,048 cells; tiles are laid side by side at a pitch of 2,048 \
                 cells, exchange signals with their neighbours and, every **35,328 generations**, each one shows the \
                 state its programmed rule gives it.\n\n\
                 Nothing here is drawn or faked: every tile is a region of **one** B3/S23 pattern. The meta-blinker \
                 below is 342,076 live cells at t = 0, every one of them following Conway's rule.",
            )
            .with_action("load:metapixel.blinker", "Load the meta-blinker"),
            TutorialPage::new(
                "One simulation, two layers",
                "The world shows the same pattern at two levels:\n\n\
                 - **Physical cells** (exact): the B3/S23 cells of the construction, read from Hashlife's quadtree. \
                 Zoomed out, each texel summarises a power-of-two block of cells; its colour is the fraction of live \
                 cells in it.\n\
                 - **Macro layer** (readout): one big cell per tile, ON or OFF, read from the physical cells by \
                 counting the live cells in each tile's core.\n\n\
                 In **Auto** the layers blend with the zoom: only physical cells while a tile spans at least 128 \
                 pixels, only the macro layer at 32 pixels or less. The badge in the top-left corner of the world \
                 always names the layer you see. Keys: **L** cycles the layer, **F** frames the grid.",
            )
            .with_action("zoom:tile", "Frame the centre tile"),
            TutorialPage::new(
                "How a tile encodes a rule",
                "The rule is part of the pattern. In the lower left of every tile sits a table of **18 eaters**, one \
                 for each digit B0 to B8 and S0 to S8. Unprogrammed, every eater is broken; adding **one cell** \
                 completes it, and a complete eater in row *d* puts digit *d* in the rule. Conway's rule B3/S23 adds \
                 exactly three cells to every tile: B3 at (148, 1470), S2 at (162, 1498) and S3 at (162, 1472), in \
                 tile coordinates.\n\n\
                 Which neighbours a tile hears is set by **eight fuse objects** on its edges and corners: removing \
                 an edge block makes every tile deaf to that edge; removing a corner LWSS makes it deaf to the \
                 **opposite** corner. Zoomed in, programmed eaters are outlined in green and broken ones in grey.",
            )
            .with_action("zoom:rules", "Show the rule table"),
            TutorialPage::new(
                "Why 35,328 generations",
                "One metageneration is **P = 35,328 = 2^9 · 3 · 23** generations: the period of the tile's own \
                 clock. In Brice Due's description of the design, a tile spends each cycle hearing its neighbours, \
                 counting them, looking the count up in its rule table, latching its new state and redrawing its \
                 1,720 × 1,720 display. The signals doing this work are gliders and spaceships, which in Life travel \
                 at most c/2 orthogonally (gliders c/4 diagonally), so a signal needs at least 4,096 generations just \
                 to cross one 2,048-cell tile. The live-cell count of a steady ON core repeats with period 46, which \
                 divides P (35,328 = 768 × 46).\n\n\
                 Measured timing (Golly's bgolly, reproduced by this model's tests): metagen *n* is read at \
                 **t = (n + 1) · P**, when every core holds exactly 0 (OFF) or 29,795 (ON) live cells. From the \
                 second cycle on, displays switch while **t mod P** is between **4,320 and 7,407**: a dying core \
                 leaves its steady state at 4,320 and is empty from 7,362; a new one gets its first cell at 4,366 \
                 and is steady from 7,408. So the display shows m(t) = max(0, floor((t - 7,408) / P)).",
            )
            .with_action("watch:transition", "Watch the next transition"),
            TutorialPage::new(
                "How Hashlife makes this feasible",
                "One metagen of the 15 × 15 galaxy is 35,328 generations of a 30,730 × 30,730-cell plane: about 33 \
                 trillion cell updates if computed cell by cell. **Hashlife** (Gosper, 1984) stores the plane as a \
                 quadtree in which identical squares are one shared node, and memoises the future of every square: \
                 the centre of a level-L node (2^L cells on a side) is advanced up to 2^(L-2) generations in one \
                 step and the result is stored with the node.\n\n\
                 Metapixel tilings are extremely repetitive (every tile is one of two patterns, and every distinct \
                 2,048 × 2,048 square is built once), so results are reused across tiles and cycles; later metagens \
                 cost far less than the first. One metagen is advanced as three power-of-two jumps, 2^9 + 2^11 + \
                 2^15. The Time card shows what each tick costs; the simulation runs on its own thread, so the \
                 window never waits for it.",
            )
            .with_action("step:metagen", "Advance one metagen"),
            TutorialPage::new(
                "What you see at each zoom level",
                "- **Cells** (a cell spans several pixels): individual B3/S23 cells; grid lines appear.\n\
                 - **Inside a tile** (one tile fills the view): glider and spaceship traffic, the readout core and \
                 the display (outlined for the pinned or hovered tile); zoom in further to see the rule table and \
                 the fuses outlined. Texels summarise blocks of cells until a cell spans a pixel.\n\
                 - **Tiles** (a tile spans 32 to 128 pixels): the physical view blended with the macro layer.\n\
                 - **Grid** (a tile below 32 pixels): only the macro layer, one big cell per tile.\n\n\
                 The mini map in the Inspector shows the whole macro grid and your view (white frame).",
            )
            .with_action("zoom:grid", "Frame the whole grid"),
            TutorialPage::new(
                "Simple example: the meta-blinker",
                "Three ON metacells in a row inside a 3 × 3 grid. Under the macro rule B3/S23 the row flips between \
                 horizontal and vertical every metagen, exactly like a blinker, computed by 342,076 live cells at \
                 t = 0. Golly's bgolly gives population 330,244 at t = 35,328; the Validation tab compares the run \
                 with this golden table as it reaches each readout. One tick is one metagen in this scenario.",
            )
            .with_action("example:blinker", "Load and run the meta-blinker"),
            TutorialPage::new(
                "Advanced example: Kok's galaxy",
                "Kok's galaxy is a period-8 oscillator. Here it is built from **225 metacells** (15 × 15), about 7.4 \
                 million live cells at t = 0: the macro pattern of Golly's metapixel-galaxy demo, rebuilt by this \
                 application. Eight metagens (282,624 generations) bring it back to its starting phase. Zoom out to \
                 watch the macro galaxy turn; zoom in on any tile to see the same moment as cells.",
            )
            .with_action("example:galaxy", "Load and run the galaxy"),
            TutorialPage::new(
                "Program another rule",
                "In **Macro rule** (Tools panel) toggle birth and survival digits or pick a named rule, choose the \
                 neighbourhood and press **Program and rebuild**: every tile gets the matching eater cells and fuses, \
                 and the construction restarts at t = 0. The physical cells keep following B3/S23; only the rule the \
                 tiles compute changes.\n\n\
                 The parity scenario programs **B135/S135** with the von Neumann neighbourhood (the four corner LWSS \
                 fuses removed): each metacell becomes the XOR of its four neighbours.",
            )
            .with_action("load:metapixel.parity-von-neumann", "Load the parity rule"),
            TutorialPage::new(
                "Edit the macro pattern",
                "Pick **Draw** (key 2) and click tiles to switch them, or drag to paint several; **Stamp** (key 3) \
                 places a library pattern, such as a glider, as macro cells. Every edit changes the generation-0 \
                 pattern and rebuilds the construction at t = 0: tiles cannot be inserted into a running \
                 construction, because all tiles must share one phase and their fuses burn during the first cycle. \
                 Each click or drag is one undo step (Ctrl/Cmd+Z). After a run, *Continue from the readout* makes \
                 the latest macro state the new starting pattern.",
            )
            .with_action("tool:draw", "Pick the Draw tool"),
            TutorialPage::new(
                "Metrics and validation",
                "- **population**: live B3/S23 cells of the whole construction.\n\
                 - **metagen**: m(t), the metagen on display; **phase**: t mod 35,328.\n\
                 - **readouts**: metagens read so far.\n\
                 - **exact_on_tiles** / **accelerated_on_tiles**: ON tiles read from the cells, and predicted by the \
                 accelerated macro model (the rule applied directly to the macro grid, not a physical simulation).\n\
                 - **mismatches**, **ambiguous_tiles**: readings that differ from the prediction, and cores that were \
                 neither 0 nor 29,795 cells.\n\
                 - **node_count**, **memory_bytes**: Hashlife's quadtree.\n\n\
                 The Validation tab lists every readout against the prediction and compares populations and core \
                 counts with Golly's golden table where one exists. A failure is shown in red, never hidden.",
            )
            .with_action("tab:validation", "Open the Validation tab"),
            TutorialPage::new(
                "Experiments to try",
                "- Run the **meta-glider** for 16 metagens: it moves one metacell diagonally every 4 metagens until \
                 it meets the dead border of the grid.\n\
                 - Make a **random soup**, run it, and check that every readout still matches the prediction.\n\
                 - Program **B0/S** on a 1 × 1 grid: the lone tile is born from nothing, then dies, a metacell \
                 blinking with period 2.\n\
                 - Make every tile deaf to the north (untick N in the compass) and see which births disappear.\n\
                 - Use 512 generations per tick with the **Live cores** layer and watch displays switch between \
                 t mod P = 4,320 and 7,407.",
            )
            .with_action("load:metapixel.glider", "Load the meta-glider"),
        ],
        concepts: CONCEPTS.to_string(),
    }
}

const CONCEPTS: &str = "## Three representations, one truth\n\n\
| Layer | What it is | Status |\n\
|---|---|---|\n\
| Physical cells | Every cell of the construction under B3/S23, advanced by Hashlife | **Exact**: the simulation |\n\
| Macro readout | Live cells counted in each tile's core [250, 1750]^2 at t = (n + 1)·35,328: 0 = OFF, 29,795 = ON, anything else ambiguous | Derived from the cells |\n\
| Live cores | The same count at any generation, with thresholds (0 OFF, at least 21,677 ON, switching in between) | Derived from the cells |\n\
| Prediction | The macro rule applied directly to the macro grid | **Accelerated macro representation, not a physical simulation**: a check only |\n\n\
## Timing\n\n\
- Period: P = 35,328 = 2^9 · 3 · 23 generations per metagen.\n\
- First cycle (t < P): proximity fuses burn, the displays keep showing metagen 0.\n\
- Readout of metagen n: t = (n + 1)·P.\n\
- Switching window (t at least P): t mod P from 4,320 to 7,407.\n\
- Displayed metagen: m(t) = max(0, floor((t - 7,408) / P)).\n\n\
## The tile\n\n\
- 2,048 × 2,048 nominal cells; the files are 2,058 × 2,058, so neighbouring tiles overlap by 5 cells.\n\
- Unprogrammed: 23,920 cells (OFF tile) and 66,203 cells (ON tile).\n\
- Rule table: 18 broken eaters; digit d of B or S is programmed by one cell, B_d = (148, 1404 + o) and \
S_d = (162, 1406 + o) with o = 0, 10, 20, 46, 56, 66, 92, 102, 112 for d = 8 down to 0.\n\
- Neighbourhood: edge blocks and corner LWSS fuses; a removed corner fuse silences the opposite corner.\n\
- Grid border: tiles outside the grid do not exist, so they are permanently OFF and never born.\n\n\
## Credits and licence\n\n\
The OTCA metapixel was designed and built by **Brice Due** (2005 to 2006). The tile files and Golly's metapixel \
demos are third-party pattern data under **GPL-2.0-or-later**, loaded from `assets/patterns/otca/` at run time and \
never embedded in the application; `PROVENANCE.md` in that folder lists their sources and checksums. Golden values \
and timings were measured with Golly's `bgolly`. The internal mechanism (clock, latches, display lanes) is described \
from Brice Due's slide captions and is an interpretation; the behaviour shown here is simulated, not assumed.";

/// Handles the tutorial's action buttons.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tutorial_actions(
    mut actions: MessageReader<TutorialAction>,
    mut requests: ResMut<LoadRequests>,
    mut lifecycle: ResMut<Lifecycle>,
    session: Option<Res<MpSession>>,
    mut data: ResMut<SimData>,
    mut tools: ResMut<ToolState>,
    mut settings: ResMut<ViewSettings>,
    mut fit: ResMut<CameraFit>,
    mut tab: ResMut<PropertiesTab>,
    mut shell: MessageWriter<ShellCommand>,
) {
    for TutorialAction(id) in actions.read() {
        let shape = current_shape(&data);
        let centre = shape.map(|s| (s.columns / 2, s.rows / 2));
        match id.as_str() {
            "example:blinker" => {
                requests.0.push("metapixel.blinker".into());
                lifecycle.play_after_load = true;
                settings.layer = LayerMode::Auto;
            }
            "example:galaxy" => {
                requests.0.push("metapixel.galaxy".into());
                lifecycle.play_after_load = true;
                settings.layer = LayerMode::Auto;
            }
            id if id.starts_with("load:") => requests.0.push(id.trim_start_matches("load:").to_string()),
            "zoom:grid" => fit.request(true),
            "zoom:tile" => {
                if let (Some(shape), Some((c, r))) = (shape, centre) {
                    tools.selected = Some((c, r));
                    fit.frame(shape.tile_rect(c, r), 0.06);
                }
            }
            "zoom:rules" => {
                if let (Some(shape), Some((c, r))) = (shape, centre) {
                    tools.selected = Some((c, r));
                    settings.anatomy = true;
                    settings.layer = LayerMode::Auto;
                    fit.frame(rule_table_rect(shape, c, r), 0.05);
                }
            }
            "watch:transition" => {
                // Jump to just before the next switching window, then play
                // slowly enough to see the displays change (about 1.6 s).
                if let Some(session) = &session {
                    session.send(Control::SetParam("step".into(), ParamValue::Text(StepSize::Pow2(7).value())));
                    session.send(Control::SetSpeed(Speed::TicksPerSecond(15.0)));
                    crate::session::jump(session, &mut data, crate::timing::transition_lead_in);
                    session.send(Control::Play);
                }
                settings.source = MacroSource::Live;
                settings.layer = LayerMode::Auto;
                fit.request(true);
            }
            "step:metagen" => {
                if let Some(session) = &session {
                    session.send(Control::SetParam("step".into(), ParamValue::Text(StepSize::Metagen.value())));
                    session.send(Control::Step(1));
                }
            }
            "tool:draw" => tools.tool = Tool::Draw,
            "tab:validation" => {
                *tab = PropertiesTab::Validation;
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
            }
            other => warn!("unknown metapixel tutorial action `{other}`"),
        }
        data.refresh_info();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metapixel::spec::{
        PERIOD, POPULATION_OFF_UNPROGRAMMED, POPULATION_ON_UNPROGRAMMED, READOUT_ON_COUNT, READOUT_ON_STEADY_MIN,
        RuleTable, TRANSITION_END, TRANSITION_START, rule_bit_cell,
    };

    #[test]
    fn every_page_has_an_action_the_mode_handles() {
        let t = tutorial();
        assert!(t.pages.len() >= 10);
        let catalog = crate::session::Catalog::builtin();
        for page in &t.pages {
            let action = page.action.as_ref().expect("interactive steps");
            let id = action.id.as_str();
            let known = [
                "example:blinker",
                "example:galaxy",
                "zoom:grid",
                "zoom:tile",
                "zoom:rules",
                "watch:transition",
                "step:metagen",
                "tool:draw",
                "tab:validation",
            ];
            match id.strip_prefix("load:") {
                Some(scenario) => assert!(catalog.get(scenario).is_some(), "{scenario}"),
                None => assert!(known.contains(&id), "{id}"),
            }
        }
    }

    /// The numbers quoted in the tutorial are the model's.
    #[test]
    fn quoted_numbers_match_the_model() {
        assert_eq!(PERIOD, 35_328);
        assert_eq!(PERIOD, 512 * 3 * 23);
        assert_eq!(PERIOD, 768 * 46);
        assert_eq!((TRANSITION_START, TRANSITION_END), (4_320, 7_407));
        assert_eq!(READOUT_ON_COUNT, 29_795);
        assert_eq!(READOUT_ON_STEADY_MIN, 21_677);
        assert_eq!((POPULATION_OFF_UNPROGRAMMED, POPULATION_ON_UNPROGRAMMED), (23_920, 66_203));
        assert_eq!(rule_bit_cell(RuleTable::Birth, 3), (148, 1470));
        assert_eq!(rule_bit_cell(RuleTable::Survival, 2), (162, 1498));
        assert_eq!(rule_bit_cell(RuleTable::Survival, 3), (162, 1472));
        assert_eq!(crate::timing::jumps_text(PERIOD), "2^9 + 2^11 + 2^15");
        // 15 tiles of 2,048 cells plus the 5-cell margin on both sides.
        assert_eq!(crate::geometry::GridShape::new(15, 15).bounds().width(), 30_730);
        assert_eq!(8 * PERIOD, 282_624);
        assert_eq!(crate::geometry::MICRO_FULL_PX, 128.0);
        assert_eq!(crate::geometry::MACRO_FULL_PX, 32.0);
        let galaxy = crate::session::Catalog::builtin();
        let galaxy = galaxy.get("metapixel.galaxy").unwrap();
        assert_eq!((galaxy.width, galaxy.height), (15, 15));
        let parity = crate::session::Catalog::builtin().get("metapixel.parity-von-neumann").cloned().unwrap();
        assert_eq!((parity.rule.as_str(), parity.neighbourhood.as_str()), ("B135/S135", "von_neumann"));
    }

    #[test]
    fn concepts_credit_brice_due_and_the_gpl_data() {
        let t = tutorial();
        assert!(t.concepts.contains("Brice Due") && t.concepts.contains("GPL-2.0-or-later"));
        assert!(t.concepts.contains("PROVENANCE.md"));
    }
}
