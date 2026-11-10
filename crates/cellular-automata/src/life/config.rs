//! Configuration of [`super::LifeSim`]: rule presets, world, engine, initial
//! content, and the pattern library that resolves pattern ids.

use std::path::Path;

use patterns::{Catalog, LifeRule, Pattern};
use serde::{Deserialize, Serialize};
use simulation_core::Topology;

use crate::engine::EngineChoice;
use crate::error::CaError;
use crate::geometry::{CellRect, Rotation, Symmetry};

/// Named Life-like rules offered by the rule panel. Presets are constants:
/// editing the rule never modifies a preset, it selects
/// [`RulePreset::Custom`] (or the preset whose rule was typed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulePreset {
    /// Conway's Game of Life, `B3/S23`, unaltered.
    #[default]
    Conway,
    /// HighLife `B36/S23` (has a replicator).
    #[serde(rename = "highlife")]
    HighLife,
    /// Seeds `B2/S` (no cell survives).
    Seeds,
    /// Day & Night `B3678/S34678` (symmetric under inversion).
    DayAndNight,
    /// Life without Death `B3/S012345678` (cells never die).
    LifeWithoutDeath,
    /// Maze `B3/S12345`.
    Maze,
    /// 2×2 `B36/S125`.
    TwoByTwo,
    /// Replicator `B1357/S1357` (every pattern replicates).
    Replicator,
    /// Diamoeba `B35678/S5678`.
    Diamoeba,
    /// Morley (Move) `B368/S245`.
    Morley,
    /// Anneal `B4678/S35678` (twisted majority).
    Anneal,
    /// A rule typed by the user.
    Custom,
}

impl RulePreset {
    /// Every preset, in menu order.
    pub const ALL: [RulePreset; 12] = [
        Self::Conway,
        Self::HighLife,
        Self::Seeds,
        Self::DayAndNight,
        Self::LifeWithoutDeath,
        Self::Maze,
        Self::TwoByTwo,
        Self::Replicator,
        Self::Diamoeba,
        Self::Morley,
        Self::Anneal,
        Self::Custom,
    ];

    /// Machine name used in parameters and scenario files.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conway => "conway",
            Self::HighLife => "highlife",
            Self::Seeds => "seeds",
            Self::DayAndNight => "day_and_night",
            Self::LifeWithoutDeath => "life_without_death",
            Self::Maze => "maze",
            Self::TwoByTwo => "two_by_two",
            Self::Replicator => "replicator",
            Self::Diamoeba => "diamoeba",
            Self::Morley => "morley",
            Self::Anneal => "anneal",
            Self::Custom => "custom",
        }
    }

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Conway => "Conway (B3/S23, unaltered)",
            Self::HighLife => "HighLife (B36/S23)",
            Self::Seeds => "Seeds (B2/S)",
            Self::DayAndNight => "Day & Night (B3678/S34678)",
            Self::LifeWithoutDeath => "Life without Death (B3/S012345678)",
            Self::Maze => "Maze (B3/S12345)",
            Self::TwoByTwo => "2×2 (B36/S125)",
            Self::Replicator => "Replicator (B1357/S1357)",
            Self::Diamoeba => "Diamoeba (B35678/S5678)",
            Self::Morley => "Morley (B368/S245)",
            Self::Anneal => "Anneal (B4678/S35678)",
            Self::Custom => "Custom rule",
        }
    }

    /// The preset's rule (`None` for [`RulePreset::Custom`]).
    pub fn rule(self) -> Option<LifeRule> {
        let text = match self {
            Self::Conway => "B3/S23",
            Self::HighLife => "B36/S23",
            Self::Seeds => "B2/S",
            Self::DayAndNight => "B3678/S34678",
            Self::LifeWithoutDeath => "B3/S012345678",
            Self::Maze => "B3/S12345",
            Self::TwoByTwo => "B36/S125",
            Self::Replicator => "B1357/S1357",
            Self::Diamoeba => "B35678/S5678",
            Self::Morley => "B368/S245",
            Self::Anneal => "B4678/S35678",
            Self::Custom => return None,
        };
        LifeRule::parse(text).ok()
    }

    /// The first preset running `rule`, or [`RulePreset::Custom`].
    pub fn matching(rule: LifeRule) -> Self {
        Self::ALL.into_iter().find(|p| p.rule() == Some(rule)).unwrap_or(Self::Custom)
    }

    /// Parses a machine name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == name)
    }
}

/// Where the cells of a pattern come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatternSource {
    /// A pattern id of the [`PatternLibrary`] (embedded classics first, then
    /// the asset catalog).
    Library(String),
    /// Pattern text in any format `patterns::parse_any` reads (RLE,
    /// plaintext, Life 1.05, Life 1.06).
    Text(String),
}

