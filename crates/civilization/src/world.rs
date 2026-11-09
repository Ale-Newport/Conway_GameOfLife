//! The tile world: terrain, regenerating resources, trails/roads, danger,
//! building occupancy and territory.
//!
//! **Roads** follow the active-walker trail model (Helbing, Keltsch & Molnár
//! 1997) on tiles. Every time an agent enters a tile it deposits
//! `ΔG = 1 − G/G_max` (saturating footfall), and every tick trails fade,
//! `G ← G·(1 − 1/T)`. The intensity `I = G/G_max ∈ [0, 1)` lowers the
//! movement cost, `c = c_terrain · (1 − k·I)`, which in turn attracts more
//! route planners: frequently used routes reinforce themselves into roads.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;
use simulation_core::glam::Vec2;

use crate::config::Rules;
use crate::error::CivError;
use crate::groups::GroupId;
use crate::terrain::{Resource, Terrain};

/// Trail saturation level `G_max`.
pub const TRAIL_MAX: f32 = 100.0;
/// Fraction of danger kept per tick.
pub const DANGER_RETENTION: f32 = 0.97;

/// Territory state of a tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// Owning group.
    pub owner: Option<GroupId>,
    /// Whether another group's influence nearly equals the owner's.
    pub contested: bool,
}

/// Tile world state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    terrain: Grid2<Terrain>,
    richness: f32,
    stock: Vec<f32>,
    capacity: Vec<f32>,
    trail: Vec<f32>,
    danger: Vec<f32>,
    near_water: Vec<bool>,
    building_at: Vec<u32>,
    territory: Vec<Claim>,
    /// Label of the 4-connected passable region of each tile (0 =
    /// impassable). Derived from the terrain, so not serialised; rebuilt by
    /// [`World::rebuild_regions`].
    #[serde(skip)]
    region: Vec<u32>,
}

impl World {
    /// World over `terrain` with resource stocks full. `richness` scales
    /// capacities and growback.
    pub fn new(terrain: Grid2<Terrain>, richness: f32) -> Self {
        let n = terrain.as_slice().len();
        let mut world = Self {
            terrain,
            richness,
            stock: vec![0.0; n],
            capacity: vec![0.0; n],
            trail: vec![0.0; n],
            danger: vec![0.0; n],
            near_water: vec![false; n],
            building_at: vec![0; n],
            territory: vec![Claim::default(); n],
            region: Vec::new(),
        };
        for i in 0..n {
            world.reset_resource(i);
        }
        world.refresh_near_water(0..n);
        world.rebuild_regions();
        world
    }

    /// Labels the 4-connected regions of passable tiles (flood fill in index
    /// order). Moves between 8-neighbours are only allowed when both
    /// orthogonal tiles are passable, so A* reachability equals 4-connectivity.
    pub fn rebuild_regions(&mut self) {
        let n = self.len();
        self.region = vec![0; n];
        let mut label = 0;
        let mut stack = Vec::new();
        for start in 0..n {
            if self.region[start] != 0 || !self.passable(start as u32) {
                continue;
            }
            label += 1;
            self.region[start] = label;
            stack.push(start as u32);
            while let Some(t) = stack.pop() {
                let (x, y) = self.coords(t);
                let (w, h) = (self.width(), self.height());
                let around = [
                    (x > 0).then(|| t - 1),
                    (x + 1 < w).then(|| t + 1),
                    (y > 0).then(|| t - w),
                    (y + 1 < h).then(|| t + w),
                ];
                for nb in around.into_iter().flatten() {
                    if self.region[nb as usize] == 0 && self.passable(nb) {
                        self.region[nb as usize] = label;
                        stack.push(nb);
                    }
                }
            }
        }
    }

    /// Whether an agent on tile `a` can walk to tile `b`.
    pub fn connected(&self, a: u32, b: u32) -> bool {
        match (self.region.get(a as usize), self.region.get(b as usize)) {
            (Some(&ra), Some(&rb)) => ra != 0 && ra == rb,
            _ => false,
        }
    }

    /// Width in tiles.
    pub fn width(&self) -> u32 {
        self.terrain.width() as u32
    }

    /// Height in tiles.
    pub fn height(&self) -> u32 {
        self.terrain.height() as u32
    }

    /// Number of tiles.
    pub fn len(&self) -> usize {
        self.stock.len()
    }

    /// Whether the world has no tiles (never true for validated worlds).
    pub fn is_empty(&self) -> bool {
        self.stock.is_empty()
    }

    /// Capacity and growback multiplier.
    pub fn richness(&self) -> f32 {
        self.richness
    }

