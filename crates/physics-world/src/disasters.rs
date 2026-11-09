//! Disasters and interventions with explicit physical effects: ignition,
//! drought, heatwave, storm, extra rain, floods, earthquakes (fault
//! displacement and rockfall), tornadoes (wind-damage map), landslides and
//! volcanoes (cone uplift, lava, ash).

use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent};

use crate::biome::{SoilType, SurfaceKind};
use crate::cells::{STEP_LEN, neighbor};
use crate::error::{WorldError, check_range};
use crate::fire::FireState;
use crate::report::{CellDamage, FuelBurn};
use crate::weather::WeatherKind;
use crate::world::World;

/// Organic matter (g/m²) deposited by volcanic ash next to the vent.
pub const ASH_ORGANIC_G_M2: f32 = 300.0;
/// Relative soil saturation above which steep slopes can fail on their own.
pub const LANDSLIDE_MOISTURE: f32 = 0.95;
/// Longest imposed weather spell (ticks).
const MAX_SPELL_TICKS: u64 = 10_000_000;

/// An intervention. Coordinates are cells; radii and widths are in cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Disaster {
    /// Sets a cell on fire (if it has dry fuel).
    Ignite {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
    },
    /// Imposes the Drought weather state (no rain, ×1.5 evaporation).
    Drought {
        /// Duration in ticks (≥ 1).
        duration_ticks: u64,
    },
    /// Imposes the Heatwave weather state (+4.2 °C to +7 °C, ×1.4 evaporation).
    Heatwave {
        /// Duration in ticks (≥ 1).
        duration_ticks: u64,
    },
    /// Imposes a thunderstorm (rain ×3 × intensity, gusts, lightning).
    Storm {
        /// Duration in ticks (≥ 1).
        duration_ticks: u64,
        /// Precipitation multiplier, `(0, 10]`.
        intensity: f32,
    },
    /// Adds rain (or snow below 0 °C) uniformly on every non-ocean cell.
    AddRain {
        /// Depth in mm, `(0, 1000]`.
        amount_mm: f32,
    },
    /// Adds a dome of water `depth × (1 − (d/r)²)` around a cell.
    Flood {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius in cells.
        radius: f32,
        /// Depth at the centre in metres, `(0, 50]`.
        depth_m: f32,
    },
    /// Fault displacement, rockfall and slope failures around an epicentre.
    Earthquake {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius of the affected area in cells.
        radius: f32,
        /// Moment magnitude, `[3, 9.5]`.
        magnitude: f32,
    },
    /// Wind damage along a polyline.
    Tornado {
        /// Path vertices `[x, y]` in cells (at least two).
        path: Vec<[f32; 2]>,
        /// Width of the damage swath in cells.
        width: f32,
    },
    /// Forces slope failures on steep cells around a point.
    Landslide {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Radius in cells.
        radius: f32,
    },
    /// Raises a volcanic cone, pours lava and deposits ash.
    Volcano {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
        /// Cone radius in cells, ≥ 2.
        radius: f32,
        /// Cone height in metres, `(0, 5000]`.
        height_m: f32,
    },
}

impl Disaster {
    /// Machine-readable kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ignite { .. } => "ignite",
            Self::Drought { .. } => "drought",
            Self::Heatwave { .. } => "heatwave",
            Self::Storm { .. } => "storm",
            Self::AddRain { .. } => "add_rain",
            Self::Flood { .. } => "flood",
            Self::Earthquake { .. } => "earthquake",
            Self::Tornado { .. } => "tornado",
            Self::Landslide { .. } => "landslide",
            Self::Volcano { .. } => "volcano",
        }
    }
}

/// What an intervention did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisasterOutcome {
    /// Cells whose state was changed.
    pub affected_cells: u32,
    /// Cells set on fire (or turned to lava).
    pub ignited: u32,
    /// Water added (m³).
    pub water_added_m3: f64,
    /// Largest absolute elevation change (m).
    pub max_elevation_change_m: f32,
    /// Whether static terrain changed (terrain version bumped).
    pub terrain_changed: bool,
    /// River cells raised by debris (possible dams).
    pub dammed_river_cells: u32,
    /// Slope failures triggered.
    pub landslides: u32,
    /// Wind damage map (tornado).
    pub wind_damage: Vec<CellDamage>,
}

