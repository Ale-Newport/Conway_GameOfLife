//! Scenario files (RON) for continuous worlds: a header, the world size, the
//! rule and a list of initialisation elements (creatures, soups, discs).
//!
//! Positions of initialisation elements are fractions of the world size
//! (`x = 0.5` is the centre column), so a scenario keeps its layout when the
//! world is resized; radii are in cells. RON files may use the
//! `unwrap_variant_newtypes` and `implicit_some` extensions, which are enabled
//! by default: `rule: Lenia(radius: 13.0, ...)`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use simulation_core::{Grid2, SimRng};

use crate::edit::{self, Blend, CellData, CreatureCells, Placement, Region};
use crate::error::ContinuousError;
use crate::rule::Rule;

/// Smallest world side.
pub const MIN_SIDE: usize = 16;
/// Largest world side.
pub const MAX_SIDE: usize = 4096;
/// Largest world area in cells.
pub const MAX_CELLS: usize = 4096 * 4096;
/// Largest scenario file read by [`load_file`].
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

fn one() -> f32 {
    1.0
}

/// Identity of a scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioHeader {
    /// Stable identifier, e.g. `"lenia.orbium"`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows.
    pub description: String,
    /// Seed of the world's random generator.
    pub seed: u64,
}

/// Shape of a random soup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoupShape {
    /// Disc of the given radius.
    #[default]
    Disc,
    /// Square of side `2·radius`.
    Square,
}

/// One step of a scenario's initial state, applied in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InitElement {
    /// Places creature cells (exact copy when unrotated and unscaled).
    Creature {
        /// Name shown in creature pickers.
        name: String,
        /// Cell data.
        cells: CellData,
        /// Centre column as a fraction of the width.
        x: f32,
        /// Centre row as a fraction of the height.
        y: f32,
        /// Clockwise rotation in degrees.
        #[serde(default)]
        rotation: f32,
        /// Scale factor.
        #[serde(default = "one")]
        scale: f32,
    },
    /// Random matter (see [`edit::randomize`]).
    Soup {
        /// Centre column as a fraction of the width.
        x: f32,
        /// Centre row as a fraction of the height.
        y: f32,
        /// Radius (or half side) in cells.
        radius: f32,
        /// Disc or square.
        #[serde(default)]
        shape: SoupShape,
        /// Probability that a cell receives matter.
        density: f32,
        /// Values are uniform in `[0, amplitude)`.
        #[serde(default = "one")]
        amplitude: f32,
    },
    /// Random filled discs (see [`edit::random_discs`]).
    RandomDiscs {
        /// Number of discs.
        count: u32,
        /// Smallest radius in cells.
        min_radius: f32,
        /// Largest radius in cells.
        max_radius: f32,
        /// Value inside the discs.
        #[serde(default = "one")]
        value: f32,
    },
}

/// A complete scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Identity and seed.
    pub header: ScenarioHeader,
    /// World width in cells.
    pub width: usize,
    /// World height in cells.
    pub height: usize,
    /// Rule executed.
    pub rule: Rule,
    /// Initial state, built in order from an empty world.
    #[serde(default)]
    pub init: Vec<InitElement>,
    /// Provenance and licence notices of embedded data.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A named creature from a scenario, ready to place with
/// [`crate::ContinuousCommand::PlaceCreature`].
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryCreature {
    /// Name of the creature.
    pub name: String,
    /// Scenario that defines it (and its native rule).
    pub scenario_id: String,
    /// Decoded cells.
    pub cells: CreatureCells,
}

/// Checks a world size.
pub fn validate_size(width: usize, height: usize) -> Result<(), ContinuousError> {
    let reason = if !(MIN_SIDE..=MAX_SIDE).contains(&width) || !(MIN_SIDE..=MAX_SIDE).contains(&height) {
        Some(format!("each side must be between {MIN_SIDE} and {MAX_SIDE} cells"))
    } else if width * height > MAX_CELLS {
        Some(format!("at most {MAX_CELLS} cells are supported"))
    } else {
        None
    };
    match reason {
        Some(reason) => Err(ContinuousError::InvalidWorld { width, height, reason }),
        None => Ok(()),
    }
}

