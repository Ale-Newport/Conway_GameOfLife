//! View settings of the mode: colouring, transparency, cut, region,
//! inspector and tools. Everything here is display state; the simulation
//! never reads it.

use bevy::prelude::*;
use voxel_life::{Axis, Bounds, SliceRange};

/// What a voxel's colour encodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ColorMode {
    /// Alive (state 1) in gold; dying states 2 … C−1 fade from orange to
    /// dark purple.
    #[default]
    State,
    /// Generations since birth (logarithmic viridis ramp).
    Age,
    /// Height (z layer) within the world (plasma ramp).
    Height,
}

impl ColorMode {
    /// Every mode in display order.
    pub const ALL: [ColorMode; 3] = [ColorMode::State, ColorMode::Age, ColorMode::Height];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            ColorMode::State => "State",
            ColorMode::Age => "Age",
            ColorMode::Height => "Height",
        }
    }

    /// The next mode (cycles).
    pub fn next(self) -> Self {
        match self {
            ColorMode::State => ColorMode::Age,
            ColorMode::Age => ColorMode::Height,
            ColorMode::Height => ColorMode::State,
        }
    }
}

/// How the age ramp is scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgeScale {
    /// The oldest voxel of the current view is the top of the ramp.
    Auto,
    /// Ages at or above this value are drawn with the top colour.
    Fixed(u16),
}

impl Default for AgeScale {
    fn default() -> Self {
        AgeScale::Fixed(100)
    }
}

/// Which voxels are drawn translucent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Transparency {
    /// Every voxel opaque.
    #[default]
    Opaque,
    /// Dying (refractory) voxels translucent, alive voxels opaque, so the
    /// alive core shows through its decaying trail.
    GhostDying,
    /// Every voxel translucent.
    XRay,
}

impl Transparency {
    /// Every mode in display order.
    pub const ALL: [Transparency; 3] = [Transparency::Opaque, Transparency::GhostDying, Transparency::XRay];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Transparency::Opaque => "Opaque",
            Transparency::GhostDying => "Ghost dying",
            Transparency::XRay => "X-ray",
        }
    }

    /// The next mode (cycles).
    pub fn next(self) -> Self {
        match self {
            Transparency::Opaque => Transparency::GhostDying,
            Transparency::GhostDying => Transparency::XRay,
            Transparency::XRay => Transparency::Opaque,
        }
    }

    /// Whether a voxel in `state` is drawn translucent.
    pub fn is_translucent(self, state: u8) -> bool {
        match self {
            Transparency::Opaque => false,
            Transparency::GhostDying => state >= 2,
            Transparency::XRay => true,
        }
    }
}

/// What a left click in the world does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// Select a voxel and show its state, age, neighbour count and fate.
    #[default]
    Inspect,
    /// Place an alive voxel on the clicked face (Shift+click erases).
    Place,
    /// Empty the clicked voxel.
    Erase,
}

impl Tool {
    /// Every tool in display order.
    pub const ALL: [Tool; 3] = [Tool::Inspect, Tool::Place, Tool::Erase];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Inspect => "Inspect",
            Tool::Place => "Place",
            Tool::Erase => "Erase",
        }
    }
}

/// An axis-aligned cut: only layers `min ..= max` along `axis` are drawn.
/// The model exposes the faces of voxels cut open, so the interior shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CutSettings {
    /// Whether the cut applies.
    pub enabled: bool,
    /// Axis perpendicular to the cut planes.
    pub axis: Axis,
    /// First visible layer.
    pub min: i32,
    /// Last visible layer.
    pub max: i32,
}

impl Default for CutSettings {
    fn default() -> Self {
        Self { enabled: false, axis: Axis::Z, min: 0, max: 0 }
    }
}

impl CutSettings {
    /// The surface-view range this cut asks the model for.
    pub fn range(&self) -> SliceRange {
        if self.enabled {
            SliceRange::new(self.axis, self.min.min(self.max), self.max.max(self.min))
        } else {
            SliceRange::all()
        }
    }

