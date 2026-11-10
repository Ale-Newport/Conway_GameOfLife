//! User interventions, definition edits and their apply policies.
//!
//! **Change-application policy.** Editing the definition of a running
//! system is classified by [`classify_edit`]:
//!
//! * [`ApplyPolicy::NextTick`] — rules, scripts, relationships, rule
//!   metadata, entity-type behaviour and appearance (colour, icon, sprite,
//!   movement, energy budget, reproduction, lifecycle, perception, property
//!   ranges), resource dynamics (regen, max, diffusion, decay,
//!   energy value, colour), environment fields, random events, end
//!   conditions, script limits and `max_entities`. Live entities keep their
//!   state; per-rule cooldowns restart; property values are clamped to new
//!   ranges; carried stock above a lowered capacity is released into the
//!   field.
//! * [`ApplyPolicy::RequiresReset`] — anything that changes the *shape* of
//!   the state: adding, removing, renaming or reordering entity types,
//!   their states, properties or carried resources; adding, removing or
//!   renaming resource fields; world width, height or topology. Such an
//!   edit is stored and becomes the system used by the next reset; the
//!   running world is left untouched.
//!
//! Initial populations and initial field contents only matter at reset;
//! changing them alongside next-tick edits is reported in the notes.

use serde::{Deserialize, Serialize};
use simulation_core::ApplyPolicy;

use crate::def::LifeSystem;

/// How [`CustomLifeCommand::PaintResource`] combines with the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaintMode {
    /// Add the amount (negative amounts remove, never below zero).
    #[default]
    Add,
    /// Set cells to the amount.
    Set,
}

/// Commands accepted by [`crate::CustomLifeSim`].
#[derive(Clone, Debug, PartialEq)]
pub enum CustomLifeCommand {
    /// Create an entity of a type at a position.
    Spawn {
        /// Entity type name.
        entity: String,
        /// x in world units.
        x: f32,
        /// y in world units.
        y: f32,
    },
    /// Remove an entity (its carried resources are dropped).
    Remove {
        /// Entity id.
        id: u64,
    },
    /// Paint a resource field with a disc brush.
    PaintResource {
        /// Resource name.
        resource: String,
        /// Centre x.
        x: f32,
        /// Centre y.
        y: f32,
        /// Brush radius in cells.
        radius: f32,
        /// Amount per cell.
        amount: f32,
        /// Add or set.
        mode: PaintMode,
    },
    /// Replace the definition (see the module documentation for when it applies).
    ReplaceSystem(Box<LifeSystem>),
    /// Whether reaching an end condition finishes (pauses) the run.
    SetPauseOnEnd(bool),
    /// Re-enable scripts disabled by errors and clear the error list.
    ReenableScripts,
}

/// Result of a definition edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditReport {
    /// When the edit takes effect.
    pub policy: ApplyPolicy,
    /// Human-readable explanation of what changes when.
    pub notes: Vec<String>,
}

/// Classifies a definition edit (see the module documentation).
pub fn classify_edit(old: &LifeSystem, new: &LifeSystem) -> EditReport {
    let mut reset = Vec::new();
    let names = |s: &LifeSystem| s.entity_types.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    if names(old) != names(new) {
        reset.push("entity types were added, removed, renamed or reordered".to_string());
    } else {
        for (a, b) in old.entity_types.iter().zip(&new.entity_types) {
            if a.state_names() != b.state_names() {
                reset.push(format!("states of `{}` changed", a.name));
            }
            let props = |t: &crate::def::EntityType| t.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
            if props(a) != props(b) {
                reset.push(format!("properties of `{}` were added, removed or renamed", a.name));
            }
            let carried = |t: &crate::def::EntityType| t.carries.iter().map(|c| c.resource.clone()).collect::<Vec<_>>();
            if carried(a) != carried(b) {
                reset.push(format!("carried resources of `{}` changed", a.name));
            }
        }
    }
    let res = |s: &LifeSystem| s.world.resources.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
    if res(old) != res(new) {
        reset.push("resource fields were added, removed, renamed or reordered".to_string());
    }
    if (old.world.width, old.world.height) != (new.world.width, new.world.height) {
        reset.push("world size changed".to_string());
    }
    if old.world.topology != new.world.topology {
        reset.push("world topology changed".to_string());
    }
    if !reset.is_empty() {
        let mut notes = vec!["the change requires a reset; it will be used from the next reset on".to_string()];
        notes.extend(reset);
        return EditReport { policy: ApplyPolicy::RequiresReset, notes };
    }
    let mut notes = vec!["rules and parameters apply from the next tick; per-rule cooldowns restart".to_string()];
    let initial_changed = old.entity_types.iter().zip(&new.entity_types).any(|(a, b)| a.initial != b.initial)
        || old.world.resources.iter().zip(&new.world.resources).any(|(a, b)| a.initial != b.initial);
    if initial_changed {
        notes.push("initial populations or initial field contents take effect at the next reset".to_string());
    }
    EditReport { policy: ApplyPolicy::NextTick, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{EntityType, ResourceField};
    use simulation_core::Topology;

    fn base() -> LifeSystem {
        let mut s = LifeSystem::new("t", 16, 16);
        s.entity_types.push(EntityType::new("a"));
        s.world.resources.push(ResourceField::new("food"));
        s
    }

    #[test]
    fn behaviour_edits_apply_next_tick() {
        let old = base();
        let mut new = base();
        new.entity_types[0].energy.metabolism = 2.0;
        new.world.resources[0].regen = 0.5;
        new.rules.push(crate::dsl::parse_rule("rule r for a: then die()").unwrap());
        assert_eq!(classify_edit(&old, &new).policy, ApplyPolicy::NextTick);
        new.entity_types[0].initial.count = 5;
        let report = classify_edit(&old, &new);
        assert_eq!(report.policy, ApplyPolicy::NextTick);
        assert!(report.notes.iter().any(|n| n.contains("next reset")));
    }

    #[test]
    fn structural_edits_require_reset() {
        let old = base();
        let edits: [fn(&mut LifeSystem); 5] = [
            |s: &mut LifeSystem| s.entity_types.push(EntityType::new("b")),
            |s: &mut LifeSystem| s.entity_types[0].states = vec!["x".into(), "y".into()],
            |s: &mut LifeSystem| s.world.width = 32,
            |s: &mut LifeSystem| s.world.topology = Topology::Bounded,
            |s: &mut LifeSystem| s.world.resources[0].name = "water".into(),
        ];
        for edit in edits {
            let mut new = base();
            edit(&mut new);
            assert_eq!(classify_edit(&old, &new).policy, ApplyPolicy::RequiresReset);
        }
    }
}
