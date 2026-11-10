//! # view-voxel — the Three-Dimensional Life mode
//!
//! A laboratory for three-dimensional outer-totalistic cellular automata
//! (the `voxel-life` model: Bays' 4555 / 5766, Softology multi-state rules,
//! Moore-26 and von Neumann-6 neighbourhoods). **These rules are not
//! Conway's 2D rules**; the mode says so in its card, overlay, panels and
//! tutorial.
//!
//! Integration (one line in `apps/desktop`):
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # let mut app = App::new();
//! app.add_plugins(view_voxel::VoxelViewPlugin);
//! ```
//!
//! The plugin follows the shell contract (`docs/UI_SHELL.md`):
//!
//! * registers the [`interface::ModeCard`] of [`AppMode::ThreeDimensionalLife`],
//!   the built-in scenarios (top-bar picker, loaded with `Control::Replace`)
//!   and a tutorial;
//! * on enter, spawns a `Runner<VoxelLife>` (with Clear and Randomize) as the
//!   [`interface::ActiveSession`], an orbit camera (`rendering::OrbitCameraRig`,
//!   a `WorldCamera`), lights and a floor; on exit, removes them;
//! * renders the model's surface view with the rendering crate's chunked
//!   voxel mesher in a background task (only changed 16³ chunks are
//!   re-meshed, only when a new view arrives or the colouring changes);
//! * contributes the left Tools panel (tool, rule editor with live
//!   validation and presets, seeding, cut, region, display, camera,
//!   shortcuts), the Properties tab (scenario, rule, population, probe,
//!   region counts, 2D slice inspector, plots, render statistics) and the
//!   overlay (rule, phase, legend, cut and scale);
//! * gates world interaction on `rendering::WorldInput`.
//!
//! The shell's Parameters, Rules and Metrics tabs read the descriptor the
//! runner publishes for the running rule and world (see `session.rs`).

mod build;
mod colors;
mod coords;
mod interact;
mod phase;
mod requests;
mod scene;
mod session;
mod settings;
mod shortcuts;
mod tutorial;
mod ui;

use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use interface::{
    ActiveSession, AppMode, ModeAppExt, ModeCard, ModeScenarios, ModeStatus, PanelSlot, ScenarioEntry, ScenarioList,
    ShellSet, Toasts, TutorialAppExt,
};
use rendering::OrbitCameraPlugin;
use rendering::orbit::OrbitCameraSystems;
use simulation_core::Control;
use voxel_life::{Scenario, VoxelLife, VoxelViewRequest};

pub use settings::{
    AgeScale, ColorMode, CutSettings, InspectorSettings, RENDER_CAPS, RegionSettings, Tool, Transparency, ViewSettings,
};
pub use shortcuts::{ModeShortcut, SHORTCUTS, ShortcutAction};

use crate::session::VoxelRunner;

/// The mode this crate implements.
pub const MODE: AppMode = AppMode::ThreeDimensionalLife;

/// Scenario loaded when the mode opens for the first time.
pub const DEFAULT_SCENARIO: &str = "builder-1";

/// Scenario ids in picker order (simple objects first); built-ins missing
/// here are appended.
const SCENARIO_ORDER: [&str; 12] = [
    "life-5766-glider",
    "life-4555-glider",
    "rokicki-ship",
    "life-in-5766-two-planes",
    "life-5766-soup",
    "builder-1",
    "softology-amoeba",
    "softology-pyroclastic",
    "softology-445",
    "clouds-1",
    "crystal-growth-1",
    "single-point-replication",
];

/// The Three-Dimensional Life mode.
pub struct VoxelViewPlugin;

impl Plugin for VoxelViewPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<OrbitCameraPlugin>() {
            app.add_plugins(OrbitCameraPlugin);
        }
        let (frame_gizmos, overlay_gizmos) = scene::gizmo_configs();
        app.register_mode(mode_card())
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_gizmo_config(scene::FrameGizmos, frame_gizmos)
            .insert_gizmo_config(scene::OverlayGizmos, overlay_gizmos)
            .init_resource::<ViewSettings>()
            .init_resource::<build::MeshState>()
            .init_resource::<requests::WorldState>()
            .init_resource::<requests::SliceState>()
            .init_resource::<requests::RegionState>()
            .init_resource::<requests::ProbeState>()
            .init_resource::<interact::PointerState>()
            .init_resource::<phase::PhaseInfo>()
            .init_resource::<ViewRequestState>()
            .init_resource::<ui::UiState>()
            .add_message::<scene::CameraAction>()
            .add_mode_panel(MODE, PanelSlot::Left, ui::tools::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, ui::properties::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, ui::overlay::overlay_panel)
            .add_systems(Startup, fill_scenarios)
            .add_systems(OnEnter(MODE), (enter, scene::spawn_scene).chain())
            .add_systems(OnExit(MODE), exit)
            .add_systems(
                Update,
                (
                    interact::handle_actions,
                    requests::refresh_world,
                    interact::keyboard_shortcuts,
                    scene::animate,
                    sync_view_request,
                    build::pump_builds,
                    (requests::poll_slice, requests::poll_region, requests::poll_probe, phase::update_phase),
                    interact::world_pointer,
                    scene::camera_actions,
                    scene::update_scene,
                    scene::draw_gizmos,
                )
                    .chain()
                    .run_if(in_state(MODE))
                    .run_if(resource_exists::<VoxelRunner>)
                    .after(ShellSet::Poll)
                    // Camera requests are applied after the rig, see
                    // `scene::needs_preset`.
                    .after(OrbitCameraSystems),
            );
    }
}

