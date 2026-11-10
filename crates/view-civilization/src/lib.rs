//! # view-civilization — the Emergent Civilizations mode
//!
//! A 3D view of the `civilization` model inside the Emergence Lab shell:
//!
//! * **World**: tile terrain with heights by class and an earth skirt,
//!   animated water, procedural trees on forest tiles (organism-render),
//!   berry bushes and rock outcrops whose look follows the resource stocks,
//!   huts, group stores and farm plots with five crop stages and
//!   construction scaffolds, roads drawn from the model's trail grid and a
//!   territory overlay in group colours.
//! * **People**: articulated figures (head, tunic in the group colour, arms,
//!   legs) with a walk cycle matched to their on-screen speed, work, rest,
//!   eating, talking and theft poses, a bundle on the back for the good
//!   carried most and painted activity badges.
//! * **Panels**: intervention tools (spawn, add/remove resources, disaster,
//!   regeneration), layers and camera presets (left); inspector of the
//!   selected person (needs, goal and route, inventory, exchange rates,
//!   skills, preferences, relationships, recent actions) or building,
//!   society charts, a trade-network graph, groups and activities (right);
//!   clock, legend and scale bar (overlay).
//!
//! The model runs on the shell's runner as [`session::CivSession`], which
//! adds observations the model does not publish (who traded with whom, see
//! [`ledger`]) without changing the model or its snapshots.
//!
//! Integration: `app.add_plugins(view_civilization::Plugin)` after the
//! shell's `InterfacePlugin` (see `docs/UI_SHELL.md`).

pub mod coords;
pub mod graph;
pub mod interact;
pub mod ledger;
pub mod meshes;
pub mod palette;
pub mod people;
pub mod scene;
pub mod session;
pub mod state;
pub mod tutorial;
pub mod ui;
pub mod world_mesh;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::{
    App, Commands, IntoScheduleConfigs, MessageReader, MessageWriter, Mut, OnEnter, OnExit, Res, ResMut, Startup,
    Update, World, in_state,
};
use civilization::scenarios;
use interface::{
    ActiveSession, AppMode, ModeAppExt, ModeCard, ModeScenarios, ModeStatus, PanelSlot, RightPanelTab, ScenarioEntry,
    ScenarioList, ScenarioSelected, ShellCommand, Toasts, TutorialAction, TutorialAppExt,
};
use simulation_core::{Control, ParamValue, Runner, RunnerConfig, SessionControl};

use crate::people::{CivMeshes, PeopleIndex};
use crate::scene::{BuildingIndex, CivMaterials, MapCache};
use crate::session::{CivSession, Selection};
use crate::state::{CivRunner, CurrentFrame, ViewState};
use crate::tutorial::TutorialStep;

/// The mode this crate implements.
pub const MODE: AppMode = AppMode::EmergentCivilizations;
/// Scenario opened first.
pub const DEFAULT_SCENARIO: &str = "trade_routes";

/// The Emergent Civilizations mode (also exported as [`Plugin`]).
#[derive(Clone, Debug, Default)]
pub struct CivilizationViewPlugin;

/// Short name of [`CivilizationViewPlugin`]: `app.add_plugins(view_civilization::Plugin)`.
pub use self::CivilizationViewPlugin as Plugin;