/// One step of building the initial state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InitItem {
    /// Places a pattern: its cells are rotated/mirrored inside their own
    /// bounding box (top-left corner fixed), then cell `(cx, cy)` goes to
    /// `(x + cx, y + cy)`. Any non-zero state is a live cell.
    Pattern {
        /// The pattern.
        source: PatternSource,
        /// Horizontal offset.
        #[serde(default)]
        x: i64,
        /// Vertical offset.
        #[serde(default)]
        y: i64,
        /// Clockwise rotation.
        #[serde(default)]
        rotation: Rotation,
        /// Left-right mirror, applied before the rotation.
        #[serde(default)]
        mirror: bool,
    },
    /// Replaces a region by a random soup (see [`crate::geometry::random_fill`]).
    Random {
        /// Region (default: the whole finite world, or the home region of the
        /// infinite plane).
        #[serde(default)]
        region: Option<CellRect>,
        /// Probability of a live cell.
        density: f64,
        /// Symmetry of the soup.
        #[serde(default)]
        symmetry: Symmetry,
    },
}

/// Initial-state step with patterns resolved to cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum ResolvedInit {
    /// Live cells to set.
    Cells(Vec<(i64, i64)>),
    /// A random fill.
    Random { region: Option<CellRect>, density: f64, symmetry: Symmetry },
}

fn default_topology() -> Topology {
    Topology::Torus
}

fn default_side() -> u32 {
    256
}

fn one() -> u64 {
    1
}

/// Everything needed to build a [`super::LifeSim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifeConfig {
    /// Rule preset (ignored when `rule` is given).
    #[serde(default)]
    pub preset: RulePreset,
    /// Explicit rule in any notation `LifeRule::parse` accepts; selects the
    /// matching preset or [`RulePreset::Custom`].
    #[serde(default)]
    pub rule: Option<String>,
    /// Edge behaviour.
    #[serde(default = "default_topology")]
    pub topology: Topology,
    /// World width (home-region width on the infinite plane).
    #[serde(default = "default_side")]
    pub width: u32,
    /// World height (home-region height on the infinite plane).
    #[serde(default = "default_side")]
    pub height: u32,
    /// Engine choice.
    #[serde(default)]
    pub engine: EngineChoice,
    /// Generations per tick (a power of two when Hashlife runs).
    #[serde(default = "one")]
    pub generations_per_tick: u64,
    /// Track cell ages (view channel G).
    #[serde(default)]
    pub track_ages: bool,
    /// Track activity trails (view channel B).
    #[serde(default)]
    pub track_activity: bool,
    /// Initial content, applied in order.
    #[serde(default)]
    pub init: Vec<InitItem>,
}

impl Default for LifeConfig {
    fn default() -> Self {
        Self {
            preset: RulePreset::Conway,
            rule: None,
            topology: Topology::Torus,
            width: 256,
            height: 256,
            engine: EngineChoice::Auto,
            generations_per_tick: 1,
            track_ages: false,
            track_activity: false,
            init: vec![InitItem::Random { region: None, density: 0.3, symmetry: Symmetry::None }],
        }
    }
}

impl LifeConfig {
    /// The rule and preset this configuration selects.
    ///
    /// # Errors
    /// An unparsable `rule`, or [`RulePreset::Custom`] without a rule.
    pub fn resolve_rule(&self) -> Result<(LifeRule, RulePreset), CaError> {
        match &self.rule {
            Some(text) => {
                let rule = LifeRule::parse(text)?;
                Ok((rule, RulePreset::matching(rule)))
            }
            None => match self.preset.rule() {
                Some(rule) => Ok((rule, self.preset)),
                None => Err(CaError::invalid("the custom preset needs an explicit `rule`")),
            },
        }
    }
}

/// Resolves pattern ids: the patterns embedded in this crate
/// ([`crate::library`], e.g. `wireworld/diode`), then those embedded in the
/// `patterns` crate (`glider`, `gosper-glider-gun`...), then an optional asset
/// catalog (`assets/patterns/catalog.toml`).
#[derive(Clone, Debug, Default)]
pub struct PatternLibrary {
    catalog: Option<Catalog>,
}

impl PatternLibrary {
    /// Only the embedded classics (glider, guns, methuselahs...).
    pub fn builtin() -> Self {
        Self { catalog: None }
    }

    /// Embedded classics plus `catalog`.
    pub fn with_catalog(catalog: Catalog) -> Self {
        Self { catalog: Some(catalog) }
    }

