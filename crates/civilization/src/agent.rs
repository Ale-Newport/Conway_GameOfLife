//! Agent state: needs, energy, skills, inventory, preferences, relationships
//! and the current goal.

use organism_engine::{AgentId, Drive, Kinematics};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::buildings::{Building, BuildingId, BuildingKind};
use crate::config::Rules;
use crate::economy::{Bundle, Good};
use crate::groups::GroupId;
pub use crate::skills::{Skill, Skills, Work};

/// Ticks per simulated day (one tick is one hour).
pub const TICKS_PER_DAY: u64 = 24;
/// Ticks per simulated year. Demography is compressed: one "year" of life
/// lasts ten days (240 ticks) so that generations turn over within a session.
pub const TICKS_PER_YEAR: u64 = 240;
/// Age at which agents become adults (may build, trade, pair, steal, fight).
pub const ADULT_AGE: u64 = 14 * TICKS_PER_YEAR;
/// Age after which agents no longer reproduce.
pub const FERTILE_UNTIL: u64 = 45 * TICKS_PER_YEAR;
/// Maximum total units of food, wood and stone an agent can carry.
pub const CARRY_CAPACITY: f32 = 24.0;
/// Energy gained per unit of food eaten.
pub const FOOD_ENERGY: f32 = 0.25;
/// Loneliness gained per tick.
pub const SOCIAL_RATE: f32 = 0.004;
/// Minimum ticks between two births of the same parent.
pub const BIRTH_INTERVAL: u64 = 2 * TICKS_PER_YEAR;
/// Largest share of the carrying capacity that stocks kept for sale fill.
pub const SALE_CAPACITY_SHARE: f32 = 0.5;
/// Food each parent gives to a newborn (Sugarscape rule S: parents endow the child).
pub const BIRTH_GIFT: f32 = 3.0;

/// Biological sex (pairing requires one of each).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    /// Female.
    Female,
    /// Male.
    Male,
}

/// Behavioural preferences in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preferences {
    /// Risk tolerance: theft, fights, poaching in foreign territory.
    pub risk: f32,
    /// Cooperation tendency: sharing, depositing in group stores, bonding.
    pub cooperation: f32,
    /// Trade propensity: seeking and accepting barter, specialising.
    pub trade: f32,
    /// Time preference (impatience): high values discount delayed rewards
    /// (farming, tools, buildings) and keep smaller food reserves.
    pub time: f32,
}

impl Preferences {
    /// Clamps every field to `[0, 1]` (NaN becomes 0.5).
    pub fn clamped(self) -> Self {
        let c = |x: f32| if x.is_nan() { 0.5 } else { x.clamp(0.0, 1.0) };
        Self { risk: c(self.risk), cooperation: c(self.cooperation), trade: c(self.trade), time: c(self.time) }
    }

    /// Child preferences: parental mean plus `N(0, sd)` noise per trait
    /// (cultural and genetic transmission lumped together). Draws 8 numbers.
    pub fn inherit(a: &Self, b: &Self, sd: f32, rng: &mut SimRng) -> Self {
        let mut mix = |x: f32, y: f32| 0.5 * (x + y) + rng.normal_with(0.0, f64::from(sd)) as f32;
        Self {
            risk: mix(a.risk, b.risk),
            cooperation: mix(a.cooperation, b.cooperation),
            trade: mix(a.trade, b.trade),
            time: mix(a.time, b.time),
        }
        .clamped()
    }

    /// Food an agent tries to keep in its inventory: `2 + 6·(1 − time)`.
    pub fn food_reserve(&self) -> f32 {
        2.0 + 6.0 * (1.0 - self.time)
    }
}

/// Affinity towards one known agent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    /// The other agent.
    pub other: AgentId,
    /// Affinity in `[-1, 1]`.
    pub affinity: f32,
}

/// Bounded memory of relationships, sorted by agent id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Relations {
    entries: Vec<Relationship>,
}

