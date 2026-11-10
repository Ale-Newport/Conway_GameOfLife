//! Typed errors and diagnostics.
//!
//! Every problem found in user input (DSL text, definitions, scripts,
//! scenario files) is reported as a [`Diagnostic`] carrying a stable code,
//! the rule it belongs to, a 1-based line/column [`Span`] inside that rule's
//! text and an optional `help` line. Diagnostics are collected, never thrown
//! one at a time, so an editor can show all of them at once.

use std::fmt;

use serde::{Deserialize, Serialize};
use simulation_core::SimError;
use thiserror::Error;

/// Location inside a piece of text: 1-based line and column (in characters)
/// and the length of the highlighted token.
///
/// Spans are *metadata*: two spans always compare equal, so AST values that
/// differ only in where they were written (for example a rule and its
/// pretty-printed re-parse) are equal. They are not serialised.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Span {
    /// 1-based line (0 when unknown, e.g. for generated rules).
    pub line: u32,
    /// 1-based column in characters (0 when unknown).
    pub col: u32,
    /// Number of characters covered (at least 1 for real tokens).
    pub len: u32,
}

impl PartialEq for Span {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Span {
    /// Span at `line:col` covering `len` characters.
    pub fn new(line: u32, col: u32, len: u32) -> Self {
        Self { line, col, len }
    }

    /// Whether the span points at real text.
    pub fn is_known(&self) -> bool {
        self.line > 0
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Severity of a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Blocks compilation.
    Error,
    /// Accepted, but probably not what the author intended.
    Warning,
}

/// The definition item a [`Diagnostic`] belongs to, so that an editor can
/// select the offending entry. Indices refer to the lists of the
/// [`crate::LifeSystem`] that was validated (`u16` keeps diagnostics small;
/// validation caps every list at [`crate::compile::MAX_LIST_LEN`] entries;
/// use [`ItemRef::index`] to get a `usize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ItemRef {
    /// The system as a whole (e.g. names shared across lists).
    System,
    /// `world` settings (size, topology, entity cap).
    World,
    /// `script_limits`.
    ScriptLimits,
    /// `entity_types[i]` (also rules generated from its reproduction settings).
    EntityType(u16),
    /// `world.resources[i]`.
    Resource(u16),
    /// `world.environment[i]`.
    Environment(u16),
    /// `world.random_events[i]`.
    RandomEvent(u16),
    /// `end_conditions[i]`.
    EndCondition(u16),
    /// `relationships[i]` (also the rules generated from it).
    Relationship(u16),
    /// `rules[i]` (a hand-written DSL rule).
    Rule(u16),
    /// `scripts[i]`.
    Script(u16),
}

impl ItemRef {
    /// Index into the list the item belongs to (`None` for `System`,
    /// `World` and `ScriptLimits`).
    pub fn index(self) -> Option<usize> {
        match self {
            Self::System | Self::World | Self::ScriptLimits => None,
            Self::EntityType(i)
            | Self::Resource(i)
            | Self::Environment(i)
            | Self::RandomEvent(i)
            | Self::EndCondition(i)
            | Self::Relationship(i)
            | Self::Rule(i)
            | Self::Script(i) => Some(usize::from(i)),
        }
    }
}

/// `usize` list index as stored in an [`ItemRef`] (validation caps every
/// list at [`crate::compile::MAX_LIST_LEN`] entries, well below `u16::MAX`).
pub(crate) fn item_index(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}

/// One problem found in user input.
///
/// Codes: `E001` unknown property, `E002` type mismatch, `E003` unknown
/// entity type / resource / state / field, `E004` bad arguments, `E005`
/// probability out of range, `E006` sensing radius out of range, `E007`
/// action not allowed for the event, `E008` read-only property, `E009`
/// invalid cost, `E010` two movement or two exclusive actions in one rule,
/// `E012` invalid count, `E013` `other` outside a collision rule, `E014`
/// unknown function or action, `E015` syntax error, `E016` duplicate name,
/// `E017` sessile type asked to move, `E018` type cannot reproduce, `E020`
/// invalid definition, `E030` script compile error. Warnings: `W102`
/// duplicate relationship, `W103` unconditional transformation cycle, `W105`
/// unbounded spawning, `W201` contradictory relationships, `W204`
/// relationship without effect, `W205` competition without shared food.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable code such as `E001`.
    pub code: String,
    /// Error or warning.
    pub severity: Severity,
    /// What is wrong, in user vocabulary.
    pub message: String,
    /// Rule, script or definition the problem belongs to (e.g.
    /// `rule forage (rabbit)` or `entity type rabbit`).
    pub context: Option<String>,
    /// Location inside the rule or script text, when known.
    pub span: Option<Span>,
    /// A concrete suggestion.
    pub help: Option<String>,
    /// The definition item it belongs to (for editors), when known.
    #[serde(default)]
    pub item: Option<ItemRef>,
}