impl bevy::app::Plugin for CivilizationViewPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<rendering::OrbitCameraPlugin>() {
            app.add_plugins(rendering::OrbitCameraPlugin);
        }
        if !app.is_plugin_added::<rendering::WaterPlugin>() {
            app.add_plugins(rendering::WaterPlugin);
        }
        if !app.is_plugin_added::<organism_render::OrganismRenderPlugin>() {
            let assets_root = app
                .world()
                .get_resource::<interface::AssetRoot>()
                .map(|r| r.0.clone())
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
            app.add_plugins(organism_render::OrganismRenderPlugin { assets_root });
        }
        app.register_mode(ModeCard {
            status: ModeStatus::Ready,
            tagline: "Needs, barter, roads and settlements".into(),
            description: "People with needs, skills and preferences gather, farm, build and barter on a living \
                          3D map; roads, markets, groups and territories emerge from their decisions."
                .into(),
            ..ModeCard::defaults(MODE)
        })
        .register_tutorial(MODE, tutorial::tutorial())
        .add_mode_panel(MODE, PanelSlot::Left, ui::tools::tools_panel)
        .add_mode_panel(MODE, PanelSlot::Right, ui::properties::properties_panel)
        .add_mode_panel(MODE, PanelSlot::Overlay, ui::overlay::overlay_panel)
        .init_resource::<CurrentFrame>()
        .init_resource::<ViewState>()
        .init_resource::<MapCache>()
        .init_resource::<PeopleIndex>()
        .init_resource::<BuildingIndex>()
        .init_resource::<interact::Hover>()
        .init_resource::<interact::ClickTracker>()
        .init_resource::<ui::properties::GraphLayout>()
        .init_resource::<ui::properties::ScrollTo>()
        .init_resource::<ui::overlay::LegendOpen>()
        .init_resource::<ui::overlay::OverlayRect>()
        .init_resource::<people::CameraDistance>()
        .add_systems(Startup, fill_scenarios)
        .add_systems(OnEnter(MODE), (ensure_assets, enter, scene::spawn_environment).chain())
        .add_systems(OnExit(MODE), exit)
        .add_systems(
            Update,
            (
                (handle_messages, interact::mode_shortcuts, interact::world_pointer, scene::send_request).chain(),
                (
                    (scene::pull_frame, people::measure_camera),
                    scene::sync_map,
                    (scene::sync_vegetation, scene::sync_buildings, people::sync_people),
                    people::animate_people,
                    (scene::wave_pennants, interact::draw_highlights),
                )
                    .chain(),
                // After `sync_map`: a new scenario is framed with its own map size.
                (scene::drive_camera, scene::note_scenario)
                    .chain()
                    .after(scene::sync_map)
                    .before(rendering::orbit::OrbitCameraSystems),
            )
                .after(interface::ShellSet::Poll)
                .run_if(in_state(MODE)),
        );
    }
}

/// Builds the shared meshes and materials once.
fn ensure_assets(world: &mut World) {
    if !world.contains_resource::<CivMeshes>() {
        let meshes = <CivMeshes as bevy::prelude::FromWorld>::from_world(world);
        world.insert_resource(meshes);
    }
    if !world.contains_resource::<CivMaterials>() {
        let materials =
            world.resource_scope(|world, mut standard: Mut<bevy::prelude::Assets<bevy::prelude::StandardMaterial>>| {
                let mut water = world.resource_mut::<bevy::prelude::Assets<rendering::WaterMaterial>>();
                CivMaterials::new(&mut standard, &mut water)
            });
        world.insert_resource(materials);
    }
}

fn fill_scenarios(mut list: ResMut<ModeScenarios>, mut toasts: ResMut<Toasts>) {
    match scenarios::builtin() {
        Ok(all) => list.set(
            MODE,
            ScenarioList {
                entries: all
                    .into_iter()
                    .map(|s| ScenarioEntry { id: s.id, title: s.name, description: s.description })
                    .collect(),
                selected: Some(DEFAULT_SCENARIO.into()),
            },
        ),
        Err(e) => toasts.error(format!("Civilization scenarios could not be read: {e}")),
    }
}

fn spawn_runner(session: CivSession) -> Arc<Runner<CivSession>> {
    Arc::new(Runner::spawn(session, RunnerConfig::default()).with_randomize(|s: &mut CivSession, seed| {
        // Rebuilds the same scenario with a new seed (terrain, founders, behaviour).
        let _ = s.reseed(seed);
    }))
}

fn load_session(id: &str) -> Result<CivSession, String> {
    let scenario = scenarios::builtin_by_id(id).map_err(|e| e.to_string())?;
    CivSession::new(scenario).map_err(|e| e.to_string())
}