impl Relations {
    /// Known relationships in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.entries.iter()
    }

    /// Number of known agents.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no agent is known.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Affinity towards `other` (0 when unknown).
    pub fn affinity(&self, other: AgentId) -> f32 {
        self.entries.binary_search_by_key(&other, |r| r.other).map_or(0.0, |i| self.entries[i].affinity)
    }

    /// Adds `delta` to the affinity towards `other`, clamped to `[-1, 1]`.
    /// When `other` is unknown and the memory holds `capacity` entries, the
    /// weakest relationship (smallest `|affinity|`, ties to the higher id) is
    /// forgotten if the new one would be stronger; otherwise the change is
    /// dropped.
    pub fn adjust(&mut self, other: AgentId, delta: f32, capacity: usize) {
        if !delta.is_finite() || capacity == 0 {
            return;
        }
        match self.entries.binary_search_by_key(&other, |r| r.other) {
            Ok(i) => self.entries[i].affinity = (self.entries[i].affinity + delta).clamp(-1.0, 1.0),
            Err(pos) => {
                let affinity = delta.clamp(-1.0, 1.0);
                if self.entries.len() < capacity {
                    self.entries.insert(pos, Relationship { other, affinity });
                    return;
                }
                let Some(weakest) = self.weakest() else { return };
                if self.entries[weakest].affinity.abs() < affinity.abs() {
                    self.entries.remove(weakest);
                    let pos = self.entries.binary_search_by_key(&other, |r| r.other).unwrap_or_else(|p| p);
                    self.entries.insert(pos, Relationship { other, affinity });
                }
            }
        }
    }

    fn weakest(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, r) in self.entries.iter().enumerate() {
            let better = best.is_none_or(|b| {
                let current = self.entries[b].affinity.abs();
                r.affinity.abs() < current || (r.affinity.abs() == current && r.other > self.entries[b].other)
            });
            if better {
                best = Some(i);
            }
        }
        best
    }

    /// Multiplies every affinity by `factor` and forgets near-zero entries
    /// and agents for which `alive` is false.
    pub fn decay(&mut self, factor: f32, alive: impl Fn(AgentId) -> bool) {
        for r in &mut self.entries {
            r.affinity *= factor;
        }
        self.entries.retain(|r| r.affinity.abs() >= 0.005 && alive(r.other));
    }

    /// Keeps only the `capacity` strongest relationships.
    pub fn truncate(&mut self, capacity: usize) {
        while self.entries.len() > capacity {
            match self.weakest() {
                Some(i) => {
                    self.entries.remove(i);
                }
                None => break,
            }
        }
    }
}

/// Places an agent remembers beyond its vision (most recent sighting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// A shore tile to drink at.
    pub water: Option<u32>,
    /// A tile with wild food (berries or fish).
    pub food: Option<u32>,
    /// A forest tile.
    pub wood: Option<u32>,
    /// A hills tile.
    pub stone: Option<u32>,
    /// Where the agent last traded successfully (an emergent market place).
    pub market: Option<u32>,
}

impl Memory {
    /// Fills places this memory lacks from `other` (knowledge shared while
    /// talking or trading).
    pub fn learn_from(&mut self, other: &Memory) {
        self.water = self.water.or(other.water);
        self.food = self.food.or(other.food);
        self.wood = self.wood.or(other.wood);
        self.stone = self.stone.or(other.stone);
        self.market = self.market.or(other.market);
    }

    /// Every remembered tile.
    pub fn tiles(&self) -> impl Iterator<Item = u32> {
        [self.water, self.food, self.wood, self.stone, self.market].into_iter().flatten()
    }
}

/// Homeostatic needs. Food is represented by [`Agent::energy`] (hunger is
/// `1 − energy`); the others are deprivation levels in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Needs {
    /// Thirst.
    pub water: Drive,
    /// Fatigue.
    pub rest: Drive,
    /// Loneliness.
    pub social: Drive,
    /// Perceived danger in `[0, 1]` (relaxes towards the local threat level).
    pub safety: f32,
}

impl Needs {
    /// Satisfied needs with the given growth rates.
    pub fn new(thirst_rate: f32, fatigue_rate: f32, social_rate: f32) -> Self {
        let drive = |rate: f32| Drive { value: 0.0, rate, threshold: 0.5 };
        Self { water: drive(thirst_rate), rest: drive(fatigue_rate), social: drive(social_rate), safety: 0.0 }
    }
}

/// What an agent is doing; shown by renderers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// No goal.
    #[default]
    Idle,
    /// Eating from the inventory or a group store.
    Eat,
    /// Drinking at a shore.
    Drink,
    /// Resting (at home if possible).
    Rest,
    /// Foraging berries or fishing.
    GatherFood,
    /// Logging.
    GatherWood,
    /// Quarrying.
    GatherStone,
    /// Planting or harvesting a farm plot.
    Farm,
    /// Making tools.
    Craft,
    /// Constructing a building.
    Build,
    /// Bartering with another agent.
    Trade,
    /// Spending time with another agent.
    Socialize,
    /// Giving food to a hungry group member or relative.
    Share,
    /// Depositing food in a group store.
    Store,
    /// Trying to take food from another agent.
    Steal,
    /// Seeking a partner to have a child.
    Mate,
    /// Wandering to discover resources.
    Explore,
}

