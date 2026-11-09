//! Scenario description and runtime rule parameters.

use serde::{Deserialize, Serialize};
use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError, SimRng};

use crate::agent::Preferences;
use crate::error::CivError;
use crate::terrain::{Resource, TerrainSpec};

/// Normal distribution of a preference, truncated to `[0, 1]` by clamping.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dist {
    /// Mean.
    pub mean: f32,
    /// Standard deviation (0 gives identical agents).
    pub sd: f32,
}

impl Dist {
    /// Draws one value (two random numbers, Box–Muller).
    pub fn sample(&self, rng: &mut SimRng) -> f32 {
        (rng.normal_with(f64::from(self.mean), f64::from(self.sd)) as f32).clamp(0.0, 1.0)
    }

    fn validate(&self, name: &str) -> Result<(), CivError> {
        if !(0.0..=1.0).contains(&self.mean) || !(0.0..=1.0).contains(&self.sd) {
            return Err(CivError::InvalidScenario(format!(
                "preference `{name}` needs mean and sd in [0, 1] (got {} and {})",
                self.mean, self.sd
            )));
        }
        Ok(())
    }
}

/// Distributions of the four preferences of a population.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreferenceDist {
    /// Risk tolerance.
    pub risk: Dist,
    /// Cooperation tendency.
    pub cooperation: Dist,
    /// Trade propensity.
    pub trade: Dist,
    /// Time preference (impatience).
    pub time: Dist,
}

impl PreferenceDist {
    /// Draws one agent's preferences (eight random numbers).
    pub fn sample(&self, rng: &mut SimRng) -> Preferences {
        Preferences {
            risk: self.risk.sample(rng),
            cooperation: self.cooperation.sample(rng),
            trade: self.trade.sample(rng),
            time: self.time.sample(rng),
        }
    }
}

/// Rectangle given as fractions of the world size (`0..=1`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

impl Region {
    /// The whole world.
    pub const WHOLE: Region = Region { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };

    fn validate(&self) -> Result<(), CivError> {
        let ok = [self.x0, self.y0, self.x1, self.y1].iter().all(|v| (0.0..=1.0).contains(v))
            && self.x0 < self.x1
            && self.y0 < self.y1;
        if ok {
            Ok(())
        } else {
            Err(CivError::InvalidScenario(format!(
                "region ({}, {})–({}, {}) must be an increasing rectangle inside [0, 1]",
                self.x0, self.y0, self.x1, self.y1
            )))
        }
    }
}

fn default_starting_food() -> f32 {
    6.0
}

/// One founding population.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PopulationSpec {
    /// Display name ("Farmers", "Raiders"...).
    pub label: String,
    /// Number of founders.
    pub count: u32,
    /// Preference distributions.
    pub preferences: PreferenceDist,
    /// Where founders are placed (whole world if omitted).
    #[serde(default)]
    pub region: Option<Region>,
    /// Initial mutual affinity among founders of this population (a founding
    /// band that already knows each other); 0 for strangers.
    #[serde(default)]
    pub kin_affinity: f32,
    /// Food each founder carries.
    #[serde(default = "default_starting_food")]
    pub starting_food: f32,
}

fn default_richness() -> f32 {
    1.0
}

/// World description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSpec {
    /// Terrain source.
    pub terrain: TerrainSpec,
    /// Multiplier of resource capacities and growback rates.
    #[serde(default = "default_richness")]
    pub richness: f32,
}

/// A complete scenario file: header, world, founding populations and rule
/// parameter overrides (keys of [`Rules::param_specs`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario demonstrates.
    pub description: String,
    /// Seed of terrain generation, placement and behaviour.
    pub seed: u64,
    /// World.
    pub world: WorldSpec,
    /// Founding populations (at least one; counts may be zero).
    pub populations: Vec<PopulationSpec>,
    /// Rule parameter overrides.
    #[serde(default)]
    pub params: ParamSet,
}