    /// Tile index of `(x, y)`.
    pub fn index(&self, x: u32, y: u32) -> u32 {
        y * self.width() + x
    }

    /// Coordinates of tile `i`.
    pub fn coords(&self, i: u32) -> (u32, u32) {
        (i % self.width(), i / self.width())
    }

    /// Centre of tile `i` in world units.
    pub fn center(&self, i: u32) -> Vec2 {
        let (x, y) = self.coords(i);
        Vec2::new(x as f32 + 0.5, y as f32 + 0.5)
    }

    /// Tile under a position, if inside the world.
    pub fn tile_at(&self, p: Vec2) -> Option<u32> {
        if !(p.x >= 0.0 && p.y >= 0.0) {
            return None;
        }
        let (x, y) = (p.x as u32, p.y as u32);
        (x < self.width() && y < self.height()).then(|| self.index(x, y))
    }

    /// Terrain grid.
    pub fn terrain_grid(&self) -> &Grid2<Terrain> {
        &self.terrain
    }

    /// Terrain of tile `i`.
    pub fn terrain(&self, i: u32) -> Terrain {
        self.terrain.as_slice()[i as usize]
    }

    /// Whether agents can stand on tile `i`.
    pub fn passable(&self, i: u32) -> bool {
        self.terrain(i).passable()
    }

    /// Whether a position is on a passable tile.
    pub fn passable_at(&self, p: Vec2) -> bool {
        self.tile_at(p).is_some_and(|i| self.passable(i))
    }

    /// Whether tile `i` is land touching water (drinking spot).
    pub fn near_water(&self, i: u32) -> bool {
        self.near_water[i as usize]
    }

    /// Shore tile nearest to `p` (ties to the lower index), if any.
    pub fn nearest_shore(&self, p: Vec2) -> Option<u32> {
        self.nearest_of(&self.shore_tiles(), p)
    }

