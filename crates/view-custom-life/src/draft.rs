//! The editable draft of a life system: rename propagation across rules,
//! relationships, scripts and the world; the comparison of the draft with
//! the running system and the system the next reset will use.

use rule_engine::def::{EndCondition, EventEffect, LifeSystem, RelationKind, TransformTrigger};
use rule_engine::dsl::{Action, Arg, Event, Expr, ExprKind, RuleAst, parse_expr, print_expr};
use rule_engine::{EditReport, classify_edit};
use simulation_core::ApplyPolicy;

/// How the draft relates to the running system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// Draft, running system and reset target are identical.
    InSync,
    /// The draft equals the applied definition, which waits for a reset.
    PendingReset,
    /// The draft has edits that were not applied.
    Edited,
}

/// Compares the draft with the running system and the reset target.
pub fn sync_status(running: &LifeSystem, target: &LifeSystem, draft: &LifeSystem) -> SyncStatus {
    if draft != target {
        SyncStatus::Edited
    } else if target != running {
        SyncStatus::PendingReset
    } else {
        SyncStatus::InSync
    }
}

/// What applying the draft would do (from the model's own classification).
pub fn apply_preview(running: &LifeSystem, draft: &LifeSystem) -> EditReport {
    classify_edit(running, draft)
}

/// Short user-facing text of an apply policy for definition edits.
pub fn policy_text(policy: ApplyPolicy) -> &'static str {
    match policy {
        ApplyPolicy::Immediate | ApplyPolicy::NextTick => "Applies from the next tick; live entities are kept.",
        ApplyPolicy::RequiresReset => "Structural change: it takes effect at the next reset.",
    }
}

/// A new, valid, empty system (48 × 48 torus).
pub fn blank_system(name: &str) -> LifeSystem {
    let mut sys = LifeSystem::new(name, 48, 48);
    sys.description = "A new life system.".into();
    sys
}

/// A name that is not in `taken`: `base`, `base_2`, `base_3`...
pub fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str> + Clone) -> String {
    if !taken.clone().into_iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| !taken.clone().into_iter().any(|t| t == candidate))
        .unwrap_or_else(|| base.to_string())
}

/// Every name of the shared namespace (types, resources, environment fields).
pub fn namespace(sys: &LifeSystem) -> Vec<&str> {
    sys.entity_types
        .iter()
        .map(|t| t.name.as_str())
        .chain(sys.world.resources.iter().map(|r| r.name.as_str()))
        .chain(sys.world.environment.iter().map(|e| e.name.as_str()))
        .collect()
}

fn rename_in_expr(e: &mut Expr, old: &str, new: &str) {
    match &mut e.kind {
        ExprKind::Path(p) if p.len() == 1 && p[0] == old => p[0] = new.to_string(),
        ExprKind::Str(s) if s == old => *s = new.to_string(),
        ExprKind::Call { args, .. } => {
            for a in args {
                rename_in_expr(&mut a.value, old, new);
            }
        }
        ExprKind::Unary { expr, .. } => rename_in_expr(expr, old, new),
        ExprKind::Binary { lhs, rhs, .. } => {
            rename_in_expr(lhs, old, new);
            rename_in_expr(rhs, old, new);
        }
        _ => {}
    }
}

fn rename_in_args(args: &mut [Arg], old: &str, new: &str) {
    for a in args {
        rename_in_expr(&mut a.value, old, new);
    }
}

/// Renames a name (`Path([old])` / `"old"`) everywhere inside one rule:
/// condition, action arguments, values, collision partner, entered state
/// and products. Property paths (`self.x`) are left alone.
pub fn rename_in_rule(rule: &mut RuleAst, old: &str, new: &str) {
    if let Some(c) = &mut rule.condition {
        rename_in_expr(c, old, new);
    }
    for a in &mut rule.actions {
        match a {
            Action::Call { args, .. } => rename_in_args(args, old, new),
            Action::Set { value, .. } | Action::Add { value, .. } => rename_in_expr(value, old, new),
        }
    }
    match &mut rule.event {
        Event::Collision { with: Some(n) } if n.text == old => n.text = new.to_string(),
        Event::StateEnter { state } if state.text == old => state.text = new.to_string(),
        _ => {}
    }
    for p in &mut rule.meta.produces {
        if p.name.text == old {
            p.name.text = new.to_string();
        }
    }
}