    /// Keeps `min <= max` inside `[lo, hi]` (the layers of the axis).
    pub fn clamp_to(&mut self, lo: i32, hi: i32) {
        let (lo, hi) = (lo.min(hi), hi.max(lo));
        self.min = self.min.clamp(lo, hi);
        self.max = self.max.clamp(lo, hi);
        if self.min > self.max {
            std::mem::swap(&mut self.min, &mut self.max);
        }
    }

    /// Moves the upper plane by `delta` layers inside `[lo, hi]`, never
    /// below the lower plane.
    pub fn nudge_max(&mut self, delta: i32, lo: i32, hi: i32) {
        self.max = self.max.saturating_add(delta).clamp(self.min.max(lo), hi);
    }

    /// Moves the lower plane by `delta` layers inside `[lo, hi]`, never
    /// above the upper plane.
    pub fn nudge_min(&mut self, delta: i32, lo: i32, hi: i32) {
        self.min = self.min.saturating_add(delta).clamp(lo, self.max.min(hi));
    }

    /// One sweep step: the upper plane advances one layer and wraps to the
    /// lower plane after the last layer.
    pub fn sweep_step(&mut self, hi: i32) {
        self.max = if self.max >= hi { self.min } else { self.max + 1 };
    }
}

/// A box of voxels whose population is counted per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RegionSettings {
    /// Whether the region is shown and counted.
    pub enabled: bool,
    /// Smallest corner (inclusive).
    pub min: [i32; 3],
    /// Largest corner (inclusive).
    pub max: [i32; 3],
}

impl RegionSettings {
    /// The region as a model box with ordered corners.
    pub fn bounds(&self) -> Bounds {
        let min = [0, 1, 2].map(|a| self.min[a].min(self.max[a]));
        let max = [0, 1, 2].map(|a| self.min[a].max(self.max[a]));
        Bounds { min, max }
    }

    /// Sets the region to `bounds`.
    pub fn set(&mut self, bounds: Bounds) {
        self.min = bounds.min;
        self.max = bounds.max;
    }

    /// Clamps both corners into `world`.
    pub fn clamp_to(&mut self, world: Bounds) {
        for a in 0..3 {
            self.min[a] = self.min[a].clamp(world.min[a], world.max[a]);
            self.max[a] = self.max[a].clamp(world.min[a], world.max[a]);
        }
    }
}

/// The 2D layer shown in the slice inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InspectorSettings {
    /// Axis perpendicular to the layer.
    pub axis: Axis,
    /// Layer coordinate.
    pub layer: i32,
    /// Follow the upper cut plane (the exposed cross-section).
    pub follow_cut: bool,
}

impl Default for InspectorSettings {
    fn default() -> Self {
        Self { axis: Axis::Z, layer: 0, follow_cut: true }
    }
}

/// Everything the user can change about how the world is shown.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewSettings {
    /// What colour encodes.
    pub color_mode: ColorMode,
    /// Scale of the age ramp.
    pub age_scale: AgeScale,
    /// Which voxels are translucent.
    pub transparency: Transparency,
    /// Opacity of translucent voxels (0.05–0.9).
    pub opacity: f32,
    /// Darken voxels in crevices (ambient-occlusion-like face colours).
    pub depth_shading: bool,
    /// Tint odd voxels slightly (3D checkerboard) so single voxels read.
    pub checker: bool,
    /// Fade far voxels towards the background.
    pub fog: bool,
    /// Draw the world's wireframe box.
    pub show_bounds: bool,
    /// Draw the floor (shadow receiver) and its grid.
    pub show_floor: bool,
    /// Draw the x / y / z axes at the world origin.
    pub show_axes: bool,
    /// Most voxels requested from the model per view.
    pub render_cap: usize,
    /// Cut through the volume.
    pub cut: CutSettings,
    /// Advance the upper cut plane automatically.
    pub sweep: bool,
    /// Layers per second of the sweep.
    pub sweep_rate: f32,
    /// Region counted per state.
    pub region: RegionSettings,
    /// 2D slice inspector.
    pub inspector: InspectorSettings,
    /// Left-click tool.
    pub tool: Tool,
    /// Rotate the camera slowly around the world.
    pub turntable: bool,
    /// Turntable speed in radians per second.
    pub turntable_speed: f32,
}