    /// Tile of `tiles` nearest to `p` (ties to the earlier entry), if any.
    /// Callers placing many agents compute [`World::shore_tiles`] once and
    /// use this instead of [`World::nearest_shore`].
    pub fn nearest_of(&self, tiles: &[u32], p: Vec2) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        for &i in tiles {
            let d = p.distance_squared(self.center(i));
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Current wild resource stock of tile `i`.
    pub fn stock(&self, i: u32) -> f32 {
        self.stock[i as usize]
    }

    /// Stock capacity of tile `i`.
    pub fn capacity(&self, i: u32) -> f32 {
        self.capacity[i as usize]
    }

    /// Trail intensity `G/G_max` of tile `i` in `[0, 1)`.
    pub fn road(&self, i: u32) -> f32 {
        self.trail[i as usize] / TRAIL_MAX
    }

    /// Danger level of tile `i` (recent conflicts).
    pub fn danger(&self, i: u32) -> f32 {
        self.danger[i as usize]
    }

    /// Building on tile `i`, as the raw id (0 = none).
    pub fn building_at(&self, i: u32) -> u32 {
        self.building_at[i as usize]
    }

    /// Territory of tile `i`.
    pub fn claim(&self, i: u32) -> Claim {
        self.territory[i as usize]
    }

    /// Movement cost of tile `i`: terrain cost times the road discount
    /// `1 − k·I` (infinite on impassable tiles).
    pub fn move_cost(&self, i: u32, rules: &Rules) -> f32 {
        self.terrain(i).move_cost() * (1.0 - rules.road_discount * self.road(i))
    }

    /// Indices of the (up to 8) neighbours of tile `i`, in a fixed order.
    pub fn neighbors(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let (x, y) = self.coords(i);
        let (w, h) = (i64::from(self.width()), i64::from(self.height()));
        const OFFSETS: [(i64, i64); 8] = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)];
        OFFSETS.into_iter().filter_map(move |(dx, dy)| {
            let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
            (nx >= 0 && ny >= 0 && nx < w && ny < h).then(|| (ny * w + nx) as u32)
        })
    }

    fn reset_resource(&mut self, i: usize) {
        let cap = self.terrain.as_slice()[i].resource().map_or(0.0, |r| r.base_capacity() * self.richness);
        self.capacity[i] = cap;
        self.stock[i] = cap;
    }

    fn refresh_near_water(&mut self, range: impl IntoIterator<Item = usize>) {
        for i in range {
            let land = self.terrain.as_slice()[i].passable();
            let wet = self.neighbors(i as u32).any(|n| self.terrain(n) == Terrain::Water);
            self.near_water[i] = land && wet;
        }
    }

    /// Changes the terrain of tile `i` (ignored outside the world); the new
    /// resource starts full.
    pub fn set_terrain(&mut self, i: u32, terrain: Terrain) {
        self.set_terrains(std::iter::once(i), terrain);
    }

    /// Changes the terrain of every tile in `tiles` (indices outside the
    /// world are ignored); the new resources start full. Shores are refreshed
    /// around the changed tiles and the region labels are rebuilt once, so an
    /// area edit costs `O(area + tiles)` rather than `O(area · tiles)`.
    pub fn set_terrains(&mut self, tiles: impl IntoIterator<Item = u32>, terrain: Terrain) {
        let mut touched: Vec<usize> = Vec::new();
        for i in tiles {
            let Some(slot) = self.terrain.as_mut_slice().get_mut(i as usize) else { continue };
            *slot = terrain;
            self.reset_resource(i as usize);
            touched.extend(std::iter::once(i).chain(self.neighbors(i)).map(|n| n as usize));
        }
        if touched.is_empty() {
            return;
        }
        touched.sort_unstable();
        touched.dedup();
        self.refresh_near_water(touched);
        self.rebuild_regions();
    }

    /// Indices of all shore tiles (passable land touching water), ascending.
    pub fn shore_tiles(&self) -> Vec<u32> {
        (0..self.len() as u32).filter(|&i| self.near_water(i)).collect()
    }

    /// Removes up to `amount` from the stock of tile `i` and returns what was taken.
    pub fn take(&mut self, i: u32, amount: f32) -> f32 {
        let stock = &mut self.stock[i as usize];
        let taken = amount.clamp(0.0, *stock);
        *stock -= taken;
        taken
    }

    /// Sets the stock of tile `i` (clamped to `[0, capacity]`).
    pub fn set_stock(&mut self, i: u32, amount: f32) {
        self.stock[i as usize] = amount.clamp(0.0, self.capacity[i as usize]);
    }

    /// Records footfall on tile `i` (saturating deposit).
    pub fn add_footfall(&mut self, i: u32) {
        let g = &mut self.trail[i as usize];
        *g += 1.0 - *g / TRAIL_MAX;
    }

    /// Adds danger around a conflict at tile `i` (3×3 neighbourhood).
    pub fn add_danger(&mut self, i: u32, amount: f32) {
        self.danger[i as usize] += amount;
        let around: Vec<u32> = self.neighbors(i).collect();
        for n in around {
            self.danger[n as usize] += 0.5 * amount;
        }
    }

    /// Marks tile `i` as holding building `id` (0 clears it).
    pub fn set_building(&mut self, i: u32, id: u32) {
        self.building_at[i as usize] = id;
    }

    /// Replaces the territory layer.
    pub fn set_territory(&mut self, claims: Vec<Claim>) {
        debug_assert_eq!(claims.len(), self.territory.len());
        self.territory = claims;
    }

    /// One tick of environment dynamics: Sugarscape growback
    /// `stock ← min(capacity, stock + α·richness·multiplier)`, trail decay
    /// `G ← G(1 − 1/T)` and danger decay.
    pub fn update(&mut self, rules: &Rules) {
        let growth: Vec<f32> =
            Resource::ALL.iter().map(|r| r.base_regen() * self.richness * rules.regen_of(*r)).collect();
        for (i, t) in self.terrain.as_slice().iter().enumerate() {
            if let Some(r) = t.resource() {
                let cap = self.capacity[i];
                let s = &mut self.stock[i];
                *s = (*s + growth[r.index()]).min(cap);
            }
        }
        let keep = 1.0 - 1.0 / rules.road_decay_ticks.max(1.0);
        for g in &mut self.trail {
            *g *= keep;
        }
        for d in &mut self.danger {
            *d *= DANGER_RETENTION;
        }
    }

    /// Total wild food (berries and fish) on the map.
    pub fn wild_food(&self) -> f64 {
        self.terrain
            .as_slice()
            .iter()
            .zip(&self.stock)
            .filter(|(t, _)| matches!(t.resource(), Some(Resource::Berries | Resource::Fish)))
            .map(|(_, s)| f64::from(*s))
            .sum()
    }

    /// Checks layer sizes after deserialisation and rebuilds the derived
    /// region labels (which snapshots do not store).
    pub fn restore_derived(&mut self) -> Result<(), CivError> {
        let n = self.terrain.width() * self.terrain.height();
        let lens = [
            self.terrain.as_slice().len(),
            self.stock.len(),
            self.capacity.len(),
            self.trail.len(),
            self.danger.len(),
            self.near_water.len(),
            self.building_at.len(),
            self.territory.len(),
        ];
        if n == 0 || lens.iter().any(|&l| l != n) {
            return Err(CivError::InvalidScenario(format!(
                "world layers have inconsistent sizes {lens:?} for {n} tiles"
            )));
        }
        self.rebuild_regions();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::parse_map;

    fn world() -> World {
        let rows: Vec<String> = vec![
            "........".into(),
            ".bb~~...".into(),
            ".TT~~...".into(),
            ".hh.....".into(),
            "........".into(),
            "....^^..".into(),
            "........".into(),
            "........".into(),
        ];
        World::new(parse_map(&rows).unwrap(), 1.0)
    }

    #[test]
    fn growback_is_linear_up_to_capacity() {
        let mut w = world();
        let berry = w.index(1, 1);
        assert_eq!(w.stock(berry), 8.0);
        assert_eq!(w.take(berry, 10.0), 8.0);
        let rules = Rules::default();
        for _ in 0..50 {
            w.update(&rules);
        }
        assert!((w.stock(berry) - 50.0 * 0.02).abs() < 1e-4);
        for _ in 0..1000 {
            w.update(&rules);
        }
        assert_eq!(w.stock(berry), 8.0);
        let mut zero = rules.clone();
        zero.regen[Resource::Berries.index()] = 0.0;
        w.take(berry, 8.0);
        w.update(&zero);
        assert_eq!(w.stock(berry), 0.0, "regeneration multiplier 0 stops growback");
    }

    #[test]
    fn footfall_builds_trails_that_lower_cost_and_fade() {
        let mut w = world();
        let rules = Rules::default();
        let t = w.index(6, 6);
        let base = w.move_cost(t, &rules);
        for _ in 0..200 {
            w.add_footfall(t);
        }
        assert!(w.road(t) > 0.8 && w.road(t) < 1.0, "saturating: {}", w.road(t));
        assert!(w.move_cost(t, &rules) < base * 0.55);
        let before = w.road(t);
        w.update(&rules);
        assert!((w.road(t) - before * (1.0 - 1.0 / 600.0)).abs() < 1e-6);
    }

    #[test]
    fn shores_and_passability() {
        let mut w = world();
        assert!(w.near_water(w.index(2, 1)));
        assert!(!w.near_water(w.index(3, 1)), "water itself is not a shore");
        assert!(!w.passable(w.index(4, 5)));
        assert!(w.passable_at(Vec2::new(0.5, 0.5)));
        assert!(!w.passable_at(Vec2::new(-0.5, 0.5)));
        let tile = w.index(6, 3);
        w.set_terrain(tile, Terrain::Water);
        assert!(w.near_water(w.index(6, 4)));
        assert_eq!(w.stock(tile), 6.0, "new fish stock starts full");
        assert!(w.restore_derived().is_ok());
        assert_eq!(w.neighbors(0).count(), 3);
        assert!(w.connected(w.index(0, 0), w.index(7, 7)));
        assert!(!w.connected(w.index(0, 0), w.index(3, 1)), "water is not reachable");
    }

    #[test]
    fn regions_follow_four_connectivity() {
        // A diagonal gap between mountains does not connect the two sides.
        let rows: Vec<String> = vec![
            "...^....".into(),
            "...^....".into(),
            "...^....".into(),
            "....^...".into(),
            "....^...".into(),
            "....^...".into(),
            "....^...".into(),
            "....^...".into(),
        ];
        let mut w = World::new(parse_map(&rows).unwrap(), 1.0);
        assert!(!w.connected(w.index(0, 0), w.index(7, 0)));
        let gap = w.index(3, 2);
        w.set_terrain(gap, Terrain::Grassland);
        assert!(w.connected(w.index(0, 0), w.index(7, 0)), "opening the wall joins the regions");
    }

    #[test]
    fn batch_terrain_edits_equal_single_edits() {
        let mut one_by_one = world();
        let mut batch = world();
        let tiles: Vec<u32> = (0..8).map(|y| one_by_one.index(5, y)).collect();
        for &t in &tiles {
            one_by_one.set_terrain(t, Terrain::Water);
        }
        batch.set_terrains(tiles.iter().copied().chain([9999]), Terrain::Water);
        assert_eq!(batch, one_by_one, "same terrain, stocks and shores (indices outside are ignored)");
        // A full-height lake splits the land in two.
        assert!(!batch.connected(batch.index(0, 0), batch.index(7, 0)));
        assert!(batch.near_water(batch.index(6, 3)) && batch.near_water(batch.index(4, 6)));
        assert_eq!(batch.shore_tiles(), (0..batch.len() as u32).filter(|&t| batch.near_water(t)).collect::<Vec<_>>());
    }
}
