//! Owned render data ([`EcosystemView`]) and the organism inspector
//! ([`OrganismInspection`]).

mod inspect;

pub use inspect::{AnimalInspection, OrganismInspection, TreeInspection};

use organism_engine::AgentId;
use physics_world::view::downsample_mean;
use physics_world::{CalendarTime, Season, WorldView, WorldViewRequest};
use serde::{Deserialize, Serialize};
use simulation_core::Grid2;
use simulation_core::glam::Vec2;

use crate::animal::{Action, DeathCause, OrganismRef, Sex};
use crate::colony::ForagerState;
use crate::config::Rules;
use crate::fields::FieldMarker;
use crate::species::{
    ActivityPattern, BodyPlan, Locomotion, PlantLayer, Representation, SocialStructure, TaxonClass, TreeStyle,
};
use crate::state::{EcoState, Runtime};
use crate::trees::TreeStage;

/// Representative markers of density fields included at most in one view.
pub const DEFAULT_MAX_MARKERS: usize = 20_000;

/// What a renderer asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EcosystemViewRequest {
    /// Abiotic layers, fire grid and static terrain.
    pub world: WorldViewRequest,
    /// Visible region `(x0, y0, x1, y1)` in cells for animals, trees and
    /// carcasses (`None` = whole map).
    pub region: Option<(f32, f32, f32, f32)>,
    /// Include animals.
    pub animals: bool,
    /// Include trees.
    pub trees: bool,
    /// Include carcasses and clutches.
    pub remains: bool,
    /// Vegetation layers to include as block-mean grids (kg/m²).
    pub vegetation: Vec<PlantLayer>,
    /// Include the mushroom grid (kg/m²).
    pub mushrooms: bool,
    /// Include the litter grid (kg/m²).
    pub litter: bool,
    /// Block size of the vegetation, mushroom, litter, pheromone and
    /// pollination grids.
    pub block: usize,
    /// Include representative markers of the density fields (insects,
    /// spiders, worms) in the region.
    pub field_markers: bool,
    /// Largest number of field markers.
    pub max_markers: usize,
    /// Include colonies and their forager squads.
    pub colonies: bool,
    /// Include the trail-pheromone grid.
    pub pheromone: bool,
    /// Include the flower pollination grid.
    pub pollination: bool,
}

impl Default for EcosystemViewRequest {
    fn default() -> Self {
        Self {
            world: WorldViewRequest::default(),
            region: None,
            animals: true,
            trees: true,
            remains: true,
            vegetation: vec![PlantLayer::Grass, PlantLayer::Shrubs, PlantLayer::Flowers],
            mushrooms: true,
            litter: false,
            block: 1,
            field_markers: true,
            max_markers: DEFAULT_MAX_MARKERS,
            colonies: true,
            pheromone: false,
            pollination: false,
        }
    }
}

/// One animal for rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimalView {
    /// Stable id.
    pub id: u64,
    /// Species index (see [`EcosystemView::species`]).
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Heading (radians, `atan2(y, x)`).
    pub heading: f32,
    /// Velocity (cells per tick).
    pub velocity: Vec2,
    /// Current action (animation).
    pub action: Action,
    /// Age as a fraction of the lifespan.
    pub age_fraction: f32,
    /// Body length (m) at the current mass, including the visual scale.
    pub size_m: f32,
    /// Health in `[0, 1]`.
    pub health: f32,
    /// Energy reserve as a fraction of capacity.
    pub energy_fraction: f32,
    /// Sex.
    pub sex: Sex,
    /// Pregnant or forming eggs.
    pub pregnant: bool,
    /// Not yet mature.
    pub juvenile: bool,
    /// Hidden in a burrow or den.
    pub hidden: bool,
    /// Height above the ground (m); negative below the water surface for swimmers.
    pub altitude_m: f32,
    /// Injury in `[0, 1]` (limping).
    pub injury: f32,
    /// Infectious.
    pub infected: bool,
    /// Calling an alarm.
    pub alarm: bool,
    /// Social group (herd, pack, flock, school).
    pub group: Option<u64>,
}

/// One colony for rendering (nest or hive).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColonyView {
    /// Colony id.
    pub id: u64,
    /// Species index.
    pub species: u16,
    /// Nest position (cells).
    pub nest: Vec2,
    /// Workers.
    pub workers: f32,
    /// Developing brood.
    pub brood: f32,
    /// Food store (kJ).
    pub store_kj: f32,
}