/// The dashboard card.
pub fn mode_card() -> ModeCard {
    ModeCard {
        title: "Three-Dimensional Life".into(),
        tagline: "Voxel automata with their own 3D rules".into(),
        description: "Survival / birth rules on the cubic lattice (Moore-26, von Neumann-6): Bays' 4555 and 5766, \
                      Softology multi-state rules. Cut the volume, inspect 2D slices and count regions. These are \
                      not Conway's 2D rules."
            .into(),
        status: ModeStatus::Ready,
        ..ModeCard::defaults(MODE)
    }
}

/// The built-in scenarios (parsed once).
pub(crate) fn builtin_scenarios() -> &'static [Scenario] {
    static CACHE: OnceLock<Vec<Scenario>> = OnceLock::new();
    CACHE.get_or_init(|| match voxel_life::scenarios::builtin() {
        Ok(mut list) => {
            let rank = |id: &str| SCENARIO_ORDER.iter().position(|o| *o == id).unwrap_or(SCENARIO_ORDER.len());
            list.sort_by_key(|s| rank(&s.id));
            list
        }
        Err(e) => {
            error!("3D Life: built-in scenarios: {e}");
            Vec::new()
        }
    })
}

/// A fresh model of a built-in scenario.
pub(crate) fn scenario_life(id: &str) -> Result<VoxelLife, String> {
    let scenario = builtin_scenarios()
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| format!("3D Life: no built-in scenario `{id}`"))?;
    VoxelLife::new(scenario).map_err(|e| format!("3D Life: scenario `{id}`: {e}"))
}

/// Picker entries of the built-in scenarios.
pub(crate) fn scenario_entries() -> Vec<ScenarioEntry> {
    builtin_scenarios()
        .iter()
        .map(|s| ScenarioEntry {
            id: s.id.clone(),
            title: s.name.clone(),
            description: format!("Rule {}. {}\n\nEvidence: {}.", s.rule, s.description, s.evidence.label()),
        })
        .collect()
}

fn fill_scenarios(mut scenarios: ResMut<ModeScenarios>) {
    let entries = scenario_entries();
    let selected = entries.iter().any(|e| e.id == DEFAULT_SCENARIO).then(|| DEFAULT_SCENARIO.to_string());
    scenarios.set(MODE, ScenarioList { entries, selected });
}

/// The surface request last sent to the runner.
#[derive(Resource, Default)]
struct ViewRequestState(Option<VoxelViewRequest>);

/// Sends the surface request (cut range and render cap) when it changes.
fn sync_view_request(settings: Res<ViewSettings>, runner: Res<VoxelRunner>, mut sent: ResMut<ViewRequestState>) {
    let request = VoxelViewRequest::Surface { range: settings.cut.range(), max_voxels: settings.render_cap };
    if sent.0.as_ref() != Some(&request) {
        runner.0.send(Control::SetViewRequest(request.clone()));
        sent.0 = Some(request);
    }
}

fn reset_mode_state(commands: &mut Commands) {
    commands.insert_resource(build::MeshState::default());
    commands.insert_resource(requests::WorldState::requesting());
    commands.insert_resource(requests::SliceState::default());
    commands.insert_resource(requests::RegionState::default());
    commands.insert_resource(requests::ProbeState::default());
    commands.insert_resource(interact::PointerState::default());
    commands.insert_resource(phase::PhaseInfo::default());
    commands.insert_resource(ViewRequestState::default());
    commands.insert_resource(ui::UiState::default());
}

fn enter(
    mut commands: Commands,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut settings: ResMut<ViewSettings>,
) {
    let selected = scenarios.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.to_string());
    let life = match scenario_life(&selected) {
        Ok(life) => life,
        Err(e) => {
            toasts.error(e);
            match scenario_life(DEFAULT_SCENARIO) {
                Ok(life) => {
                    scenarios.mark_selected(MODE, DEFAULT_SCENARIO);
                    life
                }
                Err(e) => {
                    toasts.error(e);
                    return;
                }
            }
        }
    };
    let session = Arc::new(session::spawn_runner(life));
    settings.cut.enabled = false;
    settings.sweep = false;
    settings.region.enabled = false;
    commands.insert_resource(ActiveSession::new(session.clone()));
    commands.insert_resource(VoxelRunner(session));
    reset_mode_state(&mut commands);
}

fn exit(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<VoxelRunner>();
    commands.remove_resource::<scene::SceneAssets>();
    // Dropping the mesh state cancels a build in flight; the runner thread
    // stops when the last handle drops.
    reset_mode_state(&mut commands);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_scenario_is_offered_and_loads() {
        let entries = scenario_entries();
        assert_eq!(entries.len(), voxel_life::scenarios::builtin().expect("builtin").len());
        assert_eq!(entries[0].id, "life-5766-glider", "simple objects first");
        assert!(entries.iter().any(|e| e.id == DEFAULT_SCENARIO));
        for entry in &entries {
            assert!(scenario_life(&entry.id).is_ok(), "{}", entry.id);
            assert!(entry.description.contains("Evidence"));
        }
        assert!(scenario_life("no-such-scenario").is_err());
    }

    #[test]
    fn card_says_it_is_not_conway() {
        let card = mode_card();
        assert_eq!(card.mode, MODE);
        assert_eq!(card.status, ModeStatus::Ready);
        assert!(card.description.contains("not Conway's 2D rules"));
    }
}