impl Scenario {
    /// Validates everything that can be checked without running.
    pub fn validate(&self) -> Result<(), CivError> {
        let bad = |m: String| Err(CivError::InvalidScenario(m));
        if self.id.trim().is_empty() {
            return bad("the scenario id is empty".into());
        }
        if i64::try_from(self.seed).is_err() {
            return bad(format!("seed {} is larger than {}", self.seed, i64::MAX));
        }
        self.world.terrain.validate()?;
        if !(0.05..=10.0).contains(&self.world.richness) {
            return bad(format!("richness {} must be in [0.05, 10]", self.world.richness));
        }
        if self.populations.is_empty() {
            return bad("at least one population is required".into());
        }
        if self.populations.len() > usize::from(u8::MAX) {
            return bad(format!("{} populations given; at most 255 are supported", self.populations.len()));
        }
        let total: u64 = self.populations.iter().map(|p| u64::from(p.count)).sum();
        if total > 5000 {
            return bad(format!("{total} founders requested; at most 5000 are supported"));
        }
        for p in &self.populations {
            let d = &p.preferences;
            for (name, dist) in [("risk", d.risk), ("cooperation", d.cooperation), ("trade", d.trade), ("time", d.time)]
            {
                dist.validate(name)?;
            }
            if let Some(region) = p.region {
                region.validate()?;
            }
            if !(-1.0..=1.0).contains(&p.kin_affinity) {
                return bad(format!("kin_affinity {} of `{}` must be in [-1, 1]", p.kin_affinity, p.label));
            }
            if !(0.0..=24.0).contains(&p.starting_food) {
                return bad(format!("starting_food {} of `{}` must be in [0, 24]", p.starting_food, p.label));
            }
        }
        Rules::from_params(&self.params)?;
        Ok(())
    }
}

/// Rule parameters that can change while the simulation runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// Growback multipliers indexed by [`Resource::index`].
    pub regen: [f32; 5],
    /// Energy burned per tick at rest.
    pub metabolism: f32,
    /// Thirst growth per tick.
    pub thirst_rate: f32,
    /// Fatigue growth per tick.
    pub fatigue_rate: f32,
    /// Perception radius in tiles.
    pub vision: u32,
    /// Whether barter happens.
    pub trade_enabled: bool,
    /// Whether theft and fights happen.
    pub conflict_enabled: bool,
    /// Whether children are born.
    pub births_enabled: bool,
    /// Productivity bonus of carrying tools.
    pub tool_bonus: f32,
    /// Skill learning rate λ per tick of practice.
    pub learning_rate: f32,
    /// Skill decay rate φ per tick without practice.
    pub forgetting_rate: f32,
    /// Time constant of road (trail) decay in ticks.
    pub road_decay_ticks: f32,
    /// Maximum movement-cost reduction of a saturated road.
    pub road_discount: f32,
    /// Trail intensity at which a tile counts as a road.
    pub road_threshold: f32,
    /// Mutual affinity needed to bond into a group.
    pub bond_threshold: f32,
    /// Maximum number of remembered relationships per agent.
    pub max_relationships: u32,
    /// Territorial influence lost per tile of distance.
    pub territory_falloff: f32,
    /// Births stop at this population.
    pub max_population: u32,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            regen: [1.0; 5],
            metabolism: 0.006,
            thirst_rate: 0.012,
            fatigue_rate: 0.008,
            vision: 10,
            trade_enabled: true,
            conflict_enabled: true,
            births_enabled: true,
            tool_bonus: 0.8,
            learning_rate: 0.01,
            forgetting_rate: 0.0005,
            road_decay_ticks: 600.0,
            road_discount: 0.6,
            road_threshold: 0.2,
            bond_threshold: 0.3,
            max_relationships: 12,
            territory_falloff: 1.0,
            max_population: 600,
        }
    }
}

const REGEN_KEYS: [&str; 5] = ["regen_berries", "regen_fish", "regen_wood", "regen_stone", "crop_growth"];

