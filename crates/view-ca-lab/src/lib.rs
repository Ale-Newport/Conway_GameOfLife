//! # view-ca-lab — the Cellular Automata Lab mode
//!
//! A laboratory of other automata (`AppMode::CellularAutomataLab`), one
//! session at a time, built on the models of the `cellular-automata` crate
//! and integrated into the application shell following `docs/UI_SHELL.md`:
//!
//! ```ignore
//! app.add_plugins(view_ca_lab::Plugin);
//! ```
//!
//! | Family | Model | Highlights |
//! |---|---|---|
//! | Life-like | `ca.life_like` | presets (HighLife, Seeds, Day & Night, Life without Death...), any typed B/S rule with live validation, B/S toggles |
//! | Generations | `ca.generations` | multi-state colouring and legend, Brian's Brain, Star Wars, typed B/S/C rules |
//! | Wireworld | `ca.wireworld` | head / tail / conductor painting, wire drawing, the bundled diode, clocks and gates as stamps |
//! | Langton's ant | `ca.langtons_ant` | turmite rules (RL, LLRR...), ant markers with heading, follow mode, adding and removing ants |
//! | Elementary CA | `ca.wolfram` | rule number and clickable 8-case rule table, equivalent rules, measured behaviour class, space-time diagram growing downward |
//!
//! | Part | Module |
//! |---|---|
//! | Families, state names and colours | [`family`] |
//! | Rule analysis (validation, comparison with B3/S23, Wolfram tables and classes) | [`rules`] |
//! | The five models behind one `Simulation` | [`sim`] |
//! | Runner, served descriptor, scenarios | [`session`] |
//! | Camera, view requests, surface, gizmos | [`world`] |
//! | Editing tools and shortcuts | [`tools`] |
//! | Tools / Properties panels and the overlay | [`panels`] |
//! | Tutorial | [`tutorial`] |
//!
//! Keyboard shortcuts (in addition to the shell's): 1–6 tools, `[` / `]`
//! brush size, `,` / `.` paint state, Q / E turn the circuit stamp or the
//! new ant, F frame the world, G grid lines, T colouring, V follow the ant
//! or the newest row. The shell's `=` / `-` keep changing the speed: they
//! are withheld from the grid camera, which would otherwise also zoom.

pub mod family;
pub mod panels;
pub mod rules;
pub mod session;
pub mod sim;
pub mod tools;
pub mod tutorial;
pub mod world;

use bevy::prelude::*;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use rendering::{GridCameraPlugin, GridCameraSystems, GridRenderPlugin};

pub use family::Family;
pub use session::{Catalog, LabRunner, LabSession, LoadRequests, LoadedScenario};
pub use sim::{LAB_MODEL_ID, LabCommand, LabInfo, LabSim, LabView};
pub use tools::{Tool, ToolSettings};
pub use world::{CameraFit, Colouring, LatestView, ViewSettings};

/// The mode implemented by this crate.
pub const MODE: AppMode = AppMode::CellularAutomataLab;

/// Ordering of the mode's `Update` systems (all run only in [`MODE`], after
/// the shell polled the session and after the camera updated
/// [`rendering::VisibleCells`]).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum CaLabSystems {
    /// Scenario loading, tutorial actions, descriptor synchronisation.
    Session,
    /// Pointer tools and keyboard shortcuts.
    Input,
    /// View requests and drawing of the latest view.
    View,
    /// Gizmos.
    Draw,
}

/// The Cellular Automata Lab mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaLabViewPlugin;

/// Short name: `app.add_plugins(view_ca_lab::Plugin)`.
pub use self::CaLabViewPlugin as Plugin;