impl Activity {
    /// All activities in display order.
    pub const ALL: [Activity; 17] = [
        Self::Idle,
        Self::Eat,
        Self::Drink,
        Self::Rest,
        Self::GatherFood,
        Self::GatherWood,
        Self::GatherStone,
        Self::Farm,
        Self::Craft,
        Self::Build,
        Self::Trade,
        Self::Socialize,
        Self::Share,
        Self::Store,
        Self::Steal,
        Self::Mate,
        Self::Explore,
    ];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Eat => "eating",
            Self::Drink => "drinking",
            Self::Rest => "resting",
            Self::GatherFood => "foraging",
            Self::GatherWood => "logging",
            Self::GatherStone => "quarrying",
            Self::Farm => "farming",
            Self::Craft => "crafting",
            Self::Build => "building",
            Self::Trade => "trading",
            Self::Socialize => "socialising",
            Self::Share => "sharing",
            Self::Store => "storing",
            Self::Steal => "stealing",
            Self::Mate => "courting",
            Self::Explore => "exploring",
        }
    }
}

/// Object of a goal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Performed in place.
    #[default]
    None,
    /// A tile (resource, shore, building site, exploration point).
    Tile(u32),
    /// Another agent.
    Agent(AgentId),
    /// A building.
    Building(BuildingId),
}

/// Current goal chosen by the utility AI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// What to do.
    pub activity: Activity,
    /// Where or with whom.
    pub target: Target,
    /// Building to place, for a [`Activity::Build`] goal on a new site.
    pub plan: Option<BuildingKind>,
}

/// Why an agent died.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    /// Energy exhausted.
    Starvation,
    /// Thirst saturated.
    Thirst,
    /// Injuries from fights.
    Conflict,
    /// Maximum age reached.
    Age,
}

impl DeathCause {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Starvation => "starvation",
            Self::Thirst => "thirst",
            Self::Conflict => "conflict",
            Self::Age => "old age",
        }
    }
}

/// One person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// Position (tile units), velocity and heading.
    pub kin: Kinematics,
    /// Sex.
    pub sex: Sex,
    /// Age in ticks.
    pub age: u64,
    /// Age at which the agent dies of old age.
    pub max_age: u64,
    /// Metabolic reserve in `[0, 1]`; hunger is `1 − energy`.
    pub energy: f32,
    /// Health in `[0, 1]`; the agent dies at 0.
    pub health: f32,
    /// Most recent cause of health loss.
    pub last_damage: Option<DeathCause>,
    /// Needs other than food.
    pub needs: Needs,
    /// Skill levels.
    pub skills: Skills,
    /// Exponential moving average, over production ticks, of the share of
    /// each kind of production ([`Work`], used for the specialisation index).
    pub practice: [f32; Work::COUNT],
    /// Carried goods.
    pub inventory: Bundle,
    /// Preferences.
    pub prefs: Preferences,
    /// Known agents and affinities.
    pub relations: Relations,
    /// Current group.
    pub group: Option<GroupId>,
    /// Hut the agent lives in.
    pub home: Option<BuildingId>,
    /// Farm plot the agent owns.
    pub farm: Option<BuildingId>,
    /// Whether the agent's group already has a store (refreshed daily).
    pub group_storage: bool,
    /// Index of the scenario population the agent descends from.
    pub population: u8,
    /// Parents (none for founders).
    pub parents: [Option<AgentId>; 2],
    /// Tick of the last birth the agent took part in.
    pub last_birth: Option<u64>,
    /// Generation (founders are 0).
    pub generation: u32,
    /// Current goal.
    pub goal: Goal,
    /// Remaining waypoints (tile indices), next one last.
    pub path: Vec<u32>,
    /// Tick at which the goal is reconsidered at the latest.
    pub next_decision: u64,
    /// Remembered places.
    pub memory: Memory,
    /// Learned demand: stock of each good the agent keeps for selling, raised
    /// by the amounts others bought from it and fading by 3 % a day.
    pub sales: Bundle,
    /// Activity performed during the last tick (for rendering).
    pub doing: Activity,
}