impl Rules {
    /// Parameter declarations (all apply at the next tick).
    pub fn param_specs() -> Vec<ParamSpec> {
        let d = Rules::default();
        let mut specs: Vec<ParamSpec> = Resource::ALL
            .iter()
            .zip(REGEN_KEYS)
            .map(|(r, key)| {
                ParamSpec::float(key, &format!("{} regeneration", r.label()), 0.0, 5.0, 1.0)
                    .describe(&format!("Multiplier of the {} growback rate.", r.label().to_lowercase()))
                    .group("Regeneration")
            })
            .collect();
        let float = |key: &str, label: &str, min: f64, max: f64, default: f32, group: &str, text: &str| {
            ParamSpec::float(key, label, min, max, f64::from(default)).describe(text).group(group)
        };
        specs.extend([
            float(
                "metabolism",
                "Metabolism",
                0.001,
                0.05,
                d.metabolism,
                "Needs",
                "Energy burned per tick at rest (work burns 60 % more).",
            ),
            float("thirst_rate", "Thirst rate", 0.001, 0.05, d.thirst_rate, "Needs", "Thirst gained per tick."),
            float(
                "fatigue_rate",
                "Fatigue rate",
                0.001,
                0.05,
                d.fatigue_rate,
                "Needs",
                "Fatigue gained per tick of activity.",
            ),
            ParamSpec::int("vision", "Vision", 2, 32, i64::from(d.vision))
                .describe("Perception radius in tiles.")
                .group("Needs"),
            ParamSpec::boolean("trade_enabled", "Barter", d.trade_enabled)
                .describe("Agents trade goods at MRS-based prices when both gain.")
                .group("Economy"),
            float(
                "tool_bonus",
                "Tool bonus",
                0.0,
                3.0,
                d.tool_bonus,
                "Economy",
                "Productivity bonus while carrying tools.",
            ),
            float(
                "learning_rate",
                "Learning rate",
                0.0,
                0.1,
                d.learning_rate,
                "Economy",
                "Skill gain per tick of practice: s += rate·(1 − s).",
            ),
            float(
                "forgetting_rate",
                "Forgetting rate",
                0.0,
                0.01,
                d.forgetting_rate,
                "Economy",
                "Skill loss per tick without practice: s −= rate·s.",
            ),
            float(
                "road_decay_ticks",
                "Trail decay time",
                10.0,
                20000.0,
                d.road_decay_ticks,
                "Roads",
                "Time constant of trail fading in ticks.",
            ),
            float(
                "road_discount",
                "Road discount",
                0.0,
                0.9,
                d.road_discount,
                "Roads",
                "Movement-cost reduction on a saturated road.",
            ),
            float(
                "road_threshold",
                "Road threshold",
                0.01,
                1.0,
                d.road_threshold,
                "Roads",
                "Trail intensity counted as a road in metrics and views.",
            ),
            ParamSpec::boolean("conflict_enabled", "Theft and fights", d.conflict_enabled)
                .describe("Desperate or aggressive agents steal and defend territory.")
                .group("Society"),
            ParamSpec::boolean("births_enabled", "Births", d.births_enabled)
                .describe("Well-fed adult pairs have children.")
                .group("Society"),
            float(
                "bond_threshold",
                "Bond threshold",
                0.05,
                1.0,
                d.bond_threshold,
                "Society",
                "Mutual affinity that links two agents into a group.",
            ),
            ParamSpec::int("max_relationships", "Relationship memory", 2, 64, i64::from(d.max_relationships))
                .describe("Relationships remembered per agent; weakest are forgotten.")
                .group("Society"),
            float(
                "territory_falloff",
                "Territory falloff",
                0.1,
                5.0,
                d.territory_falloff,
                "Society",
                "Influence lost per tile of distance from a building.",
            ),
            ParamSpec::int("max_population", "Population cap", 1, 5000, i64::from(d.max_population))
                .describe("No births at or above this population.")
                .group("Society"),
        ]);
        specs.into_iter().map(|s| s.apply(ApplyPolicy::NextTick)).collect()
    }

    /// Current value of a parameter.
    pub fn value(&self, key: &str) -> Option<ParamValue> {
        if let Some(i) = REGEN_KEYS.iter().position(|k| *k == key) {
            return Some(ParamValue::Float(f64::from(self.regen[i])));
        }
        let f = |x: f32| Some(ParamValue::Float(f64::from(x)));
        match key {
            "metabolism" => f(self.metabolism),
            "thirst_rate" => f(self.thirst_rate),
            "fatigue_rate" => f(self.fatigue_rate),
            "vision" => Some(ParamValue::Int(i64::from(self.vision))),
            "trade_enabled" => Some(ParamValue::Bool(self.trade_enabled)),
            "tool_bonus" => f(self.tool_bonus),
            "learning_rate" => f(self.learning_rate),
            "forgetting_rate" => f(self.forgetting_rate),
            "road_decay_ticks" => f(self.road_decay_ticks),
            "road_discount" => f(self.road_discount),
            "road_threshold" => f(self.road_threshold),
            "conflict_enabled" => Some(ParamValue::Bool(self.conflict_enabled)),
            "births_enabled" => Some(ParamValue::Bool(self.births_enabled)),
            "bond_threshold" => f(self.bond_threshold),
            "max_relationships" => Some(ParamValue::Int(i64::from(self.max_relationships))),
            "territory_falloff" => f(self.territory_falloff),
            "max_population" => Some(ParamValue::Int(i64::from(self.max_population))),
            _ => None,
        }
    }