fn enter(mut commands: Commands, scenarios: Res<ModeScenarios>, mut toasts: ResMut<Toasts>) {
    let id = scenarios.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.to_string());
    let session = match load_session(&id).or_else(|e| {
        toasts.error(format!("Scenario `{id}` could not be loaded: {e}"));
        load_session(DEFAULT_SCENARIO)
    }) {
        Ok(session) => session,
        Err(e) => {
            toasts.error(format!("The civilization model could not start: {e}"));
            return;
        }
    };
    let runner = spawn_runner(session);
    commands.insert_resource(ActiveSession::new(runner.clone()));
    commands.insert_resource(CivRunner(runner));
    reset_view(&mut commands);
}

fn reset_view(commands: &mut Commands) {
    commands.insert_resource(CurrentFrame::default());
    commands.insert_resource(ViewState::default());
    commands.insert_resource(MapCache::default());
    commands.insert_resource(PeopleIndex::default());
    commands.insert_resource(BuildingIndex::default());
    commands.insert_resource(interact::Hover::default());
    commands.insert_resource(ui::properties::GraphLayout::default());
    commands.insert_resource(ui::overlay::OverlayRect::default());
}

fn exit(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    // The runner thread stops when the last handle drops.
    commands.remove_resource::<CivRunner>();
    reset_view(&mut commands);
}

/// Scenario picks from the top bar and tutorial buttons.
#[allow(clippy::too_many_arguments)]
fn handle_messages(
    mut picked: MessageReader<ScenarioSelected>,
    mut actions: MessageReader<TutorialAction>,
    runner: Option<Res<CivRunner>>,
    current: Res<CurrentFrame>,
    mut scenarios: ResMut<ModeScenarios>,
    mut state: ResMut<ViewState>,
    mut toasts: ResMut<Toasts>,
    mut shell: MessageWriter<ShellCommand>,
) {
    let mut steps: Vec<TutorialStep> =
        picked.read().filter(|s| s.mode == MODE).map(|s| TutorialStep::Load(s.id.clone())).collect();
    steps.extend(actions.read().filter_map(|a| tutorial::parse_action(&a.0)));
    let Some(runner) = runner else { return };
    for step in steps {
        match step {
            TutorialStep::Load(id) => match load_session(&id) {
                Ok(session) => {
                    let name = session.sim().scenario().name.clone();
                    runner.0.send(Control::Replace(Box::new(session)));
                    scenarios.mark_selected(MODE, &id);
                    state.selection = Selection::None;
                    state.follow = false;
                    toasts.info(format!("Loaded “{name}”. Press Space to run."));
                }
                Err(e) => toasts.error(format!("Scenario `{id}` could not be loaded: {e}")),
            },
            TutorialStep::InspectNeediest => {
                let neediest = current
                    .get()
                    .and_then(|f| {
                        f.view.agents.iter().min_by(|a, b| (a.energy * a.health).total_cmp(&(b.energy * b.health)))
                    })
                    .map(|a| a.id);
                match neediest {
                    Some(id) => {
                        state.selection = Selection::Agent(id);
                        state.follow = true;
                        state.tool = state::Tool::Inspect;
                        shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
                    }
                    None => toasts.warning("Nobody is alive in this world."),
                }
            }
            TutorialStep::ShowRoads(ticks) => {
                state.layers.roads = true;
                state.layers.territory = false;
                runner.0.step(ticks);
            }
            TutorialStep::ShowTerritory => state.layers.territory = true,
            TutorialStep::Barter(on) => {
                runner.0.set_param("trade_enabled", ParamValue::Bool(on));
                toasts.info(if on {
                    "Barter switched on."
                } else {
                    "Barter switched off (Parameters tab, Economy group)."
                });
            }
            TutorialStep::Run(ticks) => runner.0.step(ticks),
            TutorialStep::Tool(tool) => state.tool = tool,
            TutorialStep::Metrics => {
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
            }
        }
    }
}
