//! Buildings: huts, group storage and farm plots.

use organism_engine::AgentId;
use serde::{Deserialize, Serialize};

use crate::economy::Bundle;
use crate::groups::GroupId;

/// Ticks a planted crop needs to mature on fully fertile land.
pub const CROP_GROWTH_TICKS: f32 = 96.0;
/// Food yield of a mature crop on fully fertile land with quality 1.
pub const CROP_YIELD: f32 = 10.0;
/// Ticks a mature crop stays harvestable before it rots.
pub const CROP_SPOIL_TICKS: u32 = 120;
/// Food used as seed when planting.
pub const SEED_FOOD: f32 = 0.5;
/// Ticks an abandoned building (no owner, no group) takes to fall apart.
pub const ABANDON_TICKS: f32 = 1000.0;

/// Stable building identifier (never reused; `0` is never issued).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuildingId(pub u32);

/// Kind of building.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildingKind {
    /// Home: faster rest, a feeling of safety and a household food store
    /// for the people living in it.
    Hut,
    /// Group food store: members deposit surplus and eat from it.
    Storage,
    /// Farm plot: plant, wait for growth, harvest.
    FarmPlot,
}

impl BuildingKind {
    /// All kinds.
    pub const ALL: [BuildingKind; 3] = [Self::Hut, Self::Storage, Self::FarmPlot];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hut => "hut",
            Self::Storage => "storage",
            Self::FarmPlot => "farm plot",
        }
    }

    /// Materials consumed when the site is laid out.
    pub fn cost(self) -> Bundle {
        match self {
            Self::Hut => Bundle::new(0.0, 6.0, 3.0, 0.0),
            Self::Storage => Bundle::new(0.0, 8.0, 6.0, 0.0),
            Self::FarmPlot => Bundle::new(0.0, 2.0, 0.0, 0.0),
        }
    }

    /// Work units needed to finish construction (one skilled, equipped
    /// worker contributes about 0.06–0.16 per tick).
    pub fn work(self) -> f32 {
        match self {
            Self::Hut => 2.0,
            Self::Storage => 3.0,
            Self::FarmPlot => 0.8,
        }
    }

    /// Relative territorial influence of a finished building.
    pub fn influence(self) -> f32 {
        match self {
            Self::Hut => 1.0,
            Self::Storage => 1.6,
            Self::FarmPlot => 0.6,
        }
    }
}

/// A growing or mature crop on a farm plot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    /// Growth in `[0, 1]`; mature at 1.
    pub growth: f32,
    /// `0.5 + farming skill` of the planter (scales the yield).
    pub quality: f32,
    /// Food left to harvest (set when the crop matures).
    pub yield_left: f32,
    /// Ticks since maturity.
    pub ripe_ticks: u32,
}

impl Crop {
    /// Whether the crop can be harvested.
    pub fn is_ripe(&self) -> bool {
        self.growth >= 1.0 && self.yield_left > 0.0
    }
}

/// A building on one tile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Building {
    /// Identifier.
    pub id: BuildingId,
    /// Kind.
    pub kind: BuildingKind,
    /// Tile index.
    pub tile: u32,
    /// Owner (`None` after the owner died without heirs).
    pub owner: Option<AgentId>,
    /// Group the building belongs to.
    pub group: Option<GroupId>,
    /// Construction progress in `[0, 1]`; finished at 1.
    pub progress: f32,
    /// Stored goods (group storage and household stores in huts).
    pub store: Bundle,
    /// Crop (farm plots only).
    pub crop: Option<Crop>,
    /// Structural condition in `[0, 1]`; abandoned buildings decay and vanish at 0.
    pub condition: f32,
}

impl Building {
    /// Whether construction is finished.
    pub fn is_complete(&self) -> bool {
        self.progress >= 1.0
    }
}

/// All buildings, sorted by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Buildings {
    items: Vec<Building>,
    next_id: u32,
}

impl Default for Buildings {
    fn default() -> Self {
        Self { items: Vec::new(), next_id: 1 }
    }
}

impl Buildings {
    /// Buildings in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Building> {
        self.items.iter()
    }

    /// Mutable buildings in id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Building> {
        self.items.iter_mut()
    }

    /// Number of buildings (including sites under construction).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no buildings.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Building by id.
    pub fn get(&self, id: BuildingId) -> Option<&Building> {
        self.items.binary_search_by_key(&id, |b| b.id).ok().map(|i| &self.items[i])
    }

    /// Mutable building by id.
    pub fn get_mut(&mut self, id: BuildingId) -> Option<&mut Building> {
        self.items.binary_search_by_key(&id, |b| b.id).ok().map(|i| &mut self.items[i])
    }

    /// Lays out a new construction site and returns its id.
    pub fn insert(&mut self, kind: BuildingKind, tile: u32, owner: AgentId, group: Option<GroupId>) -> BuildingId {
        let id = BuildingId(self.next_id);
        self.next_id += 1;
        self.items.push(Building {
            id,
            kind,
            tile,
            owner: Some(owner),
            group,
            progress: 0.0,
            store: Bundle::ZERO,
            crop: None,
            condition: 1.0,
        });
        id
    }

    /// Removes buildings for which `keep` is false and returns them.
    pub fn remove_where(&mut self, mut keep: impl FnMut(&Building) -> bool) -> Vec<Building> {
        let mut removed = Vec::new();
        let mut kept = Vec::with_capacity(self.items.len());
        for b in self.items.drain(..) {
            if keep(&b) {
                kept.push(b);
            } else {
                removed.push(b);
            }
        }
        self.items = kept;
        removed
    }

    /// Checks the id ordering after deserialisation.
    pub fn validate(&self) -> Result<(), String> {
        let mut last = 0;
        for b in &self.items {
            if b.id.0 <= last || b.id.0 >= self.next_id {
                return Err(format!("building ids are not increasing below the counter ({})", b.id.0));
            }
            last = b.id.0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_increase_and_lookup_works() {
        let mut b = Buildings::default();
        let a = b.insert(BuildingKind::Hut, 3, AgentId(1), None);
        let c = b.insert(BuildingKind::FarmPlot, 9, AgentId(2), Some(GroupId(1)));
        assert!(a < c);
        assert_eq!(b.get(c).map(|x| x.tile), Some(9));
        assert!(!b.get(a).is_some_and(Building::is_complete));
        let removed = b.remove_where(|x| x.kind != BuildingKind::Hut);
        assert_eq!(removed.len(), 1);
        assert!(b.get(a).is_none());
        let d = b.insert(BuildingKind::Storage, 1, AgentId(1), None);
        assert!(d > c, "ids are never reused");
        assert!(b.validate().is_ok());
    }
}
