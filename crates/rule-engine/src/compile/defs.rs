//! Validation of entity, world and relationship definitions (everything
//! except rule and script bodies).

use std::collections::BTreeSet;

use simulation_core::Topology;

use crate::compile::ir::{MAX_SENSE_RADIUS, PropRef};
use crate::def::{
    Distribution, EndCondition, EnvPattern, EventEffect, FieldInit, LifeSystem, MovementDef, RelationKind,
    ReproductionMode, ScriptLimits, TransformTrigger,
};
use crate::error::{Diagnostic, ItemRef, did_you_mean, item_index};

/// Largest accepted world side in cells.
pub const MAX_WORLD_SIDE: u32 = 4096;
/// Smallest accepted world side in cells.
pub const MIN_WORLD_SIDE: u32 = 4;
/// Maximum number of entity types.
pub const MAX_TYPES: usize = 256;
/// Hard cap of `max_entities`.
pub const MAX_ENTITIES_CEILING: u32 = 1_000_000;
/// Maximum number of entries of every other list of a system (rules,
/// relationships, scripts, resources, environment fields, random events,
/// end conditions, properties, states, carried resources).
pub const MAX_LIST_LEN: usize = 10_000;

/// Words with a fixed meaning in rules, unusable as property names.
const RESERVED: &[&str] = &["self", "organism", "other", "world", "tick", "state", "type", "any", "same_species"];

struct Check {
    diags: Vec<Diagnostic>,
    /// Item the diagnostics being produced belong to.
    item: ItemRef,
}

impl Check {
    fn push(&mut self, d: Diagnostic) {
        let item = self.item;
        self.diags.push(d.for_item(item));
    }

    fn fail(&mut self, context: &str, message: impl Into<String>) {
        self.push(Diagnostic::error("E020", message).context(context.to_string()));
    }

    fn require(&mut self, ok: bool, context: &str, message: impl FnOnce() -> String) {
        if !ok {
            self.fail(context, message());
        }
    }

    fn finite_range(&mut self, value: f64, lo: f64, hi: f64, context: &str, what: &str) {
        self.require(value.is_finite() && value >= lo && value <= hi, context, || {
            format!("{what} must be in [{lo}, {hi}] (got {value})")
        });
    }

    fn unique<'a>(&mut self, names: impl IntoIterator<Item = &'a str>, context: &str, what: &str) {
        let mut seen = BTreeSet::new();
        for n in names {
            if n.trim().is_empty() {
                self.fail(context, format!("{what} names must not be empty"));
            } else if !seen.insert(n) {
                self.push(Diagnostic::error("E016", format!("duplicate {what} name `{n}`")).context(context));
            }
        }
    }
}

