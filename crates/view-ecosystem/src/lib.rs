//! # view-ecosystem — the Ecosystem Simulator mode
//!
//! A 3D view of the `ecosystem` model inside the Emergence Lab shell:
//!
//! | Part | Module |
//! |---|---|
//! | Session wrapper: frames, observations (kills, grazing), extra metrics, epochs | [`session`] |
//! | Model ↔ scene coordinates, terrain sampling, ray casts | [`coords`] |
//! | Ground colours, overlays and legends, sky and sun | [`palette`] |
//! | Species → visuals, actions → animations, speeds, sizes, tree stages | [`mapping`] |
//! | Food web graph and layout | [`foodweb`] |
//! | Camera, sky, terrain, water, plants, animals, weather and fire | [`scene`] |
//! | Picking, tools (interventions), shortcuts, gizmos | [`interact`] |
//! | Tools, Properties and overlay panels | [`ui`] |
//! | Scenario cards and the tutorial | [`scenarios`], [`tutorial`] |
//!
//! Integration (after the shell's `InterfacePlugin`):
//!
//! ```ignore
//! app.add_plugins(view_ecosystem::EcosystemViewPlugin);
//! ```

pub mod coords;
pub mod foodweb;
pub mod interact;
pub mod mapping;
pub mod palette;
pub mod scenarios;
pub mod scene;
pub mod session;
pub mod state;
pub mod tutorial;
pub mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use interface::{
    ActiveSession, AppMode, ModeAppExt, ModeCard, ModePlaceholders, ModeScenarios, ModeStatus, PanelSlot,
    ScenarioEntry, ScenarioList, ScenarioSelected, ShellSet, ToastLevel, Toasts, TutorialAction, TutorialAppExt,
};
use organism_render::{OrganismRenderPlugin, OrganismSystems};
use rendering::orbit::OrbitCameraSystems;
use rendering::{OrbitCameraPlugin, ParticlePlugin, WaterPlugin};
use simulation_core::{Control, ParamValue, Runner, RunnerConfig, Simulation};

use crate::session::EcoSession;
use crate::state::{EcoData, EcoRunner, EcoUi, PendingJobs};
use crate::tutorial::TutorialStep;

/// The application state of this mode.
pub const MODE: AppMode = AppMode::EcosystemSimulator;
/// Scenario opened first.
pub const DEFAULT_SCENARIO: &str = "balanced_meadow";

/// The Ecosystem Simulator mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct EcosystemViewPlugin;

/// Short alias: `app.add_plugins(view_ecosystem::Plugin)`.
pub type Plugin = EcosystemViewPlugin;

/// System sets of the mode in `Update` (all after `ShellSet::Poll`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EcosystemViewSystems {
    /// Frames, messages, requests.
    Data,
    /// Landscape, plants, animals, effects.
    World,
    /// Picking, tools, shortcuts, camera, gizmos.
    Input,
}

fn default_assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets")
}