fn rename_in_condition_text(text: &mut String, old: &str, new: &str) {
    if let Ok(mut e) = parse_expr(text) {
        let before = e.clone();
        rename_in_expr(&mut e, old, new);
        if e != before {
            *text = print_expr(&e);
        }
    }
}

/// Renames an entity type and every reference to it (rules, relationships,
/// transform triggers, scripts' owner, random events, end conditions).
/// Script sources are not rewritten (they are free text).
pub fn rename_entity_type(sys: &mut LifeSystem, old: &str, new: &str) {
    if old == new {
        return;
    }
    for t in &mut sys.entity_types {
        if t.name == old {
            t.name = new.to_string();
        }
    }
    for r in &mut sys.rules {
        if r.entity.text == old {
            r.entity.text = new.to_string();
        }
        rename_in_rule(r, old, new);
    }
    for rel in &mut sys.relationships {
        if rel.subject == old {
            rel.subject = new.to_string();
        }
        if rel.object == old {
            rel.object = new.to_string();
        }
        if let RelationKind::Transforms { trigger, .. } = &mut rel.kind {
            match trigger {
                TransformTrigger::Contact(t) if t == old => *t = new.to_string(),
                TransformTrigger::Condition(c) => rename_in_condition_text(c, old, new),
                _ => {}
            }
        }
    }
    for s in &mut sys.scripts {
        if s.entity == old {
            s.entity = new.to_string();
        }
        match &mut s.event {
            Event::Collision { with: Some(n) } if n.text == old => n.text = new.to_string(),
            _ => {}
        }
    }
    for ev in &mut sys.world.random_events {
        match &mut ev.effect {
            EventEffect::Catastrophe { entity: Some(e), .. } | EventEffect::Immigration { entity: e, .. }
                if e == old =>
            {
                *e = new.to_string();
            }
            _ => {}
        }
    }
    for c in &mut sys.end_conditions {
        match c {
            EndCondition::Extinction(e)
            | EndCondition::PopulationAbove { entity: Some(e), .. }
            | EndCondition::PopulationBelow { entity: Some(e), .. }
                if e == old =>
            {
                *e = new.to_string();
            }
            _ => {}
        }
    }
}

/// Renames a resource field and every reference to it (carried resources,
/// rules, relationships incl. trades, random events). A resource is also
/// readable as a property when carried, so `self.old` paths are renamed in
/// rules of types that carry it.
pub fn rename_resource(sys: &mut LifeSystem, old: &str, new: &str) {
    if old == new {
        return;
    }
    let carriers: Vec<String> = sys
        .entity_types
        .iter()
        .filter(|t| t.carries.iter().any(|c| c.resource == old))
        .map(|t| t.name.clone())
        .collect();
    for r in &mut sys.world.resources {
        if r.name == old {
            r.name = new.to_string();
        }
    }
    for t in &mut sys.entity_types {
        for c in &mut t.carries {
            if c.resource == old {
                c.resource = new.to_string();
            }
        }
    }
    for r in &mut sys.rules {
        rename_in_rule(r, old, new);
        if carriers.contains(&r.entity.text) {
            rename_property_paths(r, old, new);
        }
        for c in &mut r.meta.costs {
            if carriers.contains(&r.entity.text)
                && let Some(last) = c.path.last_mut()
                && last == old
            {
                *last = new.to_string();
            }
        }
    }
    for rel in &mut sys.relationships {
        if rel.object == old {
            rel.object = new.to_string();
        }
        match &mut rel.kind {
            RelationKind::TradesWith { give, receive, .. } => {
                if give == old {
                    *give = new.to_string();
                }
                if receive == old {
                    *receive = new.to_string();
                }
            }
            RelationKind::Transforms { trigger: TransformTrigger::Condition(c), .. } => {
                rename_in_condition_text(c, old, new);
            }
            _ => {}
        }
    }
    for ev in &mut sys.world.random_events {
        match &mut ev.effect {
            EventEffect::ResourcePulse { resource, .. } | EventEffect::ScaleResource { resource, .. }
                if resource == old =>
            {
                *resource = new.to_string();
            }
            _ => {}
        }
    }
}

fn rename_path(path: &mut [String], old: &str, new: &str) {
    match path {
        [one] if one == old => *one = new.to_string(),
        [subject, last] if (subject == "self" || subject == "organism") && last == old => *last = new.to_string(),
        _ => {}
    }
}

