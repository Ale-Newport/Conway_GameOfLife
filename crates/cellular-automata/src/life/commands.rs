//! Editing commands of [`LifeSim`].

use patterns::{LifeRule, Pattern};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use super::config::{ResolvedInit, place_pattern};
use super::{LifeSim, RulePreset};
use crate::engine::LifeEngine;
use crate::error::CaError;
use crate::geometry::{
    CellRect, MAX_FILL_CELLS, Rotation, Symmetry, check_fill, circle_outline, disc, line, random_fill_each, rectangle,
    stroke, transform_in_rect,
};

/// Live cells handed to the engine at once by streaming random fills.
const FILL_CHUNK: usize = 1 << 16;

/// How pasted cells combine with the cells already there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteMode {
    /// Pasted live cells are added; existing cells stay (Golly's "Or").
    #[default]
    Or,
    /// The bounding box of the pasted pattern is cleared first ("Copy").
    Copy,
    /// Pasted live cells toggle the cells below them ("Xor").
    Xor,
}

/// Axis of an in-place mirror.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorAxis {
    /// Left-right mirror (`x ↦ x0 + x1 − x`).
    Horizontal,
    /// Top-bottom mirror (`y ↦ y0 + y1 − y`).
    Vertical,
}

/// Mutations accepted by [`LifeSim`] (`Simulation::apply`).
///
/// On finite worlds single cells map through the topology (they wrap on a
/// torus and are ignored outside a bounded plane); rectangles (clear,
/// random fills, region tools, the clearing of a copy-mode paste) are clipped
/// to the world. Copying a region is the query [`LifeSim::copy_region`];
/// "cut" is a copy followed by [`LifeCommand::ClearRegion`]. Pasting with a
/// live preview is done by the user interface, which sends
/// [`LifeCommand::Paste`] when the user confirms.
#[derive(Clone, Debug, PartialEq)]
pub enum LifeCommand {
    /// Sets or clears one cell.
    SetCell {
        /// Column.
        x: i64,
        /// Row.
        y: i64,
        /// New state.
        alive: bool,
    },
    /// Brush (`alive = true`) or eraser stroke: discs of `radius` along the
    /// segments joining consecutive points (see [`crate::geometry::stroke`]).
    Stroke {
        /// Stroke points.
        points: Vec<(i64, i64)>,
        /// Brush radius (0 = single cells).
        radius: u32,
        /// Paint (`true`) or erase.
        alive: bool,
    },
    /// One-cell-wide straight line.
    Line {
        /// Start cell.
        from: (i64, i64),
        /// End cell (inclusive).
        to: (i64, i64),
        /// Paint or erase.
        alive: bool,
    },
    /// Rectangle, filled or outline.
    Rectangle {
        /// The rectangle.
        rect: CellRect,
        /// Filled or one-cell outline.
        filled: bool,
        /// Paint or erase.
        alive: bool,
    },
    /// Circle (disc `dx² + dy² ≤ r(r+1)`), filled or outline.
    Circle {
        /// Centre cell.
        centre: (i64, i64),
        /// Radius.
        radius: u32,
        /// Filled disc or outline ring.
        filled: bool,
        /// Paint or erase.
        alive: bool,
    },
    /// Pastes a pattern: rotated/mirrored inside its bounding box, then cell
    /// `(cx, cy)` goes to `(x + cx, y + cy)`. Every non-zero state is live.
    Paste {
        /// The pattern.
        pattern: Pattern,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
        /// Clockwise rotation.
        rotation: Rotation,
        /// Left-right mirror (applied before the rotation).
        mirror: bool,
        /// Combination with existing cells.
        mode: PasteMode,
    },
    /// Kills every cell.
    Clear,
    /// Kills every cell of a rectangle.
    ClearRegion {
        /// The rectangle.
        rect: CellRect,
    },
    /// Replaces a region by a random soup.
    Randomize {
        /// Probability of a live cell (`0..=1`).
        density: f64,
        /// Seed of a dedicated generator; `None` draws from the model's
        /// generator (whose state is part of snapshots).
        seed: Option<u64>,
        /// Region (default: the whole finite world, or the home region of the
        /// infinite plane).
        region: Option<CellRect>,
        /// Symmetry of the soup.
        symmetry: Symmetry,
    },
    /// Moves the live cells of a rectangle by `(dx, dy)`; the destination
    /// rectangle is cleared first.
    MoveRegion {
        /// Source rectangle.
        rect: CellRect,
        /// Horizontal shift.
        dx: i64,
        /// Vertical shift.
        dy: i64,
    },
    /// Rotates the cells of a rectangle clockwise in place (the top-left
    /// corner stays; width and height swap for odd quarter turns).
    RotateRegion {
        /// The rectangle.
        rect: CellRect,
        /// Rotation.
        rotation: Rotation,
    },
    /// Mirrors the cells of a rectangle in place.
    MirrorRegion {
        /// The rectangle.
        rect: CellRect,
        /// Mirror axis.
        axis: MirrorAxis,
    },
    /// Replaces everything by a pattern placed at `(x, y)`; the generation
    /// restarts at 0 and the pattern becomes the reset point. With
    /// `use_pattern_rule`, a Life-like rule stored in the pattern is adopted
    /// immediately (its bounded-grid suffix, if any, is ignored).
    LoadPattern {
        /// The pattern.
        pattern: Pattern,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
        /// Adopt the pattern's rule.
        use_pattern_rule: bool,
    },
    /// Parses pattern text in any supported format (RLE, plaintext, Life
    /// 1.05/1.06; `file_name` helps format detection) and either loads it
    /// (`replace`, adopting its rule like [`LifeCommand::LoadPattern`]) or
    /// pastes it at `(x, y)` in [`PasteMode::Or`].
    ImportText {
        /// File content.
        text: String,
        /// Original file name, if any.
        file_name: Option<String>,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
        /// Replace everything or paste.
        replace: bool,
    },
}