/// Render caps offered in the display settings.
pub const RENDER_CAPS: [usize; 5] = [50_000, 150_000, 400_000, 1_000_000, 2_000_000];

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::State,
            age_scale: AgeScale::default(),
            transparency: Transparency::Opaque,
            opacity: 0.3,
            depth_shading: true,
            checker: true,
            fog: true,
            show_bounds: true,
            show_floor: true,
            show_axes: true,
            render_cap: 400_000,
            cut: CutSettings::default(),
            sweep: false,
            sweep_rate: 6.0,
            region: RegionSettings::default(),
            inspector: InspectorSettings::default(),
            tool: Tool::Inspect,
            turntable: false,
            turntable_speed: 0.25,
        }
    }
}

impl ViewSettings {
    /// Resets the settings tied to one world (cut, region, inspector) for a
    /// world whose layers span `layers` (inclusive box).
    pub fn reset_for_world(&mut self, layers: Bounds) {
        let z = Axis::Z.index();
        self.cut = CutSettings { enabled: false, axis: Axis::Z, min: layers.min[z], max: layers.max[z] };
        self.sweep = false;
        self.region = RegionSettings { enabled: false, min: layers.min, max: layers.max };
        let mid = layers.min[z] + (layers.max[z] - layers.min[z]) / 2;
        self.inspector = InspectorSettings { axis: Axis::Z, layer: mid, follow_cut: true };
    }

    /// Turns the cut on or off. Turning on a cut that hides nothing cuts the
    /// volume in half, so the interior shows at once.
    pub fn set_cut_enabled(&mut self, enabled: bool, layers: Bounds) {
        let (lo, hi) = axis_range(layers, self.cut.axis);
        if enabled && !self.cut.enabled && self.cut.min <= lo && self.cut.max >= hi {
            self.cut.min = lo;
            self.cut.max = lo + (hi - lo) / 2;
        }
        self.cut.enabled = enabled;
        self.cut.clamp_to(lo, hi);
    }

    /// Changes the cut axis, cutting the volume in half along it.
    pub fn set_cut_axis(&mut self, axis: Axis, layers: Bounds) {
        let (lo, hi) = axis_range(layers, axis);
        self.cut.axis = axis;
        self.cut.min = lo;
        self.cut.max = lo + (hi - lo) / 2;
    }

    /// The layer the slice inspector shows: the upper cut plane when it
    /// follows an enabled cut on its axis, else its own layer.
    pub fn inspector_layer(&self) -> (Axis, i32) {
        let inspector = self.inspector;
        if inspector.follow_cut && self.cut.enabled {
            let max = self.cut.max.max(self.cut.min);
            (self.cut.axis, max)
        } else {
            (inspector.axis, inspector.layer)
        }
    }
}

/// Layer range `[lo, hi]` of `axis` within `layers`.
pub fn axis_range(layers: Bounds, axis: Axis) -> (i32, i32) {
    (layers.min[axis.index()], layers.max[axis.index()])
}