    /// Loads the asset catalog from `path` (the catalog file or its directory).
    ///
    /// # Errors
    /// The catalog cannot be read or is malformed.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CaError> {
        Ok(Self::with_catalog(Catalog::load(path)?))
    }

    /// The asset catalog, if one was loaded.
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref()
    }

    /// Looks a pattern up by id: embedded patterns, then the asset catalog's
    /// entries, then a pattern file `<catalog directory>/<id>.rle` (used for
    /// separately licensed data that is not compiled in, such as
    /// `wireworld/and-gate-mcintosh`).
    ///
    /// # Errors
    /// Unknown ids (the message says whether an asset catalog was available)
    /// and unreadable pattern files.
    pub fn resolve(&self, id: &str) -> Result<Pattern, CaError> {
        if let Some(text) = crate::library::embedded_pattern(id) {
            return Ok(patterns::rle::parse(text)?);
        }
        if let Some(builtin) = patterns::builtin_patterns().iter().find(|p| p.id == id) {
            return Ok(builtin.pattern()?);
        }
        match &self.catalog {
            Some(catalog) => match catalog.by_id(id) {
                Some(entry) => Ok(catalog.load_pattern(entry)?),
                None => match catalog.root().and_then(|root| asset_file(root, id)) {
                    Some(path) => {
                        let text = std::fs::read_to_string(&path)
                            .map_err(|e| CaError::Io { path: path.display().to_string(), message: e.to_string() })?;
                        Ok(patterns::rle::parse(&text)?)
                    }
                    None => Err(CaError::invalid(format!("unknown pattern id `{id}`"))),
                },
            },
            None => Err(CaError::invalid(format!(
                "pattern `{id}` is not embedded; load the asset catalog (assets/patterns/catalog.toml) to use it"
            ))),
        }
    }

    /// Resolves a [`PatternSource`].
    ///
    /// # Errors
    /// See [`PatternLibrary::resolve`] and `patterns::parse_any`.
    pub fn load_source(&self, source: &PatternSource) -> Result<Pattern, CaError> {
        match source {
            PatternSource::Library(id) => self.resolve(id),
            PatternSource::Text(text) => Ok(patterns::parse_any(text, None)?),
        }
    }
}

/// Places `pattern` as described by [`InitItem::Pattern`]: returns its
/// non-zero cells `(x, y, state)` after mirroring, rotating and translating,
/// sorted row-major (where a pattern lists a position twice, the entry
/// listed first is kept).
///
/// # Errors
/// [`CaError::InvalidArgument`] when a cell would leave the 64-bit plane.
pub fn place_pattern(
    pattern: &Pattern,
    x: i64,
    y: i64,
    rotation: Rotation,
    mirror: bool,
) -> Result<Vec<(i64, i64, u8)>, CaError> {
    let mut p = pattern.clone();
    if mirror {
        p = p.flip_horizontal();
    }
    for _ in 0..rotation.quarter_turns() {
        p = p.rotate_cw();
    }
    let mut cells = p
        .cells
        .iter()
        .filter(|c| c.state != 0)
        .map(|c| match (c.x.checked_add(x), c.y.checked_add(y)) {
            (Some(cx), Some(cy)) => Ok((cx, cy, c.state)),
            _ => Err(CaError::invalid(format!(
                "the pattern placed at ({x}, {y}) would leave the 64-bit coordinate plane"
            ))),
        })
        .collect::<Result<Vec<_>, CaError>>()?;
    cells.sort_by_key(|&(cx, cy, _)| (cy, cx));
    cells.dedup_by_key(|c| (c.0, c.1));
    Ok(cells)
}

/// `<root>/<id>.rle` when `id` is a plain relative id (no `..`, no root) and
/// the file exists.
fn asset_file(root: &Path, id: &str) -> Option<std::path::PathBuf> {
    let relative = Path::new(id);
    let plain = !id.is_empty() && relative.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    let path = root.join(format!("{id}.rle"));
    (plain && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_constant_and_matched() {
        assert_eq!(RulePreset::Conway.rule(), Some(LifeRule::conway()));
        assert_eq!(RulePreset::matching(LifeRule::parse("B36/S23").unwrap()), RulePreset::HighLife);
        assert_eq!(RulePreset::matching(LifeRule::parse("B37/S23").unwrap()), RulePreset::Custom);
        for p in RulePreset::ALL {
            assert_eq!(RulePreset::parse(p.as_str()), Some(p));
            if p != RulePreset::Custom {
                assert_eq!(RulePreset::matching(p.rule().unwrap()), p, "{p:?}");
            }
        }
    }

    #[test]
    fn config_rule_resolution() {
        let mut c = LifeConfig { rule: Some("23/36".into()), ..LifeConfig::default() };
        assert_eq!(c.resolve_rule().unwrap(), (LifeRule::highlife(), RulePreset::HighLife));
        c.rule = None;
        c.preset = RulePreset::Custom;
        assert!(c.resolve_rule().is_err());
        c.rule = Some("B3/S2x".into());
        assert!(c.resolve_rule().is_err());
    }

    #[test]
    fn library_resolution_and_placement() {
        let lib = PatternLibrary::builtin();
        let glider = lib.resolve("glider").unwrap();
        assert_eq!(glider.population(), 5);
        assert!(lib.resolve("no-such-pattern").unwrap_err().to_string().contains("asset catalog"));
        let text = lib.load_source(&PatternSource::Text("x = 3, y = 1\n3o!".into())).unwrap();
        let placed = place_pattern(&text, 10, 20, Rotation::Cw90, false).unwrap();
        assert_eq!(placed, vec![(10, 20, 1), (10, 21, 1), (10, 22, 1)]);
        let err = place_pattern(&text, i64::MAX - 1, 0, Rotation::None, false).unwrap_err().to_string();
        assert!(err.contains("64-bit"), "{err}");
    }
}
