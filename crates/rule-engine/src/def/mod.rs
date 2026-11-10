//! Serializable, versioned definitions of a custom life system.
//!
//! A [`LifeSystem`] is plain data: entity types, relationships, DSL rules,
//! Rhai script rules, the world and end conditions. It reads and writes RON
//! and JSON ([`LifeSystem::from_ron`], [`LifeSystem::to_ron`], ...). Rules are
//! stored as DSL text — one string per rule, in canonical pretty-printed form
//! — so files stay readable and diff-friendly; they are parsed on load.
//! Semantic validation happens in [`crate::compile::compile`].

pub mod entity;
pub mod relationship;
pub mod world;

use serde::{Deserialize, Serialize};

pub use entity::{
    CarriedResource, Distribution, EnergyDef, EntityType, InitialPopulation, LifecycleDef, MovementDef, PropertyDef,
    ReproductionDef, ReproductionMode,
};
pub use relationship::{RelationKind, Relationship, TransformTrigger};
pub use world::{
    Axis, EndCondition, EnvPattern, EnvironmentField, EventEffect, FieldInit, RandomEvent, ResourceField, Seasonality,
    WorldDef,
};

use crate::dsl::ast::{Event, RuleAst};
use crate::error::RuleEngineError;

/// Newest definition format version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;

fn format_version() -> u32 {
    FORMAT_VERSION
}

fn enabled() -> bool {
    true
}

/// A rule written in Rhai, run in the sandbox (see [`crate::script`]).
///
/// The script must define `fn run(me)`; `me` is the acting entity. Action
/// functions (`move_towards`, `eat`, `emit`...) only *record intents* that
/// the engine resolves exactly like DSL actions. Top-level statements are
/// never executed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptRule {
    /// Name shown in the rule list and in error reports (unique per type).
    pub name: String,
    /// Entity type the script runs for.
    pub entity: String,
    /// When it runs (default: every tick).
    #[serde(default)]
    pub event: Event,
    /// Order among the type's scripts (higher first). Scripts always run
    /// after all DSL rules of the entity.
    #[serde(default)]
    pub priority: i32,
    /// Rhai source.
    pub source: String,
    /// Disabled scripts are compiled but not run.
    #[serde(default = "enabled")]
    pub enabled: bool,
}

/// Sandbox limits for Rhai scripts (per call).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptLimits {
    /// Operation budget per call (the only budget that decides outcomes).
    pub max_operations: u64,
    /// Maximum function-call nesting.
    pub max_call_levels: usize,
    /// Maximum expression nesting (also inside functions).
    pub max_expr_depth: usize,
    /// Maximum string length in bytes.
    pub max_string_size: usize,
    /// Maximum array length.
    pub max_array_size: usize,
    /// Maximum number of map entries.
    pub max_map_size: usize,
    /// Maximum number of live variables.
    pub max_variables: usize,
    /// Maximum number of script-defined functions.
    pub max_functions: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_operations: 20_000,
            max_call_levels: 16,
            max_expr_depth: 64,
            max_string_size: 1024,
            max_array_size: 1024,
            max_map_size: 256,
            max_variables: 128,
            max_functions: 64,
        }
    }
}

impl ScriptLimits {
    /// Hard upper bounds accepted for each limit.
    pub const CEILING: ScriptLimits = ScriptLimits {
        max_operations: 1_000_000,
        max_call_levels: 64,
        max_expr_depth: 128,
        max_string_size: 65_536,
        max_array_size: 65_536,
        max_map_size: 4096,
        max_variables: 1024,
        max_functions: 256,
    };
}

/// A complete custom life system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LifeSystem {
    /// Format version of the document.
    #[serde(default = "format_version")]
    pub format: u32,
    /// Display name.
    pub name: String,
    /// Free text.
    #[serde(default)]
    pub description: String,
    /// Entity types (order defines the type index used in views).
    pub entity_types: Vec<EntityType>,
    /// Relationship graph; compiled into rules.
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    /// DSL rules, stored as text.
    #[serde(default, with = "rule_text")]
    pub rules: Vec<RuleAst>,
    /// Sandboxed Rhai rules.
    #[serde(default)]
    pub scripts: Vec<ScriptRule>,
    /// The world.
    pub world: WorldDef,
    /// End conditions.
    #[serde(default)]
    pub end_conditions: Vec<EndCondition>,
    /// Script sandbox limits.
    #[serde(default)]
    pub script_limits: ScriptLimits,
}

impl LifeSystem {
    /// Empty system with a world of the given size.
    pub fn new(name: &str, width: u32, height: u32) -> Self {
        Self {
            format: FORMAT_VERSION,
            name: name.to_string(),
            description: String::new(),
            entity_types: Vec::new(),
            relationships: Vec::new(),
            rules: Vec::new(),
            scripts: Vec::new(),
            world: WorldDef::new(width, height),
            end_conditions: Vec::new(),
            script_limits: ScriptLimits::default(),
        }
    }