fn rename_property_paths(rule: &mut RuleAst, old: &str, new: &str) {
    fn walk(e: &mut Expr, old: &str, new: &str) {
        match &mut e.kind {
            ExprKind::Path(p) => rename_path(p, old, new),
            ExprKind::Call { args, .. } => args.iter_mut().for_each(|a| walk(&mut a.value, old, new)),
            ExprKind::Unary { expr, .. } => walk(expr, old, new),
            ExprKind::Binary { lhs, rhs, .. } => {
                walk(lhs, old, new);
                walk(rhs, old, new);
            }
            _ => {}
        }
    }
    if let Some(c) = &mut rule.condition {
        walk(c, old, new);
    }
    for a in &mut rule.actions {
        match a {
            Action::Call { args, .. } => args.iter_mut().for_each(|x| walk(&mut x.value, old, new)),
            Action::Set { path, value, .. } | Action::Add { path, value, .. } => {
                rename_path(path, old, new);
                walk(value, old, new);
            }
        }
    }
}

/// Renames a user property of `entity` in its definition and in that type's
/// rules (`name`, `self.name`, `organism.name`, `set`/`add` targets and
/// costs). `other.name` in other types' collision rules and script sources
/// are not rewritten.
pub fn rename_property(sys: &mut LifeSystem, entity: &str, old: &str, new: &str) {
    if old == new {
        return;
    }
    if let Some(t) = sys.entity_types.iter_mut().find(|t| t.name == entity) {
        for p in &mut t.properties {
            if p.name == old {
                p.name = new.to_string();
            }
        }
    }
    for r in sys.rules.iter_mut().filter(|r| r.entity.text == entity) {
        rename_property_paths(r, old, new);
        for c in &mut r.meta.costs {
            rename_path(&mut c.path, old, new);
        }
    }
}

/// Renames an environment field (read with `field("name")` in rules and
/// transform conditions).
pub fn rename_environment(sys: &mut LifeSystem, old: &str, new: &str) {
    if old == new {
        return;
    }
    for e in &mut sys.world.environment {
        if e.name == old {
            e.name = new.to_string();
        }
    }
    for r in &mut sys.rules {
        rename_in_rule(r, old, new);
    }
    for rel in &mut sys.relationships {
        if let RelationKind::Transforms { trigger: TransformTrigger::Condition(c), .. } = &mut rel.kind {
            rename_in_condition_text(c, old, new);
        }
    }
}

/// Renames a state of `entity` in that type's rules and scripts' events
/// (`set_state`, `state ==`, `on_state_enter`, `produces`).
pub fn rename_state(sys: &mut LifeSystem, entity: &str, old: &str, new: &str) {
    if old == new {
        return;
    }
    if let Some(t) = sys.entity_types.iter_mut().find(|t| t.name == entity) {
        for s in &mut t.states {
            if s == old {
                *s = new.to_string();
            }
        }
    }
    for r in sys.rules.iter_mut().filter(|r| r.entity.text == entity) {
        rename_in_rule(r, old, new);
    }
    for s in sys.scripts.iter_mut().filter(|s| s.entity == entity) {
        if let Event::StateEnter { state } = &mut s.event
            && state.text == old
        {
            state.text = new.to_string();
        }
    }
}

/// Removes an entity type and every relationship, rule and script that
/// belongs to it (references inside other rules are left for validation
/// to report).
pub fn remove_entity_type(sys: &mut LifeSystem, index: usize) {
    if index >= sys.entity_types.len() {
        return;
    }
    let name = sys.entity_types.remove(index).name;
    sys.relationships.retain(|r| r.subject != name && r.object != name);
    sys.rules.retain(|r| r.entity.text != name);
    sys.scripts.retain(|s| s.entity != name);
}