fn check_duration(duration: u64) -> Result<u64, WorldError> {
    if duration == 0 || duration > MAX_SPELL_TICKS {
        return Err(WorldError::arg("duration_ticks", format!("{duration} is outside [1, {MAX_SPELL_TICKS}]")));
    }
    Ok(duration)
}

/// Average coseismic fault displacement (m) of a moment magnitude `M`
/// earthquake, Wells & Coppersmith (1994) regression for all slip types:
/// `log₁₀ AD = −4.80 + 0.69 M` (fitted for M 5.6–8.1; extrapolated outside).
pub fn fault_displacement_m(magnitude: f32) -> f32 {
    10f32.powf(-4.80 + 0.69 * magnitude)
}

/// Distance from `p` to segment `a–b`.
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

impl World {
    /// Applies an intervention immediately; its events and side effects
    /// (fuel consumed, wind damage, terrain change) also appear in the next
    /// [`EnvironmentReport`](crate::EnvironmentReport).
    pub fn trigger(&mut self, disaster: Disaster) -> Result<DisasterOutcome, WorldError> {
        let kind = disaster.kind();
        let outcome = match disaster {
            Disaster::Ignite { x, y } => {
                let i = self.cell_index(x, y)?;
                let ignited = u32::from(self.ignite_index(i));
                self.pending.ignitions += ignited;
                DisasterOutcome { affected_cells: ignited, ignited, ..DisasterOutcome::default() }
            }
            Disaster::Drought { duration_ticks } => self.spell(WeatherKind::Drought, duration_ticks, 1.0)?,
            Disaster::Heatwave { duration_ticks } => self.spell(WeatherKind::Heatwave, duration_ticks, 1.0)?,
            Disaster::Storm { duration_ticks, intensity } => {
                check_range("intensity", intensity, f32::MIN_POSITIVE, 10.0)?;
                self.spell(WeatherKind::Storm, duration_ticks, intensity)?
            }
            Disaster::AddRain { amount_mm } => self.add_rain_mm(amount_mm)?,
            Disaster::Flood { x, y, radius, depth_m } => self.flood_at(x, y, radius, depth_m)?,
            Disaster::Earthquake { x, y, radius, magnitude } => self.earthquake_at(x, y, radius, magnitude)?,
            Disaster::Tornado { path, width } => self.tornado_along(&path, width)?,
            Disaster::Landslide { x, y, radius } => {
                let center = self.cell_index(x, y)?;
                self.check_radius(radius)?;
                let slides = self.slides_around(center, radius, 0.5);
                if slides > 0 {
                    self.refresh_terrain();
                    self.pending.terrain_changed = true;
                    self.pending.landslides += slides;
                }
                DisasterOutcome {
                    affected_cells: slides,
                    landslides: slides,
                    terrain_changed: slides > 0,
                    ..DisasterOutcome::default()
                }
            }
            Disaster::Volcano { x, y, radius, height_m } => self.volcano_at(x, y, radius, height_m)?,
        };
        self.pending.events.push(
            SimEvent::new(self.tick, "disaster", format!("{kind}: {} cells affected", outcome.affected_cells))
                .severity(EventSeverity::Major),
        );
        Ok(outcome)
    }

    /// Sets cell `(x, y)` on fire. Returns whether it ignited (it needs dry
    /// fuel, no water and no snow).
    pub fn ignite(&mut self, x: usize, y: usize) -> Result<bool, WorldError> {
        self.trigger(Disaster::Ignite { x, y }).map(|o| o.ignited > 0)
    }

    /// Adds `amount_mm` of rain uniformly over non-ocean cells; returns m³ added.
    pub fn add_rain(&mut self, amount_mm: f32) -> Result<f64, WorldError> {
        self.trigger(Disaster::AddRain { amount_mm }).map(|o| o.water_added_m3)
    }

