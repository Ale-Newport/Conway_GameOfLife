//! Resources shared by the mode's systems and panels: the runner handle,
//! the latest frame, and the user-interface state (camera, overlay,
//! layers, tool, selection, tabs).

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ecosystem::OrganismRef;
use simulation_core::Runner;

use crate::palette::Overlay;
use crate::session::{EcoSession, Frame};

/// The running session.
#[derive(Resource, Clone)]
pub struct EcoRunner(pub Arc<Runner<EcoSession>>);

/// The latest published frame and its timing.
#[derive(Resource, Default)]
pub struct EcoData {
    /// Latest frame.
    pub frame: Option<Arc<Frame>>,
    /// Publication counter of `frame`.
    pub seq: u64,
    /// Real time (s) the frame arrived.
    pub arrived: f64,
    /// Smoothed real time between frames (s).
    pub interval: f32,
    /// A new frame arrived during this update.
    pub fresh: bool,
    /// Last frame that carried trees (frames carry them at most hourly).
    pub trees_frame: Option<Arc<Frame>>,
    /// Publication counter of `trees_frame`.
    pub trees_seq: u64,
}

impl EcoData {
    /// The latest frame.
    pub fn get(&self) -> Option<&Frame> {
        self.frame.as_deref()
    }
}

/// Camera behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CameraMode {
    /// Free orbit around a focus (left drag orbits, right drag pans).
    #[default]
    Orbit,
    /// Map view from straight above.
    TopDown,
    /// Low, eye-level view.
    Ground,
    /// Slow automatic orbit (around the selection if any).
    Cinematic,
    /// Follows the selected animal.
    Follow,
}

impl CameraMode {
    /// Every mode in display order.
    pub const ALL: [CameraMode; 5] = [Self::Orbit, Self::TopDown, Self::Ground, Self::Cinematic, Self::Follow];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Orbit => "Orbit",
            Self::TopDown => "Top-down",
            Self::Ground => "Ground level",
            Self::Cinematic => "Cinematic",
            Self::Follow => "Follow",
        }
    }

    /// Short description.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Orbit => "Left drag orbits, right drag pans, wheel zooms; WASD / arrows pan, Q / E rotate.",
            Self::TopDown => "Map view from straight above; left drag rotates, right drag pans.",
            Self::Ground => "Low view across the landscape at animal height.",
            Self::Cinematic => "Slow automatic orbit around the selected organism or the view centre.",
            Self::Follow => "Tracks the selected animal (select one first).",
        }
    }
}

/// What a left click in the world does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Select animals and trees.
    #[default]
    Inspect,
    /// Add animals of a species.
    Spawn,
    /// Kill animals in a radius.
    Cull,
    /// Plant a tree seedling.
    PlantTree,
    /// Set a cell on fire.
    Ignite,
    /// Flood around a point.
    Flood,
    /// Earthquake around a point.
    Earthquake,
    /// Landslide around a point.
    Landslide,
    /// Volcano at a point.
    Volcano,
    /// Tornado from a first to a second click.
    Tornado,
}

impl Tool {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Inspect => "Inspect",
            Self::Spawn => "Add animals",
            Self::Cull => "Cull",
            Self::PlantTree => "Plant a tree",
            Self::Ignite => "Ignite",
            Self::Flood => "Flood",
            Self::Earthquake => "Earthquake",
            Self::Landslide => "Landslide",
            Self::Volcano => "Volcano",
            Self::Tornado => "Tornado",
        }
    }

    /// Whether the tool uses a radius brush.
    pub fn has_radius(self) -> bool {
        matches!(self, Self::Spawn | Self::Cull | Self::Flood | Self::Earthquake | Self::Landslide | Self::Volcano)
    }
}

/// Layers that can be hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layers {
    /// Individual animals, carcasses and clutches.
    pub animals: bool,
    /// Trees.
    pub trees: bool,
    /// Grass, shrubs and flowers near the camera.
    pub ground_cover: bool,
    /// Insects, worms, small fish (density-field markers), colonies and forager squads.
    pub small_life: bool,
    /// Rain, snow, fire and smoke.
    pub weather: bool,
    /// Trail of the followed animal and the selection ring.
    pub highlights: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self { animals: true, trees: true, ground_cover: true, small_life: true, weather: true, highlights: true }
    }
}