impl bevy::app::Plugin for EcosystemViewPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<OrganismRenderPlugin>() {
            let root = app.world().get_resource::<interface::AssetRoot>().map_or_else(default_assets, |r| r.0.clone());
            app.add_plugins(OrganismRenderPlugin { assets_root: root });
        }
        if !app.is_plugin_added::<OrbitCameraPlugin>() {
            app.add_plugins(OrbitCameraPlugin);
        }
        if !app.is_plugin_added::<WaterPlugin>() {
            app.add_plugins(WaterPlugin);
        }
        if !app.is_plugin_added::<ParticlePlugin>() {
            app.add_plugins(ParticlePlugin);
        }
        app.register_mode(ModeCard {
            status: ModeStatus::Ready,
            tagline: "Food webs, energy and nutrient cycles on a living 3D landscape".into(),
            description:
                "Hundreds to thousands of individual animals with their own physiology and decisions, density fields \
                          of insects, ant and bee colonies, vegetation, trees and fungi, fire, weather and \
                          seasons, with every flow of energy and nutrients booked."
                    .into(),
            ..ModeCard::defaults(MODE)
        })
        .register_tutorial(MODE, tutorial::tutorial())
        .add_mode_panel(MODE, PanelSlot::Left, ui::tools::tools_panel)
        .add_mode_panel(MODE, PanelSlot::Right, ui::properties::properties_panel)
        .add_mode_panel(MODE, PanelSlot::Overlay, ui::overlay::overlay_panel)
        .init_gizmo_group::<interact::EcoGizmos>()
        .init_resource::<EcoData>()
        .init_resource::<EcoUi>()
        .init_resource::<PendingJobs>()
        .init_resource::<scene::SceneState>()
        .init_resource::<scene::terrain::MapState>()
        .init_resource::<scene::terrain::GroundDetail>()
        .init_resource::<scene::flora::TreeIndex>()
        .init_resource::<scene::flora::TreeMeshes>()
        .init_resource::<scene::flora::CoverIndex>()
        .init_resource::<scene::flora::MushroomAssets>()
        .init_resource::<scene::fauna::AnimalIndex>()
        .init_resource::<scene::fauna::NestAssets>()
        .init_resource::<scene::effects::EffectPools>()
        .init_resource::<interact::Hover>()
        .init_resource::<interact::ClickTracker>()
        .init_resource::<ui::UiCache>()
        .add_systems(Startup, (fill_scenarios, configure_gizmos))
        .add_systems(OnEnter(MODE), (enter, scene::spawn_scene).chain())
        .add_systems(OnExit(MODE), exit)
        .configure_sets(
            Update,
            (EcosystemViewSystems::Data, EcosystemViewSystems::World, EcosystemViewSystems::Input)
                .chain()
                .after(ShellSet::Poll)
                .before(OrganismSystems::Lod)
                .run_if(in_state(MODE)),
        )
        .add_systems(
            Update,
            (
                scene::poll_frame,
                handle_messages,
                sync_scenario_pick,
                interact::poll_jobs,
                interact::sync_follow,
                scene::send_request,
            )
                .chain()
                .in_set(EcosystemViewSystems::Data),
        )
        .add_systems(
            Update,
            (
                scene::terrain::rebuild_terrain,
                scene::update_sky,
                scene::terrain::refresh_landscape,
                scene::flora::sync_trees,
                scene::flora::tree_lod,
                scene::flora::hide_occluding_trees,
                scene::flora::sync_ground_cover,
                scene::fauna::layer_visibility,
                scene::fauna::sync_fauna,
                scene::fauna::animate_glides,
                scene::effects::sync_effects,
            )
                .chain()
                .in_set(EcosystemViewSystems::World),
        )
        .add_systems(
            Update,
            scene::fauna::no_lod_markers
                .after(OrganismSystems::Lod)
                .before(OrganismSystems::Visuals)
                .run_if(in_state(MODE)),
        )
        .add_systems(
            Update,
            (interact::mode_shortcuts, interact::world_pointer, scene::drive_camera, interact::draw_gizmos)
                .chain()
                .in_set(EcosystemViewSystems::Input)
                .before(OrbitCameraSystems),
        );
    }
}

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<interact::EcoGizmos>();
    config.line.width = 2.5;
    config.depth_bias = -0.002;
}

fn fill_scenarios(mut list: ResMut<ModeScenarios>, mut toasts: ResMut<Toasts>) {
    match ecosystem::scenario::builtin() {
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
        Err(e) => toasts.error(format!("Ecosystem scenarios could not be read: {e}")),
    }
}

/// Builds the session of a built-in scenario.
pub fn load_session(id: &str) -> Result<EcoSession, String> {
    let scenario = ecosystem::scenario::find_builtin(id).map_err(|e| e.to_string())?;
    EcoSession::new(&scenario).map_err(|e| e.to_string())
}

