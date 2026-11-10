//! Interaction state of the mode (tools, selection, panels, display).

use bevy::prelude::*;
use evolution::Species;
use evolution::genes::{GENES, HERITABLE_COUNT, MARKER_A, PERCEPTION, SIZE, SPEED};
use organism_engine::AgentId;
use rendering::OrbitPreset;

use crate::genealogy::TreeView;

/// Sub-tabs of the Properties panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PropertiesTab {
    /// Selected organism.
    #[default]
    Inspector,
    /// Gene statistics and charts.
    Genes,
    /// Genealogy tree.
    Genealogy,
    /// Regions, divergence matrix, speciation.
    Regions,
}

impl PropertiesTab {
    /// Every tab in display order.
    pub const ALL: [PropertiesTab; 4] = [Self::Inspector, Self::Genes, Self::Genealogy, Self::Regions];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Inspector => "Inspector",
            Self::Genes => "Genes",
            Self::Genealogy => "Genealogy",
            Self::Regions => "Regions",
        }
    }

    /// Parses a tab name (case-insensitive).
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(name.trim()))
    }
}

/// World tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Click an organism to inspect it.
    #[default]
    Select,
    /// Click to add food in a disc.
    Food,
    /// Click to cull a fraction of the organisms in a disc.
    Cull,
    /// Click to spawn organisms with the custom genome.
    Spawn,
    /// Click to introduce predators.
    Predators,
}

impl Tool {
    /// Every tool, in shortcut order (1–5).
    pub const ALL: [Tool; 5] = [Self::Select, Self::Food, Self::Cull, Self::Spawn, Self::Predators];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Food => "Add food",
            Self::Cull => "Cull",
            Self::Spawn => "Spawn genome",
            Self::Predators => "Predators",
        }
    }

    /// One-line explanation.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Select => "Click an organism to inspect it; click empty ground to clear the selection.",
            Self::Food => "Click the ground to add food to every land cell in the circle.",
            Self::Cull => "Click to remove a random fraction of the organisms in the circle.",
            Self::Spawn => "Click to release organisms carrying the custom genome below.",
            Self::Predators => "Click to introduce predators in the circle.",
        }
    }

    /// Whether the tool acts on a disc of the world.
    pub fn uses_brush(self) -> bool {
        self != Self::Select
    }
}

/// Ground tint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GroundOverlay {
    /// Temperature on the thermal colour scale.
    #[default]
    Temperature,
    /// Food per cell.
    Food,
    /// Regions (islands or sectors).
    Regions,
    /// Neutral.
    Plain,
}

impl GroundOverlay {
    /// Every overlay.
    pub const ALL: [GroundOverlay; 4] = [Self::Temperature, Self::Food, Self::Regions, Self::Plain];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Food => "Food",
            Self::Regions => "Regions",
            Self::Plain => "Plain",
        }
    }

    /// Next overlay (shortcut V).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|o| *o == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Which organisms a cull affects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CullTarget {
    /// Prey only.
    #[default]
    Prey,
    /// Predators only.
    Predators,
    /// Both species.
    Both,
}

impl CullTarget {
    /// The model's species filter.
    pub fn species(self) -> Option<Species> {
        match self {
            Self::Prey => Some(Species::Prey),
            Self::Predators => Some(Species::Predator),
            Self::Both => None,
        }
    }
}

/// Distance measure of the region matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MatrixMeasure {
    /// Net divergence `d_AB − (d_A + d_B)/2`.
    #[default]
    Net,
    /// Mean between-region distance `d_AB`.
    Between,
}

/// Custom genome for the Spawn tool (phenotypic values).
#[derive(Clone, Debug, PartialEq)]
pub struct GenomeEditor {
    /// Value per heritable gene.
    pub values: [f32; HERITABLE_COUNT],
    /// Organisms per click.
    pub count: u32,
    /// Spread around the values (normalised units; 0 = clones).
    pub spread: f32,
    /// Species spawned.
    pub species: Species,
}