impl Scenario {
    /// Validates the header, size, rule and every initialisation element
    /// (values are checked against [`Rule::edit_limit`]; whether creatures
    /// and kernels fit the world is checked when the world is built).
    pub fn validate(&self) -> Result<(), ContinuousError> {
        if self.header.id.trim().is_empty() || self.header.name.trim().is_empty() {
            return Err(ContinuousError::rule("scenario id and name must not be empty"));
        }
        validate_size(self.width, self.height)?;
        self.rule.validate()?;
        let limit = self.rule.edit_limit();
        for (i, element) in self.init.iter().enumerate() {
            let fail = |m: String| Err(ContinuousError::rule(format!("init element {i}: {m}")));
            let (x, y) = match element {
                InitElement::Creature { cells, x, y, rotation, scale, .. } => {
                    let decoded = cells.decode()?;
                    if decoded.channels() > self.rule.channels() {
                        return fail(format!(
                            "the creature has {} channels but the rule has {}",
                            decoded.channels(),
                            self.rule.channels()
                        ));
                    }
                    if !rotation.is_finite() || !(scale.is_finite() && (0.05..=32.0).contains(scale)) {
                        return fail(format!(
                            "rotation must be finite and scale in [0.05, 32] (got {rotation}, {scale})"
                        ));
                    }
                    (*x, *y)
                }
                InitElement::Soup { x, y, radius, density, amplitude, .. } => {
                    let valid = radius.is_finite()
                        && *radius > 0.0
                        && (0.0..=1.0).contains(density)
                        && amplitude.is_finite()
                        && (0.0..=limit).contains(amplitude);
                    if !valid {
                        return fail(format!(
                            "soup radius must be positive, density in [0, 1] and amplitude in [0, {limit}]"
                        ));
                    }
                    (*x, *y)
                }
                InitElement::RandomDiscs { count, min_radius, max_radius, value } => {
                    let radii = min_radius.is_finite() && max_radius.is_finite() && 0.0 < *min_radius;
                    if !(radii && min_radius <= max_radius) {
                        return fail(format!("disc radii must satisfy 0 < min ≤ max (got {min_radius}, {max_radius})"));
                    }
                    if *count > edit::MAX_DISCS {
                        return fail(format!("at most {} discs can be drawn (got {count})", edit::MAX_DISCS));
                    }
                    if !(value.is_finite() && (0.0..=limit).contains(value)) {
                        return fail(format!("disc value {value} is outside [0, {limit}]"));
                    }
                    (0.0, 0.0)
                }
            };
            if !((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)) {
                return fail(format!("position ({x}, {y}) must be fractions of the world size in [0, 1]"));
            }
        }
        Ok(())
    }

    /// Builds the initial channels of a `width × height` world (which may
    /// differ from the scenario's own size), drawing randomness from `rng`.
    pub fn initial_fields(
        &self,
        width: usize,
        height: usize,
        rng: &mut SimRng,
    ) -> Result<Vec<Grid2<f32>>, ContinuousError> {
        validate_size(width, height)?;
        let mut fields = vec![Grid2::new(width, height, 0.0f32); self.rule.channels()];
        let limit = self.rule.edit_limit();
        let (w, h) = (width as f32, height as f32);
        for element in &self.init {
            match element {
                InitElement::Creature { cells, x, y, rotation, scale, .. } => {
                    let cells = cells.decode()?;
                    let at =
                        Placement { x: x * w, y: y * h, rotation_deg: *rotation, scale: *scale, blend: Blend::Replace };
                    edit::place(&mut fields, &cells, &at, Some(limit))?;
                }
                InitElement::Soup { x, y, radius, shape, density, amplitude } => {
                    let (cx, cy) = (x * w, y * h);
                    let region = match shape {
                        SoupShape::Disc => Region::Disc { x: cx, y: cy, radius: *radius },
                        SoupShape::Square => {
                            Region::Rect { x: cx - radius, y: cy - radius, width: 2.0 * radius, height: 2.0 * radius }
                        }
                    };
                    edit::randomize(&mut fields, &region, *density, amplitude.min(limit), rng)?;
                }
                InitElement::RandomDiscs { count, min_radius, max_radius, value } => {
                    edit::random_discs(&mut fields, *count, *min_radius, *max_radius, value.min(limit), rng)?;
                }
            }
        }
        Ok(fields)
    }