/// Spawns the runner: Randomize rebuilds the scenario with another seed.
pub fn spawn_runner(session: EcoSession) -> Arc<Runner<EcoSession>> {
    // 30 views per second: frames carry thousands of animals and trees, and
    // the scene interpolates motion between them.
    let config = RunnerConfig { publish_hz: 30.0, ..RunnerConfig::default() };
    Arc::new(Runner::spawn(session, config).with_randomize(|s: &mut EcoSession, seed| {
        let seed = i64::try_from(seed % (i64::MAX as u64)).unwrap_or(1);
        // `seed` applies at reset (RequiresReset); the reset then rebuilds the world.
        if s.set_param("seed", ParamValue::Int(seed)).is_ok() {
            s.reset();
        }
    }))
}

fn enter(
    mut commands: Commands,
    scenarios: Res<ModeScenarios>,
    mut placeholders: ResMut<ModePlaceholders>,
    mut toasts: ResMut<Toasts>,
) {
    reset_view(&mut commands);
    let id = scenarios.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.to_string());
    let session = load_session(&id).or_else(|e| {
        toasts.error(format!("Scenario `{id}` could not be loaded: {e}"));
        load_session(DEFAULT_SCENARIO)
    });
    match session {
        Ok(session) => {
            let runner = spawn_runner(session);
            commands.insert_resource(ActiveSession::new(runner.clone()));
            commands.insert_resource(EcoRunner(runner));
            placeholders.clear(MODE);
        }
        Err(e) => placeholders.set(MODE, ToastLevel::Error, format!("The ecosystem model could not start: {e}")),
    }
}

fn reset_view(commands: &mut Commands) {
    commands.insert_resource(EcoData::default());
    commands.insert_resource(EcoUi::default());
    commands.insert_resource(PendingJobs::default());
    commands.insert_resource(scene::terrain::MapState::default());
    commands.insert_resource(scene::flora::TreeIndex::default());
    commands.insert_resource(scene::flora::CoverIndex::default());
    commands.insert_resource(scene::fauna::AnimalIndex::default());
    commands.insert_resource(scene::effects::EffectPools::default());
    commands.insert_resource(interact::Hover::default());
    commands.insert_resource(interact::ClickTracker::default());
    commands.insert_resource(ui::UiCache::default());
}

fn exit(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    // Dropping the last runner handle stops its thread (without waiting).
    commands.remove_resource::<EcoRunner>();
    reset_view(&mut commands);
}

/// Keeps the top-bar picker on the scenario actually running (a loaded
/// save may come from another one).
fn sync_scenario_pick(data: Res<EcoData>, mut scenarios: ResMut<ModeScenarios>) {
    let Some(frame) = data.get().filter(|_| data.fresh) else { return };
    let id = &frame.info.scenario.id;
    let selected = scenarios.get(MODE).and_then(|l| l.selected.as_deref());
    if !id.is_empty() && selected != Some(id.as_str()) {
        scenarios.mark_selected(MODE, id);
    }
}

