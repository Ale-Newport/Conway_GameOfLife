//! A small Markdown renderer for rule statements, concepts pages and
//! credits: ATX headings, paragraphs, bullet and numbered lists (nested by
//! indentation), fenced code blocks, block quotes, horizontal rules, pipe
//! tables, and inline `**bold**`, `*italic*` and `` `code` ``. Anything
//! else is shown as plain text, so no input can break the layout.

use bevy_egui::egui::{self, Color32, FontId, RichText, TextFormat, Ui, text::LayoutJob};

/// Inline text run with its emphasis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Text.
    pub text: String,
    /// `**bold**`.
    pub bold: bool,
    /// `*italic*`.
    pub italic: bool,
    /// `` `code` ``.
    pub code: bool,
}

impl Span {
    fn plain(text: &str) -> Self {
        Self { text: text.to_string(), bold: false, italic: false, code: false }
    }
}

/// Block-level element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `#` to `######` heading.
    Heading {
        /// 1 to 6.
        level: u8,
        /// Inline text.
        text: String,
    },
    /// Paragraph (consecutive lines joined with spaces).
    Paragraph(String),
    /// List item (`-`, `*`, `+` or `1.`).
    ListItem {
        /// Nesting depth (0 = top level), from indentation in steps of 2.
        depth: usize,
        /// `Some(n)` for numbered items.
        number: Option<u64>,
        /// Inline text.
        text: String,
    },
    /// Fenced code block.
    Code(String),
    /// `>` quote.
    Quote(String),
    /// `---`, `***` or `___`.
    Rule,
    /// Pipe table.
    Table {
        /// Header cells.
        header: Vec<String>,
        /// Body rows.
        rows: Vec<Vec<String>>,
    },
}

fn list_item(line: &str) -> Option<(usize, Option<u64>, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    let depth = indent / 2;
    for marker in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(marker) {
            return Some((depth, None, text.trim()));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        let after = &rest[digits..];
        if let Some(text) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
            let number = rest[..digits].parse().ok()?;
            return Some((depth, Some(number), text.trim()));
        }
    }
    None
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-') || compact.chars().all(|c| c == '*') || compact.chars().all(|c| c == '_'))
}

fn table_cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_table_separator(line: &str) -> bool {
    let cells = table_cells(line);
    !cells.is_empty()
        && cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')) && c.contains('-'))
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&level) {
        let rest = &trimmed[level..];
        if rest.is_empty() || rest.starts_with(' ') {
            return Some((level as u8, rest.trim().trim_end_matches('#').trim()));
        }
    }
    None
}

/// Splits Markdown text into blocks.
pub fn parse_blocks(markdown: &str) -> Vec<Block> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph.join(" ")));
            paragraph.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
            i += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush(&mut paragraph, &mut blocks);
            let fence = &trimmed[..3];
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Code(code.join("\n")));
            i += 1; // closing fence (or end of input)
            continue;
        }
        if let Some((level, text)) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading { level, text: text.to_string() });
        } else if is_rule(trimmed) && paragraph.is_empty() {
            blocks.push(Block::Rule);
        } else if trimmed.starts_with('|') && lines.get(i + 1).is_some_and(|next| is_table_separator(next)) {
            flush(&mut paragraph, &mut blocks);
            let header = table_cells(trimmed);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            blocks.push(Block::Table { header, rows });
            continue;
        } else if let Some(quote) = trimmed.strip_prefix('>') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Quote(quote.trim().to_string()));
        } else if let Some((depth, number, text)) = list_item(line) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::ListItem { depth, number, text: text.to_string() });
        } else if let Some(Block::ListItem { text, .. }) =
            blocks.last_mut().filter(|_| paragraph.is_empty() && line.starts_with("  "))
        {
            // Continuation line of a list item.
            text.push(' ');
            text.push_str(trimmed);
        } else {
            paragraph.push(trimmed);
        }
        i += 1;
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