    fn check_format(self) -> Result<Self, RuleEngineError> {
        if self.format > FORMAT_VERSION || self.format == 0 {
            return Err(RuleEngineError::UnsupportedFormat { found: self.format, supported: FORMAT_VERSION });
        }
        Ok(self)
    }

    /// Reads a RON document (syntax only; call [`crate::compile::compile`]
    /// to validate the semantics).
    pub fn from_ron(text: &str) -> Result<Self, RuleEngineError> {
        ron::from_str::<Self>(text).map_err(|e| RuleEngineError::Ron(e.to_string()))?.check_format()
    }

    /// Writes a pretty RON document.
    pub fn to_ron(&self) -> Result<String, RuleEngineError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| RuleEngineError::Ron(e.to_string()))
    }

    /// Reads a JSON document.
    pub fn from_json(text: &str) -> Result<Self, RuleEngineError> {
        serde_json::from_str::<Self>(text).map_err(|e| RuleEngineError::Json(e.to_string()))?.check_format()
    }

    /// Writes a pretty JSON document.
    pub fn to_json(&self) -> Result<String, RuleEngineError> {
        serde_json::to_string_pretty(self).map_err(|e| RuleEngineError::Json(e.to_string()))
    }

    /// Index of an entity type by name.
    pub fn type_index(&self, name: &str) -> Option<usize> {
        self.entity_types.iter().position(|t| t.name == name)
    }

    /// Index of a resource field by name.
    pub fn resource_index(&self, name: &str) -> Option<usize> {
        self.world.resources.iter().position(|r| r.name == name)
    }
}

/// Serde adapter storing each rule as its canonical DSL text.
mod rule_text {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::dsl::ast::RuleAst;
    use crate::dsl::{parse_rule, print_rule};
    use crate::error::render_diagnostics;

    pub fn serialize<S: Serializer>(rules: &[RuleAst], s: S) -> Result<S::Ok, S::Error> {
        rules.iter().map(print_rule).collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RuleAst>, D::Error> {
        let texts = Vec::<String>::deserialize(d)?;
        let mut rules = Vec::with_capacity(texts.len());
        for (i, text) in texts.iter().enumerate() {
            match parse_rule(text) {
                Ok(rule) => rules.push(rule),
                Err(diags) => {
                    return Err(D::Error::custom(format!(
                        "rule #{} (positions are relative to that rule's text): {}",
                        i + 1,
                        render_diagnostics(&diags)
                    )));
                }
            }
        }
        Ok(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::parse_rule;

    fn sample() -> LifeSystem {
        let mut sys = LifeSystem::new("demo", 32, 24);
        let mut grass = EntityType::new("grass");
        grass.initial.count = 10;
        let mut rabbit = EntityType::new("rabbit");
        rabbit.movement = MovementDef::RandomWalk { speed: 0.5 };
        rabbit.states = vec!["young".into(), "adult".into()];
        sys.entity_types = vec![grass, rabbit];
        sys.relationships.push(Relationship {
            subject: "rabbit".into(),
            kind: RelationKind::Consumes { efficiency: 0.5, hunger: Some(40.0), radius: 4.0, bite: 1.0, handling: 0 },
            object: "grass".into(),
            priority_shift: 0,
        });
        sys.rules.push(parse_rule("rule grow for rabbit: when age > 10 then set_state(adult) once").unwrap());
        sys.world.resources.push(ResourceField::new("water"));
        sys.end_conditions.push(EndCondition::Extinction("rabbit".into()));
        sys
    }

    #[test]
    fn ron_and_json_round_trip() {
        let sys = sample();
        let ron_text = sys.to_ron().unwrap();
        assert!(ron_text.contains("rule grow for rabbit"), "rules are stored as text:\n{ron_text}");
        assert_eq!(LifeSystem::from_ron(&ron_text).unwrap(), sys);
        let json = sys.to_json().unwrap();
        assert_eq!(LifeSystem::from_json(&json).unwrap(), sys);
    }

    #[test]
    fn rejects_future_format_and_bad_rule_text() {
        let mut sys = sample();
        sys.format = FORMAT_VERSION + 1;
        let err = LifeSystem::from_ron(&sys.to_ron().unwrap()).unwrap_err();
        assert!(matches!(err, RuleEngineError::UnsupportedFormat { .. }));
        let bad = r#"(name: "x", entity_types: [], world: (width: 4, height: 4), rules: ["rule a for b: then"])"#;
        let err = LifeSystem::from_ron(bad).unwrap_err().to_string();
        assert!(err.contains("rule #1") && err.contains("E015"), "{err}");
    }

    #[test]
    fn minimal_document_uses_defaults() {
        let sys = LifeSystem::from_ron(
            r#"(name: "min", entity_types: [(name: "dot", movement: RandomWalk(speed: 1.0))], world: (width: 8, height: 8))"#,
        )
        .unwrap();
        assert_eq!(sys.format, FORMAT_VERSION);
        assert_eq!(sys.world.topology, simulation_core::Topology::Torus);
        assert_eq!(sys.entity_types[0].energy.initial, 10.0);
        assert_eq!(sys.entity_types[0].state_names(), vec!["default".to_string()]);
    }
}
