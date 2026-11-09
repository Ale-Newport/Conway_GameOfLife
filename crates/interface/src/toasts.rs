//! Transient notifications shown in the bottom-right corner.
//!
//! Errors reported by the running simulation ([`simulation_core::SessionControl::take_errors`])
//! and results of shell operations (saved, exported, failed) appear here.
//! Identical consecutive messages are merged with a repeat counter so a
//! failing command cannot flood the screen.

use std::time::Duration;

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, CornerRadius, RichText, Stroke};

use crate::theme::{Palette, mix};

/// Importance of a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToastLevel {
    /// Neutral information.
    Info,
    /// An operation succeeded.
    Success,
    /// Something needs attention.
    Warning,
    /// An operation failed.
    Error,
}

impl ToastLevel {
    /// How long the toast stays visible.
    pub fn lifetime(self) -> Duration {
        match self {
            ToastLevel::Info | ToastLevel::Success => Duration::from_secs(4),
            ToastLevel::Warning => Duration::from_secs(6),
            ToastLevel::Error => Duration::from_secs(9),
        }
    }
}

/// One notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Importance.
    pub level: ToastLevel,
    /// Text shown to the user.
    pub message: String,
    /// How many identical messages were merged into this one.
    pub repeats: u32,
    /// Remaining display time.
    pub remaining: Duration,
}

/// Queue of visible notifications. Any system may push to it.
#[derive(Resource, Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

/// Maximum number of toasts kept at once (oldest dropped first).
pub const MAX_TOASTS: usize = 6;

impl Toasts {
    /// Adds a toast, merging it with the newest one if the text and level
    /// are identical.
    pub fn push(&mut self, level: ToastLevel, message: impl Into<String>) {
        let message = message.into();
        if let Some(last) = self.items.last_mut()
            && last.level == level
            && last.message == message
        {
            last.repeats = last.repeats.saturating_add(1);
            last.remaining = level.lifetime();
            return;
        }
        self.items.push(Toast { level, message, repeats: 1, remaining: level.lifetime() });
        if self.items.len() > MAX_TOASTS {
            let excess = self.items.len() - MAX_TOASTS;
            self.items.drain(..excess);
        }
    }

    /// Adds an information toast.
    pub fn info(&mut self, message: impl Into<String>) {
        self.push(ToastLevel::Info, message);
    }

    /// Adds a success toast.
    pub fn success(&mut self, message: impl Into<String>) {
        self.push(ToastLevel::Success, message);
    }

    /// Adds a warning toast.
    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(ToastLevel::Warning, message);
    }

    /// Adds an error toast.
    pub fn error(&mut self, message: impl Into<String>) {
        self.push(ToastLevel::Error, message);
    }

    /// Visible toasts, oldest first.
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// Ages every toast by `dt` and removes expired ones.
    pub fn tick(&mut self, dt: Duration) {
        for toast in &mut self.items {
            toast.remaining = toast.remaining.saturating_sub(dt);
        }
        self.items.retain(|t| !t.remaining.is_zero());
    }

    /// Removes the toast at `index` (dismissed by the user).
    pub fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }
}

/// Ages toasts every frame.
pub(crate) fn age_toasts(time: Res<Time>, mut toasts: ResMut<Toasts>) {
    toasts.tick(time.delta());
}

/// Draws the toasts stacked above `anchor` (bottom-right corner, logical
/// pixels). Clicking a toast dismisses it.
pub(crate) fn show_toasts(ctx: &egui::Context, toasts: &mut Toasts, palette: &Palette, anchor: egui::Pos2) {
    if toasts.items().is_empty() {
        return;
    }
    let mut dismissed = None;
    egui::Area::new(egui::Id::new("shell_toasts"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, anchor.to_vec2() - ctx.content_rect().right_bottom().to_vec2())
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            for (index, toast) in toasts.items().iter().enumerate().rev() {
                let tint = level_color(toast.level, palette);
                let label = match toast.level {
                    ToastLevel::Info => "Info",
                    ToastLevel::Success => "Done",
                    ToastLevel::Warning => "Warning",
                    ToastLevel::Error => "Error",
                };
                let response = egui::Frame::new()
                    .fill(mix(palette.surface, tint, 0.10))
                    .stroke(Stroke::new(palette.stroke_width, mix(palette.stroke, tint, 0.6)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(356.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).strong().color(tint));
                            if toast.repeats > 1 {
                                ui.label(
                                    RichText::new(format!("×{}", toast.repeats)).small().color(palette.text_muted),
                                );
                            }
                        });
                        ui.add(egui::Label::new(RichText::new(&toast.message).color(palette.text)).wrap());
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text("Click to dismiss");
                if response.clicked() {
                    dismissed = Some(index);
                }
                ui.add_space(6.0);
            }
        });
    if let Some(index) = dismissed {
        toasts.dismiss(index);
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}

/// Severity colour helper shared with other widgets.
pub fn level_color(level: ToastLevel, palette: &Palette) -> Color32 {
    match level {
        ToastLevel::Info => palette.accent,
        ToastLevel::Success => palette.success,
        ToastLevel::Warning => palette.warning,
        ToastLevel::Error => palette.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_messages_merge_and_expire() {
        let mut toasts = Toasts::default();
        toasts.error("boom");
        toasts.error("boom");
        toasts.info("boom");
        assert_eq!(toasts.items().len(), 2);
        assert_eq!(toasts.items()[0].repeats, 2);
        toasts.tick(Duration::from_secs(5));
        assert_eq!(toasts.items().len(), 1, "the info toast expires after 4 s");
        toasts.tick(Duration::from_secs(5));
        assert!(toasts.items().is_empty());
    }

    #[test]
    fn queue_is_bounded_and_dismissable() {
        let mut toasts = Toasts::default();
        for i in 0..10 {
            toasts.info(format!("message {i}"));
        }
        assert_eq!(toasts.items().len(), MAX_TOASTS);
        assert_eq!(toasts.items()[0].message, "message 4");
        toasts.dismiss(0);
        toasts.dismiss(99);
        assert_eq!(toasts.items()[0].message, "message 5");
    }
}