impl Agent {
    /// New agent at `position` with satisfied needs.
    pub fn new(position: Vec2, sex: Sex, age: u64, max_age: u64, prefs: Preferences, needs: Needs) -> Self {
        Self {
            kin: Kinematics::new(position, 0.0),
            sex,
            age,
            max_age,
            energy: 0.8,
            health: 1.0,
            last_damage: None,
            needs,
            skills: Skills::default(),
            practice: [0.0; Work::COUNT],
            inventory: Bundle::ZERO,
            prefs: prefs.clamped(),
            relations: Relations::default(),
            group: None,
            home: None,
            farm: None,
            group_storage: false,
            population: 0,
            parents: [None, None],
            last_birth: None,
            generation: 0,
            goal: Goal::default(),
            path: Vec::new(),
            next_decision: 0,
            memory: Memory::default(),
            sales: Bundle::ZERO,
            doing: Activity::Idle,
        }
    }

    /// Whether the agent is an adult.
    pub fn is_adult(&self) -> bool {
        self.age >= ADULT_AGE
    }

    /// Whether the agent can have children (adult and not too old).
    pub fn is_fertile(&self) -> bool {
        self.is_adult() && self.age < FERTILE_UNTIL
    }

    /// Whether the agent may have a child now: births enabled, fertile, well
    /// fed (energy ≥ 0.6), able to endow the child with [`BIRTH_GIFT`] food,
    /// and at least [`BIRTH_INTERVAL`] ticks since its last birth.
    pub fn can_reproduce(&self, tick: u64, rules: &Rules) -> bool {
        rules.births_enabled
            && self.is_fertile()
            && self.energy >= 0.6
            && self.inventory.get(Good::Food) >= BIRTH_GIFT
            && self.last_birth.is_none_or(|t| tick >= t + BIRTH_INTERVAL)
    }

    /// Stocks the agent aims to carry for sale: its learned demand
    /// ([`Agent::sales`]), with the bulk goods (food, wood, stone) scaled down
    /// together so that they fill at most [`SALE_CAPACITY_SHARE`] of the
    /// carrying capacity (tools are not bulk).
    pub fn sale_targets(&self) -> Bundle {
        let bulk = self.sales.get(Good::Food) + self.sales.get(Good::Wood) + self.sales.get(Good::Stone);
        let room = SALE_CAPACITY_SHARE * CARRY_CAPACITY;
        let scale = if bulk > room { room / bulk } else { 1.0 };
        let mut targets = self.sales;
        for g in [Good::Food, Good::Wood, Good::Stone] {
            targets.set(g, self.sales.get(g) * scale);
        }
        targets
    }

    /// Food the agent keeps on itself rather than depositing in a store: its
    /// reserve or, if larger, the food it aims to carry for sale.
    pub fn food_to_carry(&self) -> f32 {
        self.prefs.food_reserve().max(self.sale_targets().get(Good::Food))
    }

    /// Hunger `1 − energy`.
    pub fn hunger(&self) -> f32 {
        1.0 - self.energy
    }

    /// Units of food, wood and stone carried (tools are worn, not carried in bulk).
    pub fn load(&self) -> f32 {
        self.inventory.get(Good::Food) + self.inventory.get(Good::Wood) + self.inventory.get(Good::Stone)
    }

    /// Free carrying capacity.
    pub fn free_capacity(&self) -> f32 {
        (CARRY_CAPACITY - self.load()).max(0.0)
    }

    /// Work output multiplier for `skill`:
    /// `(0.5 + skill) · (1 + tool_bonus if tools are held) · (0.5 for children) · (0.6 + 0.4·health)`.
    pub fn productivity(&self, skill: Skill, tool_bonus: f32) -> f32 {
        let tools = if self.inventory.get(Good::Tools) > 0.0 { 1.0 + tool_bonus } else { 1.0 };
        let maturity = if self.is_adult() { 1.0 } else { 0.5 };
        (0.5 + self.skills.get(skill)) * tools * maturity * (0.6 + 0.4 * self.health)
    }

    /// Wealth for inequality statistics: food + wood + stone + 2 × tools
    /// (a tool embodies one wood and one stone).
    pub fn wealth(&self) -> f32 {
        self.inventory.get(Good::Food)
            + self.inventory.get(Good::Wood)
            + self.inventory.get(Good::Stone)
            + 2.0 * self.inventory.get(Good::Tools)
    }

    /// Whether the agent may deposit food in and eat from `building`: a
    /// finished store of its own group, or its own finished hut (the
    /// household store shared by everyone living there).
    pub fn may_use_store(&self, building: &Building) -> bool {
        building.is_complete()
            && match building.kind {
                BuildingKind::Storage => building.group.is_some() && building.group == self.group,
                BuildingKind::Hut => self.home == Some(building.id),
                BuildingKind::FarmPlot => false,
            }
    }