    /// Imposes a drought for `duration_ticks`.
    pub fn start_drought(&mut self, duration_ticks: u64) -> Result<(), WorldError> {
        self.trigger(Disaster::Drought { duration_ticks }).map(|_| ())
    }

    /// Imposes a heatwave for `duration_ticks`.
    pub fn heatwave(&mut self, duration_ticks: u64) -> Result<(), WorldError> {
        self.trigger(Disaster::Heatwave { duration_ticks }).map(|_| ())
    }

    /// Imposes a storm of the given intensity for `duration_ticks`.
    pub fn storm(&mut self, duration_ticks: u64, intensity: f32) -> Result<(), WorldError> {
        self.trigger(Disaster::Storm { duration_ticks, intensity }).map(|_| ())
    }

    /// Adds a dome of flood water (see [`Disaster::Flood`]).
    pub fn flood(&mut self, x: usize, y: usize, radius: f32, depth_m: f32) -> Result<DisasterOutcome, WorldError> {
        self.trigger(Disaster::Flood { x, y, radius, depth_m })
    }

    /// Earthquake around an epicentre (see [`Disaster::Earthquake`]).
    pub fn earthquake(
        &mut self,
        x: usize,
        y: usize,
        radius: f32,
        magnitude: f32,
    ) -> Result<DisasterOutcome, WorldError> {
        self.trigger(Disaster::Earthquake { x, y, radius, magnitude })
    }

    /// Tornado along a path (see [`Disaster::Tornado`]).
    pub fn tornado(&mut self, path: Vec<[f32; 2]>, width: f32) -> Result<DisasterOutcome, WorldError> {
        self.trigger(Disaster::Tornado { path, width })
    }

    /// Forced slope failures around a point (see [`Disaster::Landslide`]).
    pub fn landslide(&mut self, x: usize, y: usize, radius: f32) -> Result<DisasterOutcome, WorldError> {
        self.trigger(Disaster::Landslide { x, y, radius })
    }

    /// Volcanic cone, lava and ash (see [`Disaster::Volcano`]).
    pub fn volcano(&mut self, x: usize, y: usize, radius: f32, height_m: f32) -> Result<DisasterOutcome, WorldError> {
        self.trigger(Disaster::Volcano { x, y, radius, height_m })
    }

    fn check_radius(&self, radius: f32) -> Result<f32, WorldError> {
        let max = (self.config.width.max(self.config.height) * 2) as f32;
        check_range("radius", radius, 0.5, max)
    }

