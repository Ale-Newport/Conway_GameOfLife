//! Render data for the evolution laboratory.

use organism_engine::AgentId;
use organism_engine::lineage::{GenealogyEdge, GenealogySummary};
use serde::{Deserialize, Serialize};
use simulation_core::SimEvent;
use simulation_core::glam::Vec2;

use crate::config::Terrain;
use crate::environment::NO_REGION;
use crate::genes::GENE_COUNT;
use crate::organism::{Activity, Species};
use crate::speciation::{ClusterSplit, SpeciationStatus};
use crate::stats::{LineageCount, PopulationStats};
use crate::world::{EvolutionWorld, TickReport};

/// Scalar field drawn under the organisms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldLayer {
    /// Terrain only.
    None,
    /// Food per cell.
    #[default]
    Food,
    /// Temperature (°C).
    Temperature,
    /// Region labels (`-1` for impassable cells).
    Region,
}

/// What the renderer wants.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvolutionViewRequest {
    /// Field layer.
    pub field: FieldLayer,
    /// Gene whose normalised value is reported per organism for colouring
    /// (`None`: no gene value).
    pub color_gene: Option<usize>,
    /// Include genealogy nodes and ancestry edges (tree view).
    pub genealogy: bool,
}

/// One organism as drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrganismView {
    /// Stable id.
    pub id: AgentId,
    /// Species.
    pub species: Species,
    /// Position (world units).
    pub position: Vec2,
    /// Heading (radians).
    pub heading: f32,
    /// Body size (phenotype).
    pub size: f32,
    /// Energy as a fraction of storage.
    pub condition: f32,
    /// Health in `[0, 1]`.
    pub health: f32,
    /// Normalised value of the requested gene (0 if none was requested).
    pub gene_value: f32,
    /// Generation.
    pub generation: u32,
    /// Founder of its line of first parents.
    pub founder: AgentId,
    /// Behaviour of the last tick.
    pub activity: Activity,
}

/// A scalar field over the grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldData {
    /// Which field.
    pub layer: FieldLayer,
    /// Row-major values.
    pub values: Vec<f32>,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
}

/// A genealogy node for tree views.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenealogyNodeView {
    /// The organism.
    pub id: AgentId,
    /// Generation.
    pub generation: u32,
    /// Birth tick.
    pub birth_tick: u64,
    /// Death tick (`None` while alive).
    pub death_tick: Option<u64>,
}

/// Ancestry of the living population (pruned genealogy).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenealogyExport {
    /// Recorded nodes (living organisms and ancestors with living descendants), by id.
    pub nodes: Vec<GenealogyNodeView>,
    /// Parent → child edges between recorded nodes.
    pub edges: Vec<GenealogyEdge>,
    /// Living organisms per founder.
    pub lineages: Vec<LineageCount>,
    /// Counters that survive pruning.
    pub summary: GenealogySummary,
}

/// State of one barrier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarrierView {
    /// Name from the scenario.
    pub name: String,
    /// Whether it is raised.
    pub active: bool,
}

/// Owned render data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionView {
    /// Tick.
    pub tick: u64,
    /// Grid width (cells).
    pub width: usize,
    /// Grid height (cells).
    pub height: usize,
    /// Row-major terrain.
    pub terrain: Vec<Terrain>,
    /// Requested field.
    pub field: Option<FieldData>,
    /// Living organisms in id order.
    pub organisms: Vec<OrganismView>,
    /// Population statistics.
    pub stats: PopulationStats,
    /// Speciation criterion state.
    pub speciation: SpeciationStatus,
    /// Latest two-cluster split.
    pub split: Option<ClusterSplit>,
    /// Genealogy (if requested).
    pub genealogy: Option<GenealogyExport>,
    /// Barriers.
    pub barriers: Vec<BarrierView>,
    /// Events of the last tick.
    pub report: TickReport,
    /// Current temperature offset (°C).
    pub temperature_offset: f64,
    /// Up to [`RECENT_EVENTS`] most recent notable events (speciation,
    /// extinctions, interventions), newest first.
    pub recent_events: Vec<SimEvent>,
}

/// Number of events included in a view.
pub const RECENT_EVENTS: usize = 20;

impl EvolutionWorld {
    /// Ancestry edges, nodes and lineage counts of the living population.
    pub fn genealogy_export(&self) -> GenealogyExport {
        GenealogyExport {
            nodes: self
                .genealogy
                .nodes()
                .map(|n| GenealogyNodeView {
                    id: n.lineage.id,
                    generation: n.lineage.generation,
                    birth_tick: n.lineage.birth_tick,
                    death_tick: n.death_tick,
                })
                .collect(),
            edges: self.genealogy.edges(),
            lineages: self.stats.lineages.clone(),
            summary: *self.genealogy.summary(),
        }
    }

    /// Builds render data.
    pub fn view(&self, request: &EvolutionViewRequest) -> EvolutionView {
        let phys = &self.config.physiology;
        let gene = request.color_gene.filter(|g| *g < GENE_COUNT);
        let organisms = self
            .store
            .iter()
            .map(|(id, o)| {
                let p = o.phenotype(&self.schema);
                OrganismView {
                    id,
                    species: o.species,
                    position: o.kinematics.position,
                    heading: o.kinematics.heading,
                    size: p.size,
                    condition: o.condition(&p, phys),
                    health: o.health,
                    gene_value: gene.map_or(0.0, |g| o.genome.values()[g]),
                    generation: o.lineage.generation,
                    founder: o.founder,
                    activity: o.activity,
                }
            })
            .collect();
        EvolutionView {
            tick: self.tick,
            width: self.env.width(),
            height: self.env.height(),
            terrain: self.env.terrain().as_slice().to_vec(),
            field: self.field(request.field),
            organisms,
            stats: self.stats.clone(),
            speciation: self.speciation.status().clone(),
            split: self.speciation.last_split().cloned(),
            genealogy: request.genealogy.then(|| self.genealogy_export()),
            barriers: self
                .config
                .barriers
                .iter()
                .zip(self.env.barrier_active())
                .map(|(b, &active)| BarrierView { name: b.name.clone(), active })
                .collect(),
            report: self.last,
            temperature_offset: self.env.temperature_offset(),
            recent_events: self.events.recent(RECENT_EVENTS).cloned().collect(),
        }
    }

    fn field(&self, layer: FieldLayer) -> Option<FieldData> {
        let w = self.env.width();
        let values: Vec<f32> = match layer {
            FieldLayer::None => return None,
            FieldLayer::Food => self.env.food().as_slice().to_vec(),
            FieldLayer::Temperature => (0..w * self.env.height())
                .map(|i| {
                    let centre = Vec2::new((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                    self.env.temperature_at(&self.config.climate, centre, self.tick)
                })
                .collect(),
            FieldLayer::Region => self
                .env
                .regions()
                .as_slice()
                .iter()
                .map(|&r| if r == NO_REGION { -1.0 } else { f32::from(r) })
                .collect(),
        };
        let (min, max) = values.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        Some(FieldData { layer, values, min, max })
    }
}
