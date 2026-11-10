//! Syntax highlighting of rule DSL and Rhai text for the code editors, and
//! the mapping of 1-based `line:col` diagnostics to text ranges.

use std::ops::Range;

use bevy_egui::egui::text::{LayoutJob, TextFormat};
use bevy_egui::egui::{Color32, FontId, Stroke};
use rule_engine::compile::vocab::{self, FunctionKind};
use rule_engine::dsl::parser::KEYWORDS;

/// Token classes shown in different colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// Language keyword (`rule`, `when`, `then`, modifiers, `fn`, `let`...).
    Keyword,
    /// Event keyword (`on_collision`...).
    Event,
    /// Sensor function (`nearby`, `field`...).
    Sensor,
    /// Action function (`eat`, `spawn`...).
    Action,
    /// A name defined by the system (type, resource, field, state).
    Name,
    /// Built-in property or subject (`energy`, `self`, `other`...).
    Property,
    /// Number literal.
    Number,
    /// String literal.
    Str,
    /// Comment.
    Comment,
    /// Operator.
    Operator,
    /// Punctuation.
    Punct,
    /// Any other identifier.
    Ident,
    /// Whitespace.
    Space,
}

/// A classified slice of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Byte range.
    pub range: Range<usize>,
    /// Class.
    pub class: Class,
}

const SUBJECTS: [&str; 7] = ["self", "other", "organism", "world", "tick", "state", "type"];
const KIND_WORDS: [&str; 4] = ["same_species", "same_type", "same", "any"];

fn scan(text: &str, comment: impl Fn(&str) -> Option<usize>, classify: impl Fn(&str) -> Class) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap_or(' ');
        let (len, class) = if c.is_whitespace() {
            (rest.find(|ch: char| !ch.is_whitespace()).unwrap_or(rest.len()), Class::Space)
        } else if let Some(len) = comment(rest) {
            (len, Class::Comment)
        } else if c == '"' || c == '`' {
            let mut end = rest.len();
            let mut escaped = false;
            for (j, ch) in rest.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == c {
                    end = j + ch.len_utf8();
                    break;
                } else if ch == '\n' && c == '"' {
                    end = j;
                    break;
                }
            }
            (end, Class::Str)
        } else if c.is_ascii_digit() {
            let mut end = 0;
            let b = rest.as_bytes();
            while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'_') {
                end += 1;
            }
            if end + 1 < b.len() && b[end] == b'.' && b[end + 1].is_ascii_digit() {
                end += 1;
                while end < b.len() && b[end].is_ascii_digit() {
                    end += 1;
                }
            }
            if end < b.len() && (b[end] == b'e' || b[end] == b'E') {
                let mut k = end + 1;
                if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
                    k += 1;
                }
                if k < b.len() && b[k].is_ascii_digit() {
                    end = k;
                    while end < b.len() && b[end].is_ascii_digit() {
                        end += 1;
                    }
                }
            }
            (end, Class::Number)
        } else if c.is_ascii_alphabetic() || c == '_' {
            let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).unwrap_or(rest.len());
            (end, classify(&rest[..end]))
        } else {
            let two = rest.get(..2).unwrap_or("");
            if matches!(two, "==" | "!=" | "<=" | ">=" | "+=" | "-=" | "&&" | "||" | "=>" | "::" | "..") {
                (2, Class::Operator)
            } else if "+-*/<>=!%&|^?".contains(c) {
                (c.len_utf8(), Class::Operator)
            } else {
                (c.len_utf8(), Class::Punct)
            }
        };
        let len = len.max(c.len_utf8());
        out.push(Token { range: i..i + len, class });
        i += len;
        debug_assert!(i <= bytes.len());
    }
    out
}

fn line_comment(rest: &str, hash: bool) -> Option<usize> {
    let starts = (hash && rest.starts_with('#')) || rest.starts_with("//");
    starts.then(|| rest.find('\n').unwrap_or(rest.len()))
}

/// Tokens of rule DSL text. `is_name` recognises the system's own names
/// (types, resources, fields, states, user properties).
pub fn dsl_tokens(text: &str, is_name: &dyn Fn(&str) -> bool) -> Vec<Token> {
    scan(
        text,
        |rest| line_comment(rest, true),
        |word| {
            let lower = word.to_ascii_lowercase();
            if lower.starts_with("on_") && KEYWORDS.contains(&lower.as_str()) {
                Class::Event
            } else if KEYWORDS.contains(&lower.as_str()) {
                Class::Keyword
            } else if let Some(f) = vocab::function(word) {
                if f.kind == FunctionKind::Sensor { Class::Sensor } else { Class::Action }
            } else if vocab::BUILTIN_PROPERTIES.iter().any(|p| p.name == word)
                || SUBJECTS.contains(&word)
                || KIND_WORDS.contains(&word)
            {
                Class::Property
            } else if is_name(word) {
                Class::Name
            } else {
                Class::Ident
            }
        },
    )
}