    /// Building the agent currently wants to build, by need.
    pub fn building_plan(&self) -> Option<BuildingKind> {
        if !self.is_adult() {
            None
        } else if self.home.is_none() {
            Some(BuildingKind::Hut)
        } else if self.farm.is_none() && self.prefs.time < 0.6 {
            Some(BuildingKind::FarmPlot)
        } else if self.group.is_some() && !self.group_storage && self.prefs.cooperation > 0.55 {
            Some(BuildingKind::Storage)
        } else {
            None
        }
    }

    /// Cobb–Douglas demand weights `m_g` used for welfare and trade: food has
    /// a constant metabolic weight; materials are wanted for the current
    /// building plan and for tools; tools are wanted when worn out.
    pub fn demand_weights(&self) -> Bundle {
        let mut m = Bundle::new(1.0, 0.15, 0.1, 0.1);
        let tools_low = self.inventory.get(Good::Tools) < 0.5;
        if tools_low {
            m.add(Good::Tools, 0.3);
            m.add(Good::Wood, 0.1);
            m.add(Good::Stone, 0.1);
        }
        if let Some(plan) = self.building_plan() {
            let cost = plan.cost();
            let urgency = if plan == BuildingKind::Hut { 0.08 } else { 0.05 };
            m.add(Good::Wood, urgency * cost.get(Good::Wood));
            m.add(Good::Stone, urgency * cost.get(Good::Stone));
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_improve_with_practice_and_decay_without() {
        let mut s = Skills::default();
        for _ in 0..200 {
            s.practice(Some(Skill::Farming), 0.01, 0.0005);
        }
        let farming = s.get(Skill::Farming);
        assert!((farming - (1.0 - 0.99f32.powi(200))).abs() < 1e-3, "{farming}");
        for _ in 0..200 {
            s.practice(Some(Skill::Trading), 0.01, 0.0005);
        }
        assert!(s.get(Skill::Farming) < farming, "unpractised skill decays");
        assert!((s.get(Skill::Farming) - farming * 0.9995f32.powi(200)).abs() < 1e-3);
    }

    #[test]
    fn relations_are_bounded_and_keep_the_strongest() {
        let mut r = Relations::default();
        for i in 1..=5 {
            r.adjust(AgentId(i), 0.1 * i as f32, 3);
        }
        // Capacity 3: the strongest three (3, 4, 5) remain.
        let kept: Vec<u64> = r.iter().map(|e| e.other.0).collect();
        assert_eq!(kept, vec![3, 4, 5]);
        r.adjust(AgentId(4), 2.0, 3);
        assert_eq!(r.affinity(AgentId(4)), 1.0, "clamped");
        r.adjust(AgentId(9), -0.05, 3);
        assert_eq!(r.affinity(AgentId(9)), 0.0, "weaker newcomer is not remembered");
        r.decay(0.5, |id| id != AgentId(5));
        assert_eq!(r.len(), 2);
        assert!((r.affinity(AgentId(3)) - 0.15).abs() < 1e-6);
        r.truncate(1);
        assert_eq!(r.iter().next().map(|e| e.other), Some(AgentId(4)));
    }

    #[test]
    fn inheritance_averages_parents() {
        let a = Preferences { risk: 0.0, cooperation: 1.0, trade: 0.2, time: 0.4 };
        let b = Preferences { risk: 1.0, cooperation: 1.0, trade: 0.6, time: 0.8 };
        let mut rng = SimRng::seed_from_u64(1);
        let child = Preferences::inherit(&a, &b, 0.0, &mut rng);
        assert_eq!(child, Preferences { risk: 0.5, cooperation: 1.0, trade: 0.4, time: 0.6 });
    }

    #[test]
    fn plans_drive_material_demand() {
        let prefs = Preferences { risk: 0.5, cooperation: 0.5, trade: 0.5, time: 0.3 };
        let mut a = Agent::new(Vec2::ZERO, Sex::Female, ADULT_AGE, ADULT_AGE * 4, prefs, Needs::new(0.01, 0.01, 0.01));
        assert_eq!(a.building_plan(), Some(BuildingKind::Hut));
        let homeless = a.demand_weights();
        a.home = Some(BuildingId(1));
        a.farm = Some(BuildingId(2));
        a.inventory.set(Good::Tools, 1.0);
        let settled = a.demand_weights();
        assert!(homeless.get(Good::Wood) > settled.get(Good::Wood));
        assert_eq!(a.building_plan(), None, "settled, ungrouped agents have nothing to build");
        a.age = 0;
        assert!(a.productivity(Skill::Gathering, 0.8) < 1.0, "children work at half rate");
    }
}