/// Splits inline text into styled spans. Unclosed markers are literal.
pub fn parse_inline(text: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut rest = text;
    let push = |spans: &mut Vec<Span>, plain: &mut String, bold: bool, italic: bool| {
        if !plain.is_empty() {
            spans.push(Span { text: std::mem::take(plain), bold, italic, code: false });
        }
    };
    while let Some(c) = rest.chars().next() {
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            push(&mut spans, &mut plain, bold, italic);
            spans.push(Span { text: rest[1..1 + end].to_string(), bold, italic, code: true });
            rest = &rest[end + 2..];
            continue;
        }
        if rest.starts_with("**") && (bold || rest[2..].contains("**")) {
            push(&mut spans, &mut plain, bold, italic);
            bold = !bold;
            rest = &rest[2..];
            continue;
        }
        // `*` is one byte, so `rest[1..]` is a character boundary here.
        let toggles_italic = c == '*'
            && (italic || (rest[1..].chars().next().is_some_and(|n| n != ' ' && n != '*') && rest[1..].contains('*')));
        if toggles_italic {
            push(&mut spans, &mut plain, bold, italic);
            italic = !italic;
            rest = &rest[1..];
            continue;
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    push(&mut spans, &mut plain, bold, italic);
    if spans.is_empty() {
        spans.push(Span::plain(""));
    }
    spans
}

fn inline_job(ui: &Ui, text: &str, size: f32, color: Color32, strong_color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    for span in parse_inline(text) {
        let mut format = TextFormat {
            font_id: if span.code { FontId::monospace(size * 0.93) } else { FontId::proportional(size) },
            color: if span.bold { strong_color } else { color },
            italics: span.italic,
            ..Default::default()
        };
        if span.code {
            format.background = ui.visuals().code_bg_color;
        }
        job.append(&span.text, 0.0, format);
    }
    job
}

/// Renders Markdown into `ui` using the current visuals.
pub fn markdown(ui: &mut Ui, text: &str) {
    let visuals = ui.visuals().clone();
    let color = visuals.override_text_color.unwrap_or(visuals.widgets.noninteractive.fg_stroke.color);
    let strong = visuals.strong_text_color();
    // egui ships no bold face: emphasised words are tinted towards the accent.
    let emphasis = crate::theme::mix(color, visuals.hyperlink_color, 0.55);
    let muted = visuals.weak_text_color();
    let body = 14.0;
    for block in parse_blocks(text) {
        match block {
            Block::Heading { level, text } => {
                let size = match level {
                    1 => 20.0,
                    2 => 17.0,
                    3 => 15.0,
                    _ => 14.0,
                };
                ui.add_space(if level <= 2 { 8.0 } else { 4.0 });
                ui.label(inline_job(ui, &text, size, strong, strong));
                ui.add_space(2.0);
            }
            Block::Paragraph(text) => {
                ui.label(inline_job(ui, &text, body, color, emphasis));
                ui.add_space(4.0);
            }
            Block::ListItem { depth, number, text } => {
                // Hanging indent: wrapped lines align with the text, not the marker.
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(4.0 + depth as f32 * 16.0);
                    let marker = number.map_or_else(|| "•".to_string(), |n| format!("{n}."));
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, body + 4.0), egui::Sense::hover());
                    ui.painter().text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        marker,
                        FontId::proportional(body),
                        muted,
                    );
                    ui.add(egui::Label::new(inline_job(ui, &text, body, color, emphasis)).wrap());
                });
            }
            Block::Code(code) => {
                egui::Frame::new()
                    .fill(visuals.code_bg_color)
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(code).monospace().color(color));
                    });
                ui.add_space(4.0);
            }
            Block::Quote(text) => {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 18.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 1.0, visuals.selection.stroke.color);
                    ui.label(inline_job(ui, &text, body, muted, emphasis));
                });
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Table { header, rows } => {
                egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                    egui::Grid::new(ui.next_auto_id()).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                        for cell in &header {
                            ui.label(inline_job(ui, cell, 13.0, strong, strong));
                        }
                        ui.end_row();
                        for row in &rows {
                            for cell in row {
                                ui.label(inline_job(ui, cell, 13.0, color, emphasis));
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_recognised() {
        let md = "# Rules\n\nA cell is **alive**\nor dead.\n\n- birth: `B3`\n  continued\n  - nested\n1. first\n2) second\n\n```\ncode *x*\n```\n> quote\n---\n| a | b |\n|---|:-:|\n| 1 | 2 |\ntrailing";
        let blocks = parse_blocks(md);
        assert_eq!(
            blocks,
            vec![
                Block::Heading { level: 1, text: "Rules".into() },
                Block::Paragraph("A cell is **alive** or dead.".into()),
                Block::ListItem { depth: 0, number: None, text: "birth: `B3` continued".into() },
                Block::ListItem { depth: 1, number: None, text: "nested".into() },
                Block::ListItem { depth: 0, number: Some(1), text: "first".into() },
                Block::ListItem { depth: 0, number: Some(2), text: "second".into() },
                Block::Code("code *x*".into()),
                Block::Quote("quote".into()),
                Block::Rule,
                Block::Table { header: vec!["a".into(), "b".into()], rows: vec![vec!["1".into(), "2".into()]] },
                Block::Paragraph("trailing".into()),
            ]
        );
    }

    #[test]
    fn headings_need_a_space_and_unclosed_fences_end_at_eof() {
        assert_eq!(parse_blocks("#hashtag"), vec![Block::Paragraph("#hashtag".into())]);
        assert_eq!(parse_blocks("```\nopen"), vec![Block::Code("open".into())]);
        assert_eq!(parse_blocks("### Three ###"), vec![Block::Heading { level: 3, text: "Three".into() }]);
    }

    #[test]
    fn inline_styles() {
        let spans = parse_inline("a **b `c`** *d* `e*f` 2 * 3");
        let summary: Vec<(String, bool, bool, bool)> =
            spans.into_iter().map(|s| (s.text, s.bold, s.italic, s.code)).collect();
        assert_eq!(
            summary,
            vec![
                ("a ".into(), false, false, false),
                ("b ".into(), true, false, false),
                ("c".into(), true, false, true),
                (" ".into(), false, false, false),
                ("d".into(), false, true, false),
                (" ".into(), false, false, false),
                ("e*f".into(), false, false, true),
                (" 2 * 3".into(), false, false, false),
            ]
        );
        assert_eq!(parse_inline("**unclosed"), vec![Span::plain("**unclosed")]);
        assert_eq!(parse_inline("`open"), vec![Span::plain("`open")]);
    }

    #[test]
    fn inline_text_with_multibyte_characters() {
        // Rule texts and concepts use symbols such as `–`, `→`, `μ` and `σ`.
        let spans = parse_inline("predator–prey: μ → *σ* ≥ **½**");
        let summary: Vec<(String, bool, bool)> = spans.into_iter().map(|s| (s.text, s.bold, s.italic)).collect();
        assert_eq!(
            summary,
            vec![
                ("predator–prey: μ → ".into(), false, false),
                ("σ".into(), false, true),
                (" ≥ ".into(), false, false),
                ("½".into(), true, false),
            ]
        );
        assert_eq!(parse_inline("é*"), vec![Span::plain("é*")]);
    }

    /// Every string of up to five symbols from an alphabet of markers,
    /// spaces, ASCII and multi-byte letters: parsing never panics and never
    /// drops or reorders the non-marker text.
    #[test]
    fn inline_parsing_is_total_and_keeps_the_text() {
        let alphabet = ['*', '`', ' ', 'a', 'μ', '–'];
        let strip = |s: &str| s.chars().filter(|c| !matches!(c, '*' | '`')).collect::<String>();
        let mut stack = vec![String::new()];
        let mut checked = 0;
        while let Some(text) = stack.pop() {
            let spans = parse_inline(&text);
            let shown: String = spans.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(strip(&shown), strip(&text), "{text:?}");
            checked += 1;
            if text.chars().count() < 5 {
                for c in alphabet {
                    stack.push(format!("{text}{c}"));
                }
            }
        }
        assert_eq!(checked, (0..=5).map(|n| 6usize.pow(n)).sum::<usize>());
    }

    #[test]
    fn builtin_texts_render() {
        let ctx = egui::Context::default();
        let tutorial = crate::tutorial::dashboard_tutorial();
        let _ = crate::run_headless_ui(&ctx, |ui| {
            markdown(ui, &tutorial.concepts);
            for page in &tutorial.pages {
                markdown(ui, &page.body);
            }
        });
    }

    #[test]
    fn rendering_never_panics_on_odd_input() {
        let ctx = egui::Context::default();
        let _ = crate::run_headless_ui(&ctx, |ui| {
            markdown(ui, "| only header |\n|---|\n\n####### seven\n* \n1.\n```");
        });
    }
}
