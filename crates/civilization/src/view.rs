//! Owned render data for renderers.

use organism_engine::AgentId;
use simulation_core::glam::Vec2;

use crate::agent::{Activity, Sex, TICKS_PER_YEAR};
use crate::buildings::{BuildingId, BuildingKind};
use crate::economy::{Bundle, Good};
use crate::groups::GroupId;
use crate::state::CivState;
use crate::terrain::Terrain;

/// Rectangle of tiles `[x, x + width) × [y, y + height)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRect {
    /// Left column.
    pub x: u32,
    /// Top row.
    pub y: u32,
    /// Width in tiles.
    pub width: u32,
    /// Height in tiles.
    pub height: u32,
}

/// What a renderer asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivViewRequest {
    /// Region to include (whole world if `None`); clipped to the world.
    pub region: Option<TileRect>,
    /// Include terrain and resource levels.
    pub tiles: bool,
    /// Include the road intensity grid.
    pub roads: bool,
    /// Include the territory grid.
    pub territory: bool,
}

impl Default for CivViewRequest {
    fn default() -> Self {
        Self { region: None, tiles: true, roads: true, territory: true }
    }
}

/// One agent as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentView {
    /// Identifier.
    pub id: AgentId,
    /// Position in tile units.
    pub position: Vec2,
    /// Facing in radians.
    pub heading: f32,
    /// Group (for colouring).
    pub group: Option<GroupId>,
    /// Current activity.
    pub activity: Activity,
    /// Carried food, wood, stone and tools.
    pub carrying: Bundle,
    /// Metabolic reserve in `[0, 1]`.
    pub energy: f32,
    /// Health in `[0, 1]`.
    pub health: f32,
    /// Age in years.
    pub age_years: f32,
    /// Sex.
    pub sex: Sex,
    /// Founding population index.
    pub population: u8,
    /// Whether the agent is an adult.
    pub adult: bool,
}

/// One building as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingView {
    /// Identifier.
    pub id: BuildingId,
    /// Kind.
    pub kind: BuildingKind,
    /// Tile coordinates.
    pub tile: (u32, u32),
    /// Owning group.
    pub group: Option<GroupId>,
    /// Construction progress in `[0, 1]`.
    pub progress: f32,
    /// Crop growth in `[0, 1]` (farm plots with a crop).
    pub crop_growth: Option<f32>,
    /// Food stored (group storage and household stores in huts).
    pub stored_food: f32,
}

/// One group as listed in legends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupView {
    /// Identifier.
    pub id: GroupId,
    /// RGBA colour.
    pub color: [f32; 4],
    /// Members.
    pub members: u32,
    /// Claimed tiles.
    pub territory: u32,
    /// Contested claimed tiles.
    pub contested: u32,
}

/// Render data for one request. Grids are row-major over `region`.
#[derive(Clone, Debug, PartialEq)]
pub struct CivView {
    /// Tick the view was built at.
    pub tick: u64,
    /// World width in tiles.
    pub world_width: u32,
    /// World height in tiles.
    pub world_height: u32,
    /// Region covered by the grids.
    pub region: TileRect,
    /// Terrain per tile (empty unless requested).
    pub terrain: Vec<Terrain>,
    /// Wild resource level `stock / capacity` per tile (0 without a resource).
    pub resources: Vec<f32>,
    /// Trail intensity per tile in `[0, 1)` (empty unless requested).
    pub roads: Vec<f32>,
    /// Trail intensity at which a tile counts as a road.
    pub road_threshold: f32,
    /// Owning group id per tile, 0 for unclaimed (empty unless requested).
    pub territory: Vec<u32>,
    /// Contested flag per tile (empty unless requested).
    pub contested: Vec<bool>,
    /// Agents inside the region.
    pub agents: Vec<AgentView>,
    /// Buildings inside the region.
    pub buildings: Vec<BuildingView>,
    /// All groups.
    pub groups: Vec<GroupView>,
}

fn clip(request: Option<TileRect>, width: u32, height: u32) -> TileRect {
    let r = request.unwrap_or(TileRect { x: 0, y: 0, width, height });
    let x = r.x.min(width);
    let y = r.y.min(height);
    TileRect { x, y, width: r.width.min(width - x), height: r.height.min(height - y) }
}

/// Builds the view of `state` for `request`.
pub(crate) fn build(state: &CivState, request: &CivViewRequest) -> CivView {
    let world = &state.world;
    let region = clip(request.region, world.width(), world.height());
    let tiles: Vec<u32> = (region.y..region.y + region.height)
        .flat_map(|y| (region.x..region.x + region.width).map(move |x| (x, y)))
        .map(|(x, y)| world.index(x, y))
        .collect();
    let inside = |p: Vec2| {
        p.x >= region.x as f32
            && p.y >= region.y as f32
            && p.x < (region.x + region.width) as f32
            && p.y < (region.y + region.height) as f32
    };
    let (terrain, resources) = if request.tiles {
        (
            tiles.iter().map(|&t| world.terrain(t)).collect(),
            tiles
                .iter()
                .map(|&t| if world.capacity(t) > 0.0 { world.stock(t) / world.capacity(t) } else { 0.0 })
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let roads = if request.roads { tiles.iter().map(|&t| world.road(t)).collect() } else { Vec::new() };
    let (territory, contested) = if request.territory {
        (
            tiles.iter().map(|&t| world.claim(t).owner.map_or(0, |g| g.0)).collect(),
            tiles.iter().map(|&t| world.claim(t).contested).collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let agents = state
        .agents
        .iter()
        .filter(|(_, a)| inside(a.kin.position))
        .map(|(id, a)| AgentView {
            id,
            position: a.kin.position,
            heading: a.kin.heading,
            group: a.group,
            activity: a.doing,
            carrying: a.inventory,
            energy: a.energy,
            health: a.health,
            age_years: a.age as f32 / TICKS_PER_YEAR as f32,
            sex: a.sex,
            population: a.population,
            adult: a.is_adult(),
        })
        .collect();
    let buildings = state
        .buildings
        .iter()
        .filter(|b| inside(world.center(b.tile)))
        .map(|b| BuildingView {
            id: b.id,
            kind: b.kind,
            tile: world.coords(b.tile),
            group: b.group,
            progress: b.progress,
            crop_growth: b.crop.map(|c| c.growth),
            stored_food: b.store.get(Good::Food),
        })
        .collect();
    let groups = state
        .groups
        .iter()
        .map(|g| GroupView {
            id: g.id,
            color: g.color,
            members: g.members,
            territory: g.territory,
            contested: g.contested,
        })
        .collect();
    CivView {
        tick: state.tick,
        world_width: world.width(),
        world_height: world.height(),
        region,
        terrain,
        resources,
        roads,
        road_threshold: state.rules.road_threshold,
        territory,
        contested,
        agents,
        buildings,
        groups,
    }
}