    /// Validates `value` against the declared spec and stores it.
    pub fn set(&mut self, key: &str, value: &ParamValue) -> Result<ApplyPolicy, SimError> {
        let specs = Self::param_specs();
        let spec = specs.iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(value)?;
        let float = value.as_f64().map(|v| v as f32);
        let int = value.as_i64();
        let missing = || SimError::invalid_param(key, "value has the wrong type");
        if let Some(i) = REGEN_KEYS.iter().position(|k| *k == key) {
            self.regen[i] = float.ok_or_else(missing)?;
            return Ok(spec.apply);
        }
        match key {
            "metabolism" => self.metabolism = float.ok_or_else(missing)?,
            "thirst_rate" => self.thirst_rate = float.ok_or_else(missing)?,
            "fatigue_rate" => self.fatigue_rate = float.ok_or_else(missing)?,
            "vision" => self.vision = int.and_then(|v| u32::try_from(v).ok()).ok_or_else(missing)?,
            "trade_enabled" => self.trade_enabled = value.as_bool().ok_or_else(missing)?,
            "tool_bonus" => self.tool_bonus = float.ok_or_else(missing)?,
            "learning_rate" => self.learning_rate = float.ok_or_else(missing)?,
            "forgetting_rate" => self.forgetting_rate = float.ok_or_else(missing)?,
            "road_decay_ticks" => self.road_decay_ticks = float.ok_or_else(missing)?,
            "road_discount" => self.road_discount = float.ok_or_else(missing)?,
            "road_threshold" => self.road_threshold = float.ok_or_else(missing)?,
            "conflict_enabled" => self.conflict_enabled = value.as_bool().ok_or_else(missing)?,
            "births_enabled" => self.births_enabled = value.as_bool().ok_or_else(missing)?,
            "bond_threshold" => self.bond_threshold = float.ok_or_else(missing)?,
            "max_relationships" => {
                self.max_relationships = int.and_then(|v| u32::try_from(v).ok()).ok_or_else(missing)?;
            }
            "territory_falloff" => self.territory_falloff = float.ok_or_else(missing)?,
            "max_population" => self.max_population = int.and_then(|v| u32::try_from(v).ok()).ok_or_else(missing)?,
            _ => return Err(SimError::UnknownParameter(key.to_string())),
        }
        Ok(spec.apply)
    }

    /// All current values.
    pub fn values(&self) -> ParamSet {
        Self::param_specs().iter().filter_map(|s| self.value(&s.key).map(|v| (s.key.clone(), v))).collect()
    }

    /// Defaults overridden by `params` (every key validated).
    pub fn from_params(params: &ParamSet) -> Result<Rules, CivError> {
        let mut rules = Rules::default();
        for (key, value) in params.iter() {
            rules.set(key, value).map_err(|e| CivError::InvalidScenario(e.to_string()))?;
        }
        Ok(rules)
    }

    /// Growback multiplier of `resource`.
    pub fn regen_of(&self, resource: Resource) -> f32 {
        self.regen[resource.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_round_trips_through_rules() {
        let mut rules = Rules::default();
        for spec in Rules::param_specs() {
            assert_eq!(rules.value(&spec.key), Some(spec.default.clone()), "default of {}", spec.key);
            assert_eq!(rules.set(&spec.key, &spec.default), Ok(ApplyPolicy::NextTick));
        }
        assert_eq!(rules, Rules::default());
        assert_eq!(rules.values().len(), Rules::param_specs().len());
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut rules = Rules::default();
        assert!(matches!(rules.set("metabolism", &ParamValue::Float(1.0)), Err(SimError::InvalidParameter { .. })));
        assert!(matches!(rules.set("nope", &ParamValue::Float(1.0)), Err(SimError::UnknownParameter(_))));
        assert!(rules.set("vision", &ParamValue::Text("far".into())).is_err());
        rules.set("regen_wood", &ParamValue::Int(2)).unwrap();
        assert_eq!(rules.regen_of(Resource::Wood), 2.0);
        let overrides = ParamSet::new().with("metabolism", ParamValue::Float(0.5));
        assert!(Rules::from_params(&overrides).unwrap_err().to_string().contains("metabolism"));
    }

    #[test]
    fn dist_sampling_is_clamped() {
        let mut rng = SimRng::seed_from_u64(2);
        let d = Dist { mean: 0.95, sd: 0.5 };
        for _ in 0..500 {
            assert!((0.0..=1.0).contains(&d.sample(&mut rng)));
        }
    }
}