/// One forager squad for rendering (it stands for many workers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForagerView {
    /// Colony id.
    pub colony: u64,
    /// Species index.
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Heading (radians).
    pub heading: f32,
    /// Height above the ground (m).
    pub altitude_m: f32,
    /// Carrying food home.
    pub loaded: bool,
}

/// One tree for rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeView {
    /// Stable id.
    pub id: u64,
    /// Tree species index.
    pub species: u16,
    /// Rendering style.
    pub style: TreeStyle,
    /// Trunk position (cells).
    pub position: Vec2,
    /// Height (m).
    pub height_m: f32,
    /// Crown radius (m).
    pub crown_radius_m: f32,
    /// Growth stage.
    pub stage: TreeStage,
    /// Health in `[0, 1]`.
    pub health: f32,
    /// Charred by fire.
    pub burnt: bool,
    /// Leaf mass relative to a full crown (deciduous phenology, browsing, fire).
    pub foliage: f32,
    /// Model id for the stage (from the species' model lists and the tree's variant).
    pub model: Option<String>,
}

/// One carcass for rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarcassView {
    /// Carcass id.
    pub id: u64,
    /// Species index.
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Remaining fraction of the body.
    pub remaining: f32,
}

/// One clutch for rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClutchView {
    /// Clutch id.
    pub id: u64,
    /// Species index.
    pub species: u16,
    /// Nest position (cells).
    pub position: Vec2,
    /// Eggs.
    pub eggs: u16,
}

/// Static information about an animal species for renderers and legends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciesInfo {
    /// Species id.
    pub id: String,
    /// Common name.
    pub name: String,
    /// Authored model id, if any.
    pub model: Option<String>,
    /// Id renderers use: the manifest model, or `proc:<rig>` for a
    /// procedural body plan of `organism-render`.
    pub visual_id: String,
    /// How the species is simulated: `individual`, `field` (density field
    /// drawn with representative markers) or `colony`.
    pub representation: String,
    /// Cruising flight altitude (m; 0 for non-fliers).
    pub flight_altitude_m: f32,
    /// Procedural body plan.
    pub body_plan: BodyPlan,
    /// Base colour.
    pub color: (f32, f32, f32),
    /// Adult body length × visual scale (m).
    pub length_m: f32,
    /// Taxonomic class.
    pub class: TaxonClass,
    /// Locomotion modes (animation sets: flying, swimming, burrowing...).
    pub locomotion: Vec<Locomotion>,
    /// Daily activity rhythm.
    pub activity: ActivityPattern,
    /// Group structure (herd, pack, flock, school...).
    pub social: SocialStructure,
    /// Living animals.
    pub population: u32,
}

/// Render data of one tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EcosystemView {
    /// Tick.
    pub tick: u64,
    /// Calendar time.
    pub time: CalendarTime,
    /// Season.
    pub season: Season,
    /// Hour of the day (fractional).
    pub hour: f32,
    /// Rule set (`"ecological"` or `"netlogo"`).
    pub rules: String,
    /// Cell size (m).
    pub cell_size_m: f32,
    /// Abiotic world (layers, fire, weather, static terrain when stale).
    pub world: WorldView,
    /// Animal species (index = species index).
    pub species: Vec<SpeciesInfo>,
    /// Animals in the region.
    pub animals: Vec<AnimalView>,
    /// Trees in the region.
    pub trees: Vec<TreeView>,
    /// Carcasses in the region.
    pub carcasses: Vec<CarcassView>,
    /// Clutches in the region.
    pub clutches: Vec<ClutchView>,
    /// Requested vegetation layers (block means, kg/m²). In the NetLogo
    /// variant the grass layer is 1 for green and 0 for brown patches.
    pub vegetation: Vec<(PlantLayer, Grid2<f32>)>,
    /// Mushrooms (block means, kg/m²).
    pub mushrooms: Option<Grid2<f32>>,
    /// Litter (block means, kg/m²).
    pub litter: Option<Grid2<f32>>,
    /// Block size of the grids above.
    pub block: usize,
    /// Inspection of the selected organism.
    pub selected: Option<OrganismInspection>,
    /// Followed organism and its current position.
    pub followed: Option<(OrganismRef, Vec2)>,
    /// Recent positions of the followed organism.
    pub trail: Vec<Vec2>,
    /// Representative markers of the density fields in the region.
    pub field_markers: Vec<FieldMarker>,
    /// Colonies in the region.
    pub colonies: Vec<ColonyView>,
    /// Forager squads in the region.
    pub foragers: Vec<ForagerView>,
    /// Trail pheromone (block means).
    pub pheromone: Option<Grid2<f32>>,
    /// Flower pollination index (block means).
    pub pollination: Option<Grid2<f32>>,
}