fn shifted(rect: CellRect, dx: i64, dy: i64) -> Result<CellRect, CaError> {
    let overflow = || CaError::invalid("the move would leave the 64-bit coordinate plane");
    Ok(CellRect {
        x0: rect.x0.checked_add(dx).ok_or_else(overflow)?,
        y0: rect.y0.checked_add(dy).ok_or_else(overflow)?,
        x1: rect.x1.checked_add(dx).ok_or_else(overflow)?,
        y1: rect.y1.checked_add(dy).ok_or_else(overflow)?,
    })
}

/// Fills `region` with a random soup (see
/// [`crate::geometry::random_fill_each`]), streaming the live cells into
/// `engine` in chunks, so regions up to the largest world need no cell list.
/// The region's previous content is cleared.
///
/// # Errors
/// Invalid density or symmetry, a region larger than [`MAX_FILL_CELLS`], or
/// an engine that cannot hold the region; nothing changes on error.
pub(super) fn fill_random(
    engine: &mut dyn LifeEngine,
    rng: &mut SimRng,
    region: CellRect,
    density: f64,
    symmetry: Symmetry,
) -> Result<(), CaError> {
    check_fill(region, density, symmetry, MAX_FILL_CELLS)?;
    engine.reserve(region)?;
    engine.clear_rect(region);
    let mut buffer = Vec::with_capacity(FILL_CHUNK);
    random_fill_each(rng, region, density, symmetry, |x, y| {
        buffer.push((x, y));
        if buffer.len() == FILL_CHUNK {
            engine.set_cells(&buffer, true);
            buffer.clear();
        }
    })?;
    engine.set_cells(&buffer, true);
    Ok(())
}

impl LifeSim {
    /// Writes cells and returns the bounding box of the cells actually
    /// touched (after topology mapping).
    ///
    /// # Errors
    /// The engine cannot hold the live cells (the dense engine's window on
    /// the infinite plane); nothing is written then.
    fn write(&mut self, cells: &[(i64, i64)], alive: bool) -> Result<Option<CellRect>, CaError> {
        let touched = CellRect::bounding(cells.iter().filter_map(|&(x, y)| self.world.map(x, y)));
        if alive && let Some(rect) = touched {
            self.engine.reserve(rect)?;
        }
        self.engine.set_cells(cells, alive);
        Ok(touched)
    }