/// Tabs of the Properties panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PropertiesTab {
    /// Selected organism.
    #[default]
    Inspector,
    /// Species sheet.
    Species,
    /// Populations and trophic groups.
    Populations,
    /// Energy and nutrient budgets.
    Energy,
    /// Weather and climate.
    Climate,
    /// Food web.
    FoodWeb,
    /// Births, deaths, disease.
    Deaths,
    /// Scenario description.
    Scenario,
}

impl PropertiesTab {
    /// Every tab in display order.
    pub const ALL: [PropertiesTab; 8] = [
        Self::Scenario,
        Self::Inspector,
        Self::Species,
        Self::Populations,
        Self::FoodWeb,
        Self::Energy,
        Self::Climate,
        Self::Deaths,
    ];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Inspector => "Inspector",
            Self::Species => "Species",
            Self::Populations => "Populations",
            Self::Energy => "Energy & nutrients",
            Self::Climate => "Climate",
            Self::FoodWeb => "Food web",
            Self::Deaths => "Deaths & disease",
            Self::Scenario => "Scenario",
        }
    }
}

/// Settings of the intervention tools.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    /// Species to add (id).
    pub spawn_species: String,
    /// Animals (or field individuals, or colonies) to add.
    pub spawn_count: u32,
    /// Brush radius (cells).
    pub radius: f32,
    /// Species to cull (`None`: every species).
    pub cull_species: Option<u16>,
    /// Tree species to plant (id).
    pub tree_species: String,
    /// Flood depth at the centre (m).
    pub flood_depth_m: f32,
    /// Earthquake magnitude.
    pub magnitude: f32,
    /// Volcano height (m).
    pub volcano_height_m: f32,
    /// Tornado swath width (cells).
    pub tornado_width: f32,
    /// First click of a tornado path (cells).
    pub tornado_start: Option<Vec2>,
    /// Duration of weather spells (days).
    pub spell_days: f32,
    /// Storm rain multiplier.
    pub storm_intensity: f32,
    /// Rain amount (mm).
    pub rain_mm: f32,
    /// Pathogen of an outbreak (id).
    pub pathogen: String,
    /// Host species of an outbreak (id).
    pub outbreak_species: String,
    /// Hosts to infect.
    pub outbreak_count: u32,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            spawn_species: "rabbit".into(),
            spawn_count: 10,
            radius: 3.0,
            cull_species: None,
            tree_species: "oak".into(),
            flood_depth_m: 2.0,
            magnitude: 6.0,
            volcano_height_m: 120.0,
            tornado_width: 2.0,
            tornado_start: None,
            spell_days: 14.0,
            storm_intensity: 2.0,
            rain_mm: 40.0,
            pathogen: String::new(),
            outbreak_species: String::new(),
            outbreak_count: 5,
        }
    }
}

/// User-interface state of the mode.
#[derive(Resource, Debug)]
pub struct EcoUi {
    /// Camera behaviour.
    pub camera: CameraMode,
    /// Data overlay.
    pub overlay: Overlay,
    /// Visible layers.
    pub layers: Layers,
    /// Left-click tool.
    pub tool: Tool,
    /// Tool settings.
    pub settings: ToolSettings,
    /// Selected organism.
    pub selection: Option<OrganismRef>,
    /// Species shown on the Species tab (index).
    pub species: Option<u16>,
    /// Properties tab.
    pub tab: PropertiesTab,
    /// Frame the whole map at the next opportunity.
    pub frame_map: bool,
    /// Move the camera focus here (scene coordinates).
    pub focus: Option<Vec3>,
    /// Show all species in the population plot (else the ten most abundant).
    pub all_species: bool,
    /// Logarithmic population axis.
    pub log_populations: bool,
}

impl Default for EcoUi {
    fn default() -> Self {
        Self {
            camera: CameraMode::Orbit,
            overlay: Overlay::None,
            layers: Layers::default(),
            tool: Tool::Inspect,
            settings: ToolSettings::default(),
            selection: None,
            species: None,
            tab: PropertiesTab::Scenario,
            frame_map: true,
            focus: None,
            all_species: false,
            log_populations: false,
        }
    }
}

/// Result of a batched edit sent to the simulation thread.
#[derive(Resource, Default)]
pub struct PendingJobs {
    /// Cull results: `(species label, requested, receiver of the number killed)`.
    pub culls: Vec<(String, usize, Mutex<Receiver<usize>>)>,
}