    /// Creatures defined by this scenario's initialisation elements.
    pub fn creatures(&self) -> Result<Vec<LibraryCreature>, ContinuousError> {
        self.init
            .iter()
            .filter_map(|e| match e {
                InitElement::Creature { name, cells, .. } => Some((name, cells)),
                _ => None,
            })
            .map(|(name, cells)| {
                Ok(LibraryCreature { name: name.clone(), scenario_id: self.header.id.clone(), cells: cells.decode()? })
            })
            .collect()
    }
}

fn ron_options() -> ron::Options {
    use ron::extensions::Extensions;
    ron::Options::default().with_default_extension(Extensions::UNWRAP_VARIANT_NEWTYPES | Extensions::IMPLICIT_SOME)
}

/// Parses and validates scenario text. `source_name` labels errors.
pub fn parse(text: &str, source_name: &str) -> Result<Scenario, ContinuousError> {
    let scenario: Scenario = ron_options()
        .from_str(text)
        .map_err(|e| ContinuousError::Scenario { source_name: source_name.to_string(), message: e.to_string() })?;
    scenario
        .validate()
        .map_err(|e| ContinuousError::Scenario { source_name: source_name.to_string(), message: e.to_string() })?;
    Ok(scenario)
}

/// Serialises a scenario as pretty RON (readable by [`parse`]).
pub fn to_ron(scenario: &Scenario) -> Result<String, ContinuousError> {
    let config = ron::ser::PrettyConfig::default().struct_names(false);
    ron_options()
        .to_string_pretty(scenario, config)
        .map_err(|e| ContinuousError::Scenario { source_name: scenario.header.id.clone(), message: e.to_string() })
}

/// Loads one scenario file.
pub fn load_file(path: &Path) -> Result<Scenario, ContinuousError> {
    let io = |e: std::io::Error| ContinuousError::Io { path: path.to_path_buf(), message: e.to_string() };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_FILE_BYTES {
        return Err(ContinuousError::Io {
            path: path.to_path_buf(),
            message: format!("file is larger than {MAX_FILE_BYTES} bytes"),
        });
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    parse(&text, &path.display().to_string())
}

/// Loads every `*.ron` file of a directory, sorted by file name. Fails on
/// the first invalid file (the error names it) so typos are visible.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, ContinuousError> {
    let io = |e: std::io::Error| ContinuousError::Io { path: dir.to_path_buf(), message: e.to_string() };
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(io)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(io)?
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ron")))
        .collect();
    paths.sort();
    paths.iter().map(|p| load_file(p)).collect()
}

/// Scenario files embedded in the binary (copies of `scenarios/continuous/`).
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("orbium.ron", include_str!("../../../scenarios/continuous/orbium.ron")),
    ("orbium-chan.ron", include_str!("../../../scenarios/continuous/orbium-chan.ron")),
    ("scutium-solidus.ron", include_str!("../../../scenarios/continuous/scutium-solidus.ron")),
    ("hydrogeminium-natans.ron", include_str!("../../../scenarios/continuous/hydrogeminium-natans.ron")),
    ("tessellatium-gyrans.ron", include_str!("../../../scenarios/continuous/tessellatium-gyrans.ron")),
    ("fish.ron", include_str!("../../../scenarios/continuous/fish.ron")),
    ("lenia-soup.ron", include_str!("../../../scenarios/continuous/lenia-soup.ron")),
    ("smoothlife-glider.ron", include_str!("../../../scenarios/continuous/smoothlife-glider.ron")),
    ("smoothlife-soup.ron", include_str!("../../../scenarios/continuous/smoothlife-soup.ron")),
    ("flow-lenia-demo.ron", include_str!("../../../scenarios/continuous/flow-lenia-demo.ron")),
];

/// Parses the embedded scenarios (in [`BUILTIN_FILES`] order).
///
/// # Panics
/// Never for a released build: every embedded file is parsed and validated
/// by the crate's tests.
pub fn builtin() -> Vec<Scenario> {
    BUILTIN_FILES
        .iter()
        .map(|(name, text)| match parse(text, name) {
            Ok(s) => s,
            Err(e) => panic!("built-in scenario is invalid: {e}"),
        })
        .collect()
}

/// Looks up a built-in scenario by id.
pub fn builtin_by_id(id: &str) -> Option<Scenario> {
    builtin().into_iter().find(|s| s.header.id == id)
}

/// Every creature of the built-in scenarios, in scenario order.
pub fn creature_library() -> Vec<LibraryCreature> {
    builtin().iter().filter_map(|s| s.creatures().ok()).flatten().collect()
}
