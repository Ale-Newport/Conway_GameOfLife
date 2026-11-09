//! Productive skills (learning by doing) and the kinds of production used
//! by the specialisation index.

use serde::{Deserialize, Serialize};

use crate::agent::Activity;

/// Productive skills that improve with practice and decay without it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    /// Foraging berries, fishing, logging and quarrying.
    Gathering,
    /// Planting and harvesting crops.
    Farming,
    /// Making tools.
    Crafting,
    /// Construction.
    Building,
    /// Barter.
    Trading,
}

impl Skill {
    /// All skills in index order.
    pub const ALL: [Skill; 5] = [Self::Gathering, Self::Farming, Self::Crafting, Self::Building, Self::Trading];

    /// Index into [`Skills`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Gathering => "gathering",
            Self::Farming => "farming",
            Self::Crafting => "crafting",
            Self::Building => "building",
            Self::Trading => "trading",
        }
    }
}

/// Kinds of production distinguished by the specialisation index.
/// Gathering is split by the good produced so that a division of labour
/// between food producers and material producers is visible. Barter is
/// exchange rather than production and is not a kind of work here (the
/// trading *skill* still enters the index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Work {
    /// Foraging berries and fishing.
    Forage,
    /// Logging.
    Log,
    /// Quarrying.
    Quarry,
    /// Planting and harvesting.
    Farm,
    /// Making tools.
    Craft,
    /// Construction.
    Build,
}

impl Work {
    /// Number of kinds.
    pub const COUNT: usize = 6;
    /// All kinds in index order.
    pub const ALL: [Work; Self::COUNT] = [Self::Forage, Self::Log, Self::Quarry, Self::Farm, Self::Craft, Self::Build];

    /// Index into [`crate::agent::Agent::practice`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Production done when `skill` was practised during a tick whose
    /// activity was `activity` (the activity tells which good was gathered);
    /// `None` for trading.
    pub fn of(skill: Skill, activity: Activity) -> Option<Self> {
        match skill {
            Skill::Gathering => Some(match activity {
                Activity::GatherWood => Self::Log,
                Activity::GatherStone => Self::Quarry,
                _ => Self::Forage,
            }),
            Skill::Farming => Some(Self::Farm),
            Skill::Crafting => Some(Self::Craft),
            Skill::Building => Some(Self::Build),
            Skill::Trading => None,
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Forage => "foraging",
            Self::Log => "logging",
            Self::Quarry => "quarrying",
            Self::Farm => "farming",
            Self::Craft => "crafting",
            Self::Build => "building",
        }
    }
}

/// Skill levels in `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Skills(pub [f32; 5]);

impl Skills {
    /// Level of `skill`.
    pub fn get(&self, skill: Skill) -> f32 {
        self.0[skill.index()]
    }

    /// Learning by doing for one tick: the practised skill moves towards 1,
    /// `s ← s + λ(1 − s)`; every other skill decays, `s ← s(1 − φ)`.
    pub fn practice(&mut self, practised: Option<Skill>, learn: f32, forget: f32) {
        for skill in Skill::ALL {
            let s = &mut self.0[skill.index()];
            if Some(skill) == practised {
                *s += learn * (1.0 - *s);
            } else {
                *s -= forget * *s;
            }
            *s = s.clamp(0.0, 1.0);
        }
    }
}