impl Default for GenomeEditor {
    fn default() -> Self {
        Self { values: std::array::from_fn(|i| GENES[i].default), count: 20, spread: 0.02, species: Species::Prey }
    }
}

/// UI state of the mode.
#[derive(Resource, Debug, Clone)]
pub struct EvoUi {
    /// Properties sub-tab.
    pub tab: PropertiesTab,
    /// World tool.
    pub tool: Tool,
    /// Brush radius (cells).
    pub brush_radius: f32,
    /// Food added per cell by the food tool.
    pub food_amount: f32,
    /// Fraction removed by a cull.
    pub cull_fraction: f32,
    /// Species culled.
    pub cull_target: CullTarget,
    /// Predators introduced per click.
    pub predator_count: u32,
    /// Predator founder speed, perception and size.
    pub predator_traits: [f32; 3],
    /// Custom genome.
    pub genome: GenomeEditor,
    /// Temperature step of the climate buttons (°C).
    pub temp_step: f32,
    /// Climate trend being edited (°C per 1000 ticks).
    pub trend: f32,
    /// Running trend the editor was last synchronised with.
    pub trend_seen: Option<f32>,
    /// Ground tint.
    pub overlay: GroundOverlay,
    /// Show vegetation.
    pub vegetation: bool,
    /// Camera preset.
    pub camera_preset: OrbitPreset,
    /// Gene shown in the charts (heritable index).
    pub chart_gene: usize,
    /// Species shown in the charts.
    pub chart_species: Species,
    /// Selected organism.
    pub selected: Option<AgentId>,
    /// Organism under the cursor.
    pub hovered: Option<AgentId>,
    /// Camera follows the selection.
    pub follow: bool,
    /// Seed typed for a replicate run.
    pub seed_input: u64,
    /// Genealogy canvas.
    pub tree: TreeView,
    /// Region matrix measure.
    pub matrix: MatrixMeasure,
    /// Frame at which the Properties panel was last drawn.
    pub properties_frame: u32,
    /// Current frame (written every frame).
    pub frame: u32,
    /// Show the overlay legend.
    pub legend_open: bool,
    /// Show the all-genes table in the Genes tab.
    pub genes_table_open: bool,
}

impl Default for EvoUi {
    fn default() -> Self {
        Self {
            tab: PropertiesTab::Inspector,
            tool: Tool::Select,
            brush_radius: 5.0,
            food_amount: 6.0,
            cull_fraction: 0.5,
            cull_target: CullTarget::Prey,
            predator_count: 10,
            predator_traits: [0.85, 9.0, 1.4],
            genome: GenomeEditor::default(),
            temp_step: 2.0,
            trend: 0.0,
            trend_seen: None,
            overlay: GroundOverlay::Temperature,
            vegetation: true,
            camera_preset: OrbitPreset::Strategic,
            chart_gene: SPEED,
            chart_species: Species::Prey,
            selected: None,
            hovered: None,
            follow: false,
            seed_input: 0,
            tree: TreeView::default(),
            matrix: MatrixMeasure::Net,
            properties_frame: 0,
            frame: 0,
            legend_open: true,
            genes_table_open: true,
        }
    }
}

impl EvoUi {
    /// Resets per-run state when a new experiment starts.
    pub fn on_new_run(&mut self, seed: u64) {
        self.selected = None;
        self.hovered = None;
        self.follow = false;
        self.seed_input = seed;
        self.trend_seen = None;
        self.tree.collapsed.clear();
        self.tree.fit();
    }

    /// Whether the genealogy tree is on screen (so it must be fetched).
    pub fn wants_genealogy(&self) -> bool {
        self.tab == PropertiesTab::Genealogy && self.frame.saturating_sub(self.properties_frame) <= 2
    }

    /// Selects a chart gene by name (heritable genes only).
    pub fn select_gene(&mut self, name: &str) -> bool {
        match evolution::genes::gene_index(name).filter(|i| *i < HERITABLE_COUNT) {
            Some(i) => {
                self.chart_gene = i;
                true
            }
            None => false,
        }
    }

