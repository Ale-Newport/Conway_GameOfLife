//! egui panels of the mode: left tools, right properties, world overlay.

pub mod icons;
pub mod overlay;
pub mod properties;
pub mod tools;
pub mod widgets;

#[cfg(test)]
mod tests {
    use bevy_egui::egui;

    /// Every non-ASCII character of the mode's UI text and tutorial exists
    /// in egui's built-in fonts (missing ones render as boxes).
    #[test]
    fn ui_text_glyphs_exist_in_the_builtin_fonts() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let sources = [
            include_str!("tools.rs"),
            include_str!("properties.rs"),
            include_str!("overlay.rs"),
            include_str!("widgets.rs"),
            include_str!("../tutorial.rs"),
            include_str!("../state.rs"),
            include_str!("../lib.rs"),
        ];
        let mut missing = Vec::new();
        for source in sources {
            for c in source.chars().filter(|c| !c.is_ascii()) {
                let s = c.to_string();
                let present = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), &s));
                if !present && !missing.contains(&c) {
                    missing.push(c);
                }
            }
        }
        assert!(missing.is_empty(), "glyphs missing from egui's fonts: {missing:?}");
    }
}