/// Scenario picks from the top bar and tutorial buttons.
#[allow(clippy::too_many_arguments)]
fn handle_messages(
    mut picked: MessageReader<ScenarioSelected>,
    mut actions: MessageReader<TutorialAction>,
    runner: Option<Res<EcoRunner>>,
    data: Res<EcoData>,
    camera: Option<Single<&bevy_panorbit_camera::PanOrbitCamera, With<scene::EcoCamera>>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut ui: ResMut<EcoUi>,
    mut toasts: ResMut<Toasts>,
) {
    let mut steps: Vec<TutorialStep> =
        picked.read().filter(|s| s.mode == MODE).map(|s| TutorialStep::Load(s.id.clone())).collect();
    for a in actions.read() {
        steps.extend(tutorial::parse_action(&a.0));
    }
    let Some(runner) = runner else { return };
    for step in steps {
        match step {
            TutorialStep::Load(id) => match load_session(&id) {
                Ok(session) => {
                    let name = session.scenario().name.clone();
                    runner.0.send(Control::Replace(Box::new(session)));
                    scenarios.mark_selected(MODE, &id);
                    interact::select(&runner, &mut ui, None);
                    ui.frame_map = true;
                    ui.tool = state::Tool::Inspect;
                    ui.tab = state::PropertiesTab::Scenario;
                    toasts.info(format!("Loaded “{name}”. Press Space to run."));
                }
                Err(e) => toasts.error(format!("Scenario `{id}` could not be loaded: {e}")),
            },
            TutorialStep::Tab(tab) => ui.tab = tab,
            TutorialStep::Overlay(overlay) => ui.overlay = overlay,
            TutorialStep::Play => runner.0.play(),
            TutorialStep::Tool(tool) => ui.tool = tool,
            TutorialStep::Camera(mode) => ui.camera = mode,
            TutorialStep::Follow => {
                let focus = camera.as_ref().map(|c| c.target_focus);
                let pick = data.get().and_then(|f| {
                    let cell = f.view.cell_size_m.max(1e-3);
                    let target = focus.map(|p| simulation_core::glam::Vec2::new(p.x / cell, p.z / cell));
                    f.view
                        .animals
                        .iter()
                        .filter(|a| !a.hidden)
                        .min_by(|a, b| {
                            let d = |p: simulation_core::glam::Vec2| target.map_or(0.0, |t| p.distance(t));
                            d(a.position).total_cmp(&d(b.position))
                        })
                        .map(|a| a.id)
                });
                match pick {
                    Some(id) => {
                        interact::select(&runner, &mut ui, Some(ecosystem::OrganismRef::Animal(id)));
                        ui.camera = state::CameraMode::Follow;
                    }
                    None => toasts.warning("No animal is visible to follow."),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;
    use std::time::{Duration, Instant};

    fn lifecycle_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<AppMode>()
            .init_resource::<ModeScenarios>()
            .init_resource::<ModePlaceholders>()
            .init_resource::<Toasts>()
            .add_systems(Startup, fill_scenarios)
            .add_systems(OnEnter(MODE), enter)
            .add_systems(OnExit(MODE), exit);
        app.update();
        app
    }

    fn switch(app: &mut App, mode: AppMode) {
        app.world_mut().resource_mut::<NextState<AppMode>>().set(mode);
        app.update();
    }

    #[test]
    fn reentering_while_running_never_leaks_runner_threads() {
        let baseline = simulation_core::live_workers();
        let mut app = lifecycle_app();
        for _ in 0..3 {
            switch(&mut app, MODE);
            let runner = app.world().get_resource::<EcoRunner>().expect("runner while in the mode").clone();
            assert!(app.world().contains_resource::<ActiveSession>());
            runner.0.play();
            runner.0.step(5);
            drop(runner);
            switch(&mut app, AppMode::Dashboard);
            assert!(!app.world().contains_resource::<EcoRunner>());
            assert!(!app.world().contains_resource::<ActiveSession>());
        }
        // Dropping the last handle stops the worker without blocking; it exits shortly after.
        let deadline = Instant::now() + Duration::from_secs(20);
        while simulation_core::live_workers() > baseline && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(simulation_core::live_workers(), baseline, "every runner thread exited");
    }

    #[test]
    fn selecting_an_animal_shows_its_species_sheet() {
        let runner = EcoRunner(spawn_runner(load_session(DEFAULT_SCENARIO).expect("scenario")));
        let mut ui = EcoUi { species: Some(3), tab: state::PropertiesTab::Species, ..EcoUi::default() };
        interact::select(&runner, &mut ui, Some(ecosystem::OrganismRef::Animal(7)));
        assert_eq!(ui.species, None, "the Species tab falls back to the selection's species");
        assert_eq!(ui.tab, state::PropertiesTab::Species, "an open species sheet stays open");
        ui.species = Some(2);
        interact::select(&runner, &mut ui, Some(ecosystem::OrganismRef::Tree(1)));
        assert_eq!(ui.species, Some(2), "trees have no animal species sheet");
        assert_eq!(ui.tab, state::PropertiesTab::Species);
    }

    #[test]
    fn every_builtin_scenario_loads() {
        for (file, _) in ecosystem::scenario::BUILTIN_FILES {
            let id = file.trim_end_matches(".ron");
            let session = load_session(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(session.scenario().id, id);
        }
    }
}