    /// Copies the running climate trend into the trend editor whenever it
    /// changes (new run, Parameters tab, Set climate trend), so the editor
    /// never silently proposes a stale value.
    pub fn sync_trend(&mut self, running: f32) {
        if self.trend_seen != Some(running) {
            self.trend_seen = Some(running);
            self.trend = running;
        }
    }

    /// Next camera preset (shortcut C).
    pub fn cycle_camera(&mut self) {
        let i = OrbitPreset::ALL.iter().position(|p| *p == self.camera_preset).unwrap_or(0);
        self.camera_preset = OrbitPreset::ALL[(i + 1) % OrbitPreset::ALL.len()];
    }

    /// Predator founder traits as model trait values.
    pub fn predator_trait_values(&self) -> Vec<evolution::TraitValue> {
        [SPEED, PERCEPTION, SIZE]
            .iter()
            .zip(self.predator_traits)
            .map(|(&g, v)| evolution::TraitValue::new(GENES[g].name, v.clamp(GENES[g].min, GENES[g].max)))
            .collect()
    }

    /// Custom genome as model trait values.
    pub fn genome_trait_values(&self) -> Vec<evolution::TraitValue> {
        self.genome
            .values
            .iter()
            .enumerate()
            .map(|(g, v)| evolution::TraitValue::new(GENES[g].name, v.clamp(GENES[g].min, GENES[g].max)))
            .collect()
    }
}

/// Gene picked by default for the drift lesson.
pub const DRIFT_GENE: usize = MARKER_A;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_parse_and_tools_cycle() {
        assert_eq!(PropertiesTab::parse("genes"), Some(PropertiesTab::Genes));
        assert_eq!(PropertiesTab::parse(" Genealogy "), Some(PropertiesTab::Genealogy));
        assert_eq!(PropertiesTab::parse("charts"), None);
        assert!(!Tool::Select.uses_brush() && Tool::Food.uses_brush());
        assert_eq!(GroundOverlay::Plain.next(), GroundOverlay::Temperature);
        assert_eq!(CullTarget::Both.species(), None);
    }

    #[test]
    fn genome_editor_produces_valid_traits() {
        let mut ui = EvoUi::default();
        ui.genome.values[0] = 99.0;
        let traits = ui.genome_trait_values();
        assert_eq!(traits.len(), HERITABLE_COUNT);
        for t in &traits {
            t.resolve().unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(traits[0].value, GENES[0].max, "values are clamped to the gene range");
        for t in ui.predator_trait_values() {
            t.resolve().unwrap();
        }
    }

    #[test]
    fn trend_editor_follows_the_running_trend() {
        let mut ui = EvoUi::default();
        ui.sync_trend(6.0);
        assert_eq!(ui.trend, 6.0);
        ui.trend = 15.0;
        ui.sync_trend(6.0);
        assert_eq!(ui.trend, 15.0, "an edit is kept while the running trend is unchanged");
        ui.sync_trend(-2.0);
        assert_eq!(ui.trend, -2.0);
        ui.on_new_run(3);
        ui.sync_trend(-2.0);
        assert_eq!(ui.trend, -2.0);
        assert_eq!(ui.trend_seen, Some(-2.0));
    }

    #[test]
    fn genealogy_is_wanted_only_while_visible() {
        let mut ui = EvoUi { tab: PropertiesTab::Genealogy, frame: 10, properties_frame: 9, ..EvoUi::default() };
        assert!(ui.wants_genealogy());
        ui.frame = 30;
        assert!(!ui.wants_genealogy(), "the panel is hidden");
        assert!(ui.select_gene("marker_a"));
        assert_eq!(ui.chart_gene, DRIFT_GENE);
        assert!(!ui.select_gene("acclimation"), "acquired traits are not charted");
        let before = ui.camera_preset;
        for _ in 0..OrbitPreset::ALL.len() {
            ui.cycle_camera();
        }
        assert_eq!(ui.camera_preset, before);
    }
}
