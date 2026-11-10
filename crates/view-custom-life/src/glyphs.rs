//! Whether a glyph renders with egui's built-in fonts, and the fallback
//! shown when it does not (the type's initial letter).
//!
//! `egui::text::Fonts::has_glyphs` is not reliable for this (it reports
//! false negatives, and some fonts map characters such as `→` to an empty
//! box). Laying out one character at a time is: every character without a
//! drawable glyph uses the atlas cell of the replacement glyph, the same as
//! a private-use character. Results are cached per string.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use bevy_egui::egui::{Color32, Context, FontId};

fn cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

type UvKey = ([u16; 2], [u16; 2]);

fn uv(ctx: &Context, s: &str) -> Option<UvKey> {
    ctx.fonts_mut(|f| {
        let galley = f.layout_no_wrap(s.to_string(), FontId::proportional(16.0), Color32::WHITE);
        galley.rows.first().and_then(|r| r.row.glyphs.first()).map(|g| (g.uv_rect.min, g.uv_rect.max))
    })
}

/// Whether every character of `text` has a drawable glyph (variation
/// selectors and zero-width joiners are ignored).
pub fn renders(ctx: &Context, text: &str) -> bool {
    if let Some(known) = cache().lock().unwrap_or_else(PoisonError::into_inner).get(text) {
        return *known;
    }
    let missing = uv(ctx, "\u{E000}");
    let ok = text.chars().filter(|c| !matches!(*c, '\u{FE0E}' | '\u{FE0F}' | '\u{200D}')).all(|c| {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        c.is_whitespace() || uv(ctx, s) != missing
    });
    cache().lock().unwrap_or_else(PoisonError::into_inner).insert(text.to_string(), ok);
    ok
}

/// Initial letter of a name, upper case (empty when there is none).
pub fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// The icon to draw for a type: its glyph when it renders, else its initial.
pub fn shown_icon(ctx: &Context, icon: &str, name: &str) -> String {
    if !icon.trim().is_empty() && renders(ctx, icon) { icon.to_string() } else { initial(name) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        let ctx = Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        ctx
    }

    #[test]
    fn detects_missing_glyphs_and_falls_back_to_initials() {
        let ctx = ctx();
        assert!(renders(&ctx, "🌿"));
        assert!(renders(&ctx, "A"));
        assert!(!renders(&ctx, "🦠"), "not in the built-in emoji fonts");
        assert!(!renders(&ctx, "→"), "mapped to an empty box by the text font");
        assert_eq!(shown_icon(&ctx, "🦠", "bacterium"), "B");
        assert_eq!(shown_icon(&ctx, "", "lumen_moss"), "L");
        assert_eq!(shown_icon(&ctx, "🌿", "plant"), "🌿");
        assert_eq!(initial("_x"), "X");
    }

    #[test]
    fn every_quick_pick_and_ui_symbol_renders() {
        let ctx = ctx();
        for g in crate::colors::GLYPHS {
            assert!(renders(&ctx, g), "quick pick {g} would draw as a box");
        }
        for s in ["·", "—", "×", "…", "≤", "≥", "−", "Σ", "•", "⚠", "✔", "⚙", "°", "±", "µ", "π", "ƒ", "≈", "“", "”"]
        {
            assert!(renders(&ctx, s), "{s} would draw as a box");
        }
    }
}