impl Diagnostic {
    /// New error diagnostic.
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: Severity::Error,
            message: message.into(),
            context: None,
            span: None,
            help: None,
            item: None,
        }
    }

    /// New warning diagnostic.
    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, ..Self::error(code, message) }
    }

    /// Attaches a location.
    pub fn at(mut self, span: Span) -> Self {
        if span.is_known() {
            self.span = Some(span);
        }
        self
    }

    /// Attaches the rule/definition context.
    pub fn context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Attaches a help line.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attaches the definition item (kept if one is already set).
    pub fn for_item(mut self, item: ItemRef) -> Self {
        self.item.get_or_insert(item);
        self
    }

    /// Whether this is an error (as opposed to a warning).
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}[{}]", self.code)?;
        if let Some(context) = &self.context {
            write!(f, " in {context}")?;
        }
        if let Some(span) = self.span {
            write!(f, " at {span}")?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(help) = &self.help {
            write!(f, " (help: {help})")?;
        }
        Ok(())
    }
}

/// Renders a list of diagnostics, one per line.
pub fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
}

/// Errors produced by the rule engine.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum RuleEngineError {
    /// Definitions, rules or scripts failed validation.
    #[error("{}", render_diagnostics(.0))]
    Invalid(Vec<Diagnostic>),
    /// A RON document could not be read or written.
    #[error("RON error: {0}")]
    Ron(String),
    /// A JSON document could not be read or written.
    #[error("JSON error: {0}")]
    Json(String),
    /// A definition written by a newer version of the format.
    #[error("definition format version {found} is not supported (this build reads up to {supported})")]
    UnsupportedFormat {
        /// Version found in the document.
        found: u32,
        /// Newest version this build reads.
        supported: u32,
    },
    /// A file or directory could not be read.
    #[error("cannot read `{path}`: {reason}")]
    Io {
        /// File or directory involved.
        path: String,
        /// Operating-system reason.
        reason: String,
    },
    /// A scenario file is invalid.
    #[error("scenario `{file}`: {message}")]
    Scenario {
        /// File name or built-in scenario id.
        file: String,
        /// What is wrong.
        message: String,
    },
}

impl RuleEngineError {
    /// The diagnostics carried by an [`RuleEngineError::Invalid`] error.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Self::Invalid(d) => d,
            _ => &[],
        }
    }
}

impl From<RuleEngineError> for SimError {
    fn from(error: RuleEngineError) -> Self {
        match error {
            RuleEngineError::Invalid(_) | RuleEngineError::UnsupportedFormat { .. } => {
                SimError::InvalidConfig(error.to_string())
            }
            RuleEngineError::Ron(_) | RuleEngineError::Json(_) => SimError::Parse(error.to_string()),
            RuleEngineError::Io { .. } | RuleEngineError::Scenario { .. } => SimError::InvalidConfig(error.to_string()),
        }
    }
}

/// Levenshtein distance (characters), used for "did you mean" hints.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// "did you mean `x`?" for the closest candidate within edit distance 2.
pub(crate) fn did_you_mean<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    candidates
        .into_iter()
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map(|(_, c)| format!("did you mean `{c}`?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_code_context_location_and_help() {
        let d = Diagnostic::error("E001", "unknown property `enrgy`")
            .context("rule forage (rabbit)")
            .at(Span::new(2, 7, 5))
            .help("did you mean `energy`?");
        assert_eq!(
            d.to_string(),
            "error[E001] in rule forage (rabbit) at 2:7: unknown property `enrgy` (help: did you mean `energy`?)"
        );
    }

    #[test]
    fn suggestions_use_edit_distance() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(did_you_mean("enrgy", ["energy", "age"]).as_deref(), Some("did you mean `energy`?"));
        assert_eq!(did_you_mean("zzz", ["energy"]), None);
    }
}