fn is_identifier(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// Validates everything but rule/script bodies.
pub(crate) fn validate_definitions(sys: &LifeSystem) -> Vec<Diagnostic> {
    let mut ck = Check { diags: Vec::new(), item: ItemRef::System };
    let w = &sys.world;
    let world_ctx = "world";
    ck.require(sys.entity_types.len() <= MAX_TYPES, "system", || format!("at most {MAX_TYPES} entity types"));
    for (what, len) in [
        ("rules", sys.rules.len()),
        ("relationships", sys.relationships.len()),
        ("scripts", sys.scripts.len()),
        ("end conditions", sys.end_conditions.len()),
        ("resource fields", sys.world.resources.len()),
        ("environment fields", sys.world.environment.len()),
        ("random events", sys.world.random_events.len()),
    ] {
        ck.require(len <= MAX_LIST_LEN, "system", || format!("at most {MAX_LIST_LEN} {what} (got {len})"));
    }
    ck.item = ItemRef::World;
    for (what, side) in [("width", w.width), ("height", w.height)] {
        ck.require((MIN_WORLD_SIDE..=MAX_WORLD_SIDE).contains(&side), world_ctx, || {
            format!("world {what} must be in [{MIN_WORLD_SIDE}, {MAX_WORLD_SIDE}] (got {side})")
        });
    }
    ck.require(w.topology != Topology::Infinite, world_ctx, || {
        "custom-life worlds must be finite (topology Infinite is not supported: fields need a size)".into()
    });
    ck.require((1..=MAX_ENTITIES_CEILING).contains(&w.max_entities), world_ctx, || {
        format!("max_entities must be in [1, {MAX_ENTITIES_CEILING}]")
    });
    let initial_total: u64 = sys.entity_types.iter().map(|t| u64::from(t.initial.count)).sum();
    ck.require(initial_total <= u64::from(w.max_entities), world_ctx, || {
        format!("the initial populations add up to {initial_total}, more than max_entities ({})", w.max_entities)
    });
    ck.item = ItemRef::System;

    // Names: types, resources and environment fields share one namespace.
    let all_names = sys
        .entity_types
        .iter()
        .map(|t| t.name.as_str())
        .chain(w.resources.iter().map(|r| r.name.as_str()))
        .chain(w.environment.iter().map(|e| e.name.as_str()));
    ck.unique(all_names, "system", "entity type / resource / environment field");
    for name in sys.entity_types.iter().map(|t| &t.name).chain(w.resources.iter().map(|r| &r.name)) {
        if RESERVED.contains(&name.as_str()) || name == "same" || name == "same_type" {
            ck.fail("system", format!("`{name}` is a reserved word and cannot name a type or resource"));
        }
    }
    let resource_names: Vec<&str> = w.resources.iter().map(|r| r.name.as_str()).collect();
    let type_names: Vec<&str> = sys.entity_types.iter().map(|t| t.name.as_str()).collect();
    let known_type = |ck: &mut Check, name: &str, context: &str| {
        if !type_names.contains(&name) {
            let help = did_you_mean(name, type_names.iter().copied());
            let mut d = Diagnostic::error("E003", format!("unknown entity type `{name}`")).context(context);
            if let Some(h) = help {
                d = d.help(h);
            }
            ck.push(d);
        }
    };
    let known_resource = |ck: &mut Check, name: &str, context: &str| {
        if !resource_names.contains(&name) {
            let mut d = Diagnostic::error("E003", format!("unknown resource `{name}`")).context(context);
            if let Some(h) = did_you_mean(name, resource_names.iter().copied()) {
                d = d.help(h);
            }
            ck.push(d);
        }
    };

    for (ti, t) in sys.entity_types.iter().enumerate() {
        ck.item = ItemRef::EntityType(item_index(ti));
        let ctx = format!("entity type `{}`", t.name);
        ck.require(t.color.iter().all(|c| (0.0..=1.0).contains(c)), &ctx, || {
            "colour channels must be in [0, 1]".into()
        });
        ck.finite_range(f64::from(t.size), 0.01, 100.0, &ctx, "size");
        ck.finite_range(f64::from(t.perception), 0.1, f64::from(MAX_SENSE_RADIUS), &ctx, "perception radius");
        ck.unique(t.states.iter().map(String::as_str), &ctx, "state");
        for (what, len) in
            [("states", t.states.len()), ("properties", t.properties.len()), ("carried resources", t.carries.len())]
        {
            ck.require(len <= MAX_LIST_LEN, &ctx, || format!("at most {MAX_LIST_LEN} {what} (got {len})"));
        }
        let mut prop_names: Vec<&str> = Vec::new();
        for p in &t.properties {
            ck.require(is_identifier(&p.name), &ctx, || format!("property name `{}` must be an identifier", p.name));
            ck.require(PropRef::builtin(&p.name).is_none() && !RESERVED.contains(&p.name.as_str()), &ctx, || {
                format!("property `{}` clashes with a built-in name", p.name)
            });
            ck.require(
                p.min.is_finite() && p.max.is_finite() && p.min <= p.default && p.default <= p.max,
                &ctx,
                || format!("property `{}` needs finite min <= default <= max", p.name),
            );
            prop_names.push(&p.name);
        }
        for c in &t.carries {
            known_resource(&mut ck, &c.resource, &ctx);
            ck.require(is_identifier(&c.resource), &ctx, || {
                format!("carried resource `{}` must have an identifier name", c.resource)
            });
            ck.require(
                c.capacity.is_finite() && c.capacity >= 0.0 && c.initial >= 0.0 && c.initial <= c.capacity,
                &ctx,
                || format!("carried `{}` needs 0 <= initial <= capacity", c.resource),
            );
            ck.require(PropRef::builtin(&c.resource).is_none(), &ctx, || {
                format!("carried resource `{}` clashes with a built-in property", c.resource)
            });
            prop_names.push(&c.resource);
        }
        ck.unique(prop_names, &ctx, "property / carried resource");
        match &t.movement {
            MovementDef::None => {}
            MovementDef::RandomWalk { speed } => ck.finite_range(f64::from(*speed), 0.0, 50.0, &ctx, "speed"),
            MovementDef::Directed { speed, turn } => {
                ck.finite_range(f64::from(*speed), 0.0, 50.0, &ctx, "speed");
                ck.finite_range(f64::from(*turn), 0.0, std::f64::consts::PI, &ctx, "turn");
            }
        }
        let e = &t.energy;
        ck.require(e.max.is_finite() && e.max > 0.0, &ctx, || "energy max must be positive".into());
        ck.finite_range(e.initial, 0.0, e.max.max(0.0), &ctx, "initial energy");
        ck.finite_range(e.metabolism, 0.0, 1.0e6, &ctx, "metabolism");
        let r = &t.reproduction;
        ck.finite_range(r.cost, 0.0, 1.0e9, &ctx, "reproduction cost");
        ck.finite_range(r.rate, 0.0, 1.0, &ctx, "reproduction rate");
        ck.require((1..=100).contains(&r.litter), &ctx, || "litter must be in [1, 100]".into());
        ck.require(r.mode != ReproductionMode::None || r.rate == 0.0, &ctx, || {
            "reproduction rate > 0 needs mode Asexual or Sexual".into()
        });
        if let Some(max_age) = t.lifecycle.max_age {
            ck.require(max_age > 0, &ctx, || "max_age must be at least 1".into());
        }
        match t.initial.distribution {
            Distribution::Cluster { x, y, radius } => {
                ck.require(x.is_finite() && y.is_finite() && radius.is_finite() && radius >= 0.0, &ctx, || {
                    "cluster needs a finite centre and a non-negative radius".into()
                });
            }
            Distribution::Rect { x, y, width, height } => {
                ck.require(
                    [x, y, width, height].iter().all(|v| v.is_finite()) && width >= 0.0 && height >= 0.0,
                    &ctx,
                    || "rectangle needs finite position and non-negative size".into(),
                );
            }
            Distribution::Uniform | Distribution::Grid => {}
        }
        ck.require(t.initial.count <= w.max_entities, &ctx, || "initial count exceeds max_entities".into());
    }

    for (ri, r) in w.resources.iter().enumerate() {
        ck.item = ItemRef::Resource(item_index(ri));
        let ctx = format!("resource `{}`", r.name);
        ck.require(is_identifier(&r.name), &ctx, || "resource names must be identifiers".into());
        ck.finite_range(f64::from(r.max), 0.0, 1.0e9, &ctx, "max");
        ck.finite_range(f64::from(r.regen), 0.0, 1.0e6, &ctx, "regen");
        ck.finite_range(f64::from(r.diffusion), 0.0, 0.25, &ctx, "diffusion (explicit-scheme stability limit)");
        ck.finite_range(f64::from(r.decay), 0.0, 1.0, &ctx, "decay");
        ck.require(r.energy_value.is_finite(), &ctx, || "energy_value must be finite".into());
        match r.initial {
            FieldInit::Uniform(v) => ck.finite_range(f64::from(v), 0.0, 1.0e9, &ctx, "initial amount"),
            FieldInit::Random { min, max } => {
                ck.require(min.is_finite() && max.is_finite() && 0.0 <= min && min <= max, &ctx, || {
                    "random initial needs 0 <= min <= max".into()
                })
            }
            FieldInit::Patches { count, radius, amount } => ck.require(
                count <= 10_000 && radius.is_finite() && radius >= 0.0 && amount.is_finite() && amount >= 0.0,
                &ctx,
                || "patches need count <= 10000 and non-negative radius/amount".into(),
            ),
            FieldInit::Gradient { from, to, .. } => {
                ck.require(from.is_finite() && to.is_finite() && from >= 0.0 && to >= 0.0, &ctx, || {
                    "gradient values must be finite and non-negative".into()
                })
            }
        }
    }
    for (ei, e) in w.environment.iter().enumerate() {
        ck.item = ItemRef::Environment(item_index(ei));
        let ctx = format!("environment field `{}`", e.name);
        let finite = match &e.pattern {
            EnvPattern::Constant(v) => v.is_finite(),
            EnvPattern::Gradient { from, to, .. } => from.is_finite() && to.is_finite(),
            EnvPattern::Radial { x, y, radius, peak, edge } => {
                [x, y, peak, edge].iter().all(|v| v.is_finite()) && radius.is_finite() && *radius > 0.0
            }
        };
        ck.require(finite, &ctx, || "pattern values must be finite (and radius positive)".into());
        if let Some(s) = &e.seasonal {
            ck.require(s.amplitude.is_finite() && s.period.is_finite() && s.period > 0.0, &ctx, || {
                "seasonality needs a finite amplitude and a positive period".into()
            });
        }
    }
    ck.item = ItemRef::World;
    ck.unique(w.random_events.iter().map(|e| e.name.as_str()), world_ctx, "random event");
    for (ei, ev) in w.random_events.iter().enumerate() {
        ck.item = ItemRef::RandomEvent(item_index(ei));
        let ctx = format!("random event `{}`", ev.name);
        ck.finite_range(ev.probability, 0.0, 1.0, &ctx, "probability");
        match &ev.effect {
            EventEffect::ResourcePulse { resource, amount, radius } => {
                known_resource(&mut ck, resource, &ctx);
                ck.require(amount.is_finite() && radius.is_finite() && *radius >= 0.0, &ctx, || {
                    "pulse needs a finite amount and a non-negative radius".into()
                });
            }
            EventEffect::ScaleResource { resource, factor } => {
                known_resource(&mut ck, resource, &ctx);
                ck.finite_range(f64::from(*factor), 0.0, 1.0e3, &ctx, "factor");
            }
            EventEffect::Catastrophe { radius, kill_probability, entity } => {
                ck.require(radius.is_finite() && *radius >= 0.0, &ctx, || "radius must be non-negative".into());
                ck.finite_range(*kill_probability, 0.0, 1.0, &ctx, "kill probability");
                if let Some(t) = entity {
                    known_type(&mut ck, t, &ctx);
                }
            }
            EventEffect::Immigration { entity, count } => {
                known_type(&mut ck, entity, &ctx);
                ck.require(*count <= 10_000, &ctx, || "immigration count must be <= 10000".into());
            }
        }
    }
    for (ci, c) in sys.end_conditions.iter().enumerate() {
        ck.item = ItemRef::EndCondition(item_index(ci));
        match c {
            EndCondition::Extinction(t) => known_type(&mut ck, t, "end conditions"),
            EndCondition::PopulationAbove { entity: Some(t), .. }
            | EndCondition::PopulationBelow { entity: Some(t), .. } => known_type(&mut ck, t, "end conditions"),
            _ => {}
        }
    }
    ck.item = ItemRef::ScriptLimits;
    validate_limits(&mut ck, &sys.script_limits);

    let carries = |ty: &str, res: &str| {
        sys.entity_types.iter().find(|t| t.name == ty).is_some_and(|t| t.carries.iter().any(|c| c.resource == res))
    };
    for (i, rel) in sys.relationships.iter().enumerate() {
        ck.item = ItemRef::Relationship(item_index(i));
        let ctx = format!("relationship #{} ({} {} {})", i + 1, rel.subject, rel.kind.label(), rel.object);
        if let Some(j) = sys.relationships[..i].iter().position(|r| r == rel) {
            ck.push(
                Diagnostic::warning("W102", format!("relationship #{} is an exact duplicate of #{}", i + 1, j + 1))
                    .context(ctx.clone())
                    .help("remove one of them; each copy generates its own rules"),
            );
        }
        known_type(&mut ck, &rel.subject, &ctx);
        ck.require((-99..=99).contains(&rel.priority_shift), &ctx, || "priority_shift must be in [-99, 99]".into());
        let object_is_resource = resource_names.contains(&rel.object.as_str());
        let param = |ck: &mut Check, v: f64, lo: f64, hi: f64, what: &str| ck.finite_range(v, lo, hi, &ctx, what);
        match &rel.kind {
            RelationKind::Consumes { efficiency, hunger, radius, bite, handling: _ } => {
                if !object_is_resource {
                    known_type(&mut ck, &rel.object, &ctx);
                }
                param(&mut ck, *efficiency, 0.0, 1.0, "efficiency");
                param(&mut ck, *radius, 0.1, f64::from(MAX_SENSE_RADIUS), "radius");
                param(&mut ck, *bite, 0.0, 1.0e6, "bite");
                if let Some(h) = hunger {
                    param(&mut ck, *h, 0.0, 1.0e9, "hunger");
                }
            }
            RelationKind::Produces { amount, probability, cost } => {
                if !object_is_resource {
                    known_type(&mut ck, &rel.object, &ctx);
                    ck.require(amount.fract() == 0.0 && (1.0..=100.0).contains(amount), &ctx, || {
                        "producing entities needs a whole amount in [1, 100]".into()
                    });
                }
                param(&mut ck, *amount, 0.0, 1.0e6, "amount");
                param(&mut ck, *probability, 0.0, 1.0, "probability");
                param(&mut ck, *cost, 0.0, 1.0e9, "cost");
            }
            RelationKind::TradesWith { give, give_amount, receive, receive_amount, radius } => {
                known_type(&mut ck, &rel.object, &ctx);
                known_resource(&mut ck, give, &ctx);
                known_resource(&mut ck, receive, &ctx);
                param(&mut ck, *give_amount, 0.0, 1.0e9, "give_amount");
                param(&mut ck, *receive_amount, 0.0, 1.0e9, "receive_amount");
                param(&mut ck, *radius, 0.1, f64::from(MAX_SENSE_RADIUS), "radius");
                ck.require(give != receive, &ctx, || {
                    "trades_with needs two different resources (give ≠ receive)".into()
                });
                // A trade moves stocks between both parties, so both must hold both resources.
                for party in [&rel.subject, &rel.object] {
                    let party_known = type_names.contains(&party.as_str());
                    for res in [give, receive] {
                        if party_known && resource_names.contains(&res.as_str()) && !carries(party, res) {
                            ck.push(
                                Diagnostic::error("E020", format!("`{party}` must carry `{res}` to trade it"))
                                    .context(ctx.clone())
                                    .help(format!(
                                        "add (resource: \"{res}\", capacity: ...) to the `carries` of `{party}`"
                                    )),
                            );
                        }
                    }
                }
            }
            other => {
                known_type(&mut ck, &rel.object, &ctx);
                match other {
                    RelationKind::Protects { radius, damage } => {
                        param(&mut ck, *radius, 0.1, f64::from(MAX_SENSE_RADIUS) / 2.0, "radius");
                        param(&mut ck, *damage, 0.0, 1.0e9, "damage");
                    }
                    RelationKind::Chases { radius, speed_factor } | RelationKind::Avoids { radius, speed_factor } => {
                        param(&mut ck, *radius, 0.1, f64::from(MAX_SENSE_RADIUS), "radius");
                        param(&mut ck, *speed_factor, 0.0, 10.0, "speed_factor");
                    }
                    RelationKind::Transforms { trigger, probability } => {
                        param(&mut ck, *probability, 0.0, 1.0, "probability");
                        if let TransformTrigger::Contact(t) = trigger {
                            known_type(&mut ck, t, &ctx);
                        }
                    }
                    RelationKind::ReproducesWith { radius, probability, min_energy } => {
                        param(&mut ck, *radius, 0.1, f64::from(MAX_SENSE_RADIUS), "radius");
                        param(&mut ck, *probability, 0.0, 1.0, "probability");
                        param(&mut ck, *min_energy, 0.0, 1.0e9, "min_energy");
                        let sexual = sys
                            .entity_types
                            .iter()
                            .find(|t| t.name == rel.subject)
                            .is_some_and(|t| t.reproduction.mode == ReproductionMode::Sexual);
                        ck.require(sexual, &ctx, || "reproduces_with needs a subject with Sexual reproduction".into());
                    }
                    RelationKind::CompetesWith { aggression, damage } => {
                        param(&mut ck, *aggression, 0.0, 1.0, "aggression");
                        param(&mut ck, *damage, 0.0, 1.0e9, "damage");
                    }
                    _ => {}
                }
            }
        }
    }
    ck.diags
}

fn validate_limits(ck: &mut Check, l: &ScriptLimits) {
    let c = ScriptLimits::CEILING;
    let ctx = "script limits";
    ck.require((1..=c.max_operations).contains(&l.max_operations), ctx, || {
        format!("max_operations must be in [1, {}]", c.max_operations)
    });
    for (v, max, what) in [
        (l.max_call_levels, c.max_call_levels, "max_call_levels"),
        (l.max_expr_depth, c.max_expr_depth, "max_expr_depth"),
        (l.max_string_size, c.max_string_size, "max_string_size"),
        (l.max_array_size, c.max_array_size, "max_array_size"),
        (l.max_map_size, c.max_map_size, "max_map_size"),
        (l.max_variables, c.max_variables, "max_variables"),
        (l.max_functions, c.max_functions, "max_functions"),
    ] {
        ck.require((1..=max).contains(&v), ctx, || format!("{what} must be in [1, {max}]"));
    }
}