/// Rhai keywords.
pub const RHAI_KEYWORDS: &[&str] = &[
    "fn", "let", "const", "if", "else", "while", "loop", "for", "in", "return", "break", "continue", "true", "false",
    "switch", "do", "until", "throw", "try", "catch", "this", "private", "global",
];

/// Functions of the script API (queries first, then actions).
pub const RHAI_QUERIES: &[&str] = &["nearby", "nearest", "field", "population", "other", "tick", "random", "prop"];
/// Action functions of the script API (they record intents).
pub const RHAI_ACTIONS: &[&str] = &[
    "move_towards",
    "move_away",
    "wander",
    "stay",
    "eat",
    "attack",
    "die",
    "reproduce",
    "reproduce_with",
    "transform_into",
    "set_state",
    "emit",
    "consume",
    "set_prop",
    "add_prop",
    "spawn_entity",
];

/// Tokens of Rhai source.
pub fn rhai_tokens(text: &str) -> Vec<Token> {
    scan(
        text,
        |rest| {
            if rest.starts_with("/*") {
                Some(rest.find("*/").map_or(rest.len(), |e| e + 2))
            } else {
                line_comment(rest, false)
            }
        },
        |word| {
            if RHAI_KEYWORDS.contains(&word) {
                Class::Keyword
            } else if RHAI_ACTIONS.contains(&word) {
                Class::Action
            } else if RHAI_QUERIES.contains(&word) {
                Class::Sensor
            } else if word == "me" {
                Class::Property
            } else {
                Class::Ident
            }
        },
    )
}

/// Byte range of the text covered by a 1-based `line:col` span of `len`
/// characters (at least one character; a column past the end of its line
/// marks the line end). `None` for line 0 or a line beyond the text.
pub fn span_range(text: &str, line: u32, col: u32, len: u32) -> Option<Range<usize>> {
    if line == 0 {
        return None;
    }
    let mut start_of_line = 0usize;
    for _ in 1..line {
        let nl = text[start_of_line..].find('\n')?;
        start_of_line += nl + 1;
    }
    let line_text = &text[start_of_line..];
    let line_end = start_of_line + line_text.find('\n').unwrap_or(line_text.len());
    let char_starts: Vec<usize> =
        text[start_of_line..line_end].char_indices().map(|(i, _)| start_of_line + i).chain([line_end]).collect();
    let col = col.max(1) as usize;
    let start = char_starts.get(col - 1).copied().unwrap_or(line_end);
    if start >= line_end {
        // Past the end: highlight the last character of the line (or nothing on an empty line).
        let prev = text[start_of_line..line_end].char_indices().last().map(|(i, _)| start_of_line + i);
        return Some(prev.map_or(line_end..line_end, |p| p..line_end));
    }
    let end_index = (col - 1 + len.max(1) as usize).min(char_starts.len() - 1);
    Some(start..char_starts[end_index].max(start + 1).min(line_end))
}

/// Colours of the code editors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodeColors {
    /// Default text.
    pub text: Color32,
    /// Keywords.
    pub keyword: Color32,
    /// Events.
    pub event: Color32,
    /// Sensors.
    pub sensor: Color32,
    /// Actions.
    pub action: Color32,
    /// System names.
    pub name: Color32,
    /// Properties.
    pub property: Color32,
    /// Numbers.
    pub number: Color32,
    /// Strings.
    pub string: Color32,
    /// Comments.
    pub comment: Color32,
    /// Operators and punctuation.
    pub operator: Color32,
    /// Error underline.
    pub error: Color32,
}

impl CodeColors {
    /// Colours for a dark or light theme.
    pub fn new(dark: bool, text: Color32, muted: Color32, accent: Color32, error: Color32) -> Self {
        let pick = |d: (u8, u8, u8), l: (u8, u8, u8)| {
            let (r, g, b) = if dark { d } else { l };
            Color32::from_rgb(r, g, b)
        };
        Self {
            text,
            keyword: accent,
            event: pick((255, 160, 110), (180, 80, 20)),
            sensor: pick((110, 200, 250), (20, 100, 170)),
            action: pick((130, 220, 150), (20, 130, 60)),
            name: pick((245, 205, 110), (150, 100, 0)),
            property: pick((190, 180, 255), (90, 70, 190)),
            number: pick((240, 150, 190), (170, 40, 110)),
            string: pick((200, 220, 140), (90, 120, 20)),
            comment: muted,
            operator: pick((200, 205, 215), (70, 75, 85)),
            error,
        }
    }

    fn of(&self, class: Class) -> Color32 {
        match class {
            Class::Keyword => self.keyword,
            Class::Event => self.event,
            Class::Sensor => self.sensor,
            Class::Action => self.action,
            Class::Name => self.name,
            Class::Property => self.property,
            Class::Number => self.number,
            Class::Str => self.string,
            Class::Comment => self.comment,
            Class::Operator | Class::Punct => self.operator,
            Class::Ident | Class::Space => self.text,
        }
    }
}