    /// Cells whose centre lies within `radius` of cell `center`, with their
    /// distance, in index order.
    fn cells_within(&self, center: usize, radius: f32) -> Vec<(usize, f32)> {
        let (w, h) = (self.config.width, self.config.height);
        let (cx, cy) = ((center % w) as f32, (center / w) as f32);
        let r = radius.ceil() as i64;
        let y0 = (cy as i64 - r).max(0) as usize;
        let y1 = ((cy as i64 + r) as usize).min(h - 1);
        let x0 = (cx as i64 - r).max(0) as usize;
        let x1 = ((cx as i64 + r) as usize).min(w - 1);
        let mut cells = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d <= radius {
                    cells.push((y * w + x, d));
                }
            }
        }
        cells
    }

    fn spell(&mut self, kind: WeatherKind, duration: u64, intensity: f32) -> Result<DisasterOutcome, WorldError> {
        let duration = check_duration(duration)?;
        self.weather.force(kind, self.tick + duration, intensity);
        let land = self.surface.as_slice().iter().filter(|s| **s != SurfaceKind::Ocean).count() as u32;
        Ok(DisasterOutcome { affected_cells: land, ..DisasterOutcome::default() })
    }

    fn add_rain_mm(&mut self, amount_mm: f32) -> Result<DisasterOutcome, WorldError> {
        check_range("amount_mm", amount_mm, f32::MIN_POSITIVE, 1000.0)?;
        let depth = amount_mm / 1000.0;
        let mut cells = 0u32;
        for i in 0..self.config.width * self.config.height {
            if self.surface.as_slice()[i] == SurfaceKind::Ocean {
                continue;
            }
            if self.temperature.as_slice()[i] < 0.0 {
                self.snow.as_mut_slice()[i] += depth;
            } else {
                self.water.as_mut_slice()[i] += f64::from(depth);
            }
            cells += 1;
        }
        self.sync_water_depth();
        let added = f64::from(depth) * f64::from(cells) * self.config.cell_area_m2();
        self.water_budget.added_m3 += added;
        Ok(DisasterOutcome { affected_cells: cells, water_added_m3: added, ..DisasterOutcome::default() })
    }

    fn flood_at(&mut self, x: usize, y: usize, radius: f32, depth: f32) -> Result<DisasterOutcome, WorldError> {
        let center = self.cell_index(x, y)?;
        self.check_radius(radius)?;
        check_range("depth_m", depth, f32::MIN_POSITIVE, 50.0)?;
        let mut added = 0.0f64;
        let mut cells = 0;
        for (i, d) in self.cells_within(center, radius) {
            if self.surface.as_slice()[i] == SurfaceKind::Ocean {
                continue;
            }
            let add = depth * (1.0 - (d / radius).powi(2));
            if add > 0.0 {
                self.water.as_mut_slice()[i] += f64::from(add);
                added += f64::from(add);
                cells += 1;
            }
        }
        self.sync_water_depth();
        let added = added * self.config.cell_area_m2();
        self.water_budget.added_m3 += added;
        Ok(DisasterOutcome { affected_cells: cells, water_added_m3: added, ..DisasterOutcome::default() })
    }

    /// Moves `dz` metres of soil from cell `i` to cell `j` together with a
    /// proportional share of nutrients and organic matter (mass conserving).
    fn move_soil(&mut self, i: usize, j: usize, dz: f32) {
        self.elevation.as_mut_slice()[i] -= dz;
        self.elevation.as_mut_slice()[j] += dz;
        let share = (0.5 * dz).min(0.5);
        for layer in [&mut self.nutrients, &mut self.organic] {
            let moved = layer.as_slice()[i] * share;
            layer.as_mut_slice()[i] -= moved;
            layer.as_mut_slice()[j] += moved;
        }
        let fertility = self.fertility.as_mut_slice();
        fertility[i] *= 1.0 - share;
        if share >= 0.25 {
            self.set_soil(i, SoilType::Rock);
        }
    }

    /// Changes the soil type of cell `i` while conserving its soil water:
    /// the saturation is rescaled to the new capacity and whatever no longer
    /// fits is released as surface water.
    pub(crate) fn set_soil(&mut self, i: usize, soil: SoilType) {
        let old_capacity = self.soil.as_slice()[i].properties().capacity_m;
        let new_capacity = soil.properties().capacity_m;
        let stored = f64::from(self.soil_moisture.as_slice()[i]) * f64::from(old_capacity);
        let moisture = (stored / f64::from(new_capacity)).min(1.0) as f32;
        let kept = f64::from(moisture) * f64::from(new_capacity);
        self.soil_moisture.as_mut_slice()[i] = moisture;
        self.water.as_mut_slice()[i] += (stored - kept).max(0.0);
        self.soil.as_mut_slice()[i] = soil;
    }

    /// Steepest downhill neighbour of `i` by current elevation, with its
    /// gradient (rise/run).
    fn steepest_descent(&self, i: usize) -> Option<(usize, f32, f32)> {
        let (w, h) = (self.config.width, self.config.height);
        let z = self.elevation.as_slice();
        let mut best: Option<(usize, f32, f32)> = None;
        for (k, len) in STEP_LEN.iter().enumerate() {
            if let Some(j) = neighbor(i, k, w, h) {
                let run = len * self.config.cell_size_m;
                let gradient = (z[i] - z[j]) / run;
                if gradient > 0.0 && best.is_none_or(|(_, g, _)| gradient > g) {
                    best = Some((j, gradient, run));
                }
            }
        }
        best
    }

    /// Slope failure of cell `i` if its steepest gradient exceeds
    /// `scale × critical_slope(soil)`: half of the excess height (at most
    /// 5 m) moves to the steepest downhill neighbour. Returns the receiver.
    fn slide(&mut self, i: usize, scale: f32) -> Option<usize> {
        let critical = self.soil.as_slice()[i].properties().critical_slope * scale;
        let (j, gradient, run) = self.steepest_descent(i)?;
        if gradient <= critical {
            return None;
        }
        let dz = (0.5 * (gradient - critical) * run).min(5.0);
        self.move_soil(i, j, dz);
        Some(j)
    }

    /// Forced slope failures within `radius` of `center` on non-water cells
    /// steeper than `scale × critical slope`. Returns the number of slides.
    fn slides_around(&mut self, center: usize, radius: f32, scale: f32) -> u32 {
        let mut slides = 0;
        for (i, _) in self.cells_within(center, radius) {
            if self.surface.as_slice()[i].is_water() {
                continue;
            }
            let critical = self.soil.as_slice()[i].properties().critical_slope * scale;
            if self.slope.as_slice()[i] > critical && self.slide(i, scale).is_some() {
                slides += 1;
            }
        }
        slides
    }

    /// Hourly spontaneous landslides: land cells with a soil cover (not
    /// bare rock) steeper than their soil's critical slope and wetter than
    /// 95 % saturation fail with probability `0.005 × (s − 0.95) / 0.05` per
    /// hour. Returns the number of slides.
    pub(crate) fn natural_landslides(&mut self, tick: u64, report: &mut crate::report::EnvironmentReport) -> u32 {
        let candidates: Vec<usize> = (0..self.config.width * self.config.height)
            .filter(|&i| {
                self.surface.as_slice()[i] == SurfaceKind::Land
                    && self.soil.as_slice()[i] != SoilType::Rock
                    && self.soil_moisture.as_slice()[i] > LANDSLIDE_MOISTURE
                    && self.slope.as_slice()[i] > self.soil.as_slice()[i].properties().critical_slope
            })
            .collect();
        let mut slides = 0;
        for i in candidates {
            let p = 0.005 * (self.soil_moisture.as_slice()[i] - LANDSLIDE_MOISTURE) / (1.0 - LANDSLIDE_MOISTURE);
            if self.rngs.disasters.chance(f64::from(p)) && self.slide(i, 1.0).is_some() {
                slides += 1;
            }
        }
        if slides > 0 {
            report.events.push(
                SimEvent::new(tick, "landslide", format!("{slides} landslide(s) on saturated slopes"))
                    .severity(EventSeverity::Info),
            );
        }
        slides
    }

    /// Earthquake of moment magnitude `M`: a straight fault through the
    /// epicentre at a random angle; cells within `r` are displaced by
    /// `±A (1 − (d/r)²)` (up on one side, down on the other) with the
    /// average coseismic displacement of Wells & Coppersmith (1994, all slip
    /// types) `log₁₀ A = −4.80 + 0.69 M` (A in m), applied vertically.
    /// Shaking `s = (1 − d/r)(M − 3)/6.5`
    /// triggers rockfall on slopes > 0.35 with probability `s × min(slope, 1)`
    /// (debris of `2 m × s × min(slope, 1)` moved downhill, halving the
    /// source fertility and possibly damming rivers) and forced slope
    /// failures above `(1 − 0.5 s_max)` × critical slope.
    fn earthquake_at(
        &mut self,
        x: usize,
        y: usize,
        radius: f32,
        magnitude: f32,
    ) -> Result<DisasterOutcome, WorldError> {
        let center = self.cell_index(x, y)?;
        self.check_radius(radius)?;
        check_range("magnitude", magnitude, 3.0, 9.5)?;
        let angle = self.rngs.disasters.f32() * std::f32::consts::PI;
        let normal = Vec2::new(angle.cos(), angle.sin());
        let amplitude = fault_displacement_m(magnitude);
        let strength = (magnitude - 3.0) / 6.5;
        let origin = Vec2::new(x as f32, y as f32);
        let w = self.config.width;
        let cells = self.cells_within(center, radius);
        let mut outcome = DisasterOutcome { affected_cells: cells.len() as u32, ..DisasterOutcome::default() };
        for &(i, d) in &cells {
            let p = Vec2::new((i % w) as f32, (i / w) as f32);
            let side = if (p - origin).dot(normal) >= 0.0 { 1.0 } else { -1.0 };
            let dz = side * amplitude * (1.0 - (d / radius).powi(2));
            self.elevation.as_mut_slice()[i] += dz;
            outcome.max_elevation_change_m = outcome.max_elevation_change_m.max(dz.abs());
        }
        for &(i, d) in &cells {
            let slope = self.slope.as_slice()[i];
            if slope <= 0.35 || self.surface.as_slice()[i].is_water() {
                continue;
            }
            let shaking = (1.0 - d / radius) * strength;
            if !self.rngs.disasters.chance(f64::from(shaking * slope.min(1.0))) {
                continue;
            }
            if let Some((j, _, _)) = self.steepest_descent(i) {
                let dz = (2.0 * shaking * slope.min(1.0))
                    .min(0.5 * (self.elevation.as_slice()[i] - self.elevation.as_slice()[j]));
                if dz > 0.0 {
                    self.move_soil(i, j, dz);
                    self.fertility.as_mut_slice()[i] *= 0.5;
                    self.fertility.as_mut_slice()[j] *= 0.7;
                    if self.surface.as_slice()[j] == SurfaceKind::River {
                        outcome.dammed_river_cells += 1;
                    }
                    outcome.max_elevation_change_m = outcome.max_elevation_change_m.max(dz);
                }
            }
        }
        outcome.landslides = self.slides_around(center, radius, 1.0 - 0.5 * strength);
        self.refresh_terrain();
        self.pending.terrain_changed = true;
        self.pending.landslides += outcome.landslides;
        outcome.terrain_changed = true;
        Ok(outcome)
    }

    /// Tornado: cells within `width / 2` of the path get damage
    /// `1 − (d / (width/2))²`; canopy is reduced by the same fraction and the
    /// damage map is reported to the ecosystem.
    fn tornado_along(&mut self, path: &[[f32; 2]], width: f32) -> Result<DisasterOutcome, WorldError> {
        if path.len() < 2 {
            return Err(WorldError::arg("path", "a tornado path needs at least two points"));
        }
        if path.iter().flatten().any(|v| !v.is_finite()) {
            return Err(WorldError::arg("path", "path coordinates must be finite"));
        }
        let (w, h) = (self.config.width, self.config.height);
        check_range("width", width, 0.5, w.max(h) as f32)?;
        let half = width / 2.0;
        let points: Vec<Vec2> = path.iter().map(|&[x, y]| Vec2::new(x, y)).collect();
        let min = points.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p)) - Vec2::splat(half);
        let max = points.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p)) + Vec2::splat(half);
        let x0 = min.x.floor().max(0.0) as usize;
        let y0 = min.y.floor().max(0.0) as usize;
        let x1 = (max.x.ceil().max(0.0) as usize).min(w - 1);
        let y1 = (max.y.ceil().max(0.0) as usize).min(h - 1);
        let mut damage = Vec::new();
        if min.x <= (w - 1) as f32 && min.y <= (h - 1) as f32 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = Vec2::new(x as f32, y as f32);
                    let d = points.windows(2).map(|s| segment_distance(p, s[0], s[1])).fold(f32::MAX, f32::min);
                    if d < half {
                        let amount = 1.0 - (d / half).powi(2);
                        let canopy = &mut self.canopy.as_mut_slice()[y * w + x];
                        *canopy *= 1.0 - amount;
                        damage.push(CellDamage { x: x as u32, y: y as u32, damage: amount });
                    }
                }
            }
        }
        self.pending.wind_damage.extend_from_slice(&damage);
        Ok(DisasterOutcome { affected_cells: damage.len() as u32, wind_damage: damage, ..DisasterOutcome::default() })
    }

    /// Volcano: a cone `H (1 − d/r)^1.5` with a crater of depth `0.3 H` in the
    /// inner 12 % of the radius; non-ocean cells within `0.35 r` become lava
    /// for two days (fuel consumed, water and snow boiled off, soil turned to
    /// rock, fertility 0.05); ash adds `300 g/m² × (1 − d/3r)` of organic
    /// matter within `3 r`, which mineralises into nutrients over the
    /// following weeks.
    fn volcano_at(&mut self, x: usize, y: usize, radius: f32, height: f32) -> Result<DisasterOutcome, WorldError> {
        let center = self.cell_index(x, y)?;
        check_range("radius", radius, 2.0, (self.config.width.max(self.config.height) * 2) as f32)?;
        check_range("height_m", height, f32::MIN_POSITIVE, 5000.0)?;
        let crater = 0.12 * radius;
        let mut outcome = DisasterOutcome::default();
        for (i, d) in self.cells_within(center, radius) {
            let mut dz = height * (1.0 - d / radius).powf(1.5);
            if d < crater {
                dz -= 0.3 * height * (1.0 - d / crater);
            }
            self.elevation.as_mut_slice()[i] += dz;
            outcome.max_elevation_change_m = outcome.max_elevation_change_m.max(dz.abs());
            outcome.affected_cells += 1;
        }
        self.refresh_terrain();
        let area = self.config.cell_area_m2();
        let substeps = u64::from(self.calendar.substeps_per_tick());
        let lava_steps = (2 * u64::from(self.calendar.ticks_per_day()) * substeps).min(u64::from(u32::MAX)) as u32;
        let w = self.config.width as u32;
        for (i, _) in self.cells_within(center, (0.35 * radius).max(0.5)) {
            if self.surface.as_slice()[i] == SurfaceKind::Ocean {
                continue;
            }
            let fuel = self.fuel.as_slice()[i];
            if fuel > 0.0 {
                self.pending.fuel_consumed.push(FuelBurn { x: i as u32 % w, y: i as u32 / w, amount: fuel });
                self.fuel.as_mut_slice()[i] = 0.0;
            }
            let capacity = self.soil.as_slice()[i].properties().capacity_m;
            let boiled = self.water.as_slice()[i]
                + f64::from(self.snow.as_slice()[i] + self.soil_moisture.as_slice()[i] * capacity);
            self.water_budget.evaporated_m3 += boiled * area;
            self.water.as_mut_slice()[i] = 0.0;
            self.snow.as_mut_slice()[i] = 0.0;
            self.soil_moisture.as_mut_slice()[i] = 0.0;
            self.soil.as_mut_slice()[i] = SoilType::Rock;
            self.fertility.as_mut_slice()[i] = 0.05;
            self.fire.as_mut_slice()[i] = FireState::Lava { steps_left: lava_steps };
            if let Err(pos) = self.burning.binary_search(&(i as u32)) {
                self.burning.insert(pos, i as u32);
            }
            outcome.ignited += 1;
        }
        self.sync_water_depth();
        let ash_radius = 3.0 * radius;
        for (i, d) in self.cells_within(center, ash_radius) {
            if self.surface.as_slice()[i] == SurfaceKind::Ocean {
                continue;
            }
            let ash = ASH_ORGANIC_G_M2 * (1.0 - d / ash_radius);
            self.organic.as_mut_slice()[i] += ash;
            self.nutrient_budget.deposited_g += f64::from(ash) * area;
        }
        self.pending.ignitions += outcome.ignited;
        self.pending.terrain_changed = true;
        outcome.terrain_changed = true;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wells_coppersmith_average_displacement() {
        // log10(AD) = -4.80 + 0.69 M: about 0.22 m at M6, 1.07 m at M7,
        // 2.37 m at M7.5, and a factor 10^0.69 per magnitude unit.
        assert!((fault_displacement_m(6.0) - 0.2188).abs() < 1e-3);
        assert!((fault_displacement_m(7.0) - 1.0715).abs() < 1e-3);
        assert!((fault_displacement_m(7.5) - 2.3714).abs() < 1e-3);
        let ratio = fault_displacement_m(8.0) / fault_displacement_m(7.0);
        assert!((ratio - 10f32.powf(0.69)).abs() < 1e-3);
    }

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        let (a, b) = (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0));
        assert_eq!(segment_distance(Vec2::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(segment_distance(Vec2::new(-4.0, 3.0), a, b), 5.0);
        assert_eq!(segment_distance(Vec2::new(2.0, 2.0), a, a), 8f32.sqrt());
    }
}