fn in_region(region: Option<(f32, f32, f32, f32)>, p: Vec2) -> bool {
    region.is_none_or(|(x0, y0, x1, y1)| p.x >= x0 && p.y >= y0 && p.x <= x1 && p.y <= y1)
}

impl EcoState {
    /// Builds the render view.
    pub(crate) fn build_view(&self, rt: &Runtime, request: &EcosystemViewRequest) -> EcosystemView {
        let time = self.world.time();
        let scale = rt.scale;
        let block = request.block.clamp(1, self.world.width().max(self.world.height()));
        let area = self.cell_area();
        let mut populations = vec![0u32; self.catalog.animals().len()];
        for a in self.animals.as_slice() {
            if let Some(p) = populations.get_mut(usize::from(a.species)) {
                *p += 1;
            }
        }
        for slot in 0..self.fields.len() {
            let s = usize::from(self.fields.species_of(slot));
            populations[s] = self.fields.population(slot, area).min(f64::from(u32::MAX)) as u32;
        }
        for c in &self.colonies.colonies {
            let p = &mut populations[usize::from(c.species)];
            *p = p.saturating_add(c.workers as u32);
        }
        let species = self
            .catalog
            .animals()
            .iter()
            .zip(&populations)
            .map(|(s, &population)| SpeciesInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                model: s.visual.model.clone(),
                visual_id: s.visual.visual_id(),
                representation: match s.representation {
                    Representation::Individual => "individual",
                    Representation::Field(_) => "field",
                    Representation::Colony(_) => "colony",
                }
                .to_string(),
                flight_altitude_m: if s.can(Locomotion::Fly) { s.movement.flight_altitude_m } else { 0.0 },
                body_plan: s.visual.body_plan,
                color: s.visual.color,
                length_m: s.body.length_m * s.visual.scale,
                class: s.class,
                locomotion: s.locomotion.clone(),
                activity: s.behaviour.activity,
                social: s.social.structure,
                population,
            })
            .collect();
        let animals = if request.animals {
            self.animals
                .iter()
                .filter(|(_, a)| in_region(request.region, a.kin.position))
                .map(|(id, a)| {
                    let s = usize::from(a.species);
                    let traits = &rt.traits[s];
                    let sp = &self.catalog.animals()[s];
                    AnimalView {
                        id: id.get(),
                        species: a.species,
                        position: a.kin.position,
                        heading: a.kin.heading,
                        velocity: a.kin.velocity,
                        action: a.action,
                        age_fraction: (a.age_ticks(self.tick) as f64 / traits.lifespan_ticks) as f32,
                        size_m: sp.body.length_m * sp.visual.scale * (a.mass_kg / traits.adult_mass_kg).max(0.0).cbrt(),
                        health: a.health,
                        energy_fraction: (a.energy_kj / a.reserve_max_kj(traits).max(1e-6)).clamp(0.0, 1.0),
                        sex: a.sex,
                        pregnant: a.is_pregnant(),
                        juvenile: !a.is_adult(),
                        hidden: a.hidden,
                        altitude_m: a.altitude_m,
                        injury: a.injury,
                        infected: a.infection.is_some_and(|i| i.is_infected()),
                        alarm: a.alarm.is_some(),
                        group: a.group,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let trees = if request.trees {
            self.trees
                .iter()
                .filter(|(_, t)| in_region(request.region, t.position))
                .map(|(id, t)| {
                    let sp = &self.catalog.trees()[usize::from(t.species)];
                    let stage = t.stage(sp, self.tick, &scale);
                    let models = match stage {
                        TreeStage::Seedling | TreeStage::Sapling => &sp.models.young,
                        TreeStage::Mature => &sp.models.mature,
                        TreeStage::Old => &sp.models.old,
                        TreeStage::Dead => &sp.models.dead,
                    };
                    let full = sp.full_leaf_mass(t.height_m).max(1e-6);
                    TreeView {
                        id: id.get(),
                        species: t.species,
                        style: sp.style,
                        position: t.position,
                        height_m: t.height_m,
                        crown_radius_m: sp.crown_radius(t.height_m),
                        stage,
                        health: t.health,
                        burnt: t.burnt,
                        foliage: (t.leaf_kg / full).clamp(0.0, 1.0),
                        model: (!models.is_empty()).then(|| models[usize::from(t.variant) % models.len()].clone()),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let (carcasses, clutches) = if request.remains {
            (
                self.remains
                    .carcasses
                    .iter()
                    .filter(|c| in_region(request.region, c.position))
                    .map(|c| CarcassView {
                        id: c.id,
                        species: c.species,
                        position: c.position,
                        remaining: (c.meat_kg / c.initial_kg.max(1e-6)).clamp(0.0, 1.0),
                    })
                    .collect(),
                self.remains
                    .clutches
                    .iter()
                    .filter(|c| in_region(request.region, c.position))
                    .map(|c| ClutchView { id: c.id, species: c.species, position: c.position, eggs: c.eggs })
                    .collect(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let vegetation =
            request.vegetation.iter().map(|&l| (l, downsample_mean(&self.vegetation.layer_grid(l), block))).collect();
        let (w, h) = (self.world.width(), self.world.height());
        let field_markers = if request.field_markers && !self.fields.is_empty() {
            let (x0, y0, x1, y1) = request.region.unwrap_or((0.0, 0.0, w as f32, h as f32));
            let cells = (
                x0.max(0.0).floor() as usize,
                y0.max(0.0).floor() as usize,
                (x1.max(0.0).ceil() as usize).min(w),
                (y1.max(0.0).ceil() as usize).min(h),
            );
            self.fields.markers(&self.catalog, &self.world, cells, self.tick, request.max_markers)
        } else {
            Vec::new()
        };
        let (colonies, foragers) = if request.colonies {
            let colonies = self
                .colonies
                .colonies
                .iter()
                .filter(|c| in_region(request.region, c.nest))
                .map(|c| ColonyView {
                    id: c.id,
                    species: c.species,
                    nest: c.nest,
                    workers: c.workers as f32,
                    brood: c.brood_count() as f32,
                    store_kj: c.store_kj as f32,
                })
                .collect();
            let foragers = self
                .colonies
                .foragers
                .iter()
                .filter(|f| in_region(request.region, f.position))
                .filter_map(|f| {
                    let c = self.colonies.colonies.iter().find(|c| c.id == f.colony)?;
                    let sp = &self.catalog.animals()[usize::from(c.species)];
                    let flying = sp.can(Locomotion::Fly) && f.position.distance(c.nest) > 0.5;
                    Some(ForagerView {
                        colony: f.colony,
                        species: c.species,
                        position: f.position,
                        heading: f.heading,
                        altitude_m: if flying { sp.movement.flight_altitude_m } else { 0.0 },
                        loaded: f.state == ForagerState::Returning,
                    })
                })
                .collect();
            (colonies, foragers)
        } else {
            (Vec::new(), Vec::new())
        };
        let pheromone =
            request.pheromone.then(|| downsample_mean(&Grid2::from_vec(w, h, self.colonies.pheromone.clone()), block));
        let pollination = request.pollination.then(|| downsample_mean(&self.vegetation.pollination_grid(), block));
        let selected = self.follow.selected.and_then(|r| self.inspect(rt, r).ok());
        let followed = self.follow.followed.and_then(|r| {
            let p = match r {
                OrganismRef::Animal(id) => self.animals.get(AgentId(id)).map(|a| a.kin.position),
                OrganismRef::Tree(id) => self.trees.get(AgentId(id)).map(|t| t.position),
            };
            p.map(|p| (r, p))
        });
        EcosystemView {
            tick: self.tick,
            time,
            season: time.season,
            hour: self.world.calendar().hour_of_day(self.tick) as f32,
            rules: match self.config.rules {
                Rules::Ecological => "ecological".into(),
                Rules::NetLogo(_) => "netlogo".into(),
            },
            cell_size_m: self.world.config().cell_size_m,
            world: self.world.view(&request.world),
            species,
            animals,
            trees,
            carcasses,
            clutches,
            vegetation,
            mushrooms: request.mushrooms.then(|| downsample_mean(&self.fungi.mushroom_grid(), block)),
            litter: request.litter.then(|| downsample_mean(&self.detritus.litter_grid(), block)),
            block,
            selected,
            followed,
            trail: self.follow.trail.iter().copied().collect(),
            field_markers,
            colonies,
            foragers,
            pheromone,
            pollination,
        }
    }
}

/// Causes of death in [`crate::state::Counters::deaths_by_cause`] order.
pub const DEATH_CAUSES: [DeathCause; 10] = [
    DeathCause::Predation,
    DeathCause::Starvation,
    DeathCause::Dehydration,
    DeathCause::OldAge,
    DeathCause::Fire,
    DeathCause::Drowning,
    DeathCause::Exposure,
    DeathCause::Injury,
    DeathCause::Killed,
    DeathCause::Disease,
];