    /// Kills every cell of `rect` as the topology sees it ([`World::images`]:
    /// clipped on the bounded plane, wrapped and mirrored on tori, Klein
    /// bottles and cross-surfaces). Returns the bounding box of the cleared
    /// world cells.
    fn clear_images(&mut self, rect: CellRect) -> Option<CellRect> {
        let images = self.world.images(&rect);
        for image in &images {
            self.engine.clear_rect(*image);
        }
        images.into_iter().reduce(CellRect::union)
    }

    /// Executes a command; returns the region whose cells may have changed
    /// (`None` = anywhere).
    pub(super) fn execute(&mut self, command: LifeCommand) -> Result<Option<CellRect>, CaError> {
        match command {
            LifeCommand::SetCell { x, y, alive } => self.write(&[(x, y)], alive),
            LifeCommand::Stroke { points, radius, alive } => {
                let cells = stroke(&points, radius)?;
                self.write(&cells, alive)
            }
            LifeCommand::Line { from, to, alive } => {
                let cells = line(from.0, from.1, to.0, to.1)?;
                self.write(&cells, alive)
            }
            LifeCommand::Rectangle { rect, filled, alive } => {
                let cells = rectangle(rect, filled)?;
                self.write(&cells, alive)
            }
            LifeCommand::Circle { centre, radius, filled, alive } => {
                let cells = if filled {
                    disc(centre.0, centre.1, radius)?
                } else {
                    circle_outline(centre.0, centre.1, radius)?
                };
                self.write(&cells, alive)
            }
            LifeCommand::Paste { pattern, x, y, rotation, mirror, mode } => {
                let cells: Vec<(i64, i64)> =
                    place_pattern(&pattern, x, y, rotation, mirror)?.into_iter().map(|(x, y, _)| (x, y)).collect();
                self.paste(&cells, mode)
            }
            LifeCommand::Clear => {
                self.engine.clear();
                Ok(None)
            }
            LifeCommand::ClearRegion { rect } => {
                self.engine.clear_rect(rect);
                Ok(Some(rect))
            }
            LifeCommand::Randomize { density, seed, region, symmetry } => {
                let region = match region {
                    Some(r) => {
                        self.world.clip(&r).ok_or_else(|| CaError::invalid("the region lies outside the world"))?
                    }
                    None => self.world.home(),
                };
                let mut own = seed.map(SimRng::seed_from_u64);
                let rng = match own.as_mut() {
                    Some(rng) => rng,
                    None => &mut self.rng,
                };
                fill_random(self.engine.as_mut(), rng, region, density, symmetry)?;
                Ok(Some(region))
            }
            LifeCommand::MoveRegion { rect, dx, dy } => {
                let Some(source) = self.world.clip(&rect) else {
                    return Ok(Some(rect)); // nothing of the world inside
                };
                let dest = shifted(source, dx, dy)?;
                let moved: Vec<(i64, i64)> =
                    self.region_cells(source)?.into_iter().map(|(x, y)| (x + dx, y + dy)).collect();
                if let Some(bbox) = CellRect::bounding(moved.iter().filter_map(|&(x, y)| self.world.map(x, y))) {
                    self.engine.reserve(bbox)?;
                }
                self.engine.clear_rect(source);
                let cleared = self.clear_images(dest);
                let touched = self.write(&moved, true)?;
                Ok(Some([cleared, touched].into_iter().flatten().fold(source, CellRect::union)))
            }
            LifeCommand::RotateRegion { rect, rotation } => self.transform_region(rect, rotation, false),
            LifeCommand::MirrorRegion { rect, axis } => match axis {
                MirrorAxis::Horizontal => self.transform_region(rect, Rotation::None, true),
                MirrorAxis::Vertical => self.transform_region(rect, Rotation::Half, true),
            },
            LifeCommand::LoadPattern { pattern, x, y, use_pattern_rule } => {
                self.load_pattern(&pattern, x, y, use_pattern_rule)?;
                Ok(None)
            }
            LifeCommand::ImportText { text, file_name, x, y, replace } => {
                let pattern = patterns::parse_any(&text, file_name.as_deref())?;
                if replace {
                    self.load_pattern(&pattern, x, y, true)?;
                    Ok(None)
                } else {
                    let cells: Vec<(i64, i64)> = place_pattern(&pattern, x, y, Rotation::None, false)?
                        .into_iter()
                        .map(|(x, y, _)| (x, y))
                        .collect();
                    self.paste(&cells, PasteMode::Or)
                }
            }
        }
    }