/// Detaches a generated rule: copies it into the manual rules under a free
/// name (`<rule> (detached)`). The caller removes its source (relationship
/// or reproduction rate) so it is not generated twice.
pub fn detach_rule(sys: &mut LifeSystem, generated: &RuleAst) -> usize {
    let mut rule = generated.clone();
    let taken: Vec<&str> =
        sys.rules.iter().filter(|r| r.entity.text == rule.entity.text).map(|r| r.name.as_str()).collect();
    rule.name = unique_name(&format!("{} (detached)", rule.name), taken);
    sys.rules.push(rule);
    sys.rules.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::{compile, scenarios};

    fn preset(id: &str) -> LifeSystem {
        scenarios::builtin_by_id(id).unwrap().system
    }

    #[test]
    fn renaming_a_type_keeps_the_system_valid() {
        for id in ["producers-consumers", "virus-and-cells", "artificial-chemistry", "competitive-colonies"] {
            let mut sys = preset(id);
            let old = sys.entity_types[0].name.clone();
            rename_entity_type(&mut sys, &old, "renamed_type");
            let compiled = compile(&sys).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(compiled.type_index("renamed_type").is_some());
            assert!(compiled.type_index(&old).is_none());
            let text = compiled.rules_text();
            assert!(!text.contains(&format!(" {old} ")) && !text.contains(&format!("({old}")), "{id}:\n{text}");
        }
    }

    #[test]
    fn renaming_a_resource_keeps_the_system_valid() {
        for id in ["producers-consumers", "bacterial-growth", "virus-and-cells", "robot-swarm"] {
            let mut sys = preset(id);
            let old = sys.world.resources[0].name.clone();
            rename_resource(&mut sys, &old, "renamed_resource");
            compile(&sys).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(sys.world.resources[0].name, "renamed_resource");
        }
    }

    #[test]
    fn renaming_states_and_environment_fields() {
        let mut sys = preset("robot-swarm");
        rename_state(&mut sys, "robot", "waiting", "resting");
        assert_eq!(sys.entity_types[0].states, vec!["moving".to_string(), "resting".to_string()]);
        // The rule's set_state(waiting) follows; the script source is free text and does not.
        let text = rule_engine::dsl::print_rule(&sys.rules[0]);
        assert!(text.contains("set_state(resting)"), "{text}");
        let mut alien = preset("alien-ecosystem");
        rename_environment(&mut alien, "daylight", "sunshine");
        compile(&alien).unwrap();
    }

    #[test]
    fn renaming_a_property_follows_into_rules_and_costs() {
        let mut sys = preset("bacterial-growth");
        sys.entity_types[0].properties.push(rule_engine::def::PropertyDef {
            name: "toxin".into(),
            default: 0.0,
            min: 0.0,
            max: 10.0,
        });
        sys.rules.push(
            rule_engine::dsl::parse_rule(
                "rule brew for bacterium: when self.toxin < 5 then add toxin += 1 cost toxin 0.5 cooldown 3",
            )
            .unwrap(),
        );
        compile(&sys).unwrap();
        rename_property(&mut sys, "bacterium", "toxin", "venom");
        compile(&sys).unwrap();
        let text = rule_engine::dsl::print_rule(&sys.rules[2]);
        assert!(
            text.contains("self.venom < 5") && text.contains("add venom += 1") && text.contains("cost venom 0.5"),
            "{text}"
        );
    }

    #[test]
    fn sync_status_tracks_draft_target_and_running() {
        let a = preset("bacterial-growth");
        let mut b = a.clone();
        b.entity_types[0].energy.metabolism += 0.01;
        assert_eq!(sync_status(&a, &a, &a), SyncStatus::InSync);
        assert_eq!(sync_status(&a, &a, &b), SyncStatus::Edited);
        assert_eq!(sync_status(&a, &b, &b), SyncStatus::PendingReset);
        assert_eq!(apply_preview(&a, &b).policy, ApplyPolicy::NextTick);
        let mut c = a.clone();
        c.world.width = 64;
        assert_eq!(apply_preview(&a, &c).policy, ApplyPolicy::RequiresReset);
    }

    #[test]
    fn removal_detach_and_names() {
        let mut sys = preset("producers-consumers");
        let compiled = compile(&sys).unwrap();
        let generated = compiled.rules().iter().find(|r| r.origin.is_generated()).unwrap().ast.clone();
        let i = detach_rule(&mut sys, &generated);
        assert!(sys.rules[i].name.ends_with("(detached)"));
        remove_entity_type(&mut sys, 2);
        assert!(sys.relationships.iter().all(|r| r.subject != "hunter" && r.object != "hunter"));
        assert_eq!(unique_name("plant", ["plant", "plant_2"]), "plant_3");
        assert_eq!(unique_name("tree", ["plant"]), "tree");
        assert!(compile(&blank_system("x")).is_ok());
        assert_eq!(namespace(&preset("producers-consumers")), vec!["plant", "grazer", "hunter", "sunlight"]);
    }
}