impl bevy::app::Plugin for CaLabViewPlugin {
    fn build(&self, app: &mut App) {
        // Shared rendering plugins; another mode may have added them already.
        if !app.is_plugin_added::<GridRenderPlugin>() {
            app.add_plugins(GridRenderPlugin);
        }
        if !app.is_plugin_added::<GridCameraPlugin>() {
            app.add_plugins(GridCameraPlugin);
        }
        let card = ModeCard {
            status: ModeStatus::Ready,
            description: "Life-like and Generations rules (HighLife, Seeds, Brian's Brain...), Wireworld circuits, \
                          Langton's ant and turmites, and the 256 elementary automata as space-time diagrams."
                .to_string(),
            ..ModeCard::defaults(MODE)
        };
        app.register_mode(card)
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(tools::Circuits::load(None))
            .init_resource::<session::LoadRequests>()
            .init_resource::<session::DescriptorSync>()
            .init_resource::<world::ViewSettings>()
            .init_resource::<world::LatestView>()
            .init_resource::<world::ViewRequestState>()
            .init_resource::<world::CameraFit>()
            .init_resource::<world::FitQuery>()
            .init_resource::<world::OverlayInset>()
            .init_resource::<tools::ToolSettings>()
            .init_resource::<tools::ToolState>()
            .init_resource::<tools::Hover>()
            .init_resource::<panels::PanelState>()
            .add_mode_panel(MODE, PanelSlot::Left, panels::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, panels::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, panels::overlay_panel)
            .add_systems(Startup, session::init_catalog)
            .add_systems(OnEnter(MODE), (world::spawn_world, session::enter_session).chain())
            .add_systems(OnExit(MODE), session::exit_session)
            .configure_sets(
                Update,
                (CaLabSystems::Session, CaLabSystems::Input, CaLabSystems::View, CaLabSystems::Draw)
                    .chain()
                    .after(ShellSet::Poll)
                    .after(GridCameraSystems::Visible)
                    .run_if(in_state(MODE)),
            )
            .add_systems(
                Update,
                (
                    world::release_speed_keys_from_camera
                        .after(ShellSet::Commands)
                        .before(GridCameraSystems::Control)
                        .run_if(in_state(MODE)),
                    (world::frame_world, world::follow_subject)
                        .chain()
                        .before(GridCameraSystems::Control)
                        .run_if(in_state(MODE)),
                    (
                        session::collect_scenario_picks,
                        tutorial::tutorial_actions,
                        session::load_requested_scenarios,
                        session::sync_live_model,
                        panels::fit_tool_settings,
                    )
                        .chain()
                        .in_set(CaLabSystems::Session),
                    (tools::pointer_tools, tools::mode_shortcuts).in_set(CaLabSystems::Input),
                    (world::request_visible_region, world::draw_latest_view).chain().in_set(CaLabSystems::View),
                    world::draw_gizmos.in_set(CaLabSystems::Draw),
                ),
            );
    }
}

#[cfg(test)]
mod glyph_tests {
    use bevy_egui::egui;
    use simulation_core::Simulation;

    /// Non-ASCII characters of the string literals of this crate's sources
    /// (comment lines, unit tests and the glyph fallback table are skipped).
    fn literal_chars() -> Vec<(char, &'static str)> {
        let sources: [(&str, &str); 8] = [
            ("family.rs", include_str!("family.rs")),
            ("panels.rs", include_str!("panels.rs")),
            ("rules.rs", include_str!("rules.rs")),
            ("session.rs", include_str!("session.rs")),
            ("sim.rs", include_str!("sim.rs")),
            ("tools.rs", include_str!("tools.rs")),
            ("tutorial.rs", include_str!("tutorial.rs")),
            ("world.rs", include_str!("world.rs")),
        ];
        let mut out = Vec::new();
        for (file, text) in sources {
            let text = text.split("\n#[cfg(test)]").next().unwrap_or(text);
            // The fallback table lists exactly the characters that are never displayed.
            let text = match text.find("const GLYPH_FALLBACKS") {
                Some(a) => {
                    let b = text[a..].find("];").map_or(text.len(), |b| a + b);
                    format!("{}{}", &text[..a], &text[b..])
                }
                None => text.to_string(),
            };
            for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for c in line.chars().filter(|c| !c.is_ascii()) {
                    if !out.iter().any(|(x, _)| *x == c) {
                        out.push((c, file));
                    }
                }
            }
        }
        out
    }

    /// Every character the mode displays (its own texts, the tutorial and
    /// the served descriptors of every scenario) exists in egui's built-in
    /// fonts; otherwise it would render as a box.
    #[test]
    fn every_displayed_character_has_a_glyph() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.label("x");
        });
        output.textures_delta.clear();
        let has = |c: char| ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), &c.to_string()));
        let mut missing: Vec<String> = literal_chars()
            .into_iter()
            .filter(|(c, _)| !has(*c))
            .map(|(c, file)| format!("{c} (U+{:04X}, {file})", c as u32))
            .collect();
        let tutorial = crate::tutorial::tutorial();
        let mut texts: Vec<(String, String)> = tutorial
            .pages
            .iter()
            .flat_map(|p| [p.title.clone(), p.body.clone()])
            .chain([tutorial.concepts.clone()])
            .map(|t| ("tutorial".to_string(), t))
            .collect();
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let catalog = crate::session::Catalog::load(Some(&assets));
        for s in &catalog.scenarios {
            let sim = catalog.build(&s.id).expect("builds");
            let d = crate::session::display_descriptor(&sim.descriptor());
            let mut all = vec![d.name, d.summary, d.rules, s.name.clone(), s.description.clone()];
            for p in &d.params {
                all.extend([p.label.clone(), p.description.clone(), p.group.clone()]);
                if let simulation_core::ParamKind::Choice { options } = &p.kind {
                    all.extend(options.iter().map(|o| o.label.clone()));
                }
            }
            for m in &d.metrics {
                all.extend([m.label.clone(), m.description.clone(), m.unit.clone()]);
            }
            all.extend(crate::panels::state_names(&sim.info()));
            texts.extend(all.into_iter().map(|t| (s.id.clone(), t)));
        }
        for (source, text) in texts {
            for c in text.chars().filter(|c| !c.is_ascii() && !has(*c)) {
                let entry = format!("{c} (U+{:04X}, {source})", c as u32);
                if !missing.contains(&entry) {
                    missing.push(entry);
                }
            }
        }
        assert!(missing.is_empty(), "characters without a glyph: {missing:?}");
    }
}