    fn paste(&mut self, cells: &[(i64, i64)], mode: PasteMode) -> Result<Option<CellRect>, CaError> {
        let Some(bbox) = CellRect::bounding(cells.iter().copied()) else {
            return Ok(None);
        };
        let touched = CellRect::bounding(cells.iter().filter_map(|&(x, y)| self.world.map(x, y)));
        if let Some(rect) = touched {
            self.engine.reserve(rect)?;
        }
        match mode {
            PasteMode::Or => self.write(cells, true),
            PasteMode::Copy => {
                let cleared = self.clear_images(bbox);
                let written = self.write(cells, true)?;
                Ok([cleared, written].into_iter().flatten().reduce(CellRect::union))
            }
            PasteMode::Xor => {
                for &(x, y) in cells {
                    let alive = self.engine.get_cell(x, y);
                    self.engine.set_cell(x, y, !alive);
                }
                Ok(touched)
            }
        }
    }

    fn transform_region(
        &mut self,
        rect: CellRect,
        rotation: Rotation,
        mirror: bool,
    ) -> Result<Option<CellRect>, CaError> {
        let Some(rect) = self.world.clip(&rect) else {
            return Ok(Some(rect)); // nothing of the world inside
        };
        let (w, h) = (rect.width(), rect.height());
        let (nw, nh) = if rotation.quarter_turns() % 2 == 1 { (h, w) } else { (w, h) };
        if CellRect::from_origin_size(rect.x0, rect.y0, nw, nh).is_none() {
            return Err(CaError::invalid("the rotated region would leave the 64-bit coordinate plane"));
        }
        let cells = self.region_cells(rect)?;
        let (out, new_rect) = transform_in_rect(&cells, rect, rotation, mirror);
        if let Some(bbox) = CellRect::bounding(out.iter().filter_map(|&(x, y)| self.world.map(x, y))) {
            self.engine.reserve(bbox)?;
        }
        self.engine.clear_rect(rect);
        let cleared = self.clear_images(new_rect);
        let written = self.write(&out, true)?;
        Ok(Some([cleared, written].into_iter().flatten().fold(rect, CellRect::union)))
    }

    /// Replaces everything by `pattern` and makes it the new starting point.
    fn load_pattern(&mut self, pattern: &Pattern, x: i64, y: i64, use_pattern_rule: bool) -> Result<(), CaError> {
        let rule = match (&pattern.rule, use_pattern_rule) {
            (Some(text), true) => {
                let (rule, _grid) = LifeRule::parse_with_grid(text).map_err(|e| {
                    CaError::invalid(format!("the pattern's rule `{text}` is not a two-state Life-like rule: {e}"))
                })?;
                rule
            }
            _ => self.pending_rule.map_or(self.rule, |(rule, _)| rule),
        };
        let kind = Self::choose(self.engine_choice, self.world, rule, 0, self.generations_per_tick, None)?;
        let cells: Vec<(i64, i64)> =
            place_pattern(pattern, x, y, Rotation::None, false)?.into_iter().map(|(x, y, _)| (x, y)).collect();
        let mut engine = crate::engine::create_engine(kind, self.world, rule)?;
        if let Some(bbox) = CellRect::bounding(cells.iter().copied()) {
            engine.reserve(bbox)?;
        }
        engine.set_cells(&cells, true);
        self.engine = engine;
        if rule != self.rule || self.pending_rule.is_some() {
            self.rule = rule;
            self.preset = RulePreset::matching(rule);
            self.pending_rule = None;
        }
        self.init = vec![ResolvedInit::Cells(cells)];
        self.generation = 0;
        self.last_stats = None;
        self.reset_point = None;
        self.reselect()?;
        self.restart_tracking();
        Ok(())
    }
}