/// A layout job colouring `tokens` of `text`; bytes inside `errors` are
/// underlined in the error colour.
pub fn layout_job(
    text: &str,
    tokens: &[Token],
    colors: &CodeColors,
    font: FontId,
    errors: &[Range<usize>],
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut cuts: Vec<usize> = tokens.iter().flat_map(|t| [t.range.start, t.range.end]).collect();
    for e in errors {
        cuts.extend([e.start.min(text.len()), e.end.min(text.len())]);
    }
    cuts.push(0);
    cuts.push(text.len());
    cuts.retain(|&c| text.is_char_boundary(c));
    cuts.sort_unstable();
    cuts.dedup();
    let mut token_index = 0;
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a == b {
            continue;
        }
        while token_index < tokens.len() && tokens[token_index].range.end <= a {
            token_index += 1;
        }
        let class = tokens.get(token_index).filter(|t| t.range.start <= a).map_or(Class::Ident, |t| t.class);
        let in_error = errors.iter().any(|e| e.start <= a && a < e.end.max(e.start + 1));
        let mut format = TextFormat { font_id: font.clone(), color: colors.of(class), ..Default::default() };
        if in_error {
            format.underline = Stroke::new(1.5, colors.error);
            format.background = colors.error.gamma_multiply(0.18);
        }
        job.append(&text[a..b], 0.0, format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<(String, Class)> {
        let names = |w: &str| w == "rabbit" || w == "fox" || w == "grass";
        dsl_tokens(text, &names)
            .into_iter()
            .filter(|t| t.class != Class::Space)
            .map(|t| (text[t.range.clone()].to_string(), t.class))
            .collect()
    }

    #[test]
    fn dsl_tokens_are_classified() {
        let toks = classes(
            "rule flee for rabbit: # run\n  on_collision with fox when nearby(fox, radius=6) >= 1.5e1 and self.energy < \"x\" then move_away(nearest(fox))",
        );
        let find = |w: &str| toks.iter().find(|(t, _)| t == w).map(|(_, c)| *c);
        assert_eq!(find("rule"), Some(Class::Keyword));
        assert_eq!(find("rabbit"), Some(Class::Name));
        assert_eq!(find("# run"), Some(Class::Comment));
        assert_eq!(find("on_collision"), Some(Class::Event));
        assert_eq!(find("nearby"), Some(Class::Sensor));
        assert_eq!(find("move_away"), Some(Class::Action));
        assert_eq!(find("radius"), Some(Class::Ident));
        assert_eq!(find(">="), Some(Class::Operator));
        assert_eq!(find("1.5e1"), Some(Class::Number));
        assert_eq!(find("self"), Some(Class::Property));
        assert_eq!(find("energy"), Some(Class::Property));
        assert_eq!(find("\"x\""), Some(Class::Str));
    }

    #[test]
    fn tokens_cover_the_text_exactly() {
        for text in ["rule a for b: then die()", "fn run(me) { /* c */ let x = \"s\"; } // tail", "ünïcode → x", ""]
        {
            let toks = rhai_tokens(text);
            let mut pos = 0;
            for t in &toks {
                assert_eq!(t.range.start, pos, "{text}");
                pos = t.range.end;
            }
            assert_eq!(pos, text.len());
        }
        let toks = rhai_tokens("fn run(me) { stay(); nearby(\"robot\", 2.5) }");
        assert_eq!(toks[0].class, Class::Keyword);
        assert!(toks.iter().any(|t| t.class == Class::Action));
        assert!(toks.iter().any(|t| t.class == Class::Sensor));
        assert!(toks.iter().any(|t| t.class == Class::Property));
    }

    #[test]
    fn spans_map_to_byte_ranges() {
        let text = "rule a for x:\n  when energy > then die()";
        assert_eq!(span_range(text, 2, 17, 4), Some(30..34));
        assert_eq!(&text[30..34], "then");
        assert_eq!(span_range(text, 1, 1, 4), Some(0..4));
        assert_eq!(span_range(text, 3, 1, 1), None);
        assert_eq!(span_range(text, 0, 1, 1), None);
        // Past the end of the line: the last character.
        assert_eq!(span_range(text, 1, 40, 1), Some(12..13));
        // Multi-byte characters count as one column.
        let t = "é = x";
        assert_eq!(span_range(t, 1, 3, 1), Some(3..4));
    }

    #[test]
    fn layout_marks_errors_and_keeps_the_text() {
        let text = "rule a for x: then die()";
        let toks = dsl_tokens(text, &|_| false);
        let colors = CodeColors::new(true, Color32::WHITE, Color32::GRAY, Color32::GOLD, Color32::RED);
        let then = 14..18;
        let job = layout_job(text, &toks, &colors, FontId::monospace(13.0), std::slice::from_ref(&then));
        assert_eq!(job.text, text);
        let underlined: String = job
            .sections
            .iter()
            .filter(|s| s.format.underline.width > 0.0)
            .map(|s| &text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(underlined, "then");
    }
}