/// Axis label.
pub fn axis_label(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// The next axis (x → y → z → x).
pub fn next_axis(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::Z,
        Axis::Z => Axis::X,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_range_is_all_when_disabled_and_ordered_when_enabled() {
        let mut cut = CutSettings { enabled: false, axis: Axis::Y, min: 9, max: 3 };
        assert_eq!(cut.range(), SliceRange::all());
        cut.enabled = true;
        assert_eq!(cut.range(), SliceRange::new(Axis::Y, 3, 9));
    }

    #[test]
    fn cut_nudges_stay_ordered_and_inside() {
        let mut cut = CutSettings { enabled: true, axis: Axis::Z, min: 2, max: 5 };
        cut.nudge_max(-10, 0, 63);
        assert_eq!((cut.min, cut.max), (2, 2), "the upper plane stops at the lower one");
        cut.nudge_max(100, 0, 63);
        assert_eq!(cut.max, 63);
        cut.nudge_min(100, 0, 63);
        assert_eq!(cut.min, 63, "the lower plane stops at the upper one");
        cut.nudge_min(-100, 0, 63);
        assert_eq!(cut.min, 0);
        cut.min = 40;
        cut.max = 10;
        cut.clamp_to(0, 20);
        assert_eq!((cut.min, cut.max), (10, 20), "clamping reorders the planes");
    }

    #[test]
    fn sweep_wraps_to_the_lower_plane() {
        let mut cut = CutSettings { enabled: true, axis: Axis::Z, min: 3, max: 6 };
        cut.sweep_step(7);
        assert_eq!(cut.max, 7);
        cut.sweep_step(7);
        assert_eq!(cut.max, 3);
        cut.sweep_step(7);
        assert_eq!(cut.max, 4);
    }

    #[test]
    fn region_bounds_are_ordered_and_clamped() {
        let mut region = RegionSettings { enabled: true, min: [5, 0, 9], max: [1, 4, 2] };
        assert_eq!(region.bounds(), Bounds { min: [1, 0, 2], max: [5, 4, 9] });
        region.clamp_to(Bounds { min: [0, 0, 0], max: [3, 3, 3] });
        // min [5, 0, 9] -> [3, 0, 3], max [1, 4, 2] -> [1, 3, 2], then ordered.
        assert_eq!(region.bounds(), Bounds { min: [1, 0, 2], max: [3, 3, 3] });
    }

    #[test]
    fn world_reset_and_inspector_follow() {
        let mut settings = ViewSettings::default();
        settings.reset_for_world(Bounds { min: [0, 0, 0], max: [63, 31, 15] });
        assert_eq!(settings.cut, CutSettings { enabled: false, axis: Axis::Z, min: 0, max: 15 });
        assert_eq!(settings.region.bounds(), Bounds { min: [0, 0, 0], max: [63, 31, 15] });
        assert_eq!(settings.inspector_layer(), (Axis::Z, 7));
        settings.cut = CutSettings { enabled: true, axis: Axis::X, min: 4, max: 20 };
        assert_eq!(settings.inspector_layer(), (Axis::X, 20), "follows the exposed upper plane");
        settings.inspector.follow_cut = false;
        assert_eq!(settings.inspector_layer(), (Axis::Z, 7));
    }

    #[test]
    fn enabling_a_full_cut_halves_the_volume() {
        let layers = Bounds { min: [0, 0, 0], max: [63, 63, 31] };
        let mut settings = ViewSettings::default();
        settings.reset_for_world(layers);
        settings.set_cut_enabled(true, layers);
        assert_eq!(settings.cut, CutSettings { enabled: true, axis: Axis::Z, min: 0, max: 15 });
        // Re-enabling keeps a cut the user positioned.
        settings.cut.max = 20;
        settings.set_cut_enabled(false, layers);
        settings.set_cut_enabled(true, layers);
        assert_eq!(settings.cut.max, 20);
        settings.set_cut_axis(Axis::X, layers);
        assert_eq!((settings.cut.axis, settings.cut.min, settings.cut.max), (Axis::X, 0, 31));
    }

    #[test]
    fn cycles_visit_every_option() {
        let mut mode = ColorMode::State;
        let mut seen = vec![];
        for _ in 0..3 {
            seen.push(mode);
            mode = mode.next();
        }
        assert_eq!(seen, ColorMode::ALL.to_vec());
        assert_eq!(mode, ColorMode::State);
        let mut t = Transparency::Opaque;
        for _ in 0..3 {
            t = t.next();
        }
        assert_eq!(t, Transparency::Opaque);
        assert_eq!(next_axis(next_axis(next_axis(Axis::Y))), Axis::Y);
    }

    #[test]
    fn translucency_per_mode() {
        assert!(!Transparency::Opaque.is_translucent(3));
        assert!(!Transparency::GhostDying.is_translucent(1));
        assert!(Transparency::GhostDying.is_translucent(2));
        assert!(Transparency::XRay.is_translucent(1));
    }
}
