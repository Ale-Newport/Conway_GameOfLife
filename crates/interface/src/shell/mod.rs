//! The shell's egui pass: top bar, mode panels, bottom bar, dashboard,
//! windows and toasts, drawn by one exclusive system so that mode panel
//! one-shot systems can run inside the panels.

mod bottom_bar;
mod panels;
mod timeline;
mod top_bar;
mod windows;

use std::sync::Arc;

use bevy::ecs::system::RegisteredSystemError;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_egui::{EguiContext, PrimaryEguiContext};
use simulation_core::{ModelDescriptor, RunnerStatus};

use crate::layout::ShellLayout;
use crate::mode::{ActiveSession, AppMode, ModePanelRegistry, ModePanels, ModeRegistry};
use crate::session::SessionStatus;
use crate::state::ShellState;
use crate::theme::{Palette, ThemeSettings, apply_palette};
use crate::toasts::{Toasts, show_toasts};

/// Per-frame data shared by the shell's drawing functions.
pub(crate) struct Frame {
    /// Current state.
    pub mode: AppMode,
    /// Colours of this frame.
    pub palette: Palette,
    /// The active session.
    pub session: Option<ActiveSession>,
    /// Its descriptor this frame.
    pub descriptor: Option<Arc<ModelDescriptor>>,
    /// Its status this frame.
    pub status: Option<RunnerStatus>,
    /// Panel systems of the mode.
    pub panels: ModePanels,
}

impl Frame {
    fn gather(world: &World) -> Self {
        let mode = *world.resource::<State<AppMode>>().get();
        let theme = world.resource::<ThemeSettings>().kind;
        let accent = world.resource::<ModeRegistry>().accent(mode);
        let session = world.get_resource::<ActiveSession>().cloned();
        Self {
            mode,
            palette: Palette::new(theme, accent),
            descriptor: session.as_ref().map(|s| s.session().descriptor()),
            session,
            status: world.resource::<SessionStatus>().get().cloned(),
            panels: world.resource::<ModePanelRegistry>().get(mode),
        }
    }
}

/// Runs a mode panel system inside `ui`. A system skipped because its
/// parameters are unavailable (e.g. a resource the mode has not inserted
/// yet) draws nothing, which Bevy does not treat as an error; real failures
/// become an error toast (identical toasts merge with a repeat counter).
pub(crate) fn run_panel(world: &mut World, system: crate::mode::PanelSystem, ui: &mut egui::Ui) {
    match world.run_system_with(system, ui) {
        Ok(()) => {}
        Err(RegisteredSystemError::Skipped(reason)) => debug!("mode panel skipped: {reason}"),
        Err(e) => {
            let message = format!("A panel of this mode failed to draw: {e}");
            warn!("{message}");
            world.resource_mut::<Toasts>().error(message);
        }
    }
}

fn primary_context(world: &mut World) -> Option<egui::Context> {
    let mut query = world.query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>();
    let mut context = query.single_mut(world).ok()?;
    Some(context.get_mut().clone())
}

/// The shell's exclusive UI system (runs in `EguiPrimaryContextPass`).
pub(crate) fn shell_ui(world: &mut World) {
    let Some(ctx) = primary_context(world) else { return };
    let frame = Frame::gather(world);
    world.resource_scope(|world, mut shell: Mut<ShellState>| {
        let style_key = (world.resource::<ThemeSettings>().kind, frame.palette.accent);
        if shell.applied_style != Some(style_key) {
            apply_palette(&ctx, &frame.palette);
            shell.applied_style = Some(style_key);
        }
        let mut root = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("emergence_shell_root"),
            egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.content_rect()),
        );
        top_bar::show(&mut root, world, &mut shell, &frame);
        let central = if frame.mode == AppMode::Dashboard {
            crate::dashboard::show(&mut root, world, &frame);
            None
        } else {
            Some(panels::show_mode_layout(&mut root, world, &mut shell, &frame))
        };
        windows::show(&ctx, world, &mut shell, &frame);
        let anchor = central.map_or(ctx.content_rect().right_bottom(), |r| r.right_bottom()) - egui::vec2(14.0, 14.0);
        show_toasts(&ctx, &mut world.resource_mut::<Toasts>(), &frame.palette, anchor);
        let layout = ShellLayout { central: central.map(|r| Rect::new(r.min.x, r.min.y, r.max.x, r.max.y)) };
        world.resource_mut::<ShellLayout>().set_if_neq(layout);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource)]
    struct NeverInserted;

    #[derive(Component)]
    struct Selected;

    fn needs_selection(InMut(ui): InMut<egui::Ui>, _selected: Single<&Selected>) {
        ui.label("selection");
    }

    fn needs_missing_resource(InMut(ui): InMut<egui::Ui>, _missing: Res<NeverInserted>) {
        ui.label("unreachable");
    }

    #[test]
    fn skipped_panels_are_silent_and_failures_are_reported() {
        let mut world = World::new();
        world.init_resource::<Toasts>();
        let skipped = world.register_system(needs_selection);
        let failing = world.register_system(needs_missing_resource);
        let ctx = egui::Context::default();
        let _ = crate::run_headless_ui(&ctx, |ui| run_panel(&mut world, skipped, ui));
        assert!(world.resource::<Toasts>().items().is_empty(), "a skipped system is not an error");
        let _ = crate::run_headless_ui(&ctx, |ui| run_panel(&mut world, failing, ui));
        let toasts = world.resource::<Toasts>().items();
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].message.contains("failed to draw"), "{}", toasts[0].message);
    }
}
